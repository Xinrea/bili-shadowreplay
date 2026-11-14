use std::{io, sync::Arc};

use recording_catalog::{
    object_store::{memory::InMemory, ObjectStore, ObjectStoreExt},
    CatalogLocation, CatalogRecord,
};
use uuid::Uuid;

fn record(live: &str, time: i64) -> CatalogRecord {
    CatalogRecord {
        recording_id: Uuid::new_v4(),
        platform: "bilibili".into(),
        room_id: "room".into(),
        live_id: live.into(),
        parent_id: "session".into(),
        title: "测试标题".into(),
        created_at_ms: time,
        length: 12.5,
        size: 1024,
        cover: Some("cover.jpg".into()),
        results: serde_json::json!({"summary": "归档摘要", "version": 1}),
    }
}

#[tokio::test]
async fn concurrent_initialization_keeps_one_identity_and_one_writer() {
    let store = Arc::new(InMemory::new());
    let first = CatalogLocation::object_store(store.clone());
    let second = CatalogLocation::object_store(store);
    let (a, b) = tokio::join!(first.open_writer(true), second.open_writer(true));
    let (writer, error) = match (a, b) {
        (Ok(writer), Err(error)) | (Err(error), Ok(writer)) => (writer, error),
        _ => panic!("exactly one initializer must own the writer"),
    };
    assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
    assert_eq!(&first.identity().await.unwrap(), writer.identity());
    assert_eq!(
        first.identity().await.unwrap(),
        second.identity().await.unwrap()
    );
    writer.close().await.unwrap();
    second
        .open_writer(false)
        .await
        .unwrap()
        .close()
        .await
        .unwrap();
}

async fn contract(location: CatalogLocation) {
    assert_eq!(
        location.open_reader().await.err().unwrap().kind(),
        io::ErrorKind::NotFound
    );
    let writer = location.open_writer(true).await.unwrap();
    let identity = writer.identity().clone();
    assert_eq!(
        location.open_writer(false).await.err().unwrap().kind(),
        io::ErrorKind::WouldBlock
    );
    let first = writer.put(record("first", -1), None).await.unwrap();
    let second = writer.put(record("second", 10), None).await.unwrap();
    let third = writer.put(record("third", 20), None).await.unwrap();
    let page = writer.list(None, 2).await.unwrap();
    assert_eq!(page.entries, [third.clone(), second.clone()]);
    let page = writer.list(page.next_cursor.as_deref(), 2).await.unwrap();
    assert_eq!(page.entries.as_slice(), std::slice::from_ref(&first));
    assert!(page.next_cursor.is_none());
    assert!(writer.list(None, 0).await.is_err());
    assert!(writer.list(Some("record/not-a-cursor"), 2).await.is_err());

    let reader = location.open_reader().await.unwrap();
    assert_eq!(reader.identity(), &identity);
    assert_eq!(
        reader.get(first.record.recording_id).await.unwrap(),
        Some(first.clone())
    );
    // Opening a reader did not fence or acquire ownership from the live writer.
    let mut changed = first.record.clone();
    changed.title = "修改后的标题".into();
    changed.created_at_ms = 30;
    let updated = writer
        .put(changed.clone(), Some(first.revision))
        .await
        .unwrap();
    assert!(updated.revision > third.revision);
    assert_eq!(
        writer
            .put(changed, Some(first.revision))
            .await
            .unwrap_err()
            .kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(
        writer.list(None, 10).await.unwrap().entries,
        [updated.clone(), third.clone(), second.clone()]
    );
    let mut conflict = record("first", 100);
    assert_eq!(
        writer.put(conflict.clone(), None).await.unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    conflict.recording_id = first.record.recording_id;
    conflict.live_id = "changed-identity".into();
    assert!(writer.put(conflict, Some(updated.revision)).await.is_err());
    reader.close().await.unwrap();
    writer.close().await.unwrap();
    assert!(writer.get(first.record.recording_id).await.is_err());

    // Remount only from the backend; no application/global SQLite state exists.
    let reader = location.open_reader().await.unwrap();
    assert_eq!(reader.identity(), &identity);
    assert_eq!(
        reader.list(None, 10).await.unwrap().entries,
        [updated.clone(), third, second.clone()]
    );
    reader.close().await.unwrap();
    let writer = location.open_writer(false).await.unwrap();
    assert_eq!(writer.identity(), &identity);
    assert!(writer
        .delete(first.record.recording_id, first.revision)
        .await
        .is_err());
    writer
        .delete(first.record.recording_id, updated.revision)
        .await
        .unwrap();
    writer.close().await.unwrap();
    let reader = location.open_reader().await.unwrap();
    assert!(reader
        .get(first.record.recording_id)
        .await
        .unwrap()
        .is_none());
    assert_eq!(reader.list(None, 10).await.unwrap().entries.len(), 2);
    reader.close().await.unwrap();
    let writer = location.open_writer(false).await.unwrap();
    let restored = writer.put(first.record.clone(), None).await.unwrap();
    assert!(restored.revision > updated.revision);
    assert!(writer
        .put(first.record, Some(updated.revision))
        .await
        .is_err());
    writer.close().await.unwrap();
}

#[tokio::test]
async fn local_catalog_conformance() {
    let root = tempfile::tempdir().unwrap();
    let location = CatalogLocation::local(root.path()).unwrap();
    contract(location).await;
    assert!(root.path().join(".bsr/storage.json").is_file());
}

#[tokio::test]
async fn object_catalog_conformance() {
    contract(CatalogLocation::object_store(Arc::new(InMemory::new()))).await;
}

#[tokio::test]
async fn metadata_copy_keeps_uuid_and_can_resume_before_source_retirement() {
    let root = tempfile::tempdir().unwrap();
    let a = CatalogLocation::local(root.path()).unwrap();
    let b = CatalogLocation::object_store(Arc::new(InMemory::new()));
    let source = a.open_writer(true).await.unwrap();
    let target = b.open_writer(true).await.unwrap();
    assert_ne!(
        source.identity().storage_uuid,
        target.identity().storage_uuid
    );
    let original = source.put(record("move", 20), None).await.unwrap();
    let copied = target.put(original.record.clone(), None).await.unwrap();
    assert_eq!(copied.record, original.record);
    assert!(
        target.put(original.record.clone(), None).await.is_err(),
        "copy must not overwrite a pre-existing record"
    );
    source.close().await.unwrap();
    target.close().await.unwrap();

    // Crash boundary: target is durable but source retirement has not happened.
    // The future application coordinator must retain its transfer intent here.
    let source = a.open_writer(false).await.unwrap();
    let target = b.open_reader().await.unwrap();
    assert_eq!(
        source.get(original.record.recording_id).await.unwrap(),
        Some(original.clone())
    );
    assert_eq!(
        target
            .get(original.record.recording_id)
            .await
            .unwrap()
            .unwrap()
            .record,
        original.record
    );
    source
        .delete(original.record.recording_id, original.revision)
        .await
        .unwrap();
    source.close().await.unwrap();
    target.close().await.unwrap();
    assert!(a
        .open_reader()
        .await
        .unwrap()
        .list(None, 10)
        .await
        .unwrap()
        .entries
        .is_empty());
    assert_eq!(
        b.open_reader()
            .await
            .unwrap()
            .list(None, 10)
            .await
            .unwrap()
            .entries
            .len(),
        1
    );
}

#[tokio::test]
async fn catalog_snapshot_paginates_after_many_json_updates() {
    let root = tempfile::tempdir().unwrap();
    let location = CatalogLocation::local(root.path()).unwrap();
    let writer = location.open_writer(true).await.unwrap();
    for time in 0..160 {
        writer
            .put(record(&format!("live-{time}"), time), None)
            .await
            .unwrap();
    }
    writer.close().await.unwrap();
    let started = std::time::Instant::now();
    let reader = location.open_reader().await.unwrap();
    let mut cursor = None;
    let mut times = Vec::new();
    loop {
        let page = reader.list(cursor.as_deref(), 17).await.unwrap();
        times.extend(
            page.entries
                .into_iter()
                .map(|entry| entry.record.created_at_ms),
        );
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(times, (0..160).rev().collect::<Vec<_>>());
    eprintln!("Local cold open + 160-record scan: {:?}", started.elapsed());
    reader.close().await.unwrap();
}

#[tokio::test]
async fn unsupported_formats_are_rejected_without_modifying_storage() {
    use futures::TryStreamExt;
    for format_version in [0, 1, 3, u32::MAX] {
        let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
        let location = CatalogLocation::object_store(store.clone());
        let identity = serde_json::to_vec(&recording_catalog::StorageIdentity {
            format_version,
            storage_uuid: Uuid::new_v4(),
        })
        .unwrap();
        store
            .put(&".bsr/storage.json".into(), identity.clone().into())
            .await
            .unwrap();
        assert_eq!(
            location.open_writer(false).await.err().unwrap().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            location.open_writer(true).await.err().unwrap().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            location.open_reader().await.err().unwrap().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            store
                .get(&".bsr/storage.json".into())
                .await
                .unwrap()
                .bytes()
                .await
                .unwrap()
                .as_ref(),
            identity
        );
        assert_eq!(
            store
                .list(None)
                .try_collect::<Vec<_>>()
                .await
                .unwrap()
                .len(),
            1
        );
    }
}

#[tokio::test]
async fn reads_never_initialize_missing_storage() {
    use futures::TryStreamExt;
    let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
    let location = CatalogLocation::object_store(store.clone());
    assert!(location.open_reader().await.is_err());
    assert!(store
        .list(None)
        .try_collect::<Vec<_>>()
        .await
        .unwrap()
        .is_empty());
}

#[test]
fn dropping_writer_outside_runtime_releases_local_lock() {
    let root = tempfile::tempdir().unwrap();
    let location = CatalogLocation::local(root.path()).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let writer = runtime.block_on(location.open_writer(true)).unwrap();
    drop(writer); // no current Tokio runtime in this calling thread
    runtime.block_on(async {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                match location.open_writer(false).await {
                    Ok(writer) => {
                        writer.close().await.unwrap();
                        break;
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        tokio::task::yield_now().await
                    }
                    Err(error) => panic!("unexpected reopen error: {error}"),
                }
            }
        })
        .await
        .unwrap();
    });
}

#[test]
#[ignore = "child process of local_catalog_survives_process_exit"]
fn crash_writer_child() {
    let root = std::env::var("BSR_CATALOG_CHILD_ROOT").unwrap();
    let id: Uuid = std::env::var("BSR_CATALOG_CHILD_ID")
        .unwrap()
        .parse()
        .unwrap();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let writer = CatalogLocation::local(root)
            .unwrap()
            .open_writer(true)
            .await
            .unwrap();
        let mut entry = record("crash-test", 0);
        entry.recording_id = id;
        writer.put(entry, None).await.unwrap();
        // No close or destructor: acknowledged JSON must already be durable.
        std::process::exit(0);
    });
}

#[tokio::test]
async fn local_catalog_survives_process_exit() {
    let root = tempfile::tempdir().unwrap();
    let id = Uuid::new_v4();
    let status = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "crash_writer_child"])
        .env("BSR_CATALOG_CHILD_ROOT", root.path())
        .env("BSR_CATALOG_CHILD_ID", id.to_string())
        .status()
        .await
        .unwrap();
    assert!(status.success());
    let location = CatalogLocation::local(root.path()).unwrap();
    let reader = location.open_reader().await.unwrap();
    assert_eq!(
        reader.get(id).await.unwrap().unwrap().record.live_id,
        "crash-test"
    );
    reader.close().await.unwrap();
    location
        .open_writer(false)
        .await
        .unwrap()
        .close()
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires RustFS and explicit S3_TEST_* test credentials"]
async fn rustfs_catalog_conformance() {
    use futures::TryStreamExt;
    use recording_catalog::object_store::{aws::AmazonS3Builder, prefix::PrefixStore};
    let endpoint = std::env::var("S3_TEST_ENDPOINT").expect("set S3_TEST_ENDPOINT");
    let bucket = std::env::var("S3_TEST_BUCKET").expect("set S3_TEST_BUCKET");
    let prefix = format!("catalog-test-{}", Uuid::new_v4());
    let store = AmazonS3Builder::new()
        .with_endpoint(&endpoint)
        .with_allow_http(endpoint.starts_with("http://"))
        .with_bucket_name(bucket)
        .with_region("us-east-1")
        .with_access_key_id(
            std::env::var("S3_TEST_ACCESS_KEY_ID").expect("set S3_TEST_ACCESS_KEY_ID"),
        )
        .with_secret_access_key(
            std::env::var("S3_TEST_SECRET_ACCESS_KEY").expect("set S3_TEST_SECRET_ACCESS_KEY"),
        )
        .build()
        .unwrap();
    let scoped: Arc<dyn ObjectStore> = Arc::new(PrefixStore::new(store, prefix.clone()));
    eprintln!("RustFS catalog conformance prefix: {prefix}");
    let location = CatalogLocation::object_store(scoped.clone());
    let started = std::time::Instant::now();
    contract(location.clone()).await;
    eprintln!("RustFS catalog conformance: {:?}", started.elapsed());
    // The same metadata copy/retire primitives work between actual S3 and a
    // filesystem, without moving the storage's identity or unrelated records.
    let local_root = tempfile::tempdir().unwrap();
    let local = CatalogLocation::local(local_root.path()).unwrap();
    let remote = location.open_writer(false).await.unwrap();
    let local_writer = local.open_writer(true).await.unwrap();
    let entries = remote.list(None, 10).await.unwrap().entries;
    for entry in &entries {
        let copied = local_writer.put(entry.record.clone(), None).await.unwrap();
        assert_eq!(copied.record, entry.record);
        remote
            .delete(entry.record.recording_id, entry.revision)
            .await
            .unwrap();
    }
    assert!(remote.list(None, 10).await.unwrap().entries.is_empty());
    for entry in local_writer.list(None, 10).await.unwrap().entries {
        remote.put(entry.record.clone(), None).await.unwrap();
        local_writer
            .delete(entry.record.recording_id, entry.revision)
            .await
            .unwrap();
    }
    assert_eq!(
        remote.list(None, 10).await.unwrap().entries.len(),
        entries.len()
    );
    assert!(local_writer
        .list(None, 10)
        .await
        .unwrap()
        .entries
        .is_empty());
    remote.close().await.unwrap();
    local_writer.close().await.unwrap();
    let objects = scoped.list(None).try_collect::<Vec<_>>().await.unwrap();
    for object in objects {
        scoped.delete(&object.location).await.unwrap();
    }
}
