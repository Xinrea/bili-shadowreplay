use std::path::PathBuf;
use std::sync::Arc;

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
