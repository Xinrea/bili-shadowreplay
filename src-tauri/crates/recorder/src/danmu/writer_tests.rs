use super::*;
use recording_storage::{LocalWorkspace, ObjectMetadata, ObjectPage, StorageBackend};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

mod batching;

/// Remote-style log backend: historical reads hang unless explicitly enabled.
/// Recording startup must not touch the legacy base or any committed chunk.
pub(crate) struct HistoryBackend {
    pub(crate) objects: Mutex<BTreeMap<String, Vec<u8>>>,
    pub(crate) allow_history: AtomicBool,
    pub(crate) history_reads: AtomicUsize,
    pub(crate) head_reads: AtomicUsize,
    writes: AtomicUsize,
    fail_writes: AtomicBool,
    fail_head: AtomicBool,
    pause_write: AtomicBool,
    writing: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

impl HistoryBackend {
    pub(crate) fn new(prefix: &str, count: u64) -> Self {
        let directory = format!("{prefix}.recording-logs/6576656e74732e6a736f6e6c/log");
        let mut objects = BTreeMap::new();
        objects.insert(format!("{directory}/head"), count.to_le_bytes().to_vec());
        for index in 0..count {
            let event = LiveEvent {
                ts: index as i64,
                platform: "bilibili".into(),
                room_id: "room".into(),
                event_type: "danmu".into(),
                data: serde_json::json!({"content":"history"}),
                raw: Value::Null,
            };
            objects.insert(
                format!("{directory}/{index:020}"),
                format!("{}\n", serde_json::to_string(&event).unwrap()).into_bytes(),
            );
        }
        Self {
            objects: Mutex::new(objects),
            allow_history: AtomicBool::new(false),
            history_reads: AtomicUsize::new(0),
            head_reads: AtomicUsize::new(0),
            writes: AtomicUsize::new(0),
            fail_writes: AtomicBool::new(false),
            fail_head: AtomicBool::new(false),
            pause_write: AtomicBool::new(false),
            writing: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
        }
    }

    fn storage(self: &Arc<Self>) -> RecordingStorage {
        RecordingStorage::with_backend(self.clone(), LocalWorkspace::new("unused-history-probe"))
    }
}

#[async_trait::async_trait]
impl StorageBackend for HistoryBackend {
    async fn read(&self, key: &str) -> io::Result<Vec<u8>> {
        let history = key.ends_with("events.jsonl")
            || (key.contains(".recording-logs/") && !key.ends_with("/head"));
        if history {
            self.history_reads.fetch_add(1, Ordering::Relaxed);
            if !self.allow_history.load(Ordering::Relaxed) {
                std::future::pending::<()>().await;
            }
        }
        if key.ends_with("/head") {
            self.head_reads.fetch_add(1, Ordering::Relaxed);
        }
        self.objects
            .lock()
            .unwrap()
            .get(key)
            .cloned()
            .ok_or_else(|| io::ErrorKind::NotFound.into())
    }
    async fn read_bounded(&self, key: &str, max: usize) -> io::Result<Vec<u8>> {
        let bytes = self.read(key).await?;
        if bytes.len() > max {
            return Err(io::ErrorKind::InvalidData.into());
        }
        Ok(bytes)
    }
    async fn read_range(&self, _: &str, _: std::ops::Range<u64>) -> io::Result<Vec<u8>> {
        unreachable!("startup must not range-read event history")
    }
    async fn write(&self, key: &str, bytes: &[u8]) -> io::Result<()> {
        self.writes.fetch_add(1, Ordering::Relaxed);
        if self.pause_write.swap(false, Ordering::Relaxed) {
            self.writing.notify_one();
            self.release.notified().await;
        }
        if self.fail_writes.load(Ordering::Relaxed)
            || (key.ends_with("/head") && self.fail_head.load(Ordering::Relaxed))
        {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        self.objects
            .lock()
            .unwrap()
            .insert(key.into(), bytes.to_vec());
        Ok(())
    }
    async fn remove(&self, key: &str) -> io::Result<()> {
        self.objects.lock().unwrap().remove(key);
        Ok(())
    }
    async fn stat(&self, key: &str) -> io::Result<ObjectMetadata> {
        self.objects
            .lock()
            .unwrap()
            .get(key)
            .map(|v| ObjectMetadata {
                size: v.len() as u64,
            })
            .ok_or_else(|| io::ErrorKind::NotFound.into())
    }
    async fn list(&self, _: &str, _: Option<&str>, _: usize) -> io::Result<ObjectPage> {
        unreachable!("startup must not enumerate history objects")
    }
}

#[tokio::test]
async fn append_open_cost_is_constant_and_never_reads_history() {
    for count in [0, 10, 10_000] {
        let backend = Arc::new(HistoryBackend::new("", count));
        let storage = backend.storage();
        let writer = tokio::time::timeout(
            Duration::from_secs(1),
            DanmuWriter::from_storage(&storage, "events.jsonl"),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(backend.head_reads.load(Ordering::Relaxed), 1);
        assert_eq!(backend.history_reads.load(Ordering::Relaxed), 0);
        assert_eq!(backend.writes.load(Ordering::Relaxed), 0);
        writer.close().await.unwrap();
    }
}

#[tokio::test]
async fn resumed_append_preserves_base_chunks_and_cursor() {
    let backend = Arc::new(HistoryBackend::new("", 2));
    backend
        .objects
        .lock()
        .unwrap()
        .insert("events.jsonl".into(), b"0:legacy base\n".to_vec());
    let old = backend.objects.lock().unwrap().clone();
    let storage = backend.storage();
    let writer = DanmuWriter::from_storage(&storage, "events.jsonl")
        .await
        .unwrap();
    writer
        .add_event(&LiveEvent {
            ts: 2,
            platform: "bilibili".into(),
            room_id: "room".into(),
            event_type: "danmu".into(),
            data: serde_json::json!({"content":"new"}),
            raw: Value::Null,
        })
        .await
        .unwrap();
    writer.close().await.unwrap();
    assert_eq!(backend.history_reads.load(Ordering::Relaxed), 0);
    for (key, value) in old {
        if !key.ends_with("/head") {
            assert_eq!(backend.objects.lock().unwrap()[&key], value);
        }
    }
    backend.allow_history.store(true, Ordering::Relaxed);
    let entries = DanmuStorage::read_entries(&storage, "events.jsonl")
        .await
        .unwrap();
    assert_eq!(
        entries
            .iter()
            .map(|e| e.content.as_str())
            .collect::<Vec<_>>(),
        ["legacy base", "history", "history", "new"]
    );
}

#[tokio::test]
async fn corrupt_cursor_is_an_error_not_permission_to_reset_history() {
    let backend = Arc::new(HistoryBackend::new("", 2));
    let head = ".recording-logs/6576656e74732e6a736f6e6c/log/head";
    backend
        .objects
        .lock()
        .unwrap()
        .insert(head.into(), b"bad".to_vec());
    let before = backend.objects.lock().unwrap().clone();
    assert!(
        DanmuWriter::from_storage(&backend.storage(), "events.jsonl")
            .await
            .is_err()
    );
    assert_eq!(*backend.objects.lock().unwrap(), before);
    assert_eq!(backend.writes.load(Ordering::Relaxed), 0);
}
