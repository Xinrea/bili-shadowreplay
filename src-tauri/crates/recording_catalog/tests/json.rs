use async_trait::async_trait;
use futures::{stream::BoxStream, TryStreamExt};
use recording_catalog::{
    object_store::{
        self, memory::InMemory, path::Path, CopyOptions, GetOptions, GetResult, ListResult,
        MultipartUpload, ObjectMeta, ObjectStore, ObjectStoreExt, PutMode, PutMultipartOptions,
        PutOptions, PutPayload, PutResult,
    },
    CatalogLocation, CatalogRecord, ImportDigest, ImportPlan,
};
use std::{
    fmt, io,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

#[derive(Default, Debug)]
struct JsonOnlyStore {
    inner: InMemory,
    // Fail one selected write, either before or after it reaches storage.
    failure: Mutex<Option<(String, bool)>>,
}
impl fmt::Display for JsonOnlyStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("json-only")
    }
}
#[async_trait]
impl ObjectStore for JsonOnlyStore {
    async fn put_opts(
        &self,
        path: &Path,
        data: PutPayload,
        opts: PutOptions,
    ) -> object_store::Result<PutResult> {
        assert!(
            matches!(opts.mode, PutMode::Overwrite),
            "JSON catalogs must not use conditional PUT"
        );
        let fault = {
            let mut failure = self.failure.lock().unwrap();
            if failure
                .as_ref()
                .is_some_and(|(key, _)| key == path.as_ref())
            {
                failure.take()
            } else {
                None
            }
        };
        let error = || object_store::Error::Generic {
            store: "fault",
            source: "injected lost request/ack".into(),
        };
        if matches!(fault, Some((_, false))) {
            return Err(error());
        }
        let result = self.inner.put_opts(path, data, opts).await?;
        if fault.is_some() {
            return Err(error());
        }
        Ok(result)
    }
    async fn put_multipart_opts(
        &self,
        _: &Path,
        _: PutMultipartOptions,
    ) -> object_store::Result<Box<dyn MultipartUpload>> {
        panic!("catalog JSON must not require multipart uploads")
    }
    async fn get_opts(&self, path: &Path, opts: GetOptions) -> object_store::Result<GetResult> {
        assert!(opts.if_match.is_none() && opts.if_none_match.is_none());
        self.inner.get_opts(path, opts).await
    }
    fn delete_stream(
        &self,
        paths: BoxStream<'static, object_store::Result<Path>>,
    ) -> BoxStream<'static, object_store::Result<Path>> {
        self.inner.delete_stream(paths)
    }
    fn list(&self, prefix: Option<&Path>) -> BoxStream<'static, object_store::Result<ObjectMeta>> {
        self.inner.list(prefix)
    }
    async fn list_with_delimiter(&self, prefix: Option<&Path>) -> object_store::Result<ListResult> {
        self.inner.list_with_delimiter(prefix).await
    }
    async fn copy_opts(
        &self,
        from: &Path,
        to: &Path,
        opts: CopyOptions,
    ) -> object_store::Result<()> {
        self.inner.copy_opts(from, to, opts).await
    }
}
fn record() -> CatalogRecord {
    CatalogRecord {
        recording_id: Uuid::new_v4(),
        platform: "bilibili".into(),
        room_id: "123".into(),
        live_id: Uuid::new_v4().to_string(),
        parent_id: "session".into(),
        title: "Title".into(),
        created_at_ms: 1,
        length: 2.0,
        size: 3,
        cover: None,
        results: serde_json::Value::Null,
    }
}

#[tokio::test]
async fn ordinary_puts_keep_one_current_document_and_recover_lost_ack() {
    let store = Arc::new(JsonOnlyStore::default());
    let location = CatalogLocation::object_store(store.clone());
    let writer = location.open_writer(true).await.unwrap();
    let record = record();
    let path = format!(".bsr/records/{}/recording.json", record.recording_id);
    *store.failure.lock().unwrap() = Some((path.clone(), true));
    let mut entry = writer.put(record.clone(), None).await.unwrap();
    for i in 0..10 {
        entry.record.title = format!("Title {i}");
        entry = writer
            .put(entry.record, Some(entry.revision))
            .await
            .unwrap();
    }
    let keys: Vec<_> = store
        .list(None)
        .map_ok(|m| m.location.to_string())
        .try_collect()
        .await
        .unwrap();
    assert_eq!(
        keys.len(),
        2,
        "identity + one current JSON; no lease, WAL or version history"
    );
    assert!(keys.contains(&path));
    writer
        .delete_recording(entry.record.clone(), entry.revision)
        .await
        .unwrap();
    writer.close().await.unwrap();
    let writer = location.open_writer(false).await.unwrap();
    assert!(writer.get(record.recording_id).await.unwrap().is_none());
    assert_eq!(
        writer.pending_deletions().await.unwrap(),
        vec![entry.record]
    );
    writer.finish_deletion(record.recording_id).await.unwrap();
    assert!(writer.pending_deletions().await.unwrap().is_empty());
    writer.close().await.unwrap();
}

#[tokio::test]
async fn failed_import_progress_replays_record_without_clobbering() {
    let store = Arc::new(JsonOnlyStore::default());
    let location = CatalogLocation::object_store(store.clone());
    let writer = location.open_writer(true).await.unwrap();
    let record = record();
    let mut digest = ImportDigest::default();
    digest.append(&record).unwrap();
    let plan = ImportPlan {
        import_id: Uuid::new_v4(),
        storage_uuid: writer.identity().storage_uuid,
        count: 1,
        digest,
    };
    writer.begin_import(plan.clone()).await.unwrap();
    *store.failure.lock().unwrap() = Some((".bsr/import.json".into(), false));
    assert!(writer
        .import_record(plan.clone(), 0, record.clone())
        .await
        .is_err());
    assert!(writer.scan().await.is_err(), "unknown outcome fails closed");
    writer.close().await.unwrap();
    let writer = location.open_writer(false).await.unwrap();
    let revision = writer
        .get(record.recording_id)
        .await
        .unwrap()
        .unwrap()
        .revision;
    writer
        .import_record(plan.clone(), 0, record.clone())
        .await
        .unwrap();
    writer.finish_import(plan).await.unwrap();
    assert_eq!(
        writer
            .get(record.recording_id)
            .await
            .unwrap()
            .unwrap()
            .revision,
        revision
    );
    writer.close().await.unwrap();
}

#[tokio::test]
async fn corrupt_or_foreign_json_never_becomes_an_empty_catalog() {
    let store = Arc::new(JsonOnlyStore::default());
    let location = CatalogLocation::object_store(store.clone());
    let writer = location.open_writer(true).await.unwrap();
    let record = record();
    writer.put(record.clone(), None).await.unwrap();
    writer.close().await.unwrap();
    let path = Path::from(format!(
        ".bsr/records/{}/recording.json",
        record.recording_id
    ));
    let original = store.get(&path).await.unwrap().bytes().await.unwrap();
    store.put(&path, "{broken".into()).await.unwrap();
    assert!(location.open_reader().await.unwrap().scan().await.is_err());
    assert_eq!(
        location.open_writer(false).await.err().unwrap().kind(),
        io::ErrorKind::InvalidData
    );
    let mut doc: serde_json::Value = serde_json::from_slice(&original).unwrap();
    doc["storage_uuid"] = Uuid::new_v4().to_string().into();
    store
        .put(&path, serde_json::to_vec(&doc).unwrap().into())
        .await
        .unwrap();
    assert!(location.open_writer(false).await.is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn local_catalog_refuses_symlinked_metadata_directories() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join(".bsr")).unwrap();
    let location = CatalogLocation::local(root.path()).unwrap();
    assert!(location.open_writer(true).await.is_err());
    assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
}
