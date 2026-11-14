use recording_catalog::{CatalogLocation, CatalogRecord};
use recording_storage::{WebDavBackend, WebDavConfig, WebDavCredentials};
use std::sync::Arc;
use uuid::Uuid;
#[path = "../../recording_storage/tests/support/webdav.rs"]
mod fixture;

#[tokio::test]
async fn webdav_catalog_survives_reopen_and_keeps_deletion_receipts() {
    let fixture = fixture::Fixture::new().await;
    let backend = Arc::new(
        WebDavBackend::new(
            &WebDavConfig {
                endpoint: fixture.endpoint.clone(),
                prefix: "recordings".into(),
            },
            &WebDavCredentials {
                username: "user".into(),
                password: "pass".into(),
            },
        )
        .unwrap(),
    );
    let location = CatalogLocation::recording_backend(backend);
    let writer = location.open_writer(true).await.unwrap();
    let identity = writer.identity().clone();
    let record = CatalogRecord {
        recording_id: Uuid::new_v4(),
        platform: "bilibili".into(),
        room_id: "123".into(),
        live_id: "456".into(),
        parent_id: "456".into(),
        title: "WebDAV 录播".into(),
        created_at_ms: 123456789,
        length: 10.0,
        size: 100,
        cover: None,
        results: serde_json::json!({}),
    };
    let entry = writer.put(record.clone(), None).await.unwrap();
    writer.close().await.unwrap();
    let reader = location.open_reader().await.unwrap();
    assert_eq!(reader.identity(), &identity);
    assert_eq!(
        reader
            .get(record.recording_id)
            .await
            .unwrap()
            .unwrap()
            .record,
        record
    );
    assert_eq!(reader.list(None, 10).await.unwrap().entries.len(), 1);
    let writer = location.open_writer(false).await.unwrap();
    writer
        .delete_recording(record.clone(), entry.revision)
        .await
        .unwrap();
    assert_eq!(
        writer.pending_deletions().await.unwrap(),
        vec![record.clone()]
    );
    writer.close().await.unwrap();
    let writer = location.open_writer(false).await.unwrap();
    assert_eq!(
        writer.pending_deletions().await.unwrap(),
        vec![record.clone()]
    );
    writer.finish_deletion(record.recording_id).await.unwrap();
    assert!(writer.pending_deletions().await.unwrap().is_empty());
    writer.close().await.unwrap();
    assert!(location
        .open_reader()
        .await
        .unwrap()
        .list(None, 10)
        .await
        .unwrap()
        .entries
        .is_empty());
    assert!(fixture
        .server
        .files
        .lock()
        .unwrap()
        .contains_key("/dav/recordings/.bsr/storage.json"));
}
