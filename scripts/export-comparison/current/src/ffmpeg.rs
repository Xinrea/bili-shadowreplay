//! These are the application files, including Windows GPU capability probing.
#![allow(dead_code)]
#[path = "../../../../src-tauri/src/ffmpeg/hwaccel.rs"]
pub mod hwaccel;
#[cfg(any(target_os = "windows", target_os = "macos"))]
#[path = "../../../../src-tauri/src/ffmpeg/native_gpu.rs"]
pub mod native_gpu;
#[path = "../../../../src-tauri/src/ffmpeg/native_overlay.rs"]
pub mod native_overlay;
#[path = "../../../../src-tauri/src/ffmpeg/runner.rs"]
pub mod runner;

pub(crate) fn is_vulkan_encoder(encoder: &str) -> bool {
    hwaccel::is_vulkan_encoder(encoder)
}
pub(crate) async fn get_cpu_render_encoder(
    settings: &crate::project::RenderSettings,
) -> &'static str {
    hwaccel::get_cpu_render_encoder(settings).await
}
