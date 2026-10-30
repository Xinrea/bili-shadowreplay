//! GPU-only frame path. Independent verification decodes the finished MP4 in a
//! separate process; this executable never asks Skia/FFmpeg for CPU pixels.
mod scene;

use ffmpeg_utils::vulkan_video::{VulkanImageState, VulkanVideoOptions, VulkanVideoSession};
use native_overlay::{ExternalVulkanImage, NativeVulkanDevice, VulkanProcAddress, VulkanRenderer};
use serde_json::json;
use std::{path::PathBuf, time::Instant};

#[derive(Debug)]
struct Args {
    output: PathBuf,
    encoder: String,
    device: String,
    width: u32,
    height: u32,
    fps: u32,
    frames: u32,
    abort_after: Option<u32>,
}

impl Args {
    fn parse() -> Result<Self, String> {
        let mut result = Self {
            output: PathBuf::from("vulkan-poc.mp4"),
            encoder: "h264_vulkan".into(),
            device: "0".into(),
            width: 1920,
            height: 1080,
            fps: 60,
            frames: 120,
            abort_after: None,
        };
        let mut arguments = std::env::args().skip(1);
        while let Some(name) = arguments.next() {
            if name == "--help" {
                println!("vulkan-video-poc [--output FILE] [--encoder h264_vulkan|hevc_vulkan] [--device SELECTOR] [--width N] [--height N] [--fps N] [--frames N] [--abort-after N]");
                std::process::exit(0);
            }
            let value = arguments
                .next()
                .ok_or_else(|| format!("Missing value for {name}"))?;
            let integer = || {
                value
                    .parse::<u32>()
                    .map_err(|_| format!("Invalid integer for {name}"))
            };
            match name.as_str() {
                "--output" => result.output = value.into(),
                "--encoder" => result.encoder = value,
                "--device" => result.device = value,
                "--width" => result.width = integer()?,
                "--height" => result.height = integer()?,
                "--fps" => result.fps = integer()?,
                "--frames" => result.frames = integer()?,
                "--abort-after" => result.abort_after = Some(integer()?),
                _ => return Err(format!("Unknown option: {name}")),
            }
        }
        if !["h264_vulkan", "hevc_vulkan"].contains(&result.encoder.as_str()) {
            return Err("PoC requires a Vulkan Video H.264 or HEVC encoder".into());
        }
        if result.width < 160
            || result.height < 64
            || result.width > 4096
            || result.height > 4096
            || !result.width.is_multiple_of(2)
            || !result.height.is_multiple_of(2)
            || !(1..=120).contains(&result.fps)
            || result.frames == 0
            || result.frames > 3600
        {
            return Err(
                "Invalid PoC size, rate or frame count (maximum 4096x4096 / 3600 frames)".into(),
            );
        }
        if result
            .abort_after
            .is_some_and(|frame| frame >= result.frames)
        {
            return Err("--abort-after must be smaller than --frames".into());
        }
        Ok(result)
    }
}

fn run(args: Args) -> Result<(), String> {
    let assets = tempfile::tempdir().map_err(|error| error.to_string())?;
    let red = assets.path().join("red.png");
    std::fs::write(&red, include_bytes!("../assets/red.png")).map_err(|error| error.to_string())?;
    let input = scene::scene(&red, f64::from(args.frames) / f64::from(args.fps));
    let started = Instant::now();
    eprintln!("Creating shared Vulkan frame pool, GPU graph and encoder");
    let mut session = VulkanVideoSession::new(
        &args.output,
        VulkanVideoOptions {
            width: args.width,
            height: args.height,
            fps_num: args.fps,
            fps_den: 1,
            encoder: args.encoder.clone(),
            device_selector: args.device.clone(),
            bit_rate: 8_000_000,
            gop: args.fps * 2,
        },
    )?;
    let device = session.device().clone();
    let info = device.info();
    let native = NativeVulkanDevice {
        instance: info.instance,
        physical_device: info.physical_device,
        device: info.device,
        queue: info.queue,
        queue_family: info.queue_family,
        instance_extensions: info.instance_extensions.clone(),
        device_extensions: info.device_extensions.clone(),
        max_api_version: info.max_api_version,
    };
    eprintln!(
        "Creating Skia Vulkan renderer on graphics queue {}",
        info.queue_family
    );
    // The device owner remains alive until the renderer and all frame leases
    // are dropped. This thread is the sole external submitter to its queue.
    let mut renderer = unsafe {
        VulkanRenderer::new(
            args.width,
            args.height,
            input,
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
    renderer.set_profiling(true);
    eprintln!("Rendering {} GPU frames", args.frames);
    let initialized = started.elapsed();
    let render_started = Instant::now();
    for index in 0..args.frames {
        if args.abort_after == Some(index) {
            return Err(format!(
                "Intentional PoC cancellation after {index} GPU frames"
            ));
        }
        let lease = session.acquire_frame(i64::from(index))?;
        if index == 0 {
            eprintln!("Acquired first shared GPU image");
        }
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
        let state = unsafe {
            renderer.render_into(
                f64::from(index) / f64::from(args.fps),
                &target,
                scene::BACKGROUND,
            )
        }?;
        if index == 0 {
            eprintln!("Skia GPU frame complete; submitting for GPU conversion and encode");
        }
        unsafe {
            session.submit_frame(
                lease,
                VulkanImageState {
                    layout: state.layout,
                    queue_family: state.queue_family,
                },
            )
        }?;
    }
    let render_wall = render_started.elapsed();
    let rendered = renderer.stats();
    let missing_glyphs = renderer.missing_glyph_count();
    // Destroy Skia's surfaces/cache/context while the pool and device are live.
    drop(renderer);
    let encoded = session.finish()?;
    let report = json!({
        "width": args.width, "height": args.height,
        "fps_num": args.fps, "fps_den": 1, "frames": args.frames,
        "encoder": args.encoder, "output": args.output,
        "device": format!("{info:?}"),
        "rendered_frames": rendered.frames,
        "wrapped_gpu_textures": rendered.texture_wraps,
        "initialization_ms": initialized.as_secs_f64() * 1000.0,
        "render_and_encode_ms": render_wall.as_secs_f64() * 1000.0,
        "total_ms": started.elapsed().as_secs_f64() * 1000.0,
        "gpu_submit_and_wait_ms": rendered.submit_and_wait.as_secs_f64() * 1000.0,
        "renderer_pixel_readback_ms": rendered.scene.readback.as_secs_f64() * 1000.0,
        "missing_glyphs": missing_glyphs,
        "ffmpeg": encoded,
        "synchronization": "initial PoC waits for GPU completion; no CPU frame pixel API",
        "readback_proof": "verify.py checks a separate Vulkan API dump; zero duration alone is not proof"
    });
    println!("{report}");
    Ok(())
}

fn main() {
    if let Err(error) = Args::parse().and_then(run) {
        eprintln!("Vulkan Video PoC: {error}");
        std::process::exit(1);
    }
}
