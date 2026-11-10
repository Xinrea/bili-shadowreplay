use super::*;
use crate::{
    archive_storage::ArchiveStorage,
    config::{Config, RecordingBackendConfig},
    database::credentials::CredentialCipher,
    progress::progress_reporter::ProgressReporter,
};
use recorder::platforms::PlatformType;
use tokio::sync::RwLock;

async fn fixture() -> (
    tempfile::TempDir,
    Arc<Database>,
    Arc<ArchiveStorage>,
    Arc<RwLock<Config>>,
) {
    let root = tempfile::tempdir().unwrap();
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    for migration in crate::get_migrations() {
        sqlx::raw_sql(migration.sql).execute(&pool).await.unwrap();
    }
    let db = Arc::new(Database::new(
        pool,
        CredentialCipher::new(&[9; 32]).unwrap(),
        EventBus::new(),
    ));
    let mut config = Config::load(
        &root.path().join("config.toml"),
        &root.path().join("a"),
        &root.path().join("output"),
    )
    .unwrap();
    config.storage.default_backend = "a".into();
    config.storage.backends.clear();
    for id in ["a", "b"] {
        std::fs::create_dir_all(root.path().join(id)).unwrap();
        config.storage.backends.insert(
            id.into(),
            RecordingBackendConfig {
                kind: "local".into(),
                root: root
                    .path()
                    .join(id)
                    .canonicalize()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .into(),
                s3: None,
            },
        );
    }
    let config = Arc::new(RwLock::new(config));
    let storage = Arc::new(ArchiveStorage::new(db.clone(), config.clone()));
    storage.initialize().await.unwrap();
    (root, db, storage, config)
}

async fn create(db: &Database, storage: &ArchiveStorage, live: &str, alias: &str) {
    let scope = storage
        .selected_store(Some(alias))
        .await
        .unwrap()
        .recording("bilibili", "123", live)
        .unwrap();
    scope.prepare().await.unwrap();
    scope.write("video.ts", b"recording bytes").await.unwrap();
    db.add_record(PlatformType::BiliBili, "parent", live, "123", "Title", None)
        .await
        .unwrap();
    db.update_record_delta(live, 3.5, 15).await.unwrap();
    db.finish_catalog_recording(live).await.unwrap();
}

async fn assert_ready(db: &Database) {
    let statuses = db.catalog_status().await.unwrap();
    assert_eq!(statuses.len(), 2, "{statuses:?}");
    assert!(
        statuses.iter().all(|s| s.ready && s.writable),
        "{statuses:?}"
    );
}

async fn close(db: &Database) {
    let runtime = db.catalogs.get().unwrap();
    let aliases: Vec<_> = runtime.handles.lock().await.keys().cloned().collect();
    for alias in aliases {
        runtime.detach(&alias).await.unwrap();
    }
}

#[tokio::test]
async fn refresh_preserves_summary_work_but_reconciles_completed_and_removed_results() {
    for regenerate in [false, true] {
        let (_root, db, storage, _) = fixture().await;
        storage.start_catalogs(false).await.unwrap();
        create(&db, &storage, "summary", "a").await;
        if regenerate {
            db.start_record_summary("bilibili", "123", "summary", "previous-task", false)
                .await
                .unwrap();
            db.complete_record_summary(
                "bilibili",
                "123",
                "summary",
                "old summary",
                "[]",
                "test",
                "test",
            )
            .await
            .unwrap();
        }
        db.start_record_summary("bilibili", "123", "summary", "current-task", true)
            .await
            .unwrap();
        db.save_record_summary_subtitle(
            "bilibili",
            "123",
            "summary",
            "cached srt",
            "cached text",
            3.5,
        )
        .await
        .unwrap();
        let before = db
            .get_record_summary("bilibili", "123", "summary")
            .await
            .unwrap()
            .unwrap();
        storage.refresh_catalogs(true).await.unwrap();
        let after = db
            .get_record_summary("bilibili", "123", "summary")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::to_value(&before).unwrap(),
            serde_json::to_value(&after).unwrap()
        );
        db.fail_record_summary("bilibili", "123", "summary", "temporary failure")
            .await
            .unwrap();
        storage.refresh_catalogs(true).await.unwrap();
        let failed = db
            .get_record_summary("bilibili", "123", "summary")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(failed.status, "failed");
        assert_eq!(failed.subtitle_text.as_deref(), Some("cached text"));
        assert_eq!(failed.task_id.as_deref(), Some("current-task"));
        db.start_record_summary("bilibili", "123", "summary", "retry-task", false)
            .await
            .unwrap();
        db.complete_record_summary(
            "bilibili",
            "123",
            "summary",
            "new summary",
            "[]",
            "test",
            "test",
        )
        .await
        .unwrap();
        storage.refresh_catalogs(true).await.unwrap();
        let completed = db
            .get_record_summary("bilibili", "123", "summary")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(completed.status, "success");
        assert_eq!(completed.summary_markdown.as_deref(), Some("new summary"));
        let writer = db.catalogs.get().unwrap().handles.lock().await["a"]
            .writer
            .clone()
            .unwrap();
        let mut entry = writer
            .find_recording("bilibili", "123", "summary")
            .await
            .unwrap()
            .unwrap();
        entry.record.results = serde_json::json!({"version": 1, "summary": null});
        writer
            .put(entry.record, Some(entry.revision))
            .await
            .unwrap();
        storage.refresh_catalogs(true).await.unwrap();
        assert!(db
            .get_record_summary("bilibili", "123", "summary")
            .await
            .unwrap()
            .is_none());
        db.start_record_summary("bilibili", "123", "summary", "deleted-task", false)
            .await
            .unwrap();
        let writer = db.catalogs.get().unwrap().handles.lock().await["a"]
            .writer
            .clone()
            .unwrap();
        let entry = writer
            .find_recording("bilibili", "123", "summary")
            .await
            .unwrap()
            .unwrap();
        writer
            .delete_recording(entry.record, entry.revision)
            .await
            .unwrap();
        storage.refresh_catalogs(true).await.unwrap();
        assert!(db
            .get_record_summary("bilibili", "123", "summary")
            .await
            .unwrap()
            .is_none());
        close(&db).await;
    }
}

#[tokio::test]
async fn remount_finishes_media_cleanup_from_json_deletion_marker() {
    let (root, db, storage, _) = fixture().await;
    storage.start_catalogs(false).await.unwrap();
    create(&db, &storage, "delete-recovery", "a").await;
    let media = root.path().join("a/bilibili/123/delete-recovery/video.ts");
    assert!(media.exists());
    db.catalogs
        .get()
        .unwrap()
        .begin_delete("delete-recovery")
        .await
        .unwrap();
    // Simulate interruption before file removal and SQLite cleanup.
    assert!(media.exists());
    close(&db).await;
    storage.refresh_catalogs(true).await.unwrap();
    assert!(!media.exists());
    assert!(db.get_record("123", "delete-recovery").await.is_err());
    assert_ready(&db).await;
    close(&db).await;
}

async fn transfer(
    db: &Arc<Database>,
    storage: &ArchiveStorage,
    from: &str,
    to: &str,
) -> Option<String> {
    let task = db
        .generate_task("storage_transfer", "", "{}")
        .await
        .unwrap();
    let reporter = ProgressReporter::new(db.clone(), &EventBus::new(), &task.id)
        .await
        .unwrap();
    storage
        .execute_storage_transfer(from, to, storage.reserve_transfer().unwrap(), &reporter)
        .await
        .error
}

#[tokio::test]
async fn production_catalog_writes_unmount_remount_and_alias_reuse() {
    let (root, db, storage, _) = fixture().await;
    // Existing records and completed results migrate before normal recording starts.
    create(&db, &storage, "old", "a").await;
    storage.start_catalogs(false).await.unwrap();
    assert_ready(&db).await;
    create(&db, &storage, "new", "a").await;
    db.start_record_summary("bilibili", "123", "new", "task", false)
        .await
        .unwrap();
    db.save_record_summary_subtitle("bilibili", "123", "new", "srt", "text", 3.5)
        .await
        .unwrap();
    db.complete_record_summary("bilibili", "123", "new", "Summary", "[]", "openai", "model")
        .await
        .unwrap();
    db.add_recorder_with_storage(PlatformType::BiliBili, "123", "", Some("a"))
        .await
        .unwrap();
    let original = storage.configuration().await.unwrap();
    let mut removed = original.clone();
    removed.default_backend = "b".into();
    removed.backends.remove("a");
    storage.replace_configuration(removed).await.unwrap();
    assert!(db.get_recent_record("", 0, 100).await.unwrap().is_empty());
    assert_eq!(db.get_record_disk_usage().await.unwrap(), 0);
    assert!(root.path().join("a/bilibili/123/new/video.ts").exists());
    let mut reused = storage.configuration().await.unwrap();
    reused.backends.insert(
        "a".into(),
        RecordingBackendConfig {
            kind: "local".into(),
            root: root.path().join("different").to_str().unwrap().into(),
            s3: None,
        },
    );
    storage.replace_configuration(reused).await.unwrap();
    assert!(
        db.recorder_storage_id("bilibili", "123").await.is_err(),
        "old room preference must not follow alias reuse"
    );
    assert!(
        storage.recording("bilibili", "123", "old").await.is_err(),
        "an unmounted URL must not fall back to the reused legacy alias"
    );
    let mut restored = storage.configuration().await.unwrap();
    restored
        .backends
        .insert("restored".into(), original.backends["a"].clone());
    storage.replace_configuration(restored).await.unwrap();
    assert_eq!(db.get_recent_record("", 0, 100).await.unwrap().len(), 2);
    assert_eq!(
        db.recorder_storage_id("bilibili", "123")
            .await
            .unwrap()
            .as_deref(),
        Some("restored")
    );
    sqlx::query("DELETE FROM catalog_record_links WHERE live_id='old'")
        .execute(&db.pool)
        .await
        .unwrap();
    storage.refresh_catalogs(true).await.unwrap();
    assert!(
        db.get_record("123", "old").await.unwrap().storage_available,
        "damaged projection links are rebuilt, not treated as another legacy import"
    );
    assert_eq!(
        db.get_record_summary("bilibili", "123", "new")
            .await
            .unwrap()
            .unwrap()
            .summary_markdown
            .as_deref(),
        Some("Summary")
    );
    storage
        .recording("bilibili", "123", "new")
        .await
        .unwrap()
        .remove_all()
        .await
        .unwrap();
    db.remove_record("new").await.unwrap();
    storage
        .forget_recording("bilibili", "123", "new")
        .await
        .unwrap();
    storage.refresh_catalogs(true).await.unwrap();
    assert_eq!(db.get_recent_record("", 0, 100).await.unwrap().len(), 1);
    assert!(db.get_record("123", "new").await.is_err());
    close(&db).await;
}

#[tokio::test]
async fn catalog_transfer_round_trip_and_lost_binding_commit_recover() {
    let (_root, db, storage, config) = fixture().await;
    storage.start_catalogs(false).await.unwrap();
    assert_ready(&db).await;
    create(&db, &storage, "move", "a").await;
    sqlx::raw_sql("CREATE TRIGGER fail_catalog_binding BEFORE UPDATE OF storage_id ON recording_storage_bindings WHEN NEW.storage_id='b' BEGIN SELECT RAISE(ABORT,'injected commit failure'); END")
        .execute(&db.pool).await.unwrap();
    let failure = transfer(&db, &storage, "a", "b").await;
    assert!(
        failure
            .as_deref()
            .is_some_and(|error| error.contains("injected commit failure")),
        "{failure:?}"
    );
    assert_eq!(db.storage_transfer_journals().await.unwrap().len(), 1);
    close(&db).await;
    let reopened = Arc::new(Database::new(
        db.pool.clone(),
        CredentialCipher::new(&[9; 32]).unwrap(),
        EventBus::new(),
    ));
    let restored = Arc::new(ArchiveStorage::new(reopened.clone(), config));
    restored.start_catalogs(false).await.unwrap();
    assert_ready(&reopened).await;
    sqlx::raw_sql("DROP TRIGGER fail_catalog_binding")
        .execute(&db.pool)
        .await
        .unwrap();
    assert_eq!(transfer(&reopened, &restored, "a", "b").await, None);
    assert_eq!(
        reopened
            .get_record("123", "move")
            .await
            .unwrap()
            .storage_backend_id,
        "b"
    );
    assert_eq!(
        restored
            .recording("bilibili", "123", "move")
            .await
            .unwrap()
            .read("video.ts")
            .await
            .unwrap(),
        b"recording bytes"
    );
    assert_eq!(transfer(&reopened, &restored, "b", "a").await, None);
    restored.refresh_catalogs(true).await.unwrap();
    assert_eq!(
        reopened
            .get_record("123", "move")
            .await
            .unwrap()
            .storage_backend_id,
        "a"
    );
    assert!(reopened
        .storage_transfer_journals()
        .await
        .unwrap()
        .is_empty());
    close(&reopened).await;
}

#[tokio::test]
async fn unavailable_catalog_preserves_cached_rows_and_rejects_writes() {
    let (root, db, storage, _) = fixture().await;
    storage.start_catalogs(false).await.unwrap();
    create(&db, &storage, "offline", "a").await;
    db.catalogs.get().unwrap().detach("a").await.unwrap();
    std::fs::rename(root.path().join("a"), root.path().join("disconnected")).unwrap();
    storage.refresh_catalogs(true).await.unwrap();
    let row = db.get_record("123", "offline").await.unwrap();
    assert!(!row.storage_available);
    assert!(storage
        .recording("bilibili", "123", "offline")
        .await
        .is_err());
    assert!(
        !root.path().join("a").exists(),
        "a missing mount must not be recreated"
    );
    assert!(storage.selected_store(Some("a")).await.is_err());
    std::fs::rename(root.path().join("disconnected"), root.path().join("a")).unwrap();
    storage.refresh_catalogs(true).await.unwrap();
    assert!(
        db.get_record("123", "offline")
            .await
            .unwrap()
            .storage_available
    );
    close(&db).await;
}

#[tokio::test]
async fn pending_end_events_and_overlapping_namespaces_block_configuration_changes() {
    let (root, db, storage, _) = fixture().await;
    storage.start_catalogs(false).await.unwrap();
    let initial = storage.configuration().await.unwrap();
    let mut overlapping = initial.clone();
    overlapping.backends.insert(
        "nested".into(),
        RecordingBackendConfig {
            kind: "local".into(),
            root: root.path().join("a/nested").to_str().unwrap().into(),
            s3: None,
        },
    );
    assert!(storage.replace_configuration(overlapping).await.is_err());
    assert_eq!(storage.configuration().await.unwrap(), initial);
    assert!(!root.path().join("a/nested").exists());
    let scope = storage
        .selected_store(Some("a"))
        .await
        .unwrap()
        .recording("bilibili", "123", "pending-end")
        .unwrap();
    scope.prepare().await.unwrap();
    drop(scope);
    db.add_record(
        PlatformType::BiliBili,
        "parent",
        "pending-end",
        "123",
        "Title",
        None,
    )
    .await
    .unwrap();
    let mut removed = initial;
    removed.default_backend = "b".into();
    removed.backends.remove("a");
    assert!(storage
        .replace_configuration(removed.clone())
        .await
        .is_err());
    db.update_record_delta("pending-end", 1.0, 1).await.unwrap();
    db.finish_catalog_recording("pending-end").await.unwrap();
    storage.replace_configuration(removed).await.unwrap();
    assert!(db.get_recent_record("", 0, 10).await.unwrap().is_empty());
    close(&db).await;
}

#[tokio::test]
async fn another_writer_allows_reading_but_blocks_asset_mutations() {
    let (_root, db, storage, config) = fixture().await;
    storage.start_catalogs(false).await.unwrap();
    create(&db, &storage, "shared", "a").await;
    let (_other_root, other_db, _, _) = fixture().await;
    // Use the same storage roots with an independent main database.
    sqlx::query("DELETE FROM recording_storage_enabled")
        .execute(&other_db.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM recording_storage_backends")
        .execute(&other_db.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM recording_storage_configuration")
        .execute(&other_db.pool)
        .await
        .unwrap();
    let other = Arc::new(ArchiveStorage::new(other_db.clone(), config));
    other.start_catalogs(false).await.unwrap();
    assert!(
        other_db
            .get_record("123", "shared")
            .await
            .unwrap()
            .storage_available
    );
    assert!(
        !other_db
            .catalog_status()
            .await
            .unwrap()
            .iter()
            .find(|s| s.alias == "a")
            .unwrap()
            .writable
    );
    assert!(other
        .try_lock_recording("bilibili", "123", "shared")
        .is_err());
    assert!(other.selected_store(Some("a")).await.is_err());
    assert_eq!(
        other
            .recording("bilibili", "123", "shared")
            .await
            .unwrap()
            .read("video.ts")
            .await
            .unwrap(),
        b"recording bytes"
    );
    close(&other_db).await;
    close(&db).await;
}

#[tokio::test]
#[ignore = "requires RustFS and explicit S3_TEST_* test credentials"]
async fn rustfs_production_catalog_transfers_all_directions_and_remounts() {
    use futures::TryStreamExt;
    use recording_catalog::object_store::{
        aws::AmazonS3Builder, prefix::PrefixStore, ObjectStore, ObjectStoreExt,
    };
    let (_root, db, storage, _) = fixture().await;
    let prefix = format!("catalog-runtime-test-{}/中文 %测试", Uuid::new_v4());
    eprintln!("RustFS production catalog prefix: {prefix}");
    let credentials = recording_storage::S3Credentials {
        access_key_id: std::env::var("S3_TEST_ACCESS_KEY_ID").expect("set S3_TEST_ACCESS_KEY_ID"),
        secret_access_key: std::env::var("S3_TEST_SECRET_ACCESS_KEY")
            .expect("set S3_TEST_SECRET_ACCESS_KEY"),
        session_token: String::new(),
    };
    let s3 = recording_storage::S3Config {
        endpoint: std::env::var("S3_TEST_ENDPOINT").expect("set S3_TEST_ENDPOINT"),
        bucket: std::env::var("S3_TEST_BUCKET").expect("set S3_TEST_BUCKET"),
        region: "us-east-1".into(),
        prefix: prefix.clone(),
        path_style: true,
    };
    let mut config = storage.configuration().await.unwrap();
    for alias in ["cloud1", "cloud2"] {
        config.backends.insert(
            alias.into(),
            RecordingBackendConfig {
                kind: "s3".into(),
                root: String::new(),
                s3: Some(recording_storage::S3Config {
                    prefix: format!("{prefix}/{alias}"),
                    ..s3.clone()
                }),
            },
        );
    }
    storage
        .replace_configuration_with_credentials(
            config.clone(),
            &storage.configuration().await.unwrap(),
            &HashMap::from([
                ("cloud1".into(), credentials.clone()),
                ("cloud2".into(), credentials.clone()),
            ]),
        )
        .await
        .unwrap();
    storage.start_catalogs(false).await.unwrap();
    assert!(
        db.catalog_status()
            .await
            .unwrap()
            .iter()
            .all(|status| status.ready),
        "{:?}",
        db.catalog_status().await.unwrap()
    );
    let media_backend = recording_storage::S3Backend::new(
        config.backends["cloud1"].s3.as_ref().unwrap(),
        &credentials,
    )
    .unwrap();
    assert!(
        !recording_storage::StorageBackend::read(&media_backend, ".bsr/storage.json")
            .await
            .unwrap()
            .is_empty(),
        "media and catalogs must use exactly the same literal S3 prefix"
    );
    create(&db, &storage, "round-trip", "a").await;
    db.start_record_summary("bilibili", "123", "round-trip", "test", false)
        .await
        .unwrap();
    db.complete_record_summary(
        "bilibili",
        "123",
        "round-trip",
        "Preserved",
        "[]",
        "provider",
        "model",
    )
    .await
    .unwrap();
    let mut from = "a";
    for to in ["cloud1", "cloud2", "b", "cloud2", "cloud1", "a"] {
        assert_eq!(
            transfer(&db, &storage, from, to).await,
            None,
            "{from} -> {to}"
        );
        storage.refresh_catalogs(true).await.unwrap();
        assert_eq!(db.get_recent_record("", 0, 10).await.unwrap().len(), 1);
        assert_eq!(
            db.get_record("123", "round-trip")
                .await
                .unwrap()
                .storage_backend_id,
            to
        );
        assert_eq!(
            db.get_record_summary("bilibili", "123", "round-trip")
                .await
                .unwrap()
                .unwrap()
                .summary_markdown
                .as_deref(),
            Some("Preserved")
        );
        assert_eq!(
            storage
                .recording("bilibili", "123", "round-trip")
                .await
                .unwrap()
                .read("video.ts")
                .await
                .unwrap(),
            b"recording bytes"
        );
        if to == "cloud1" {
            let mut next = storage.configuration().await.unwrap();
            next.backends.remove("cloud1");
            storage.replace_configuration(next).await.unwrap();
            assert!(db.get_recent_record("", 0, 10).await.unwrap().is_empty());
            storage
                .replace_configuration_with_credentials(
                    config.clone(),
                    &storage.configuration().await.unwrap(),
                    &HashMap::from([("cloud1".into(), credentials.clone())]),
                )
                .await
                .unwrap();
            assert_eq!(db.get_recent_record("", 0, 10).await.unwrap().len(), 1);
        }
        from = to;
    }
    close(&db).await;
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
        recording_catalog::object_store::path::Path::parse(prefix).unwrap(),
    );
    for object in store.list(None).try_collect::<Vec<_>>().await.unwrap() {
        store.delete(&object.location).await.unwrap();
    }
}
