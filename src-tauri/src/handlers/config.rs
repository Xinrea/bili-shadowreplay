use crate::config::{Config, LlmConfig};
#[cfg(feature = "headless")]
use crate::constants::API_PORT;
use crate::danmu2ass::Danmu2AssOptions;
use crate::state::State;
use crate::state_type;
use crate::subtitle_generator::SubtitleGeneratorType;

#[cfg(feature = "gui")]
use tauri::State as TauriState;

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_config(state: state_type!()) -> Result<Config, ()> {
    Ok(state.config.read().await.clone())
}

/// Show the settings in the main window, scrolled to the option `target`.
#[cfg_attr(feature = "gui", tauri::command)]
pub async fn request_settings(state: state_type!(), target: Option<String>) -> Result<(), String> {
    state
        .events
        .publish(crate::events::AppEvent::SettingsRequested { target });
    crate::handlers::utils::focus_main_window(&state);
    Ok(())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_mcp_server_status(
    state: state_type!(),
) -> Result<crate::mcp::McpServerStatus, String> {
    Ok(state.mcp_server.status(&state).await)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn update_mcp_server(
    state: state_type!(),
    enabled: bool,
    port: Option<u16>,
) -> Result<crate::mcp::McpServerStatus, String> {
    #[cfg(feature = "headless")]
    if state.readonly {
        return Err("MCP settings cannot be changed in readonly mode".into());
    }
    #[cfg(feature = "gui")]
    let app_state = state.inner().clone();
    #[cfg(feature = "headless")]
    let app_state = state.clone();
    state.mcp_server.set_enabled(app_state, enabled, port).await
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
    let mut config = state.config.write().await;
    config.live_start_notify = live_start_notify;
    config.live_end_notify = live_end_notify;
    config.clip_notify = clip_notify;
    config.post_notify = post_notify;
    config.save();
    Ok(())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn update_whisper_model(state: state_type!(), whisper_model: String) -> Result<(), ()> {
    let mut config = state.config.write().await;
    config.whisper_model = whisper_model;
    config.save();
    Ok(())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn update_subtitle_setting(state: state_type!(), auto_subtitle: bool) -> Result<(), ()> {
    let mut config = state.config.write().await;
    config.auto_subtitle = auto_subtitle;
    config.save();
    Ok(())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn update_clip_name_format(
    state: state_type!(),
    clip_name_format: String,
) -> Result<(), ()> {
    let mut config = state.config.write().await;
    config.clip_name_format = clip_name_format;
    config.save();
    Ok(())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn update_whisper_prompt(state: state_type!(), whisper_prompt: String) -> Result<(), ()> {
    let mut config = state.config.write().await;
    config.whisper_prompt = whisper_prompt;
    config.save();
    Ok(())
}

/// Save one PowerLive dictionary and return it as stored, not the merged list.
/// Omit both scope keys for global, or supply both for a room.
#[cfg_attr(feature = "gui", tauri::command)]
pub async fn update_powerlive_hotwords(
    state: state_type!(),
    hotwords: Vec<String>,
    platform: Option<String>,
    room_id: Option<String>,
) -> Result<Vec<String>, String> {
    let mut config = state.config.write().await;
    let hotwords =
        config.set_powerlive_hotwords(hotwords, platform.as_deref(), room_id.as_deref())?;
    config.try_save()?;
    Ok(hotwords)
}

/// Save the PowerLive subtitle line length and return it as stored.
#[cfg_attr(feature = "gui", tauri::command)]
pub async fn update_powerlive_sentence_max_length(
    state: state_type!(),
    sentence_max_length: u32,
) -> Result<u32, String> {
    let length =
        crate::subtitle_generator::powerlive::clamp_sentence_max_length(sentence_max_length);
    let mut config = state.config.write().await;
    config.powerlive_sentence_max_length = length;
    config.try_save()?;
    Ok(length)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn update_subtitle_generator_type(
    state: state_type!(),
    subtitle_generator_type: String,
) -> Result<(), ()> {
    let generator_type =
        SubtitleGeneratorType::parse(&subtitle_generator_type).map_err(|error| {
            log::warn!("Rejecting unsupported subtitle generator type: {error}");
        })?;
    log::info!(
        "Updating subtitle generator type to {}",
        generator_type.as_str()
    );
    let mut config = state.config.write().await;
    config.subtitle_generator_type = generator_type.as_str().to_string();
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
    render_settings: Option<crate::project::RenderSettings>,
) -> Result<(), String> {
    let mut config = state.config.write().await;
    let mut next = config.clone();
    next.auto_generate.enabled = enabled;
    next.auto_generate.encode_danmu = encode_danmu;
    if let Some(settings) = render_settings {
        next.auto_generate.render_settings = settings;
    }
    next.auto_generate.render_settings.validate()?;
    // Persist first so validation or I/O errors cannot partially change the
    // running configuration. Omitted settings retain the latest saved value.
    next.try_save()?;
    *config = next;
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
    state
        .config
        .write()
        .await
        .set_whisper_language(&whisper_language);
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
    let mut config = state.config.write().await;
    config.webhook_url = webhook_url;
    config.save();
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
pub async fn get_available_video_encoders(
    native_gpu: Option<bool>,
    priority: Option<crate::project::EncodingPriority>,
) -> Result<Vec<crate::ffmpeg::encoding::AvailableVideoEncoder>, String> {
    use crate::ffmpeg::encoding::{available_encoders, VideoEncodingSettings};
    let native_gpu = native_gpu.unwrap_or(false);
    let mut encoders: Vec<_> = available_encoders()
        .await?
        .iter()
        .filter(|item| native_gpu || !item.is_native())
        .cloned()
        .collect();
    for encoder in &mut encoders {
        let defaults =
            VideoEncodingSettings::default().options_for(encoder, priority.unwrap_or_default());
        for option in &mut encoder.options {
            option.default = defaults[option.name].clone();
        }
    }
    Ok(encoders)
}
