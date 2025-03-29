use crate::database::video::VideoRow;
use crate::ffmpeg;
use crate::progress_event::{emit_progress_finished, emit_progress_update};
use crate::recorder::bilibili::profile::Profile;
use crate::recorder::PlatformType;
use crate::state::State;
use crate::subtitle_generator::whisper::{self};
use crate::subtitle_generator::SubtitleGenerator;
use chrono::Utc;
use std::path::Path;
use tauri::State as TauriState;
use tauri_plugin_notification::NotificationExt;

#[tauri::command]
pub async fn clip_range(
    state: TauriState<'_, State>,
    title: String,
    cover: String,
    platform: String,
    room_id: u64,
    live_id: String,
    x: f64,
    y: f64,
) -> Result<VideoRow, String> {
    log::info!(
        "Clip room_id: {}, ts: {}, start: {}, end: {}",
        room_id,
        live_id,
        x,
        y
    );
    let event_id = format!("clip_{}", room_id);
    let platform = PlatformType::from_str(&platform).unwrap();

    // get format config
    // filter special characters from title to make sure file name is valid
    let title = title
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>();
    let format_config = state.config.read().await.clip_name_format.clone();
    let format_config = format_config.replace("{title}", &title);
    let format_config = format_config.replace("{platform}", platform.as_str());
    let format_config = format_config.replace("{room_id}", &room_id.to_string());
    let format_config = format_config.replace("{live_id}", &live_id);
    let format_config = format_config.replace("{x}", &x.to_string());
    let format_config = format_config.replace("{y}", &y.to_string());
    let format_config = format_config.replace(
        "{created_at}",
        &Utc::now().format("%Y-%m-%d_%H-%M-%S").to_string(),
    );
    let format_config = format_config.replace("{length}", &((y - x) as i64).to_string());

    let output = state.config.read().await.output.clone();
    let clip_file = Path::new(&output).join(&format_config);

    let file = state
        .recorder_manager
        .clip_range(clip_file, platform, room_id, &live_id, x, y)
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
    if state.config.read().await.auto_subtitle
        && !state.config.read().await.whisper_model.is_empty()
    {
        if let Ok(generator) = whisper::new(
            Path::new(&state.config.read().await.whisper_model),
            &state.config.read().await.whisper_prompt,
        )
        .await
        {
            emit_progress_update(&state.app_handle, event_id.as_str(), "提取音频中", "");
            let audio_path = file.with_extension("wav");
            ffmpeg::extract_audio(&file).await?;
            emit_progress_update(&state.app_handle, event_id.as_str(), "生成字幕中", "");
            generator
                .generate_subtitle(&audio_path, &file.with_extension("srt"))
                .await?;
        }
    }
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

    emit_progress_finished(&state.app_handle, event_id.as_str());

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
    let event_id = format!("post_{}", room_id);
    let account = state.db.get_account("bilibili", uid).await?;
    // get video info from dbs
    let mut video_row = state.db.get_video(video_id).await?;
    // construct file path
    let output = state.config.read().await.output.clone();
    let file = format!("{}/{}", output, video_row.file);
    let path = Path::new(&file);
    let cover_url = state.client.upload_cover(&account, &cover);

    let cancel_id = format!("cancel_{}_{}", room_id, video_id);
    if state.cancel_flag_map.read().await.get(&cancel_id).is_some() {
        return Err("已经处于上传状态".to_string());
    }

    let cancel_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

    state
        .cancel_flag_map
        .write()
        .await
        .insert(cancel_id.clone(), cancel_flag.clone());

    emit_progress_update(
        &state.app_handle,
        event_id.as_str(),
        "投稿预处理中",
        &cancel_id,
    );

    match state
        .client
        .prepare_video(
            &state.app_handle,
            &event_id,
            &cancel_id,
            &account,
            path,
            &cancel_flag,
        )
        .await
    {
        Ok(video) => {
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
                emit_progress_finished(&state.app_handle, event_id.as_str());
                state.cancel_flag_map.write().await.remove(&cancel_id);
                Ok(ret.bvid)
            } else {
                emit_progress_finished(&state.app_handle, event_id.as_str());
                state.cancel_flag_map.write().await.remove(&cancel_id);
                Err("Submit video failed".to_string())
            }
        }
        Err(e) => {
            emit_progress_finished(&state.app_handle, event_id.as_str());
            state.cancel_flag_map.write().await.remove(&cancel_id);
            Err(format!("Preload video failed: {}", e))
        }
    }
}

#[tauri::command]
pub async fn cancel_upload(state: TauriState<'_, State>, cancel_id: String) -> Result<(), String> {
    if let Some(cancel_flag) = state.cancel_flag_map.read().await.get(&cancel_id) {
        cancel_flag.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    Ok(())
}

#[tauri::command]
pub async fn get_video(state: TauriState<'_, State>, id: i64) -> Result<VideoRow, String> {
    Ok(state.db.get_video(id).await?)
}

#[tauri::command]
pub async fn get_videos(
    state: TauriState<'_, State>,
    room_id: u64,
) -> Result<Vec<VideoRow>, String> {
    Ok(state.db.get_videos(room_id).await?)
}

#[tauri::command]
pub async fn delete_video(state: TauriState<'_, State>, id: i64) -> Result<(), String> {
    // get video info from dbus
    let video = state.db.get_video(id).await?;
    // delete video from db
    state.db.delete_video(id).await?;
    // delete video files
    let filepath = format!("{}/{}", state.config.read().await.output, video.file);
    let file = Path::new(&filepath);
    if let Err(e) = std::fs::remove_file(file) {
        log::warn!("Delete video file error: {}", e);
    }
    // delete srt file
    let srt_path = file.with_extension("srt");
    if let Err(e) = std::fs::remove_file(srt_path) {
        log::warn!("Delete srt file error: {}", e);
    }
    // delete wav file
    let wav_path = file.with_extension("wav");
    if let Err(e) = std::fs::remove_file(wav_path) {
        log::warn!("Delete wav file error: {}", e);
    }
    Ok(())
}

#[tauri::command]
pub async fn get_video_typelist(
    state: TauriState<'_, State>,
) -> Result<Vec<crate::recorder::bilibili::response::Typelist>, String> {
    let account = state
        .db
        .get_account("bilibili", state.config.read().await.primary_uid)
        .await?;
    Ok(state.client.get_video_typelist(&account).await?)
}

#[tauri::command]
pub async fn update_video_cover(
    state: TauriState<'_, State>,
    id: i64,
    cover: String,
) -> Result<(), String> {
    Ok(state.db.update_video_cover(id, cover).await?)
}

#[tauri::command]
pub async fn get_video_subtitle(state: TauriState<'_, State>, id: i64) -> Result<String, String> {
    let video = state.db.get_video(id).await?;
    let filepath = format!("{}/{}", state.config.read().await.output, video.file);
    let file = Path::new(&filepath);
    // read file content
    if let Ok(content) = std::fs::read_to_string(file.with_extension("srt")) {
        Ok(content)
    } else {
        Ok("".to_string())
    }
}

#[tauri::command]
pub async fn generate_video_subtitle(
    state: TauriState<'_, State>,
    id: i64,
) -> Result<String, String> {
    let video = state.db.get_video(id).await?;
    let filepath = format!("{}/{}", state.config.read().await.output, video.file);
    let file = Path::new(&filepath);
    if let Ok(generator) = whisper::new(
        Path::new(&state.config.read().await.whisper_model),
        &state.config.read().await.whisper_prompt,
    )
    .await
    {
        let audio_path = file.with_extension("wav");
        ffmpeg::extract_audio(file).await?;
        let subtitle = generator
            .generate_subtitle(&audio_path, &file.with_extension("srt"))
            .await?;
        Ok(subtitle)
    } else {
        Err("Whisper model not found".to_string())
    }
}

#[tauri::command]
pub async fn update_video_subtitle(
    state: TauriState<'_, State>,
    id: i64,
    subtitle: String,
) -> Result<(), String> {
    let video = state.db.get_video(id).await?;
    let filepath = format!("{}/{}", state.config.read().await.output, video.file);
    let file = Path::new(&filepath);
    let subtitle_path = file.with_extension("srt");
    if let Err(e) = std::fs::write(subtitle_path, subtitle) {
        log::warn!("Update video subtitle error: {}", e);
    }
    Ok(())
}

#[tauri::command]
pub async fn encode_video_subtitle(
    state: TauriState<'_, State>,
    id: i64,
    srt_style: String,
) -> Result<VideoRow, String> {
    let video = state.db.get_video(id).await?;
    let filepath = format!("{}/{}", state.config.read().await.output, video.file);
    let file = Path::new(&filepath);
    let output_file =
        ffmpeg::encode_video_subtitle(file, &file.with_extension("srt"), srt_style).await?;

    let new_video = state
        .db
        .add_video(&VideoRow {
            id: 0,
            status: video.status,
            room_id: video.room_id,
            created_at: Utc::now().to_rfc3339(),
            cover: video.cover.clone(),
            file: output_file,
            length: video.length,
            size: video.size,
            bvid: video.bvid.clone(),
            title: video.title.clone(),
            desc: video.desc.clone(),
            tags: video.tags.clone(),
            area: video.area,
        })
        .await?;

    Ok(new_video)
}
