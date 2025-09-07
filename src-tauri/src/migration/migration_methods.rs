use std::path::PathBuf;
use std::sync::Arc;

use base64::Engine;
use chrono::Utc;

use crate::database::Database;
use crate::recorder::PlatformType;

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
                let record = db.get_record(room_id, live_id).await;
                if record.is_ok() {
                    continue;
                }

                // get created_at from folder metadata
                let metadata = file.metadata().await?;
                let created_at = metadata.created();
                if created_at.is_err() {
                    continue;
                }
                let created_at = created_at.unwrap();
                let created_at = chrono::DateTime::<Utc>::from(created_at)
                    .format("%Y-%m-%dT%H:%M:%S.%fZ")
                    .to_string();
                // create a record for this live_id
                let record = db
                    .add_record(
                        PlatformType::from_str(room.platform.as_str()).unwrap(),
                        live_id,
                        room_id,
                        &format!("UnknownLive {}", live_id),
                        None,
                        Some(&created_at),
                    )
                    .await?;

                log::info!("rebuild archive {:?}", record);
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
        let records = db.get_records(room_id, 0, 999999999).await?;
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
                cover_file_path
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_string(),
            )
            .await?;
        }
    }
    Ok(())
}
