fn main() {
    // Skia 0.82's prebuilt ICU references Windows registry APIs.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-lib=advapi32");
    }
}
