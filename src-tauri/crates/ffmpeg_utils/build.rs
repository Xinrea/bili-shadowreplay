fn main() {
    println!("cargo:rerun-if-changed=src/vulkan_video/metal_transfer.m");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos")
        && std::env::var_os("CARGO_FEATURE_VULKAN_VIDEO").is_some()
    {
        cc::Build::new()
            .file("src/vulkan_video/metal_transfer.m")
            .flag("-fobjc-arc")
            .compile("bsr_metal_transfer");
        for framework in ["Foundation", "Metal", "CoreVideo", "IOSurface"] {
            println!("cargo:rustc-link-lib=framework={framework}");
        }
    }
    // ffmpeg-sys-next forwards its header-version checks through links=ffmpeg.
    // Keep ABI-specific field access aligned with the same SDK as the bindings.
    println!("cargo:rustc-check-cfg=cfg(bsr_ffmpeg_8_0)");
    println!("cargo:rustc-check-cfg=cfg(bsr_ffmpeg_9_0)");
    if std::env::var("DEP_FFMPEG_FFMPEG_8_0").as_deref() == Ok("true") {
        println!("cargo:rustc-cfg=bsr_ffmpeg_8_0");
    }
    if std::env::var("DEP_FFMPEG_FFMPEG_9_0").as_deref() == Ok("true") {
        println!("cargo:rustc-cfg=bsr_ffmpeg_9_0");
    }
}
