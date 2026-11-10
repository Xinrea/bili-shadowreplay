use super::projection::CatalogTotals;
use super::*;
use crate::{
    config::{RecordingBackendConfig, StorageConfig},
    database::credentials::CredentialCipher,
    events::EventBus,
};
use recorder::platforms::PlatformType;
use recording_catalog::{object_store::memory::InMemory, CatalogLocation};
use std::{collections::HashMap, sync::Arc};

async fn database() -> Database {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    for migration in crate::get_migrations() {
        sqlx::raw_sql(migration.sql).execute(&pool).await.unwrap();
    }
    let db = Database::new(
        pool,
        CredentialCipher::new(&[7; 32]).unwrap(),
        EventBus::new(),
    );
    db.initialize_storage_config(&StorageConfig {
        default_backend: "legacy".into(),
        credential_revisions: HashMap::new(),
        backends: HashMap::from([(
            "legacy".into(),
            RecordingBackendConfig {
                kind: "local".into(),
                root: "/legacy".into(),
                s3: None,
            },
        )]),
    })
    .await
    .unwrap();
    db
}

async fn legacy(db: &Database, live: &str) {
    db.add_record(
        PlatformType::BiliBili,
        "parent",
        live,
        "123",
        "Original title",
        None,
    )
    .await
    .unwrap();
    db.update_record_delta(live, 1.5, 42).await.unwrap();
}

fn record(live: &str, time: i64) -> CatalogRecord {
    CatalogRecord {
        recording_id: Uuid::new_v4(),
        platform: "bilibili".into(),
        room_id: "123".into(),
        live_id: live.into(),
        parent_id: "parent".into(),
        title: live.into(),
        created_at_ms: time,
        length: 1.5,
        size: 42,
        cover: None,
        results: serde_json::Value::Null,
    }
}

async fn writer() -> CatalogWriter {
    CatalogLocation::object_store(Arc::new(InMemory::new()))
        .open_writer(true)
        .await
        .unwrap()
}

#[tokio::test]
async fn frozen_legacy_snapshot_preserves_completed_results_and_survives_lost_ack() {
    let db = database().await;
    legacy(&db, "1").await;
    legacy(&db, "2").await;
    sqlx::query("UPDATE records SET created_at = '2024-01-02 03:04:05'")
        .execute(&db.pool)
        .await
        .unwrap();
    db.start_record_summary("bilibili", "123", "1", "private-task-id", false)
        .await
        .unwrap();
    db.save_record_summary_subtitle("bilibili", "123", "1", "srt", "text", 1.5)
        .await
        .unwrap();
    db.complete_record_summary("bilibili", "123", "1", "Summary", "[]", "openai", "model")
        .await
        .unwrap();
    db.start_record_summary("bilibili", "123", "2", "running-task", false)
        .await
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    let location = CatalogLocation::local(root.path()).unwrap();
    let writer = location.open_writer(true).await.unwrap();
    let plan = db
        .prepare_legacy_catalog_import("legacy", writer.identity().storage_uuid)
        .await
        .unwrap();
    assert_eq!(plan.count, 2);
    assert!(db
        .prepare_legacy_catalog_import("legacy", Uuid::new_v4())
        .await
        .is_err());
    sqlx::query("UPDATE records SET title = 'Changed later', size = 900")
        .execute(&db.pool)
        .await
        .unwrap();
    assert_eq!(
        plan,
        db.prepare_legacy_catalog_import("legacy", writer.identity().storage_uuid)
            .await
            .unwrap()
    );
    // Simulate a process that copied one entry and lost all in-memory state.
    let first: String =
        sqlx::query_scalar("SELECT record FROM catalog_legacy_entries WHERE ordinal = 0")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    let first: CatalogRecord = from_json(&first).unwrap();
    writer.begin_import(plan.clone()).await.unwrap();
    writer
        .import_record(plan.clone(), 0, first.clone())
        .await
        .unwrap();
    writer.close().await.unwrap();
    let reopened = Database::new(
        db.pool.clone(),
        CredentialCipher::new(&[7; 32]).unwrap(),
        EventBus::new(),
    );
    let writer = location.open_writer(false).await.unwrap();
    sqlx::raw_sql("CREATE TRIGGER reject_import_ack BEFORE UPDATE ON catalog_legacy_imports BEGIN SELECT RAISE(ABORT, 'simulated lost acknowledgement'); END;")
        .execute(&db.pool).await.unwrap();
    assert!(reopened
        .resume_legacy_catalog_import("legacy", &writer)
        .await
        .is_err());
    assert!(writer.begin_import(plan.clone()).await.unwrap().complete);
    let entries = writer.list(None, 10).await.unwrap().entries;
    assert_eq!(entries.len(), 2);
    let imported = entries
        .iter()
        .find(|entry| entry.record.live_id == "1")
        .unwrap();
    assert_eq!(imported.record.title, "Original title");
    assert_eq!(imported.record.size, 42);
    assert_eq!(imported.record.created_at_ms, 1704164645000);
    let result: ArchiveResults = serde_json::from_value(imported.record.results.clone()).unwrap();
    assert_eq!(
        result.summary.unwrap().summary_markdown.as_deref(),
        Some("Summary")
    );
    let encoded = json(&entries).unwrap();
    assert!(!encoded.contains("private-task-id"));
    assert!(!encoded.contains("running-task"));
    assert!(entries
        .iter()
        .find(|entry| entry.record.live_id == "2")
        .unwrap()
        .record
        .results["summary"]
        .is_null());
    writer
        .delete(imported.record.recording_id, imported.revision)
        .await
        .unwrap();
    sqlx::raw_sql("DROP TRIGGER reject_import_ack")
        .execute(&db.pool)
        .await
        .unwrap();
    reopened
        .resume_legacy_catalog_import("legacy", &writer)
        .await
        .unwrap();
    assert!(writer
        .get(imported.record.recording_id)
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        db.get_recent_record("", 0, 10).await.unwrap().len(),
        2,
        "legacy source retained"
    );
    assert!(
        db.get_record_summary("bilibili", "123", "2")
            .await
            .unwrap()
            .is_some(),
        "task state retained"
    );
    writer.close().await.unwrap();
}

#[tokio::test]
async fn invalid_legacy_rows_rollback_the_whole_snapshot() {
    let db = database().await;
    legacy(&db, "1").await;
    legacy(&db, "2").await;
    sqlx::query("UPDATE records SET created_at = 'not a date' WHERE live_id = '2'")
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(db
        .prepare_legacy_catalog_import("legacy", Uuid::new_v4())
        .await
        .is_err());
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM catalog_legacy_entries")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM catalog_legacy_imports")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert_eq!(db.get_recent_record("", 0, 10).await.unwrap().len(), 2);
}

#[tokio::test]
async fn projection_aggregates_by_uuid_and_unmount_never_deletes_content() {
    let db = database().await;
    let a = writer().await;
    let b = writer().await;
    // Even identical legacy IDs and recording UUIDs across two roots must not
    // silently overwrite each other in the aggregate projection.
    let record = record("same-live", 1);
    a.put(record.clone(), None).await.unwrap();
    b.put(record.clone(), None).await.unwrap();
    let ma = db
        .mount_catalog("a", a.identity().storage_uuid)
        .await
        .unwrap();
    let mb = db
        .mount_catalog("b", b.identity().storage_uuid)
        .await
        .unwrap();
    assert!(db
        .mount_catalog("another-alias", a.identity().storage_uuid)
        .await
        .is_err());
    assert!(db.mount_catalog("a", Uuid::new_v4()).await.is_err());
    assert!(db
        .rebuild_catalog_projection(ma.clone(), b.scan().await.unwrap())
        .await
        .is_err());
    db.rebuild_catalog_projection(ma.clone(), a.scan().await.unwrap())
        .await
        .unwrap();
    db.rebuild_catalog_projection(mb.clone(), b.scan().await.unwrap())
        .await
        .unwrap();
    assert_eq!(
        db.catalog_totals().await.unwrap(),
        CatalogTotals {
            count: 2,
            length: 3.0,
            size: 84
        }
    );
    assert_eq!(db.catalog_records(0, 1).await.unwrap().len(), 1);
    assert_eq!(db.catalog_records(1, 1).await.unwrap().len(), 1);
    let stale_scan = a.scan().await.unwrap();
    db.unmount_catalog(&ma).await.unwrap();
    assert_eq!(db.catalog_records(0, 10).await.unwrap().len(), 1);
    assert!(a.get(record.recording_id).await.unwrap().is_some());
    db.mark_catalog_offline(&mb, "S3 unavailable")
        .await
        .unwrap();
    let cached = db.catalog_records(0, 10).await.unwrap();
    assert!(!cached[0].online);
    assert_eq!(cached[0].error.as_deref(), Some("S3 unavailable"));
    db.unmount_catalog(&mb).await.unwrap();
    assert_eq!(db.catalog_totals().await.unwrap(), CatalogTotals::default());
    let remounted = db
        .mount_catalog("b", a.identity().storage_uuid)
        .await
        .unwrap();
    assert_eq!(
        db.catalog_records(0, 10).await.unwrap().len(),
        1,
        "cache restored by UUID under new alias"
    );
    assert!(db
        .rebuild_catalog_projection(ma.clone(), stale_scan)
        .await
        .is_err());
    assert!(db.mark_catalog_offline(&ma, "stale error").await.is_err());
    assert!(
        db.unmount_catalog(&mb).await.is_err(),
        "old alias incarnation cannot remove new mount"
    );
    db.rebuild_catalog_projection(remounted, a.scan().await.unwrap())
        .await
        .unwrap();
    let cached = db.catalog_records(0, 10).await.unwrap();
    assert!(cached[0].online);
    assert_eq!(cached[0].backend_id, "b");
    a.close().await.unwrap();
    b.close().await.unwrap();
}

#[tokio::test]
async fn failed_refresh_keeps_last_complete_generation_then_recovers() {
    let db = database().await;
    let writer = writer().await;
    let old = writer.put(record("old", 1000), None).await.unwrap();
    let mount = db
        .mount_catalog("archive", writer.identity().storage_uuid)
        .await
        .unwrap();
    db.rebuild_catalog_projection(mount.clone(), writer.scan().await.unwrap())
        .await
        .unwrap();
    for i in 0..130 {
        writer.put(record(&i.to_string(), i), None).await.unwrap();
    }
    // Fail on the final page, after the first staging page has committed.
    sqlx::raw_sql("CREATE TRIGGER reject_projection BEFORE INSERT ON catalog_projection_entries WHEN NEW.created_at_ms = 0 BEGIN SELECT RAISE(ABORT, 'injected page failure'); END;")
        .execute(&db.pool).await.unwrap();
    assert!(db
        .rebuild_catalog_projection(mount.clone(), writer.scan().await.unwrap())
        .await
        .is_err());
    let rows = db.catalog_records(0, 1000).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].entry, old);
    let staged: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM catalog_projection_entries")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(staged, 1, "failed generation is cleaned up");
    sqlx::raw_sql("DROP TRIGGER reject_projection")
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::raw_sql("CREATE TRIGGER reject_projection_publish BEFORE UPDATE ON catalog_projection_heads BEGIN SELECT RAISE(ABORT, 'injected publish failure'); END;")
        .execute(&db.pool).await.unwrap();
    assert!(db
        .rebuild_catalog_projection(mount.clone(), writer.scan().await.unwrap())
        .await
        .is_err());
    assert_eq!(db.catalog_records(0, 1000).await.unwrap()[0].entry, old);
    assert_eq!(db.catalog_totals().await.unwrap().count, 1);
    sqlx::raw_sql("DROP TRIGGER reject_projection_publish")
        .execute(&db.pool)
        .await
        .unwrap();
    writer
        .delete(old.record.recording_id, old.revision)
        .await
        .unwrap();
    db.rebuild_catalog_projection(mount.clone(), writer.scan().await.unwrap())
        .await
        .unwrap();
    assert_eq!(db.catalog_totals().await.unwrap().count, 130);
    for entry in writer.list(None, 1000).await.unwrap().entries {
        writer
            .delete(entry.record.recording_id, entry.revision)
            .await
            .unwrap();
    }
    db.rebuild_catalog_projection(mount, writer.scan().await.unwrap())
        .await
        .unwrap();
    assert_eq!(
        db.catalog_totals().await.unwrap(),
        CatalogTotals::default(),
        "verified empty catalog replaces old cache"
    );
    writer.close().await.unwrap();
}

#[tokio::test]
async fn old_scans_cannot_resurrect_a_newer_deletion() {
    let db = database().await;
    let writer = writer().await;
    let entry = writer.put(record("deleted", 0), None).await.unwrap();
    let mount = db
        .mount_catalog("archive", writer.identity().storage_uuid)
        .await
        .unwrap();
    let old_scan = writer.scan().await.unwrap();
    let before = old_scan.revision();
    writer
        .delete(entry.record.recording_id, entry.revision)
        .await
        .unwrap();
    let new_scan = writer.scan().await.unwrap();
    assert!(
        new_scan.revision() > before,
        "deletion advances the scan watermark"
    );
    db.rebuild_catalog_projection(mount.clone(), new_scan)
        .await
        .unwrap();
    assert!(db
        .rebuild_catalog_projection(mount, old_scan)
        .await
        .is_err());
    assert_eq!(db.catalog_totals().await.unwrap(), CatalogTotals::default());
    writer.close().await.unwrap();
}

#[tokio::test]
async fn import_snapshot_uses_bindings_and_excludes_deletion_tombstones() {
    let db = database().await;
    legacy(&db, "1").await;
    legacy(&db, "2").await;
    legacy(&db, "3").await;
    db.bind_recording_storage("bilibili", "123", "2", "other")
        .await
        .unwrap();
    sqlx::query("INSERT INTO recording_storage_deletions VALUES ('bilibili', '123', '3')")
        .execute(&db.pool)
        .await
        .unwrap();
    let plan = db
        .prepare_legacy_catalog_import("legacy", Uuid::new_v4())
        .await
        .unwrap();
    assert_eq!(plan.count, 1);
    let row: String = sqlx::query_scalar("SELECT record FROM catalog_legacy_entries")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(from_json::<CatalogRecord>(&row).unwrap().live_id, "1");
}

#[tokio::test]
#[ignore = "requires RustFS and explicit S3_TEST_* test credentials"]
async fn rustfs_catalog_import_projection_and_remount() {
    use crate::{archive_storage::ArchiveStorage, config::Config};
    use futures::TryStreamExt;
    use recording_catalog::object_store::{
        aws::AmazonS3Builder, prefix::PrefixStore, ObjectStore, ObjectStoreExt,
    };
    let directory = tempfile::tempdir().unwrap();
    let db = Arc::new(database().await);
    let config = Arc::new(tokio::sync::RwLock::new(
        Config::load(
            &directory.path().join("config.toml"),
            &directory.path().join("recordings"),
            &directory.path().join("output"),
        )
        .unwrap(),
    ));
    let storage = ArchiveStorage::new(db.clone(), config);
    let current = storage.configuration().await.unwrap();
    let s3 = recording_storage::S3Config {
        endpoint: std::env::var("S3_TEST_ENDPOINT").expect("set S3_TEST_ENDPOINT"),
        bucket: std::env::var("S3_TEST_BUCKET").expect("set S3_TEST_BUCKET"),
        region: "us-east-1".into(),
        prefix: format!("catalog-migration-test-{}", Uuid::new_v4()),
        path_style: true,
    };
    eprintln!("RustFS catalog migration test prefix: {}", s3.prefix);
    let credentials = recording_storage::S3Credentials {
        access_key_id: std::env::var("S3_TEST_ACCESS_KEY_ID").expect("set S3_TEST_ACCESS_KEY_ID"),
        secret_access_key: std::env::var("S3_TEST_SECRET_ACCESS_KEY")
            .expect("set S3_TEST_SECRET_ACCESS_KEY"),
        session_token: String::new(),
    };
    let mut next = current.clone();
    next.backends.insert(
        "remote".into(),
        RecordingBackendConfig {
            kind: "s3".into(),
            root: String::new(),
            s3: Some(s3.clone()),
        },
    );
    storage
        .replace_configuration_with_credentials(
            next,
            &current,
            &HashMap::from([("remote".into(), credentials.clone())]),
        )
        .await
        .unwrap();
    legacy(&db, "remote-live").await;
    db.bind_recording_storage("bilibili", "123", "remote-live", "remote")
        .await
        .unwrap();
    // The application adapter receives credentials from settings/SQLite only.
    let location = storage.catalog_location("remote").await.unwrap();
    let writer = location.open_writer(true).await.unwrap();
    let uuid = writer.identity().storage_uuid;
    db.prepare_legacy_catalog_import("remote", uuid)
        .await
        .unwrap();
    db.resume_legacy_catalog_import("remote", &writer)
        .await
        .unwrap();
    let imported = writer.list(None, 10).await.unwrap().entries.remove(0);
    writer.close().await.unwrap();
    let reader = location.open_reader().await.unwrap();
    let mount = db.mount_catalog("remote", uuid).await.unwrap();
    db.rebuild_catalog_projection(mount.clone(), reader.scan().await.unwrap())
        .await
        .unwrap();
    assert_eq!(db.catalog_totals().await.unwrap().count, 1);
    db.unmount_catalog(&mount).await.unwrap();
    assert_eq!(db.catalog_totals().await.unwrap().count, 0);
    assert!(reader
        .get(imported.record.recording_id)
        .await
        .unwrap()
        .is_some());
    reader.close().await.unwrap();

    // A fresh main DB discovers records without the original legacy table,
    // configuration alias, import journal or SQLite recording primary keys.
    let restored = database().await;
    let reader = location.open_reader().await.unwrap();
    let mount = restored.mount_catalog("renamed", uuid).await.unwrap();
    restored
        .rebuild_catalog_projection(mount, reader.scan().await.unwrap())
        .await
        .unwrap();
    assert_eq!(
        restored.catalog_records(0, 10).await.unwrap()[0].entry,
        imported
    );
    reader.close().await.unwrap();

    let writer = location.open_writer(false).await.unwrap();
    writer
        .delete(imported.record.recording_id, imported.revision)
        .await
        .unwrap();
    db.resume_legacy_catalog_import("remote", &writer)
        .await
        .unwrap();
    assert!(writer
        .get(imported.record.recording_id)
        .await
        .unwrap()
        .is_none());
    writer.close().await.unwrap();

    // Delete only this test's random prefix, never user recordings or the bucket.
    let store = PrefixStore::new(
        AmazonS3Builder::new()
            .with_endpoint(&s3.endpoint)
            .with_allow_http(s3.endpoint.starts_with("http://"))
            .with_bucket_name(&s3.bucket)
            .with_region(&s3.region)
            .with_access_key_id(&credentials.access_key_id)
            .with_secret_access_key(&credentials.secret_access_key)
            .build()
            .unwrap(),
        s3.prefix,
    );
    for object in store.list(None).try_collect::<Vec<_>>().await.unwrap() {
        store.delete(&object.location).await.unwrap();
    }
}
