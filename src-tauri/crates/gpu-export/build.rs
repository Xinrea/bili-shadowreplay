use std::{collections::BTreeMap, env, fs, path::PathBuf, process::Command};

const LIBRARIES: [(&str, &str); 4] = [
    ("libavformat", "62.3.100"),
    ("libavcodec", "62.11.100"),
    ("libavfilter", "11.4.100"),
    ("libavutil", "60.8.100"),
];

fn main() {
    let target = env::var("CARGO_CFG_TARGET_OS").unwrap();
    let metal = target == "macos";
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_VULKAN");
    if !metal
        && !(matches!(target.as_str(), "linux" | "windows")
            && env::var_os("CARGO_FEATURE_VULKAN").is_some())
    {
        return;
    }
    assert_eq!(env::var("CARGO_CFG_TARGET_POINTER_WIDTH").unwrap(), "64");
    for name in ["BSR_FFMPEG_PREFIXES", "PATH", "VULKAN_SDK", "GLSLC"] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    println!("cargo:rerun-if-changed=native");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let mut includes = Vec::new();
    if metal {
        let shader = fs::read_to_string("native/composite.metal").unwrap();
        fs::write(
            out.join("shader.h"),
            format!("static const char* COMPOSITE_SHADER = R\"METAL({shader})METAL\";\n"),
        )
        .unwrap();
    } else {
        let sdk = env::var_os("VULKAN_SDK").map(PathBuf::from);
        if let Some(sdk) = &sdk {
            includes.push(sdk.join(if target == "windows" {
                "Include"
            } else {
                "include"
            }));
        } else {
            includes.extend(
                pkg_config::Config::new()
                    .cargo_metadata(false)
                    .atleast_version("1.3.277")
                    .probe("vulkan")
                    .expect("Install Vulkan SDK headers and glslc, or set VULKAN_SDK")
                    .include_paths,
            );
        }
        let glslc = env::var_os("GLSLC").map(PathBuf::from).unwrap_or_else(|| {
            sdk.map(|sdk| {
                sdk.join(if target == "windows" {
                    "Bin/glslc.exe"
                } else {
                    "bin/glslc"
                })
            })
            .unwrap_or_else(|| PathBuf::from("glslc"))
        });
        let spirv = out.join("composite.spv");
        let status = Command::new(glslc)
            .args([
                "--target-env=vulkan1.3",
                "-fshader-stage=compute",
                "native/composite.comp",
                "-o",
            ])
            .arg(&spirv)
            .status()
            .expect("Run glslc from the Vulkan SDK");
        assert!(status.success(), "Cannot compile Vulkan compositor");
        let words = fs::read(spirv)
            .unwrap()
            .chunks_exact(4)
            .map(|v| u32::from_le_bytes(v.try_into().unwrap()).to_string())
            .collect::<Vec<_>>()
            .join(",");
        fs::write(
            out.join("shader.h"),
            format!("static const uint32_t COMPOSITE_SHADER[] = {{{words}}};\n"),
        )
        .unwrap();
        // SDK prefixes may also contain FFmpeg or older Vulkan headers. Expose
        // only the selected Vulkan C headers first, preserving each FFmpeg ABI.
        let headers = out.join("vulkan-headers");
        if headers.exists() {
            fs::remove_dir_all(&headers).unwrap();
        }
        for include in &includes {
            for name in ["vulkan", "vk_video"] {
                let source = include.join(name);
                if !source.is_dir() {
                    continue;
                }
                println!("cargo:rerun-if-changed={}", source.display());
                let destination = headers.join(name);
                fs::create_dir_all(&destination).unwrap();
                for entry in fs::read_dir(source).unwrap() {
                    let entry = entry.unwrap();
                    if entry.path().extension().and_then(|v| v.to_str()) == Some("h") {
                        fs::copy(entry.path(), destination.join(entry.file_name())).unwrap();
                    }
                }
            }
        }
        includes = vec![headers];
    }
    // Without explicit prefixes, discover one SDK through Homebrew or pkg-config.
    let prefixes = match env::var_os("BSR_FFMPEG_PREFIXES") {
        Some(paths) => env::split_paths(&paths).map(Some).collect::<Vec<_>>(),
        None if metal => {
            let brew = Command::new("brew")
                .args(["--prefix", "ffmpeg"])
                .output()
                .expect("Install ffmpeg and pkg-config with Homebrew");
            assert!(
                brew.status.success(),
                "Install ffmpeg and pkg-config with Homebrew"
            );
            vec![Some(PathBuf::from(
                String::from_utf8(brew.stdout).unwrap().trim(),
            ))]
        }
        None => vec![None],
    };
    assert!(
        !prefixes.is_empty(),
        "BSR_FFMPEG_PREFIXES must not be empty"
    );
    let mut adapters = BTreeMap::new();
    for prefix in prefixes {
        if let Some(prefix) = &prefix {
            assert!(prefix.is_absolute(), "FFmpeg prefixes must be absolute");
            let pc = prefix.join("lib/pkgconfig");
            println!("cargo:rerun-if-changed={}", pc.display());
            env::set_var("PKG_CONFIG_LIBDIR", &pc);
            env::set_var("PKG_CONFIG_PATH", &pc);
        }
        let mut versions = [0u32; 4];
        let mut build = cc::Build::new();
        build.includes(&includes);
        for ((name, minimum), version) in LIBRARIES.into_iter().zip(&mut versions) {
            let library = pkg_config::Config::new()
                .cargo_metadata(false)
                .atleast_version(minimum)
                .probe(name)
                .unwrap_or_else(|e| {
                    panic!("FFmpeg >= 8 development files are required ({name}): {e}")
                });
            *version = library.version.split('.').next().unwrap().parse().unwrap();
            for include in library.include_paths {
                build.include(include);
            }
        }
        if adapters.contains_key(&versions) {
            continue;
        }
        let directory = if target == "windows" {
            prefix
                .unwrap_or_else(|| {
                    PathBuf::from(pkg_config::get_variable("libavformat", "prefix").unwrap())
                })
                .join("bin")
        } else {
            PathBuf::from(pkg_config::get_variable("libavformat", "libdir").unwrap())
        };
        let id = versions.map(|v| v.to_string()).join("_");
        build
            .cpp(true)
            .std("c++17")
            .include(&out)
            .define("bsr_gpu_export", format!("bsr_gpu_export_{id}").as_str())
            .define("bsr_gpu_probe", format!("bsr_gpu_probe_{id}").as_str())
            .define("bsr_ffmpeg_load", format!("bsr_ffmpeg_load_{id}").as_str());
        if metal {
            build
                .file("native/metal.mm")
                .flag("-fobjc-arc")
                .flag("-mmacosx-version-min=13.3");
        } else {
            build.file("native/vulkan.cpp");
            if target == "windows" {
                build.flag("/EHsc");
                build
                    .define("NOMINMAX", None)
                    .define("VK_USE_PLATFORM_WIN32_KHR", None);
            }
        }
        build.compile(&format!("gpu_export_{id}"));
        adapters.insert(versions, directory);
    }
    let mut bindings = String::from("extern \"C\" {\n");
    let mut entries = String::from("const ADAPTERS: &[Adapter] = &[\n");
    for (versions, directory) in adapters.iter().rev() {
        let id = versions.map(|v| v.to_string()).join("_");
        bindings.push_str(&format!("fn bsr_gpu_export_{id}(config: *const ExportConfig, error: *mut c_char, capacity: usize) -> i32;\nfn bsr_gpu_probe_{id}(error: *mut c_char, capacity: usize, quality_max: *mut i32) -> i32;\nfn bsr_ffmpeg_load_{id}(directory: *const c_char) -> *const c_char;\n"));
        let directory = directory
            .to_str()
            .expect("FFmpeg library directory must be UTF-8");
        entries.push_str(&format!("Adapter {{ versions: {versions:?}, directory: {directory:?}, load: bsr_ffmpeg_load_{id}, export: bsr_gpu_export_{id}, probe: bsr_gpu_probe_{id} }},\n"));
    }
    fs::write(
        out.join("backends.rs"),
        format!("{bindings}}}\n{entries}];\n"),
    )
    .unwrap();
    if metal {
        for name in ["Foundation", "Metal", "CoreVideo"] {
            println!("cargo:rustc-link-lib=framework={name}");
        }
    } else {
        println!(
            "cargo:rustc-link-lib={}",
            if target == "windows" {
                "kernel32"
            } else {
                "dl"
            }
        );
    }
}
