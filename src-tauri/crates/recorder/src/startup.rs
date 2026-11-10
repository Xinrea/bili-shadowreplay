//! Per-attempt startup diagnostics. Never log URLs, cookies or event contents.
use std::{
    fmt,
    future::Future,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Instant,
};

#[derive(Clone)]
pub(crate) struct RecordingTrace(Arc<Attempt>);

struct Attempt {
    platform: String,
    room: String,
    live: String,
    id: uuid::Uuid,
    started: Instant,
    first_media: AtomicBool,
}

impl fmt::Display for RecordingTrace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}][{}][{}][attempt={}]",
            self.0.platform, self.0.room, self.0.live, self.0.id
        )
    }
}

impl RecordingTrace {
    pub(crate) fn new(platform: &str, room: &str, live: &str) -> Self {
        let trace = Self(Arc::new(Attempt {
            platform: platform.into(),
            room: room.into(),
            live: live.into(),
            id: uuid::Uuid::new_v4(),
            started: Instant::now(),
            first_media: AtomicBool::new(false),
        }));
        log::info!("{trace} Recording preparation started (not yet receiving video)");
        trace
    }

    pub(crate) async fn stage<T, E>(
        &self,
        name: &'static str,
        operation: impl Future<Output = Result<T, E>>,
    ) -> Result<T, E> {
        log::info!("{self} Startup stage started: stage={name}");
        let mut stage = Stage {
            trace: self,
            name,
            started: Instant::now(),
            finished: false,
        };
        let result = operation.await;
        stage.finished = true;
        let elapsed = stage.started.elapsed().as_millis();
        let total = self.0.started.elapsed().as_millis();
        if result.is_ok() {
            log::info!("{self} Startup stage completed: stage={name} duration_ms={elapsed} total_ms={total}");
        } else {
            // The caller handles the error; do not print a secret-bearing provider URL.
            log::warn!(
                "{self} Startup stage failed: stage={name} duration_ms={elapsed} total_ms={total}"
            );
        }
        result
    }

    /// Only called after new media AND its referencing playlist are published.
    pub(crate) fn media_committed(&self, source: &'static str) {
        if !self.0.first_media.swap(true, Ordering::AcqRel) {
            log::info!(
                "{self} First new media committed: source={source} startup_ms={}",
                self.0.started.elapsed().as_millis()
            );
        }
    }

    pub(crate) fn has_media(&self) -> bool {
        self.0.first_media.load(Ordering::Acquire)
    }
}

struct Stage<'a> {
    trace: &'a RecordingTrace,
    name: &'static str,
    started: Instant,
    finished: bool,
}

impl Drop for Stage<'_> {
    fn drop(&mut self) {
        if !self.finished {
            log::warn!(
                "{} Startup stage cancelled: stage={} duration_ms={} total_ms={}",
                self.trace,
                self.name,
                self.started.elapsed().as_millis(),
                self.trace.0.started.elapsed().as_millis()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, Once};

    struct Capture(Mutex<Vec<String>>);
    static CAPTURE: Capture = Capture(Mutex::new(Vec::new()));
    static INIT: Once = Once::new();
    impl log::Log for Capture {
        fn enabled(&self, meta: &log::Metadata<'_>) -> bool {
            meta.target() == module_path!().trim_end_matches("::tests")
        }
        fn log(&self, record: &log::Record<'_>) {
            if self.enabled(record.metadata()) {
                self.0.lock().unwrap().push(record.args().to_string());
            }
        }
        fn flush(&self) {}
    }

    #[tokio::test]
    async fn diagnostics_distinguish_completion_failure_cancellation_and_first_media() {
        // Other platform tests install env_logger. Isolate the process-global
        // logger instead of making this assertion depend on test execution order.
        if std::env::var_os("BSR_STARTUP_LOG_TEST_CHILD").is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "startup::tests::diagnostics_distinguish_completion_failure_cancellation_and_first_media", "--nocapture"])
                .env("BSR_STARTUP_LOG_TEST_CHILD", "1")
                .output().unwrap();
            assert!(
                output.status.success(),
                "child failed: {} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        INIT.call_once(|| {
            log::set_logger(&CAPTURE).unwrap();
            log::set_max_level(log::LevelFilter::Info);
        });
        let trace = RecordingTrace::new("test", "room", "live");
        let prefix = trace.to_string();
        assert_eq!(
            trace
                .stage("success", async { Ok::<_, ()>(42) })
                .await
                .unwrap(),
            42
        );
        assert!(trace
            .stage("failure", async {
                Err::<(), _>("test-only-secret-bearing-error")
            })
            .await
            .is_err());
        assert!(tokio::time::timeout(
            std::time::Duration::from_millis(5),
            trace.stage("blocked", std::future::pending::<Result<(), ()>>())
        )
        .await
        .is_err());
        trace.media_committed("test");
        trace.clone().media_committed("test");
        let messages: Vec<_> = CAPTURE
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|m| m.starts_with(&prefix))
            .cloned()
            .collect();
        assert!(messages
            .iter()
            .any(|m| m.contains("stage=success duration_ms=") && m.contains("completed")));
        assert!(messages
            .iter()
            .any(|m| m.contains("stage=failure duration_ms=") && m.contains("failed")));
        assert!(messages
            .iter()
            .any(|m| m.contains("stage=blocked duration_ms=") && m.contains("cancelled")));
        assert_eq!(
            messages
                .iter()
                .filter(|m| m.contains("First new media committed"))
                .count(),
            1
        );
        assert!(!messages.iter().any(|m| m.contains("secret-bearing")));
    }
}
