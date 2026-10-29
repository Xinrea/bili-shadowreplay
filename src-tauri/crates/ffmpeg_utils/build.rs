fn main() {
    // ffmpeg-sys-next forwards its header-version checks through links=ffmpeg.
    // Keep ABI-specific field access aligned with the same SDK as the bindings.
    println!("cargo:rustc-check-cfg=cfg(bsr_ffmpeg_8_0)");
    if std::env::var("DEP_FFMPEG_FFMPEG_8_0").as_deref() == Ok("true") {
        println!("cargo:rustc-cfg=bsr_ffmpeg_8_0");
    }
}
