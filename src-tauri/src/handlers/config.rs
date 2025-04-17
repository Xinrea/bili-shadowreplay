use crate::config::Config;
use crate::state::State;
use tauri::State as TauriState;

#[tauri::command]
pub async fn get_config(state: TauriState<'_, State>) -> Result<Config, ()> {
    Ok(state.config.read().await.clone())
}

#[tauri::command]
pub async fn set_cache_path(
    state: TauriState<'_, State>,
    cache_path: String,
) -> Result<(), String> {
    let old_cache_path = state.config.read().await.cache.clone();
    if old_cache_path == cache_path {
        return Ok(());
    }
    // TODO only pause recorders
    // stop and clear all recorders
    state.recorder_manager.stop_all().await;
    // first switch to new cache
    state.config.write().await.set_cache_path(&cache_path);
    log::info!("Cache path changed: {}", cache_path);
    // Copy old cache to new cache
    log::info!("Start copy old cache to new cache");
    state
        .db
        .new_message(
            "缓存目录切换",
            "缓存正在迁移中，根据数据量情况可能花费较长时间，在此期间流预览功能不可用",
        )
        .await?;

    let mut old_cache_entries = vec![];
    if let Ok(entries) = std::fs::read_dir(&old_cache_path) {
        for entry in entries.flatten() {
            // check if entry is the same as new cache path
            if entry.path() == std::path::Path::new(&cache_path) {
                continue;
            }
            old_cache_entries.push(entry.path());
        }
    }

    // copy all entries to new cache
    for entry in &old_cache_entries {
        let new_entry = std::path::Path::new(&cache_path).join(entry.file_name().unwrap());
        // if entry is a folder
        if entry.is_dir() {
            if let Err(e) = crate::handlers::utils::copy_dir_all(entry, &new_entry) {
                log::error!("Copy old cache to new cache error: {}", e);
            }
        } else if let Err(e) = std::fs::copy(entry, &new_entry) {
            log::error!("Copy old cache to new cache error: {}", e);
        }
    }

    log::info!("Copy old cache to new cache done");
    state.db.new_message("缓存目录切换", "缓存切换完成").await?;
    // start all recorders
    let bili_account = state.db.get_account_by_platform("bilibili").await?;
    crate::init_rooms(
        &state.db,
        state.recorder_manager.clone(),
        &bili_account,
        &state.config.read().await.webid,
    )
    .await;

    // remove all old cache entries
    for entry in old_cache_entries {
        if entry.is_dir() {
            if let Err(e) = std::fs::remove_dir_all(&entry) {
                log::error!("Remove old cache error: {}", e);
            }
        } else if let Err(e) = std::fs::remove_file(&entry) {
            log::error!("Remove old cache error: {}", e);
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn set_output_path(state: TauriState<'_, State>, output_path: String) -> Result<(), ()> {
    let mut config = state.config.write().await;
    let old_output_path = config.output.clone();
    if old_output_path == output_path {
        return Ok(());
    }
    // list all file and folder in old output
    let mut old_output_entries = vec![];
    if let Ok(entries) = std::fs::read_dir(&old_output_path) {
        for entry in entries.flatten() {
            // check if entry is the same as new output path
            if entry.path() == std::path::Path::new(&output_path) {
                continue;
            }
            old_output_entries.push(entry.path());
        }
    }

    // rename all entries to new output
    for entry in &old_output_entries {
        let new_entry = std::path::Path::new(&output_path).join(entry.file_name().unwrap());
        // if entry is a folder
        if entry.is_dir() {
            if let Err(e) = crate::handlers::utils::copy_dir_all(entry, &new_entry) {
                log::error!("Copy old cache to new cache error: {}", e);
            }
        } else if let Err(e) = std::fs::copy(entry, &new_entry) {
            log::error!("Copy old cache to new cache error: {}", e);
        }
    }

    // remove all old output entries
    for entry in old_output_entries {
        if entry.is_dir() {
            if let Err(e) = std::fs::remove_dir_all(&entry) {
                log::error!("Remove old cache error: {}", e);
            }
        } else if let Err(e) = std::fs::remove_file(&entry) {
            log::error!("Remove old cache error: {}", e);
        }
    }

    config.set_output_path(&output_path);
    Ok(())
}

#[tauri::command]
pub async fn update_notify(
    state: TauriState<'_, State>,
    live_start_notify: bool,
    live_end_notify: bool,
    clip_notify: bool,
    post_notify: bool,
) -> Result<(), ()> {
    state.config.write().await.live_start_notify = live_start_notify;
    state.config.write().await.live_end_notify = live_end_notify;
    state.config.write().await.clip_notify = clip_notify;
    state.config.write().await.post_notify = post_notify;
    state.config.write().await.save();
    Ok(())
}

#[tauri::command]
pub async fn update_whisper_model(
    state: TauriState<'_, State>,
    whisper_model: String,
) -> Result<(), ()> {
    state.config.write().await.whisper_model = whisper_model;
    state.config.write().await.save();
    Ok(())
}

#[tauri::command]
pub async fn update_subtitle_setting(
    state: TauriState<'_, State>,
    auto_subtitle: bool,
) -> Result<(), ()> {
    state.config.write().await.auto_subtitle = auto_subtitle;
    state.config.write().await.save();
    Ok(())
}

#[tauri::command]
pub async fn update_clip_name_format(
    state: TauriState<'_, State>,
    clip_name_format: String,
) -> Result<(), ()> {
    state.config.write().await.clip_name_format = clip_name_format;
    state.config.write().await.save();
    Ok(())
}

#[tauri::command]
pub async fn update_whisper_prompt(
    state: TauriState<'_, State>,
    whisper_prompt: String,
) -> Result<(), ()> {
    state.config.write().await.whisper_prompt = whisper_prompt;
    state.config.write().await.save();
    Ok(())
}

#[tauri::command]
pub async fn update_auto_generate(
    state: tauri::State<'_, State>,
    enabled: bool,
    encode_danmu: bool,
) -> Result<(), String> {
    let mut config = state.config.write().await;
    config.auto_generate.enabled = enabled;
    config.auto_generate.encode_danmu = encode_danmu;
    config.save();
    Ok(())
}
