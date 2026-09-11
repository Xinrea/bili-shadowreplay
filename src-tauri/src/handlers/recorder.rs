use std::str::FromStr;

use crate::danmu2ass;
use crate::database::record::RecordRow;
use crate::database::recorder::RecorderRow;
use crate::database::task::TaskRow;
use crate::progress::progress_reporter::EventEmitter;
use crate::progress::progress_reporter::ProgressReporter;
use crate::progress::progress_reporter::ProgressReporterTrait;
use crate::recorder_manager::{GenerateWholeClipParams, RecorderList};
use crate::state::State;
use crate::state_type;
use crate::task::Task;
use crate::task::TaskPriority;
use crate::webhook::events;
use recorder::account::Account;
use recorder::danmu::DanmuEntry;
use recorder::platforms::bilibili;
use recorder::platforms::douyin;
use recorder::platforms::PlatformType;
use recorder::RecorderInfo;

#[cfg(feature = "gui")]
use tauri::State as TauriState;

use serde::Deserialize;
use serde::Serialize;

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_recorder_list(state: state_type!()) -> Result<RecorderList, ()> {
    Ok(state.recorder_manager.get_recorder_list().await)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn add_recorder(
    state: state_type!(),
    platform: String,
    room_id: String,
    mut extra: String,
) -> Result<RecorderRow, String> {
    log::info!("Add recorder: {platform} {room_id}");
    let platform = PlatformType::from_str(&platform).map_err(|e| e.to_string())?;
    let account = match platform {
        PlatformType::BiliBili => {
            if let Ok(account) = state.db.get_account_by_platform("bilibili").await {
                Ok(account.to_account())
            } else {
                log::error!("No available bilibili account found");
                Err("没有可用账号，请先添加账号".to_string())
            }
        }
        PlatformType::Douyin => {
            let client = reqwest::Client::new();
            let sec_uid = douyin::api::get_room_owner_sec_uid(&client, &room_id)
                .await
                .map_err(|e| e.to_string())?;
            extra = sec_uid;

            if let Ok(account) = state.db.get_account_by_platform("douyin").await {
                Ok(account.to_account())
            } else {
                log::error!("No available douyin account found");
                Err("没有可用账号，请先添加账号".to_string())
            }
        }
        PlatformType::Huya => {
            if let Ok(account) = state.db.get_account_by_platform("huya").await {
                Ok(account.to_account())
            } else {
                Ok(Account::default())
            }
        }
        PlatformType::Kuaishou => {
            if let Ok(account) = state.db.get_account_by_platform("kuaishou").await {
                Ok(account.to_account())
            } else {
                Ok(Account::default())
            }
        }
        PlatformType::TikTok => {
            if let Ok(account) = state.db.get_account_by_platform("tiktok").await {
                Ok(account.to_account())
            } else {
                Ok(Account::default())
            }
        }
        PlatformType::Xiaohongshu => {
            if let Ok(account) = state.db.get_account_by_platform("xiaohongshu").await {
                Ok(account.to_account())
            } else {
                Ok(Account::default())
            }
        }
        PlatformType::Weibo => {
            if let Ok(account) = state.db.get_account_by_platform("weibo").await {
                Ok(account.to_account())
            } else {
                Ok(Account::default())
            }
        }
        _ => Err("不支持的平台".to_string()),
    };

    match account {
        Ok(account) => match state
            .recorder_manager
            .add_recorder(&account, platform, &room_id, &extra, true)
            .await
        {
            Ok(()) => {
                let room = state.db.add_recorder(platform, &room_id, &extra).await?;
                state
                    .db
                    .new_message("添加直播间", &format!("添加了新直播间 {room_id}"))
                    .await?;
                // post webhook event
                let event = events::new_webhook_event(
                    events::RECORDER_ADDED,
                    events::Payload::Recorder(room.clone()),
                );
                if let Err(e) = state.webhook_poster.post_event(&event).await {
                    log::error!("Post webhook event error: {e}");
                }
                Ok(room)
            }
            Err(e) => {
                log::error!("Failed to add recorder: {e}");
                Err(format!("添加失败: {e}"))
            }
        },
        Err(e) => {
            log::error!("Failed to add recorder: {e}");
            Err(format!("添加失败: {e}"))
        }
    }
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn remove_recorder(
    state: state_type!(),
    platform: String,
    room_id: String,
) -> Result<(), String> {
    log::info!("Remove recorder: {platform} {room_id}");
    let platform = PlatformType::from_str(&platform).map_err(|e| e.to_string())?;
    match state
        .recorder_manager
        .remove_recorder(platform, &room_id)
        .await
    {
        Ok(recorder) => {
            state
                .db
                .new_message("移除直播间", &format!("移除了直播间 {room_id}"))
                .await?;
            // post webhook event
            let event = events::new_webhook_event(
                events::RECORDER_REMOVED,
                events::Payload::Recorder(recorder),
            );
            if let Err(e) = state.webhook_poster.post_event(&event).await {
                log::error!("Post webhook event error: {e}");
            }
            log::info!("Removed recorder: {} {}", platform.as_str(), room_id);
            Ok(())
        }
        Err(e) => {
            log::error!("Failed to remove recorder: {e}");
            Err(e.to_string())
        }
    }
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_room_info(
    state: state_type!(),
    platform: String,
    room_id: String,
) -> Result<RecorderInfo, String> {
    let platform = PlatformType::from_str(&platform).map_err(|e| e.to_string())?;
    if let Some(info) = state
        .recorder_manager
        .get_recorder_info(platform, &room_id)
        .await
    {
        Ok(info)
    } else {
        Err("Not found".to_string())
    }
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_archive_disk_usage(state: state_type!()) -> Result<i64, String> {
    Ok(state.recorder_manager.get_archive_disk_usage().await?)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_archives(
    state: state_type!(),
    room_id: String,
    offset: i64,
    limit: i64,
) -> Result<Vec<RecordRow>, String> {
    Ok(state
        .recorder_manager
        .get_archives(&room_id, offset, limit)
        .await?)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_archive(
    state: state_type!(),
    room_id: String,
    live_id: String,
) -> Result<RecordRow, String> {
    Ok(state
        .recorder_manager
        .get_archive(&room_id, &live_id)
        .await?)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_archives_by_parent_id(
    state: state_type!(),
    room_id: String,
    parent_id: String,
) -> Result<Vec<RecordRow>, String> {
    Ok(state
        .db
        .get_archives_by_parent_id(&room_id, &parent_id)
        .await?)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_archive_subtitle(
    state: state_type!(),
    platform: String,
    room_id: String,
    live_id: String,
) -> Result<String, String> {
    let platform = PlatformType::from_str(&platform)?;
    Ok(state
        .recorder_manager
        .get_archive_subtitle(platform, &room_id, &live_id)
        .await?)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn generate_archive_subtitle(
    state: state_type!(),
    platform: String,
    room_id: String,
    live_id: String,
) -> Result<String, String> {
    let platform = PlatformType::from_str(&platform)?;
    Ok(state
        .recorder_manager
        .generate_archive_subtitle(platform, &room_id, &live_id)
        .await?)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn delete_archive(
    state: state_type!(),
    platform: String,
    room_id: String,
    live_id: String,
) -> Result<(), String> {
    let platform = PlatformType::from_str(&platform)?;
    let to_delete = state
        .recorder_manager
        .delete_archive(platform, &room_id, &live_id)
        .await?;
    state
        .db
        .new_message(
            "删除历史缓存",
            &format!("删除了房间 {room_id} 的历史缓存 {live_id}"),
        )
        .await?;
    // post webhook event
    let event =
        events::new_webhook_event(events::ARCHIVE_DELETED, events::Payload::Archive(to_delete));
    if let Err(e) = state.webhook_poster.post_event(&event).await {
        log::error!("Post webhook event error: {e}");
    }
    Ok(())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn delete_archives(
    state: state_type!(),
    platform: String,
    room_id: String,
    live_ids: Vec<String>,
) -> Result<(), String> {
    let platform = PlatformType::from_str(&platform)?;
    let to_deletes = state
        .recorder_manager
        .delete_archives(
            platform,
            &room_id,
            &live_ids
                .iter()
                .map(std::string::String::as_str)
                .collect::<Vec<&str>>(),
        )
        .await?;
    state
        .db
        .new_message(
            "删除历史缓存",
            &format!("删除了房间 {} 的历史缓存 {}", room_id, live_ids.join(", ")),
        )
        .await?;
    for to_delete in to_deletes {
        // post webhook event
        let event =
            events::new_webhook_event(events::ARCHIVE_DELETED, events::Payload::Archive(to_delete));
        if let Err(e) = state.webhook_poster.post_event(&event).await {
            log::error!("Post webhook event error: {e}");
        }
    }
    Ok(())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_danmu_record(
    state: state_type!(),
    platform: String,
    room_id: String,
    live_id: String,
) -> Result<Vec<DanmuEntry>, String> {
    let platform = PlatformType::from_str(&platform)?;
    Ok(state
        .recorder_manager
        .load_danmus(platform, &room_id, &live_id)
        .await?)
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportDanmuOptions {
    platform: String,
    room_id: String,
    live_id: String,
    x: i64,
    y: i64,
    /// Archive start timestamp in seconds (PROGRAM-DATE-TIME). Used when the
    /// playlist start time cannot be read from cache.
    #[serde(default)]
    offset: f64,
    ass: bool,
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn export_danmu(
    state: state_type!(),
    options: ExportDanmuOptions,
) -> Result<String, String> {
    let platform = PlatformType::from_str(&options.platform)?;
    let danmus = state
        .recorder_manager
        .load_danmus(platform, &options.room_id, &options.live_id)
        .await?;

    log::debug!("First danmu entry: {:?}", danmus.first());

    let stream_start_ms = match state
        .recorder_manager
        .first_segment_timestamp(platform, &options.room_id, &options.live_id)
        .await
    {
        Ok(ts) => ts,
        Err(e) => {
            if options.offset != 0.0 {
                (options.offset * 1000.0).round() as i64
            } else {
                return Err(e.to_string());
            }
        }
    };

    let danmus = prepare_danmus_for_export(danmus, stream_start_ms, options.x, options.y);

    if options.ass {
        Ok(danmu2ass::danmu_to_ass(
            danmus,
            danmu2ass::Danmu2AssOptions::default(),
        ))
    } else {
        // map and join entries
        Ok(danmus
            .iter()
            .map(|e| format!("{}:{}", e.ts, e.content))
            .collect::<Vec<_>>()
            .join("\n"))
    }
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn send_danmaku(
    state: state_type!(),
    uid: String,
    room_id: String,
    message: String,
) -> Result<(), String> {
    let account = state.db.get_account("bilibili", &uid).await?;
    let client = reqwest::Client::new();
    match bilibili::api::send_danmaku(&client, &account.to_account(), &room_id, &message).await {
        Ok(()) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_total_length(state: state_type!()) -> Result<f64, String> {
    match state.db.get_total_length().await {
        Ok(total_length) => Ok(total_length),
        Err(e) => Err(format!("Failed to get total length: {e}")),
    }
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_today_record_count(state: state_type!()) -> Result<i64, String> {
    match state.db.get_today_record_count().await {
        Ok(count) => Ok(count),
        Err(e) => Err(format!("Failed to get today record count: {e}")),
    }
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_recent_record(
    state: state_type!(),
    room_id: String,
    offset: i64,
    limit: i64,
) -> Result<Vec<RecordRow>, String> {
    match state.db.get_recent_record(&room_id, offset, limit).await {
        Ok(records) => Ok(records),
        Err(e) => Err(format!("Failed to get recent record: {e}")),
    }
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn set_enable(
    state: state_type!(),
    platform: String,
    room_id: String,
    enabled: bool,
) -> Result<(), String> {
    log::info!("Set enable for recorder {platform} {room_id} {enabled}");
    let platform = PlatformType::from_str(&platform)?;
    state
        .recorder_manager
        .set_enable(platform, &room_id, enabled)
        .await;
    Ok(())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn fetch_hls(state: state_type!(), uri: String) -> Result<Vec<u8>, String> {
    // Handle wildcard pattern in the URI
    let uri = if uri.contains("/hls/") {
        uri.split("/hls/").last().unwrap_or(&uri).to_string()
    } else {
        uri
    };
    state
        .recorder_manager
        .handle_hls_request(&uri)
        .await
        .map_err(|e| e.to_string())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn generate_whole_clip(
    state: state_type!(),
    encode_danmu: bool,
    platform: String,
    room_id: String,
    parent_id: String,
    selected_live_ids: Option<Vec<String>>,
    output_name: Option<String>,
) -> Result<TaskRow, String> {
    log::info!("Generate whole clip for {platform} {room_id} {parent_id}");

    let task = state
        .db
        .generate_task(
            "generate_whole_clip",
            "",
            &serde_json::json!({
                "platform": platform,
                "room_id": room_id,
                "parent_id": parent_id,
                "encode_danmu": encode_danmu,
                "selected_live_ids": selected_live_ids,
                "output_name": output_name,
            })
            .to_string(),
        )
        .await?;

    #[cfg(feature = "gui")]
    let emitter = EventEmitter::new(state.app_handle.clone());
    #[cfg(feature = "headless")]
    let emitter = EventEmitter::new(state.progress_manager.get_event_sender());
    let reporter = ProgressReporter::new(state.db.clone(), &emitter, &task.id).await?;

    log::info!("Create task: {} {}", task.id, task.task_type);
    // create a tokio task to run in background
    #[cfg(feature = "gui")]
    let state_clone = (*state).clone();
    #[cfg(feature = "headless")]
    let state_clone = state.clone();

    let task_id = task.id.clone();
    state
        .task_manager
        .add_task(Task::new(
            task_id.clone(),
            TaskPriority::Normal,
            async move {
                match state_clone
                    .recorder_manager
                    .generate_whole_clip(
                        Some(&reporter),
                        GenerateWholeClipParams {
                            encode_danmu,
                            platform,
                            room_id,
                            parent_id,
                            selected_live_ids,
                            output_name,
                        },
                    )
                    .await
                {
                    Ok(()) => {
                        reporter.finish(true, "切片生成完成").await;
                        let _ = state_clone
                            .db
                            .update_task(&task_id, "success", "切片生成完成", None)
                            .await;
                        Ok(())
                    }
                    Err(e) => {
                        reporter.finish(false, &format!("切片生成失败: {e}")).await;
                        let _ = state_clone
                            .db
                            .update_task(&task_id, "failed", &format!("切片生成失败: {e}"), None)
                            .await;
                        Err(format!("切片生成失败: {e}"))
                    }
                }
            },
        ))
        .await?;
    Ok(task)
}

/// Shift absolute danmu timestamps to be relative to the exported timeline.
///
/// `stream_start_ms` is the first media segment timestamp. `range_start_s` /
/// `range_end_s` are the selected preview range in seconds; both 0 means the
/// full archive.
fn prepare_danmus_for_export(
    mut danmus: Vec<DanmuEntry>,
    stream_start_ms: i64,
    range_start_s: i64,
    range_end_s: i64,
) -> Vec<DanmuEntry> {
    let start_ms = stream_start_ms + range_start_s * 1000;
    for d in &mut danmus {
        d.ts -= start_ms;
    }
    if range_start_s != 0 || range_end_s != 0 {
        danmus.retain(|e| e.ts >= 0 && e.ts <= (range_end_s - range_start_s) * 1000);
    } else {
        danmus.retain(|e| e.ts >= 0);
    }
    danmus
}

#[cfg(test)]
mod prepare_danmus_for_export_tests {
    use super::*;
    use crate::danmu2ass::{danmu_to_ass, Danmu2AssOptions};

    const STREAM_START_MS: i64 = 1_700_000_000_000;

    fn entry(ts: i64, content: &str) -> DanmuEntry {
        DanmuEntry {
            ts,
            content: content.to_string(),
            user_name: None,
        }
    }

    #[test]
    fn full_export_converts_unix_ms_to_relative_ms() {
        let danmus = vec![
            entry(STREAM_START_MS + 1_000, "a"),
            entry(STREAM_START_MS + 5_000, "b"),
        ];

        let result = prepare_danmus_for_export(danmus, STREAM_START_MS, 0, 0);

        assert_eq!(
            result
                .iter()
                .map(|d| (d.ts, d.content.as_str()))
                .collect::<Vec<_>>(),
            vec![(1_000, "a"), (5_000, "b")]
        );
    }

    #[test]
    fn range_export_keeps_in_range_danmus_relative_to_range_start() {
        let danmus = vec![
            entry(STREAM_START_MS + 50_000, "before"),
            entry(STREAM_START_MS + 120_000, "in"),
            entry(STREAM_START_MS + 250_000, "after"),
        ];

        let result = prepare_danmus_for_export(danmus, STREAM_START_MS, 100, 200);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].content, "in");
        assert_eq!(result[0].ts, 20_000);
    }

    #[test]
    fn drops_danmus_before_stream_start() {
        let danmus = vec![
            entry(STREAM_START_MS - 1_000, "early"),
            entry(STREAM_START_MS + 1_000, "ok"),
        ];

        let result = prepare_danmus_for_export(danmus, STREAM_START_MS, 0, 0);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].content, "ok");
        assert_eq!(result[0].ts, 1_000);
    }

    #[test]
    fn ass_times_use_relative_clock_not_unix_epoch() {
        let danmus = prepare_danmus_for_export(
            vec![entry(STREAM_START_MS + 3_661_990, "hi")],
            STREAM_START_MS,
            0,
            0,
        );
        let ass = danmu_to_ass(danmus, Danmu2AssOptions::default());

        assert!(ass.contains("1:01:01.99"), "{ass}");
        assert!(
            !ass.contains("472222:"),
            "unix-epoch timestamps leaked into ASS: {ass}"
        );
    }
}
