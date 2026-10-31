//! Exercise the production Skia/FFmpeg GPU compositor without requiring a
//! Vulkan Video encoder. The output remains hardware NV12; this probe does not
//! download pixels or establish visual correctness or encoder interoperability.
mod scene;

use ffmpeg_utils::vulkan_video::{VulkanCompositor, VulkanImageState, VulkanRenderOptions};
use native_overlay::{ExternalVulkanImage, NativeVulkanDevice, VulkanProcAddress, VulkanRenderer};
use serde_json::json;
use std::sync::{atomic::AtomicBool, Arc};

fn run() -> Result<(), String> {
    let assets = tempfile::tempdir().map_err(|error| error.to_string())?;
    let red = assets.path().join("red.png");
    std::fs::write(&red, include_bytes!("../assets/red.png")).map_err(|error| error.to_string())?;
    let options = VulkanRenderOptions::default();
    let frame_count = 60;
    let mut compositor =
        VulkanCompositor::new(options.clone(), false, Arc::new(AtomicBool::new(false)))?;
    let device = compositor.device().clone();
    let info = device.info();
    let native = NativeVulkanDevice {
        instance: info.instance,
        physical_device: info.physical_device,
        device: info.device,
        queue: info.queue,
        queue_family: info.queue_family,
        instance_extensions: info.instance_extensions,
        device_extensions: info.device_extensions,
        max_api_version: info.max_api_version,
    };
    // The compositor owns the device and frame leases. The renderer is dropped
    // only after synchronizing all submitted work, including error paths.
    let mut renderer = unsafe {
        VulkanRenderer::new(
            options.width,
            options.height,
            scene::scene(&red, 1.0),
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
    let result = (|| -> Result<usize, String> {
        let mut outputs = 0;
        for index in 0..frame_count {
            let lease = compositor.acquire_frame(i64::from(index))?;
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
                renderer.render_into(f64::from(index) / 60.0, &target, scene::BACKGROUND)
            }?;
            let frames = unsafe {
                compositor.submit_frame(
                    lease,
                    VulkanImageState {
                        layout: state.layout,
                        queue_family: state.queue_family,
                    },
                    None,
                )
            }?;
            for frame in &frames {
                if frame.format().descriptor().map(|format| format.name()) != Some("vulkan") {
                    return Err("Compositor returned a software frame".into());
                }
            }
            outputs += frames.len();
        }
        let tail = compositor.flush()?;
        for frame in &tail {
            if frame.format().descriptor().map(|format| format.name()) != Some("vulkan") {
                return Err("Compositor flushed a software frame".into());
            }
        }
        outputs += tail.len();
        if outputs != frame_count as usize {
            return Err(format!(
                "Expected {frame_count} hardware frames, got {outputs}"
            ));
        }
        Ok(outputs)
    })();
    let synchronized = compositor.synchronize();
    let stats = renderer.stats();
    let missing_glyphs = renderer.missing_glyph_count();
    drop(renderer);
    let outputs = result?;
    synchronized?;
    println!(
        "{}",
        json!({
            "scope": "Skia Vulkan scene and libplacebo NV12 conversion; no encoder or pixel readback",
            "width": options.width,
            "height": options.height,
            "rendered_frames": stats.frames,
            "hardware_output_frames": outputs,
            "missing_glyphs": missing_glyphs,
            "scene_readback_ns": stats.scene.readback.as_nanos(),
            "compositor": compositor.stats(),
        })
    );
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Vulkan render probe: {error}");
        std::process::exit(1);
    }
}
