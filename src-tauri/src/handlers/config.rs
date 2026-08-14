use crate::config::{Config, LlmConfig};
#[cfg(feature = "headless")]
use crate::constants::API_PORT;
use crate::danmu2ass::Danmu2AssOptions;
use crate::state::State;
use crate::state_type;

#[cfg(feature = "gui")]
use tauri::State as TauriState;

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_config(state: state_type!()) -> Result<Config, ()> {
    Ok(state.config.read().await.clone())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_llm_config(state: state_type!()) -> Result<LlmConfig, ()> {
    Ok(state.config.read().await.llm.clone())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn update_llm_config(
    state: state_type!(),
    provider: String,
    endpoint: String,
    api_key: String,
    model: String,
) -> Result<(), String> {
    if !matches!(provider.as_str(), "openai" | "ollama") {
        return Err("Unsupported AI provider".to_string());
    }
    let mut config = state.config.write().await;
    config.llm = LlmConfig {
        provider,
        endpoint: endpoint.trim_end_matches('/').to_string(),
        api_key,
        model: model.trim().to_string(),
    };
    config.save();
    Ok(())
}

#[derive(serde::Deserialize)]
struct OpenAiModelList {
    #[serde(default)]
    data: Vec<OpenAiModel>,
}

#[derive(serde::Deserialize)]
struct OpenAiModel {
    id: String,
}

#[derive(serde::Deserialize)]
struct OllamaModelList {
    #[serde(default)]
    models: Vec<OllamaModel>,
}

#[derive(serde::Deserialize)]
struct OllamaModel {
    name: String,
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn list_llm_models(
    provider: String,
    endpoint: String,
    api_key: String,
) -> Result<Vec<String>, String> {
    let endpoint = endpoint.trim_end_matches('/');
    let client = reqwest::Client::new();
    let mut models: Vec<String> = match provider.as_str() {
        "openai" => {
            let response = client
                .get(format!("{endpoint}/models"))
                .bearer_auth(api_key)
                .send()
                .await
                .map_err(|e| e.to_string())?
                .error_for_status()
                .map_err(|e| e.to_string())?
                .json::<OpenAiModelList>()
                .await
                .map_err(|e| e.to_string())?;
            response.data.into_iter().map(|model| model.id).collect()
        }
        "ollama" => {
            let response = client
                .get(format!("{endpoint}/api/tags"))
                .send()
                .await
                .map_err(|e| e.to_string())?
                .error_for_status()
                .map_err(|e| e.to_string())?
                .json::<OllamaModelList>()
                .await
                .map_err(|e| e.to_string())?;
            response
                .models
                .into_iter()
                .map(|model| model.name)
                .collect()
        }
        _ => return Err("Unsupported AI provider".to_string()),
    };
    models.sort();
    Ok(models)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_static_port(_state: state_type!()) -> Result<u16, ()> {
    #[cfg(feature = "headless")]
    {
        Ok(API_PORT)
    }
    #[cfg(not(feature = "headless"))]
    {
        Ok(_state.static_server.port)
    }
}

#[cfg_attr(feature = "gui", tauri::command)]
#[allow(dead_code)]
pub async fn set_cache_path(state: state_type!(), cache_path: String) -> Result<(), String> {
    let old_cache_path = state.config.read().await.cache.clone();
    log::info!("Try to set cache path: {old_cache_path} -> {cache_path}");
    if old_cache_path == cache_path {
        return Ok(());
    }

    let old_cache_path_obj = std::path::Path::new(&old_cache_path);
    let new_cache_path_obj = std::path::Path::new(&cache_path);
    // check if new cache path is under old cache path
    if new_cache_path_obj.starts_with(old_cache_path_obj) {
        log::error!("New cache path is under old cache path: {old_cache_path} -> {cache_path}");
        return Err("New cache path cannot be under old cache path".to_string());
    }

    state.recorder_manager.set_migrating(true);
    // stop and clear all recorders
    state.recorder_manager.stop_all().await;
    // first switch to new cache
    state.config.write().await.set_cache_path(&cache_path);
    log::info!("Cache path changed: {cache_path}");
    // Copy old cache to new cache
    log::info!("Start copy old cache to new cache");
    state
        .db
        .new_message(
            "缓存目录切换",
            "缓存正在迁移中，根据数据量情况可能花费较长时间，在此期间流预览功能不可用",
        )
        .await?;

    // Only migrate BSR-owned folders: the new directory may already hold the
    // user's own files, which must be left untouched.
    let plan =
        crate::handlers::migrate::plan_cache_migration(old_cache_path_obj, new_cache_path_obj);
    let migrate_result = crate::handlers::migrate::run_plan(&plan, new_cache_path_obj);

    state.recorder_manager.set_migrating(false);

    match migrate_result {
        Ok(moved) => {
            log::info!("Cache migration done: {moved} entries moved");
            state.db.new_message("缓存目录切换", "缓存切换完成").await?;
            Ok(())
        }
        Err(e) => {
            log::error!("Cache migration failed: {e}");
            state
                .db
                .new_message("缓存目录切换", &format!("缓存迁移失败：{e}"))
                .await?;
            Err(e.to_string())
        }
    }
}

#[cfg_attr(feature = "gui", tauri::command)]
#[allow(dead_code)]
pub async fn set_output_path(state: state_type!(), output_path: String) -> Result<(), String> {
    // Read the old path and release the lock immediately: holding the write
    // lock across the migration would block every config reader for its whole
    // duration.
    let old_output_path = state.config.read().await.output.clone();
    log::info!("Try to set output path: {old_output_path} -> {output_path}");
    if old_output_path == output_path {
        return Ok(());
    }

    let old_output_path_obj = std::path::Path::new(&old_output_path);
    let new_output_path_obj = std::path::Path::new(&output_path);
    // check if new output path is under old output path
    if new_output_path_obj.starts_with(old_output_path_obj) {
        log::error!("New output path is under old output path: {old_output_path} -> {output_path}");
        return Err("New output path cannot be under old output path".to_string());
    }

    state
        .db
        .new_message(
            "切片目录切换",
            "切片正在迁移中，根据数据量情况可能花费较长时间",
        )
        .await?;

    // Only migrate clips and their sidecars, so pre-existing user files in the
    // old directory stay where they are.
    let plan =
        crate::handlers::migrate::plan_output_migration(old_output_path_obj, new_output_path_obj);
    let moved = match crate::handlers::migrate::run_plan(&plan, new_output_path_obj) {
        Ok(moved) => moved,
        Err(e) => {
            log::error!("Output migration failed: {e}");
            state
                .db
                .new_message("切片目录切换", &format!("切片迁移失败：{e}"))
                .await?;
            return Err(e.to_string());
        }
    };

    log::info!("Output migration done: {moved} entries moved");
    state.config.write().await.set_output_path(&output_path);
    state.db.new_message("切片目录切换", "切片切换完成").await?;

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
    log::info!("Updating subtitle generator type to {subtitle_generator_type}");
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
    log::info!("Updating openai api endpoint to {openai_api_endpoint}");
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
    log::info!("Updating status check interval to {interval} seconds");
    state
        .config
        .write()
        .await
        .set_status_check_interval(interval);
    Ok(())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn update_whisper_language(
    state: state_type!(),
    whisper_language: String,
) -> Result<(), ()> {
    log::info!("Updating whisper language to {whisper_language}");
    state.config.write().await.whisper_language = whisper_language;
    state.config.write().await.save();
    Ok(())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn update_webhook_url(state: state_type!(), webhook_url: String) -> Result<(), ()> {
    log::info!("Updating webhook url to {webhook_url}");
    let _ = state
        .webhook_poster
        .update_config(crate::webhook::poster::WebhookConfig {
            url: webhook_url.clone(),
            ..Default::default()
        })
        .await;
    state.config.write().await.webhook_url = webhook_url;
    state.config.write().await.save();
    Ok(())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn update_danmu_ass_options(
    state: state_type!(),
    font_size: f64,
    opacity: f64,
) -> Result<(), ()> {
    log::info!("Updating danmu ass options");
    state
        .config
        .write()
        .await
        .set_danmu_ass_options(Danmu2AssOptions { font_size, opacity });
    Ok(())
}

#[cfg_attr(feature = "gui", tauri::command)]
#[cfg(feature = "gui")]
pub async fn update_powerlive_key(state: state_type!(), powerlive_key: String) -> Result<(), ()> {
    state.config.write().await.powerlive_key = powerlive_key.clone();
    state.config.write().await.save();
    log::info!("Updated powerlive key");
    Ok(())
}
