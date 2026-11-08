use std::collections::BTreeSet;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use crate::database::{Database, DatabaseError};
use crate::handlers::utils::decode_data_url;
use crate::recorder_manager::RecorderManager;
use recorder::entry::EntryStore;
use recorder::platforms::PlatformType;
use recording_storage::RecordingStorage;

async fn recording_ids(storage: &RecordingStorage) -> std::io::Result<BTreeSet<String>> {
    let mut live_ids = BTreeSet::new();
    let mut cursor = None;
    loop {
        let page = storage.list("", cursor.as_deref(), 1000).await?;
        for key in page.keys {
            if let Some((live, _)) = key.split_once('/') {
                if !live.starts_with('.') {
                    live_ids.insert(live.to_string());
                }
            }
        }
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => return Ok(live_ids),
        }
    }
}

/// An unreadable or malformed entry log is not evidence that an archive is
/// empty. Only remove scopes containing no nonempty objects at all; retain
/// media, covers and partial/corrupt logs for recovery.
async fn confirmed_empty(storage: &RecordingStorage) -> std::io::Result<bool> {
    let mut cursor = None;
    loop {
        let page = storage.list("", cursor.as_deref(), 1000).await?;
        for key in page.keys {
            if storage.size(&key).await? != 0 {
                return Ok(false);
            }
        }
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => return Ok(true),
        }
    }
}

pub async fn try_rebuild_archives(
    db: &Arc<Database>,
    manager: &Arc<RecorderManager>,
) -> Result<(), Box<dyn std::error::Error>> {
    manager.archive_storage().initialize().await?;
    let rooms = db.get_recorders().await?;
    for room in rooms {
        let room_id = room.room_id;
        let platform = PlatformType::from_str(&room.platform)?;
        let mut live_ids = BTreeSet::new();
        for scope in manager
            .archive_storage()
            .room_stores(&room.platform, &room_id)
            .await?
        {
            live_ids.extend(recording_ids(&scope).await?);
        }
        for live_id in live_ids {
            let _mutation =
                manager
                    .archive_storage()
                    .try_lock_recording(&room.platform, &room_id, &live_id)?;
            let storage = manager
                .recording_storage(platform, &room_id, &live_id)
                .await?;
            let entry_store = EntryStore::from_storage(&storage).await?;
            let existing_record = match db.get_record(&room_id, &live_id).await {
                Ok(record) => Some(record),
                Err(DatabaseError::NotFound | DatabaseError::DB(sqlx::Error::RowNotFound)) => None,
                Err(error) => return Err(error.into()),
            };

            if entry_store.is_empty() {
                if existing_record
                    .as_ref()
                    .is_some_and(|record| record.size != 0)
                {
                    log::warn!("archive {}/{room_id}/{live_id} has database data but no entries; preserving it", room.platform);
                } else if confirmed_empty(&storage).await? {
                    storage.remove_all().await?;
                    if existing_record.is_some() {
                        db.remove_record(&live_id).await?;
                    }
                    log::info!(
                        "removed confirmed-empty archive {}/{room_id}/{live_id}",
                        room.platform
                    );
                }
                continue;
            }

            if let Some(record) = existing_record {
                if record.size == 0 {
                    db.update_record_delta(
                        &live_id,
                        entry_store.total_duration(),
                        entry_store.total_size(),
                    )
                    .await?;
                }
                continue;
            }

            let record = db
                .add_record(
                    platform,
                    &live_id,
                    &live_id,
                    &room_id,
                    &format!("UnknownLive {live_id}"),
                    None,
                )
                .await?;
            db.update_record_delta(
                &live_id,
                entry_store.total_duration(),
                entry_store.total_size(),
            )
            .await?;

            log::info!("rebuild archive {record:?}");
        }
    }
    Ok(())
}

pub async fn try_convert_live_covers(
    db: &Arc<Database>,
    manager: &Arc<RecorderManager>,
) -> Result<(), Box<dyn std::error::Error>> {
    manager.archive_storage().initialize().await?;
    let rooms = db.get_recorders().await?;
    for room in rooms {
        let room_id = room.room_id;
        let records = db.get_records(&room_id, 0, 999_999_999).await?;
        for record in &records {
            let Some(cover) = record.cover.clone() else {
                continue;
            };
            if cover.starts_with("data:") {
                let _mutation = manager.archive_storage().try_lock_recording(
                    &record.platform,
                    &record.room_id,
                    &record.live_id,
                )?;
                let bytes = match decode_data_url(&cover) {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        log::warn!("Skipping invalid cover for {}: {e}", record.live_id);
                        continue;
                    }
                };
                // A recorder can replace another platform's room with the same
                // ID; historical archives retain their own platform identity.
                let platform = PlatformType::from_str(&record.platform)?;
                let storage = manager
                    .recording_storage(platform, &record.room_id, &record.live_id)
                    .await?;
                storage.write("cover.jpg", &bytes).await?;

                log::info!(
                    "convert live cover: {}/{}/{}",
                    record.platform,
                    record.room_id,
                    record.live_id
                );
                // update record
                db.update_record_cover(
                    record.live_id.as_str(),
                    Some(format!(
                        "{}/{}/{}/cover.jpg",
                        record.platform, record.room_id, record.live_id
                    )),
                )
                .await?;
            }
        }
    }
    Ok(())
}

pub async fn try_convert_clip_covers(
    db: &Arc<Database>,
    output_path: PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    let videos = db.get_all_videos().await?;
    log::debug!("videos: {}", videos.len());
    for video in &videos {
        let cover = video.cover.clone();
        if cover.starts_with("data:") {
            let bytes = match decode_data_url(&cover) {
                Ok(bytes) => bytes,
                Err(e) => {
                    log::warn!("Skipping invalid cover for {}: {e}", video.file);
                    continue;
                }
            };

            let video_file_path = output_path.join(video.file.clone());
            let cover_file_path = video_file_path.with_extension("jpg");
            log::debug!("cover_file_path: {}", cover_file_path.display());
            tokio::fs::write(&cover_file_path, bytes).await?;

            log::info!("convert clip cover: {}", cover_file_path.display());
            let cover_name = cover_file_path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| format!("Invalid cover path: {}", cover_file_path.display()))?;
            // update record
            db.update_video_cover(video.id, cover_name).await?;
        }
    }
    Ok(())
}

pub async fn try_add_parent_id_to_records(
    db: &Arc<Database>,
) -> Result<(), Box<dyn std::error::Error>> {
    let rooms = db.get_recorders().await?;
    for room in &rooms {
        let records = db.get_records(&room.room_id, 0, 999_999_999).await?;
        for record in &records {
            if record.parent_id.is_empty() {
                db.update_record_parent_id(record.live_id.as_str(), record.live_id.as_str())
                    .await?;
            }
        }
    }
    Ok(())
}

pub async fn try_convert_entry_to_m3u8(
    db: &Arc<Database>,
    manager: &Arc<RecorderManager>,
) -> Result<(), Box<dyn std::error::Error>> {
    manager.archive_storage().initialize().await?;
    let rooms = db.get_recorders().await?;
    for room in &rooms {
        let records = db.get_records(&room.room_id, 0, 999_999_999).await?;
        for record in &records {
            let platform = PlatformType::from_str(&record.platform)?;
            let _mutation = manager.archive_storage().try_lock_recording(
                &record.platform,
                &record.room_id,
                &record.live_id,
            )?;
            let storage = manager
                .recording_storage(platform, &record.room_id, &record.live_id)
                .await?;
            if !storage.log_exists("entries.log").await? || storage.exists("playlist.m3u8").await? {
                continue;
            }
            let entry_store = EntryStore::from_storage(&storage).await?;
            if entry_store.is_empty() {
                continue;
            }
            let m3u8_content = entry_store.manifest(true, true, None);

            storage
                .write("playlist.m3u8", m3u8_content.as_bytes())
                .await?;
            log::info!(
                "Convert entry to m3u8: {}/{}/{}",
                record.platform,
                record.room_id,
                record.live_id
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "headless")]
    #[tokio::test]
    async fn conversions_keep_archive_platform_when_a_room_recorder_is_replaced() {
        use crate::config::Config;
        use crate::database::credentials::CredentialCipher;
        use crate::events::EventBus;
        use crate::task::TaskManager;
        use crate::webhook::poster::{WebhookConfig, WebhookPoster};
        use tokio::sync::RwLock;

        let directory = tempfile::tempdir().unwrap();
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate::Migrator::new(crate::MigrationList(crate::get_migrations()))
            .await
            .unwrap()
            .run(&pool)
            .await
            .unwrap();
        let events = EventBus::new();
        let db = Arc::new(Database::new(
            pool,
            CredentialCipher::new(&[0; 32]).unwrap(),
            events.clone(),
        ));
        let mut config = Config::load(
            &directory.path().join("config.toml"),
            directory.path(),
            directory.path(),
        )
        .unwrap();
        config.storage = crate::config::StorageConfig::local(
            directory
                .path()
                .join("cache")
                .to_string_lossy()
                .into_owned(),
        );
        let manager = Arc::new(RecorderManager::new(
            events,
            db.clone(),
            Arc::new(RwLock::new(config)),
            Arc::new(TaskManager::new()),
            directory.path().into(),
            WebhookPoster::new(WebhookConfig::default()).unwrap(),
        ));
        let room = "shared-room";
        db.add_recorder(PlatformType::BiliBili, room, "{}")
            .await
            .unwrap();
        db.add_recorder(PlatformType::Douyin, room, "{}")
            .await
            .unwrap();
        for (platform, live) in [
            (PlatformType::BiliBili, "historical"),
            (PlatformType::Douyin, "current"),
        ] {
            db.add_record(
                platform,
                live,
                live,
                room,
                "title",
                Some("data:image/jpeg;base64,Y292ZXI=".into()),
            )
            .await
            .unwrap();
            let storage = manager
                .recording_storage(platform, room, live)
                .await
                .unwrap();
            storage
                .write("entries.log", b"segment.ts|1|2.0|4|1700000000|false\n")
                .await
                .unwrap();
        }

        try_convert_live_covers(&db, &manager).await.unwrap();
        try_convert_entry_to_m3u8(&db, &manager).await.unwrap();
        for (platform, live) in [
            (PlatformType::BiliBili, "historical"),
            (PlatformType::Douyin, "current"),
        ] {
            let storage = manager
                .recording_storage(platform, room, live)
                .await
                .unwrap();
            assert_eq!(storage.read("cover.jpg").await.unwrap(), b"cover");
            assert!(storage.exists("playlist.m3u8").await.unwrap());
            assert_eq!(
                db.get_record(room, live).await.unwrap().cover,
                Some(format!("{}/{room}/{live}/cover.jpg", platform.as_str())),
            );
        }
        let incorrect =
            RecordingStorage::local(directory.path().join("cache/douyin/shared-room/historical"));
        assert!(!incorrect.exists("cover.jpg").await.unwrap());
        assert!(!incorrect.exists("playlist.m3u8").await.unwrap());
    }

    #[tokio::test]
    async fn discovery_uses_object_prefixes_not_workspace_directories() {
        let directory = tempfile::tempdir().unwrap();
        let room = RecordingStorage::local(directory.path());
        room.write("cover.jpg", b"room cover").await.unwrap();
        room.write("first/playlist.m3u8", b"playlist")
            .await
            .unwrap();
        room.write("first/media/segment.ts", b"media")
            .await
            .unwrap();
        room.write("second/entries.log", b"").await.unwrap();
        // Copy fallback stages can live beside archives, including across
        // restart. Neither empty nor populated private stages are recordings.
        room.write(".recording-transfer-stage-empty/archive/entries.log", b"")
            .await
            .unwrap();
        room.write(
            ".recording-transfer-stage-data/archive/media.ts",
            b"staged media",
        )
        .await
        .unwrap();
        assert_eq!(
            recording_ids(&room).await.unwrap(),
            BTreeSet::from(["first".to_string(), "second".to_string()])
        );
    }

    #[tokio::test]
    async fn malformed_logs_or_orphan_media_are_not_confirmed_empty() {
        let directory = tempfile::tempdir().unwrap();
        let storage = RecordingStorage::local(directory.path());
        storage.write("entries.log", b"").await.unwrap();
        assert!(confirmed_empty(&storage).await.unwrap());
        storage
            .write("entries.log", b"damaged legacy log")
            .await
            .unwrap();
        assert!(EntryStore::from_storage(&storage).await.unwrap().is_empty());
        assert!(!confirmed_empty(&storage).await.unwrap());
        storage.write("entries.log", b"").await.unwrap();
        storage
            .write("orphan.ts", b"media without an entry")
            .await
            .unwrap();
        assert!(!confirmed_empty(&storage).await.unwrap());
    }
}
