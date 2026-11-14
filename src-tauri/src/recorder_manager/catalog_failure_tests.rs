use super::*;
use recorder::traits::RecorderBasicTrait;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

async fn manager(dir: &Path) -> (RecorderManager, RecorderEventReceiver) {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    for migration in crate::get_migrations() {
        sqlx::raw_sql(migration.sql).execute(&pool).await.unwrap();
    }
    let events = EventBus::new();
    let db = Arc::new(Database::new(
        pool,
        crate::database::credentials::CredentialCipher::new(&[0; 32]).unwrap(),
        events.clone(),
    ));
    let mut config = Config::load(
        &dir.join("config.toml"),
        &dir.join("cache"),
        &dir.join("output"),
    )
    .unwrap();
    config.storage.backends.insert(
        "other".into(),
        crate::config::RecordingBackendConfig {
            kind: "local".into(),
            root: dir.join("other").to_string_lossy().into_owned(),
            s3: None,
            webdav: None,
        },
    );
    let config = Arc::new(RwLock::new(config));
    let archive_storage = Arc::new(ArchiveStorage::new(db.clone(), config.clone()));
    archive_storage.start_catalogs(false).await.unwrap();
    let (event_tx, event_rx) = recorder_event_channel(1);
    (
        RecorderManager {
            events,
            db,
            config,
            archive_storage,
            event_tx,
            task_manager: Arc::new(TaskManager::new()),
            resource_dir: dir.into(),
            recorders: Arc::new(RwLock::new(HashMap::new())),
            to_remove: Arc::new(RwLock::new(HashSet::new())),
            webhook_poster: WebhookPoster::new(crate::webhook::poster::WebhookConfig::default())
                .unwrap(),
            material_hook: Arc::new(std::sync::OnceLock::new()),
            archive_timing: Arc::new(ArchiveTimingCache::default()),
        },
        event_rx,
    )
}

async fn create(manager: &RecorderManager, platform: PlatformType, alias: &str, live: &str) {
    let scope = manager
        .archive_storage
        .selected_store(Some(alias))
        .await
        .unwrap()
        .recording(platform.as_str(), "room", live)
        .unwrap();
    scope.prepare().await.unwrap();
    scope.write("video.ts", b"preserve media").await.unwrap();
    manager
        .db
        .add_record(platform, "parent", live, "room", "Title", None)
        .await
        .unwrap();
    manager.db.update_record_delta(live, 1.0, 14).await.unwrap();
}

struct Stopped(Arc<AtomicBool>);
impl Drop for Stopped {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

#[tokio::test]
async fn failed_catalog_update_retires_aborted_attempts_and_reconnects_without_restart() {
    for next_attempt_queued in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let (manager, rx) = manager(directory.path()).await;
        create(&manager, PlatformType::BiliBili, "default", "failed").await;
        create(&manager, PlatformType::Twitch, "other", "unrelated").await;
        if next_attempt_queued {
            create(&manager, PlatformType::BiliBili, "default", "next").await;
        }
        let live = if next_attempt_queued {
            "next"
        } else {
            "failed"
        };
        let recorder = Arc::new(
            BiliRecorder::new(
                "room",
                &Account::default(),
                manager.archive_storage.recording_writer(),
                manager.event_tx.clone(),
                Arc::new(AtomicU64::new(30)),
                true,
                manager.db.clone(),
            )
            .unwrap(),
        );
        *recorder.live_id().write().await = live.into();
        recorder.is_recording().store(true, Ordering::Relaxed);
        let lease = recorder.recording_storage(live).await.unwrap();
        let stopped = Arc::new(AtomicBool::new(false));
        let flag = stopped.clone();
        let started = Arc::new(tokio::sync::Notify::new());
        let signal = started.clone();
        *recorder.record_task().lock().await = Some(tokio::spawn(async move {
            let _lease = lease;
            let _stopped = Stopped(flag);
            signal.notify_one();
            std::future::pending::<()>().await;
        }));
        started.notified().await;
        manager
            .recorders
            .write()
            .await
            .insert("bilibili:room".into(), recorder.clone());

        // A closed writer rejects metadata writes just like an unavailable
        // object store. The persisted JSON and media remain available to reopen.
        let runtime = manager.db.catalogs.get().unwrap().clone();
        let writer = runtime.handles.lock().await["default"]
            .writer
            .clone()
            .unwrap();
        writer.close().await.unwrap();
        assert!(manager
            .db
            .catalog_recording_active("default")
            .await
            .unwrap());
        manager
            .event_tx
            .send(RecorderEvent::RecordUpdate {
                live_id: "failed".into(),
                duration_secs: 0.5,
                cached_size_bytes: 5,
            })
            .await
            .unwrap();
        manager
            .event_tx
            .send(RecorderEvent::RecordUpdate {
                live_id: "unrelated".into(),
                duration_secs: 2.0,
                cached_size_bytes: 20,
            })
            .await
            .unwrap();
        if next_attempt_queued {
            // Fill the control queue with a later attempt. It fails before the
            // older delta, and must not make the older active marker immortal.
            manager
                .event_tx
                .send(RecorderEvent::RecordStart {
                    recorder: recorder.info().await,
                })
                .await
                .unwrap();
        }
        let consumer = manager.clone();
        let events = tokio::spawn(async move { consumer.handle_events(rx).await });
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if !manager.recorders.read().await.contains_key("bilibili:room")
                    && !manager.to_remove.read().await.contains("bilibili:room")
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(
            stopped.load(Ordering::Acquire),
            "producer must finish before retirement"
        );
        assert!(!manager
            .db
            .catalog_recording_active("default")
            .await
            .unwrap());
        assert!(manager.db.catalog_recording_active("other").await.unwrap());
        assert!(
            !manager
                .db
                .catalog_status()
                .await
                .unwrap()
                .into_iter()
                .find(|status| status.alias == "default")
                .unwrap()
                .ready
        );

        manager
            .archive_storage
            .refresh_catalogs(true)
            .await
            .unwrap();
        assert!(
            manager
                .db
                .catalog_status()
                .await
                .unwrap()
                .into_iter()
                .find(|status| status.alias == "default")
                .unwrap()
                .ready
        );
        create(&manager, PlatformType::BiliBili, "default", "recovered").await;
        let old = manager
            .recording_storage(PlatformType::BiliBili, "room", "failed")
            .await
            .unwrap();
        assert_eq!(old.read("video.ts").await.unwrap(), b"preserve media");
        // Complete the consumer frontier before checking the unrelated update.
        manager.event_tx.drain_recording("unrelated").await.unwrap();
        assert_eq!(
            manager
                .db
                .get_record("room", "unrelated")
                .await
                .unwrap()
                .size,
            34
        );
        assert!(
            manager
                .db
                .catalog_recording_active("default")
                .await
                .unwrap(),
            "new recordings can start again"
        );
        events.abort();
        let _ = events.await;
        for alias in ["default", "other"] {
            runtime.detach(alias).await.unwrap();
        }
    }
}
