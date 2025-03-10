use crate::database::video::VideoRow;
use crate::recorder::bilibili::profile::Profile;
use crate::state::State;
use chrono::Utc;
use std::path::Path;
use tauri::State as TauriState;
use tauri_plugin_notification::NotificationExt;

#[tauri::command]
pub async fn clip_range(
    state: TauriState<'_, State>,
    cover: String,
    room_id: u64,
    ts: u64,
    x: f64,
    y: f64,
) -> Result<VideoRow, String> {
    log::info!(
        "Clip room_id: {}, ts: {}, start: {}, end: {}",
        room_id,
        ts,
        x,
        y
    );
    let file = state
        .recorder_manager
        .clip_range(&state.config.read().await.output, room_id, ts, x, y)
        .await?;
    // get file metadata from fs
    let metadata = std::fs::metadata(&file).map_err(|e| e.to_string())?;
    // get filename from path
    let filename = Path::new(&file)
        .file_name()
        .ok_or("Invalid file path")?
        .to_str()
        .ok_or("Invalid file path")?;
    // add video to db
    let video = state
        .db
        .add_video(&VideoRow {
            id: 0,
            status: 0,
            room_id,
            created_at: Utc::now().to_rfc3339(),
            cover: cover.clone(),
            file: filename.into(),
            length: (y - x) as i64,
            size: metadata.len() as i64,
            bvid: "".into(),
            title: "".into(),
            desc: "".into(),
            tags: "".into(),
            area: 0,
        })
        .await?;
    state
        .db
        .new_message(
            "生成新切片",
            &format!(
                "生成了房间 {} 的切片，长度 {:.1}s：{}",
                room_id,
                y - x,
                filename
            ),
        )
        .await?;
    if state.config.read().await.clip_notify {
        state
            .app_handle
            .notification()
            .builder()
            .title("BiliShadowReplay - 切片完成")
            .body(format!("生成了房间 {} 的切片: {}", room_id, filename))
            .show()
            .unwrap();
    }
    Ok(video)
}

#[tauri::command]
pub async fn upload_procedure(
    state: TauriState<'_, State>,
    uid: u64,
    room_id: u64,
    video_id: i64,
    cover: String,
    mut profile: Profile,
) -> Result<String, String> {
    let account = state.db.get_account(uid).await?;
    // get video info from dbs
    let mut video_row = state.db.get_video(video_id).await?;
    // construct file path
    let output = state.config.read().await.output.clone();
    let file = format!("{}/{}", output, video_row.file);
    let path = Path::new(&file);
    let cover_url = state.client.upload_cover(&account, &cover);
    if let Ok(video) = state.client.prepare_video(&account, path).await {
        profile.cover = cover_url.await.unwrap_or("".to_string());
        if let Ok(ret) = state.client.submit_video(&account, &profile, &video).await {
            // update video status and details
            // 1 means uploaded
            video_row.status = 1;
            video_row.bvid = ret.bvid.clone();
            video_row.title = profile.title;
            video_row.desc = profile.desc;
            video_row.tags = profile.tag;
            video_row.area = profile.tid as i64;
            state.db.update_video(&video_row).await?;
            state
                .db
                .new_message(
                    "投稿成功",
                    &format!("投稿了房间 {} 的切片：{}", room_id, ret.bvid),
                )
                .await?;
            if state.config.read().await.post_notify {
                state
                    .app_handle
                    .notification()
                    .builder()
                    .title("BiliShadowReplay - 投稿成功")
                    .body(format!("投稿了房间 {} 的切片: {}", room_id, ret.bvid))
                    .show()
                    .unwrap();
            }
            Ok(ret.bvid)
        } else {
            Err("Submit video failed".to_string())
        }
    } else {
        Err("Preload video failed".to_string())
    }
}

#[tauri::command]
pub async fn get_video(state: TauriState<'_, State>, id: i64) -> Result<VideoRow, String> {
    Ok(state.db.get_video(id).await?)
}

#[tauri::command]
pub async fn get_videos(state: TauriState<'_, State>, room_id: u64) -> Result<Vec<VideoRow>, String> {
    Ok(state.db.get_videos(room_id).await?)
}

#[tauri::command]
pub async fn delete_video(state: TauriState<'_, State>, id: i64) -> Result<(), String> {
    // get video info from dbus
    let video = state.db.get_video(id).await?;
    // delete video files
    let filepath = format!("{}/{}", state.config.read().await.output, video.file);
    let file = Path::new(&filepath);
    if let Err(e) = std::fs::remove_file(file) {
        log::error!("Delete video file error: {}", e);
    }
    Ok(state.db.delete_video(id).await?)
}

#[tauri::command]
pub async fn get_video_typelist(
    state: TauriState<'_, State>,
) -> Result<Vec<crate::recorder::bilibili::response::Typelist>, String> {
    let account = state
        .db
        .get_account(state.config.read().await.primary_uid)
        .await?;
    Ok(state.client.get_video_typelist(&account).await?)
} 