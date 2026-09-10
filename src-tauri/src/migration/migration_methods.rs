use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use base64::Engine;

use crate::database::Database;
use crate::recorder_manager::RecorderManagerError;
use recorder::entry::EntryStore;
use recorder::platforms::PlatformType;

pub async fn try_rebuild_archives(
    db: &Arc<Database>,
    cache_path: PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    let rooms = db.get_recorders().await?;
    for room in rooms {
        let room_id = room.room_id;
        let room_cache_path = cache_path.join(format!("{}/{}", room.platform, room_id));
        let mut files = tokio::fs::read_dir(room_cache_path).await?;
        while let Some(file) = files.next_entry().await? {
            if file.file_type().await?.is_dir() {
                // use folder name as live_id
                let file_name = file.file_name();
                let Some(live_id) = file_name.to_str() else {
                    log::warn!("Skipping archive with non-UTF-8 name: {:?}", file.path());
                    continue;
                };
                let record_path = file.path();
                let entry_store = EntryStore::new(record_path.to_string_lossy().as_ref()).await;
                let existing_record = db.get_record(&room_id, live_id).await;

                // Empty folders are left behind when recording fails before the first segment.
                // They are not archives and must not be restored as 0-byte records.
                if entry_store.is_empty() {
                    match existing_record {
                        Ok(record) if record.size == 0 => {
                            db.remove_record(live_id).await?;
                            tokio::fs::remove_dir_all(&record_path).await?;
                            log::info!("removed empty archive folder: {}", record_path.display());
                        }
                        Ok(_) => {
                            log::warn!(
                                "archive {} has database data but no cached entries; preserving it",
                                record_path.display()
                            );
                        }
                        Err(_) => {
                            tokio::fs::remove_dir_all(&record_path).await?;
                            log::info!("removed empty archive folder: {}", record_path.display());
                        }
                    }
                    continue;
                }

                // check if live_id is in db
                if let Ok(record) = existing_record {
                    if record.size == 0 {
                        db.update_record_delta(
                            live_id,
                            entry_store.total_duration(),
                            entry_store.total_size(),
                        )
                        .await?;
                    }
                    continue;
                }

                // create a record for this live_id
                let record = db
                    .add_record(
                        PlatformType::from_str(room.platform.as_str()).map_err(|_| {
                            RecorderManagerError::InvalidPlatformType {
                                platform: room.platform.to_string(),
                            }
                        })?,
                        live_id,
                        live_id,
                        &room_id,
                        &format!("UnknownLive {live_id}"),
                        None,
                    )
                    .await?;
                db.update_record_delta(
                    live_id,
                    entry_store.total_duration(),
                    entry_store.total_size(),
                )
                .await?;

                log::info!("rebuild archive {record:?}");
            }
        }
    }
    Ok(())
}

pub async fn try_convert_live_covers(
    db: &Arc<Database>,
    cache_path: PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    let rooms = db.get_recorders().await?;
    for room in rooms {
        let room_id = room.room_id;
        let room_cache_path = cache_path.join(format!("{}/{}", room.platform, room_id));
        let records = db.get_records(&room_id, 0, 999_999_999).await?;
        for record in &records {
            let record_path = room_cache_path.join(record.live_id.clone());
            let Some(cover) = record.cover.clone() else {
                continue;
            };
            if cover.starts_with("data:") {
                let Some(base64) = cover.split("base64,").nth(1) else {
                    log::warn!("Skipping cover with invalid data URL: {}", record.live_id);
                    continue;
                };
                let bytes = match base64::engine::general_purpose::STANDARD.decode(base64) {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        log::warn!("Skipping invalid cover for {}: {e}", record.live_id);
                        continue;
                    }
                };
                let path = record_path.join("cover.jpg");
                tokio::fs::write(&path, bytes).await?;

                log::info!("convert live cover: {}", path.display());
                // update record
                db.update_record_cover(
                    record.live_id.as_str(),
                    Some(format!(
                        "{}/{}/{}/cover.jpg",
                        room.platform, room_id, record.live_id
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
            let Some(base64) = cover.split("base64,").nth(1) else {
                log::warn!("Skipping cover with invalid data URL: {}", video.file);
                continue;
            };
            let bytes = match base64::engine::general_purpose::STANDARD.decode(base64) {
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
    cache_path: PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    let rooms = db.get_recorders().await?;
    for room in &rooms {
        let records = db.get_records(&room.room_id, 0, 999_999_999).await?;
        for record in &records {
            let record_path = cache_path.join(format!(
                "{}/{}/{}",
                room.platform, room.room_id, record.live_id
            ));
            let entry_file = record_path.join("entries.log");
            let m3u8_file_path = record_path.join("playlist.m3u8");
            if !entry_file.exists() || m3u8_file_path.exists() {
                continue;
            }
            let entry_store = EntryStore::new(record_path.to_string_lossy().as_ref()).await;
            if entry_store.is_empty() {
                continue;
            }
            let m3u8_content = entry_store.manifest(true, true, None);

            tokio::fs::write(&m3u8_file_path, m3u8_content).await?;
            log::info!(
                "Convert entry to m3u8: {} => {}",
                entry_file.display(),
                m3u8_file_path.display()
            );
        }
    }

    Ok(())
}
