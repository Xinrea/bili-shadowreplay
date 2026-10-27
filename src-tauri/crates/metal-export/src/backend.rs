use std::{
    ffi::{c_char, CStr, CString},
    path::{Path, PathBuf},
    sync::OnceLock,
};

type Export = unsafe extern "C" fn(*const c_char) -> i32;
type Probe = unsafe extern "C" fn() -> i32;
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
fn select(directories: &[PathBuf]) -> Result<(&'static Adapter, &Path), String> {
    for adapter in ADAPTERS {
        for directory in directories {
            if LIBRARIES
                .iter()
                .zip(adapter.versions)
                .all(|(name, version)| {
                    directory
                        .join(format!("lib{name}.{version}.dylib"))
                        .is_file()
                })
            {
                return Ok((adapter, directory));
            }
        }
    }
    Err(format!(
        "No matching FFmpeg libraries in {directories:?}. This build includes libavcodec ABIs: {:?}.",
        ADAPTERS.iter().map(|adapter| adapter.versions[1]).collect::<Vec<_>>()
    ))
}

fn load() -> Result<Backend, String> {
    // An explicit directory is useful for isolated validation and non-default
    // Homebrew prefixes. It must contain a complete, single-ABI library set.
    let directories = if let Some(directory) = std::env::var_os("BSR_FFMPEG_LIBRARY_DIR") {
        vec![PathBuf::from(directory)]
    } else {
        let prefix = if cfg!(target_arch = "aarch64") {
            "/opt/homebrew"
        } else {
            "/usr/local"
        };
        let mut directories: Vec<_> = ["ffmpeg", "ffmpeg@8"]
            .map(|formula| PathBuf::from(prefix).join("opt").join(formula).join("lib"))
            .into();
        directories.extend(
            ADAPTERS
                .iter()
                .map(|adapter| PathBuf::from(adapter.directory)),
        );
        directories
    };
    let (adapter, directory) = select(&directories)?;
    use std::os::unix::ffi::OsStrExt;
    let path = CString::new(directory.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
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

#[cfg(test)]
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
