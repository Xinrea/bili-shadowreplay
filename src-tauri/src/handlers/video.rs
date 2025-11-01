use crate::database::task::TaskRow;
use crate::database::video::VideoRow;
use crate::ffmpeg;
use crate::handlers::utils::get_disk_info_inner;
use crate::progress::progress_reporter::{
    cancel_progress, EventEmitter, ProgressReporter, ProgressReporterTrait,
};
use crate::recorder_manager::ClipRangeParams;
use crate::subtitle_generator::item_to_srt;
use crate::webhook::events;
use base64::Engine;
use chrono::{Local, Utc};
use recorder::platforms::bilibili;
use recorder::platforms::bilibili::profile::Profile;
use serde_json::json;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// 检测路径是否为网络协议路径（排除Windows盘符）
fn is_network_protocol(path_str: &str) -> bool {
    // 常见的网络协议
    let network_protocols = [
        "ftp://", "sftp://", "ftps://", "http://", "https://", "smb://", "cifs://", "nfs://",
        "afp://", "ssh://", "scp://",
    ];

    // 检查是否以网络协议开头
    for protocol in &network_protocols {
        if path_str.to_lowercase().starts_with(protocol) {
            return true;
        }
    }

    // 排除Windows盘符格式 (如 C:/, D:/, E:/ 等)
    if cfg!(windows) {
        // 检查是否为Windows盘符格式：单字母 + : + /
        if path_str.len() >= 3 {
            let chars: Vec<char> = path_str.chars().collect();
            if chars.len() >= 3
                && chars[0].is_ascii_alphabetic()
                && chars[1] == ':'
                && (chars[2] == '/' || chars[2] == '\\')
            {
                return false; // 这是Windows盘符，不是网络路径
            }
        }
    }

    false
}

/// 判断是否需要转换视频格式
/// FLV格式在现代浏览器中播放兼容性差，需要转换为MP4
fn should_convert_video_format(extension: &str) -> bool {
    // FLV格式在现代浏览器中播放兼容性差，需要转换为MP4
    matches!(extension.to_lowercase().as_str(), "flv")
}

/// 获取视频的最佳缩略图截取时间点
/// 根据视频长度选择最佳时间点，避开开头可能的黑屏
fn get_optimal_thumbnail_timestamp(duration: f64) -> f64 {
    // 根据视频长度选择最佳时间点
    if duration <= 10.0 {
        // 短视频（10秒以内）：选择1/3位置，避免开头黑屏
        duration / 3.0
    } else if duration <= 60.0 {
        // 1分钟以内：选择第3秒
        3.0
    } else if duration <= 300.0 {
        // 5分钟以内：选择第5秒
        5.0
    } else {
        // 长视频：选择第10秒，确保跳过开头可能的黑屏/logo
        10.0
    }
}

use crate::state::State;
use crate::state_type;

// 带进度的文件复制函数
async fn copy_file_with_progress(
    source: &Path,
    dest: &Path,
    reporter: &ProgressReporter,
) -> Result<(), String> {
    let mut source_file = File::open(source).map_err(|e| format!("无法打开源文件: {e}"))?;
    let mut dest_file = File::create(dest).map_err(|e| format!("无法创建目标文件: {e}"))?;

    let total_size = source_file
        .metadata()
        .map_err(|e| format!("无法获取文件大小: {e}"))?
        .len();
    let mut copied = 0u64;

    // 使用固定的小缓冲区避免大文件时的内存占用
    let buffer_size = 64 * 1024; // 64KB buffer for all files

    let mut buffer = vec![0u8; buffer_size];

    let mut last_reported_percent = 0;

    loop {
        let bytes_read = source_file
            .read(&mut buffer)
            .map_err(|e| format!("读取文件失败: {e}"))?;
        if bytes_read == 0 {
            break;
        }

        dest_file
            .write_all(&buffer[..bytes_read])
            .map_err(|e| format!("写入文件失败: {e}"))?;
        copied += bytes_read as u64;

        // 计算进度百分比，只在变化时更新
        let percent = if total_size > 0 {
            ((copied as f64 / total_size as f64) * 100.0) as u32
        } else {
            0
        };

        // 使用固定的进度报告频率
        let report_threshold = 1; // 每1%报告一次

        if percent != last_reported_percent && (percent % report_threshold == 0 || percent == 100) {
            reporter.update(&format!("正在复制视频文件... {percent}%"));
            last_reported_percent = percent;
        }
    }

    dest_file
        .flush()
        .map_err(|e| format!("刷新文件缓冲区失败: {e}"))?;
    Ok(())
}

// 智能边拷贝边转换函数（针对网络文件优化）
async fn copy_and_convert_with_progress(
    source: &Path,
    dest: &Path,
    need_conversion: bool,
    reporter: &ProgressReporter,
) -> Result<(), String> {
    if !need_conversion {
        // 非转换文件直接使用原有拷贝逻辑
        return copy_file_with_progress(source, dest, reporter).await;
    }

    // 检查源文件是否在网络位置（启发式判断）
    let source_str = source.to_string_lossy();
    let is_network_source = source_str.starts_with("\\\\") ||  // UNC path (Windows网络共享)
                           is_network_protocol(&source_str); // 网络协议但排除Windows盘符

    if is_network_source {
        // 网络文件：先复制到本地临时位置，再转换
        reporter.update("检测到网络文件，使用先复制后转换策略...");
        copy_then_convert_strategy(source, dest, reporter).await
    } else {
        // 本地文件：直接转换（更高效）
        reporter.update("检测到本地文件，使用直接转换策略...");
        ffmpeg::convert_video_format(source, dest, reporter).await
    }
}

// 网络文件处理策略：先复制到本地临时位置，再转换
async fn copy_then_convert_strategy(
    source: &Path,
    dest: &Path,
    reporter: &ProgressReporter,
) -> Result<(), String> {
    // 创建临时文件路径
    let temp_dir = std::env::temp_dir();
    let temp_filename = format!(
        "temp_video_{}.{}",
        chrono::Utc::now().timestamp(),
        source.extension().and_then(|e| e.to_str()).unwrap_or("tmp")
    );
    let temp_path = temp_dir.join(&temp_filename);

    // 确保临时目录存在
    if let Some(parent) = temp_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建临时目录失败: {e}"))?;
    }

    // 第一步：将网络文件复制到本地临时位置（使用优化的缓冲区）
    reporter.update("第1步：从网络复制文件到本地临时位置...");
    copy_file_with_network_optimization(source, &temp_path, reporter).await?;

    // 第二步：从本地临时文件转换到目标位置
    reporter.update("第2步：从临时文件转换到目标格式...");
    let convert_result = ffmpeg::convert_video_format(&temp_path, dest, reporter).await;

    // 清理临时文件
    if temp_path.exists() {
        if let Err(e) = std::fs::remove_file(&temp_path) {
            log::warn!("删除临时文件失败: {} - {}", temp_path.display(), e);
        } else {
            log::info!("已清理临时文件: {}", temp_path.display());
        }
    }

    convert_result
}

// 针对网络文件优化的复制函数
async fn copy_file_with_network_optimization(
    source: &Path,
    dest: &Path,
    reporter: &ProgressReporter,
) -> Result<(), String> {
    let mut source_file = File::open(source).map_err(|e| format!("无法打开网络源文件: {e}"))?;
    let mut dest_file = File::create(dest).map_err(|e| format!("无法创建本地临时文件: {e}"))?;

    let total_size = source_file
        .metadata()
        .map_err(|e| format!("无法获取文件大小: {e}"))?
        .len();
    let mut copied = 0u64;

    // 使用固定的小缓冲区，避免大文件时内存占用过多
    let buffer_size = 64 * 1024; // 64KB buffer for network files

    let mut buffer = vec![0u8; buffer_size];
    let mut last_reported_percent = 0;
    let mut consecutive_errors = 0;
    const MAX_RETRIES: u32 = 3;

    loop {
        match source_file.read(&mut buffer) {
            Ok(bytes_read) => {
                if bytes_read == 0 {
                    break; // 文件读取完成
                }

                // 重置错误计数
                consecutive_errors = 0;

                dest_file
                    .write_all(&buffer[..bytes_read])
                    .map_err(|e| format!("写入临时文件失败: {e}"))?;
                copied += bytes_read as u64;

                // 计算并报告进度
                let percent = if total_size > 0 {
                    ((copied as f64 / total_size as f64) * 100.0) as u32
                } else {
                    0
                };

                // 网络文件更频繁地报告进度
                if percent != last_reported_percent {
                    reporter.update(&format!(
                        "正在从网络复制文件... {}% ({:.1}MB/{:.1}MB)",
                        percent,
                        copied as f64 / (1024.0 * 1024.0),
                        total_size as f64 / (1024.0 * 1024.0)
                    ));
                    last_reported_percent = percent;
                }
            }
            Err(e) => {
                consecutive_errors += 1;
                log::warn!("网络读取错误 (尝试 {consecutive_errors}/{MAX_RETRIES}): {e}");

                if consecutive_errors >= MAX_RETRIES {
                    return Err(format!("网络文件读取失败，已重试{MAX_RETRIES}次: {e}"));
                }

                // 等待一小段时间后重试
                tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
                reporter.update(&format!(
                    "网络连接中断，正在重试... ({consecutive_errors}/{MAX_RETRIES})"
                ));
            }
        }
    }

    dest_file
        .flush()
        .map_err(|e| format!("刷新临时文件缓冲区失败: {e}"))?;
    reporter.update("网络文件复制完成");
    Ok(())
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
    params_without_cover.cover = String::new();
    let task = TaskRow {
        id: event_id.clone(),
        task_type: "clip_range".to_string(),
        status: "pending".to_string(),
        message: String::new(),
        metadata: json!({
            "params": params_without_cover,
        })
        .to_string(),
        created_at: Utc::now().to_rfc3339(),
    };

    state.db.add_task(&task).await?;
    log::info!("Create task: {} {}", task.id, task.task_type);

    let clip_result = clip_range_inner(&state, &reporter, params.clone()).await;
    if let Err(e) = clip_result {
        reporter.finish(false, &format!("切片失败: {e}")).await;
        state
            .db
            .update_task(&event_id, "failed", &format!("切片失败: {e}"), None)
            .await?;
        return Err(e);
    }

    let video = clip_result.unwrap();

    reporter.finish(true, "切片完成").await;
    state
        .db
        .update_task(&event_id, "success", "切片完成", None)
        .await?;

    if state.config.read().await.auto_subtitle {
        // generate a subtitle task event id
        let subtitle_event_id = format!("{event_id}_subtitle");
        let result = generate_video_subtitle(state.clone(), subtitle_event_id, video.id).await;
        if let Ok(subtitle) = result {
            let result = update_video_subtitle(state.clone(), video.id, subtitle).await;
            if let Err(e) = result {
                log::error!("Update video subtitle error: {e}");
            }
        } else {
            log::error!("Generate video subtitle error: {}", result.err().unwrap());
        }
    }

    // Emit webhook events
    let event =
        events::new_webhook_event(events::CLIP_GENERATED, events::Payload::Clip(video.clone()));

    if let Err(e) = state.webhook_poster.post_event(&event).await {
        log::error!("Post webhook event error: {e}");
    }

    Ok(video)
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
        params
            .range
            .as_ref()
            .map_or("None".to_string(), |r| r.start.to_string()),
        params
            .range
            .as_ref()
            .map_or("None".to_string(), |r| r.end.to_string()),
    );

    let clip_file = state.config.read().await.generate_clip_name(&params);

    let file = state
        .recorder_manager
        .clip_range(Some(reporter), clip_file, &params)
        .await?;
    log::info!("Clip range done, doing post processing");
    // get file metadata from fs
    let metadata = std::fs::metadata(&file).map_err(|e| {
        log::error!("Get file metadata error: {} {}", e, file.display());
        e.to_string()
    })?;
    let cover_file = file.with_extension("jpg");
    let base64 = params.cover.split("base64,").nth(1).unwrap();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(base64)
        .unwrap();
    // write cover file to fs
    tokio::fs::write(&cover_file, bytes).await.map_err(|e| {
        log::error!("Write cover file error: {} {}", e, cover_file.display());
        e.to_string()
    })?;
    // get filename from path
    let filename = Path::new(&file)
        .file_name()
        .ok_or("Invalid file path")?
        .to_str()
        .ok_or("Invalid file path")?;
    // add video to db
    let Ok(size) = i64::try_from(metadata.len()) else {
        log::error!(
            "Failed to convert metadata length to i64: {}",
            metadata.len()
        );
        return Err("Failed to convert metadata length to i64".to_string());
    };
    let video = state
        .db
        .add_video(&VideoRow {
            id: 0,
            status: 0,
            room_id: params.room_id.clone(),
            created_at: Local::now().to_rfc3339(),
            cover: cover_file
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .to_string(),
            file: filename.into(),
            note: params.note.clone(),
            length: params
                .range
                .as_ref()
                .map_or(0.0, super::super::ffmpeg::Range::duration) as i64,
            size,
            bvid: String::new(),
            title: String::new(),
            desc: String::new(),
            tags: String::new(),
            area: 0,
            platform: params.platform.clone(),
        })
        .await?;
    state
        .db
        .new_message(
            "生成新切片",
            &format!(
                "生成了房间 {} 的切片，长度 {}s：{}",
                &params.room_id,
                params
                    .range
                    .as_ref()
                    .map_or(0.0, super::super::ffmpeg::Range::duration),
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
                &params.room_id, filename
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
    uid: String,
    room_id: String,
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
        message: String::new(),
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
    log::info!("Create task: {task:?}");
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
            reporter.finish(false, &format!("投稿失败: {e}")).await;
            state
                .db
                .update_task(&event_id, "failed", &format!("投稿失败: {e}"), None)
                .await?;
            Err(e)
        }
    }
}

async fn upload_procedure_inner(
    state: &state_type!(),
    reporter: &ProgressReporter,
    uid: String,
    room_id: String,
    video_id: i64,
    cover: String,
    mut profile: Profile,
) -> Result<String, String> {
    let account = state.db.get_account("bilibili", &uid).await?;
    // get video info from dbs
    let mut video_row = state.db.get_video(video_id).await?;
    // construct file path
    let output = state.config.read().await.output.clone();
    let file = Path::new(&output).join(&video_row.file);
    let path = Path::new(&file);
    let client = reqwest::Client::new();
    let cover_url = bilibili::api::upload_cover(&client, &account.to_account(), &cover).await;
    reporter.update("投稿预处理中");

    match bilibili::api::prepare_video(&client, &account.to_account(), path).await {
        Ok(video) => {
            profile.cover = cover_url.unwrap_or(String::new());
            if let Ok(ret) =
                bilibili::api::submit_video(&client, &account.to_account(), &profile, &video).await
            {
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
                .finish(false, &format!("Preload video failed: {e}"))
                .await;
            Err(format!("Preload video failed: {e}"))
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
pub async fn get_videos(state: state_type!(), room_id: String) -> Result<Vec<VideoRow>, String> {
    state
        .db
        .get_videos(&room_id)
        .await
        .map_err(|e| e.to_string())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_all_videos(state: state_type!()) -> Result<Vec<VideoRow>, String> {
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
    // get video info from db
    let video = state.db.get_video(id).await?;
    let config = state.config.read().await;

    // Emit webhook events
    let event =
        events::new_webhook_event(events::CLIP_DELETED, events::Payload::Clip(video.clone()));
    if let Err(e) = state.webhook_poster.post_event(&event).await {
        log::error!("Post webhook event error: {e}");
    }

    // delete video from db
    state.db.delete_video(id).await?;

    // delete video files
    let filepath = Path::new(&config.output).join(&video.file);
    let file = Path::new(&filepath);
    if let Err(e) = std::fs::remove_file(file) {
        log::warn!("删除视频文件失败: {} - {}", file.display(), e);
    } else {
        log::info!("已删除视频文件: {}", file.display());
    }

    // delete all related files
    let srt_path = file.with_extension("srt");
    let _ = tokio::fs::remove_file(srt_path).await;
    let wav_path = file.with_extension("wav");
    let _ = tokio::fs::remove_file(wav_path).await;
    let mp3_path = file.with_extension("mp3");
    let _ = tokio::fs::remove_file(mp3_path).await;
    let cover_path = Path::new(&config.output).join(&video.cover);
    let _ = tokio::fs::remove_file(cover_path).await;

    Ok(())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_video_typelist(
    state: state_type!(),
) -> Result<Vec<bilibili::response::Typelist>, String> {
    let account = state.db.get_account_by_platform("bilibili").await?;
    let client = reqwest::Client::new();
    match bilibili::api::get_video_typelist(&client, &account.to_account()).await {
        Ok(typelist) => Ok(typelist),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn update_video_cover(
    state: state_type!(),
    id: i64,
    cover: String,
) -> Result<(), String> {
    let video = state.db.get_video(id).await?;
    let output_path = Path::new(state.config.read().await.output.as_str()).join(&video.file);
    let cover_path = output_path.with_extension("jpg");
    // decode cover and write into file
    let base64 = cover.split("base64,").nth(1).unwrap();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(base64)
        .unwrap();
    tokio::fs::write(&cover_path, bytes)
        .await
        .map_err(|e| e.to_string())?;
    let cover_file_name = cover_path.file_name().unwrap().to_str().unwrap();
    log::debug!("Update video cover: {id} {cover_file_name}");
    Ok(state.db.update_video_cover(id, cover_file_name).await?)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_video_subtitle(state: state_type!(), id: i64) -> Result<String, String> {
    log::debug!("Get video subtitle: {id}");
    let video = state.db.get_video(id).await?;
    let filepath = Path::new(state.config.read().await.output.as_str()).join(&video.file);
    let file = Path::new(&filepath);
    // read file content
    if let Ok(content) = std::fs::read_to_string(file.with_extension("srt")) {
        Ok(content)
    } else {
        Ok(String::new())
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
        message: String::new(),
        metadata: json!({
            "video_id": id,
        })
        .to_string(),
        created_at: Utc::now().to_rfc3339(),
    };
    state.db.add_task(&task).await?;
    log::info!("Create task: {task:?}");
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
                .collect::<String>();

            let result = update_video_subtitle(state.clone(), id, subtitle.clone()).await;
            if let Err(e) = result {
                log::error!("Update video subtitle error: {e}");
            }
            Ok(subtitle)
        }
        Err(e) => {
            reporter.finish(false, &format!("字幕生成失败: {e}")).await;
            state
                .db
                .update_task(&event_id, "failed", &format!("字幕生成失败: {e}"), None)
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
        log::warn!("Update video subtitle error: {e}");
    }
    Ok(())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn update_video_note(state: state_type!(), id: i64, note: String) -> Result<(), String> {
    log::info!("Update video note: {id} -> {note}");
    let mut video = state.db.get_video(id).await?;
    video.note = note;
    state.db.update_video(&video).await?;
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
        message: String::new(),
        metadata: json!({
            "video_id": id,
            "srt_style": srt_style,
        })
        .to_string(),
        created_at: Utc::now().to_rfc3339(),
    };
    state.db.add_task(&task).await?;
    log::info!("Create task: {task:?}");
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
            reporter.finish(false, &format!("字幕编码失败: {e}")).await;
            state
                .db
                .update_task(&event_id, "failed", &format!("字幕编码失败: {e}"), None)
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
    let subtitle_path = filepath.with_extension("srt");

    let output_filename =
        ffmpeg::encode_video_subtitle(reporter, &filepath, &subtitle_path, srt_style).await?;

    let new_video = state
        .db
        .add_video(&VideoRow {
            id: 0,
            status: video.status,
            room_id: video.room_id,
            created_at: Local::now().to_rfc3339(),
            cover: video.cover.clone(),
            file: output_filename,
            note: video.note.clone(),
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
    let args_str: Vec<&str> = args.iter().map(std::string::String::as_str).collect();
    ffmpeg::generic_ffmpeg_command(&args_str).await
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn import_external_video(
    state: state_type!(),
    event_id: String,
    file_path: String,
    title: String,
    room_id: String,
) -> Result<VideoRow, String> {
    #[cfg(feature = "gui")]
    let emitter = EventEmitter::new(state.app_handle.clone());
    #[cfg(feature = "headless")]
    let emitter = EventEmitter::new(state.progress_manager.get_event_sender());

    let reporter = ProgressReporter::new(&emitter, &event_id).await?;

    let source_path = Path::new(&file_path);
    if !source_path.exists() {
        return Err("文件不存在".to_string());
    }

    reporter.update("正在提取视频元数据...");
    let metadata = ffmpeg::extract_video_metadata(source_path).await?;
    let output_str = state.config.read().await.output.clone();
    let output_dir = Path::new(&output_str);
    if !output_dir.exists() {
        std::fs::create_dir_all(output_dir).map_err(|e| e.to_string())?;
    }

    let timestamp = Utc::now().format("%Y%m%d_%H%M%S").to_string();
    let extension = source_path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("mp4");
    let mut target_filename = format!(
        "{}{}{}.{}",
        crate::constants::PREFIX_IMPORTED,
        sanitize_filename(&title),
        timestamp,
        extension
    );
    let target_full_path = output_dir.join(&target_filename);

    let need_conversion = should_convert_video_format(extension);
    let final_target_full_path = if need_conversion {
        let mp4_target_full_path = target_full_path.with_extension("mp4");

        reporter.update("准备转换视频格式 (FLV → MP4)...");

        copy_and_convert_with_progress(source_path, &mp4_target_full_path, true, &reporter).await?;

        // 更新最终文件名和路径
        target_filename = mp4_target_full_path
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        mp4_target_full_path
    } else {
        // 其他格式使用智能拷贝
        copy_and_convert_with_progress(source_path, &target_full_path, false, &reporter).await?;
        target_full_path
    };

    // 步骤3: 生成缩略图
    reporter.update("正在生成视频缩略图...");

    // 生成缩略图，使用智能时间点选择
    let thumbnail_timestamp = get_optimal_thumbnail_timestamp(metadata.duration);
    let cover_path =
        match ffmpeg::generate_thumbnail(&final_target_full_path, thumbnail_timestamp).await {
            Ok(path) => path.file_name().unwrap().to_str().unwrap().to_string(),
            Err(e) => {
                log::warn!("生成缩略图失败: {e}");
                String::new() // 使用空字符串，前端会显示默认图标
            }
        };

    // 步骤4: 保存到数据库
    reporter.update("正在保存视频信息...");

    let Ok(size) = i64::try_from(
        final_target_full_path
            .metadata()
            .map_err(|e| e.to_string())?
            .len(),
    ) else {
        log::error!(
            "Failed to convert metadata length to i64: {}",
            final_target_full_path
                .metadata()
                .map_err(|e| e.to_string())?
                .len()
        );
        return Err("Failed to convert metadata length to i64".to_string());
    };

    // 添加到数据库
    let video = VideoRow {
        id: 0,
        room_id, // 使用传入的 room_id
        platform: "imported".to_string(),
        title,
        file: target_filename,
        note: String::new(),
        length: metadata.duration as i64,
        size,
        status: 1, // 导入完成
        cover: cover_path,
        desc: String::new(),
        tags: String::new(),
        bvid: String::new(),
        area: 0,
        created_at: Utc::now().to_rfc3339(),
    };

    let result = state.db.add_video(&video).await?;

    // 完成进度通知
    reporter.finish(true, "视频导入完成").await;

    // 发送通知消息
    state
        .db
        .new_message("视频导入完成", &format!("成功导入视频：{}", result.title))
        .await?;

    log::info!("导入视频成功: {} -> {}", file_path, result.file);
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
        message: String::new(),
        metadata: json!({
            "parent_video_id": parent_video_id,
            "start_time": start_time,
            "end_time": end_time,
            "clip_title": clip_title,
        })
        .to_string(),
        created_at: Utc::now().to_rfc3339(),
    };
    state.db.add_task(&task).await?;

    match clip_video_inner(
        &state,
        &reporter,
        parent_video,
        start_time,
        end_time,
        clip_title,
    )
    .await
    {
        Ok(video) => {
            reporter.finish(true, "切片完成").await;
            state
                .db
                .update_task(&event_id, "success", "切片完成", None)
                .await?;
            Ok(video)
        }
        Err(e) => {
            reporter.finish(false, &format!("切片失败: {e}")).await;
            state
                .db
                .update_task(&event_id, "failed", &format!("切片失败: {e}"), None)
                .await?;
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
    let input_path = Path::new(&config.output).join(&parent_video.file);

    if !input_path.exists() {
        return Err("原视频文件不存在".to_string());
    }

    // 统一的输出目录：clips
    let output_dir = Path::new(&config.output).join("clips");
    if !output_dir.exists() {
        std::fs::create_dir_all(&output_dir).map_err(|e| e.to_string())?;
    }

    let timestamp = Local::now().format("%Y%m%d%H%M").to_string();
    let extension = input_path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("mp4");

    // 获取原文件名（不含扩展名）
    let original_filename = input_path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("video");

    // 生成新的文件名格式：[clip]原文件名[时间戳].扩展名
    let output_filename = format!(
        "{}{}[{}].{}",
        crate::constants::PREFIX_CLIP,
        original_filename,
        timestamp,
        extension
    );
    let output_full_path = output_dir.join(&output_filename);

    // 执行切片
    reporter.update("开始切片处理");
    ffmpeg::clip_from_video_file(
        Some(reporter),
        &input_path,
        &output_full_path,
        start_time,
        end_time - start_time,
    )
    .await?;

    // 生成缩略图文件名，确保路径安全
    let thumbnail_full_path = output_full_path.with_extension("jpg");

    // 生成缩略图，选择切片开头的合理位置
    let clip_duration = end_time - start_time;
    let clip_thumbnail_timestamp = get_optimal_thumbnail_timestamp(clip_duration);
    let clip_cover_path =
        match ffmpeg::generate_thumbnail(&output_full_path, clip_thumbnail_timestamp).await {
            Ok(_) => thumbnail_full_path
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .to_string(),
            Err(e) => {
                log::warn!("生成切片缩略图失败: {e}");
                String::new() // 使用空字符串，前端会显示默认图标
            }
        };

    let file_metadata = output_full_path.metadata().map_err(|e| e.to_string())?;

    let clip_video = VideoRow {
        id: 0,
        room_id: parent_video.room_id,
        platform: "clip".to_string(),
        title: clip_title,
        file: output_filename,
        note: String::new(),
        length: (end_time - start_time) as i64,
        size: i64::try_from(file_metadata.len()).map_err(|e| e.to_string())?,
        status: 1,
        cover: clip_cover_path,
        desc: String::new(),
        tags: String::new(),
        bvid: String::new(),
        area: parent_video.area,
        created_at: Local::now().to_rfc3339(),
    };

    let result = state.db.add_video(&clip_video).await?;

    // 发送通知消息
    state
        .db
        .new_message("视频切片完成", &format!("生成切片：{}", result.title))
        .await?;

    Ok(result)
}

// 获取文件大小
#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_file_size(file_path: String) -> Result<u64, String> {
    let path = Path::new(&file_path);
    match std::fs::metadata(path) {
        Ok(metadata) => Ok(metadata.len()),
        Err(e) => Err(format!("无法获取文件信息: {e}")),
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

/// 批量导入结果结构
#[derive(serde::Serialize, serde::Deserialize)]
pub struct BatchImportResult {
    pub successful_imports: i32,
    pub failed_imports: i32,
    pub imported_video_ids: Vec<i64>,
    pub errors: Vec<String>,
}

/// 批量导入外部视频文件
///
/// # 参数
/// - `state`: 应用状态
/// - `event_id`: 进度事件ID
/// - `file_paths`: 要导入的文件路径列表
/// - `room_id`: 房间ID
///
/// # 返回值
/// 返回批量导入结果，包含成功数量、失败数量、视频ID列表和错误信息
#[cfg_attr(feature = "gui", tauri::command)]
pub async fn batch_import_external_videos(
    state: state_type!(),
    event_id: String,
    file_paths: Vec<String>,
    room_id: String,
) -> Result<BatchImportResult, String> {
    if file_paths.is_empty() {
        return Ok(BatchImportResult {
            successful_imports: 0,
            failed_imports: 0,
            imported_video_ids: Vec::new(),
            errors: Vec::new(),
        });
    }

    let mut successful_imports = 0;
    let mut failed_imports = 0;
    let mut imported_video_ids = Vec::new();
    let mut errors = Vec::new();

    // 设置批量进度事件发射器
    #[cfg(feature = "gui")]
    let emitter = EventEmitter::new(state.app_handle.clone());
    #[cfg(feature = "headless")]
    let emitter = EventEmitter::new(state.progress_manager.get_event_sender());
    let batch_reporter = ProgressReporter::new(&emitter, &event_id).await?;

    let total_files = file_paths.len();

    for (index, file_path) in file_paths.iter().enumerate() {
        let current_index = index + 1;
        let file_name = Path::new(file_path)
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("unknown")
            .to_string();

        // 更新批量进度，只显示进度信息
        batch_reporter.update(&format!(
            "正在导入第{current_index}个，共{total_files}个文件"
        ));

        // 为每个文件创建独立的事件ID
        let file_event_id = format!("{event_id}_file_{index}");

        // 从文件名生成标题（去掉扩展名）
        let title = file_name.clone();

        // 调用现有的单文件导入函数
        match import_external_video(
            state.clone(),
            file_event_id,
            file_path.clone(),
            title,
            room_id.clone(),
        )
        .await
        {
            Ok(video) => {
                imported_video_ids.push(video.id);
                successful_imports += 1;
                log::info!("批量导入成功: {} (ID: {})", file_path, video.id);
            }
            Err(e) => {
                let error_msg = format!("导入失败 {file_path}: {e}");
                errors.push(error_msg.clone());
                failed_imports += 1;
                log::error!("批量导入失败: {error_msg}");
            }
        }
    }

    // 完成批量导入
    let result_msg = if failed_imports == 0 {
        format!("批量导入完成：成功导入{successful_imports}个文件")
    } else {
        format!("批量导入完成：成功{successful_imports}个，失败{failed_imports}个")
    };
    batch_reporter
        .finish(failed_imports == 0, &result_msg)
        .await;

    // 发送通知消息
    state
        .db
        .new_message("批量视频导入完成", &result_msg)
        .await?;

    Ok(BatchImportResult {
        successful_imports,
        failed_imports,
        imported_video_ids,
        errors,
    })
}

// 查询当前导入进度
#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_import_progress(
    state: state_type!(),
) -> Result<Option<serde_json::Value>, String> {
    // 查询进行中的FLV转换任务
    let all_tasks = state.db.get_tasks().await.map_err(|e| e.to_string())?;

    // 查找状态为 "pending" 或 "running" 的 import_flv_conversion 任务
    for task in &all_tasks {
        if task.task_type == "import_flv_conversion"
            && (task.status == "pending" || task.status == "running")
        {
            // 解析任务元数据
            let metadata: serde_json::Value =
                serde_json::from_str(&task.metadata).unwrap_or_default();

            return Ok(Some(serde_json::json!({
                "task_id": task.id,
                "file_name": metadata.get("file_name").and_then(|v| v.as_str()).unwrap_or("未知文件"),
                "file_size": metadata.get("file_size").and_then(serde_json::Value::as_u64).unwrap_or(0),
                "message": task.message,
                "status": task.status,
                "created_at": task.created_at
            })));
        }
    }

    Ok(None)
}
