use std::io;
use std::sync::Arc;

use async_trait::async_trait;
use recording_storage::{
    validate_key, AppendLog, LocalWorkspace, ObjectMetadata, ObjectPage, RecordingStorage,
    RecordingStore, StorageBackend,
};

#[test]
fn rejects_unsafe_and_nonportable_names() {
    for key in [
        "",
        ".",
        "..",
        "../x",
        "a/../b",
        "/absolute",
        "a//b",
        "a/",
        "C:/x",
        "C:x",
        "\\\\server\\x",
        "a\\b",
        "a\0b",
        "CON",
        "aux.txt",
        "a.",
        "a ",
    ] {
        assert!(validate_key(key).is_err(), "accepted {key:?}");
    }
    for key in [
        ".sequence",
        "playlist.m3u8",
        "room/live/1.ts",
        "%d.ts",
        "直播/弹幕.jsonl",
    ] {
        validate_key(key).unwrap();
    }
}

#[tokio::test]
async fn scopes_preserve_layout_and_isolate_recordings() {
    let dir = tempfile::tempdir().unwrap();
    let store = RecordingStore::local(dir.path());
    let one = store.recording("bilibili", "1", "live").unwrap();
    let two = store.recording("bilibili", "2", "live").unwrap();
    one.write(".sequence", b"123").await.unwrap();
    two.write(".sequence", b"456").await.unwrap();
    assert_eq!(one.read(".sequence").await.unwrap(), b"123");
    assert_eq!(two.read(".sequence").await.unwrap(), b"456");
    assert_eq!(one.workspace().root(), dir.path().join("bilibili/1/live"));
    assert!(store.recording("bilibili", "1/nested", "live").is_err());
    assert!(store.scope("../outside").is_err());
    assert!(one.write("../outside", b"x").await.is_err());
    assert!(one.workspace().path("/outside").is_err());
}

#[tokio::test]
async fn injected_store_scopes_backend_keys_and_keeps_workspaces_separate() {
    let dir = tempfile::tempdir().unwrap();
    let backend = Arc::new(recording_storage::LocalBackend::new(
        dir.path().join("objects"),
    ));
    let store = RecordingStore::with_backend(backend.clone(), dir.path().join("workspaces"));
    let one = store.recording("bilibili", "1", "live").unwrap();
    let two = store.recording("bilibili", "2", "live").unwrap();
    one.prepare().await.unwrap();
    one.workspace().stage_media("0.ts", b"media").await.unwrap();
    assert!(!one.exists("0.ts").await.unwrap());
    one.publish_media("0.ts").await.unwrap();
    one.write(".sequence", b"42").await.unwrap();
    assert_eq!(
        backend.read("bilibili/1/live/0.ts").await.unwrap(),
        b"media"
    );
    assert_eq!(one.read(".sequence").await.unwrap(), b"42");
    assert!(!two.exists("0.ts").await.unwrap());
    assert!(!one.workspace().path(".sequence").unwrap().exists());
    one.remove("0.ts").await.unwrap();
    assert!(!one.exists("0.ts").await.unwrap());
    // Removing an object does not silently evict a native worker's local file.
    assert_eq!(one.workspace().read("0.ts").await.unwrap(), b"media");
}

#[tokio::test]
async fn replacement_remove_and_missing_semantics() {
    let dir = tempfile::tempdir().unwrap();
    let storage = RecordingStorage::local(dir.path());
    assert!(!storage.exists("missing").await.unwrap());
    storage.remove("missing").await.unwrap();
    storage.write("nested/checkpoint", b"123456").await.unwrap();
    storage.write("nested/checkpoint", b"7").await.unwrap();
    assert_eq!(storage.read("nested/checkpoint").await.unwrap(), b"7");
    assert_eq!(storage.size("nested/checkpoint").await.unwrap(), 1);
    storage.remove("nested/checkpoint").await.unwrap();
    assert!(!storage.exists("nested/checkpoint").await.unwrap());
    assert_eq!(
        storage.read("missing").await.unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
}

#[tokio::test]
async fn bounded_reads_reject_oversize_objects_and_keep_scope() {
    let directory = tempfile::tempdir().unwrap();
    let store = RecordingStore::with_backend(
        Arc::new(recording_storage::LocalBackend::new(
            directory.path().join("objects"),
        )),
        directory.path().join("workspace"),
    );
    let storage = store.recording("rtmp", "room", "live").unwrap();
    storage.write("playlist.m3u8", b"12345678").await.unwrap();
    assert_eq!(
        storage.read_bounded("playlist.m3u8", 8).await.unwrap(),
        b"12345678"
    );
    assert_eq!(
        storage
            .read_bounded("playlist.m3u8", 7)
            .await
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(
        storage.read_bounded("missing", 8).await.unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
    storage.write("empty", b"").await.unwrap();
    assert!(storage.read_bounded("empty", 0).await.unwrap().is_empty());

    // Sparse input must be stopped after limit + 1 bytes, not read in full.
    let oversized = directory.path().join("objects/rtmp/room/live/oversized");
    std::fs::File::create(&oversized)
        .unwrap()
        .set_len(64 * 1024 * 1024)
        .unwrap();
    assert_eq!(
        storage
            .read_bounded("oversized", 16)
            .await
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
    assert!(!directory.path().join("workspace").exists());
}

#[tokio::test]
async fn readers_only_see_complete_replacements() {
    let dir = tempfile::tempdir().unwrap();
    let storage = RecordingStorage::local(dir.path());
    let before = vec![b'a'; 8192];
    let after = vec![b'b'; 16384];
    storage.write("playlist", &before).await.unwrap();
    let reader = storage.clone();
    let reads = tokio::spawn(async move {
        for _ in 0..100 {
            let bytes = reader.read("playlist").await.unwrap();
            assert!(bytes == vec![b'a'; 8192] || bytes == vec![b'b'; 16384]);
        }
    });
    for _ in 0..10 {
        storage.write("playlist", &after).await.unwrap();
        storage.write("playlist", &before).await.unwrap();
    }
    reads.await.unwrap();
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[tokio::test]
async fn failed_publication_cleans_up_temporary_object() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("blocked")).unwrap();
    std::fs::write(dir.path().join("blocked/keep"), b"original").unwrap();
    let storage = RecordingStorage::local(dir.path());
    assert!(storage.write("blocked", b"replacement").await.is_err());
    assert_eq!(
        std::fs::read(dir.path().join("blocked/keep")).unwrap(),
        b"original"
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[tokio::test]
async fn logs_append_across_reopen_without_truncation() {
    let dir = tempfile::tempdir().unwrap();
    let storage = RecordingStorage::local(dir.path());
    let log = storage.open_log("events.jsonl").await.unwrap();
    log.append(b"one\n").await.unwrap();
    log.clone().append(b"two\n").await.unwrap();
    log.flush().await.unwrap();
    drop(log);
    let log = AppendLog::open(dir.path().join("events.jsonl"))
        .await
        .unwrap();
    log.append(b"three\n").await.unwrap();
    log.flush().await.unwrap();
    assert_eq!(
        storage.read("events.jsonl").await.unwrap(),
        b"one\ntwo\nthree\n"
    );
}

#[tokio::test]
async fn logs_require_prepared_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let storage = RecordingStorage::local(dir.path().join("recording"));
    assert_eq!(
        storage.open_log("events.jsonl").await.err().unwrap().kind(),
        io::ErrorKind::NotFound
    );
    assert!(!storage.workspace().root().exists());
    storage.prepare().await.unwrap();
    storage.open_log("events.jsonl").await.unwrap();
}

#[tokio::test]
async fn workspace_repairs_by_atomic_replacement_and_retains_source_until_acknowledged() {
    let dir = tempfile::tempdir().unwrap();
    let storage = RecordingStorage::local(dir.path());
    let workspace = storage.workspace();
    workspace
        .stage_media("0.ts", b"header+media")
        .await
        .unwrap();
    workspace.stage_media("1.ts", b"+fragment").await.unwrap();
    workspace.append_media("0.ts", "1.ts").await.unwrap();
    assert_eq!(
        workspace.read("0.ts").await.unwrap(),
        b"header+media+fragment"
    );
    assert_eq!(workspace.read("1.ts").await.unwrap(), b"+fragment");
    storage.publish_media("0.ts").await.unwrap();
    workspace.remove("1.ts").await.unwrap();
    assert!(workspace.append_media("0.ts", "missing.ts").await.is_err());
    assert_eq!(
        workspace.read("0.ts").await.unwrap(),
        b"header+media+fragment"
    );
    assert!(workspace.append_media("0.ts", "0.ts").await.is_err());
}

#[tokio::test]
async fn media_repair_uses_objects_without_requiring_workspace_replicas() {
    for colocated in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let objects = dir.path().join("objects");
        let workspace = dir.path().join("workspace");
        let storage = if colocated {
            RecordingStorage::local(&objects)
        } else {
            RecordingStorage::with_backend(
                Arc::new(recording_storage::LocalBackend::new(&objects)),
                LocalWorkspace::new(&workspace),
            )
        };
        storage.write("0.ts", b"header+media").await.unwrap();
        storage.write("1.ts", b"+fragment").await.unwrap();
        storage.append_media("0.ts", "1.ts").await.unwrap();
        assert_eq!(
            storage.read("0.ts").await.unwrap(),
            b"header+media+fragment"
        );
        assert_eq!(storage.read("1.ts").await.unwrap(), b"+fragment");
        assert!(!workspace.exists());

        assert!(storage.append_media("0.ts", "missing.ts").await.is_err());
        assert!(storage.append_media("missing.ts", "1.ts").await.is_err());
        for (target, fragment) in [
            ("0.ts", "0.ts"),
            ("../outside", "1.ts"),
            ("0.ts", "../outside"),
        ] {
            assert_eq!(
                storage
                    .append_media(target, fragment)
                    .await
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::InvalidInput
            );
        }
        assert_eq!(
            storage.read("0.ts").await.unwrap(),
            b"header+media+fragment"
        );
        assert_eq!(storage.read("1.ts").await.unwrap(), b"+fragment");
        storage.remove("1.ts").await.unwrap();
        assert!(!storage.exists("1.ts").await.unwrap());
        assert!(!workspace.exists());
    }
}

struct ReadOnly(recording_storage::LocalBackend);

#[async_trait]
impl StorageBackend for ReadOnly {
    async fn read_range(&self, key: &str, range: std::ops::Range<u64>) -> io::Result<Vec<u8>> {
        self.0.read_range(key, range).await
    }
    async fn list(
        &self,
        prefix: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> io::Result<ObjectPage> {
        self.0.list(prefix, cursor, limit).await
    }
    async fn read(&self, key: &str) -> io::Result<Vec<u8>> {
        self.0.read(key).await
    }
    async fn read_bounded(&self, key: &str, max_bytes: usize) -> io::Result<Vec<u8>> {
        self.0.read_bounded(key, max_bytes).await
    }
    async fn write(&self, _: &str, _: &[u8]) -> io::Result<()> {
        Err(io::ErrorKind::PermissionDenied.into())
    }
    async fn remove(&self, _: &str) -> io::Result<()> {
        Err(io::ErrorKind::PermissionDenied.into())
    }
    async fn stat(&self, key: &str) -> io::Result<ObjectMetadata> {
        self.0.stat(key).await
    }
}

#[tokio::test]
async fn failed_media_repair_preserves_target_and_fragment() {
    let dir = tempfile::tempdir().unwrap();
    let backend = recording_storage::LocalBackend::new(dir.path().join("objects"));
    backend.write("0.ts", b"original").await.unwrap();
    backend.write("1.ts", b"fragment").await.unwrap();
    let workspace = dir.path().join("workspace");
    let storage = RecordingStorage::with_backend(
        Arc::new(ReadOnly(backend)),
        LocalWorkspace::new(&workspace),
    );
    assert_eq!(
        storage
            .append_media("0.ts", "1.ts")
            .await
            .unwrap_err()
            .kind(),
        io::ErrorKind::PermissionDenied
    );
    assert_eq!(storage.read("0.ts").await.unwrap(), b"original");
    assert_eq!(storage.read("1.ts").await.unwrap(), b"fragment");
    assert!(!workspace.exists());
}

struct Denied;

#[async_trait]
impl StorageBackend for Denied {
    async fn read_range(&self, _: &str, _: std::ops::Range<u64>) -> io::Result<Vec<u8>> {
        Err(io::ErrorKind::PermissionDenied.into())
    }
    async fn list(&self, _: &str, _: Option<&str>, _: usize) -> io::Result<ObjectPage> {
        Err(io::ErrorKind::PermissionDenied.into())
    }
    async fn read(&self, _: &str) -> io::Result<Vec<u8>> {
        Err(io::ErrorKind::PermissionDenied.into())
    }
    async fn read_bounded(&self, _: &str, _: usize) -> io::Result<Vec<u8>> {
        Err(io::ErrorKind::PermissionDenied.into())
    }
    async fn write(&self, _: &str, _: &[u8]) -> io::Result<()> {
        Err(io::ErrorKind::PermissionDenied.into())
    }
    async fn remove(&self, _: &str) -> io::Result<()> {
        Err(io::ErrorKind::PermissionDenied.into())
    }
    async fn stat(&self, _: &str) -> io::Result<ObjectMetadata> {
        Err(io::ErrorKind::PermissionDenied.into())
    }
}

#[tokio::test]
async fn substituted_backend_propagates_failures_without_calling_them_missing() {
    let dir = tempfile::tempdir().unwrap();
    let storage = RecordingStorage::with_backend(Arc::new(Denied), LocalWorkspace::new(dir.path()));
    storage.prepare().await.unwrap();
    assert_eq!(
        storage.exists("x").await.unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    assert_eq!(
        storage.read("x").await.unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    assert_eq!(
        storage.read_log("x").await.unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    assert_eq!(
        storage.log_exists("x").await.unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    assert_eq!(
        storage.open_log("x").await.err().unwrap().kind(),
        io::ErrorKind::PermissionDenied
    );
    assert_eq!(
        storage.write("x", b"x").await.unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    assert_eq!(
        storage.remove("x").await.unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    storage
        .workspace()
        .stage_media("0.ts", b"complete")
        .await
        .unwrap();
    assert_eq!(
        storage.publish_media("0.ts").await.unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    assert_eq!(storage.workspace().read("0.ts").await.unwrap(), b"complete");
}
