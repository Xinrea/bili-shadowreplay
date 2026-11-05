//! Forward native FFmpeg warnings/errors to the application's logger. Retain
//! the first errors and the tail of each blocking job for its returned error.
//! Codec-owned threads are logged too, but cannot be attributed to a job's TLS.
use std::{cell::RefCell, collections::VecDeque, ffi::CStr, os::raw::c_char};

const RETAINED_HEAD: usize = 8;
const RETAINED_TAIL: usize = 8;

#[derive(Default)]
struct Diagnostics {
    head: Vec<String>,
    tail: VecDeque<String>,
    omitted: usize,
}

impl Diagnostics {
    fn push(&mut self, message: &str) {
        if self.tail.back().or(self.head.last()).map(String::as_str) == Some(message) {
            return;
        }
        if self.head.len() < RETAINED_HEAD {
            self.head.push(message.into());
        } else {
            if self.tail.len() == RETAINED_TAIL {
                self.tail.pop_front();
                self.omitted += 1;
            }
            self.tail.push_back(message.into());
        }
    }

    fn append_to(self, mut error: String) -> String {
        if !self.head.is_empty() {
            error.push_str("\nFFmpeg diagnostics:\n");
            error.push_str(&self.head.join("\n"));
            if self.omitted > 0 {
                error.push_str(&format!(
                    "\n[{} intermediate messages omitted]",
                    self.omitted
                ));
            }
            for message in self.tail {
                error.push('\n');
                error.push_str(&message);
            }
        }
        error
    }
}

thread_local! {
    static ACTIVE: RefCell<Option<Diagnostics>> = const { RefCell::new(None) };
}

struct Capture(Option<Diagnostics>);

impl Drop for Capture {
    fn drop(&mut self) {
        // Restore even when the job panics, so a reused Tokio worker cannot
        // leak one recording's errors into a later recording.
        ACTIVE.with(|active| active.replace(self.0.take()));
    }
}

pub(crate) fn capture<T>(work: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    let _capture = Capture(ACTIVE.with(|active| active.replace(Some(Diagnostics::default()))));
    let result = work();
    let diagnostics = ACTIVE
        .with(|active| active.borrow_mut().take())
        .unwrap_or_default();
    result.map_err(|error| diagnostics.append_to(error))
}

extern "C" {
    fn bsr_install_ffmpeg_log_callback();
}

pub(crate) fn install() {
    // SAFETY: initialize's OnceLock installs this once before media work starts.
    unsafe { bsr_install_ffmpeg_log_callback() };
}

#[no_mangle]
unsafe extern "C" fn bsr_ffmpeg_log_message(level: i32, message: *const c_char) {
    // A logger panic must never unwind through a libavcodec/libavfilter stack.
    let _ = std::panic::catch_unwind(|| {
        // SAFETY: the C bridge passes a live NUL-terminated stack buffer.
        let message = unsafe { CStr::from_ptr(message) }.to_string_lossy();
        let message = message.trim();
        if message.is_empty() {
            return;
        }
        if level <= ffmpeg_next::ffi::AV_LOG_ERROR {
            let _ = ACTIVE.try_with(|active| {
                if let Ok(mut active) = active.try_borrow_mut() {
                    if let Some(diagnostics) = active.as_mut() {
                        diagnostics.push(message);
                    }
                }
            });
            log::error!(target: "ffmpeg", "[FFmpeg] {message}");
        } else {
            log::warn!(target: "ffmpeg", "[FFmpeg] {message}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        ffi::CString,
        sync::{Barrier, Mutex},
    };

    struct TestLogger(Mutex<Vec<(log::Level, String)>>);
    impl log::Log for TestLogger {
        fn enabled(&self, _: &log::Metadata<'_>) -> bool {
            true
        }
        fn log(&self, record: &log::Record<'_>) {
            let message = record.args().to_string();
            if message.contains("diagnostics logger test") {
                self.0.lock().unwrap().push((record.level(), message));
            }
        }
        fn flush(&self) {}
    }

    #[test]
    fn native_diagnostics_forward_codec_thread_warnings_and_errors_to_logger() {
        static LOGGER: TestLogger = TestLogger(Mutex::new(Vec::new()));
        log::set_logger(&LOGGER).unwrap();
        log::set_max_level(log::LevelFilter::Warn);
        // Codec-owned threads have no job capture. Their diagnostics must
        // still reach the application's persistent logger.
        std::thread::spawn(|| {
            emit(
                ffmpeg_next::ffi::AV_LOG_WARNING,
                "diagnostics logger test warning",
            );
            emit(
                ffmpeg_next::ffi::AV_LOG_ERROR,
                "diagnostics logger test error",
            );
        })
        .join()
        .unwrap();
        assert_eq!(
            *LOGGER.0.lock().unwrap(),
            vec![
                (
                    log::Level::Warn,
                    "[FFmpeg] diagnostics logger test warning".into()
                ),
                (
                    log::Level::Error,
                    "[FFmpeg] diagnostics logger test error".into()
                ),
            ]
        );
    }

    fn emit(level: i32, message: &str) {
        crate::metadata::initialize().unwrap();
        let message = CString::new(message).unwrap();
        // Exercise the real C va_list formatter, including Unicode and '%'.
        unsafe {
            ffmpeg_next::ffi::av_log(
                std::ptr::null_mut(),
                level,
                c"%s\n".as_ptr(),
                message.as_ptr(),
            );
        }
    }

    #[test]
    fn native_diagnostics_keep_the_first_cause_and_tail_with_bounded_storage() {
        let error = capture::<()>(|| {
            emit(ffmpeg_next::ffi::AV_LOG_WARNING, "recoverable warning");
            emit(
                ffmpeg_next::ffi::AV_LOG_ERROR,
                "录播 50%: SAR 1:1 != 1215:1216",
            );
            for index in 0..40 {
                emit(
                    ffmpeg_next::ffi::AV_LOG_ERROR,
                    &format!("follow-up {index}"),
                );
            }
            emit(ffmpeg_next::ffi::AV_LOG_ERROR, "Nothing was written");
            Err("配置共享滤镜图失败: Invalid argument".into())
        })
        .unwrap_err();
        assert!(error.starts_with("配置共享滤镜图失败: Invalid argument"));
        assert!(error.contains("录播 50%: SAR 1:1 != 1215:1216"));
        assert!(error.ends_with("Nothing was written"));
        assert!(error.contains("26 intermediate messages omitted"));
        assert!(!error.contains("recoverable warning"));
        assert!(!error.contains("follow-up 20\n"));
    }

    #[test]
    fn native_diagnostics_do_not_mix_concurrent_jobs_or_reused_workers() {
        let barrier = Barrier::new(2);
        std::thread::scope(|scope| {
            for label in ["first recording", "second recording"] {
                let barrier = &barrier;
                scope.spawn(move || {
                    let error = capture::<()>(|| {
                        emit(ffmpeg_next::ffi::AV_LOG_ERROR, label);
                        barrier.wait();
                        Err("failed".into())
                    })
                    .unwrap_err();
                    assert_eq!(error, format!("failed\nFFmpeg diagnostics:\n{label}"));
                    assert_eq!(
                        capture::<()>(|| Err("next job".into())).unwrap_err(),
                        "next job"
                    );
                });
            }
        });
    }

    #[test]
    fn native_diagnostics_restore_capture_after_success_and_panic() {
        capture(|| {
            emit(ffmpeg_next::ffi::AV_LOG_ERROR, "recovered error");
            Ok(())
        })
        .unwrap();
        assert_eq!(capture::<()>(|| Err("clean".into())).unwrap_err(), "clean");
        let _ = std::panic::catch_unwind(|| {
            let _ = capture::<()>(|| {
                emit(ffmpeg_next::ffi::AV_LOG_ERROR, "panicked job");
                panic!("simulated job panic");
            });
        });
        assert_eq!(capture::<()>(|| Err("clean".into())).unwrap_err(), "clean");
    }
}
