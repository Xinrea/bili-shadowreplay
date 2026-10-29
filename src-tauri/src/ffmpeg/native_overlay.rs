//! Stream native-rendered premultiplied BGRA into FFmpeg without video-sized temp files.

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use tokio::{process::Command, sync::mpsc};

use crate::{
    danmu2ass::{ImageDanmu, PLAY_RES_Y},
    progress::progress_reporter::ProgressReporterTrait,
};

use super::runner::{
    run_command_with_frames, FrameFeedStats, JobOutput, ProgressMode, RawFrame, RawFrameReceiver,
};

const FRAME_BUFFERS: usize = 3;

/// The one way previews and burns create a renderer: with the system font
/// manager and no supplied font files, so a burn draws what was previewed.
pub fn system_renderer(
    width: u32,
    height: u32,
    input: native_overlay::RenderInput,
) -> Result<native_overlay::Renderer, String> {
    native_overlay::Renderer::new(width, height, input, Vec::new())
}

/// Stage measurements overlap across producer and FFmpeg; never sum them to
/// estimate wall time. `ffmpeg_drain` is post-stdin completion/muxing, not pure
/// encoder CPU time. Disabled for normal jobs unless debug logging is enabled.
#[derive(Debug, Default)]
pub struct OverlayRunStats {
    pub initialization: Duration,
    pub buffer_allocation: Duration,
    pub buffer_wait: Duration,
    pub queue_send_wait: Duration,
    pub render: native_overlay::RenderStats,
    pub feed: FrameFeedStats,
    pub wall: Duration,
    pub ffmpeg_drain: Option<Duration>,
    pub buffer_allocations: usize,
}

pub struct OverlaySpec {
    pub input: native_overlay::RenderInput,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    /// Renderer timeline offset; pipe timestamps always begin at zero.
    pub start: f64,
    pub duration: f64,
}

impl OverlaySpec {
    /// Bound allocations and iteration counts before creating a renderer.
    pub(super) fn layout(&self) -> Result<(usize, u64), String> {
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

fn supports_input_alpha_mode(help: &str) -> bool {
    help.lines().any(|line| {
        let mut parts = line.split_whitespace();
        parts.next() == Some("-alpha_mode")
            && parts.next() == Some("<int>")
            && parts.next().is_some_and(|flags| flags.contains('D'))
    })
}

#[derive(PartialEq, Eq)]
struct AlphaProbeKey {
    program: std::ffi::OsString,
    directory: Option<std::path::PathBuf>,
    environment: Vec<(std::ffi::OsString, Option<std::ffi::OsString>)>,
}

async fn input_alpha_mode_supported(command: &Command) -> Result<bool, String> {
    // One bounded cache entry; use the actual command's executable, including
    // the benchmark's explicit FFmpeg path, rather than probing a different PATH binary.
    static SUPPORT: tokio::sync::Mutex<Option<(AlphaProbeKey, bool)>> =
        tokio::sync::Mutex::const_new(None);
    let source = command.as_std();
    let key = AlphaProbeKey {
        program: source.get_program().to_owned(),
        directory: source.get_current_dir().map(ToOwned::to_owned),
        environment: source
            .get_envs()
            .map(|(key, value)| (key.to_owned(), value.map(ToOwned::to_owned)))
            .collect(),
    };
    let mut cached = SUPPORT.lock().await;
    if let Some((previous, supported)) = cached.as_ref() {
        if previous == &key {
            return Ok(*supported);
        }
    }
    let mut probe = Command::new(&key.program);
    if let Some(directory) = &key.directory {
        probe.current_dir(directory);
    }
    for (name, value) in &key.environment {
        if let Some(value) = value {
            probe.env(name, value);
        } else {
            probe.env_remove(name);
        }
    }
    probe.kill_on_drop(true);
    #[cfg(windows)]
    probe.creation_flags(0x08000000); // CREATE_NO_WINDOW, as in the FFmpeg factory.
    let output = tokio::time::timeout(
        Duration::from_secs(10),
        probe.args(["-hide_banner", "-h", "full"]).output(),
    )
    .await
    .map_err(|_| "Timed out querying FFmpeg alpha mode support".to_string())?
    .map_err(|error| format!("Cannot query FFmpeg alpha mode support: {error}"))?;
    if !output.status.success() {
        return Err("Cannot query FFmpeg alpha mode support".to_string());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stdout.contains("AVCodecContext AVOptions:")
        && !stderr.contains("AVCodecContext AVOptions:")
    {
        return Err(
            "FFmpeg help did not report decoder options; cannot determine alpha mode support"
                .to_string(),
        );
    }
    let supported = supports_input_alpha_mode(&stdout) || supports_input_alpha_mode(&stderr);
    if !supported {
        log::info!("FFmpeg has no input alpha_mode option; using explicit RGB premultiplied overlay semantics");
    }
    *cached = Some((key, supported));
    Ok(supported)
}

/// Append one raw premultiplied BGRA input. New FFmpeg releases use input alpha
/// metadata when negotiating conversions; declare it before `-i`. Older releases
/// lack this option, but still support explicit RGB overlay alpha=premultiplied.
pub async fn add_overlay_input(command: &mut Command, spec: &OverlaySpec) -> Result<(), String> {
    spec.layout()?;
    let supported = input_alpha_mode_supported(command).await?;
    append_overlay_input(command, spec, supported);
    Ok(())
}

fn append_overlay_input(command: &mut Command, spec: &OverlaySpec, alpha_metadata: bool) {
    command.args(["-f", "rawvideo", "-pixel_format", "bgra"]);
    if alpha_metadata {
        command.args(["-alpha_mode", "premultiplied"]);
    }
    command.args([
        "-video_size",
        &format!("{}x{}", spec.width, spec.height),
        "-framerate",
        &spec.fps.to_string(),
        "-i",
        "pipe:0",
    ]);
}

/// The blocking renderer checks this between frames. Dropping the receiver also
/// wakes blocking_send, including when cancellation occurs under backpressure.
struct StopRendering(Arc<AtomicBool>);

impl Drop for StopRendering {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

pub async fn run_overlay_command<R: ProgressReporterTrait>(
    command: Command,
    spec: OverlaySpec,
    progress: ProgressMode,
    context: impl Into<String>,
    reporter: Option<&R>,
) -> Result<JobOutput, String> {
    let profile = log::log_enabled!(log::Level::Debug);
    if profile {
        let (output, stats) =
            run_overlay_command_profiled(command, spec, progress, context, reporter).await?;
        log::debug!("Native overlay stages (overlapping wall times): {stats:?}");
        Ok(output)
    } else {
        run_overlay_inner(command, spec, progress, context.into(), reporter, false)
            .await
            .map(|(output, _)| output)
    }
}

/// Explicit profiling entry point for repeatable performance measurements.
pub async fn run_overlay_command_profiled<R: ProgressReporterTrait>(
    command: Command,
    spec: OverlaySpec,
    progress: ProgressMode,
    context: impl Into<String>,
    reporter: Option<&R>,
) -> Result<(JobOutput, OverlayRunStats), String> {
    run_overlay_inner(command, spec, progress, context.into(), reporter, true).await
}

async fn run_overlay_inner<R: ProgressReporterTrait>(
    command: Command,
    spec: OverlaySpec,
    progress: ProgressMode,
    context: String,
    reporter: Option<&R>,
    profile: bool,
) -> Result<(JobOutput, OverlayRunStats), String> {
    let (frame_bytes, frame_count) = spec.layout()?;
    let started = Instant::now();
    // Same bound as before: one queued, one being written, one being rendered.
    // Ownership circulates through a fixed pool instead of allocating/zeroing
    // width*height*4 bytes for every frame. No extra pixel copy is introduced.
    let (sender, receiver) = mpsc::channel(1);
    let pool_size = FRAME_BUFFERS.min(frame_count as usize);
    let (recycle, mut buffers) = mpsc::channel(pool_size);
    let seed = recycle.clone();
    let feed_stats = profile.then(|| Arc::new(Mutex::new(FrameFeedStats::default())));
    let stop = StopRendering(Arc::new(AtomicBool::new(false)));
    let cancelled = stop.0.clone();
    let renderer = tokio::task::spawn_blocking(move || {
        let mut stats = OverlayRunStats::default();
        let result = (|| -> Result<(), String> {
            if cancelled.load(Ordering::Acquire) || sender.is_closed() {
                return Ok(());
            }
            // Renderer need not be Send: construct and use on one blocking thread.
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
            // Only the runner may keep the return channel open while rendering.
            // On cancellation its drop must unblock buffers.blocking_recv().
            drop(seed);
            if let Some(allocation) = allocation {
                stats.buffer_allocation = allocation.elapsed();
            }
            for frame in 0..frame_count {
                if cancelled.load(Ordering::Acquire) || sender.is_closed() {
                    break;
                }
                let wait = profile.then(Instant::now);
                let received = buffers.blocking_recv();
                if let Some(wait) = wait {
                    stats.buffer_wait += wait.elapsed();
                }
                let Some(mut bytes) = received else { break };
                if cancelled.load(Ordering::Acquire) || sender.is_closed() {
                    break;
                }
                renderer.frame_premultiplied_bgra_into(
                    spec.start + frame as f64 / spec.fps,
                    &mut bytes,
                )?;
                if cancelled.load(Ordering::Acquire) {
                    break;
                }
                let send = profile.then(Instant::now);
                let sent = sender.blocking_send(Ok(RawFrame::Frame(bytes)));
                if let Some(send) = send {
                    stats.queue_send_wait += send.elapsed();
                }
                if sent.is_err() {
                    break;
                }
            }
            stats.render = renderer.stats();
            if !cancelled.load(Ordering::Acquire) && !sender.is_closed() {
                let _ = sender.blocking_send(Ok(RawFrame::Finished));
            }
            Ok(())
        })();
        if let Err(error) = &result {
            let _ = sender.blocking_send(Err(error.clone()));
        }
        result.map(|()| stats)
    });
    let result = run_command_with_frames(
        command,
        RawFrameReceiver {
            receiver,
            frame_bytes,
            frame_count,
            recycle: Some(recycle),
            stats: feed_stats.clone(),
        },
        progress,
        context.clone(),
        reporter,
    )
    .await;
    let ffmpeg_finished = Instant::now();
    drop(stop);
    // Receiver + return sender are dropped before waiting. This releases both
    // possible producer waits (free buffer and queued frame), also on early EOF.
    let rendered = renderer
        .await
        .map_err(|error| format!("{context}: native renderer task failed: {error}"))?;
    let output = result.map_err(|error| error.to_string())?;
    let mut stats =
        rendered.map_err(|error| format!("{context}: native rendering failed: {error}"))?;
    if let Some(feed_stats) = feed_stats {
        stats.feed = feed_stats.lock().unwrap_or_else(|e| e.into_inner()).clone();
        stats.ffmpeg_drain = stats
            .feed
            .feed_finished_at
            .map(|end| ffmpeg_finished.saturating_duration_since(end));
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
mod tests {
    use super::*;

    #[derive(Clone)]
    struct Reporter;
    #[async_trait::async_trait]
    impl ProgressReporterTrait for Reporter {
        async fn update(&self, _: &str) {}
        async fn finish(&self, _: bool, _: &str) {}
    }

    fn spec() -> OverlaySpec {
        OverlaySpec {
            input: native_overlay::RenderInput::default(),
            width: 1920,
            height: 1080,
            fps: 60.0,
            start: 0.0,
            duration: 1.01,
        }
    }

    #[tokio::test]
    async fn drop_signals_and_releases_a_backpressured_blocking_producer() {
        let stop = StopRendering(Arc::new(AtomicBool::new(false)));
        let cancelled = stop.0.clone();
        let (sender, receiver) = mpsc::channel(1);
        let (started, ready) = tokio::sync::oneshot::channel();
        let worker = tokio::task::spawn_blocking(move || {
            sender.blocking_send(vec![0u8; 4]).unwrap();
            started.send(()).unwrap();
            // No consumer: the next frame cannot fit until the receiver is
            // dropped, exactly as when the runner's write is backpressured.
            assert!(sender.blocking_send(vec![0u8; 4]).is_err());
            assert!(cancelled.load(Ordering::Acquire));
        });
        ready.await.unwrap();
        drop(stop);
        drop(receiver);
        tokio::time::timeout(std::time::Duration::from_secs(1), worker)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn dropping_return_channel_wakes_a_producer_waiting_for_a_buffer() {
        let stop = StopRendering(Arc::new(AtomicBool::new(false)));
        let cancelled = stop.0.clone();
        let (returns, mut buffers) = mpsc::channel::<Vec<u8>>(FRAME_BUFFERS);
        let (started, ready) = tokio::sync::oneshot::channel();
        let worker = tokio::task::spawn_blocking(move || {
            started.send(()).unwrap();
            assert!(buffers.blocking_recv().is_none());
            assert!(cancelled.load(Ordering::Acquire));
        });
        ready.await.unwrap();
        drop(stop);
        drop(returns);
        tokio::time::timeout(Duration::from_secs(1), worker)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    #[ignore = "requires FFmpeg"]
    async fn pooled_stream_profiles_real_ffmpeg_and_handles_early_eof() {
        for early in [false, true] {
            let mut spec = spec();
            spec.width = 64;
            spec.height = 64;
            spec.fps = 30.0;
            spec.duration = if early { 60.0 } else { 1.0 };
            let mut command = ffmpeg_utils::ffmpeg_command();
            command.args([
                "-v",
                "error",
                "-nostdin",
                "-f",
                "lavfi",
                "-i",
                "color=black:s=64x64:r=30:d=1",
            ]);
            add_overlay_input(&mut command, &spec).await.unwrap();
            command.args([
                "-filter_complex",
                "[0:v]format=bgra[base];[base][1:v]overlay=shortest=1:format=rgb:alpha=premultiplied[v]",
                "-map",
                "[v]",
                "-f",
                "null",
                "-",
            ]);
            let (_, stats) = tokio::time::timeout(
                Duration::from_secs(15),
                run_overlay_command_profiled::<Reporter>(
                    command,
                    spec,
                    ProgressMode::None,
                    "pooled stream test",
                    None,
                ),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(stats.buffer_allocations, FRAME_BUFFERS);
            assert_eq!(stats.feed.bytes, stats.feed.frames * 64 * 64 * 4);
            if early {
                assert!(stats.feed.frames < 1800);
            } else {
                assert_eq!(stats.feed.frames, 30);
                assert_eq!(stats.render.frames, 30);
                assert!(stats.ffmpeg_drain.is_some());
            }
        }
    }

    #[test]
    fn raw_input_is_premultiplied_bgra_and_rounds_up_the_last_partial_frame() {
        let spec = spec();
        assert_eq!(spec.layout().unwrap(), (1920 * 1080 * 4, 61));
        let mut command = Command::new("ffmpeg");
        append_overlay_input(&mut command, &spec, true);
        let args: Vec<_> = command
            .as_std()
            .get_args()
            .map(|arg| arg.to_str().unwrap())
            .collect();
        assert_eq!(
            args,
            [
                "-f",
                "rawvideo",
                "-pixel_format",
                "bgra",
                "-alpha_mode",
                "premultiplied",
                "-video_size",
                "1920x1080",
                "-framerate",
                "60",
                "-i",
                "pipe:0"
            ]
        );
        let mut legacy = Command::new("ffmpeg");
        append_overlay_input(&mut legacy, &spec, false);
        let args: Vec<_> = legacy
            .as_std()
            .get_args()
            .map(|arg| arg.to_str().unwrap())
            .collect();
        assert!(args
            .windows(2)
            .any(|pair| pair == ["-pixel_format", "bgra"]));
        assert!(!args.contains(&"-alpha_mode"));
    }

    #[test]
    fn alpha_mode_detection_requires_a_decoder_option_not_just_a_filter_option() {
        assert!(supports_input_alpha_mode(
            "  -alpha_mode <int> ED.V....... alpha mode"
        ));
        assert!(supports_input_alpha_mode(
            "  -alpha_mode <int> .D.V....... alpha mode"
        ));
        assert!(!supports_input_alpha_mode(
            "  -alpha_mode <int> ..FV....... alpha mode"
        ));
        assert!(!supports_input_alpha_mode(
            "  -alpha_mode <int> E..V....... alpha mode"
        ));
        assert!(!supports_input_alpha_mode(
            "  -alpha_modes <string> ..FV....... alpha modes"
        ));
        assert!(!supports_input_alpha_mode(""));
    }

    #[tokio::test]
    async fn failed_alpha_probe_is_not_treated_as_an_old_ffmpeg() {
        let directory = tempfile::tempdir().unwrap();
        let mut command = Command::new(directory.path().join("missing-ffmpeg"));
        let error = add_overlay_input(&mut command, &spec()).await.unwrap_err();
        assert!(
            error.contains("Cannot query FFmpeg alpha mode support"),
            "{error}"
        );
        assert_eq!(command.as_std().get_args().count(), 0);
    }

    #[tokio::test]
    #[ignore = "requires FFmpeg"]
    async fn premultiplied_stream_matches_straight_preview_on_coloured_backgrounds() {
        use native_overlay::{AssSource, ImageEvent, RenderInput, SourceKind};
        let directory = tempfile::tempdir().unwrap();
        let image = directory.path().join("translucent.png");
        let made = ffmpeg_utils::ffmpeg_command()
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=0xE03060@0.5:s=16x16:d=0.1,format=rgba",
                "-frames:v",
                "1",
                "-y",
            ])
            .arg(&image)
            .output()
            .await
            .unwrap();
        assert!(
            made.status.success(),
            "{}",
            String::from_utf8_lossy(&made.stderr)
        );
        let input = RenderInput {
            documents: vec![AssSource {
                kind: SourceKind::Subtitle,
                text: "[Script Info]\nScriptType: v4.00+\nPlayResX: 128\nPlayResY: 96\n\
                    [V4+ Styles]\n\
                    Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
                    Style: Default,Arial,28,&H600040E0,&H00FFFFFF,&H8000C040,&H90000000,0,0,0,0,100,100,0,0,1,2,3,5,0,0,0,1\n\
                    [Events]\n\
                    Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
                    Dialogue: 0,0:00:00.00,0:00:00.80,Default,,0,0,0,,Bg\n".into(),
            }],
            images: vec![ImageEvent {
                path: image,
                start: 0.2,
                end: 0.8,
                from: (0.0, 0.0),
                to: (0.0, 0.0),
                width: 480.0,
                height: 360.0,
                opacity: 0.5,
            }],
        };
        let (width, height, fps, frames) = (128, 96, 10.0, 10usize);
        let frame_bytes = width as usize * height as usize * 4;
        let mut preview = system_renderer(width, height, input.clone()).unwrap();
        let previews: Vec<_> = (0..frames)
            .map(|frame| preview.frame(frame as f64 / fps).unwrap())
            .collect();
        assert!(previews[5].chunks_exact(4).any(|p| p[3] > 0 && p[3] < 255));
        assert!(previews[9].chunks_exact(4).all(|p| p == [0, 0, 0, 0]));

        for background in [[0u8, 0, 0], [255, 255, 255], [16, 192, 224]] {
            let output = directory.path().join("composited.bgra");
            let spec = OverlaySpec {
                input: input.clone(),
                width,
                height,
                fps,
                start: 0.0,
                duration: 1.0,
            };
            let mut command = ffmpeg_utils::ffmpeg_command();
            command.args(["-v", "error", "-nostdin", "-f", "lavfi", "-i"]);
            command.arg(format!(
                "color=c=0x{:02X}{:02X}{:02X}:s={width}x{height}:r={fps}:d=1,format=bgra",
                background[0], background[1], background[2],
            ));
            // Use the actual executable's capability. Omitting metadata on new
            // FFmpeg is not an old-version simulation: it may auto-premultiply
            // pixels again during filter negotiation.
            add_overlay_input(&mut command, &spec).await.unwrap();
            command.args([
                "-filter_complex",
                "[0:v]format=bgra[base];[base][1:v]overlay=shortest=1:format=rgb:alpha=premultiplied[v]",
                "-map", "[v]", "-c:v", "rawvideo", "-pix_fmt", "bgra", "-f", "rawvideo", "-y",
            ]).arg(&output);
            run_overlay_command(
                command,
                spec,
                ProgressMode::None,
                "premultiplied alpha test",
                None::<&Reporter>,
            )
            .await
            .unwrap();
            let actual = tokio::fs::read(output).await.unwrap();
            assert_eq!(actual.len(), frame_bytes * frames);
            for (frame, (drawn, composited)) in previews
                .iter()
                .zip(actual.chunks_exact(frame_bytes))
                .enumerate()
            {
                for (pixel, (rgba, bgra)) in drawn
                    .chunks_exact(4)
                    .zip(composited.chunks_exact(4))
                    .enumerate()
                {
                    assert_eq!(bgra[3], 255);
                    for channel in 0..3 {
                        let alpha = u32::from(rgba[3]);
                        let expected = (u32::from(rgba[channel]) * alpha
                            + u32::from(background[channel]) * (255 - alpha)
                            + 127)
                            / 255;
                        let delta = (i32::from(bgra[2 - channel]) - expected as i32).abs();
                        assert!(delta <= 2,
                            "background={background:?} frame={frame} pixel={pixel} channel={channel}: {rgba:?} -> {bgra:?}, expected {expected}");
                    }
                }
            }
        }
    }

    #[tokio::test]
    async fn rejects_unbounded_work_before_appending_input() {
        for invalid in [f64::NAN, f64::INFINITY, -1.0, 0.0, 241.0] {
            let mut spec = spec();
            spec.fps = invalid;
            let mut command = Command::new("ffmpeg");
            assert!(add_overlay_input(&mut command, &spec).await.is_err());
            assert_eq!(command.as_std().get_args().count(), 0);
        }
        let mut spec = spec();
        spec.width = u32::MAX;
        assert!(spec.layout().is_err());
        spec.width = 1920;
        spec.duration = f64::MAX;
        assert!(spec.layout().is_err());
        spec.duration = 1.0;
        spec.start = f64::NAN;
        assert!(spec.layout().is_err());
    }

    #[test]
    fn image_coordinates_and_sizes_share_the_scheduler_scale() {
        let events = image_events(
            &[ImageDanmu {
                image: crate::danmu2ass::DanmuImage {
                    path: "emoji.png".into(),
                    width: 54.0,
                    height: 36.0,
                },
                start: 1.0,
                end: 9.0,
                from: 1280.0,
                to: -54.0,
                top: 36.0,
            }],
            0.8,
        );
        let event = &events[0];
        assert_eq!(event.from, (1920.0, 54.0));
        assert_eq!(event.to, (-81.0, 54.0));
        assert_eq!((event.width, event.height), (81.0, 54.0));
        assert_eq!((event.start, event.end), (1.0, 9.0));
        assert_eq!(event.opacity, 0.8);
    }
}
