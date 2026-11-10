use recording_catalog::{
    object_store::{memory::InMemory, ObjectStore},
    CatalogLocation, CatalogRecord, ImportDigest, ImportPlan,
};
use std::{io, sync::Arc};
use uuid::Uuid;

fn record(live: &str) -> CatalogRecord {
    CatalogRecord {
        recording_id: Uuid::new_v4(),
        platform: "bilibili".into(),
        room_id: "123".into(),
        live_id: live.into(),
        parent_id: "parent".into(),
        title: live.into(),
        created_at_ms: 0,
        length: 1.5,
        size: 42,
        cover: None,
        results: serde_json::Value::Null,
    }
}

fn plan(storage_uuid: Uuid, records: &[CatalogRecord]) -> ImportPlan {
    let mut digest = ImportDigest::default();
    for record in records {
        digest.append(record).unwrap();
    }
    ImportPlan {
        import_id: Uuid::new_v4(),
        storage_uuid,
        count: records.len() as u64,
        digest,
    }
}

async fn contract(location: CatalogLocation) {
    let writer = location.open_writer(true).await.unwrap();
    let records = [record("1"), record("2")];
    let plan = plan(writer.identity().storage_uuid, &records);
    assert_eq!(
        writer.begin_import(plan.clone()).await.unwrap().next_index,
        0
    );
    assert!(writer.finish_import(plan.clone()).await.is_err());
    assert!(writer
        .import_record(plan.clone(), 1, records[1].clone())
        .await
        .is_err());
    writer
        .import_record(plan.clone(), 0, records[0].clone())
        .await
        .unwrap();
    assert!(
        writer.scan().await.is_err(),
        "do not project partial imports"
    );
    writer.close().await.unwrap();

    let reader = location.open_reader().await.unwrap();
    assert!(
        reader.scan().await.is_err(),
        "other instances must not project partial imports"
    );
    reader.close().await.unwrap();
    let writer = location.open_writer(false).await.unwrap();
    assert_eq!(
        writer.begin_import(plan.clone()).await.unwrap().next_index,
        1
    );
    let mut changed = records[0].clone();
    changed.title = "different snapshot".into();
    assert!(writer
        .import_record(plan.clone(), 0, changed.clone())
        .await
        .is_err());
    let entry = writer.get(records[0].recording_id).await.unwrap().unwrap();
    writer
        .put(changed.clone(), Some(entry.revision))
        .await
        .unwrap();
    writer
        .import_record(plan.clone(), 0, records[0].clone())
        .await
        .unwrap();
    assert_eq!(
        writer
            .get(records[0].recording_id)
            .await
            .unwrap()
            .unwrap()
            .record,
        changed
    );
    writer
        .import_record(plan.clone(), 1, records[1].clone())
        .await
        .unwrap();
    writer.finish_import(plan.clone()).await.unwrap();
    writer.close().await.unwrap();

    let writer = location.open_writer(false).await.unwrap();
    assert!(writer.begin_import(plan.clone()).await.unwrap().complete);
    let entry = writer.get(records[0].recording_id).await.unwrap().unwrap();
    writer
        .delete(entry.record.recording_id, entry.revision)
        .await
        .unwrap();
    for (index, record) in records.iter().enumerate() {
        writer
            .import_record(plan.clone(), index as u64, record.clone())
            .await
            .unwrap();
    }
    writer.finish_import(plan.clone()).await.unwrap();
    assert!(writer.get(records[0].recording_id).await.unwrap().is_none());
    let mut restarted = plan.clone();
    restarted.import_id = Uuid::new_v4();
    assert!(
        writer.begin_import(restarted).await.is_err(),
        "a lost local journal cannot restart import with fresh UUIDs"
    );
    let mut wrong = plan.clone();
    wrong.count += 1;
    assert!(writer.begin_import(wrong).await.is_err());
    let mut wrong = plan;
    wrong.storage_uuid = Uuid::new_v4();
    assert!(writer.begin_import(wrong).await.is_err());
    writer.close().await.unwrap();
}

#[tokio::test]
async fn local_import_receipts_survive_reopen_edits_and_deletion() {
    let root = tempfile::tempdir().unwrap();
    contract(CatalogLocation::local(root.path()).unwrap()).await;
}

#[tokio::test]
async fn object_import_receipts_survive_reopen_edits_and_deletion() {
    let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
    contract(CatalogLocation::object_store(store)).await;
}

#[tokio::test]
async fn corrupt_snapshots_and_foreign_namespaces_are_not_overwritten() {
    let writer = CatalogLocation::object_store(Arc::new(InMemory::new()))
        .open_writer(true)
        .await
        .unwrap();
    let original = record("original");
    writer.put(original.clone(), None).await.unwrap();
    let mut duplicate = original.clone();
    duplicate.recording_id = Uuid::new_v4();
    let collision = plan(writer.identity().storage_uuid, &[duplicate.clone()]);
    writer.begin_import(collision.clone()).await.unwrap();
    assert_eq!(
        writer
            .import_record(collision, 0, duplicate)
            .await
            .unwrap_err()
            .kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(
        writer
            .get(original.recording_id)
            .await
            .unwrap()
            .unwrap()
            .record,
        original
    );
    writer.close().await.unwrap();
    let writer = CatalogLocation::object_store(Arc::new(InMemory::new()))
        .open_writer(true)
        .await
        .unwrap();
    let mut corrupt = plan(writer.identity().storage_uuid, &[record("new")]);
    let unexpected = record("wrong");
    writer.begin_import(corrupt.clone()).await.unwrap();
    assert!(writer
        .import_record(corrupt.clone(), 0, unexpected.clone())
        .await
        .is_err());
    assert!(writer.get(unexpected.recording_id).await.unwrap().is_none());
    assert_eq!(
        writer
            .begin_import(corrupt.clone())
            .await
            .unwrap()
            .next_index,
        0
    );
    corrupt.digest = ImportDigest::default();
    assert!(writer.begin_import(corrupt).await.is_err());
    writer.close().await.unwrap();
    let writer = CatalogLocation::object_store(Arc::new(InMemory::new()))
        .open_writer(true)
        .await
        .unwrap();
    let empty = plan(writer.identity().storage_uuid, &[]);
    writer.begin_import(empty.clone()).await.unwrap();
    writer.finish_import(empty.clone()).await.unwrap();
    assert!(writer.begin_import(empty).await.unwrap().complete);
    writer.close().await.unwrap();
}

#[tokio::test]
async fn streaming_scan_keeps_one_read_view_across_writes_and_deletions() {
    let writer = CatalogLocation::object_store(Arc::new(InMemory::new()))
        .open_writer(true)
        .await
        .unwrap();
    let mut originals = Vec::new();
    for i in 0..5 {
        let mut record = record(&i.to_string());
        record.created_at_ms = i;
        originals.push(writer.put(record, None).await.unwrap());
    }
    let mut scan = writer.scan().await.unwrap();
    assert!(scan.next_page(0).await.is_err());
    assert_eq!(scan.next_page(1).await.unwrap()[0], originals[4]);
    writer
        .delete(originals[1].record.recording_id, originals[1].revision)
        .await
        .unwrap();
    let mut changed = originals[0].record.clone();
    changed.title = "updated".into();
    writer
        .put(changed, Some(originals[0].revision))
        .await
        .unwrap();
    writer.put(record("new"), None).await.unwrap();
    let rest = scan.next_page(100).await.unwrap();
    assert_eq!(
        rest,
        originals[..4].iter().rev().cloned().collect::<Vec<_>>()
    );
    assert!(scan.next_page(100).await.unwrap().is_empty());
    writer.close().await.unwrap();
}
