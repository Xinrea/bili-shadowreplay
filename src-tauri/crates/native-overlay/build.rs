fn main() {
    let target = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    // Skia 0.82's prebuilt ICU references Windows registry APIs.
    if target == "windows" {
        println!("cargo:rustc-link-lib=advapi32");
        // The upstream Windows archive's ICU shares symbol names with V8's
        // ICU, which MSVC rejects at link time. Fail before the long build.
        println!("cargo:rerun-if-env-changed=SKIA_BINARIES_URL");
        let (backend, option) = if cfg!(feature = "vulkan") {
            ("Vulkan", "")
        } else {
            ("CPU", " --cpu")
        };
        assert!(
            std::env::var_os("SKIA_BINARIES_URL").is_some(),
            "Windows native-overlay requires the patched {backend} Skia archive: run `node scripts/prepare-windows-skia.mjs{option}` and set the SKIA_BINARIES_URL it prints; see docs/development/native-overlay.md"
        );
    }
    if target == "linux" {
        // The upstream Linux archive compiles out COLRv1 and bundles an ICU
        // symbol that collides with V8. Fail clearly rather than paint blanks.
        for name in ["FORCE_SKIA_BUILD", "SKIA_USE_SYSTEM_LIBRARIES"] {
            println!("cargo:rerun-if-env-changed={name}");
            assert!(
                std::env::var_os(name).is_some(),
                "Linux native-overlay requires FORCE_SKIA_BUILD=1 and SKIA_USE_SYSTEM_LIBRARIES=1; see docs/development/native-overlay.md"
            );
        }
        // 0.82 enables PDF subsetting with textlayout, but its system-library
        // linker metadata omits this companion to libharfbuzz.
        println!("cargo:rustc-link-lib=harfbuzz-subset");
    }
}
