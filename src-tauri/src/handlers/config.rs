use crate::config::Config;
use crate::state::State;
use tauri::State as TauriState;

#[tauri::command]
pub async fn get_config(state: TauriState<'_, State>) -> Result<Config, ()> {
    Ok(state.config.read().await.clone())
}

#[tauri::command]
pub async fn set_cache_path(state: TauriState<'_, State>, cache_path: String) -> Result<(), String> {
    let old_cache_path = state.config.read().await.cache.clone();
    // first switch to new cache
    state.config.write().await.set_cache_path(&cache_path);
    log::info!("Cache path changed: {}", cache_path);
    // wait 2 seconds for cache switch
    std::thread::sleep(std::time::Duration::from_secs(2));
    // Copy old cache to new cache
    log::info!("Start copy old cache to new cache");
    state
        .db
        .new_message(
            "缓存目录切换",
            "缓存正在迁移中，根据数据量情况可能花费较长时间，在此期间流预览功能不可用",
        )
        .await?;
    if let Err(e) = crate::handlers::utils::copy_dir_all(&old_cache_path, &cache_path) {
        log::error!("Copy old cache to new cache error: {}", e);
    }
    log::info!("Copy old cache to new cache done");
    state.db.new_message("缓存目录切换", "缓存切换完成").await?;
    // Remove old cache
    if old_cache_path != cache_path {
        if let Err(e) = std::fs::remove_dir_all(old_cache_path) {
            println!("Remove old cache error: {}", e);
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn set_output_path(state: TauriState<'_, State>, output_path: String) -> Result<(), ()> {
    let mut config = state.config.write().await;
    let old_output_path = config.output.clone();
    if let Err(e) = crate::handlers::utils::copy_dir_all(&old_output_path, &output_path) {
        log::error!("Copy old output to new output error: {}", e);
    }
    config.set_output_path(&output_path);
    // remove old output
    if old_output_path != output_path {
        if let Err(e) = std::fs::remove_dir_all(old_output_path) {
            log::error!("Remove old output error: {}", e);
        }
    }
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