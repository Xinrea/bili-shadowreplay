//! Benchmark the real shared-image Vulkan path, including all finalization.

use super::{BenchmarkSpec, BACKGROUND};
use ffmpeg_utils::vulkan_video::{VulkanImageState, VulkanVideoOptions, VulkanVideoSession};
use native_overlay::{
    ExternalVulkanImage, NativeVulkanDevice, RenderInput, VulkanProcAddress, VulkanRenderer,
};
use serde_json::{json, Value};
use std::{path::Path, time::Instant};

/// Render and encode the same prepared scene as the CPU benchmark. Asset
/// creation belongs to the caller and is outside both backends' timers.
pub fn run(spec: &BenchmarkSpec, input: RenderInput, output: &Path) -> Result<Value, String> {
    let started = Instant::now();
    let mut session = VulkanVideoSession::new(
        output,
        VulkanVideoOptions {
            width: spec.width,
            height: spec.height,
            fps_num: spec.fps,
            fps_den: 1,
            encoder: "hevc_vulkan".into(),
            device_selector: "0".into(),
            bit_rate: spec.bit_rate,
            gop: spec.gop,
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
    // The device clone remains alive through renderer destruction and session
    // finalization. This worker is the sole external queue submitter.
    let mut renderer = unsafe {
        VulkanRenderer::new(
            spec.width,
            spec.height,
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
    let initialized = started.elapsed();
    let frame_loop_started = Instant::now();
    for index in 0..spec.frames {
        let lease = session.acquire_frame(i64::from(index))?;
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
        // acquire_frame has waited and transitioned this exact borrowed image.
        // render_into waits for Skia's writes before returning its final state.
        let state = unsafe {
            renderer.render_into(f64::from(index) / f64::from(spec.fps), &target, BACKGROUND)
        }?;
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
    let frame_loop = frame_loop_started.elapsed();
    let rendered = renderer.stats();
    let missing_glyphs = renderer.missing_glyph_count();
    let finalization_started = Instant::now();
    // Include surface/cache destruction, encoder/filter drain, MP4 publication
    // and device destruction in total_ms, just as for a completed CPU job.
    drop(renderer);
    let encoded = session.finish()?;
    drop(device);
    let finalization = finalization_started.elapsed();
    let total = started.elapsed();
    if rendered.frames != u64::from(spec.frames)
        || encoded.submitted_frames != u64::from(spec.frames)
        || encoded.encoded_packets != u64::from(spec.frames)
    {
        return Err("Vulkan benchmark did not render and encode every requested frame".into());
    }
    Ok(json!({
        "backend": "vulkan",
        "encoder": "hevc_vulkan",
        "width": spec.width,
        "height": spec.height,
        "fps_num": spec.fps,
        "fps_den": 1,
        "frames": spec.frames,
        "output": output,
        "bit_rate": spec.bit_rate,
        "gop": spec.gop,
        "encoder_policy": {
            "rc_mode": "vbr",
            "quality": 0,
            "quality_source": "FFmpeg 9.0.2 encoder default; higher values trade speed for quality",
            "max_b_frames": 0,
            "max_b_frames_source": "forced by the existing Vulkan PoC",
            "codec_tag_source": "MP4 muxer default; expected hev1, checked independently"
        },
        "initialization_ms": initialized.as_secs_f64() * 1000.0,
        "render_and_encode_ms": frame_loop.as_secs_f64() * 1000.0,
        "finalization_ms": finalization.as_secs_f64() * 1000.0,
        "total_ms": total.as_secs_f64() * 1000.0,
        "rendered_frames": rendered.frames,
        "wrapped_gpu_textures": rendered.texture_wraps,
        "render_composition_ms": rendered.composition.as_secs_f64() * 1000.0,
        "gpu_submit_and_wait_ms": rendered.submit_and_wait.as_secs_f64() * 1000.0,
        "renderer_pixel_readback_ms": rendered.scene.readback.as_secs_f64() * 1000.0,
        "text_layout_ms": rendered.scene.text_layout.as_secs_f64() * 1000.0,
        "text_raster_ms": rendered.scene.text_raster.as_secs_f64() * 1000.0,
        "image_prepare_ms": rendered.scene.image_prepare.as_secs_f64() * 1000.0,
        "text_cache_hits": rendered.scene.text_cache_hits,
        "text_cache_misses": rendered.scene.text_cache_misses,
        "image_cache_hits": rendered.scene.image_cache_hits,
        "image_cache_misses": rendered.scene.image_cache_misses,
        "missing_glyphs": missing_glyphs,
        "ffmpeg": encoded,
        "synchronization": "existing PoC waits for GPU completion; no CPU frame pixel API",
        "timing_scope": "initialization through encoder/filter drain, MP4 publication and renderer/device destruction",
        "readback_proof": "GPU-only frame path validated separately with API tracing; this timing function does not enable capture"
    }))
}
