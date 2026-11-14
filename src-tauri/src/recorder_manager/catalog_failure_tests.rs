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
async fn failed_recording_cleanup_releases_map_before_waiting_for_removal_marker() {
    let directory = tempfile::tempdir().unwrap();
    let (manager, rx) = manager(directory.path()).await;
    let consumer = manager.clone();
    let events = tokio::spawn(async move { consumer.handle_events(rx).await });

    // Let the actual abort path drain its events, but park its final map write.
    let map_reader = manager.recorders.read().await;
    manager
        .abort_failed_recording(PlatformType::BiliBili, "room", "failed")
        .await;
    // This is the guard insert_recorder retains while waiting to insert.
    let removing = manager.to_remove.read().await;
    assert!(removing.contains("bilibili:room"));
    tokio::time::timeout(Duration::from_secs(5), async {
        // Tokio's writer preference makes try_read fail once cleanup is queued.
        while manager.recorders.try_read().is_ok() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    drop(map_reader);

    // Cleanup gets the map first. It must release it even while our insertion
    // guard prevents removal-marker cleanup; otherwise this is a lock cycle.
    let map_writer = tokio::time::timeout(Duration::from_secs(5), manager.recorders.write())
        .await
        .expect("failed-recording cleanup must not deadlock recorder insertion");
    drop(map_writer);
    drop(removing);
    let markers = tokio::time::timeout(Duration::from_secs(5), manager.to_remove.read())
        .await
        .unwrap();
    assert!(!markers.contains("bilibili:room"));
    events.abort();
    let _ = events.await;
}

async fn zero_size_record(manager: &RecorderManager, live: &str) -> RecordingStorage {
    let storage = manager
        .archive_storage
        .selected_store(Some("default"))
        .await
        .unwrap()
        .recording("bilibili", "room", live)
        .unwrap();
    storage.prepare().await.unwrap();
    manager
        .db
        .add_record(
            PlatformType::BiliBili,
            "parent",
            live,
            "room",
            "Title",
            None,
        )
        .await
        .unwrap();
    storage
}

async fn end_recording(manager: &RecorderManager, live: &str) {
    manager
        .event_tx
        .send(RecorderEvent::RecordEnd {
            recorder: RecorderInfo {
                room_info: RoomInfo {
                    platform: "bilibili".into(),
                    room_id: "room".into(),
                    ..Default::default()
                },
                user_info: UserInfo::default(),
                platform_live_id: "parent".into(),
                live_id: live.into(),
                recording: false,
                enabled: true,
                storage_id: Some("default".into()),
            },
        })
        .await
        .unwrap();
    tokio::time::timeout(
        Duration::from_secs(5),
        manager.event_tx.drain_recording(live),
    )
    .await
    .unwrap()
    .unwrap();
}

#[tokio::test]
async fn record_end_only_deletes_confirmed_empty_archives() {
    let directory = tempfile::tempdir().unwrap();
    let (manager, rx) = manager(directory.path()).await;
    let consumer = manager.clone();
    let events = tokio::spawn(async move { consumer.handle_events(rx).await });
    for (live, key, bytes) in [
        ("empty", "events.jsonl", b"".as_slice()),
        ("orphan", "segment.ts", b"unreported media".as_slice()),
        ("events", "events.jsonl", b"recoverable events".as_slice()),
        ("legacy", "entries.log", b"damaged legacy log".as_slice()),
        (
            "unknown",
            "future-format.bin",
            b"preserve unknown assets".as_slice(),
        ),
        ("corrupt", "playlist.m3u8", b"invalid playlist".as_slice()),
        (
            "referenced",
            "playlist.m3u8",
            b"#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXTINF:4,\nmissing.ts\n".as_slice(),
        ),
    ] {
        let storage = zero_size_record(&manager, live).await;
        storage
            .write("playlist.m3u8", b"#EXTM3U\n#EXT-X-TARGETDURATION:4\n")
            .await
            .unwrap();
        storage.write("cover.jpg", b"setup cover").await.unwrap();
        storage.write(".sequence", b"0").await.unwrap();
        storage.write(key, bytes).await.unwrap();
        end_recording(&manager, live).await;
        assert!(!manager
            .db
            .catalog_recording_active("default")
            .await
            .unwrap());
        if live == "empty" {
            assert!(manager.db.get_record("room", live).await.is_err());
            assert!(storage.list("", None, 10).await.unwrap().keys.is_empty());
        } else {
            assert_eq!(manager.db.get_record("room", live).await.unwrap().size, 0);
            assert_eq!(storage.read(key).await.unwrap(), bytes);
            // Only automatic cleanup is conservative; explicit deletion works.
            manager
                .delete_archive(PlatformType::BiliBili, "room", live)
                .await
                .unwrap();
            assert!(storage.list("", None, 10).await.unwrap().keys.is_empty());
        }
    }
    events.abort();
    let _ = events.await;
}

struct FailingBackend {
    local: recording_storage::LocalBackend,
    media_stat: bool,
    listing: bool,
    playlist_read: bool,
}

#[async_trait::async_trait]
impl recording_storage::StorageBackend for FailingBackend {
    async fn read_range(&self, key: &str, range: std::ops::Range<u64>) -> std::io::Result<Vec<u8>> {
        self.local.read_range(key, range).await
    }

    async fn list(
        &self,
        prefix: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> std::io::Result<recording_storage::ObjectPage> {
        if self.listing {
            return Err(std::io::ErrorKind::TimedOut.into());
        }
        self.local.list(prefix, cursor, limit).await
    }

    async fn read(&self, key: &str) -> std::io::Result<Vec<u8>> {
        self.local.read(key).await
    }

    async fn read_bounded(&self, key: &str, max: usize) -> std::io::Result<Vec<u8>> {
        if self.playlist_read && key == "playlist.m3u8" {
            return Err(std::io::ErrorKind::TimedOut.into());
        }
        self.local.read_bounded(key, max).await
    }

    async fn write(&self, key: &str, bytes: &[u8]) -> std::io::Result<()> {
        self.local.write(key, bytes).await
    }

    async fn remove(&self, key: &str) -> std::io::Result<()> {
        self.local.remove(key).await
    }

    async fn stat(&self, key: &str) -> std::io::Result<recording_storage::ObjectMetadata> {
        if self.media_stat && key.ends_with(".ts") {
            return Err(std::io::ErrorKind::TimedOut.into());
        }
        self.local.stat(key).await
    }
}

#[tokio::test]
async fn empty_recording_probe_propagates_storage_failures() {
    let directory = tempfile::tempdir().unwrap();
    let storage = RecordingStorage::local(directory.path());
    storage
        .write("playlist.m3u8", b"#EXTM3U\n#EXT-X-TARGETDURATION:4\n")
        .await
        .unwrap();
    storage.write("segment.ts", b"media").await.unwrap();
    for (media_stat, listing, playlist_read) in [
        (true, false, false),
        (false, true, false),
        (false, false, true),
    ] {
        let failing = RecordingStorage::with_backend(
            Arc::new(FailingBackend {
                local: recording_storage::LocalBackend::new(directory.path()),
                media_stat,
                listing,
                playlist_read,
            }),
            recording_storage::LocalWorkspace::new(directory.path()),
        );
        assert!(empty_recording::confirmed_empty(&failing).await.is_err());
        assert_eq!(storage.read("segment.ts").await.unwrap(), b"media");
    }
}

#[tokio::test]
async fn record_end_preserves_native_media_when_progress_stat_fails() {
    let directory = tempfile::tempdir().unwrap();
    let (manager, rx) = manager(directory.path()).await;
    let live = "failed-stat";
    let storage = zero_size_record(&manager, live).await;
    let root = directory.path().join("cache/bilibili/room").join(live);
    let failing = RecordingStorage::with_backend(
        Arc::new(FailingBackend {
            local: recording_storage::LocalBackend::new(&root),
            media_stat: true,
            listing: false,
            playlist_read: false,
        }),
        recording_storage::LocalWorkspace::new(&root),
    );
    let recorder = recorder::core::flv_recorder::FlvRecorder::new(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("crates/ffmpeg_utils/src/hls/fixtures/live.flv")
            .to_string_lossy()
            .into_owned(),
        None,
        Vec::new(),
        failing,
        Arc::new(AtomicBool::new(true)),
        manager.event_tx.clone(),
        live.into(),
    );
    assert!(matches!(
        recorder.start().await,
        Err(RecorderError::IoError(_))
    ));
    let bytes = storage.read("playlist.m3u8").await.unwrap();
    let (_, playlist) = m3u8_rs::parse_media_playlist(&bytes).unwrap();
    assert!(!playlist.segments.is_empty());
    let consumer = manager.clone();
    let events = tokio::spawn(async move { consumer.handle_events(rx).await });
    end_recording(&manager, live).await;
    let record = manager.db.get_record("room", live).await.unwrap();
    assert_eq!(record.size, 0, "failed stat did not report media progress");
    assert!(record.storage_available);
    assert!(!manager
        .db
        .catalog_recording_active("default")
        .await
        .unwrap());
    assert_eq!(storage.read("playlist.m3u8").await.unwrap(), bytes);
    for segment in playlist.segments {
        assert!(!storage.read(&segment.uri).await.unwrap().is_empty());
    }
    manager
        .archive_storage
        .refresh_catalogs(true)
        .await
        .unwrap();
    assert!(manager.db.get_record("room", live).await.is_ok());
    events.abort();
    let _ = events.await;
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
