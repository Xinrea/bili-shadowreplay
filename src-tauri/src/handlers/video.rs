use crate::database::task::TaskRow;
use crate::database::video::VideoRow;
use crate::ffmpeg;
use crate::handlers::subtitle_fonts::prepare_subtitle_font_directory;
use crate::handlers::utils::get_disk_info_inner;
use crate::progress::progress_reporter::{ProgressReporter, ProgressReporterTrait};
use crate::recorder_manager::ClipRangeParams;
use crate::subtitle_generator::{item_to_srt, SubtitleGeneratorType};
use crate::task::{Task, TaskPriority};
use crate::webhook::events;
use base64::Engine;
use chrono::{Local, Utc};
use recorder::platforms::bilibili;
use recorder::platforms::bilibili::profile::Profile;
use serde_json::json;
use std::fs::File;
use std::future::Future;
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
            reporter
                .update(&format!("正在复制视频文件... {percent}%"))
                .await;
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
        reporter
            .update("检测到网络文件，使用先复制后转换策略...")
            .await;
        copy_then_convert_strategy(source, dest, reporter).await
    } else {
        // 本地文件：直接转换（更高效）
        reporter.update("检测到本地文件，使用直接转换策略...").await;
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
    reporter
        .update("第1步：从网络复制文件到本地临时位置...")
        .await;
    copy_file_with_network_optimization(source, &temp_path, reporter).await?;

    // 第二步：从本地临时文件转换到目标位置
    reporter.update("第2步：从临时文件转换到目标格式...").await;
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
                    reporter
                        .update(&format!(
                            "正在从网络复制文件... {}% ({:.1}MB/{:.1}MB)",
                            percent,
                            copied as f64 / (1024.0 * 1024.0),
                            total_size as f64 / (1024.0 * 1024.0)
                        ))
                        .await;
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
                reporter
                    .update(&format!(
                        "网络连接中断，正在重试... ({consecutive_errors}/{MAX_RETRIES})"
                    ))
                    .await;
            }
        }
    }

    dest_file
        .flush()
        .map_err(|e| format!("刷新临时文件缓冲区失败: {e}"))?;
    reporter.update("网络文件复制完成").await;
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
        let cwd =
            std::env::current_dir().map_err(|e| format!("Failed to get current directory: {e}"))?;
        output = cwd.join(output);
    }

    if let Ok(disk_info) = get_disk_info_inner(output).await {
        // if free space is less than 1GB, return error
        if disk_info.free < 1024 * 1024 * 1024 {
            return Err("Storage space is not enough, clip canceled".to_string());
        }
    }

    let reporter = ProgressReporter::new(state.db.clone(), &state.events, &event_id).await?;
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

    #[cfg(feature = "gui")]
    let state_clone = (*state).clone();
    #[cfg(feature = "headless")]
    let state_clone = state.clone();

    let task_id = event_id.clone();
    run_queued(
        &state,
        &event_id,
        async move {
            match clip_range_inner(&state_clone, &reporter, params).await {
                Ok(video) => {
                    reporter.finish(true, "切片完成").await;
                    let _ = state_clone
                        .db
                        .update_task(&task_id, "success", "切片完成", None)
                        .await;

                    if state_clone.config.read().await.auto_subtitle {
                        let subtitle_event_id = format!("{task_id}_subtitle");
                        let result = generate_video_subtitle_inner(
                            &state_clone,
                            subtitle_event_id,
                            video.id,
                        )
                        .await;
                        match result {
                            Ok(subtitle) => {
                                let result =
                                    update_video_subtitle_inner(&state_clone, video.id, subtitle)
                                        .await;
                                if let Err(e) = result {
                                    log::error!("Update video subtitle error: {e}");
                                }
                            }
                            Err(e) => log::error!("Generate video subtitle error: {e}"),
                        }
                    }

                    let event = events::new_webhook_event(
                        events::CLIP_GENERATED,
                        events::Payload::Clip(video.clone()),
                    );

                    if let Err(e) = state_clone.webhook_poster.post_event(&event).await {
                        log::error!("Post webhook event error: {e}");
                    }

                    Ok(video)
                }
                Err(e) => {
                    reporter.finish(false, &format!("切片失败: {e}")).await;
                    let _ = state_clone
                        .db
                        .update_task(&task_id, "failed", &format!("切片失败: {e}"), None)
                        .await;
                    Err(e)
                }
            }
        },
        "切片任务已取消",
        "切片任务失败",
    )
    .await
}

/// Run `job` on the task queue and wait for its result, so the request can be
/// cancelled from the task list like any other queued task.
async fn run_queued<T: Send + 'static>(
    state: &State,
    task_id: &str,
    job: impl Future<Output = Result<T, String>> + Send + 'static,
    cancelled_message: &str,
    failed_message: &str,
) -> Result<T, String> {
    let (result_tx, result_rx) = tokio::sync::oneshot::channel();
    let enqueue_result = state
        .task_manager
        .add_task(Task::new(
            task_id.to_string(),
            TaskPriority::Normal,
            async move {
                let result = job.await;
                let task_result = result.as_ref().map(|_| ()).map_err(Clone::clone);
                let _ = result_tx.send(result);
                task_result
            },
        ))
        .await;
    if let Err(error) = enqueue_result {
        // The caller already recorded the task; never leave it pending with no
        // worker that could run or cancel it.
        state
            .db
            .update_task(task_id, "failed", &format!("加入队列失败: {error}"), None)
            .await?;
        return Err(error);
    }

    match result_rx.await {
        Ok(result) => result,
        // The worker dropped without a result: cancelled through the task list,
        // or an unexpected exit, which must remain a visible failure.
        Err(_) => {
            let cancelled = state
                .db
                .get_task(task_id)
                .await
                .is_ok_and(|task| task.status == "cancelled");
            Err(if cancelled {
                cancelled_message
            } else {
                failed_message
            }
            .to_string())
        }
    }
}

async fn clip_range_inner(
    state: &State,
    reporter: &ProgressReporter,
    params: ClipRangeParams,
) -> Result<VideoRow, String> {
    log::info!(
        "[{}]Clip room_id: {}, ts: {}, ranges: {:?}",
        reporter.event_id,
        params.room_id,
        params.live_id,
        params.ranges,
    );

    let clip_file = state.config.read().await.generate_clip_name(&params);

    let file = state
        .recorder_manager
        .clip_range(Some(reporter), clip_file, &params)
        .await?;
    log::info!("Clip range done, doing post processing");
    reporter.update("正在保存切片").await;
    // get file metadata from fs
    let metadata = std::fs::metadata(&file).map_err(|e| {
        log::error!("Get file metadata error: {} {}", e, file.display());
        e.to_string()
    })?;
    let mut cover_generate_ffmpeg = true;
    let cover_file = file.with_extension("jpg");
    if !params.cover.is_empty() {
        if let Some(base64) = params.cover.split("base64,").nth(1) {
            if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(base64) {
                // write cover file to fs
                tokio::fs::write(&cover_file, bytes).await.map_err(|e| {
                    log::error!("Write cover file error: {} {}", e, cover_file.display());
                    e.to_string()
                })?;
                cover_generate_ffmpeg = false;
            } else {
                log::error!("Decode base64 error: {}", params.cover);
            }
        } else {
            log::error!("Invalid cover base64: {}", params.cover);
        }
    }
    // generate cover file from video as fallback
    if cover_generate_ffmpeg {
        ffmpeg::generate_thumbnail(&file, 0.0).await?;
    }
    let _ = crate::ffmpeg::extract_audio_sample(&file).await?;
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
    let duration = params.ranges.iter().map(|r| r.duration()).sum::<f64>();
    let cover_name = cover_file
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("Invalid cover path: {}", cover_file.display()))?
        .to_string();
    let video = state
        .db
        .add_video(&VideoRow {
            id: 0,
            status: 0,
            room_id: params.room_id.clone(),
            created_at: Local::now().to_rfc3339(),
            cover: cover_name,
            file: filename.into(),
            note: params.note.clone(),
            length: duration as i64,
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
                params.room_id, duration, filename
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
            .unwrap_or_else(|e| log::warn!("Failed to show clip notification: {e}"));
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
    profile: Profile,
) -> Result<String, String> {
    let reporter = ProgressReporter::new(state.db.clone(), &state.events, &event_id).await?;
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
    match upload_procedure_inner(&state, &reporter, uid, room_id, video_id, profile).await {
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

    let cover_path = file.with_extension("jpg");
    let cover_bytes = tokio::fs::read(&cover_path).await.map_err(|e| {
        log::error!("Read cover file error: {} {}", e, cover_path.display());
        e.to_string()
    })?;
    let cover_base64 = format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(cover_bytes)
    );
    let cover_url =
        bilibili::api::upload_cover(&client, &account.to_account(), &cover_base64).await;

    reporter.update("投稿预处理中").await;

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
                        .unwrap_or_else(|e| log::warn!("Failed to show post notification: {e}"));
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
pub async fn cancel(state: state_type!(), event_id: String) -> Result<(), String> {
    log::info!("Cancel task: {event_id}");
    let task_before_cancel = state.db.get_task(&event_id).await.ok();
    let cancel_result = state
        .task_manager
        .cancel_task_if(&event_id, async {
            Ok(state.db.cancel_task_if_active(&event_id).await?)
        })
        .await;
    match cancel_result {
        Ok(true) => {}
        Ok(false) => return Ok(()),
        Err(e) if e == "Task not found" => {
            let task = state.db.get_task(&event_id).await?;
            if matches!(task.status.as_str(), "pending" | "processing") {
                return Err("任务尚未进入队列或不支持取消，请稍后重试".to_string());
            }
            return Ok(());
        }
        Err(e) => return Err(e),
    }
    if let Some(task) = task_before_cancel {
        if task.task_type == "generate_archive_summary" {
            if let Ok(metadata) = serde_json::from_str::<serde_json::Value>(&task.metadata) {
                if let (Some(platform), Some(room_id), Some(live_id)) = (
                    metadata.get("platform").and_then(|value| value.as_str()),
                    metadata.get("room_id").and_then(|value| value.as_str()),
                    metadata.get("live_id").and_then(|value| value.as_str()),
                ) {
                    let _ = state
                        .db
                        .fail_record_summary(platform, room_id, live_id, "任务已取消")
                        .await;
                }
            }
        }
    }
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
    let opus_path = file.with_extension("opus");
    let _ = tokio::fs::remove_file(opus_path).await;
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
    let base64 = cover
        .split("base64,")
        .nth(1)
        .ok_or_else(|| "Cover is not a base64 data URL".to_string())?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(base64)
        .map_err(|e| format!("Failed to decode cover: {e}"))?;
    tokio::fs::write(&cover_path, bytes)
        .await
        .map_err(|e| e.to_string())?;
    let cover_file_name = cover_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("Invalid cover path: {}", cover_path.display()))?;
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
    generate_video_subtitle_inner(&state, event_id, id).await
}

async fn generate_video_subtitle_inner(
    state: &State,
    event_id: String,
    id: i64,
) -> Result<String, String> {
    let reporter = ProgressReporter::new(state.db.clone(), &state.events, &event_id).await?;
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
    let generator_type = SubtitleGeneratorType::parse(&config.subtitle_generator_type)?;
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
        &state.resource_dir,
        &whisper_model,
        &whisper_prompt,
        &openai_api_key,
        &openai_api_endpoint,
        language_hint,
        Some(&state.db),
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

            let result = update_video_subtitle_inner(state, id, subtitle.clone()).await;
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
    update_video_subtitle_inner(&state, id, subtitle).await
}

async fn update_video_subtitle_inner(
    state: &State,
    id: i64,
    subtitle: String,
) -> Result<(), String> {
    let video = state.db.get_video(id).await?;
    let filepath = Path::new(state.config.read().await.output.as_str()).join(&video.file);
    let file = Path::new(&filepath);
    let subtitle_path = file.with_extension("srt");
    std::fs::write(&subtitle_path, subtitle).map_err(|e| {
        log::error!("Update video subtitle error: {e}");
        format!("写入字幕文件失败：{} ({e})", subtitle_path.display())
    })?;
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
    ass_content: Option<String>,
) -> Result<TaskRow, String> {
    let video = state.db.get_video(id).await?;
    let (filepath, cache) = {
        let config = state.config.read().await;
        (
            Path::new(&config.output).join(&video.file),
            PathBuf::from(&config.cache),
        )
    };
    // Freeze both ASS and legacy SRT inputs at submission time. Later edits
    // must not change a job that is waiting in the queue.
    let subtitle =
        prepare_subtitle_encoding_input(&filepath, srt_style.clone(), ass_content.as_deref())?;
    let font_name = ass_content
        .as_deref()
        .map(ass_primary_font_name)
        .transpose()?;
    let reporter = ProgressReporter::new(state.db.clone(), &state.events, &event_id).await?;
    let task = TaskRow {
        id: event_id.clone(),
        task_type: "encode_video_subtitle".to_string(),
        status: "pending".to_string(),
        message: "等待字幕压制".to_string(),
        metadata: json!({
            "video_id": id,
            "srt_style": srt_style,
            "subtitle_format": if ass_content.is_some() { "ass" } else { "srt" },
        })
        .to_string(),
        created_at: Utc::now().to_rfc3339(),
    };
    state.db.add_task(&task).await?;
    log::info!("Create task: {task:?}");
    #[cfg(feature = "gui")]
    let state_clone = (*state).clone();
    #[cfg(feature = "headless")]
    let state_clone = state.clone();

    let task_id = event_id.clone();
    let enqueue_result = state
        .task_manager
        .add_task(Task::new(
            task_id.clone(),
            TaskPriority::Normal,
            async move {
                reporter.update("准备字幕压制").await;
                let result = encode_video_subtitle_inner(
                    &state_clone,
                    &reporter,
                    SubtitleEncodingJob {
                        video,
                        filepath,
                        cache,
                        subtitle,
                        font_name,
                    },
                )
                .await;
                let (status, message) = match &result {
                    Ok(_) => ("success", "字幕编码完成".to_string()),
                    Err(error) => ("failed", format!("字幕编码失败: {error}")),
                };
                state_clone
                    .db
                    .update_task(&task_id, status, &message, None)
                    .await?;
                reporter.finish(result.is_ok(), &message).await;
                result.map(|_| ())
            },
        ))
        .await;
    if let Err(error) = enqueue_result {
        state
            .db
            .update_task(
                &event_id,
                "failed",
                &format!("加入压制队列失败: {error}"),
                None,
            )
            .await?;
        return Err(error);
    }
    Ok(task)
}

const MAX_ASS_DOCUMENT_BYTES: usize = 8 * 1024 * 1024;

fn validate_ass_document(content: &str) -> Result<(), String> {
    if content.len() > MAX_ASS_DOCUMENT_BYTES {
        return Err("ASS 字幕文件不能超过 8 MiB".to_string());
    }
    if content.contains('\0') {
        return Err("ASS 字幕文件不能包含 NUL 字符".to_string());
    }

    let mut script_info = false;
    let mut styles = false;
    let mut events = false;
    let mut style = false;
    let mut dialogue = false;
    let mut section = "";
    for line in content.trim_start_matches('\u{feff}').lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            section = line;
            script_info |= section.eq_ignore_ascii_case("[Script Info]");
            styles |= section.eq_ignore_ascii_case("[V4+ Styles]");
            events |= section.eq_ignore_ascii_case("[Events]");
        } else if let Some((key, value)) = line.split_once(':') {
            if section.eq_ignore_ascii_case("[V4+ Styles]") {
                style |= key.eq_ignore_ascii_case("Style") && !value.trim().is_empty();
            } else if section.eq_ignore_ascii_case("[Events]") {
                dialogue |= key.eq_ignore_ascii_case("Dialogue") && !value.trim().is_empty();
            }
        }
    }
    if !script_info || !styles || !events || !style || !dialogue {
        return Err(
            "ASS 字幕需要包含 [Script Info]、[V4+ Styles]、[Events] 以及有效的 Style 和 Dialogue 行"
                .to_string(),
        );
    }
    Ok(())
}

fn ass_primary_font_name(content: &str) -> Result<String, String> {
    let mut in_styles = false;
    let mut font_index = None;
    for line in content.trim_start_matches('\u{feff}').lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_styles = line.eq_ignore_ascii_case("[V4+ Styles]");
            continue;
        }
        if !in_styles {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if key.eq_ignore_ascii_case("Format") {
            font_index = value
                .split(',')
                .position(|field| field.trim().eq_ignore_ascii_case("Fontname"));
        } else if key.eq_ignore_ascii_case("Style") {
            if let Some(index) = font_index {
                if let Some(font) = value.split(',').nth(index).map(str::trim) {
                    if !font.is_empty() {
                        return Ok(font.to_string());
                    }
                }
            }
        }
    }
    Err("ASS 字幕样式需要包含 Format 定义和有效的 Fontname 字段".to_string())
}

/// Keep a snapshot of the submitted subtitles alive until the job finishes.
struct SubtitleEncodingInput {
    path: PathBuf,
    force_style: String,
    _temporary_subtitle: tempfile::TempPath,
}

struct SubtitleEncodingJob {
    video: VideoRow,
    filepath: PathBuf,
    cache: PathBuf,
    subtitle: SubtitleEncodingInput,
    font_name: Option<String>,
}

fn prepare_subtitle_encoding_input(
    video_path: &Path,
    srt_style: String,
    ass_content: Option<&str>,
) -> Result<SubtitleEncodingInput, String> {
    let (content, extension, force_style) = if let Some(content) = ass_content {
        validate_ass_document(content)?;
        (content.to_string(), ".ass", String::new())
    } else {
        let source = video_path.with_extension("srt");
        let content = std::fs::read_to_string(&source)
            .map_err(|error| format!("字幕文件不可用：{} ({error})", source.display()))?;
        if content.trim().is_empty() {
            return Err(format!("字幕文件为空：{}", source.display()));
        }
        (content, ".srt", srt_style)
    };
    let mut file = tempfile::Builder::new()
        .prefix("bili-shadowreplay-subtitle-")
        .suffix(extension)
        .tempfile()
        .map_err(|error| format!("创建临时字幕文件失败：{error}"))?;
    file.write_all(content.as_bytes())
        .map_err(|error| format!("写入临时字幕文件失败：{error}"))?;
    let temporary_subtitle = file.into_temp_path();
    Ok(SubtitleEncodingInput {
        path: temporary_subtitle.to_path_buf(),
        // The preview and burn-in must use the exact same ASS document. Applying
        // legacy SRT overrides here would silently change the document's styles.
        force_style,
        _temporary_subtitle: temporary_subtitle,
    })
}

async fn encode_video_subtitle_inner(
    state: &State,
    reporter: &ProgressReporter,
    job: SubtitleEncodingJob,
) -> Result<VideoRow, String> {
    let SubtitleEncodingJob {
        video,
        filepath,
        cache,
        subtitle,
        font_name,
    } = job;
    // Kept alive until FFmpeg exits; dropping it removes the directory.
    let fonts_directory = if let Some(font_name) = font_name {
        Some(prepare_subtitle_font_directory(cache, font_name).await?)
    } else {
        None
    };
    let output_filename = ffmpeg::encode_video_subtitle(
        reporter,
        &filepath,
        &subtitle.path,
        subtitle.force_style.clone(),
        fonts_directory.as_ref().map(|directory| directory.path()),
    )
    .await?;
    let output_filepath = filepath.with_file_name(&output_filename);
    let cover_path = filepath.with_extension("jpg");
    let output_cover_path = output_filepath.with_extension("jpg");

    if cover_path.exists() {
        tokio::fs::copy(&cover_path, &output_cover_path)
            .await
            .map_err(|e| {
                format!(
                    "复制字幕视频封面失败: {} -> {}: {e}",
                    cover_path.display(),
                    output_cover_path.display()
                )
            })?;
    } else {
        log::warn!(
            "Cover file not found for subtitle video: {}",
            cover_path.display()
        );
    }

    let new_video = state
        .db
        .add_video(&VideoRow {
            id: 0,
            status: video.status,
            room_id: video.room_id,
            created_at: Local::now().to_rfc3339(),
            cover: if output_cover_path.exists() {
                output_cover_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default()
                    .to_string()
            } else {
                String::new()
            },
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
    let reporter = ProgressReporter::new(state.db.clone(), &state.events, &event_id).await?;

    let source_path = Path::new(&file_path);
    if !source_path.exists() {
        return Err("文件不存在".to_string());
    }

    reporter.update("正在提取视频元数据...").await;
    let metadata = ffmpeg_utils::extract_video_metadata(source_path).await?;
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

        reporter.update("准备转换视频格式 (FLV → MP4)...").await;

        copy_and_convert_with_progress(source_path, &mp4_target_full_path, true, &reporter).await?;

        // 更新最终文件名和路径
        target_filename = mp4_target_full_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| format!("Invalid target path: {}", mp4_target_full_path.display()))?
            .to_string();
        mp4_target_full_path
    } else {
        // 其他格式使用智能拷贝
        copy_and_convert_with_progress(source_path, &target_full_path, false, &reporter).await?;
        target_full_path
    };

    // 步骤3: 生成缩略图
    reporter.update("正在生成视频缩略图...").await;

    // 生成缩略图，使用智能时间点选择
    let thumbnail_timestamp = get_optimal_thumbnail_timestamp(metadata.duration);
    let cover_path =
        match ffmpeg::generate_thumbnail(&final_target_full_path, thumbnail_timestamp).await {
            Ok(path) => path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| format!("Invalid thumbnail path: {}", path.display()))?
                .to_string(),
            Err(e) => {
                log::warn!("生成缩略图失败: {e}");
                String::new() // 使用空字符串，前端会显示默认图标
            }
        };

    // 步骤4: 保存到数据库
    reporter.update("正在保存视频信息...").await;

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

    let reporter = ProgressReporter::new(state.db.clone(), &state.events, &event_id).await?;

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

    #[cfg(feature = "gui")]
    let state_clone = (*state).clone();
    #[cfg(feature = "headless")]
    let state_clone = state.clone();

    let task_id = event_id.clone();
    run_queued(
        &state,
        &task_id,
        async move {
            match clip_video_inner(
                &state_clone,
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
                    state_clone
                        .db
                        .update_task(&event_id, "success", "切片完成", None)
                        .await?;
                    Ok(video)
                }
                Err(e) => {
                    reporter.finish(false, &format!("切片失败: {e}")).await;
                    state_clone
                        .db
                        .update_task(&event_id, "failed", &format!("切片失败: {e}"), None)
                        .await?;
                    Err(e)
                }
            }
        },
        "切片任务已取消",
        "切片任务失败",
    )
    .await
}

async fn clip_video_inner(
    state: &State,
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
    reporter.update("开始切片处理").await;
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
                .and_then(|name| name.to_str())
                .ok_or_else(|| {
                    format!("Invalid thumbnail path: {}", thumbnail_full_path.display())
                })?
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
    let batch_reporter = ProgressReporter::new(state.db.clone(), &state.events, &event_id).await?;

    let total_files = file_paths.len();

    for (index, file_path) in file_paths.iter().enumerate() {
        let current_index = index + 1;
        let file_name = Path::new(file_path)
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("unknown")
            .to_string();

        // 更新批量进度，只显示进度信息
        batch_reporter
            .update(&format!(
                "正在导入第{current_index}个，共{total_files}个文件"
            ))
            .await;

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

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn generate_audio_sample(state: state_type!(), video_id: i64) -> Result<(), String> {
    let video = state.db.get_video(video_id).await?;
    let video_path = Path::new(&state.config.read().await.output).join(&video.file);
    let opus_path = video_path.with_extension("opus");
    if !opus_path.exists() {
        let _ = crate::ffmpeg::extract_audio_sample(&video_path).await?;
    }
    Ok(())
}

#[cfg(test)]
mod subtitle_encoding_tests {
    use super::*;

    const ASS: &str = "[Script Info]\nScriptType: v4.00+\nPlayResX: 1920\nPlayResY: 1080\n\
        [V4+ Styles]\nStyle: Default,Arial,48,&H00FFFFFF\n\
        [Events]\nDialogue: 0,0:00:00.00,0:00:01.00,Default,,0,0,0,,字幕\\N第二行\n";

    #[test]
    fn legacy_srt_input_snapshots_content_and_style_and_removes_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let video = dir.path().join("clip.mp4");
        let source = video.with_extension("srt");
        let original = "1\n00:00:00,000 --> 00:00:01,000\nOriginal\n";
        std::fs::write(&source, original).unwrap();
        let path;
        {
            let input = prepare_subtitle_encoding_input(
                &video,
                "FontName=Arial,FontSize=24".to_string(),
                None,
            )
            .unwrap();
            path = input.path.clone();
            std::fs::write(&source, "Later edits").unwrap();
            assert_ne!(input.path, source);
            assert_eq!(std::fs::read_to_string(&input.path).unwrap(), original);
            assert_eq!(input.force_style, "FontName=Arial,FontSize=24");
        }
        assert!(!path.exists());
        assert_eq!(std::fs::read_to_string(source).unwrap(), "Later edits");
    }

    #[test]
    fn legacy_srt_rejects_missing_and_empty_input_before_enqueuing() {
        let dir = tempfile::tempdir().unwrap();
        let video = dir.path().join("clip.mp4");
        assert!(prepare_subtitle_encoding_input(&video, String::new(), None).is_err());
        std::fs::write(video.with_extension("srt"), " \n").unwrap();
        assert!(prepare_subtitle_encoding_input(&video, String::new(), None).is_err());
    }

    #[test]
    fn ass_input_is_preserved_without_srt_overrides_and_removed_after_use() {
        let path;
        {
            let input = prepare_subtitle_encoding_input(
                Path::new("/videos/clip.mp4"),
                "FontName=Other,FontSize=99".to_string(),
                Some(ASS),
            )
            .unwrap();
            path = input.path.clone();
            assert_eq!(path.extension().unwrap(), "ass");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), ASS);
            assert!(input.force_style.is_empty());
        }
        assert!(!path.exists());
    }

    #[test]
    fn ass_validation_rejects_missing_sections_empty_events_nul_and_oversized_documents() {
        assert!(validate_ass_document(ASS).is_ok());
        assert!(validate_ass_document(&format!("\u{feff}{ASS}")).is_ok());
        for invalid in [
            "",
            "[Script Info]\n[V4+ Styles]\n[Events]",
            "[Script Info]\n[V4+ Styles]\nStyle: Default,Arial\n[Events]",
            "[Script Info]\n[V4+ Styles]\n[Events]\nStyle: Default,Arial\nDialogue: example",
        ] {
            assert!(validate_ass_document(invalid).is_err(), "{invalid}");
        }
        assert!(validate_ass_document(&format!("{ASS}\0")).is_err());
        assert!(validate_ass_document(&" ".repeat(MAX_ASS_DOCUMENT_BYTES + 1)).is_err());
    }

    #[test]
    fn ass_font_name_uses_style_format_order_and_ignores_other_sections() {
        let standard = "[Script Info]\nFontname: Wrong\n[V4+ Styles]\nFormat: Name, Fontname, Fontsize\nStyle: Default, Microsoft YaHei, 48\n[Events]\n";
        assert_eq!(ass_primary_font_name(standard).unwrap(), "Microsoft YaHei");
        let reordered =
            "[V4+ Styles]\nFormat: Fontsize,Name,Fontname\nStyle: 48,Default,思源黑体\n";
        assert_eq!(ass_primary_font_name(reordered).unwrap(), "思源黑体");
        for invalid in [
            "[V4+ Styles]\nStyle: Default,Arial,48",
            "[V4+ Styles]\nFormat: Name,Fontsize\nStyle: Default,48",
            "[V4+ Styles]\nFormat: Name,Fontname\nStyle: Default, ",
        ] {
            assert!(ass_primary_font_name(invalid).is_err());
        }
    }
}

#[cfg(all(test, feature = "headless"))]
mod queued_subtitle_tests {
    use super::*;
    use crate::{
        config::Config,
        database::{credentials::CredentialCipher, Database},
        events::EventBus,
        mcp::McpServer,
        recorder_manager::RecorderManager,
        task::{TaskManager, TaskManagerConfig},
        webhook::poster::{WebhookConfig, WebhookPoster},
    };
    use std::sync::Arc;
    use tokio::sync::RwLock;

    async fn fixture(dir: &Path, capacity: usize, start_scheduler: bool) -> State {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::raw_sql(
            "CREATE TABLE videos (
                id INTEGER PRIMARY KEY, room_id TEXT, cover TEXT, file TEXT,
                note TEXT, length INTEGER, size INTEGER, status INTEGER, bvid TEXT,
                title TEXT, desc TEXT, tags TEXT, area INTEGER, created_at TEXT, platform TEXT
            );
            CREATE TABLE tasks (
                id TEXT PRIMARY KEY, type TEXT, status TEXT, message TEXT,
                metadata TEXT, created_at TEXT
            );
            INSERT INTO videos VALUES (1, 'room', '', 'clip.mp4', '', 1, 0, 0, '', '', '', '', 0, '', 'bilibili');",
        ).execute(&pool).await.unwrap();
        std::fs::write(
            dir.join("clip.srt"),
            "1\n00:00:00,000 --> 00:00:01,000\nOriginal\n",
        )
        .unwrap();
        let events = EventBus::new();
        let db = Arc::new(Database::new(
            pool,
            CredentialCipher::new(&[0; 32]).unwrap(),
            events.clone(),
        ));
        let config: Config = serde_json::from_value(json!({
            "output": dir, "cache": dir,
            "live_start_notify": false, "live_end_notify": false,
            "clip_notify": false, "post_notify": false,
        }))
        .unwrap();
        let config = Arc::new(RwLock::new(config));
        let mut task_manager = TaskManager::with_config(TaskManagerConfig {
            max_queue_size: capacity,
            check_interval_ms: 5,
            ..Default::default()
        });
        if start_scheduler {
            task_manager.start();
        }
        let task_manager = Arc::new(task_manager);
        let webhook_poster = WebhookPoster::new(WebhookConfig::default()).unwrap();
        let recorder_manager = Arc::new(RecorderManager::new(
            events.clone(),
            db.clone(),
            config.clone(),
            task_manager.clone(),
            dir.to_path_buf(),
            webhook_poster.clone(),
        ));
        State {
            mcp_server: Arc::new(McpServer::default()),
            db,
            config,
            webhook_poster,
            recorder_manager,
            task_manager,
            resource_dir: dir.to_path_buf(),
            events,
            readonly: false,
        }
    }

    #[tokio::test]
    async fn consecutive_submissions_return_pending_jobs_and_can_be_cancelled() {
        let dir = tempfile::tempdir().unwrap();
        // Submission must return even when the scheduler is stopped.
        let state = fixture(dir.path(), 2, false).await;
        for id in ["first", "second"] {
            let task = tokio::time::timeout(
                std::time::Duration::from_secs(1),
                encode_video_subtitle(state.clone(), id.into(), 1, String::new(), None),
            )
            .await
            .expect("submission must return without running the job")
            .unwrap();
            assert_eq!(task.id, id);
            assert_eq!(task.status, "pending");
            assert_eq!(state.db.get_task(id).await.unwrap().status, "pending");
        }
        assert_eq!(state.task_manager.queue_size().await, 2);
        cancel(state.clone(), "second".into()).await.unwrap();
        assert_eq!(state.task_manager.queue_size().await, 1);
        assert_eq!(
            state.db.get_task("second").await.unwrap().status,
            "cancelled"
        );
        // Repeated cancellation is harmless; it must not affect the first job.
        cancel(state.clone(), "second".into()).await.unwrap();
        assert_eq!(state.task_manager.queue_size().await, 1);
        assert_eq!(state.db.get_task("first").await.unwrap().status, "pending");
        cancel(state, "first".into()).await.unwrap();
    }

    #[tokio::test]
    async fn queue_full_marks_submission_failed_instead_of_leaving_it_pending() {
        let dir = tempfile::tempdir().unwrap();
        let state = fixture(dir.path(), 1, false).await;
        encode_video_subtitle(state.clone(), "first".into(), 1, String::new(), None)
            .await
            .unwrap();
        assert_eq!(
            encode_video_subtitle(state.clone(), "overflow".into(), 1, String::new(), None)
                .await
                .unwrap_err(),
            "Queue is full"
        );
        assert_eq!(
            state.db.get_task("overflow").await.unwrap().status,
            "failed"
        );
        assert_eq!(state.task_manager.queue_size().await, 1);
        cancel(state, "first".into()).await.unwrap();
    }

    #[tokio::test]
    async fn cancelling_pending_and_running_tasks_closes_result_without_failure_progress() {
        use crate::{
            events::{AppEvent, DataChange},
            task::TaskStatus,
        };
        use tokio::{
            sync::oneshot,
            time::{timeout, Duration},
        };

        for running in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let state = fixture(dir.path(), 1, running).await;
            let task = state
                .db
                .generate_task("clip_range", "waiting", "{}")
                .await
                .unwrap();
            let (started_tx, started_rx) = oneshot::channel();
            let (result_tx, result_rx) = oneshot::channel();
            state
                .task_manager
                .add_task(Task::new(
                    task.id.clone(),
                    TaskPriority::Normal,
                    async move {
                        started_tx.send(()).unwrap();
                        let result = std::future::pending::<Result<(), String>>().await;
                        let _ = result_tx.send(result);
                        Ok(())
                    },
                ))
                .await
                .unwrap();
            if running {
                timeout(Duration::from_secs(1), started_rx)
                    .await
                    .unwrap()
                    .unwrap();
            }
            let mut events = state.events.subscribe();

            cancel(state.clone(), task.id.clone()).await.unwrap();

            assert_eq!(
                state.db.get_task(&task.id).await.unwrap().status,
                "cancelled"
            );
            assert_eq!(
                state.task_manager.get_task_status(&task.id).await,
                Some(TaskStatus::Cancelled)
            );
            assert!(timeout(Duration::from_secs(1), result_rx)
                .await
                .expect("dropping the task must unblock its result receiver")
                .is_err());
            assert!(matches!(
                timeout(Duration::from_secs(1), events.recv())
                    .await
                    .unwrap()
                    .unwrap(),
                AppEvent::DataChanged(DataChange::Tasks)
            ));
            assert!(
                timeout(Duration::from_millis(10), events.recv())
                    .await
                    .is_err(),
                "cancellation must refresh tasks without publishing failure progress"
            );
        }
    }

    #[tokio::test]
    async fn clip_request_distinguishes_cancelled_tasks_from_unexpected_worker_drops() {
        use tokio::time::{timeout, Duration};

        for cancelled in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let state = fixture(dir.path(), 1, false).await;
            let request_state = state.clone();
            let request = tokio::spawn(async move {
                let params = serde_json::from_value(json!({
                    "title": "Clip", "note": "", "cover": "", "platform": "bilibili",
                    "room_id": "room", "live_id": "live", "ranges": [],
                    "danmu": false, "local_offset": 0, "fix_encoding": false,
                }))
                .unwrap();
                clip_range(request_state, "clip-request".into(), params).await
            });
            timeout(Duration::from_secs(3), async {
                while state.task_manager.queue_size().await == 0 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("clip request must enqueue before cancellation");

            if cancelled {
                cancel(state.clone(), "clip-request".into()).await.unwrap();
            } else {
                // Drop the worker without a persisted cancellation, as with an
                // unexpected worker exit. This must remain a visible failure.
                state
                    .task_manager
                    .cancel_task("clip-request")
                    .await
                    .unwrap();
            }

            let error = timeout(Duration::from_secs(1), request)
                .await
                .expect("clip request must unblock when its worker is dropped")
                .unwrap()
                .unwrap_err();
            assert_eq!(
                error,
                if cancelled {
                    "切片任务已取消"
                } else {
                    "切片任务失败"
                }
            );
        }
    }

    #[tokio::test]
    async fn queued_video_clip_can_be_cancelled_from_the_task_list() {
        use tokio::time::{timeout, Duration};

        let dir = tempfile::tempdir().unwrap();
        let state = fixture(dir.path(), 1, false).await;
        let request_state = state.clone();
        let request = tokio::spawn(async move {
            clip_video(
                request_state,
                "video-clip".into(),
                1,
                0.0,
                1.0,
                "Clip".into(),
            )
            .await
        });
        timeout(Duration::from_secs(3), async {
            while state.task_manager.queue_size().await == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("video clip must enqueue so the task list can cancel it");

        cancel(state.clone(), "video-clip".into()).await.unwrap();
        let error = timeout(Duration::from_secs(1), request)
            .await
            .expect("video clip request must unblock when cancelled")
            .unwrap()
            .unwrap_err();
        assert_eq!(error, "切片任务已取消");
    }

    #[tokio::test]
    async fn video_clip_rejected_by_a_full_queue_is_marked_failed() {
        use tokio::time::{timeout, Duration};

        let dir = tempfile::tempdir().unwrap();
        let state = fixture(dir.path(), 1, false).await;
        let first_state = state.clone();
        let first = tokio::spawn(async move {
            clip_video(first_state, "first".into(), 1, 0.0, 1.0, "Clip".into()).await
        });
        timeout(Duration::from_secs(3), async {
            while state.task_manager.queue_size().await == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("first video clip must occupy the queue");

        assert_eq!(
            clip_video(state.clone(), "overflow".into(), 1, 0.0, 1.0, "Clip".into())
                .await
                .unwrap_err(),
            "Queue is full"
        );
        assert_eq!(
            state.db.get_task("overflow").await.unwrap().status,
            "failed"
        );

        cancel(state.clone(), "first".into()).await.unwrap();
        timeout(Duration::from_secs(1), first)
            .await
            .expect("cancelled video clip must unblock")
            .unwrap()
            .unwrap_err();
    }

    #[tokio::test]
    async fn cancellation_preserves_committed_result_while_worker_is_still_running() {
        use crate::task::TaskStatus;
        use tokio::{
            sync::oneshot,
            time::{timeout, Duration},
        };

        for status in ["success", "failed"] {
            let dir = tempfile::tempdir().unwrap();
            let state = fixture(dir.path(), 1, true).await;
            let task = state
                .db
                .generate_task("encode_video_subtitle", "waiting", "{}")
                .await
                .unwrap();
            let task_id = task.id.clone();
            let worker_db = state.db.clone();
            let (committed_tx, committed_rx) = oneshot::channel();
            let (release_tx, release_rx) = oneshot::channel();
            state
                .task_manager
                .add_task(Task::new(
                    task.id.clone(),
                    TaskPriority::Normal,
                    async move {
                        worker_db
                            .update_task(&task_id, status, "finished", Some("{\"finished\":true}"))
                            .await
                            .unwrap();
                        committed_tx.send(()).unwrap();
                        // Hold the worker after the terminal DB commit, reproducing the
                        // window before the scheduler removes its running handle.
                        release_rx.await.unwrap();
                        if status == "failed" {
                            Err("encode failed".to_string())
                        } else {
                            Ok(())
                        }
                    },
                ))
                .await
                .unwrap();
            timeout(Duration::from_secs(1), committed_rx)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                state.task_manager.get_task_status(&task.id).await,
                Some(TaskStatus::Running)
            );
            let mut events = state.events.subscribe();

            cancel(state.clone(), task.id.clone()).await.unwrap();

            let row = state.db.get_task(&task.id).await.unwrap();
            assert_eq!(row.status, status);
            assert_eq!(row.message, "finished");
            assert_eq!(row.metadata, "{\"finished\":true}");
            assert_eq!(
                state.task_manager.get_task_status(&task.id).await,
                Some(TaskStatus::Running)
            );
            assert!(
                timeout(Duration::from_millis(10), events.recv())
                    .await
                    .is_err(),
                "ignored cancellation must not publish cancelled progress"
            );
            release_tx
                .send(())
                .expect("completed worker must not be aborted");
            let expected = if status == "failed" {
                TaskStatus::Failed("encode failed".into())
            } else {
                TaskStatus::Completed
            };
            timeout(Duration::from_secs(1), async {
                while state.task_manager.get_task_status(&task.id).await != Some(expected.clone()) {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
        }
    }

    #[tokio::test]
    async fn cancelling_an_unmanaged_task_does_not_claim_it_was_stopped() {
        let dir = tempfile::tempdir().unwrap();
        let state = fixture(dir.path(), 1, false).await;
        let task = state
            .db
            .generate_task("upload_procedure", "working", "{}")
            .await
            .unwrap();
        assert!(cancel(state.clone(), task.id.clone()).await.is_err());
        assert_eq!(state.db.get_task(&task.id).await.unwrap().status, "pending");
    }
}
