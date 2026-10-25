fn main() {
    // skia-safe 0.82's bundled ICU queries the Windows registry for timezone
    // data, but that release does not propagate this system library itself.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-lib=advapi32");
    }
}
