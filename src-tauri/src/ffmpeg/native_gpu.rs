//! Skia Vulkan composition bound to the existing native MediaPlan worker.
//!
//! The worker retains decoding, frame scheduling, audio, encoding and muxing.
//! This adapter only supplies a GPU frame processor for one video route.

use ffmpeg_utils::profiling as perf;
use std::sync::{atomic::AtomicBool, Arc};

use ffmpeg_utils::{
    vulkan_video::{
        EncoderFrameHandoff, HardwareEncoderFrames, VulkanCompositor, VulkanImageState,
        VulkanRenderOptions, VulkanSubmissionMode,
    },
    MediaBindings, MediaPlan, MediaResourceLease, StreamMode, VideoEncodeOptions, VideoFrame,
    VideoFrameInfo, VideoFrameProcessor,
};
use native_overlay::{ExternalVulkanImage, NativeVulkanDevice, VulkanProcAddress, VulkanRenderer};

use super::{
    native_overlay::OverlaySpec,
    runner::{run_native_media_with_sources, ProgressMode},
};
use crate::progress::progress_reporter::ProgressReporterTrait;

pub(super) fn uses_vulkan(plan: &MediaPlan) -> bool {
    plan.routes
        .iter()
        .any(|route| matches!(&route.mode, StreamMode::Video(options) if options.frame_processor))
}

fn bind_overlay(plan: &mut MediaPlan, spec: OverlaySpec) -> Result<MediaBindings, String> {
    bind_overlay_with_mode(plan, spec, VulkanSubmissionMode::Asynchronous)
}

fn bind_overlay_with_mode(
    plan: &mut MediaPlan,
    spec: OverlaySpec,
    mode: VulkanSubmissionMode,
) -> Result<MediaBindings, String> {
    bind_overlay_with_handoff(plan, spec, mode, EncoderFrameHandoff::default())
}

fn bind_overlay_with_handoff(
    plan: &mut MediaPlan,
    spec: OverlaySpec,
    mode: VulkanSubmissionMode,
    handoff: EncoderFrameHandoff,
) -> Result<MediaBindings, String> {
    let mut matching =
        plan.routes
            .iter()
            .enumerate()
            .filter_map(|(index, route)| match &route.mode {
                StreamMode::Video(options) if options.frame_processor => {
                    Some((index, options.clone()))
                }
                _ => None,
            });
    let (index, options) = matching.next().ok_or("Missing Vulkan video route")?;
    if matching.next().is_some() {
        return Err("A native overlay job requires exactly one Vulkan video route".into());
    }
    drop(matching);
    if let StreamMode::Video(options) = &mut plan.routes[index].mode {
        options.frame_processor = true;
    }
    let resources = plan.resources.clone();
    Ok(MediaBindings {
        video_processors: vec![(
            index,
            Box::new(move |info, cancelled| {
                VulkanOverlayProcessor::new(
                    info, spec, options, resources, cancelled, mode, handoff,
                )
                .map(|processor| Box::new(processor) as Box<dyn VideoFrameProcessor>)
            }),
        )],
        ..Default::default()
    })
}

pub(super) async fn run_overlay_plan<R: ProgressReporterTrait>(
    mut plan: MediaPlan,
    spec: OverlaySpec,
    progress: ProgressMode,
    context: impl Into<String>,
    reporter: Option<&R>,
) -> Result<ffmpeg_utils::MediaJobResult, String> {
    let bindings = bind_overlay(&mut plan, spec)?;
    run_native_media_with_sources(plan, bindings, progress, context, reporter).await
}

/// Same production binding, with an explicit collector isolated from capability probes.
#[cfg(test)]
pub(crate) async fn run_overlay_plan_measured<R: ProgressReporterTrait>(
    mut plan: MediaPlan,
    spec: OverlaySpec,
    profiler: perf::MediaProfiler,
    reporter: Option<&R>,
) -> Result<ffmpeg_utils::MediaJobResult, String> {
    let mut bindings = bind_overlay(&mut plan, spec)?;
    bindings.profiler = Some(profiler);
    run_native_media_with_sources(
        plan,
        bindings,
        ProgressMode::None,
        "Measured GPU export",
        reporter,
    )
    .await
}

/// Per-job handoff baseline for controlled validation; production uses the
/// asynchronous default. Skia/Vulkan submission remains asynchronous in both.
#[cfg(test)]
pub(crate) async fn run_overlay_plan_with_handoff<R: ProgressReporterTrait>(
    mut plan: MediaPlan,
    spec: OverlaySpec,
    handoff: EncoderFrameHandoff,
    profiler: Option<perf::MediaProfiler>,
    reporter: Option<&R>,
) -> Result<ffmpeg_utils::MediaJobResult, String> {
    let mut bindings =
        bind_overlay_with_handoff(&mut plan, spec, VulkanSubmissionMode::Asynchronous, handoff)?;
    bindings.profiler = profiler;
    run_native_media_with_sources(
        plan,
        bindings,
        ProgressMode::None,
        "GPU encoder handoff comparison",
        reporter,
    )
    .await
}

// Declaration order keeps all cached Skia surfaces alive only while the
// compositor owns their borrowed device, and all assets until both are closed.
struct VulkanOverlayProcessor {
    renderer: VulkanRenderer,
    encoder_frames: HardwareEncoderFrames,
    compositor: VulkanCompositor,
    timeline_start: f64,
    seconds_per_tick: f64,
    _resources: Vec<MediaResourceLease>,
}

impl VulkanOverlayProcessor {
    fn new(
        info: VideoFrameInfo,
        spec: OverlaySpec,
        options: VideoEncodeOptions,
        resources: Vec<MediaResourceLease>,
        cancelled: Arc<AtomicBool>,
        mode: VulkanSubmissionMode,
        handoff: EncoderFrameHandoff,
    ) -> Result<Self, String> {
        if info.width != spec.width || info.height != spec.height {
            return Err("GPU video and overlay dimensions differ".into());
        }
        if info.frame_rate.numerator() <= 0
            || info.frame_rate.denominator() <= 0
            || info.time_base.numerator() <= 0
            || info.time_base.denominator() <= 0
            || info.time_base != info.frame_rate.invert()
            || !spec.start.is_finite()
            || spec.start < 0.0
        {
            return Err("Invalid GPU video or overlay timeline".into());
        }
        let mut compositor = VulkanCompositor::new_with_mode(
            VulkanRenderOptions {
                width: info.width,
                height: info.height,
                fps_num: info.frame_rate.numerator() as u32,
                fps_den: info.frame_rate.denominator() as u32,
                device_selector: "0".into(),
            },
            true,
            cancelled,
            mode,
        )?;
        let encoder_frames =
            HardwareEncoderFrames::new_with_handoff(&mut compositor, &options.encoder, handoff)?;
        let device = compositor.device();
        let handles = device.info();
        let native = NativeVulkanDevice {
            instance: handles.instance,
            physical_device: handles.physical_device,
            device: handles.device,
            queue: handles.queue,
            queue_family: handles.queue_family,
            instance_extensions: handles.instance_extensions,
            device_extensions: handles.device_extensions,
            max_api_version: handles.max_api_version,
        };
        let mut renderer = unsafe {
            VulkanRenderer::new(
                info.width,
                info.height,
                spec.input,
                Vec::new(),
                &native,
                |request| match request {
                    VulkanProcAddress::Instance { handle, name } => {
                        device.get_proc_addr_for(false, handle, name)
                    }
                    VulkanProcAddress::Device { handle, name } => {
                        device.get_proc_addr_for(true, handle, name)
                    }
                },
            )
        }?;
        renderer.set_async_submission(mode == VulkanSubmissionMode::Asynchronous);
        renderer.set_profiling(perf::enabled() || log::log_enabled!(log::Level::Debug));
        Ok(Self {
            renderer,
            encoder_frames,
            compositor,
            timeline_start: spec.start,
            seconds_per_tick: info.time_base.numerator() as f64
                / info.time_base.denominator() as f64,
            _resources: resources,
        })
    }
}

impl VideoFrameProcessor for VulkanOverlayProcessor {
    fn output_template(&self) -> Result<VideoFrame, String> {
        self.encoder_frames.output_template()
    }

    fn process(&mut self, frame: VideoFrame) -> Result<Vec<VideoFrame>, String> {
        let pts = frame.pts().ok_or("GPU video frame has no timestamp")?;
        let time = self.timeline_start + pts as f64 * self.seconds_per_tick;
        let _frame = perf::span("process_frame");
        let lease = perf::measure("acquire_frame", || self.compositor.acquire_frame(pts))?;
        let image = lease.image();
        let target = ExternalVulkanImage {
            image: image.image,
            memory: image.memory,
            memory_offset: image.memory_offset,
            memory_size: image.memory_size,
            layout: image.layout,
            format: image.format,
            usage: image.usage,
            queue_family: image.queue_family,
            sharing_mode: image.sharing_mode,
        };
        let before = perf::enabled().then(|| self.renderer.stats());
        let state = perf::measure("skia_render", || unsafe {
            self.renderer.render_into(time, &target, [0, 0, 0, 0])
        })?;
        if let Some(before) = before {
            let after = self.renderer.stats();
            for (name, elapsed) in [
                ("skia_scene_cpu", after.composition - before.composition),
                (
                    "skia_flush_submit_cpu",
                    after.submit_and_wait - before.submit_and_wait,
                ),
                (
                    "skia_text_layout_cpu",
                    after.scene.text_layout - before.scene.text_layout,
                ),
                (
                    "skia_text_sprite_cpu",
                    after.scene.text_raster - before.scene.text_raster,
                ),
                (
                    "skia_image_prepare_cpu",
                    after.scene.image_prepare - before.scene.image_prepare,
                ),
            ] {
                perf::observe(name, elapsed);
            }
        }
        let frames = unsafe {
            self.compositor.submit_frame(
                lease,
                VulkanImageState {
                    layout: state.layout,
                    queue_family: state.queue_family,
                },
                Some(&frame),
            )
        }?;
        perf::measure("encoder_frame_bridge", || {
            self.encoder_frames.process(frames)
        })
    }

    fn flush(&mut self) -> Result<Vec<VideoFrame>, String> {
        let frames = self.compositor.flush()?;
        perf::measure("encoder_frame_bridge", || {
            let mut ready = self.encoder_frames.process(frames)?;
            ready.extend(self.encoder_frames.flush()?);
            Ok(ready)
        })
    }
}

impl Drop for VulkanOverlayProcessor {
    fn drop(&mut self) {
        // Skia borrows the compositor's device/images. Its fields must remain
        // alive until queued consumers finish, including cancelled/error jobs.
        if let Err(error) = self.compositor.synchronize() {
            log::error!("Vulkan export synchronization during cleanup failed: {error}");
        }
        perf::annotate("vulkan_submission", || {
            format!("{:?}", self.compositor.submission_mode())
        });
        perf::annotate("compositor_counters", || {
            let stats = self.compositor.stats();
            format!(
                "submitted={}, output={}, uploaded={}, srgb={}, in_flight_peak={}",
                stats.submitted_frames,
                stats.output_frames,
                stats.uploaded_frames,
                stats.srgb_frames,
                stats.in_flight_peak
            )
        });
        #[cfg(test)]
        eprintln!(
            "native_gpu_backend=vulkan mode={:?} renderer={:?} compositor={:?}",
            self.compositor.submission_mode(),
            self.renderer.stats(),
            self.compositor.stats()
        );
        log::debug!(
            "Vulkan export mode={:?}, renderer={:?}, compositor={:?}",
            self.compositor.submission_mode(),
            self.renderer.stats(),
            self.compositor.stats()
        );
    }
}

/// Probe the complete renderer/compositor/encoder route before selecting it.
/// Failures are cached per codec and leave the existing CPU selection available.
#[cfg(any(target_os = "windows", target_os = "macos"))]
pub(super) async fn probe_encoder(encoder: &'static str) -> bool {
    static H264: tokio::sync::OnceCell<bool> = tokio::sync::OnceCell::const_new();
    static HEVC: tokio::sync::OnceCell<bool> = tokio::sync::OnceCell::const_new();
    let cache = if encoder.starts_with("hevc_") {
        &HEVC
    } else {
        &H264
    };
    *cache
        .get_or_init(|| async {
            match probe(encoder).await {
                Ok(()) => {
                    log::info!("Vulkan GPU composition and encoder probe passed: {encoder}");
                    true
                }
                Err(error) => {
                    log::info!("Vulkan export unavailable for {encoder}: {error}");
                    false
                }
            }
        })
        .await
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
async fn probe(encoder: &str) -> Result<(), String> {
    probe_with_mode(encoder, VulkanSubmissionMode::Asynchronous).await
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
async fn probe_with_mode(encoder: &str, mode: VulkanSubmissionMode) -> Result<(), String> {
    let directory = Arc::new(tempfile::tempdir().map_err(|error| error.to_string())?);
    let output = directory.path().join("probe.mp4");
    let scene = probe_scene(directory.path())?;
    let mut plan = probe_plan(encoder, &output)?;
    // Cancelling the awaiting probe must not remove assets or its output
    // directory while the native worker still owns GPU work and files.
    plan.resources
        .push(MediaResourceLease::new(directory.clone()));
    let bindings = bind_overlay_with_mode(
        &mut plan,
        OverlaySpec {
            input: scene,
            width: 320,
            height: 240,
            fps: 30.0,
            start: 0.0,
            duration: 0.1,
        },
        mode,
    )?;
    let result = ffmpeg_utils::execute_with_sources(plan, bindings, None)
        .await
        .map_err(|error| error.to_string())?;
    if result.video_frames != 3 {
        return Err(format!(
            "Vulkan probe encoded {} frames instead of 3",
            result.video_frames
        ));
    }
    // Some drivers/filter failures can still emit the expected number of black
    // frames. Verify the finalized, compressed probe, never a production GPU
    // frame or an image mapped from its hardware pool.
    // The bounded verification task owns the directory through decoder I/O,
    // even if the capability-check caller disappears after GPU finalization.
    tokio::spawn(async move { validate_probe_output(&output, directory.path()).await })
        .await
        .map_err(|error| format!("Vulkan probe output verification failed: {error}"))?
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn probe_plan(encoder: &str, output: &std::path::Path) -> Result<MediaPlan, String> {
    use ffmpeg_utils::{
        ColorSource, FilterGraphPlan, FrameRatePolicy, GraphInput, GraphOutput, MediaInput,
        MediaKind, MediaOutput, StreamRoute, StreamSelector, StreamSource,
    };
    let mut input = MediaInput::generated_video(ColorSource {
        color: "blue".into(),
        width: 320,
        height: 240,
        frame_rate_num: 30,
        frame_rate_den: 1,
    });
    input.duration = Some(0.1);
    let settings = crate::project::RenderSettings {
        codec: if encoder.starts_with("hevc_") {
            crate::project::VideoCodec::Hevc
        } else {
            crate::project::VideoCodec::H264
        },
        ..Default::default()
    };
    let mut video = super::hwaccel::native_video_options(encoder, None, None, Some(&settings))?;
    video.frame_processor = true;
    video.frame_rate = FrameRatePolicy::Constant { num: 30, den: 1 };
    Ok(MediaPlan {
        inputs: vec![input],
        outputs: vec![MediaOutput::file(output)],
        filter_graph: Some(FilterGraphPlan {
            spec: "[0:v]scale=out_color_matrix=bt709:out_range=tv,format=nv12[outv]".into(),
            inputs: vec![GraphInput {
                input: 0,
                selector: StreamSelector::FirstVideo,
                label: "0:v".into(),
                required: true,
            }],
            outputs: vec![GraphOutput {
                label: "outv".into(),
                kind: MediaKind::Video,
            }],
        }),
        routes: vec![StreamRoute {
            source: StreamSource::Filter {
                label: "outv".into(),
            },
            output: 0,
            required: true,
            mode: StreamMode::Video(video),
        }],
        ..Default::default()
    })
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn probe_scene(directory: &std::path::Path) -> Result<native_overlay::RenderInput, String> {
    // Generated 1x1 opaque-red RGBA PNG: IHDR, zlib-compressed scanline, IEND.
    const RED_PNG: &[u8] = &[
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6,
        0, 0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 240,
        31, 0, 5, 0, 1, 255, 137, 153, 61, 29, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ];
    let path = directory.join("probe-red.png");
    std::fs::write(&path, RED_PNG).map_err(|error| error.to_string())?;
    Ok(native_overlay::RenderInput {
        documents: vec![],
        images: vec![native_overlay::ImageEvent {
            path,
            start: 0.0,
            end: 1.0,
            // Image events use 1920x1080 reference coordinates. This gives an
            // 80x80 patch at (120,80) in the 320x240 probe video.
            from: (720.0, 360.0),
            to: (720.0, 360.0),
            width: 480.0,
            height: 360.0,
            opacity: 0.5,
        }],
    })
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
async fn validate_probe_output(
    file: &std::path::Path,
    directory: &std::path::Path,
) -> Result<(), String> {
    for index in 0..3 {
        let path = directory.join(format!("probe-{index}.jpg"));
        ffmpeg_utils::write_jpeg(file, index as f64 / 30.0, Some(2), &path).await?;
        let pixels = native_overlay::Renderer::new(
            64,
            48,
            native_overlay::RenderInput {
                documents: vec![],
                images: vec![native_overlay::ImageEvent {
                    path,
                    start: 0.0,
                    end: 1.0,
                    from: (0.0, 0.0),
                    to: (0.0, 0.0),
                    width: 1920.0,
                    height: 1080.0,
                    opacity: 1.0,
                }],
            },
            Vec::new(),
        )?
        .frame(0.0)?;
        validate_probe_pixels(&pixels)?;
    }
    Ok(())
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn validate_probe_pixels(pixels: &[u8]) -> Result<(), String> {
    if pixels.len() != 64 * 48 * 4 {
        return Err("Vulkan probe decoded an unexpected frame size".into());
    }
    let mean = |left: usize, top: usize| {
        let mut sums = [0u32; 3];
        for y in top..top + 8 {
            for x in left..left + 8 {
                let pixel = &pixels[(y * 64 + x) * 4..][..4];
                for (sum, value) in sums.iter_mut().zip(pixel) {
                    *sum += u32::from(*value);
                }
            }
        }
        sums.map(|sum| sum / 64)
    };
    let background = mean(4, 4);
    let overlay = mean(28, 20);
    if !(background[0] < 45 && background[1] < 45 && background[2] > 200) {
        return Err(format!(
            "Vulkan probe lost its blue video base: {background:?}"
        ));
    }
    // Match the existing nonlinear, premultiplied RGB composition contract:
    // 50% red over blue is purple near (128,0,128), not linear-light ~188.
    if !(overlay[0].abs_diff(128) <= 24 && overlay[1] < 35 && overlay[2].abs_diff(128) <= 24) {
        return Err(format!(
            "Vulkan probe lost its premultiplied overlay: {overlay:?}"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "windows")]
    const ENCODERS: [&str; 2] = ["h264_vulkan", "hevc_vulkan"];
    #[cfg(target_os = "macos")]
    const ENCODERS: [&str; 2] = ["h264_videotoolbox", "hevc_videotoolbox"];

    use std::{collections::HashSet, path::Path};

    use ffmpeg_utils::{
        FrameRatePolicy, MediaJobResult, MediaOutput, StreamMode, StreamRoute, StreamSelector,
        StreamSource, VideoEncodeOptions,
    };

    #[derive(Clone)]
    pub(super) struct Silent;
    #[async_trait::async_trait]
    impl crate::progress::progress_reporter::ProgressReporterTrait for Silent {
        async fn update(&self, _: &str) {}
        async fn finish(&self, _: bool, _: &str) {}
    }

    #[test]
    fn native_gpu_probe_rejects_black_missing_overlay_and_linear_alpha() {
        assert!(super::validate_probe_pixels(&[0, 0, 0, 255].repeat(64 * 48)).is_err());
        let mut pixels = [0, 0, 255, 255].repeat(64 * 48);
        assert!(super::validate_probe_pixels(&pixels).is_err());
        for y in 16..32 {
            for x in 24..40 {
                pixels[(y * 64 + x) * 4..][..4].copy_from_slice(&[128, 0, 128, 255]);
            }
        }
        super::validate_probe_pixels(&pixels).unwrap();
        for y in 16..32 {
            for x in 24..40 {
                pixels[(y * 64 + x) * 4..][..4].copy_from_slice(&[188, 0, 188, 255]);
            }
        }
        assert!(super::validate_probe_pixels(&pixels).is_err());
    }

    #[tokio::test]
    async fn native_gpu_probe_verifies_the_decoded_cpu_scene_without_a_device() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("cpu-probe.mp4");
        let mut plan = super::probe_plan("libx264", &output).unwrap();
        if let ffmpeg_utils::StreamMode::Video(options) = &mut plan.routes[0].mode {
            options.frame_processor = false;
        }
        let graph = plan.filter_graph.as_mut().unwrap();
        graph.spec = "[0:v]format=bgra[base];[base][1:v]overlay=shortest=1:format=rgb:alpha=premultiplied,scale=out_color_matrix=bt709:out_range=tv,format=nv12[outv]".into();
        graph.inputs.push(ffmpeg_utils::GraphInput {
            input: 1,
            selector: StreamSelector::FirstVideo,
            label: "1:v".into(),
            required: true,
        });
        let result = super::super::native_overlay::run_overlay_plan(
            plan,
            super::OverlaySpec {
                input: super::probe_scene(directory.path()).unwrap(),
                width: 320,
                height: 240,
                fps: 30.0,
                start: 0.0,
                duration: 0.1,
            },
            1,
            super::ProgressMode::None,
            "CPU reference for GPU capability probe",
            None::<&Silent>,
        )
        .await
        .unwrap();
        assert_eq!(result.video_frames, 3);
        super::validate_probe_output(&output, directory.path())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn native_gpu_moving_reference_samples_change_without_a_device() {
        let directory = tempfile::tempdir().unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("crates/ffmpeg_utils/src/hls/fixtures/live.flv");
        let reference = directory.path().join("reference.mp4");
        let result = ffmpeg_utils::execute_media_plan(reference_plan(&source, &reference), None)
            .await
            .unwrap();
        let grid = decoded_frame_grid(&reference, directory.path()).await;
        assert_eq!(grid.0.len() as u64, result.video_frames);
        assert!(grid.1 > 30);
        let mut frames = Vec::new();
        for time in [1.2, 3.0, 5.8] {
            frames.push(sample(&reference, time, directory.path()).await);
        }
        let differences = [
            difference(&frames[0], &frames[1]),
            difference(&frames[0], &frames[2]),
            difference(&frames[1], &frames[2]),
        ];
        eprintln!("native_gpu_moving_reference differences={differences:?}");
        assert!(differences.into_iter().fold(0.0, f64::max) > 2.0);
    }

    #[tokio::test]
    #[ignore = "Requires a GPU render/encode backend and FFmpeg 9 SDK"]
    async fn native_gpu_export_probes_both_codecs_with_the_media_plan() {
        for encoder in ENCODERS {
            super::probe(encoder).await.unwrap();
            super::probe_with_mode(encoder, super::VulkanSubmissionMode::Synchronous)
                .await
                .unwrap();
        }
        let mut settings = crate::project::RenderSettings::default();
        assert_eq!(
            super::super::hwaccel::get_render_pipeline(&settings).await,
            super::super::hwaccel::RenderPipeline::gpu(ENCODERS[1]),
            "the default export must select the probed GPU backend"
        );
        assert_eq!(
            super::super::hwaccel::material_overlay_pipeline(Some(&settings)).await,
            super::super::hwaccel::RenderPipeline::gpu(ENCODERS[1])
        );
        settings.codec = crate::project::VideoCodec::H264;
        assert_eq!(
            super::super::hwaccel::get_render_pipeline(&settings).await,
            super::super::hwaccel::RenderPipeline::gpu(ENCODERS[0])
        );
        let ordinary = super::super::hwaccel::material_encoder(Some(&settings)).await;
        assert!(!super::super::hwaccel::is_vulkan_encoder(ordinary));
        settings.priority = crate::project::EncodingPriority::Quality;
        let quality = super::super::hwaccel::get_render_pipeline(&settings).await;
        assert!(!quality.uses_gpu());
    }

    #[tokio::test]
    #[ignore = "Requires a GPU render/encode backend and FFmpeg 9 SDK"]
    async fn native_gpu_handoff_drains_short_tails_in_order_for_both_codecs() {
        use super::EncoderFrameHandoff::{Asynchronous, Blocking};
        for encoder in ENCODERS {
            for count in [1, 2, 3, 7] {
                let directory = tempfile::tempdir().unwrap();
                let mut grids = Vec::new();
                for handoff in [Blocking, Asynchronous] {
                    let output = directory.path().join(format!("{handoff:?}.mp4"));
                    let mut plan = super::probe_plan(encoder, &output).unwrap();
                    let duration = count as f64 / 30.0;
                    plan.inputs[0].duration = Some(duration);
                    let profiler = ffmpeg_utils::profiling::MediaProfiler::default();
                    let result = super::run_overlay_plan_with_handoff(
                        plan,
                        super::OverlaySpec {
                            input: super::probe_scene(directory.path()).unwrap(),
                            width: 320,
                            height: 240,
                            fps: 30.0,
                            start: 0.0,
                            duration,
                        },
                        handoff,
                        Some(profiler.clone()),
                        None::<&Silent>,
                    )
                    .await
                    .unwrap();
                    assert_eq!(result.video_frames, count);
                    let grid = decoded_frame_grid(&output, directory.path()).await;
                    assert_eq!(grid.0.len(), count as usize);
                    grids.push(grid.0);
                    if count >= 3 {
                        super::validate_probe_output(&output, directory.path())
                            .await
                            .unwrap();
                    }
                    #[cfg(target_os = "macos")]
                    {
                        let report = profiler.report();
                        let counters = &report.metadata["vt_bridge_counters"];
                        assert!(
                            counters.starts_with(&format!("submitted={count}, completed={count},"))
                        );
                        assert!(counters.ends_with("pending_at_drop=0"));
                        let peak: usize = counters
                            .split("pending_peak=")
                            .nth(1)
                            .unwrap()
                            .split(',')
                            .next()
                            .unwrap()
                            .parse()
                            .unwrap();
                        assert!((1..=3).contains(&peak));
                        assert_eq!(report.metadata["encoder_handoff"], format!("{handoff:?}"));
                        assert_eq!(report.spans["vt_metal_submit"].calls, count);
                    }
                }
                assert_eq!(
                    grids[0], grids[1],
                    "{encoder}, {count} frames: handoff must preserve every PTS/duration"
                );
            }
        }
    }

    #[tokio::test]
    #[ignore = "Requires a GPU render/encode backend and FFmpeg 9 SDK"]
    async fn native_gpu_handoff_failure_and_cancellation_preserve_existing_output() {
        use ffmpeg_utils::{VideoFrame, VideoFrameInfo, VideoFrameProcessor};
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };
        struct Interrupted {
            inner: super::VulkanOverlayProcessor,
            cancelled: Arc<AtomicBool>,
            cancel: bool,
            seen: usize,
        }
        impl VideoFrameProcessor for Interrupted {
            fn output_template(&self) -> Result<VideoFrame, String> {
                self.inner.output_template()
            }
            fn process(&mut self, frame: VideoFrame) -> Result<Vec<VideoFrame>, String> {
                let ready = self.inner.process(frame)?;
                self.seen += 1;
                // Reach actual encoder handoffs and slot reuse before
                // interrupting a live export.
                if self.seen < 4 {
                    return Ok(ready);
                }
                if self.cancel {
                    self.cancelled.store(true, Ordering::Release);
                    Err("test GPU handoff cancellation".into())
                } else {
                    Err("test GPU handoff failure".into())
                }
            }
            fn flush(&mut self) -> Result<Vec<VideoFrame>, String> {
                self.inner.flush()
            }
        }
        for cancel in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let output = directory.path().join("existing.mp4");
            std::fs::write(&output, b"preserve existing output").unwrap();
            let source = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("crates/ffmpeg_utils/src/hls/fixtures/live.flv");
            let plan = moving_plan(&source, &output, ENCODERS[1]);
            let options = match &plan.routes[0].mode {
                StreamMode::Video(options) => options.clone(),
                _ => unreachable!(),
            };
            let resources = plan.resources.clone();
            let profiler = ffmpeg_utils::profiling::MediaProfiler::default();
            let bindings = ffmpeg_utils::MediaBindings {
                video_processors: vec![(
                    0,
                    Box::new(move |info: VideoFrameInfo, cancelled: Arc<AtomicBool>| {
                        let inner = super::VulkanOverlayProcessor::new(
                            info,
                            moving_spec(6.0),
                            options,
                            resources,
                            cancelled.clone(),
                            super::VulkanSubmissionMode::Asynchronous,
                            super::EncoderFrameHandoff::Asynchronous,
                        )?;
                        Ok(Box::new(Interrupted {
                            inner,
                            cancelled,
                            cancel,
                            seen: 0,
                        }) as Box<dyn VideoFrameProcessor>)
                    }),
                )],
                profiler: Some(profiler.clone()),
                ..Default::default()
            };
            let error = ffmpeg_utils::execute_with_sources(plan, bindings, None)
                .await
                .unwrap_err();
            assert!(
                error.message.contains("test GPU handoff") || error.message.contains("取消"),
                "{error}"
            );
            assert_eq!(std::fs::read(&output).unwrap(), b"preserve existing output");
            assert_eq!(
                std::fs::read_dir(directory.path()).unwrap().count(),
                1,
                "remove all staging files"
            );
            #[cfg(target_os = "macos")]
            {
                let report = profiler.report();
                let counters = &report.metadata["vt_bridge_counters"];
                let submitted: u64 = counters
                    .split("submitted=")
                    .nth(1)
                    .unwrap()
                    .split(',')
                    .next()
                    .unwrap()
                    .parse()
                    .unwrap();
                assert!(submitted > 0, "interrupt after a real handoff: {counters}");
                eprintln!("interrupted_handoff cancel={cancel} {counters}");
            }
        }
    }

    #[tokio::test]
    #[ignore = "Requires a GPU render/encode backend and FFmpeg 9 SDK"]
    async fn native_gpu_srgb_file_and_mixed_transfer_concat_keep_colour_and_alpha() {
        let directory = tempfile::tempdir().unwrap();
        let srgb = directory.path().join("srgb.mp4");
        let bt709 = directory.path().join("bt709.mp4");
        for (path, transfer) in [(&srgb, "iec61966-2-1"), (&bt709, "bt709")] {
            let mut plan = super::probe_plan("libx264", path).unwrap();
            plan.inputs[0].duration = Some(0.5);
            plan.filter_graph.as_mut().unwrap().spec = format!(
                "[0:v]format=yuv420p,geq=lum=128:cb=128:cr=128,\
                 setparams=colorspace=bt709:range=limited:color_primaries=bt709:color_trc={transfer}[outv]"
            );
            if let StreamMode::Video(video) = &mut plan.routes[0].mode {
                video.frame_processor = false;
                video.options.insert("crf".into(), "0".into());
            }
            assert_eq!(
                ffmpeg_utils::execute_media_plan(plan, None)
                    .await
                    .unwrap()
                    .video_frames,
                15
            );
        }
        assert!(ffmpeg_utils::video_supports_gpu_sdr(&srgb).await.unwrap());
        assert!(!ffmpeg_utils::video_is_sdr_bt709(&srgb).await.unwrap());
        assert!(ffmpeg_utils::video_is_sdr_bt709(&bt709).await.unwrap());
        for encoder in ENCODERS {
            for opacity in [0.0, 0.5, 1.0] {
                let output = directory.path().join("composited.mp4");
                let mut plan = moving_plan(&srgb, &output, encoder);
                plan.inputs.push(ffmpeg_utils::MediaInput::file(&bt709));
                let graph = plan.filter_graph.as_mut().unwrap();
                let scale = super::super::hwaccel::GPU_SCALE_PAD_FILTER;
                // No setparams override: decoder/filter/concat must preserve
                // the changing transfer on each actual frame, not the first
                // input's tag or a fixed flag selected when opening the job.
                graph.spec = format!(
                    "[0:v]{scale},fps=60[a];[1:v]{scale},fps=60[b];[a][b]concat=n=2:v=1:a=0[outv]"
                );
                graph.inputs.push(ffmpeg_utils::GraphInput {
                    input: 1,
                    selector: StreamSelector::FirstVideo,
                    label: "1:v".into(),
                    required: true,
                });
                let mut scene = super::probe_scene(directory.path()).unwrap();
                scene.images[0].opacity = opacity;
                let profiler = ffmpeg_utils::profiling::MediaProfiler::default();
                let result = super::run_overlay_plan_measured(
                    plan,
                    super::OverlaySpec {
                        input: scene,
                        width: 1920,
                        height: 1080,
                        fps: 60.0,
                        start: 0.0,
                        duration: 1.0,
                    },
                    profiler.clone(),
                    None::<&Silent>,
                )
                .await
                .unwrap();
                assert_eq!(result.video_frames, 60);
                let report = profiler.report();
                let counters = &report.metadata["compositor_counters"];
                assert!(counters.contains("srgb=30,"), "{counters}");
                assert!(ffmpeg_utils::video_is_sdr_bt709(&output).await.unwrap());
                let raw_path = directory.path().join("pixels.nv12");
                let mut raw_output = MediaOutput::file(&raw_path);
                raw_output.format = Some("rawvideo".into());
                let decode = ffmpeg_utils::MediaPlan {
                    inputs: vec![ffmpeg_utils::MediaInput::file(&output)], outputs: vec![raw_output],
                    routes: vec![StreamRoute {
                        source: StreamSource::Input { input: 0, selector: StreamSelector::FirstVideo }, output: 0, required: true,
                        mode: StreamMode::Video(VideoEncodeOptions {
                            encoder: "rawvideo".into(), filter: Some("scale=64:36:in_color_matrix=bt709:out_color_matrix=bt709:in_range=tv:out_range=tv,format=nv12".into()),
                            frame_rate: FrameRatePolicy::Passthrough, ..Default::default()
                        }),
                    }], ..Default::default()
                };
                assert_eq!(
                    ffmpeg_utils::execute_media_plan(decode, None)
                        .await
                        .unwrap()
                        .video_frames,
                    60
                );
                let pixels = std::fs::read(raw_path).unwrap();
                let encoded_gray: f64 = 112.0 / 219.0;
                let linear = ((encoded_gray + 0.055) / 1.055).powf(2.4);
                let converted = (1.099 * linear.powf(0.45) - 0.099) * 255.0;
                let mut maximum_error: f64 = 0.0;
                for (frame, gray) in [(5, converted), (35, encoded_gray * 255.0)] {
                    let alpha = (opacity as f64 * 255.0).round() / 255.0;
                    for (x, y, expected) in [
                        (16, 4, [gray; 3]),
                        (
                            32,
                            18,
                            [
                                255.0 * alpha + gray * (1.0 - alpha),
                                gray * (1.0 - alpha),
                                gray * (1.0 - alpha),
                            ],
                        ),
                    ] {
                        // Compare decoded NV12 directly. A second low-resolution
                        // YUV->RGB swscale introduces avoidable integer errors.
                        let [r, g, b] = expected.map(|c| c / 255.0);
                        let expected = [
                            16.0 + 219.0 * (0.2126 * r + 0.7152 * g + 0.0722 * b),
                            128.0
                                + 224.0 * (-0.11457210605734 * r - 0.38542789394266 * g + 0.5 * b),
                            128.0
                                + 224.0 * (0.5 * r - 0.454152908305817 * g - 0.045847091694183 * b),
                        ];
                        let start = frame * 64 * 36 * 3 / 2;
                        let uv = start + 64 * 36 + (y / 2) * 64 + (x / 2) * 2;
                        let actual = [pixels[start + y * 64 + x], pixels[uv], pixels[uv + 1]];
                        for c in 0..3 {
                            let error = (actual[c] as f64 - expected[c]).abs();
                            maximum_error = maximum_error.max(error);
                            assert!(
                                error <= 2.0,
                                "{encoder} opacity={opacity} frame={frame} ({x},{y},{c}): {} vs {}",
                                actual[c],
                                expected[c]
                            );
                        }
                    }
                }
                eprintln!("sRGB MediaPlan {encoder} opacity={opacity}: {counters}; converted pixels and BT.709 tail verified, max YUV error {maximum_error:.3}");
            }
        }
    }

    fn moving_spec(duration: f64) -> super::OverlaySpec {
        let ass = super::super::subtitle_native::srt_to_ass(
            "1\n00:00:00,000 --> 00:00:00,800\nGPU\n",
            "FontName=Arial,FontSize=36,PrimaryColour=&H800000FF",
        )
        .unwrap();
        super::OverlaySpec {
            input: native_overlay::RenderInput {
                documents: vec![native_overlay::AssSource {
                    text: ass,
                    kind: native_overlay::SourceKind::Subtitle,
                }],
                images: vec![],
            },
            width: 1920,
            height: 1080,
            fps: 60.0,
            start: 0.0,
            duration,
        }
    }

    #[tokio::test]
    #[ignore = "Requires a GPU render/encode backend and FFmpeg 9 SDK"]
    async fn native_gpu_moving_video_keeps_every_frame_audio_and_temporal_content() {
        let directory = tempfile::tempdir().unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("crates/ffmpeg_utils/src/hls/fixtures/live.flv");
        assert!(ffmpeg_utils::video_supports_gpu_sdr(&source).await.unwrap());
        let duration = super::super::danmu::probe_overlay_duration(&source, 6.0)
            .await
            .unwrap();
        let reference = directory.path().join("reference.mp4");
        let reference_result =
            ffmpeg_utils::execute_media_plan(reference_plan(&source, &reference), None)
                .await
                .unwrap();
        let reference_grid = decoded_frame_grid(&reference, directory.path()).await;
        assert_eq!(reference_grid.0.len() as u64, reference_result.video_frames);
        assert!(
            reference_grid.1 > 30,
            "fixture must contain changing frames"
        );
        let reference_audio = audio(&reference).await;
        assert!(!reference_audio.is_empty());
        let times = [1.2, 3.0, 5.8];
        let mut reference_samples = Vec::new();
        for time in times {
            reference_samples.push(sample(&reference, time, directory.path()).await);
        }
        let mut moving_pair = (0, 1, 0.0);
        for left in 0..reference_samples.len() {
            for right in left + 1..reference_samples.len() {
                let difference = difference(&reference_samples[left], &reference_samples[right]);
                if difference > moving_pair.2 {
                    moving_pair = (left, right, difference);
                }
            }
        }
        assert!(moving_pair.2 > 2.0, "source samples must show real motion");
        for encoder in ENCODERS {
            let output = directory.path().join(format!("{encoder}.mp4"));
            let ass = super::super::subtitle_native::srt_to_ass(
                "1\n00:00:00,000 --> 00:00:00,800\nGPU\n",
                "FontName=Arial,FontSize=36,PrimaryColour=&H800000FF",
            )
            .unwrap();
            let result: MediaJobResult = super::run_overlay_plan(
                moving_plan(&source, &output, encoder),
                super::OverlaySpec {
                    input: native_overlay::RenderInput {
                        documents: vec![native_overlay::AssSource {
                            text: ass,
                            kind: native_overlay::SourceKind::Subtitle,
                        }],
                        images: vec![],
                    },
                    width: 1920,
                    height: 1080,
                    fps: 60.0,
                    start: 0.0,
                    duration,
                },
                super::ProgressMode::None,
                "Moving GPU video regression",
                None::<&Silent>,
            )
            .await
            .unwrap();
            assert_eq!(result.video_frames, reference_result.video_frames);
            let grid = decoded_frame_grid(&output, directory.path()).await;
            assert_eq!(
                grid.0, reference_grid.0,
                "{encoder} complete decoded timeline"
            );
            assert!(grid.1 > 30, "{encoder} must preserve the moving base");
            assert_eq!(
                audio(&output).await,
                reference_audio,
                "{encoder} copied audio"
            );
            let expected_probe = ffmpeg_utils::probe_media(&reference).await.unwrap();
            let actual_probe = ffmpeg_utils::probe_media(&output).await.unwrap();
            assert_eq!(actual_probe.audio, expected_probe.audio);
            assert!(
                (actual_probe.video_start_time.unwrap_or(0.0)
                    - expected_probe.video_start_time.unwrap_or(0.0))
                .abs()
                    < 1.0 / 60.0
            );
            let mut samples = Vec::new();
            for (index, time) in times.into_iter().enumerate() {
                let pixels = sample(&output, time, directory.path()).await;
                let error = difference(&pixels, &reference_samples[index]);
                assert!(error < 8.0, "{encoder} frame at {time}: RGB error {error}");
                samples.push(pixels);
            }
            let (left, right, _) = moving_pair;
            assert!(
                difference(&samples[left], &reference_samples[left])
                    < difference(&samples[left], &reference_samples[right]) * 0.7,
                "{encoder} must show the corresponding time, not a frozen or shifted base"
            );
            assert!(
                difference(&samples[right], &reference_samples[right])
                    < difference(&samples[right], &reference_samples[left]) * 0.7,
                "{encoder} must preserve motion through the last sample"
            );
            eprintln!(
                "native_gpu_moving backend=vulkan encoder={encoder} frames={} unique_decoded_frames={}",
                result.video_frames, grid.1
            );
        }
    }

    fn moving_plan(source: &Path, output: &Path, encoder: &'static str) -> ffmpeg_utils::MediaPlan {
        let settings = crate::project::RenderSettings {
            codec: if encoder.starts_with("hevc_") {
                crate::project::VideoCodec::Hevc
            } else {
                crate::project::VideoCodec::H264
            },
            ..Default::default()
        };
        let mut options =
            super::super::hwaccel::native_video_options(encoder, None, None, Some(&settings))
                .unwrap();
        options.frame_processor = true;
        options.frame_rate = FrameRatePolicy::Constant { num: 60, den: 1 };
        let mut plan = super::super::danmu::overlay_media_plan(
            source.to_path_buf(),
            output.to_path_buf(),
            super::super::danmu::overlay_graph(
                60.0,
                super::super::hwaccel::RenderPipeline::gpu(encoder),
            ),
            "outv",
            options,
        );
        // The fixture is SDR with unspecified tags. Its base graph explicitly
        // converts the matrix/range; tag that common reference consistently.
        let graph = plan.filter_graph.as_mut().unwrap();
        graph.spec = graph.spec.replace(
            "format=nv12[outv]",
            "format=nv12,setparams=colorspace=bt709:range=limited:color_primaries=bt709:color_trc=bt709[outv]",
        );
        plan
    }

    fn reference_plan(source: &Path, output: &Path) -> ffmpeg_utils::MediaPlan {
        let mut plan = moving_plan(source, output, "h264_vulkan");
        // Use the same normalized base before adding a GPU overlay. A native
        // high-quality CPU encode supplies the temporal reference without CLI.
        plan.routes[0].mode = StreamMode::Video(VideoEncodeOptions {
            encoder: "libx264".into(),
            frame_rate: FrameRatePolicy::Constant { num: 60, den: 1 },
            options: [
                ("preset".into(), "veryfast".into()),
                ("crf".into(), "8".into()),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        });
        plan
    }

    async fn decoded_frame_grid(file: &Path, directory: &Path) -> (Vec<String>, usize) {
        let path = directory.join("decoded.framehash");
        let mut output = MediaOutput::file(&path);
        output.format = Some("framehash".into());
        let plan = ffmpeg_utils::MediaPlan {
            inputs: vec![ffmpeg_utils::MediaInput::file(file)],
            outputs: vec![output],
            routes: vec![StreamRoute {
                source: StreamSource::Input {
                    input: 0,
                    selector: StreamSelector::FirstVideo,
                },
                output: 0,
                required: true,
                mode: StreamMode::Video(VideoEncodeOptions {
                    encoder: "rawvideo".into(),
                    filter: Some("scale=64:48,format=gray".into()),
                    frame_rate: FrameRatePolicy::Passthrough,
                    ..Default::default()
                }),
            }],
            ..Default::default()
        };
        let result = ffmpeg_utils::execute_media_plan(plan, None).await.unwrap();
        let content = std::fs::read_to_string(path).unwrap();
        let records: Vec<_> = content
            .lines()
            .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
            .map(|line| line.split(',').map(str::trim).collect::<Vec<_>>())
            .collect();
        assert_eq!(records.len() as u64, result.video_frames);
        let grid = records.iter().map(|fields| fields[..5].join(",")).collect();
        let hashes: HashSet<_> = records.iter().map(|fields| fields[5]).collect();
        (grid, hashes.len())
    }

    async fn sample(file: &Path, time: f64, directory: &Path) -> Vec<u8> {
        let path = directory.join("sample.jpg");
        ffmpeg_utils::write_jpeg(file, time, Some(2), &path)
            .await
            .unwrap();
        native_overlay::Renderer::new(
            64,
            36,
            native_overlay::RenderInput {
                documents: vec![],
                images: vec![native_overlay::ImageEvent {
                    path,
                    start: 0.0,
                    end: 1.0,
                    from: (0.0, 0.0),
                    to: (0.0, 0.0),
                    width: 1920.0,
                    height: 1080.0,
                    opacity: 1.0,
                }],
            },
            Vec::new(),
        )
        .unwrap()
        .frame(0.0)
        .unwrap()
    }

    fn difference(left: &[u8], right: &[u8]) -> f64 {
        assert_eq!(left.len(), right.len());
        let sum: u64 = left
            .chunks_exact(4)
            .zip(right.chunks_exact(4))
            .map(|(left, right)| {
                (0..3)
                    .map(|channel| u64::from(left[channel].abs_diff(right[channel])))
                    .sum::<u64>()
            })
            .sum();
        sum as f64 / (left.len() / 4 * 3) as f64
    }

    async fn audio(file: &Path) -> Vec<i16> {
        #[derive(Default)]
        struct Pcm(Vec<i16>);
        impl ffmpeg_utils::AudioSink for Pcm {
            fn write(&mut self, samples: &[i16]) -> Result<(), String> {
                self.0.extend_from_slice(samples);
                Ok(())
            }
        }
        ffmpeg_utils::decode_audio(
            file,
            ffmpeg_utils::AudioDecodeOptions { sample_rate: 16000 },
            Pcm::default(),
        )
        .await
        .unwrap()
        .sink
        .0
    }
}
