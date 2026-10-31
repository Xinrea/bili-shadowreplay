//! Load the application's MoltenVK before FFmpeg resolves its Vulkan entry
//! points. No process-global loader environment or installed ICD is required.
use std::{
    ffi::{c_char, c_void, CStr, CString},
    path::PathBuf,
    sync::OnceLock,
};

unsafe extern "C" {
    fn dlopen(path: *const c_char, flags: i32) -> *mut c_void;
    fn dlerror() -> *const c_char;
}

pub(super) fn prepare() -> Result<(), String> {
    static LOADED: OnceLock<Result<(), String>> = OnceLock::new();
    LOADED
        .get_or_init(|| {
            let packaged = std::env::current_exe().ok().and_then(|exe| {
                let contents = exe.parent()?.parent()?;
                (contents.file_name()? == "Contents")
                    .then(|| contents.join("Frameworks/libvulkan.dylib"))
            });
            // Developer override can also select the official Vulkan loader for
            // validation/API capture. Packaged applications always use their own
            // signed MoltenVK, even when the launch environment has an override.
            let path =
                packaged.or_else(|| std::env::var_os("BSR_VULKAN_LIBRARY").map(PathBuf::from));
            let Some(path) = path else {
                return Ok(());
            };
            if !path.is_absolute()
                || path
                    .file_name()
                    .is_none_or(|name| name != "libvulkan.dylib")
            {
                return Err(
                    "Vulkan runtime must be an absolute path ending in libvulkan.dylib".into(),
                );
            }
            let name = CString::new(path.as_os_str().as_encoded_bytes())
                .map_err(|error| error.to_string())?;
            unsafe {
                // Keep this reference for the process lifetime: Skia/libplacebo
                // retain borrowed dispatch pointers beyond individual frames.
                let loaded = dlopen(name.as_ptr(), 2 | 4); // RTLD_NOW | RTLD_LOCAL on Darwin
                if loaded.is_null() {
                    let error = dlerror();
                    let error = if error.is_null() {
                        "unknown loader error".into()
                    } else {
                        CStr::from_ptr(error).to_string_lossy()
                    };
                    return Err(format!("Load bundled MoltenVK {}: {error}", path.display()));
                }
                if dlopen(c"libvulkan.dylib".as_ptr(), 2 | 4) != loaded {
                    return Err("Another Vulkan runtime was loaded before bundled MoltenVK".into());
                }
            }
            Ok(())
        })
        .clone()
}
