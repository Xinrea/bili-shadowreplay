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
use chrono::{Local, Utc};
use serde_json::json;
use std::path::{Path, PathBuf};

use crate::state::State;
use crate::state_type;

// 导入视频相关的数据结构
#[derive(serde::Serialize, serde::Deserialize)]
struct ImportedVideoMetadata {
    original_path: String,
    import_date: String,
    original_size: i64,
    video_format: String,
    duration: f64,
    resolution: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ClipMetadata {
    parent_video_id: i64,
    start_time: f64,
    end_time: f64,
    clip_source: String,
    original_platform: String,
    original_room_id: u64,
}

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
                let result =
                    generate_video_subtitle(state.clone(), subtitle_event_id, video.id).await;
                if let Ok(subtitle) = result {
                    let result = update_video_subtitle(state.clone(), video.id, subtitle).await;
                    if let Err(e) = result {
                        log::error!("Update video subtitle error: {}", e);
                    }
                } else {
                    log::error!("Generate video subtitle error: {}", result.err().unwrap());
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
            created_at: Local::now().to_rfc3339(),
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
    log::debug!("Get video subtitle: {}", id);
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

            let subtitle = result
                .subtitle_content
                .iter()
                .map(item_to_srt)
                .collect::<Vec<String>>()
                .join("");

            let result = update_video_subtitle(state.clone(), id, subtitle.clone()).await;
            if let Err(e) = result {
                log::error!("Update video subtitle error: {}", e);
            }
            Ok(subtitle)
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
    let config = state.config.read().await;
    let filepath = Path::new(&config.output).join(&video.file);
    
    // 查找字幕文件：对于切片视频，需要查找原视频的字幕文件
    let subtitle_path = find_subtitle_file(state, &video, &filepath).await?;
    
    let output_file =
        ffmpeg::encode_video_subtitle(reporter, &filepath, &subtitle_path, srt_style)
            .await?;

    // 构建正确的相对路径：如果原文件在子目录中，保持相同的目录结构
    let relative_output_file = if let Some((parent_dir, _)) = video.file.rsplit_once('/') {
        // 原文件在子目录中（如 clips/xxx.mp4），保持目录结构
        format!("{}/{}", parent_dir, output_file)
    } else {
        // 原文件在根目录
        output_file
    };

    // 为标题添加 [subtitle] 前缀
    let subtitle_title = if video.title.starts_with("[subtitle]") {
        video.title.clone() // 如果已经有前缀，不再添加
    } else {
        format!("[subtitle]{}", video.title)
    };

    let new_video = state
        .db
        .add_video(&VideoRow {
            id: 0,
            status: video.status,
            room_id: video.room_id,
            created_at: Local::now().to_rfc3339(),
            cover: video.cover.clone(),
            file: relative_output_file,
            length: video.length,
            size: video.size,
            bvid: video.bvid.clone(),
            title: subtitle_title,
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

// 导入外部视频
#[cfg_attr(feature = "gui", tauri::command)]
pub async fn import_external_video(
    state: state_type!(),
    file_path: String,
    title: String,
    _original_name: String,
    size: i64,
    room_id: u64,
) -> Result<VideoRow, String> {
    let source_path = Path::new(&file_path);
    
    // 验证文件存在
    if !source_path.exists() {
        return Err("文件不存在".to_string());
    }
    
    // 获取视频元数据
    let metadata = ffmpeg::extract_video_metadata(source_path).await?;
    
    // 生成目标文件名
    let config = state.config.read().await;
    let output_dir = Path::new(&config.output).join("imported");
    if !output_dir.exists() {
        std::fs::create_dir_all(&output_dir).map_err(|e| e.to_string())?;
    }
    
    let timestamp = Utc::now().format("%Y%m%d_%H%M%S").to_string();
    let extension = source_path.extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("mp4");
    let target_filename = format!("imported_{}_{}.{}", timestamp, 
        sanitize_filename(&title), extension);
    let target_path = output_dir.join(&target_filename);
    
    // 复制文件到目标位置
    std::fs::copy(source_path, &target_path).map_err(|e| e.to_string())?;
    
    // 生成缩略图
    let thumbnail_dir = Path::new(&config.output).join("thumbnails").join("imported");
    if !thumbnail_dir.exists() {
        std::fs::create_dir_all(&thumbnail_dir).map_err(|e| e.to_string())?;
    }
    
    let thumbnail_filename = format!("{}.jpg", 
        target_path.file_stem().unwrap().to_str().unwrap());
    let thumbnail_path = thumbnail_dir.join(&thumbnail_filename);
    
    // 生成缩略图，如果失败则使用默认封面
    let cover_path = match ffmpeg::generate_thumbnail(&target_path, &thumbnail_path, metadata.duration / 2.0).await {
        Ok(_) => format!("thumbnails/imported/{}", thumbnail_filename),
        Err(e) => {
            log::warn!("生成缩略图失败: {}", e);
            "".to_string() // 使用空字符串，前端会显示默认图标
        }
    };
    
    // 构建导入视频的元数据
    let import_metadata = ImportedVideoMetadata {
        original_path: file_path.clone(),
        import_date: Utc::now().to_rfc3339(),
        original_size: size,
        video_format: extension.to_string(),
        duration: metadata.duration,
        resolution: Some(format!("{}x{}", metadata.width, metadata.height)),
    };
    
    // 添加到数据库
    let video = VideoRow {
        id: 0,
        room_id, // 使用传入的 room_id
        platform: "imported".to_string(), // 使用 platform 字段标识
        title,
        file: format!("imported/{}", target_filename), // 包含完整相对路径
        length: metadata.duration as i64,
        size: target_path.metadata().map_err(|e| e.to_string())?.len() as i64,
        status: 1, // 导入完成
        cover: cover_path,
        desc: serde_json::to_string(&import_metadata).unwrap_or_default(),
        tags: "imported,external".to_string(),
        bvid: "".to_string(),
        area: 0,
        created_at: Utc::now().to_rfc3339(),
    };
    
    let result = state.db.add_video(&video).await?;
    
    // 发送通知消息
    state.db.new_message(
        "视频导入完成",
        &format!("成功导入视频：{}", result.title),
    ).await?;
    
    Ok(result)
}

// 通用视频切片函数（支持所有类型的视频）
#[cfg_attr(feature = "gui", tauri::command)]
pub async fn clip_video(
    state: state_type!(),
    event_id: String,
    parent_video_id: i64,
    start_time: f64,
    end_time: f64,
    clip_title: String,
) -> Result<VideoRow, String> {
    // 获取父视频信息
    let parent_video = state.db.get_video(parent_video_id).await?;
    
    // 检查是否为正在录制的视频
    if parent_video.status == -1 {
        return Err("正在录制的视频无法进行切片".to_string());
    }
    
    #[cfg(feature = "gui")]
    let emitter = EventEmitter::new(state.app_handle.clone());
    #[cfg(feature = "headless")]
    let emitter = EventEmitter::new(state.progress_manager.get_event_sender());
    let reporter = ProgressReporter::new(&emitter, &event_id).await?;
    
    // 创建任务记录
    let task = TaskRow {
        id: event_id.clone(),
        task_type: "clip_video".to_string(),
        status: "pending".to_string(),
        message: "".to_string(),
        metadata: json!({
            "parent_video_id": parent_video_id,
            "start_time": start_time,
            "end_time": end_time,
            "clip_title": clip_title,
        }).to_string(),
        created_at: Utc::now().to_rfc3339(),
    };
    state.db.add_task(&task).await?;
    
    match clip_video_inner(&state, &reporter, parent_video, start_time, end_time, clip_title).await {
        Ok(video) => {
            reporter.finish(true, "切片完成").await;
            state.db.update_task(&event_id, "success", "切片完成", None).await?;
            Ok(video)
        }
        Err(e) => {
            reporter.finish(false, &format!("切片失败: {}", e)).await;
            state.db.update_task(&event_id, "failed", &format!("切片失败: {}", e), None).await?;
            Err(e)
        }
    }
}



async fn clip_video_inner(
    state: &state_type!(),
    reporter: &ProgressReporter,
    parent_video: VideoRow,
    start_time: f64,
    end_time: f64,
    clip_title: String,
) -> Result<VideoRow, String> {
    let config = state.config.read().await;
    
    // 构建输入文件路径
    let input_path = Path::new(&config.output)
        .join(&parent_video.file);
    
    if !input_path.exists() {
        return Err("原视频文件不存在".to_string());
    }
    
    // 统一的输出目录：clips
    let output_dir = Path::new(&config.output).join("clips");
    if !output_dir.exists() {
        std::fs::create_dir_all(&output_dir).map_err(|e| e.to_string())?;
    }
    
    let timestamp = Local::now().format("%Y%m%d%H%M").to_string();
    let extension = input_path.extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("mp4");
    
    // 获取原文件名（不含扩展名）
    let original_filename = input_path.file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("video");
    
    // 生成新的文件名格式：原文件名[clip][时间戳].扩展名
    let output_filename = format!("{}[clip][{}].{}", 
        original_filename, timestamp, extension);
    let output_path = output_dir.join(&output_filename);
    
    // 执行切片
    reporter.update("开始切片处理");
    ffmpeg::clip_from_video_file(
        Some(reporter),
        &input_path,
        &output_path,
        start_time,
        end_time - start_time,
    ).await?;
    
    // 生成缩略图
    let thumbnail_dir = Path::new(&config.output).join("thumbnails").join("clips");
    if !thumbnail_dir.exists() {
        std::fs::create_dir_all(&thumbnail_dir).map_err(|e| e.to_string())?;
    }
    
    // 生成缩略图文件名，确保路径安全
    let clip_thumbnail_filename = if let Some(stem) = output_path.file_stem().and_then(|s| s.to_str()) {
        format!("{}.jpg", stem)
    } else {
        format!("thumbnail_{}.jpg", timestamp)
    };
    let thumbnail_path = thumbnail_dir.join(&clip_thumbnail_filename);
    
    // 生成缩略图，如果失败则使用默认封面
    let clip_cover_path = match ffmpeg::generate_thumbnail(&output_path, &thumbnail_path, (end_time - start_time) / 2.0).await {
        Ok(_) => format!("thumbnails/clips/{}", clip_thumbnail_filename),
        Err(e) => {
            log::warn!("生成切片缩略图失败: {}", e);
            "".to_string() // 使用空字符串，前端会显示默认图标
        }
    };
    
    // 构建统一的切片元数据
    let clip_metadata = ClipMetadata {
        parent_video_id: parent_video.id,
        start_time,
        end_time,
        clip_source: determine_clip_source(&parent_video.platform),
        original_platform: parent_video.platform.clone(),
        original_room_id: parent_video.room_id,
    };
    
    // 获取输出文件信息
    let file_metadata = output_path.metadata().map_err(|e| e.to_string())?;
    
    // 添加到数据库 - 统一使用 "clip" 平台类型
    let clip_video = VideoRow {
        id: 0,
        room_id: parent_video.room_id,
        platform: "clip".to_string(), // 统一的切片类型
        title: clip_title,
        file: format!("clips/{}", output_filename),
        length: (end_time - start_time) as i64,
        size: file_metadata.len() as i64,
        status: 1,
        cover: clip_cover_path,
        desc: serde_json::to_string(&clip_metadata).unwrap_or_default(),
        tags: "clip".to_string(),
        bvid: "".to_string(),
        area: parent_video.area,
        created_at: Local::now().to_rfc3339(),
    };
    
    let result = state.db.add_video(&clip_video).await?;
    
    // 发送通知消息
    state.db.new_message(
        "视频切片完成",
        &format!("生成切片：{}", result.title),
    ).await?;
    
    Ok(result)
}

// 确定切片来源的辅助函数
fn determine_clip_source(platform: &str) -> String {
    match platform {
        "imported" => "imported_video".to_string(),
        "clip" => "clip".to_string(),
        _ => "recorded_video".to_string(),
    }
}

// 查找字幕文件的辅助函数
async fn find_subtitle_file(
    state: &state_type!(),
    video: &VideoRow,
    video_file: &Path,
) -> Result<PathBuf, String> {
    // 首先尝试当前视频同目录下的字幕文件
    let local_subtitle = video_file.with_extension("srt");
    if local_subtitle.exists() {
        return Ok(local_subtitle);
    }
    
    // 如果是切片视频，尝试查找原视频的字幕文件
    if video.platform == "clip" && !video.desc.is_empty() {
        // 解析切片元数据，获取父视频ID
        if let Ok(metadata) = serde_json::from_str::<ClipMetadata>(&video.desc) {
            if let Ok(parent_video) = state.db.get_video(metadata.parent_video_id).await {
                let parent_filepath = Path::new(&state.config.read().await.output)
                    .join(&parent_video.file);
                let parent_subtitle = parent_filepath.with_extension("srt");
                if parent_subtitle.exists() {
                    return Ok(parent_subtitle);
                }
            }
        }
    }
    
    // 如果都找不到，返回默认路径（即使文件不存在，让ffmpeg处理错误）
    Ok(local_subtitle)
}



// 获取文件大小
#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_file_size(file_path: String) -> Result<u64, String> {
    let path = Path::new(&file_path);
    match std::fs::metadata(path) {
        Ok(metadata) => Ok(metadata.len()),
        Err(e) => Err(format!("无法获取文件信息: {}", e))
    }
}

// 辅助函数：清理文件名
fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect::<String>()
        .chars()
        .take(50) // 限制长度
        .collect()
}
