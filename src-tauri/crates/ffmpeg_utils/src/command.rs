use std::path::Path;

// Compile subprocess support only for unit tests or explicitly enabled oracles.
#[cfg(any(test, feature = "cli-test-oracles"))]
mod cli {
    use std::path::PathBuf;

    /// Path of the external `ffmpeg` executable used by optional test oracles.
    pub fn ffmpeg_path() -> PathBuf {
        executable_path("ffmpeg")
    }

    /// Path of the external `ffprobe` executable used by optional test oracles.
    pub fn ffprobe_path() -> PathBuf {
        executable_path("ffprobe")
    }

    fn executable_path(name: &str) -> PathBuf {
        let mut path = PathBuf::from(name);
        if cfg!(windows) {
            path.set_extension("exe");
        }

        path
    }

    /// A test-oracle `ffmpeg` process, killed on drop and hidden on Windows.
    pub fn ffmpeg_command() -> tokio::process::Command {
        base_command(ffmpeg_path())
    }

    /// A test-oracle `ffprobe` process, killed on drop and hidden on Windows.
    pub fn ffprobe_command() -> tokio::process::Command {
        base_command(ffprobe_path())
    }

    fn base_command(program: PathBuf) -> tokio::process::Command {
        let mut command = tokio::process::Command::new(program);
        command.kill_on_drop(true);
        hide_console_window(&mut command);
        command
    }

    /// Windows opens a console window for every child process unless told
    /// otherwise, which would flash in front of the application.
    #[cfg(target_os = "windows")]
    fn hide_console_window(command: &mut tokio::process::Command) {
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    #[cfg(not(target_os = "windows"))]
    fn hide_console_window(_command: &mut tokio::process::Command) {}
}

#[cfg(any(test, feature = "cli-test-oracles"))]
pub use cli::{ffmpeg_command, ffmpeg_path, ffprobe_command, ffprobe_path};

/// Borrow a UTF-8 media path for native FFmpeg calls and filter options.
///
/// This crate passes paths through FFmpeg string interfaces as UTF-8. Reject
/// invalid paths instead of silently replacing bytes in the file name.
pub fn path_str(path: &Path) -> Result<&str, String> {
    path.to_str()
        .ok_or_else(|| format!("Path is not valid UTF-8: {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ffmpeg_paths_are_platform_specific() {
        let ffmpeg = ffmpeg_path();
        let ffprobe = ffprobe_path();

        #[cfg(windows)]
        {
            assert_eq!(ffmpeg.extension().unwrap(), "exe");
            assert_eq!(ffprobe.extension().unwrap(), "exe");
        }

        #[cfg(not(windows))]
        {
            assert_eq!(ffmpeg.file_name().unwrap(), "ffmpeg");
            assert_eq!(ffprobe.file_name().unwrap(), "ffprobe");
        }
    }

    #[test]
    fn path_str_rejects_a_path_that_is_not_utf8() {
        #[cfg(unix)]
        {
            use std::ffi::OsStr;
            use std::os::unix::ffi::OsStrExt;

            let invalid = Path::new(OsStr::from_bytes(b"video-\xFF.mp4"));
            assert!(path_str(invalid).is_err());
        }

        assert_eq!(path_str(Path::new("video.mp4")).unwrap(), "video.mp4");
    }
}
