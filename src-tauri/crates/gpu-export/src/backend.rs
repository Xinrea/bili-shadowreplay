use super::ExportConfig;
use std::{
    ffi::{c_char, CStr, CString},
    path::{Path, PathBuf},
    sync::OnceLock,
};

type Export = unsafe extern "C" fn(*const ExportConfig, *mut c_char, usize) -> i32;
type Probe = unsafe extern "C" fn(*mut c_char, usize, *mut i32) -> i32;
type Load = unsafe extern "C" fn(*const c_char) -> *const c_char;
struct Adapter {
    versions: [u32; 4],
    directory: &'static str,
    load: Load,
    export: Export,
    probe: Probe,
}
include!(concat!(env!("OUT_DIR"), "/backends.rs"));
pub(crate) struct Backend {
    pub export: Export,
    pub probe: Probe,
}

const LIBRARIES: [&str; 4] = ["avformat", "avcodec", "avfilter", "avutil"];
fn library_name(name: &str, version: u32) -> String {
    if cfg!(target_os = "windows") {
        format!("{name}-{version}.dll")
    } else if cfg!(target_os = "macos") {
        format!("lib{name}.{version}.dylib")
    } else {
        format!("lib{name}.so.{version}")
    }
}
fn select(directories: &[PathBuf]) -> Result<(&'static Adapter, &Path), String> {
    for adapter in ADAPTERS {
        for directory in directories {
            if LIBRARIES
                .iter()
                .zip(adapter.versions)
                .all(|(name, version)| directory.join(library_name(name, version)).is_file())
            {
                return Ok((adapter, directory));
            }
        }
    }
    Err(format!("No complete matching FFmpeg libraries in {directories:?}. This build includes libavcodec ABIs: {:?}.", ADAPTERS.iter().map(|v| v.versions[1]).collect::<Vec<_>>()))
}

fn load() -> Result<Backend, String> {
    let directories = if let Some(directory) = std::env::var_os("BSR_FFMPEG_LIBRARY_DIR") {
        vec![PathBuf::from(directory)]
    } else {
        let mut paths = ADAPTERS
            .iter()
            .map(|v| PathBuf::from(v.directory))
            .collect::<Vec<_>>();
        if let Some(parent) = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
        {
            #[cfg(target_os = "linux")]
            paths.push(parent.join("../lib/bili-shadowreplay/ffmpeg-runtime"));
            paths.push(parent);
        }
        #[cfg(target_os = "macos")]
        {
            let prefix = if cfg!(target_arch = "aarch64") {
                "/opt/homebrew"
            } else {
                "/usr/local"
            };
            paths.splice(
                0..0,
                ["ffmpeg", "ffmpeg@8", "ffmpeg@9"]
                    .map(|v| PathBuf::from(prefix).join("opt").join(v).join("lib")),
            );
        }
        #[cfg(target_os = "linux")]
        {
            paths.extend(["/usr/local/lib", "/usr/lib64", "/usr/lib"].map(PathBuf::from));
            paths.push(PathBuf::from(format!(
                "/usr/lib/{}-linux-gnu",
                std::env::consts::ARCH
            )));
        }
        #[cfg(not(target_os = "macos"))]
        if let Some(value) = std::env::var_os(if cfg!(target_os = "windows") {
            "PATH"
        } else {
            "LD_LIBRARY_PATH"
        }) {
            paths.extend(std::env::split_paths(&value).filter(|p| p.is_absolute()));
        }
        paths
    };
    let (adapter, directory) = select(&directories)?;
    let directory = directory.canonicalize().map_err(|e| e.to_string())?;
    #[cfg(unix)]
    let path = {
        use std::os::unix::ffi::OsStrExt;
        CString::new(directory.as_os_str().as_bytes()).map_err(|e| e.to_string())?
    };
    #[cfg(windows)]
    let path = CString::new(
        directory
            .to_str()
            .ok_or("FFmpeg directory must be valid Unicode")?,
    )
    .map_err(|e| e.to_string())?;
    let error = unsafe { (adapter.load)(path.as_ptr()) };
    if !error.is_null() {
        return Err(unsafe { CStr::from_ptr(error) }
            .to_string_lossy()
            .into_owned());
    }
    Ok(Backend {
        export: adapter.export,
        probe: adapter.probe,
    })
}

pub(crate) fn get() -> Result<&'static Backend, String> {
    static BACKEND: OnceLock<Result<Backend, String>> = OnceLock::new();
    BACKEND.get_or_init(load).as_ref().map_err(Clone::clone)
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn select_complete_abis_without_mixing_libraries() {
        let root = std::env::temp_dir().join(format!("bsr-abi-selection-{}", std::process::id()));
        let directories: Vec<_> = (0..ADAPTERS.len())
            .map(|i| root.join(i.to_string()))
            .collect();
        for directory in &directories {
            std::fs::create_dir_all(directory).unwrap();
        }
        assert!(select(&directories).is_err());
        for (adapter, directory) in ADAPTERS.iter().zip(&directories).rev() {
            for (name, version) in LIBRARIES.iter().zip(adapter.versions) {
                std::fs::write(directory.join(format!("lib{name}.{version}.dylib")), []).unwrap();
            }
            let (selected, path) = select(&directories).unwrap();
            assert_eq!(selected.versions, adapter.versions);
            assert_eq!(path, directory.as_path());
        }
        for (adapter, directory) in ADAPTERS.iter().zip(&directories) {
            std::fs::remove_file(
                directory.join(format!("libavutil.{}.dylib", adapter.versions[3])),
            )
            .unwrap();
        }
        assert!(select(&directories).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
