use std::str::FromStr;

use crate::danmu2ass;
use crate::database::record::RecordRow;
use crate::database::recorder::RecorderRow;
use crate::database::task::TaskRow;
use crate::progress::progress_reporter::EventEmitter;
use crate::progress::progress_reporter::ProgressReporter;
use crate::progress::progress_reporter::ProgressReporterTrait;
use crate::recorder_manager::RecorderList;
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
    let platform = PlatformType::from_str(&platform).unwrap();
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
    let platform = PlatformType::from_str(&platform).unwrap();
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
    let platform = PlatformType::from_str(&platform).unwrap();
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
    ass: bool,
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn export_danmu(
    state: state_type!(),
    options: ExportDanmuOptions,
) -> Result<String, String> {
    let platform = PlatformType::from_str(&options.platform)?;
    let mut danmus = state
        .recorder_manager
        .load_danmus(platform, &options.room_id, &options.live_id)
        .await?;

    log::debug!("First danmu entry: {:?}", danmus.first());
    // update entry ts to offset
    for d in &mut danmus {
        d.ts -= (options.x + options.y) * 1000;
    }
    if options.x != 0 || options.y != 0 {
        danmus.retain(|e| e.ts >= 0 && e.ts <= (options.y - options.x) * 1000);
    }

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
pub async fn get_total_length(state: state_type!()) -> Result<i64, String> {
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
                        encode_danmu,
                        platform,
                        &room_id,
                        parent_id,
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
