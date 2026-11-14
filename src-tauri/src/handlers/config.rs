use crate::config::{Config, LlmConfig, StorageConfig};
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
    let storage = state
        .recorder_manager
        .archive_storage()
        .configuration()
        .await
        .map_err(|error| {
            log::error!("Failed to read recording storage configuration: {error}");
        })?;
    let mut config = state.config.read().await.clone();
    config.storage = storage;
    Ok(config)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_storage_initialization_status(
    state: state_type!(),
) -> Result<StorageInitializationStatus, String> {
    // Reading startup state must never wait for the storage configuration gate
    // or perform remote I/O: the UI calls this while S3 is still connecting.
    let error = state
        .db
        .catalog_startup_error
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    Ok(StorageInitializationStatus {
        loading: error.is_none()
            && state
                .db
                .catalog_startup_blocked
                .load(std::sync::atomic::Ordering::Acquire),
        error,
    })
}

#[derive(serde::Serialize)]
pub struct StorageInitializationStatus {
    pub loading: bool,
    pub error: Option<String>,
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_storage_catalog_status(
    state: state_type!(),
    refresh: Option<bool>,
) -> Result<Vec<crate::database::catalog::runtime::Status>, String> {
    // An active recording/transfer may legitimately defer a refresh. Preserve
    // its last status rather than stopping it for a settings-page health check.
    if refresh.unwrap_or(false)
        && !state
            .db
            .catalog_startup_blocked
            .load(std::sync::atomic::Ordering::Acquire)
    {
        if let Err(error) = state
            .recorder_manager
            .archive_storage()
            .refresh_catalogs(true)
            .await
        {
            log::debug!("Storage catalog refresh deferred: {error}");
        }
    }
    state
        .db
        .catalog_status()
        .await
        .map_err(|error| error.to_string())
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
pub async fn set_storage_config(
    state: state_type!(),
    storage: StorageConfig,
    expected: StorageConfig,
    credentials: Option<std::collections::HashMap<String, recording_storage::StorageCredentials>>,
) -> Result<StorageConfig, String> {
    state.db.ensure_catalog_startup_complete()?;
    #[cfg(feature = "headless")]
    if state.readonly {
        return Err("只读模式不能修改录制存储设置".into());
    }
    let registry = state.recorder_manager.archive_storage().clone();
    // Retain the configuration gate through catalog detach/attach even if the
    // HTTP client disconnects or the desktop invocation is abandoned.
    tokio::spawn(async move {
        registry
            .replace_configuration_with_credentials(
                storage,
                &expected,
                &credentials.unwrap_or_default(),
            )
            .await
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(all(test, feature = "headless"))]
mod storage_settings_tests {
    use super::*;
    use crate::config::RecordingBackendConfig;
    use crate::database::{credentials::CredentialCipher, Database};
    use crate::events::EventBus;
    use crate::recorder_manager::RecorderManager;
    use crate::task::TaskManager;
    use crate::webhook::poster::{WebhookConfig, WebhookPoster};
    use std::sync::Arc;
    use tokio::sync::RwLock;

    #[tokio::test]
    async fn initialization_status_stays_responsive_and_settles_success_or_failure() {
        let directory = tempfile::tempdir().unwrap();
        let state = fixture(directory.path()).await;
        state
            .db
            .catalog_startup_blocked
            .store(true, std::sync::atomic::Ordering::Release);
        // Background initialization can be holding configuration locks during
        // slow I/O; polling its status must still complete immediately.
        let config_guard = state.config.write().await;
        let status = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            get_storage_initialization_status(state.clone()),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(status.loading);
        assert!(status.error.is_none());
        assert!(state.db.ensure_catalog_startup_complete().is_err());
        let storage = config_guard.storage.clone();
        let error = set_storage_config(state.clone(), storage.clone(), storage, None)
            .await
            .unwrap_err();
        assert!(error.contains("元数据尚未就绪"));
        let error = state
            .recorder_manager
            .add_recorder_with_selection(
                &recorder::account::Account::default(),
                recorder::platforms::PlatformType::Rtmp,
                "startup",
                "",
                true,
                Some(None),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("元数据尚未就绪"));
        let error = crate::handlers::storage_transfer::start_storage_transfer(
            state.clone(),
            "default".into(),
            "other".into(),
        )
        .await
        .unwrap_err();
        assert!(error.contains("元数据尚未就绪"));

        *state.db.catalog_startup_error.write().unwrap() = Some("catalog failed".into());
        let status = get_storage_initialization_status(state.clone())
            .await
            .unwrap();
        assert!(
            !status.loading,
            "a fatal startup error must end the spinner"
        );
        assert_eq!(status.error.as_deref(), Some("catalog failed"));
        assert!(state
            .db
            .catalog_startup_blocked
            .load(std::sync::atomic::Ordering::Acquire));

        *state.db.catalog_startup_error.write().unwrap() = None;
        state
            .db
            .catalog_startup_blocked
            .store(false, std::sync::atomic::Ordering::Release);
        let status = get_storage_initialization_status(state.clone())
            .await
            .unwrap();
        assert!(!status.loading);
        assert!(status.error.is_none());
        assert!(state.db.ensure_catalog_startup_complete().is_ok());
    }

    #[tokio::test]
    #[ignore = "requires RustFS and explicit S3_TEST_* credentials for the test harness"]
    async fn live_s3_settings_save_and_reload() {
        let directory = tempfile::tempdir().unwrap();
        let state = fixture(directory.path()).await;
        let initial = get_config(state.clone()).await.unwrap().storage;
        let mut next = initial.clone();
        next.backends.insert(
            "rustfs".into(),
            RecordingBackendConfig {
                webdav: None,
                kind: "s3".into(),
                root: String::new(),
                s3: Some(recording_storage::S3Config {
                    endpoint: std::env::var("S3_TEST_ENDPOINT").expect("set S3_TEST_ENDPOINT"),
                    bucket: std::env::var("S3_TEST_BUCKET").expect("set S3_TEST_BUCKET"),
                    region: "us-east-1".into(),
                    prefix: format!("settings-test-{}", uuid::Uuid::new_v4()),
                    path_style: true,
                }),
            },
        );
        next.default_backend = "rustfs".into();
        let credentials = recording_storage::S3Credentials {
            access_key_id: std::env::var("S3_TEST_ACCESS_KEY_ID")
                .expect("set S3_TEST_ACCESS_KEY_ID"),
            secret_access_key: std::env::var("S3_TEST_SECRET_ACCESS_KEY")
                .expect("set S3_TEST_SECRET_ACCESS_KEY"),
            session_token: String::new(),
        };
        let response = set_storage_config(
            state.clone(),
            next,
            initial,
            Some(std::collections::HashMap::from([(
                "rustfs".into(),
                credentials.clone().into(),
            )])),
        )
        .await
        .unwrap();
        let public = serde_json::to_string(&get_config(state.clone()).await.unwrap()).unwrap();
        assert!(!public.contains(&credentials.secret_access_key));
        assert!(!public.contains(&credentials.access_key_id));
        assert!(
            !std::fs::read_to_string(directory.path().join("config.toml"))
                .unwrap()
                .contains(&credentials.secret_access_key)
        );
        assert!(response.credential_revisions.contains_key("rustfs"));
        let store = state
            .recorder_manager
            .archive_storage()
            .selected_store(None)
            .await
            .unwrap();
        let recording = store.recording("bilibili", "test", "live").unwrap();
        recording.prepare().await.unwrap();
        recording.write("segment.ts", b"0123456789").await.unwrap();
        recording
            .open_log("events.jsonl")
            .await
            .unwrap()
            .append(b"first\n")
            .await
            .unwrap();
        // Resolve through a fresh registry, whose only credential source is the DB.
        let reopened =
            crate::archive_storage::ArchiveStorage::new(state.db.clone(), state.config.clone());
        let restored = reopened
            .recording("bilibili", "test", "live")
            .await
            .unwrap();
        assert_eq!(
            restored.read_range("segment.ts", 3..7).await.unwrap(),
            b"3456"
        );
        assert_eq!(restored.read_log("events.jsonl").await.unwrap(), b"first\n");
        restored.remove_all().await.unwrap();
        assert!(restored.list("", None, 10).await.unwrap().keys.is_empty());
    }

    async fn fixture(directory: &std::path::Path) -> State {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        for migration in crate::get_migrations() {
            sqlx::raw_sql(migration.sql).execute(&pool).await.unwrap();
        }
        let events = EventBus::new();
        let db = Arc::new(Database::new(
            pool,
            CredentialCipher::new(&[0; 32]).unwrap(),
            events.clone(),
        ));
        let config = Arc::new(RwLock::new(
            Config::load(
                &directory.join("config.toml"),
                &directory.join("recordings"),
                &directory.join("output"),
            )
            .unwrap(),
        ));
        let task_manager = Arc::new(TaskManager::new());
        let webhook_poster = WebhookPoster::new(WebhookConfig::default()).unwrap();
        let recorder_manager = Arc::new(RecorderManager::new(
            events.clone(),
            db.clone(),
            config.clone(),
            task_manager.clone(),
            directory.to_path_buf(),
            webhook_poster.clone(),
        ));
        State {
            mcp_server: Arc::new(crate::mcp::McpServer::default()),
            temp_storage: crate::temp_storage::TempStorage::new().unwrap(),
            db,
            config,
            webhook_poster,
            recorder_manager,
            task_manager,
            events,
            resource_dir: directory.to_path_buf(),
            readonly: false,
        }
    }

    #[tokio::test]
    async fn storage_transfer_readonly_and_cancellation_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let state = fixture(directory.path()).await;
        let mut readonly = state.clone();
        readonly.readonly = true;
        assert!(crate::handlers::storage_transfer::start_storage_transfer(
            readonly,
            "default".into(),
            "archive".into()
        )
        .await
        .is_err());
        assert!(state.db.get_tasks().await.unwrap().is_empty());
        let task = crate::handlers::storage_transfer::start_storage_transfer(
            state.clone(),
            "default".into(),
            "archive".into(),
        )
        .await
        .unwrap();
        assert_eq!(task.task_type, "storage_transfer");
        let metadata: serde_json::Value = serde_json::from_str(&task.metadata).unwrap();
        assert_eq!(metadata["source_backend_id"], "default");
        assert!(
            crate::handlers::video::cancel(state.clone(), task.id.clone())
                .await
                .is_err()
        );
        assert_eq!(state.db.get_task(&task.id).await.unwrap().status, "pending");
        assert!(crate::handlers::storage_transfer::start_storage_transfer(
            state.clone(),
            "default".into(),
            "archive".into()
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn storage_settings_save_without_redirecting_old_archives_or_output() {
        let directory = tempfile::tempdir().unwrap();
        let state = fixture(directory.path()).await;
        let old = state
            .recorder_manager
            .recording_storage(recorder::platforms::PlatformType::BiliBili, "room", "old")
            .await
            .unwrap();
        old.write("playlist.m3u8", b"old archive").await.unwrap();
        let output = state.config.read().await.output.clone();
        let mut storage = state.config.read().await.storage.clone();
        storage.backends.insert(
            "archive".into(),
            RecordingBackendConfig {
                kind: "local".into(),
                s3: None,
                webdav: None,
                root: directory
                    .path()
                    .join("archive")
                    .to_string_lossy()
                    .into_owned(),
            },
        );
        storage.default_backend = "archive".into();
        assert_eq!(
            set_storage_config(
                state.clone(),
                storage.clone(),
                get_config(state.clone()).await.unwrap().storage,
                None,
            )
            .await
            .unwrap(),
            storage
        );
        let reloaded = Config::load(
            &directory.path().join("config.toml"),
            directory.path(),
            directory.path(),
        )
        .unwrap();
        assert_ne!(reloaded.storage, storage);
        assert_eq!(get_config(state.clone()).await.unwrap().storage, storage);
        assert_eq!(reloaded.output, output);
        assert_eq!(
            state
                .recorder_manager
                .recording_storage(recorder::platforms::PlatformType::BiliBili, "room", "old",)
                .await
                .unwrap()
                .read("playlist.m3u8")
                .await
                .unwrap(),
            b"old archive"
        );
        let selected = state
            .recorder_manager
            .archive_storage()
            .selected_store(None)
            .await
            .unwrap();
        assert_eq!(selected.identity(), Some("archive"));
    }

    #[tokio::test]
    async fn readonly_invalid_and_failed_saves_leave_storage_unchanged() {
        let directory = tempfile::tempdir().unwrap();
        let state = fixture(directory.path()).await;
        let initial = state.config.read().await.storage.clone();
        let mut invalid = initial.clone();
        invalid.backends.get_mut("default").unwrap().root = "another-root".into();
        assert!(
            set_storage_config(state.clone(), invalid, initial.clone(), None)
                .await
                .is_err()
        );
        let mut readonly = state.clone();
        readonly.readonly = true;
        assert!(
            set_storage_config(readonly, initial.clone(), initial.clone(), None)
                .await
                .is_err()
        );
        let mut proposed = initial.clone();
        proposed.backends.insert(
            "new".into(),
            RecordingBackendConfig {
                kind: "local".into(),
                s3: None,
                webdav: None,
                root: directory.path().join("new").to_string_lossy().into_owned(),
            },
        );
        state.config.write().await.config_path = directory.path().to_string_lossy().into_owned();
        // Unrelated TOML write failures do not block DB-owned storage changes.
        set_storage_config(
            state.clone(),
            proposed.clone(),
            get_config(state.clone()).await.unwrap().storage,
            None,
        )
        .await
        .unwrap();
        assert_eq!(state.db.storage_config().await.unwrap(), proposed);
    }

    #[tokio::test]
    async fn stale_storage_snapshot_cannot_overwrite_newer_settings() {
        let directory = tempfile::tempdir().unwrap();
        let state = fixture(directory.path()).await;
        let original = get_config(state.clone()).await.unwrap().storage;
        let mut changed = original.clone();
        changed.backends.insert(
            "second".into(),
            RecordingBackendConfig {
                kind: "local".into(),
                s3: None,
                webdav: None,
                root: directory
                    .path()
                    .join("second")
                    .to_string_lossy()
                    .into_owned(),
            },
        );
        set_storage_config(state.clone(), changed.clone(), original.clone(), None)
            .await
            .unwrap();
        let error = set_storage_config(state.clone(), original.clone(), original, None)
            .await
            .unwrap_err();
        assert!(error.contains("configuration changed"));
        assert_eq!(get_config(state).await.unwrap().storage, changed);
    }

    #[tokio::test]
    async fn recorder_choice_is_persisted_on_the_row_and_can_return_to_inheritance() {
        let directory = tempfile::tempdir().unwrap();
        let state = fixture(directory.path()).await;
        let mut storage = state.config.read().await.storage.clone();
        storage.backends.insert(
            "archive".into(),
            RecordingBackendConfig {
                kind: "local".into(),
                s3: None,
                webdav: None,
                root: directory
                    .path()
                    .join("archive")
                    .to_string_lossy()
                    .into_owned(),
            },
        );
        set_storage_config(
            state.clone(),
            storage,
            get_config(state.clone()).await.unwrap().storage,
            None,
        )
        .await
        .unwrap();
        state
            .db
            .add_recorder_with_storage(
                recorder::platforms::PlatformType::BiliBili,
                "room",
                "",
                None,
            )
            .await
            .unwrap();
        let row = crate::handlers::recorder::set_recorder_storage(
            state.clone(),
            "bilibili".into(),
            "room".into(),
            Some("archive".into()),
        )
        .await
        .unwrap();
        assert_eq!(row.storage_backend_id.as_deref(), Some("archive"));
        assert_eq!(
            state
                .recorder_manager
                .archive_storage()
                .recorder_store("bilibili", "room")
                .await
                .unwrap()
                .identity(),
            Some("archive")
        );
        let row = crate::handlers::recorder::set_recorder_storage(
            state.clone(),
            "bilibili".into(),
            "room".into(),
            None,
        )
        .await
        .unwrap();
        assert!(row.storage_backend_id.is_none());
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
