use std::env;
use std::process::Command;

fn xcrun_output(args: &[&str]) -> Option<String> {
    let output = Command::new("xcrun").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }

    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn macos_deployment_target() -> Option<String> {
    env::var("MACOSX_DEPLOYMENT_TARGET")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| xcrun_output(&["--sdk", "macosx", "--show-sdk-platform-version"]))
        .or_else(|| xcrun_output(&["--sdk", "macosx", "--show-sdk-version"]))
        .or_else(|| {
            let output = Command::new("sw_vers")
                .args(["-productVersion"])
                .output()
                .ok()?;
            if !output.status.success() {
                return None;
            }
            let value = String::from_utf8(output.stdout).ok()?;
            let value = value.trim();
            if value.is_empty() {
                None
            } else {
                Some(value.to_string())
            }
        })
        .map(|value| {
            let mut parts = value.split('.');
            let major = parts.next().unwrap_or("10");
            let minor = parts.next().unwrap_or("0");
            format!("{major}.{minor}")
        })
}

fn main() {
    let crate_dir = env::current_dir().unwrap();
    let whisper_dir = crate_dir.join("whisper.cpp");

    let mut config = cmake::Config::new(&whisper_dir);

    // Only build the whisper library, not examples or tests
    config.define("BUILD_SHARED_LIBS", "OFF");
    config.define("WHISPER_BUILD_EXAMPLES", "OFF");
    config.define("WHISPER_BUILD_TESTS", "OFF");
    config.define("WHISPER_BUILD_SERVER", "OFF");

    // Platform-specific GPU acceleration
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap();
    let cuda_enabled = env::var("CARGO_FEATURE_CUDA").is_ok();
    let metal_enabled = target_os == "macos";

    match target_os.as_str() {
        "macos" => {
            if let Some(sdk_root) = env::var("SDKROOT")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .or_else(|| xcrun_output(&["--sdk", "macosx", "--show-sdk-path"]))
            {
                config.define("CMAKE_OSX_SYSROOT", &sdk_root);
            }

            if let Some(target) = macos_deployment_target() {
                config.define("CMAKE_OSX_DEPLOYMENT_TARGET", &target);
                config.define("GGML_METAL_MACOSX_VERSION_MIN", &target);
                config.cflag(&format!("-mmacosx-version-min={target}"));
                config.cxxflag(&format!("-mmacosx-version-min={target}"));
            }

            config.define("WHISPER_METAL", "ON");
            config.define("GGML_METAL", "ON");
            println!("cargo:warning=Building whisper.cpp with Metal support");
        }
        _ => {
            if cuda_enabled {
                config.define("GGML_CUDA", "ON");
                println!("cargo:warning=Building whisper.cpp with CUDA support");
            } else {
                config.define("GGML_CUDA", "OFF");
                config.define("GGML_METAL", "OFF");
                println!("cargo:warning=Building whisper.cpp for CPU");
            }
        }
    };

    // Build
    let dst = config.build();

    cc::Build::new()
        .cpp(true)
        .file(crate_dir.join("src/ffi_shim.cpp"))
        .include(whisper_dir.join("include"))
        .include(whisper_dir.join("ggml/include"))
        .flag_if_supported("-std=c++17")
        .compile("whisper_rs_ffi_shim");

    // Link the platform C++ runtime only where the toolchain doesn't provide it implicitly.
    if target_os == "macos" {
        println!("cargo:rustc-link-lib=c++");
    } else if target_os != "windows" {
        println!("cargo:rustc-link-lib=stdc++");
    }

    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_os == "linux" && target_env == "gnu" {
        println!("cargo:rustc-link-lib=gomp");
    }

    // Link
    println!("cargo:rustc-link-search=native={}/lib", dst.display());
    println!("cargo:rustc-link-lib=static=whisper");
    println!("cargo:rustc-link-lib=static=ggml");
    println!("cargo:rustc-link-lib=static=ggml-base");
    println!("cargo:rustc-link-lib=static=ggml-cpu");

    if target_os == "macos" {
        println!("cargo:rustc-link-lib=static=ggml-blas");
    }

    if metal_enabled {
        println!("cargo:rustc-link-lib=static=ggml-metal");
    }

    if cuda_enabled && !metal_enabled {
        println!("cargo:rustc-link-lib=static=ggml-cuda");
    }

    // On macOS with Metal, we also need to link Foundation and Metal frameworks
    if target_os == "macos" {
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=framework=Accelerate");
    }

    if metal_enabled {
        println!("cargo:rustc-link-lib=framework=Metal");
        println!("cargo:rustc-link-lib=framework=MetalKit");
    }

    // Regenerate if any header or source in whisper.cpp changes
    println!("cargo:rerun-if-env-changed=SDKROOT");
    println!("cargo:rerun-if-env-changed=MACOSX_DEPLOYMENT_TARGET");
    println!(
        "cargo:rerun-if-changed={}",
        crate_dir.join("src/ffi_shim.cpp").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        whisper_dir.join("include").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        whisper_dir.join("src").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        whisper_dir.join("ggml").display()
    );
}
