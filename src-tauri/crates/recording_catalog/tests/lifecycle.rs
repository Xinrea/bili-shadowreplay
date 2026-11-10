use recording_catalog::{object_store::memory::InMemory, CatalogLocation, CatalogRecord};
use std::sync::Arc;
use uuid::Uuid;

#[tokio::test]
async fn transfer_receipts_do_not_resurrect_targets_or_delete_recreated_sources() {
    let a = CatalogLocation::object_store(Arc::new(InMemory::new()))
        .open_writer(true)
        .await
        .unwrap();
    let b = CatalogLocation::object_store(Arc::new(InMemory::new()))
        .open_writer(true)
        .await
        .unwrap();
    let record = CatalogRecord {
        recording_id: Uuid::new_v4(),
        platform: "bilibili".into(),
        room_id: "123".into(),
        live_id: "live".into(),
        parent_id: "parent".into(),
        title: "title".into(),
        created_at_ms: 0,
        length: 1.0,
        size: 10,
        cover: None,
        results: serde_json::Value::Null,
    };
    let source = a.put(record.clone(), None).await.unwrap();
    let token = Uuid::new_v4();
    let target = b.receive_transfer(token, record.clone()).await.unwrap();
    assert!(
        b.receive_transfer(Uuid::new_v4(), record.clone())
            .await
            .is_err(),
        "identical unowned targets are not adopted"
    );
    a.retire_transfer(token, record.recording_id, source.revision)
        .await
        .unwrap();
    assert_eq!(
        b.receive_transfer(token, record.clone()).await.unwrap(),
        target
    );
    let restored = a.put(record.clone(), None).await.unwrap();
    a.retire_transfer(token, record.recording_id, source.revision)
        .await
        .unwrap();
    assert_eq!(a.get(record.recording_id).await.unwrap().unwrap(), restored);
    let mut forged = record.clone();
    forged.room_id = "other".into();
    assert!(a.delete_recording(forged, restored.revision).await.is_err());
    b.delete_recording(record.clone(), target.revision)
        .await
        .unwrap();
    b.delete_recording(record.clone(), target.revision)
        .await
        .unwrap();
    assert!(b.receive_transfer(token, record.clone()).await.is_err());
    assert!(b
        .receive_transfer(Uuid::new_v4(), record.clone())
        .await
        .is_err());
    assert!(b.find_recording("bilibili", "123", "live").await.is_err());
    a.close().await.unwrap();
    b.close().await.unwrap();
}
