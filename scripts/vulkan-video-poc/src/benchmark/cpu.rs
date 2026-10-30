//! Production-style raster producer and native FFmpeg graph for the benchmark.
//!
//! Keep the three recyclable pixel buffers and one-frame raw queue used by
//! `src-tauri/src/ffmpeg/native_overlay.rs`. The native raw reader retains its
//! normal Vec-to-AVFrame copy, and renderer/media initialization can overlap.

use super::{BenchmarkSpec, BACKGROUND};
use ffmpeg_utils::{
    execute_with_sources, raw_video_channel, AvOptions, ColorSource, FilterGraphPlan,
    FrameRatePolicy, GraphInput, GraphOutput, MediaBindings, MediaInput, MediaKind, MediaOutput,
    MediaPlan, RawVideoAlpha, RawVideoCancelHandle, RawVideoFrame, RawVideoSender, RawVideoSpec,
    RawVideoStats, StreamMode, StreamRoute, StreamSelector, StreamSource, VideoEncodeOptions,
};
use native_overlay::{RenderInput, RenderStats, Renderer};
use serde_json::{json, Value};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

const FRAME_BUFFERS: usize = 3;
const BUFFER_POLL: Duration = Duration::from_millis(25);
const ENCODER: &str = "hevc_nvenc";

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

#[derive(Default)]
struct ProducerStats {
    initialization: Duration,
    buffer_allocation: Duration,
    buffer_wait: Duration,
    queue_send_wait: Duration,
    render_api: Duration,
    wall: Duration,
    buffer_allocations: usize,
    render: RenderStats,
}

struct Producer {
    width: u32,
    height: u32,
    fps: u32,
    frames: u32,
    frame_bytes: usize,
    pool_size: usize,
    input: RenderInput,
    sender: RawVideoSender,
    seed: SyncSender<Vec<u8>>,
    buffers: Receiver<Vec<u8>>,
    cancelled: Arc<AtomicBool>,
}

impl Producer {
    fn run(self) -> Result<ProducerStats, String> {
        let started = Instant::now();
        let mut stats = ProducerStats::default();
        let result = (|| -> Result<(), String> {
            if self.cancelled.load(Ordering::Acquire) || self.sender.is_closed() {
                return Ok(());
            }
            let initialization = Instant::now();
            let mut renderer = Renderer::new(self.width, self.height, self.input, Vec::new())?;
            renderer.set_profiling(true);
            stats.initialization = initialization.elapsed();
            let allocation = Instant::now();
            for _ in 0..self.pool_size {
                self.seed
                    .try_send(vec![0; self.frame_bytes])
                    .map_err(|_| "Cannot initialize benchmark BGRA buffer pool".to_string())?;
                stats.buffer_allocations += 1;
            }
            drop(self.seed);
            stats.buffer_allocation = allocation.elapsed();
            for frame in 0..self.frames {
                let wait = Instant::now();
                let bytes = loop {
                    if self.cancelled.load(Ordering::Acquire) || self.sender.is_closed() {
                        break None;
                    }
                    match self.buffers.recv_timeout(BUFFER_POLL) {
                        Ok(bytes) => break Some(bytes),
                        Err(RecvTimeoutError::Timeout) => continue,
                        Err(RecvTimeoutError::Disconnected) => break None,
                    }
                };
                stats.buffer_wait += wait.elapsed();
                let Some(mut bytes) = bytes else { break };
                if self.cancelled.load(Ordering::Acquire) || self.sender.is_closed() {
                    break;
                }
                let render = Instant::now();
                renderer.frame_premultiplied_bgra_into(
                    f64::from(frame) / f64::from(self.fps),
                    &mut bytes,
                )?;
                stats.render_api += render.elapsed();
                if self.cancelled.load(Ordering::Acquire) || self.sender.is_closed() {
                    break;
                }
                let send = Instant::now();
                let sent = self.sender.blocking_send(Ok(RawVideoFrame::Frame(bytes)));
                stats.queue_send_wait += send.elapsed();
                if sent.is_err() {
                    break;
                }
            }
            stats.render = renderer.stats();
            if !self.cancelled.load(Ordering::Acquire) && !self.sender.is_closed() {
                let _ = self.sender.blocking_send(Ok(RawVideoFrame::Finished));
            }
            Ok(())
        })();
        if let Err(error) = &result {
            let _ = self.sender.blocking_send(Err(error.clone()));
        }
        stats.wall = started.elapsed();
        result.map(|()| stats)
    }
}

fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

/// Render and encode the generated scene through the production CPU pathway.
/// This measures the composition/encoding subpipeline: there is no file decode
/// or audio work because the background is the same generated solid colour as
/// the GPU benchmark, rather than an existing project video.
pub fn run(spec: &BenchmarkSpec, input: RenderInput, output: &Path) -> Result<Value, String> {
    let started = Instant::now();
    let raw = RawVideoSpec::bgra(
        spec.width,
        spec.height,
        f64::from(spec.fps),
        u64::from(spec.frames),
        RawVideoAlpha::Premultiplied,
    )?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|error| format!("Cannot create CPU benchmark runtime: {error}"))?;
    let (sender, source) = raw_video_channel(raw, 1)?;
    let pool_size = FRAME_BUFFERS.min(spec.frames as usize);
    let (recycle, buffers) = mpsc::sync_channel(pool_size);
    let feed_stats = Arc::new(Mutex::new(RawVideoStats::default()));
    let stop = StopRendering {
        cancelled: Arc::new(AtomicBool::new(false)),
        source: source.cancel_handle(),
    };
    let source = source
        .with_recycle(recycle.clone())
        .with_stats(feed_stats.clone());
    let [red, green, blue, _] = BACKGROUND;
    let mut background = MediaInput::generated_video(ColorSource {
        color: format!("0x{red:02X}{green:02X}{blue:02X}"),
        width: spec.width,
        height: spec.height,
        frame_rate_num: spec.fps,
        frame_rate_den: 1,
    });
    background.duration = Some(f64::from(spec.frames) / f64::from(spec.fps));
    let plan = MediaPlan {
        inputs: vec![background, MediaInput::raw_video(raw)],
        outputs: vec![MediaOutput::file(output)],
        routes: vec![StreamRoute {
            source: StreamSource::Filter {
                label: "outv".into(),
            },
            output: 0,
            required: true,
            mode: StreamMode::Video(VideoEncodeOptions {
                encoder: ENCODER.into(),
                frame_rate: FrameRatePolicy::Constant {
                    num: spec.fps,
                    den: 1,
                },
                bit_rate: Some(spec.bit_rate),
                gop: Some(spec.gop),
                tag: Some("hvc1".into()),
                options: AvOptions::from([
                    ("preset".into(), "p1".into()),
                    ("rc".into(), "vbr".into()),
                ]),
                ..Default::default()
            }),
        }],
        filter_graph: Some(FilterGraphPlan {
            spec: format!(
                "[0:v]scale={width}:{height}:force_original_aspect_ratio=decrease,\
                 pad={width}:{height}:(ow-iw)/2:(oh-ih)/2,fps={fps},format=bgra[base];\
                 [base][1:v]overlay=shortest=1:format=rgb:alpha=premultiplied[composited];\
                 [composited]format=nv12[outv]",
                width = spec.width,
                height = spec.height,
                fps = spec.fps,
            ),
            inputs: (0..2)
                .map(|index| GraphInput {
                    input: index,
                    selector: StreamSelector::FirstVideo,
                    label: format!("{index}:v"),
                    required: true,
                })
                .collect(),
            outputs: vec![GraphOutput {
                label: "outv".into(),
                kind: MediaKind::Video,
            }],
        }),
        ..Default::default()
    };
    let producer = Producer {
        width: spec.width,
        height: spec.height,
        fps: spec.fps,
        frames: spec.frames,
        frame_bytes: spec.width as usize * spec.height as usize * 4,
        pool_size,
        input,
        sender,
        seed: recycle,
        buffers,
        cancelled: stop.cancelled.clone(),
    };
    let worker = thread::Builder::new()
        .name("benchmark-raster-producer".into())
        .spawn(move || producer.run())
        .map_err(|error| format!("Cannot start CPU benchmark renderer: {error}"))?;
    let setup = started.elapsed();
    let media_started = Instant::now();
    let media = runtime.block_on(execute_with_sources(
        plan,
        MediaBindings {
            raw_video: vec![(1, source)],
        },
        None,
    ));
    let media_finished = Instant::now();
    // Closing both signals before joining is essential: a graph/encoder error
    // can occur while the producer waits for its next recyclable frame buffer.
    drop(stop);
    let rendered = worker
        .join()
        .map_err(|_| "CPU benchmark renderer thread panicked".to_string())?;
    drop(runtime);
    let rendered = match (media.as_ref(), rendered) {
        (Err(media), Err(renderer)) => {
            return Err(format!(
                "CPU benchmark media failed: {media}; renderer: {renderer}"
            ));
        }
        (_, result) => result?,
    };
    let media = media.map_err(|error| format!("CPU benchmark media failed: {error}"))?;
    let feed = feed_stats
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    if media.video_frames != u64::from(spec.frames)
        || rendered.render.frames != u64::from(spec.frames)
        || feed.frames != u64::from(spec.frames)
    {
        return Err(format!(
            "CPU benchmark frame mismatch: expected {}, rendered {}, supplied {}, output {}",
            spec.frames, rendered.render.frames, feed.frames, media.video_frames
        ));
    }
    let total = started.elapsed();
    Ok(json!({
        "backend": "cpu",
        "encoder": ENCODER,
        "width": spec.width, "height": spec.height,
        "fps_num": spec.fps, "fps_den": 1, "frames": spec.frames,
        "output": output,
        "rendered_frames": rendered.render.frames,
        "output_video_frames": media.video_frames,
        "total_ms": milliseconds(total),
        "initialization_ms": milliseconds(setup),
        "initialization_scope": "main-thread runtime/channel/plan setup; renderer and media initialize concurrently",
        "media_wall_ms": milliseconds(media_finished.duration_since(media_started)),
        "renderer_initialization_ms": milliseconds(rendered.initialization),
        "renderer_thread_wall_ms": milliseconds(rendered.wall),
        "renderer_frame_api_ms": milliseconds(rendered.render_api),
        "renderer_pixel_readback_ms": milliseconds(rendered.render.readback),
        "renderer_composition_ms": milliseconds(rendered.render.composition),
        "renderer_text_layout_ms": milliseconds(rendered.render.text_layout),
        "renderer_text_raster_ms": milliseconds(rendered.render.text_raster),
        "renderer_image_prepare_ms": milliseconds(rendered.render.image_prepare),
        "buffer_allocation_ms": milliseconds(rendered.buffer_allocation),
        "buffer_allocations": rendered.buffer_allocations,
        "producer_buffer_wait_ms": milliseconds(rendered.buffer_wait),
        "producer_queue_send_wait_ms": milliseconds(rendered.queue_send_wait),
        "raw_video_copy_ms": milliseconds(feed.copy),
        "raw_video_queue_wait_ms": milliseconds(feed.queue_wait),
        "raw_video_frames": feed.frames,
        "raw_video_bytes": feed.bytes,
        "native_drain_ms": feed.finished_at.map(|finished| milliseconds(media_finished.saturating_duration_since(finished))),
        "encoder_settings": {
            "preset": "p1", "rc": "vbr", "bit_rate": spec.bit_rate,
            "gop": spec.gop, "tag": "hvc1", "b_frames": "encoder default",
        },
        "scope": "production-style CPU render/composite/convert/encode subpipeline with a generated background; no file decode or audio",
        "timing_note": "producer, frame copy and media stage timings overlap; total_ms includes graph/encoder drain and producer join",
    }))
}
