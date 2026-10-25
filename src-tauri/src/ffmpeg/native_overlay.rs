//! Stream native-rendered straight RGBA into FFmpeg without video-sized temp files.

use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use tokio::{process::Command, sync::mpsc};

use crate::{
    danmu2ass::{ImageDanmu, PLAY_RES_Y},
    progress::progress_reporter::ProgressReporterTrait,
};

use super::runner::{run_command_with_frames, JobOutput, ProgressMode, RawFrame, RawFrameReceiver};

pub struct OverlaySpec {
    pub input: native_overlay::RenderInput,
    pub font_paths: Vec<PathBuf>,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    /// Renderer timeline offset; pipe timestamps always begin at zero.
    pub start: f64,
    pub duration: f64,
}

impl OverlaySpec {
    /// Bound allocations and iteration counts before creating a renderer.
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

/// Append one raw RGBA input. Callers own input numbering and compositing.
pub fn add_overlay_input(command: &mut Command, spec: &OverlaySpec) -> Result<(), String> {
    spec.layout()?;
    command.args([
        "-f",
        "rawvideo",
        "-pixel_format",
        "rgba",
        "-video_size",
        &format!("{}x{}", spec.width, spec.height),
        "-framerate",
        &spec.fps.to_string(),
        "-i",
        "pipe:0",
    ]);
    Ok(())
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
    let context = context.into();
    let (frame_bytes, frame_count) = spec.layout()?;
    // At most one queued frame, one being written, and one being rendered.
    let (sender, receiver) = mpsc::channel(1);
    let stop = StopRendering(Arc::new(AtomicBool::new(false)));
    let cancelled = stop.0.clone();
    let renderer = tokio::task::spawn_blocking(move || {
        let result = (|| -> Result<(), String> {
            if cancelled.load(Ordering::Acquire) || sender.is_closed() {
                return Ok(());
            }
            // Renderer need not be Send: both its construction and use are
            // confined to this single blocking thread.
            let mut renderer = native_overlay::Renderer::new(
                spec.width,
                spec.height,
                spec.input,
                spec.font_paths,
            )?;
            for frame in 0..frame_count {
                if cancelled.load(Ordering::Acquire) || sender.is_closed() {
                    return Ok(());
                }
                let bytes = renderer.frame(spec.start + frame as f64 / spec.fps)?;
                if bytes.len() != frame_bytes {
                    return Err("Native renderer returned an invalid RGBA frame size".into());
                }
                if cancelled.load(Ordering::Acquire)
                    || sender.blocking_send(Ok(RawFrame::Frame(bytes))).is_err()
                {
                    return Ok(());
                }
            }
            let _ = sender.blocking_send(Ok(RawFrame::Finished));
            Ok(())
        })();
        if let Err(error) = &result {
            let _ = sender.blocking_send(Err(error.clone()));
        }
        result
    });
    let result = run_command_with_frames(
        command,
        RawFrameReceiver {
            receiver,
            frame_bytes,
            frame_count,
        },
        progress,
        context.clone(),
        reporter,
    )
    .await;
    drop(stop);
    // Even a successful early FFmpeg EOF must not conceal a rendering error
    // or panic. The runner has dropped its receiver, so this cannot be stuck
    // waiting to send another frame. Cancellation instead drops the guard
    // and runner futures immediately; no detached async writer owns the child.
    let rendered = renderer
        .await
        .map_err(|error| format!("{context}: native renderer task failed: {error}"))?;
    let output = result.map_err(|error| error.to_string())?;
    rendered.map_err(|error| format!("{context}: native rendering failed: {error}"))?;
    Ok(output)
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

    fn spec() -> OverlaySpec {
        OverlaySpec {
            input: native_overlay::RenderInput::default(),
            font_paths: Vec::new(),
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

    #[test]
    fn raw_input_is_rgba_and_rounds_up_the_last_partial_frame() {
        let spec = spec();
        assert_eq!(spec.layout().unwrap(), (1920 * 1080 * 4, 61));
        let mut command = Command::new("ffmpeg");
        add_overlay_input(&mut command, &spec).unwrap();
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
                "rgba",
                "-video_size",
                "1920x1080",
                "-framerate",
                "60",
                "-i",
                "pipe:0"
            ]
        );
    }

    #[test]
    fn rejects_unbounded_work_before_appending_input() {
        for invalid in [f64::NAN, f64::INFINITY, -1.0, 0.0, 241.0] {
            let mut spec = spec();
            spec.fps = invalid;
            let mut command = Command::new("ffmpeg");
            assert!(add_overlay_input(&mut command, &spec).is_err());
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
