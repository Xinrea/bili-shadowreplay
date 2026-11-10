//! Bridge native synchronous HLS callbacks to the async recording backend.
//!
//! Only FFmpeg's blocking worker calls this adapter. Awaiting each complete
//! object provides backpressure instead of queueing a live stream in memory.
use std::{
    future::Future,
    io,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::Duration,
};

use crate::startup::RecordingTrace;
use ffmpeg_utils::HlsObjectStore;
use recording_storage::RecordingStorage;

pub(super) const NATIVE_HLS_OBJECT_BYTES: usize = 64 * 1024 * 1024;

pub(super) struct NativeHlsStorage {
    storage: RecordingStorage,
    runtime: tokio::runtime::Handle,
    first_error: Mutex<Option<io::Error>>,
    trace: Option<RecordingTrace>,
    first_media_key: Mutex<Option<String>>,
}

impl NativeHlsStorage {
    pub(super) fn new(storage: RecordingStorage) -> Self {
        Self {
            storage,
            runtime: tokio::runtime::Handle::current(),
            first_error: Mutex::new(None),
            trace: None,
            first_media_key: Mutex::new(None),
        }
    }

    pub(super) fn with_trace(mut self, trace: RecordingTrace) -> Self {
        self.trace = Some(trace);
        self
    }

    /// Keep typed backend errors for recorder callers; the FFI layer only sees
    /// errno values and intentionally does not include backend URL/credentials.
    pub(super) fn take_error(&self) -> Option<io::Error> {
        self.first_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }

    fn wait<T>(
        &self,
        cancelled: &AtomicBool,
        operation: impl Future<Output = io::Result<T>>,
    ) -> io::Result<T> {
        self.runtime.block_on(async {
            if cancelled.load(Ordering::Acquire) {
                return Err(io::ErrorKind::Interrupted.into());
            }
            let cancellation = async {
                while !cancelled.load(Ordering::Acquire) {
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            };
            tokio::select! {
                biased;
                _ = cancellation => Err(io::ErrorKind::Interrupted.into()),
                result = operation => result,
            }
        })
    }

    fn remember<T>(&self, result: io::Result<T>, allow_missing: bool) -> io::Result<T> {
        if let Err(error) = &result {
            if !allow_missing || error.kind() != io::ErrorKind::NotFound {
                let mut first = self
                    .first_error
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if first.is_none() {
                    // Preserve the error kind without retaining secret-bearing
                    // URLs or provider-specific messages across the native API.
                    *first = Some(io::Error::new(error.kind(), "recording backend I/O failed"));
                }
            }
        }
        result
    }
}

impl HlsObjectStore for NativeHlsStorage {
    fn read(&self, key: &str, cancelled: &AtomicBool) -> io::Result<Vec<u8>> {
        self.remember(
            self.wait(
                cancelled,
                self.storage.read_bounded(key, self.max_object_bytes()),
            ),
            true,
        )
    }

    fn write(&self, key: &str, bytes: &[u8], cancelled: &AtomicBool) -> io::Result<()> {
        self.remember(self.wait(cancelled, self.storage.write(key, bytes)), false)?;
        if let Some(trace) = &self.trace {
            if !trace.has_media() {
                let mut first = self
                    .first_media_key
                    .lock()
                    .unwrap_or_else(|p| p.into_inner());
                if first.is_none()
                    && !bytes.is_empty()
                    && (key.ends_with(".ts") || key.ends_with(".m4s"))
                {
                    *first = Some(key.to_owned());
                }
                if key == "playlist.m3u8" {
                    if let Ok((_, playlist)) = m3u8_rs::parse_media_playlist(bytes) {
                        if first
                            .as_ref()
                            .is_some_and(|key| playlist.segments.iter().any(|s| &s.uri == key))
                        {
                            trace.media_committed("flv_segment");
                            *first = None;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn max_object_bytes(&self) -> usize {
        NATIVE_HLS_OBJECT_BYTES
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use recording_storage::{LocalWorkspace, ObjectMetadata, StorageBackend};
    use std::sync::Arc;

    #[tokio::test]
    async fn first_media_requires_new_bytes_and_successful_referencing_playlist() {
        let root = tempfile::tempdir().unwrap();
        let trace = RecordingTrace::new("test", "room", "live");
        let adapter = Arc::new(
            NativeHlsStorage::new(RecordingStorage::local(root.path())).with_trace(trace.clone()),
        );
        let worker = adapter.clone();
        tokio::task::spawn_blocking(move || {
            let cancelled = AtomicBool::new(false);
            let old_playlist = b"#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXTINF:1,\nold.ts\n";
            worker
                .write("playlist.m3u8", old_playlist, &cancelled)
                .unwrap();
            assert!(!trace.has_media(), "old media must not report recovery");
            worker
                .write("new.ts", b"new media bytes", &cancelled)
                .unwrap();
            assert!(
                !trace.has_media(),
                "object alone is not playable publication"
            );
            worker
                .write("playlist.m3u8", old_playlist, &cancelled)
                .unwrap();
            assert!(!trace.has_media());
            let new_playlist = b"#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXTINF:1,\nnew.ts\n";
            // Cancellation must not report a successful publication.
            assert!(worker
                .write("playlist.m3u8", new_playlist, &AtomicBool::new(true))
                .is_err());
            assert!(!trace.has_media());
            worker
                .write("playlist.m3u8", new_playlist, &cancelled)
                .unwrap();
            assert!(trace.has_media());
        })
        .await
        .unwrap();
    }

    struct Pending {
        started: Arc<tokio::sync::Notify>,
    }

    struct BoundedOnly;

    #[async_trait::async_trait]
    impl StorageBackend for BoundedOnly {
        async fn read_range(&self, _: &str, _: std::ops::Range<u64>) -> io::Result<Vec<u8>> {
            unreachable!()
        }
        async fn list(
            &self,
            _: &str,
            _: Option<&str>,
            _: usize,
        ) -> io::Result<recording_storage::ObjectPage> {
            unreachable!()
        }
        async fn read(&self, _: &str) -> io::Result<Vec<u8>> {
            panic!("native I/O must not allocate an unrestricted playlist");
        }
        async fn read_bounded(&self, key: &str, limit: usize) -> io::Result<Vec<u8>> {
            assert_eq!(key, "playlist.m3u8");
            assert_eq!(limit, NATIVE_HLS_OBJECT_BYTES);
            Err(io::ErrorKind::InvalidData.into())
        }
        async fn write(&self, _: &str, _: &[u8]) -> io::Result<()> {
            unreachable!()
        }
        async fn remove(&self, _: &str) -> io::Result<()> {
            unreachable!()
        }
        async fn stat(&self, _: &str) -> io::Result<ObjectMetadata> {
            panic!("a stat preflight is not a bounded read");
        }
    }

    #[tokio::test]
    async fn playlist_limit_is_enforced_by_backend_before_allocation() {
        let storage = RecordingStorage::with_backend(
            Arc::new(BoundedOnly),
            LocalWorkspace::new("unused-native-workspace"),
        );
        let adapter = Arc::new(NativeHlsStorage::new(storage));
        let worker_adapter = adapter.clone();
        let result = tokio::task::spawn_blocking(move || {
            worker_adapter.read("playlist.m3u8", &AtomicBool::new(false))
        })
        .await
        .unwrap();
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            adapter.take_error().unwrap().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[async_trait::async_trait]
    impl StorageBackend for Pending {
        async fn read_range(&self, _: &str, _: std::ops::Range<u64>) -> io::Result<Vec<u8>> {
            unreachable!()
        }
        async fn list(
            &self,
            _: &str,
            _: Option<&str>,
            _: usize,
        ) -> io::Result<recording_storage::ObjectPage> {
            unreachable!()
        }
        async fn read(&self, _: &str) -> io::Result<Vec<u8>> {
            self.started.notify_one();
            std::future::pending().await
        }
        async fn read_bounded(&self, key: &str, _: usize) -> io::Result<Vec<u8>> {
            self.read(key).await
        }
        async fn write(&self, _: &str, _: &[u8]) -> io::Result<()> {
            self.started.notify_one();
            std::future::pending().await
        }
        async fn remove(&self, _: &str) -> io::Result<()> {
            unreachable!()
        }
        async fn stat(&self, _: &str) -> io::Result<ObjectMetadata> {
            unreachable!()
        }
    }

    #[tokio::test]
    async fn cancellation_interrupts_a_pending_backend_operation() {
        let started = Arc::new(tokio::sync::Notify::new());
        let storage = RecordingStorage::with_backend(
            Arc::new(Pending {
                started: started.clone(),
            }),
            LocalWorkspace::new("unused-native-workspace"),
        );
        let adapter = Arc::new(NativeHlsStorage::new(storage));
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_adapter = adapter.clone();
        let worker_cancelled = cancelled.clone();
        let worker = tokio::task::spawn_blocking(move || {
            worker_adapter.write("0.ts", b"data", &worker_cancelled)
        });
        tokio::time::timeout(Duration::from_secs(1), started.notified())
            .await
            .unwrap();
        cancelled.store(true, Ordering::Release);
        let result = tokio::time::timeout(Duration::from_secs(1), worker)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Interrupted);
        assert_eq!(
            adapter.take_error().unwrap().kind(),
            io::ErrorKind::Interrupted
        );
    }

    #[tokio::test]
    async fn missing_initial_playlist_is_not_a_latched_backend_error() {
        let directory = tempfile::tempdir().unwrap();
        let adapter = Arc::new(NativeHlsStorage::new(RecordingStorage::local(
            directory.path(),
        )));
        let worker_adapter = adapter.clone();
        let result = tokio::task::spawn_blocking(move || {
            worker_adapter.read("playlist.m3u8", &AtomicBool::new(false))
        })
        .await
        .unwrap();
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::NotFound);
        assert!(adapter.take_error().is_none());
    }
}
