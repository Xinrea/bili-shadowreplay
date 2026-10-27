use std::{collections::BTreeMap, env, fs, path::PathBuf, process::Command};

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    println!("cargo:rerun-if-changed=native");
    println!("cargo:rerun-if-env-changed=BSR_FFMPEG_PREFIXES");
    println!("cargo:rerun-if-env-changed=PATH");
    let prefixes: Vec<_> = match env::var_os("BSR_FFMPEG_PREFIXES") {
        Some(paths) => env::split_paths(&paths).collect(),
        None => {
            let brew = Command::new("brew")
                .args(["--prefix", "ffmpeg"])
                .output()
                .expect("Install build dependencies: brew install ffmpeg pkg-config");
            assert!(
                brew.status.success(),
                "Install build dependencies: brew install ffmpeg pkg-config"
            );
            vec![PathBuf::from(
                String::from_utf8(brew.stdout).unwrap().trim(),
            )]
        }
    };
    assert!(
        !prefixes.is_empty(),
        "BSR_FFMPEG_PREFIXES must not be empty"
    );
    let shader = fs::read_to_string("native/composite.metal").unwrap();
    fs::write(
        out.join("shader.h"),
        format!("static const char* COMPOSITE_SHADER = R\"METAL({shader})METAL\";\n"),
    )
    .unwrap();
    // Each requested ABI uses its own headers. The default builds only the
    // installed FFmpeg; multiple prefixes explicitly opt in to multiple ABIs.
    let mut adapters = BTreeMap::new();
    for prefix in prefixes {
        assert!(
            prefix.is_absolute(),
            "FFmpeg prefixes must be absolute paths: {}",
            prefix.display()
        );
        let pc = prefix.join("lib/pkgconfig");
        println!("cargo:rerun-if-changed={}", pc.display());
        env::set_var("PKG_CONFIG_LIBDIR", &pc);
        env::set_var("PKG_CONFIG_PATH", &pc);
        let mut versions = [0u32; 4];
        for ((name, minimum), version) in [
            ("libavformat", "62.3.100"),
            ("libavcodec", "62.11.100"),
            ("libavfilter", "11.4.100"),
            ("libavutil", "60.8.100"),
        ]
        .into_iter()
        .zip(&mut versions)
        {
            let library = pkg_config::Config::new()
                .cargo_metadata(false)
                .atleast_version(minimum)
                .probe(name)
                .unwrap_or_else(|error| {
                    panic!(
                        "{}: FFmpeg >= 8.0 ({name} >= {minimum}) is required.\n{error}",
                        prefix.display()
                    )
                });
            *version = library.version.split('.').next().unwrap().parse().unwrap();
        }
        // Different patch/minor installations sharing an ABI need one adapter.
        if adapters.contains_key(&versions) {
            continue;
        }
        let id = versions.map(|v| v.to_string()).join("_");
        cc::Build::new()
            .cpp(true)
            .file("native/export.mm")
            .include(prefix.join("include"))
            .include(&out)
            .define(
                "bsr_metal_export",
                format!("bsr_metal_export_{id}").as_str(),
            )
            .define(
                "bsr_videotoolbox_probe",
                format!("bsr_videotoolbox_probe_{id}").as_str(),
            )
            .define("bsr_ffmpeg_load", format!("bsr_ffmpeg_load_{id}").as_str())
            .flag("-std=c++17")
            .flag("-fobjc-arc")
            .flag("-Wno-deprecated-declarations")
            .flag("-mmacosx-version-min=13.3")
            .compile(&format!("metal_export_{id}"));
        adapters.insert(versions, prefix);
    }
    let mut bindings = String::from("extern \"C\" {\n");
    let mut entries = String::from("const ADAPTERS: &[Adapter] = &[\n");
    for (versions, prefix) in adapters.iter().rev() {
        let id = versions.map(|v| v.to_string()).join("_");
        bindings.push_str(&format!(
            "fn bsr_metal_export_{id}(path: *const c_char) -> i32;\n\
             fn bsr_videotoolbox_probe_{id}() -> i32;\n\
             fn bsr_ffmpeg_load_{id}(directory: *const c_char) -> *const c_char;\n"
        ));
        let directory = prefix.join("lib");
        let directory = directory.to_str().expect("FFmpeg prefix must be UTF-8");
        entries.push_str(&format!(
            "Adapter {{ versions: {versions:?}, directory: {directory:?}, \
             load: bsr_ffmpeg_load_{id}, export: bsr_metal_export_{id}, \
             probe: bsr_videotoolbox_probe_{id} }},\n"
        ));
    }
    fs::write(
        out.join("backends.rs"),
        format!("{bindings}}}\n{entries}];\n"),
    )
    .unwrap();
    for name in ["Foundation", "Metal", "CoreVideo"] {
        println!("cargo:rustc-link-lib=framework={name}");
    }
}
