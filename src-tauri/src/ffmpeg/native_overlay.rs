//! Feed native-rendered premultiplied BGRA directly into the shared media graph.

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, RecvTimeoutError},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use super::runner::{run_native_media_with_sources, ProgressMode};
use crate::{
    danmu2ass::{ImageDanmu, PLAY_RES_Y},
    progress::progress_reporter::ProgressReporterTrait,
};
use ffmpeg_utils::{
    MediaBindings, MediaInput, MediaJobResult, MediaPlan, RawVideoAlpha, RawVideoCancelHandle,
    RawVideoFrame, RawVideoSpec, RawVideoStats,
};

const FRAME_BUFFERS: usize = 3;
const BUFFER_POLL: Duration = Duration::from_millis(25);

/// The same system font manager and scene renderer are used by preview and burn.
pub fn system_renderer(
    width: u32,
    height: u32,
    input: native_overlay::RenderInput,
) -> Result<native_overlay::Renderer, String> {
    native_overlay::Renderer::new(width, height, input, Vec::new())
}

/// Producer, frame-copy and media stages overlap. Drain includes graph flushing,
/// encoding and muxing after the raw source finishes, not isolated encoder CPU.
#[derive(Debug, Default)]
pub struct OverlayRunStats {
    pub initialization: Duration,
    pub buffer_allocation: Duration,
    pub buffer_wait: Duration,
    pub queue_send_wait: Duration,
    pub render: native_overlay::RenderStats,
    pub feed: RawVideoStats,
    pub wall: Duration,
    pub native_drain: Option<Duration>,
    pub buffer_allocations: usize,
}

pub struct OverlaySpec {
    pub input: native_overlay::RenderInput,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    /// Renderer timeline offset; raw graph timestamps begin at zero.
    pub start: f64,
    pub duration: f64,
}
impl OverlaySpec {
    fn layout(&self) -> Result<(usize, u64), String> {
        let pixels = u64::from(self.width) * u64::from(self.height);
        if self.width == 0
            || self.height == 0
            || self.width > 8192
            || self.height > 8192
            || pixels > 16_777_216
            || !self.fps.is_finite()
            || !(1.0..=240.0).contains(&self.fps)
            || !self.start.is_finite()
            || self.start < 0.0
            || !self.duration.is_finite()
            || self.duration <= 0.0
            || self.duration > 7.0 * 24.0 * 60.0 * 60.0
            || !(self.start + self.duration).is_finite()
        {
            return Err("Invalid native overlay dimensions, FPS or time range".into());
        }
        Ok((
            (pixels * 4) as usize,
            (self.duration * self.fps).ceil() as u64,
        ))
    }
}

fn append_input(
    plan: &mut MediaPlan,
    spec: &OverlaySpec,
    index: usize,
) -> Result<RawVideoSpec, String> {
    let (_, frames) = spec.layout()?;
    let raw = RawVideoSpec::bgra(
        spec.width,
        spec.height,
        spec.fps,
        frames,
        RawVideoAlpha::Premultiplied,
    )?;
    if index != plan.inputs.len() {
        return Err("Native overlay must append the next media input".into());
    }
    plan.inputs.push(MediaInput::raw_video(raw));
    Ok(raw)
}

struct StopRendering {
    cancelled: Arc<AtomicBool>,
    source: RawVideoCancelHandle,
}
impl Drop for StopRendering {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        self.source.close();
    }
}

/// Append the raw input at `input_index`, bind its renderer, and execute the
/// caller's global graph. The plan's resource leases are also held by the
/// renderer so dropping an async caller cannot delete its image/font files.
pub async fn run_overlay_plan<R: ProgressReporterTrait>(
    plan: MediaPlan,
    spec: OverlaySpec,
    input_index: usize,
    progress: ProgressMode,
    context: impl Into<String>,
    reporter: Option<&R>,
) -> Result<MediaJobResult, String> {
    spec.layout()?;
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    if super::native_gpu::uses_vulkan(&plan) {
        return super::native_gpu::run_overlay_plan(plan, spec, progress, context, reporter).await;
    }
    let profile = log::log_enabled!(log::Level::Debug);
    let (output, stats) = run_overlay_inner(
        plan,
        spec,
        input_index,
        progress,
        context.into(),
        reporter,
        profile,
    )
    .await?;
    if profile {
        log::debug!("Native overlay stages (overlapping wall times): {stats:?}");
    }
    Ok(output)
}

#[cfg(test)]
pub async fn run_overlay_plan_profiled<R: ProgressReporterTrait>(
    plan: MediaPlan,
    spec: OverlaySpec,
    input_index: usize,
    progress: ProgressMode,
    context: impl Into<String>,
    reporter: Option<&R>,
) -> Result<(MediaJobResult, OverlayRunStats), String> {
    run_overlay_inner(
        plan,
        spec,
        input_index,
        progress,
        context.into(),
        reporter,
        true,
    )
    .await
}

async fn run_overlay_inner<R: ProgressReporterTrait>(
    mut plan: MediaPlan,
    spec: OverlaySpec,
    input_index: usize,
    progress: ProgressMode,
    context: String,
    reporter: Option<&R>,
    profile: bool,
) -> Result<(MediaJobResult, OverlayRunStats), String> {
    let (frame_bytes, frame_count) = spec.layout()?;
    let raw = append_input(&mut plan, &spec, input_index)?;
    let started = Instant::now();
    let (sender, input) = ffmpeg_utils::raw_video_channel(raw, 1)?;
    let pool_size = FRAME_BUFFERS.min(frame_count as usize);
    let (recycle, buffers) = mpsc::sync_channel(pool_size);
    let seed = recycle.clone();
    let feed_stats = profile.then(|| Arc::new(Mutex::new(RawVideoStats::default())));
    let mut input = input.with_recycle(recycle);
    if let Some(stats) = &feed_stats {
        input = input.with_stats(stats.clone());
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    let stop = StopRendering {
        cancelled: cancelled.clone(),
        source: input.cancel_handle(),
    };
    let resources = plan.resources.clone();
    let renderer = tokio::task::spawn_blocking(move || {
        let _resources = resources;
        let mut stats = OverlayRunStats::default();
        let result = (|| -> Result<(), String> {
            if cancelled.load(Ordering::Acquire) || sender.is_closed() {
                return Ok(());
            }
            let initialization = profile.then(Instant::now);
            let mut renderer = system_renderer(spec.width, spec.height, spec.input)?;
            renderer.set_profiling(profile);
            if let Some(initialization) = initialization {
                stats.initialization = initialization.elapsed();
            }
            let allocation = profile.then(Instant::now);
            for _ in 0..pool_size {
                seed.try_send(vec![0; frame_bytes])
                    .map_err(|_| "Cannot initialize BGRA buffer pool")?;
                stats.buffer_allocations += 1;
            }
            drop(seed);
            if let Some(allocation) = allocation {
                stats.buffer_allocation = allocation.elapsed();
            }
            for frame in 0..frame_count {
                let wait = profile.then(Instant::now);
                let received = loop {
                    if cancelled.load(Ordering::Acquire) || sender.is_closed() {
                        break None;
                    }
                    match buffers.recv_timeout(BUFFER_POLL) {
                        Ok(bytes) => break Some(bytes),
                        Err(RecvTimeoutError::Timeout) => continue,
                        Err(RecvTimeoutError::Disconnected) => break None,
                    }
                };
                if let Some(wait) = wait {
                    stats.buffer_wait += wait.elapsed();
                }
                let Some(mut bytes) = received else {
                    break;
                };
                if cancelled.load(Ordering::Acquire) || sender.is_closed() {
                    break;
                }
                renderer.frame_premultiplied_bgra_into(
                    spec.start + frame as f64 / spec.fps,
                    &mut bytes,
                )?;
                if cancelled.load(Ordering::Acquire) || sender.is_closed() {
                    break;
                }
                let send = profile.then(Instant::now);
                let sent = sender.blocking_send(Ok(RawVideoFrame::Frame(bytes)));
                if let Some(send) = send {
                    stats.queue_send_wait += send.elapsed();
                }
                if sent.is_err() {
                    break;
                }
            }
            stats.render = renderer.stats();
            if !cancelled.load(Ordering::Acquire) && !sender.is_closed() {
                let _ = sender.blocking_send(Ok(RawVideoFrame::Finished));
            }
            Ok(())
        })();
        if let Err(error) = &result {
            let _ = sender.blocking_send(Err(error.clone()));
        }
        result.map(|()| stats)
    });
    let result = run_native_media_with_sources(
        plan,
        MediaBindings {
            raw_video: vec![(input_index, input)],
            ..Default::default()
        },
        progress,
        context.clone(),
        reporter,
    )
    .await;
    let finished = Instant::now();
    drop(stop);
    let rendered = renderer
        .await
        .map_err(|error| format!("{context}: native renderer task failed: {error}"))?;
    let output = result?;
    let mut stats =
        rendered.map_err(|error| format!("{context}: native rendering failed: {error}"))?;
    if let Some(feed_stats) = feed_stats {
        stats.feed = feed_stats
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        stats.native_drain = stats
            .feed
            .finished_at
            .map(|end| finished.saturating_duration_since(end));
    }
    stats.wall = started.elapsed();
    Ok((output, stats))
}

/// The scheduler uses 1280x720-style ASS coordinates; the native image API uses
/// normalized 1920x1080 coordinates, regardless of output frame dimensions.
pub fn image_events(images: &[ImageDanmu], opacity: f64) -> Vec<native_overlay::ImageEvent> {
    let scale = 1080.0 / PLAY_RES_Y;
    images
        .iter()
        .map(|event| native_overlay::ImageEvent {
            path: event.image.path.clone(),
            start: event.start,
            end: event.end,
            from: ((event.from * scale) as f32, (event.top * scale) as f32),
            to: ((event.to * scale) as f32, (event.top * scale) as f32),
            width: (event.image.width * scale) as f32,
            height: (event.image.height * scale) as f32,
            opacity: opacity as f32,
        })
        .collect()
}

#[cfg(test)]
mod tests;
