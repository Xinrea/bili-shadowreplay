use crate::database::record::RecordRow;
use crate::database::recorder::RecorderRow;
use crate::recorder::danmu::DanmuEntry;
use crate::recorder::PlatformType;
use crate::recorder::RecorderInfo;
use crate::recorder_manager::RecorderList;
use crate::state::State;
use tauri::State as TauriState;

#[tauri::command]
pub async fn get_recorder_list(state: TauriState<'_, State>) -> Result<RecorderList, ()> {
    Ok(state.recorder_manager.get_recorder_list().await)
}

#[tauri::command]
pub async fn add_recorder(
    state: TauriState<'_, State>,
    platform: String,
    room_id: u64,
) -> Result<RecorderRow, String> {
    log::info!("Add recorder: {} {}", platform, room_id);
    let platform = PlatformType::from_str(&platform).unwrap();
    let account = match platform {
        PlatformType::BiliBili => {
            if let Ok(account) = state
                .db
                .get_account("bilibili", state.config.read().await.primary_uid)
                .await
            {
                if state.config.read().await.webid_expired() {
                    log::info!("Webid expired, refetching");
                    state.config.write().await.webid = state.client.fetch_webid(&account).await?;
                    state.config.write().await.webid_ts = chrono::Utc::now().timestamp();
                }
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
            .add_recorder(
                state.config.read().await.webid.as_str(),
                &account,
                platform,
                room_id,
                true,
            )
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

#[tauri::command]
pub async fn remove_recorder(
    state: TauriState<'_, State>,
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

#[tauri::command]
pub async fn get_room_info(
    state: TauriState<'_, State>,
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

#[tauri::command]
pub async fn get_archives(
    state: TauriState<'_, State>,
    room_id: u64,
) -> Result<Vec<RecordRow>, String> {
    Ok(state.recorder_manager.get_archives(room_id).await?)
}

#[tauri::command]
pub async fn get_archive(
    state: TauriState<'_, State>,
    room_id: u64,
    live_id: String,
) -> Result<RecordRow, String> {
    Ok(state
        .recorder_manager
        .get_archive(room_id, &live_id)
        .await?)
}

#[tauri::command]
pub async fn delete_archive(
    state: TauriState<'_, State>,
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

#[tauri::command]
pub async fn get_danmu_record(
    state: TauriState<'_, State>,
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

#[tauri::command]
pub async fn send_danmaku(
    state: TauriState<'_, State>,
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

#[tauri::command]
pub async fn get_total_length(state: TauriState<'_, State>) -> Result<i64, String> {
    match state.db.get_total_length().await {
        Ok(total_length) => Ok(total_length),
        Err(e) => Err(format!("Failed to get total length: {}", e)),
    }
}

#[tauri::command]
pub async fn get_today_record_count(state: TauriState<'_, State>) -> Result<i64, String> {
    match state.db.get_today_record_count().await {
        Ok(count) => Ok(count),
        Err(e) => Err(format!("Failed to get today record count: {}", e)),
    }
}

#[tauri::command]
pub async fn get_recent_record(
    state: TauriState<'_, State>,
    offset: u64,
    limit: u64,
) -> Result<Vec<RecordRow>, String> {
    match state.db.get_recent_record(offset, limit).await {
        Ok(records) => Ok(records),
        Err(e) => Err(format!("Failed to get recent record: {}", e)),
    }
}

#[tauri::command]
pub async fn set_auto_start(
    state: TauriState<'_, State>,
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

#[tauri::command]
pub async fn force_start(
    state: TauriState<'_, State>,
    platform: String,
    room_id: u64,
) -> Result<(), String> {
    let platform = PlatformType::from_str(&platform).unwrap();
    state.recorder_manager.force_start(platform, room_id).await;
    Ok(())
}

#[tauri::command]
pub async fn force_stop(
    state: TauriState<'_, State>,
    platform: String,
    room_id: u64,
) -> Result<(), String> {
    let platform = PlatformType::from_str(&platform).unwrap();
    state.recorder_manager.force_stop(platform, room_id).await;
    Ok(())
}
