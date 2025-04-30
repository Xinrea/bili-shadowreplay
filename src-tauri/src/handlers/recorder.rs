use crate::danmu2ass;
use crate::database::record::RecordRow;
use crate::database::recorder::RecorderRow;
use crate::recorder::danmu::DanmuEntry;
use crate::recorder::PlatformType;
use crate::recorder::RecorderInfo;
use crate::recorder_manager::RecorderList;
use crate::state::State;
use crate::state_type;

#[cfg(not(feature = "headless"))]
use tauri::State as TauriState;

use serde::Deserialize;
use serde::Serialize;

#[cfg_attr(not(feature = "headless"), tauri::command)]
pub async fn get_recorder_list(state: state_type!()) -> Result<RecorderList, ()> {
    Ok(state.recorder_manager.get_recorder_list().await)
}

#[cfg_attr(not(feature = "headless"), tauri::command)]
pub async fn add_recorder(
    state: state_type!(),
    platform: String,
    room_id: u64,
) -> Result<RecorderRow, String> {
    log::info!("Add recorder: {} {}", platform, room_id);
    let platform = PlatformType::from_str(&platform).unwrap();
    let account = match platform {
        PlatformType::BiliBili => {
            if let Ok(account) = state.db.get_account_by_platform("bilibili").await {
                Ok(account)
            } else {
                Err("没有可用账号，请先添加账号".to_string())
            }
        }
        PlatformType::Douyin => {
            if let Ok(account) = state.db.get_account_by_platform("douyin").await {
                Ok(account)
            } else {
                Err("没有可用账号，请先添加账号".to_string())
            }
        }
        _ => Err("不支持的平台".to_string()),
    };

    match account {
        Ok(account) => match state
            .recorder_manager
            .add_recorder(&account, platform, room_id, true)
            .await
        {
            Ok(()) => {
                let room = state.db.add_recorder(platform, room_id).await?;
                state
                    .db
                    .new_message("添加直播间", &format!("添加了新直播间 {}", room_id))
                    .await?;
                Ok(room)
            }
            Err(e) => Err(format!("添加失败: {}", e)),
        },
        Err(e) => Err(format!("添加失败: {}", e)),
    }
}

#[cfg_attr(not(feature = "headless"), tauri::command)]
pub async fn remove_recorder(
    state: state_type!(),
    platform: String,
    room_id: u64,
) -> Result<(), String> {
    let platform = PlatformType::from_str(&platform).unwrap();
    match state
        .recorder_manager
        .remove_recorder(platform, room_id)
        .await
    {
        Ok(()) => {
            state
                .db
                .new_message("移除直播间", &format!("移除了直播间 {}", room_id))
                .await?;
            Ok(state.db.remove_recorder(room_id).await?)
        }
        Err(e) => Err(e.to_string()),
    }
}

#[cfg_attr(not(feature = "headless"), tauri::command)]
pub async fn get_room_info(
    state: state_type!(),
    platform: String,
    room_id: u64,
) -> Result<RecorderInfo, String> {
    let platform = PlatformType::from_str(&platform).unwrap();
    if let Some(info) = state
        .recorder_manager
        .get_recorder_info(platform, room_id)
        .await
    {
        Ok(info)
    } else {
        Err("Not found".to_string())
    }
}

#[cfg_attr(not(feature = "headless"), tauri::command)]
pub async fn get_archives(state: state_type!(), room_id: u64) -> Result<Vec<RecordRow>, String> {
    Ok(state.recorder_manager.get_archives(room_id).await?)
}

#[cfg_attr(not(feature = "headless"), tauri::command)]
pub async fn get_archive(
    state: state_type!(),
    room_id: u64,
    live_id: String,
) -> Result<RecordRow, String> {
    Ok(state
        .recorder_manager
        .get_archive(room_id, &live_id)
        .await?)
}

#[cfg_attr(not(feature = "headless"), tauri::command)]
pub async fn delete_archive(
    state: state_type!(),
    platform: String,
    room_id: u64,
    live_id: String,
) -> Result<(), String> {
    let platform = PlatformType::from_str(&platform).unwrap();
    state
        .recorder_manager
        .delete_archive(platform, room_id, &live_id)
        .await?;
    state
        .db
        .new_message(
            "删除历史缓存",
            &format!("删除了房间 {} 的历史缓存 {}", room_id, live_id),
        )
        .await?;
    Ok(())
}

#[cfg_attr(not(feature = "headless"), tauri::command)]
pub async fn get_danmu_record(
    state: state_type!(),
    platform: String,
    room_id: u64,
    live_id: String,
) -> Result<Vec<DanmuEntry>, String> {
    let platform = PlatformType::from_str(&platform).unwrap();
    Ok(state
        .recorder_manager
        .get_danmu(platform, room_id, &live_id)
        .await?)
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportDanmuOptions {
    platform: String,
    room_id: u64,
    live_id: String,
    x: i64,
    y: i64,
    ass: bool,
}

#[cfg_attr(not(feature = "headless"), tauri::command)]
pub async fn export_danmu(
    state: state_type!(),
    options: ExportDanmuOptions,
) -> Result<String, String> {
    let platform = PlatformType::from_str(&options.platform).unwrap();
    let mut danmus = state
        .recorder_manager
        .get_danmu(platform, options.room_id, &options.live_id)
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
        Ok(danmu2ass::danmu_to_ass(danmus))
    } else {
        // map and join entries
        Ok(danmus
            .iter()
            .map(|e| format!("{}:{}", e.ts, e.content))
            .collect::<Vec<_>>()
            .join("\n"))
    }
}

#[cfg_attr(not(feature = "headless"), tauri::command)]
pub async fn send_danmaku(
    state: state_type!(),
    uid: u64,
    room_id: u64,
    message: String,
) -> Result<(), String> {
    let account = state.db.get_account("bilibili", uid).await?;
    state
        .client
        .send_danmaku(&account, room_id, &message)
        .await?;
    Ok(())
}

#[cfg_attr(not(feature = "headless"), tauri::command)]
pub async fn get_total_length(state: state_type!()) -> Result<i64, String> {
    match state.db.get_total_length().await {
        Ok(total_length) => Ok(total_length),
        Err(e) => Err(format!("Failed to get total length: {}", e)),
    }
}

#[cfg_attr(not(feature = "headless"), tauri::command)]
pub async fn get_today_record_count(state: state_type!()) -> Result<i64, String> {
    match state.db.get_today_record_count().await {
        Ok(count) => Ok(count),
        Err(e) => Err(format!("Failed to get today record count: {}", e)),
    }
}

#[cfg_attr(not(feature = "headless"), tauri::command)]
pub async fn get_recent_record(
    state: state_type!(),
    offset: u64,
    limit: u64,
) -> Result<Vec<RecordRow>, String> {
    match state.db.get_recent_record(offset, limit).await {
        Ok(records) => Ok(records),
        Err(e) => Err(format!("Failed to get recent record: {}", e)),
    }
}

#[cfg_attr(not(feature = "headless"), tauri::command)]
pub async fn set_auto_start(
    state: state_type!(),
    platform: String,
    room_id: u64,
    auto_start: bool,
) -> Result<(), String> {
    let platform = PlatformType::from_str(&platform).unwrap();
    state
        .recorder_manager
        .set_auto_start(platform, room_id, auto_start)
        .await;
    Ok(())
}

#[cfg_attr(not(feature = "headless"), tauri::command)]
pub async fn force_start(
    state: state_type!(),
    platform: String,
    room_id: u64,
) -> Result<(), String> {
    let platform = PlatformType::from_str(&platform).unwrap();
    state.recorder_manager.force_start(platform, room_id).await;
    Ok(())
}

#[cfg_attr(not(feature = "headless"), tauri::command)]
pub async fn force_stop(
    state: state_type!(),
    platform: String,
    room_id: u64,
) -> Result<(), String> {
    let platform = PlatformType::from_str(&platform).unwrap();
    state.recorder_manager.force_stop(platform, room_id).await;
    Ok(())
}

#[cfg_attr(not(feature = "headless"), tauri::command)]
pub async fn fetch_hls(state: state_type!(), uri: String) -> Result<Vec<u8>, String> {
    // trim */hls/
    let uri = uri.trim_start_matches("*/hls/");
    state
        .recorder_manager
        .handle_hls_request(uri)
        .await
        .map_err(|e| e.to_string())
}
