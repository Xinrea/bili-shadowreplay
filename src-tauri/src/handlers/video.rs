use crate::database::task::TaskRow;
use crate::database::video::{VideoNoCover, VideoRow};
use crate::ffmpeg;
use crate::handlers::utils::get_disk_info_inner;
use crate::progress_reporter::{
    cancel_progress, EventEmitter, ProgressReporter, ProgressReporterTrait,
};
use crate::recorder::bilibili::profile::Profile;
use crate::recorder_manager::ClipRangeParams;
use crate::subtitle_generator::item_to_srt;
use chrono::Utc;
use serde_json::json;
use std::path::{Path, PathBuf};

use crate::state::State;
use crate::state_type;

#[cfg(feature = "gui")]
use {tauri::State as TauriState, tauri_plugin_notification::NotificationExt};

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn clip_range(
    state: state_type!(),
    event_id: String,
    params: ClipRangeParams,
) -> Result<VideoRow, String> {
    // check storage space, preserve 1GB for other usage
    let output = state.config.read().await.output.clone();
    let mut output = PathBuf::from(&output);
    if output.is_relative() {
        // get current working directory
        let cwd = std::env::current_dir().unwrap();
        output = cwd.join(output);
    }
    if let Ok(disk_info) = get_disk_info_inner(output).await {
        // if free space is less than 1GB, return error
        if disk_info.free < 1024 * 1024 * 1024 {
            return Err("Storage space is not enough, clip canceled".to_string());
        }
    }
    #[cfg(feature = "gui")]
    let emitter = EventEmitter::new(state.app_handle.clone());
    #[cfg(feature = "headless")]
    let emitter = EventEmitter::new(state.progress_manager.get_event_sender());
    let reporter = ProgressReporter::new(&emitter, &event_id).await?;
    let mut params_without_cover = params.clone();
    params_without_cover.cover = "".to_string();
    let task = TaskRow {
        id: event_id.clone(),
        task_type: "clip_range".to_string(),
        status: "pending".to_string(),
        message: "".to_string(),
        metadata: json!({
            "params": params_without_cover,
        })
        .to_string(),
        created_at: Utc::now().to_rfc3339(),
    };
    state.db.add_task(&task).await?;
    log::info!("Create task: {} {}", task.id, task.task_type);
    match clip_range_inner(&state, &reporter, params).await {
        Ok(video) => {
            reporter.finish(true, "切片完成").await;
            state
                .db
                .update_task(&event_id, "success", "切片完成", None)
                .await?;
            if state.config.read().await.auto_subtitle {
                // generate a subtitle task event id
                let subtitle_event_id = format!("{}_subtitle", event_id);
                let result = generate_video_subtitle(state, subtitle_event_id, video.id).await;
                if let Err(e) = result {
                    log::error!("Generate video subtitle error: {}", e);
                }
            }
            Ok(video)
        }
        Err(e) => {
            reporter.finish(false, &format!("切片失败: {}", e)).await;
            state
                .db
                .update_task(&event_id, "failed", &format!("切片失败: {}", e), None)
                .await?;
            Err(e)
        }
    }
}

async fn clip_range_inner(
    state: &state_type!(),
    reporter: &ProgressReporter,
    params: ClipRangeParams,
) -> Result<VideoRow, String> {
    log::info!(
        "[{}]Clip room_id: {}, ts: {}, start: {}, end: {}",
        reporter.event_id,
        params.room_id,
        params.live_id,
        params.x,
        params.y
    );

    let clip_file = state.config.read().await.generate_clip_name(&params);

    let file = state
        .recorder_manager
        .clip_range(reporter, clip_file, &params)
        .await?;
    log::info!("Clip range done, doing post processing");
    // get file metadata from fs
    let metadata = std::fs::metadata(&file).map_err(|e| {
        log::error!("Get file metadata error: {} {}", e, file.display());
        e.to_string()
    })?;
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
            room_id: params.room_id,
            created_at: Utc::now().to_rfc3339(),
            cover: params.cover.clone(),
            file: filename.into(),
            length: (params.y - params.x),
            size: metadata.len() as i64,
            bvid: "".into(),
            title: "".into(),
            desc: "".into(),
            tags: "".into(),
            area: 0,
            platform: params.platform.clone(),
        })
        .await?;
    state
        .db
        .new_message(
            "生成新切片",
            &format!(
                "生成了房间 {} 的切片，长度 {:.1}s：{}",
                params.room_id,
                params.y - params.x,
                filename
            ),
        )
        .await?;
    if state.config.read().await.clip_notify {
        #[cfg(feature = "gui")]
        state
            .app_handle
            .notification()
            .builder()
            .title("BiliShadowReplay - 切片完成")
            .body(format!(
                "生成了房间 {} 的切片: {}",
                params.room_id, filename
            ))
            .show()
            .unwrap();
    }

    reporter.finish(true, "切片完成").await;

    Ok(video)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn upload_procedure(
    state: state_type!(),
    event_id: String,
    uid: u64,
    room_id: u64,
    video_id: i64,
    cover: String,
    profile: Profile,
) -> Result<String, String> {
    #[cfg(feature = "gui")]
    let emitter = EventEmitter::new(state.app_handle.clone());
    #[cfg(feature = "headless")]
    let emitter = EventEmitter::new(state.progress_manager.get_event_sender());
    let reporter = ProgressReporter::new(&emitter, &event_id).await?;
    let task = TaskRow {
        id: event_id.clone(),
        task_type: "upload_procedure".to_string(),
        status: "pending".to_string(),
        message: "".to_string(),
        metadata: json!({
            "uid": uid,
            "room_id": room_id,
            "video_id": video_id,
            "profile": profile,
        })
        .to_string(),
        created_at: Utc::now().to_rfc3339(),
    };
    state.db.add_task(&task).await?;
    log::info!("Create task: {:?}", task);
    match upload_procedure_inner(&state, &reporter, uid, room_id, video_id, cover, profile).await {
        Ok(bvid) => {
            reporter.finish(true, "投稿成功").await;
            state
                .db
                .update_task(&event_id, "success", "投稿成功", None)
                .await?;
            Ok(bvid)
        }
        Err(e) => {
            reporter.finish(false, &format!("投稿失败: {}", e)).await;
            state
                .db
                .update_task(&event_id, "failed", &format!("投稿失败: {}", e), None)
                .await?;
            Err(e)
        }
    }
}

async fn upload_procedure_inner(
    state: &state_type!(),
    reporter: &ProgressReporter,
    uid: u64,
    room_id: u64,
    video_id: i64,
    cover: String,
    mut profile: Profile,
) -> Result<String, String> {
    let account = state.db.get_account("bilibili", uid).await?;
    // get video info from dbs
    let mut video_row = state.db.get_video(video_id).await?;
    // construct file path
    let output = state.config.read().await.output.clone();
    let file = Path::new(&output).join(&video_row.file);
    let path = Path::new(&file);
    let cover_url = state.client.upload_cover(&account, &cover);
    reporter.update("投稿预处理中");

    match state.client.prepare_video(reporter, &account, path).await {
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
                    #[cfg(feature = "gui")]
                    state
                        .app_handle
                        .notification()
                        .builder()
                        .title("BiliShadowReplay - 投稿成功")
                        .body(format!("投稿了房间 {} 的切片: {}", room_id, ret.bvid))
                        .show()
                        .unwrap();
                }
                reporter.finish(true, "投稿成功").await;
                Ok(ret.bvid)
            } else {
                reporter.finish(false, "投稿失败").await;
                Err("Submit video failed".to_string())
            }
        }
        Err(e) => {
            reporter
                .finish(false, &format!("Preload video failed: {}", e))
                .await;
            Err(format!("Preload video failed: {}", e))
        }
    }
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn cancel(_state: state_type!(), event_id: String) -> Result<(), String> {
    cancel_progress(&event_id).await;
    Ok(())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_video(state: state_type!(), id: i64) -> Result<VideoRow, String> {
    Ok(state.db.get_video(id).await?)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_videos(state: state_type!(), room_id: u64) -> Result<Vec<VideoNoCover>, String> {
    state
        .db
        .get_videos(room_id)
        .await
        .map_err(|e| e.to_string())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_all_videos(state: state_type!()) -> Result<Vec<VideoNoCover>, String> {
    state.db.get_all_videos().await.map_err(|e| e.to_string())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_video_cover(state: state_type!(), id: i64) -> Result<String, String> {
    state
        .db
        .get_video_cover(id)
        .await
        .map_err(|e| e.to_string())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn delete_video(state: state_type!(), id: i64) -> Result<(), String> {
    // get video info from dbus
    let video = state.db.get_video(id).await?;
    // delete video from db
    state.db.delete_video(id).await?;
    // delete video files
    let filepath = Path::new(state.config.read().await.output.as_str()).join(&video.file);
    let file = Path::new(&filepath);
    let _ = std::fs::remove_file(file);

    // delete srt file
    let srt_path = file.with_extension("srt");
    let _ = std::fs::remove_file(srt_path);
    // delete wav file
    let wav_path = file.with_extension("wav");
    let _ = std::fs::remove_file(wav_path);
    // delete mp3 file
    let mp3_path = file.with_extension("mp3");
    let _ = std::fs::remove_file(mp3_path);
    Ok(())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_video_typelist(
    state: state_type!(),
) -> Result<Vec<crate::recorder::bilibili::response::Typelist>, String> {
    let account = state.db.get_account_by_platform("bilibili").await?;
    Ok(state.client.get_video_typelist(&account).await?)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn update_video_cover(
    state: state_type!(),
    id: i64,
    cover: String,
) -> Result<(), String> {
    Ok(state.db.update_video_cover(id, cover).await?)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_video_subtitle(state: state_type!(), id: i64) -> Result<String, String> {
    let video = state.db.get_video(id).await?;
    let filepath = Path::new(state.config.read().await.output.as_str()).join(&video.file);
    let file = Path::new(&filepath);
    // read file content
    if let Ok(content) = std::fs::read_to_string(file.with_extension("srt")) {
        Ok(content)
    } else {
        Ok("".to_string())
    }
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn generate_video_subtitle(
    state: state_type!(),
    event_id: String,
    id: i64,
) -> Result<String, String> {
    #[cfg(feature = "gui")]
    let emitter = EventEmitter::new(state.app_handle.clone());
    #[cfg(feature = "headless")]
    let emitter = EventEmitter::new(state.progress_manager.get_event_sender());
    let reporter = ProgressReporter::new(&emitter, &event_id).await?;
    let task = TaskRow {
        id: event_id.clone(),
        task_type: "generate_video_subtitle".to_string(),
        status: "pending".to_string(),
        message: "".to_string(),
        metadata: json!({
            "video_id": id,
        })
        .to_string(),
        created_at: Utc::now().to_rfc3339(),
    };
    state.db.add_task(&task).await?;
    log::info!("Create task: {:?}", task);
    let config = state.config.read().await;
    let generator_type = config.subtitle_generator_type.as_str();
    let whisper_model = config.whisper_model.clone();
    let whisper_prompt = config.whisper_prompt.clone();
    let openai_api_key = config.openai_api_key.clone();
    let openai_api_endpoint = config.openai_api_endpoint.clone();
    let language_hint = state.config.read().await.whisper_language.clone();
    let language_hint = language_hint.as_str();

    let video = state.db.get_video(id).await?;
    let filepath = Path::new(state.config.read().await.output.as_str()).join(&video.file);
    let file = Path::new(&filepath);

    match ffmpeg::generate_video_subtitle(
        Some(&reporter),
        file,
        generator_type,
        &whisper_model,
        &whisper_prompt,
        &openai_api_key,
        &openai_api_endpoint,
        language_hint,
    )
    .await
    {
        Ok(result) => {
            reporter.finish(true, "字幕生成完成").await;
            // for local whisper, we need to update the task status to success
            state
                .db
                .update_task(
                    &event_id,
                    "success",
                    "字幕生成完成",
                    Some(
                        json!({
                            "task_id": result.subtitle_id,
                            "service": result.generator_type.as_str(),
                        })
                        .to_string()
                        .as_str(),
                    ),
                )
                .await?;

            Ok(result
                .subtitle_content
                .iter()
                .map(item_to_srt)
                .collect::<Vec<String>>()
                .join(""))
        }
        Err(e) => {
            reporter
                .finish(false, &format!("字幕生成失败: {}", e))
                .await;
            state
                .db
                .update_task(&event_id, "failed", &format!("字幕生成失败: {}", e), None)
                .await?;
            Err(e)
        }
    }
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn update_video_subtitle(
    state: state_type!(),
    id: i64,
    subtitle: String,
) -> Result<(), String> {
    let video = state.db.get_video(id).await?;
    let filepath = Path::new(state.config.read().await.output.as_str()).join(&video.file);
    let file = Path::new(&filepath);
    let subtitle_path = file.with_extension("srt");
    if let Err(e) = std::fs::write(subtitle_path, subtitle) {
        log::warn!("Update video subtitle error: {}", e);
    }
    Ok(())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn encode_video_subtitle(
    state: state_type!(),
    event_id: String,
    id: i64,
    srt_style: String,
) -> Result<VideoRow, String> {
    #[cfg(feature = "gui")]
    let emitter = EventEmitter::new(state.app_handle.clone());
    #[cfg(feature = "headless")]
    let emitter = EventEmitter::new(state.progress_manager.get_event_sender());
    let reporter = ProgressReporter::new(&emitter, &event_id).await?;
    let task = TaskRow {
        id: event_id.clone(),
        task_type: "encode_video_subtitle".to_string(),
        status: "pending".to_string(),
        message: "".to_string(),
        metadata: json!({
            "video_id": id,
            "srt_style": srt_style,
        })
        .to_string(),
        created_at: Utc::now().to_rfc3339(),
    };
    state.db.add_task(&task).await?;
    log::info!("Create task: {:?}", task);
    match encode_video_subtitle_inner(&state, &reporter, id, srt_style).await {
        Ok(video) => {
            reporter.finish(true, "字幕编码完成").await;
            state
                .db
                .update_task(&event_id, "success", "字幕编码完成", None)
                .await?;
            Ok(video)
        }
        Err(e) => {
            reporter
                .finish(false, &format!("字幕编码失败: {}", e))
                .await;
            state
                .db
                .update_task(&event_id, "failed", &format!("字幕编码失败: {}", e), None)
                .await?;
            Err(e)
        }
    }
}

async fn encode_video_subtitle_inner(
    state: &state_type!(),
    reporter: &ProgressReporter,
    id: i64,
    srt_style: String,
) -> Result<VideoRow, String> {
    let video = state.db.get_video(id).await?;
    let filepath = Path::new(&state.config.read().await.output).join(&video.file);
    let file = Path::new(&filepath);
    let output_file =
        ffmpeg::encode_video_subtitle(reporter, file, &file.with_extension("srt"), srt_style)
            .await?;

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
            platform: video.platform,
        })
        .await?;

    Ok(new_video)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn generic_ffmpeg_command(
    _state: state_type!(),
    args: Vec<String>,
) -> Result<String, String> {
    let args_str: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    ffmpeg::generic_ffmpeg_command(&args_str).await
}
