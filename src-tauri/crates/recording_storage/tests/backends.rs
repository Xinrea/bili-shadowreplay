use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use recording_storage::{
    LocalAssets, LocalBackend, LocalWorkspace, ObjectMetadata, ObjectPage, RecordingStorage,
    RecordingStore, StorageBackend, LOG_CHUNK_BYTES,
};

#[derive(Default)]
struct Memory {
    objects: Mutex<BTreeMap<String, Vec<u8>>>,
    writes: Mutex<Vec<(String, usize)>>,
    fail_remove: Mutex<Option<String>>,
    fail_write: Mutex<Option<String>>,
    fail_read: Mutex<Option<String>>,
    pause_next_write: std::sync::atomic::AtomicBool,
    started: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

#[async_trait]
impl StorageBackend for Memory {
    async fn read_range(&self, key: &str, range: std::ops::Range<u64>) -> io::Result<Vec<u8>> {
        if range.start > range.end {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let objects = self.objects.lock().unwrap();
        let bytes = objects.get(key).ok_or(io::ErrorKind::NotFound)?;
        let start = range.start.min(bytes.len() as u64) as usize;
        let end = range.end.min(bytes.len() as u64) as usize;
        Ok(bytes[start..end].to_vec())
    }

    async fn read(&self, key: &str) -> io::Result<Vec<u8>> {
        self.objects
            .lock()
            .unwrap()
            .get(key)
            .cloned()
            .ok_or(io::ErrorKind::NotFound.into())
    }

    async fn read_bounded(&self, key: &str, max: usize) -> io::Result<Vec<u8>> {
        if self.fail_read.lock().unwrap().as_deref() == Some(key) {
            return Err(io::ErrorKind::TimedOut.into());
        }
        let objects = self.objects.lock().unwrap();
        let bytes = objects.get(key).ok_or(io::ErrorKind::NotFound)?;
        if bytes.len() > max {
            return Err(io::ErrorKind::InvalidData.into());
        }
        Ok(bytes.clone())
    }

    async fn write(&self, key: &str, bytes: &[u8]) -> io::Result<()> {
        if self
            .pause_next_write
            .swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            self.started.notify_one();
            self.release.notified().await;
        }
        if self
            .fail_write
            .lock()
            .unwrap()
            .as_deref()
            .is_some_and(|suffix| key.ends_with(suffix))
        {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        self.writes
            .lock()
            .unwrap()
            .push((key.to_owned(), bytes.len()));
        self.objects
            .lock()
            .unwrap()
            .insert(key.to_owned(), bytes.to_vec());
        Ok(())
    }

    async fn remove(&self, key: &str) -> io::Result<()> {
        if self.fail_remove.lock().unwrap().as_deref() == Some(key) {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        self.objects.lock().unwrap().remove(key);
        Ok(())
    }

    async fn stat(&self, key: &str) -> io::Result<ObjectMetadata> {
        Ok(ObjectMetadata {
            size: self.read(key).await?.len() as u64,
        })
    }

    async fn list(
        &self,
        prefix: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> io::Result<ObjectPage> {
        let objects = self.objects.lock().unwrap();
        let mut matched = objects.keys().filter(|key| {
            (prefix.is_empty() || key.as_str() == prefix || key.starts_with(&format!("{prefix}/")))
                && cursor.is_none_or(|cursor| key.as_str() > cursor)
        });
        let keys: Vec<_> = matched.by_ref().take(limit).cloned().collect();
        let next_cursor = if matched.next().is_some() {
            keys.last().cloned()
        } else {
            None
        };
        Ok(ObjectPage { keys, next_cursor })
    }
}

#[tokio::test]
async fn empty_log_head_requires_zero_count_without_base_or_orphan_chunks() {
    let backend = Arc::new(Memory::default());
    let directory = tempfile::tempdir().unwrap();
    let storage =
        RecordingStorage::with_backend(backend.clone(), LocalWorkspace::new(directory.path()));
    for key in ["events.jsonl", "nested/events.jsonl"] {
        assert!(storage.empty_log_head(key).await.unwrap().is_none());
        let log = storage.open_log(key).await.unwrap();
        let head = storage.empty_log_head(key).await.unwrap().unwrap();
        assert_eq!(storage.read(&head).await.unwrap(), 0u64.to_le_bytes());
        storage.write(key, b"legacy event").await.unwrap();
        assert!(storage.empty_log_head(key).await.unwrap().is_none());
        storage.write(key, b"").await.unwrap();
        assert_eq!(
            storage.empty_log_head(key).await.unwrap(),
            Some(head.clone())
        );

        let chunk = format!(
            "{}/00000000000000000000",
            head.strip_suffix("/head").unwrap()
        );
        for bytes in [b"uncommitted event".as_slice(), b"".as_slice()] {
            storage.write(&chunk, bytes).await.unwrap();
            assert!(storage.empty_log_head(key).await.unwrap().is_none());
            storage.remove(&chunk).await.unwrap();
        }
        log.append(b"committed event").await.unwrap();
        assert!(storage.empty_log_head(key).await.unwrap().is_none());
        storage.remove(&chunk).await.unwrap();
        assert!(
            storage.empty_log_head(key).await.unwrap().is_none(),
            "a nonzero head with a missing chunk is not empty"
        );
    }
    let local = RecordingStorage::local(directory.path());
    local.open_log("events.jsonl").await.unwrap();
    assert!(local
        .empty_log_head("events.jsonl")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn empty_log_head_propagates_invalid_metadata_and_read_failures() {
    let backend = Arc::new(Memory::default());
    let directory = tempfile::tempdir().unwrap();
    let storage =
        RecordingStorage::with_backend(backend.clone(), LocalWorkspace::new(directory.path()));
    storage.open_log("events.jsonl").await.unwrap();
    let head = storage
        .empty_log_head("events.jsonl")
        .await
        .unwrap()
        .unwrap();
    for bytes in [vec![], vec![0; 7], vec![0; 9]] {
        storage.write(&head, &bytes).await.unwrap();
        assert_eq!(
            storage
                .empty_log_head("events.jsonl")
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
    }
    storage.write(&head, &0u64.to_le_bytes()).await.unwrap();
    *backend.fail_read.lock().unwrap() = Some(head);
    assert_eq!(
        storage
            .empty_log_head("events.jsonl")
            .await
            .unwrap_err()
            .kind(),
        io::ErrorKind::TimedOut
    );
}

#[tokio::test]
async fn stopping_a_recorder_drains_cancelled_log_appends_before_deletion() {
    use std::sync::atomic::Ordering;
    use std::time::Duration;
    let directory = tempfile::tempdir().unwrap();
    let backend = Arc::new(Memory::default());
    let store = RecordingStore::with_backend(backend.clone(), directory.path().join("unused"));
    let storage = store.recording("rtmp", "room", "live").unwrap();
    let log = storage.open_log("events.jsonl").await.unwrap();
    backend.pause_next_write.store(true, Ordering::Relaxed);
    let caller = tokio::spawn(async move { log.append(b"pending\n").await });
    backend.started.notified().await;
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    let mut draining = tokio::spawn(async move { store.flush_logs().await });
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut draining)
            .await
            .is_err()
    );
    backend.release.notify_one();
    tokio::time::timeout(Duration::from_secs(1), draining)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        storage.read_log("events.jsonl").await.unwrap(),
        b"pending\n"
    );
    storage.remove_all().await.unwrap();
    assert!(storage.list("", None, 10).await.unwrap().keys.is_empty());
}

#[tokio::test]
async fn object_logs_resume_with_legacy_base_without_any_local_directory() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("must-not-exist");
    let backend = Arc::new(Memory::default());
    let store = RecordingStore::with_backend(backend.clone(), &workspace).with_identity("memory");
    assert_eq!(store.clone().identity(), Some("memory"));
    let storage = store.scope("room/live").unwrap();
    storage.prepare().await.unwrap();
    for key in [
        "events.jsonl",
        "danmu.txt",
        "skipped_ad_ranges.jsonl",
        "nested/events.jsonl",
    ] {
        assert!(!storage.log_exists(key).await.unwrap());
        assert_eq!(
            storage.read_log(key).await.unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        storage.write(key, b"legacy\n").await.unwrap();
        let log = storage.open_log(key).await.unwrap();
        log.append(b"first\n").await.unwrap();
        // No flush and no writable reopen are required for durable reads.
        assert_eq!(storage.read_log(key).await.unwrap(), b"legacy\nfirst\n");
        drop(log);
        let resumed = store.scope("room/live").unwrap();
        resumed
            .open_log(key)
            .await
            .unwrap()
            .append(b"second\n")
            .await
            .unwrap();
        assert_eq!(
            resumed.read_log(key).await.unwrap(),
            b"legacy\nfirst\nsecond\n"
        );
        assert_eq!(resumed.read(key).await.unwrap(), b"legacy\n");
        assert!(resumed.log_exists(key).await.unwrap());
    }
    assert!(!workspace.exists());
    assert!(backend
        .objects
        .lock()
        .unwrap()
        .keys()
        .all(|key| key.starts_with("room/live/")));
    storage.remove_all().await.unwrap();
    assert!(backend.objects.lock().unwrap().is_empty());
}

#[tokio::test]
async fn chunks_are_bounded_and_failed_commit_is_retryable_without_rewriting_history() {
    let dir = tempfile::tempdir().unwrap();
    let backend = Arc::new(Memory::default());
    let storage = RecordingStorage::with_backend(
        backend.clone(),
        LocalWorkspace::new(dir.path().join("absent")),
    );
    let log = storage.open_log("events.jsonl").await.unwrap();
    let first = vec![b'a'; LOG_CHUNK_BYTES * 2 + 5];
    log.append(&first).await.unwrap();
    assert_eq!(storage.read_log("events.jsonl").await.unwrap(), first);
    let writes = backend.writes.lock().unwrap().clone();
    assert_eq!(
        writes.iter().map(|(_, size)| *size).collect::<Vec<_>>(),
        vec![8, LOG_CHUNK_BYTES, LOG_CHUNK_BYTES, 5, 8]
    );
    *backend.fail_write.lock().unwrap() = Some("/head".into());
    assert!(log.append(b"uncommitted").await.is_err());
    assert_eq!(storage.read_log("events.jsonl").await.unwrap(), first);
    *backend.fail_write.lock().unwrap() = None;
    drop(log);
    let resumed = storage.open_log("events.jsonl").await.unwrap();
    resumed.append(b"committed").await.unwrap();
    resumed.flush().await.unwrap();
    let mut expected = first;
    expected.extend_from_slice(b"committed");
    assert_eq!(storage.read_log("events.jsonl").await.unwrap(), expected);
    let writes = backend.writes.lock().unwrap();
    // The first three chunks were written exactly once.
    for (key, _) in &writes[1..4] {
        assert_eq!(
            writes.iter().filter(|(written, _)| written == key).count(),
            1
        );
    }
}

#[tokio::test]
async fn missing_or_invalid_committed_chunks_are_errors_not_empty_logs() {
    let dir = tempfile::tempdir().unwrap();
    let backend = Arc::new(Memory::default());
    let storage = RecordingStorage::with_backend(backend.clone(), LocalWorkspace::new(dir.path()));
    storage
        .open_log("events.jsonl")
        .await
        .unwrap()
        .append(b"one\n")
        .await
        .unwrap();
    let chunk = backend
        .objects
        .lock()
        .unwrap()
        .keys()
        .find(|key| key.ends_with("/00000000000000000000"))
        .unwrap()
        .clone();
    backend.remove(&chunk).await.unwrap();
    assert_eq!(
        storage.read_log("events.jsonl").await.unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    let head = backend
        .objects
        .lock()
        .unwrap()
        .keys()
        .find(|key| key.ends_with("/head"))
        .unwrap()
        .clone();
    backend.write(&head, b"broken").await.unwrap();
    assert_eq!(
        storage.read_log("events.jsonl").await.unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert!(storage.log_exists("events.jsonl").await.is_err());
    assert!(storage.open_log("events.jsonl").await.is_err());
}

async fn exercise_scopes(store: RecordingStore) {
    let one = store.scope("room1").unwrap();
    let ten = store.scope("room10").unwrap();
    for key in ["a", "nested/a", "nested/b", "nested10/a", "z"] {
        one.write(key, key.as_bytes()).await.unwrap();
        ten.write(key, b"keep").await.unwrap();
    }
    let mut keys = Vec::new();
    let mut cursor = None;
    loop {
        let page = one.list("", cursor.as_deref(), 2).await.unwrap();
        assert!(page.keys.len() <= 2);
        keys.extend(page.keys);
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(keys, ["a", "nested/a", "nested/b", "nested10/a", "z"]);
    assert_eq!(
        one.list("nested", None, 10).await.unwrap().keys,
        ["nested/a", "nested/b"]
    );
    assert!(one.list("../room10", None, 10).await.is_err());
    assert!(one.list("", None, 0).await.is_err());
    one.remove_all().await.unwrap();
    one.remove_all().await.unwrap();
    assert!(one.list("", None, 10).await.unwrap().keys.is_empty());
    assert_eq!(ten.list("", None, 10).await.unwrap().keys.len(), 5);
}

#[tokio::test]
async fn pagination_and_deletion_are_scoped_for_memory_and_two_local_roots() {
    let dir = tempfile::tempdir().unwrap();
    exercise_scopes(RecordingStore::with_backend(
        Arc::new(Memory::default()),
        dir.path().join("absent"),
    ))
    .await;
    exercise_scopes(RecordingStore::local(dir.path().join("local-one"))).await;
    exercise_scopes(RecordingStore::with_backend(
        Arc::new(LocalBackend::new(dir.path().join("local-two"))),
        dir.path().join("absent"),
    ))
    .await;
    assert!(!dir.path().join("absent").exists());
}

#[tokio::test]
async fn partial_deletion_failure_is_reported_and_retry_continues() {
    let dir = tempfile::tempdir().unwrap();
    let backend = Arc::new(Memory::default());
    let store = RecordingStore::with_backend(backend.clone(), dir.path());
    let storage = store.scope("room1").unwrap();
    for key in ["a", "b", "c"] {
        storage.write(key, b"x").await.unwrap();
    }
    store
        .scope("room10")
        .unwrap()
        .write("a", b"keep")
        .await
        .unwrap();
    *backend.fail_remove.lock().unwrap() = Some("room1/b".into());
    assert_eq!(
        storage.remove_all().await.unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    assert!(!storage.exists("a").await.unwrap());
    assert!(storage.exists("b").await.unwrap());
    assert!(storage.exists("c").await.unwrap());
    *backend.fail_remove.lock().unwrap() = None;
    storage.remove_all().await.unwrap();
    assert_eq!(backend.objects.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn materialization_copies_only_selected_assets_and_lease_cleans_up() {
    let dir = tempfile::tempdir().unwrap();
    let backend = Arc::new(Memory::default());
    let storage = RecordingStorage::with_backend(
        backend.clone(),
        LocalWorkspace::new(dir.path().join("absent")),
    );
    storage.write("nested/selected.ts", b"media").await.unwrap();
    storage.write("other.ts", b"not requested").await.unwrap();
    let assets = storage
        .materialize(&["nested/selected.ts".into()])
        .await
        .unwrap();
    let path = assets.path("nested/selected.ts").unwrap();
    assert!(path.exists());
    assert_eq!(assets.read("nested/selected.ts").await.unwrap(), b"media");
    assert!(!assets.path("other.ts").unwrap().exists());
    assets.write("output.mp4", b"rendered").await.unwrap();
    assert!(!storage.exists("output.mp4").await.unwrap());
    assert!(assets.path("../outside").is_err());
    let root = assets
        .path("output.mp4")
        .unwrap()
        .parent()
        .unwrap()
        .to_owned();
    let second = storage
        .materialize(&["nested/selected.ts".into()])
        .await
        .unwrap();
    assert_ne!(second.path("nested/selected.ts").unwrap(), path);
    drop(assets);
    assert!(!root.exists());
    assert!(second.path("nested/selected.ts").unwrap().exists());
    assert!(storage
        .materialize(&["nested/selected.ts".into(), "missing".into()])
        .await
        .is_err());
    assert!(!dir.path().join("absent").exists());
    assert_eq!(backend.objects.lock().unwrap().len(), 2);
    let scratch = LocalAssets::new().await.unwrap();
    scratch.write("scratch", b"temporary").await.unwrap();
    let scratch_path = scratch.path("scratch").unwrap();
    drop(scratch);
    assert!(!scratch_path.exists());
}

#[cfg(unix)]
#[tokio::test]
async fn local_listing_and_object_operations_do_not_follow_symlinks() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("keep"), b"outside").unwrap();
    let store = RecordingStore::local(dir.path());
    let storage = store.scope("room").unwrap();
    storage.prepare().await.unwrap();
    symlink(outside.path(), dir.path().join("room/escape")).unwrap();
    assert!(storage.list("", None, 10).await.is_err());
    assert!(storage.list("escape", None, 10).await.is_err());
    assert!(storage.read("escape/keep").await.is_err());
    assert!(storage.write("escape/keep", b"changed").await.is_err());
    assert!(storage.remove_all().await.is_err());
    assert_eq!(
        std::fs::read(outside.path().join("keep")).unwrap(),
        b"outside"
    );
    symlink(outside.path(), dir.path().join("linked-room")).unwrap();
    assert!(store
        .scope("linked-room")
        .unwrap()
        .list("", None, 10)
        .await
        .is_err());
}

#[tokio::test]
async fn local_backend_can_resume_object_format_logs_after_migration() {
    let dir = tempfile::tempdir().unwrap();
    let backend = Arc::new(Memory::default());
    let storage = RecordingStorage::with_backend(
        backend.clone(),
        LocalWorkspace::new(dir.path().join("absent")),
    );
    storage.write("events.jsonl", b"legacy\n").await.unwrap();
    storage
        .open_log("events.jsonl")
        .await
        .unwrap()
        .append(b"remote\n")
        .await
        .unwrap();
    let local = RecordingStorage::local(dir.path().join("archive"));
    let objects = backend.objects.lock().unwrap().clone();
    for (key, bytes) in objects {
        local.write(&key, &bytes).await.unwrap();
    }
    local
        .open_log("events.jsonl")
        .await
        .unwrap()
        .append(b"local\n")
        .await
        .unwrap();
    assert_eq!(
        local.read_log("events.jsonl").await.unwrap(),
        b"legacy\nremote\nlocal\n"
    );
    assert_eq!(local.read("events.jsonl").await.unwrap(), b"legacy\n");
}

#[tokio::test]
async fn local_logs_follow_selected_backend_not_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let one = RecordingStore::local(dir.path().join("one"))
        .scope("recording")
        .unwrap();
    let two = RecordingStore::with_backend(
        Arc::new(LocalBackend::new(dir.path().join("two"))),
        dir.path().join("unused-workspace"),
    )
    .scope("recording")
    .unwrap();
    for (storage, bytes) in [(&one, b"one\n"), (&two, b"two\n")] {
        storage.prepare().await.unwrap();
        storage
            .open_log("events.jsonl")
            .await
            .unwrap()
            .append(bytes)
            .await
            .unwrap();
        assert_eq!(storage.read_log("events.jsonl").await.unwrap(), bytes);
    }
    assert_eq!(
        std::fs::read(dir.path().join("one/recording/events.jsonl")).unwrap(),
        b"one\n"
    );
    assert_eq!(
        std::fs::read(dir.path().join("two/recording/events.jsonl")).unwrap(),
        b"two\n"
    );
    assert!(!dir.path().join("unused-workspace").exists());
}

#[tokio::test]
async fn default_media_repair_uses_only_object_operations() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("absent");
    let storage = RecordingStorage::with_backend(
        Arc::new(Memory::default()),
        LocalWorkspace::new(&workspace),
    );
    storage.write("0.ts", b"header").await.unwrap();
    storage.write("1.ts", b"fragment").await.unwrap();
    storage.append_media("0.ts", "1.ts").await.unwrap();
    assert_eq!(storage.read("0.ts").await.unwrap(), b"headerfragment");
    assert_eq!(storage.read("1.ts").await.unwrap(), b"fragment");
    assert!(!workspace.exists());
}

#[tokio::test]
async fn range_reads_are_scoped_bounded_and_handle_eof() {
    let dir = tempfile::tempdir().unwrap();
    for store in [
        RecordingStore::local(dir.path().join("local")),
        RecordingStore::with_backend(Arc::new(Memory::default()), dir.path().join("absent")),
    ] {
        let storage = store.scope("room/live").unwrap();
        storage.write("video.ts", b"0123456789").await.unwrap();
        assert_eq!(storage.read_range("video.ts", 3..6).await.unwrap(), b"345");
        assert_eq!(storage.read_range("video.ts", 8..100).await.unwrap(), b"89");
        assert!(storage
            .read_range("video.ts", 20..100)
            .await
            .unwrap()
            .is_empty());
        assert!(storage
            .read_range("video.ts", 5..5)
            .await
            .unwrap()
            .is_empty());
        let start = 6;
        assert_eq!(
            storage
                .read_range("video.ts", start..3)
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            storage
                .read_range("missing", 0..1)
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
        assert!(storage.read_range("../outside", 0..1).await.is_err());
    }
    let path = dir.path().join("local/room/live/sparse");
    std::fs::File::create(&path)
        .unwrap()
        .set_len(128 * 1024 * 1024)
        .unwrap();
    let storage = RecordingStore::local(dir.path().join("local"))
        .scope("room/live")
        .unwrap();
    assert_eq!(
        storage
            .read_range("sparse", 120_000_000..120_000_004)
            .await
            .unwrap(),
        [0; 4]
    );
    assert!(!dir.path().join("absent").exists());
}

#[test]
fn cancelled_materialization_write_keeps_lease_until_io_finishes_then_cleans_up() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(async {
        let assets = LocalAssets::new().await.unwrap();
        let path = assets.path("nested/file").unwrap();
        let root = assets
            .path("root-file")
            .unwrap()
            .parent()
            .unwrap()
            .to_owned();
        let (release, wait) = std::sync::mpsc::channel();
        let blocker = tokio::task::spawn_blocking(move || wait.recv().unwrap());
        let write = tokio::spawn(async move { assets.write("nested/file", b"temporary").await });
        // The write queues its blocking task behind the blocker.
        tokio::task::yield_now().await;
        write.abort();
        assert!(write.await.unwrap_err().is_cancelled());
        assert!(root.exists());
        release.send(()).unwrap();
        blocker.await.unwrap();
        tokio::task::spawn_blocking(|| ()).await.unwrap();
        assert!(!root.exists());
        assert!(!path.exists());
    });
}
