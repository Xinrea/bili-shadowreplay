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
                let live_id = file.file_name();
                let live_id = live_id.to_str().unwrap();
                // check if live_id is in db
                let record = db.get_record(&room_id, live_id).await;
                if record.is_ok() {
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
            let cover = record.cover.clone();
            if cover.is_none() {
                continue;
            }

            let cover = cover.unwrap();
            if cover.starts_with("data:") {
                let base64 = cover.split("base64,").nth(1).unwrap();
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(base64)
                    .unwrap();
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
            let base64 = cover.split("base64,").nth(1).unwrap();
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(base64)
                .unwrap();

            let video_file_path = output_path.join(video.file.clone());
            let cover_file_path = video_file_path.with_extension("jpg");
            log::debug!("cover_file_path: {}", cover_file_path.display());
            tokio::fs::write(&cover_file_path, bytes).await?;

            log::info!("convert clip cover: {}", cover_file_path.display());
            // update record
            db.update_video_cover(
                video.id,
                cover_file_path.file_name().unwrap().to_str().unwrap(),
            )
            .await?;
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
            let entry_store = EntryStore::new(record_path.to_str().unwrap()).await;
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
