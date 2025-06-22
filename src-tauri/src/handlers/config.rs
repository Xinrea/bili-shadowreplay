use crate::config::Config;
use crate::state::State;
use crate::state_type;

#[cfg(feature = "gui")]
use tauri::State as TauriState;

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_config(state: state_type!()) -> Result<Config, ()> {
    Ok(state.config.read().await.clone())
}

#[cfg_attr(feature = "gui", tauri::command)]
#[allow(dead_code)]
pub async fn set_cache_path(state: state_type!(), cache_path: String) -> Result<(), String> {
    let old_cache_path = state.config.read().await.cache.clone();
    if old_cache_path == cache_path {
        return Ok(());
    }

    state.recorder_manager.set_migrating(true).await;
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

    state.recorder_manager.set_migrating(false).await;

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

#[cfg_attr(feature = "gui", tauri::command)]
#[allow(dead_code)]
pub async fn set_output_path(state: state_type!(), output_path: String) -> Result<(), ()> {
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

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn update_notify(
    state: state_type!(),
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

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn update_whisper_model(state: state_type!(), whisper_model: String) -> Result<(), ()> {
    state.config.write().await.whisper_model = whisper_model;
    state.config.write().await.save();
    Ok(())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn update_subtitle_setting(state: state_type!(), auto_subtitle: bool) -> Result<(), ()> {
    state.config.write().await.auto_subtitle = auto_subtitle;
    state.config.write().await.save();
    Ok(())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn update_clip_name_format(
    state: state_type!(),
    clip_name_format: String,
) -> Result<(), ()> {
    state.config.write().await.clip_name_format = clip_name_format;
    state.config.write().await.save();
    Ok(())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn update_whisper_prompt(state: state_type!(), whisper_prompt: String) -> Result<(), ()> {
    state.config.write().await.whisper_prompt = whisper_prompt;
    state.config.write().await.save();
    Ok(())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn update_subtitle_generator_type(
    state: state_type!(),
    subtitle_generator_type: String,
) -> Result<(), ()> {
    log::info!(
        "Updating subtitle generator type to {}",
        subtitle_generator_type
    );
    let mut config = state.config.write().await;
    config.subtitle_generator_type = subtitle_generator_type;
    config.save();
    Ok(())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn update_openai_api_key(state: state_type!(), openai_api_key: String) -> Result<(), ()> {
    log::info!("Updating openai api key");
    let mut config = state.config.write().await;
    config.openai_api_key = openai_api_key;
    config.save();
    Ok(())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn update_openai_api_endpoint(
    state: state_type!(),
    openai_api_endpoint: String,
) -> Result<(), ()> {
    log::info!("Updating openai api endpoint to {}", openai_api_endpoint);
    let mut config = state.config.write().await;
    config.openai_api_endpoint = openai_api_endpoint;
    config.save();
    Ok(())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn update_auto_generate(
    state: state_type!(),
    enabled: bool,
    encode_danmu: bool,
) -> Result<(), String> {
    let mut config = state.config.write().await;
    config.auto_generate.enabled = enabled;
    config.auto_generate.encode_danmu = encode_danmu;
    config.save();
    Ok(())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn update_status_check_interval(
    state: state_type!(),
    mut interval: u64,
) -> Result<(), ()> {
    if interval < 10 {
        interval = 10; // Minimum interval of 10 seconds
    }
    log::info!("Updating status check interval to {} seconds", interval);
    state.config.write().await.status_check_interval = interval;
    state.config.write().await.save();
    Ok(())
}
