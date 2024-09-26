// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod db;
mod recorder;
mod recorder_manager;
mod tray;

use custom_error::custom_error;
use db::{AccountRow, Database};
use recorder::bilibili::errors::BiliClientError;
use recorder::bilibili::profile::Profile;
use recorder::bilibili::{BiliClient, QrInfo, QrStatus};
use recorder_manager::{RecorderInfo, RecorderList, RecorderManager};
use tauri::utils::config::WindowEffectsConfig;
use tauri_plugin_sql::{Migration, MigrationKind};

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use tauri::{Manager, Theme, WindowEvent};
use tokio::sync::RwLock;

use platform_dirs::AppDirs;

custom_error! {
    StateError
    RecorderAlreadyExists = "Recorder already exists",
    RecorderCreateError = "Recorder create error",
}

#[tauri::command]
fn show_in_folder(path: String) {
    #[cfg(target_os = "windows")]
    {
        Command::new("explorer")
            .args(["/select,", &path]) // The comma after select is not a typo
            .spawn()
            .unwrap();
    }

    #[cfg(target_os = "linux")]
    {
        use std::fs::metadata;
        use std::path::PathBuf;
        if path.contains(",") {
            // see https://gitlab.freedesktop.org/dbus/dbus/-/issues/76
            let new_path = match metadata(&path).unwrap().is_dir() {
                true => path,
                false => {
                    let mut path2 = PathBuf::from(path);
                    path2.pop();
                    path2.into_os_string().into_string().unwrap()
                }
            };
            Command::new("xdg-open").arg(&new_path).spawn().unwrap();
        } else {
            Command::new("dbus-send")
                .args([
                    "--session",
                    "--dest=org.freedesktop.FileManager1",
                    "--type=method_call",
                    "/org/freedesktop/FileManager1",
                    "org.freedesktop.FileManager1.ShowItems",
                    format!("array:string:\"file://{path}\"").as_str(),
                    "string:\"\"",
                ])
                .spawn()
                .unwrap();
        }
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open").args(["-R", &path]).spawn().unwrap();
    }
}

#[derive(serde::Deserialize, serde::Serialize, Clone)]
pub struct Config {
    cache: String,
    output: String,
    primary_uid: u64,
    profile_preset: HashMap<String, Profile>,
}

impl Config {
    pub fn load() -> Self {
        let app_dirs = AppDirs::new(Some("bili-shadowreplay"), false).unwrap();
        let config_path = app_dirs.config_dir.join("Conf.toml");
        if let Ok(content) = std::fs::read_to_string(config_path) {
            if let Ok(config) = toml::from_str(&content) {
                return config;
            }
        }
        let config = Config {
            cache: app_dirs
                .cache_dir
                .join("cache")
                .to_str()
                .unwrap()
                .to_string(),
            output: app_dirs
                .data_dir
                .join("output")
                .to_str()
                .unwrap()
                .to_string(),
            primary_uid: 0,
            profile_preset: HashMap::new(),
        };
        config.save();
        config
    }

    pub fn save(&self) {
        let content = toml::to_string(&self).unwrap();
        let app_dirs = AppDirs::new(Some("bili-shadowreplay"), false).unwrap();
        // Create app dirs if not exists
        std::fs::create_dir_all(&app_dirs.config_dir).unwrap();
        let config_path = app_dirs.config_dir.join("Conf.toml");
        std::fs::write(config_path, content).unwrap();
    }

    pub fn set_cache_path(&mut self, path: &str) {
        // Copy all files in cache to new cache
        if self.cache == path {
            return;
        }
        let old_cache = self.cache.clone();
        copy_dir_all(old_cache, path).unwrap();
        self.cache = path.to_string();
        self.save();
    }

    pub fn set_output_path(&mut self, path: String) {
        self.output = path;
        self.save();
    }

    pub fn get_profile(&self, room_id: u64) -> Option<Profile> {
        self.profile_preset.get(&room_id.to_string()).cloned()
    }

    pub fn update_profile(&mut self, room_id: u64, profile: &Profile) {
        self.profile_preset
            .insert(room_id.to_string(), profile.clone());
        self.save();
    }
}

fn copy_dir_all(
    src: impl AsRef<std::path::Path>,
    dst: impl AsRef<std::path::Path>,
) -> std::io::Result<()> {
    std::fs::create_dir_all(&dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_all(entry.path(), dst.as_ref().join(entry.file_name()))?;
        } else {
            std::fs::copy(entry.path(), dst.as_ref().join(entry.file_name()))?;
        }
    }
    Ok(())
}

#[derive(Clone)]
struct State {
    db: Arc<Database>,
    client: Arc<BiliClient>,
    config: Arc<RwLock<Config>>,
    recorder_manager: Arc<RecorderManager>,
    app_handle: tauri::AppHandle,
}

impl State {
    pub async fn get_qr(&self) -> Result<QrInfo, BiliClientError> {
        self.client.get_qr().await
    }

    pub async fn get_qr_status(&self, key: &str) -> Result<QrStatus, BiliClientError> {
        self.client.get_qr_status(key).await
    }

    pub async fn clip(&self, room_id: u64, len: f64) -> Result<String, String> {
        Ok(self
            .recorder_manager
            .clip(&self.config.read().await.output, room_id, len)
            .await?)
    }

    pub async fn clip_range(
        &self,
        room_id: u64,
        ts: u64,
        x: f64,
        y: f64,
    ) -> Result<String, String> {
        Ok(self
            .recorder_manager
            .clip_range(&self.config.read().await.output, room_id, ts, x, y)
            .await?)
    }
}

#[tauri::command]
async fn get_recorder_list(state: tauri::State<'_, State>) -> Result<RecorderList, ()> {
    Ok(state.recorder_manager.get_recorder_list().await)
}

#[tauri::command]
async fn get_qr(state: tauri::State<'_, State>) -> Result<QrInfo, ()> {
    println!("[invoke]get qr");
    match state.get_qr().await {
        Ok(qr_info) => Ok(qr_info),
        Err(_e) => Err(()),
    }
}

#[tauri::command]
async fn get_qr_status(state: tauri::State<'_, State>, qrcode_key: &str) -> Result<QrStatus, ()> {
    match state.get_qr_status(qrcode_key).await {
        Ok(qr_status) => Ok(qr_status),
        Err(_e) => Err(()),
    }
}

#[tauri::command]
async fn add_account(state: tauri::State<'_, State>, cookies: &str) -> Result<AccountRow, String> {
    let mut is_primary = false;
    if state.config.read().await.primary_uid == 0 || state.db.get_accounts().await?.len() == 0 {
        is_primary = true;
    }
    let account = state.db.add_account(cookies).await?;
    let account_info = state.client.get_user_info(&account, account.uid).await?;
    state
        .db
        .update_account(
            account_info.user_id,
            &account_info.user_name,
            &account_info.user_avatar_url,
        )
        .await?;
    if is_primary {
        state.config.write().await.primary_uid = account.uid;
    }
    Ok(account)
}

#[tauri::command]
async fn remove_account(state: tauri::State<'_, State>, uid: u64) -> Result<(), String> {
    if state.db.get_accounts().await?.len() == 1 {
        return Err("At least one account is required".into());
    }
    // logout
    let account = state.db.get_account(uid).await?;
    state.client.logout(&account).await?;
    Ok(state.db.remove_account(uid).await?)
}

#[tauri::command]
async fn set_primary(state: tauri::State<'_, State>, uid: u64) -> Result<(), String> {
    if let Ok(_) = state.db.get_account(uid).await {
        state.config.write().await.primary_uid = uid;
        Ok(())
    } else {
        Err("Account not exist".into())
    }
}

#[tauri::command]
async fn add_recorder(
    state: tauri::State<'_, State>,
    room_id: u64,
) -> Result<db::RecorderRow, String> {
    let account = state
        .db
        .get_account(state.config.read().await.primary_uid)
        .await?;
    match state
        .recorder_manager
        .add_recorder(&account, room_id, &state.config.read().await.cache)
        .await
    {
        Ok(()) => Ok(state.db.add_recorder(room_id).await?),
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
async fn remove_recorder(state: tauri::State<'_, State>, room_id: u64) -> Result<(), String> {
    match state.recorder_manager.remove_recorder(room_id).await {
        Ok(()) => Ok(state.db.remove_recorder(room_id).await?),
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
async fn get_config(state: tauri::State<'_, State>) -> Result<Config, ()> {
    Ok(state.config.read().await.clone())
}

#[tauri::command]
async fn set_cache_path(state: tauri::State<'_, State>, cache_path: String) -> Result<(), ()> {
    let mut config = state.config.write().await;
    let old_cache_path = config.cache.clone();
    config.set_cache_path(&cache_path);
    drop(config);
    // Remove old cache
    if old_cache_path != cache_path {
        if let Err(e) = std::fs::remove_dir_all(old_cache_path) {
            println!("Remove old cache error: {}", e);
        }
    }
    Ok(())
}

#[tauri::command]
async fn set_output_path(state: tauri::State<'_, State>, output_path: String) -> Result<(), ()> {
    let mut config = state.config.write().await;
    config.set_output_path(output_path);
    Ok(())
}

#[tauri::command]
async fn clip(state: tauri::State<'_, State>, room_id: u64, len: f64) -> Result<String, String> {
    println!("[invoke]clip room_id: {}, len: {}", room_id, len);
    state.clip(room_id, len).await
}

#[tauri::command]
async fn clip_range(
    state: tauri::State<'_, State>,
    room_id: u64,
    ts: u64,
    x: f64,
    y: f64,
) -> Result<String, String> {
    println!(
        "[invoke]clip room_id: {}, ts: {}, start: {}, end: {}",
        room_id, ts, x, y
    );
    state.clip_range(room_id, ts, x, y).await
}

#[tauri::command]
async fn upload_procedure(
    state: tauri::State<'_, State>,
    uid: u64,
    room_id: u64,
    file: String,
    cover: String,
    mut profile: Profile,
) -> Result<String, String> {
    // update profile
    state
        .config
        .write()
        .await
        .update_profile(room_id, &profile.clone());
    let account = state.db.get_account(uid).await?;
    let path = Path::new(&file);
    let cover_url = state.client.upload_cover(&account, &cover);
    if let Ok(video) = state.client.prepare_video(&account, path).await {
        profile.cover = cover_url.await.unwrap_or("".to_string());
        if let Ok(ret) = state.client.submit_video(&account, &profile, &video).await {
            Ok(ret.bvid)
        } else {
            Err("Submit video failed".to_string())
        }
    } else {
        Err("Preload video failed".to_string())
    }
}

#[tauri::command]
async fn get_room_info(
    state: tauri::State<'_, State>,
    room_id: u64,
) -> Result<RecorderInfo, String> {
    if let Some(info) = state.recorder_manager.get_recorder_info(room_id).await {
        Ok(info)
    } else {
        Err("Not found".to_string())
    }
}

#[tauri::command]
async fn get_archives(
    state: tauri::State<'_, State>,
    room_id: u64,
) -> Result<Option<Vec<u64>>, String> {
    log::debug!("Get archives for {}", room_id);
    Ok(state.recorder_manager.get_archives(room_id).await)
}

#[tauri::command]
async fn delete_archive(
    state: tauri::State<'_, State>,
    room_id: u64,
    ts: u64,
) -> Result<(), String> {
    state.recorder_manager.delete_archive(room_id, ts).await;
    Ok(())
}

#[derive(serde::Serialize)]
struct AccountInfo {
    pub primary_uid: u64,
    pub accounts: Vec<AccountRow>,
}

#[tauri::command]
async fn get_accounts(state: tauri::State<'_, State>) -> Result<AccountInfo, String> {
    let config = state.config.read().await.clone();
    let account_info = AccountInfo {
        primary_uid: config.primary_uid,
        accounts: state.db.get_accounts().await?,
    };
    Ok(account_info)
}

#[tauri::command]
async fn open_live(state: tauri::State<'_, State>, room_id: u64, ts: u64) -> Result<(), String> {
    let addr = state.recorder_manager.get_hls_server_addr().await.unwrap();
    let recorder_info = state
        .recorder_manager
        .get_recorder_info(room_id)
        .await
        .unwrap();
    if let Err(e) = tauri::WebviewWindowBuilder::new(
        &state.app_handle.clone(),
        format!("Live:{}:{}", room_id, ts),
        tauri::WebviewUrl::App(
            format!(
                "live_index.html?port={}&room_id={}&ts={}",
                addr.port(),
                room_id,
                ts
            )
            .into(),
        ),
    )
    .title(format!(
        "Live[{}] {}",
        room_id, recorder_info.room_info.room_title
    ))
    .theme(Some(Theme::Light))
    .decorations(false)
    .transparent(true)
    .inner_size(1200.0, 800.0)
    .effects(WindowEffectsConfig {
        effects: vec![
            tauri_utils::WindowEffect::Tabbed,
            tauri_utils::WindowEffect::Mica,
        ],
        state: None,
        radius: None,
        color: None,
    })
    .build()
    {
        log::error!("Create live-window failed: {}", e.to_string());
    }
    Ok(())
}

#[tauri::command]
async fn get_profile(state: tauri::State<'_, State>, room_id: u64) -> Result<Profile, String> {
    Ok(state
        .config
        .read()
        .await
        .profile_preset
        .get(&room_id.to_string())
        .cloned()
        .unwrap_or(Profile::new("", "", 27)))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Setup log
    simplelog::CombinedLogger::init(vec![simplelog::TermLogger::new(
        simplelog::LevelFilter::Debug,
        simplelog::Config::default(),
        simplelog::TerminalMode::Mixed,
        simplelog::ColorChoice::Auto,
    )])
    .unwrap();

    // Setup ffmpeg
    ffmpeg_sidecar::download::auto_download().unwrap();

    //Setup database
    let migrations = vec![Migration {
        version: 1,
        description: "create_initial_tables",
        sql: r#"
            CREATE TABLE accounts (uid INTEGER PRIMARY KEY, name TEXT, avatar TEXT, csrf TEXT, cookies TEXT, created_at TEXT);
            CREATE TABLE recorders (room_id INTEGER PRIMARY KEY, created_at TEXT);
            CREATE TABLE records (live_id INTEGER PRIMARY KEY, room_id INTEGER, length INTEGER, size INTEGER, created_at TEXT);
            CREATE TABLE danmu_statistics (live_id INTEGER PRIMARY KEY, room_id INTEGER, value INTEGER, time_point TEXT);
            CREATE TABLE messages (id INTEGER PRIMARY KEY, title TEXT, content TEXT, read INTEGER, created_at TEXT);
            CREATE TABLE videos (id INTEGER PRIMARY KEY, file TEXT, length INTEGER, size INTEGER, status INTEGER, title TEXT, desc TEXT, tags TEXT, area INTEGER);
            "#,
        kind: MigrationKind::Up,
    }];

    // Tauri part
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|_, _, _| {}))
        .plugin(
            tauri_plugin_sql::Builder::default()
                .add_migrations("sqlite:data.db", migrations)
                .build(),
        )
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // init
            let client = Arc::new(BiliClient::new().unwrap());
            let config = Arc::new(RwLock::new(Config::load()));
            let config_clone = config.clone();
            let recorder_manager = Arc::new(RecorderManager::new(config.clone()));
            let recorder_manager_clone = recorder_manager.clone();
            let dbs = app.state::<tauri_plugin_sql::DbInstances>().inner();
            let db = Arc::new(Database::new());
            let db_clone = db.clone();
            tauri::async_runtime::block_on(async move {
                db_clone
                    .set(dbs.0.lock().await.get("sqlite:data.db").unwrap().clone())
                    .await;
                let initial_rooms = db_clone.get_recorders().await.unwrap();
                let mut primary_uid = config_clone.read().await.primary_uid;
                if primary_uid == 0 {
                    let accounts = db_clone.get_accounts().await.unwrap();
                    if !accounts.is_empty() {
                        primary_uid = accounts.first().unwrap().uid;
                        config_clone.write().await.primary_uid = primary_uid;
                        config_clone.write().await.save();
                    }
                }
                let account = db_clone.get_account(primary_uid).await;
                if let Ok(account) = account {
                    for room in initial_rooms {
                        if let Err(e) = recorder_manager_clone
                            .add_recorder(&account, room.room_id, &config_clone.read().await.cache)
                            .await
                        {
                            log::error!("error when adding initial rooms: {}", e);
                        }
                    }
                } else {
                    log::warn!("No available account found");
                }
                let _ = recorder_manager_clone.run_hls().await;
            });
            let state = State {
                db,
                client,
                config,
                recorder_manager,
                app_handle: app.handle().clone(),
            };
            let _ = tray::create_tray(app.handle());
            app.manage(state);
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if !window.label().starts_with("Live") {
                    window.hide().unwrap();
                    api.prevent_close();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_recorder_list,
            add_recorder,
            remove_recorder,
            get_config,
            set_cache_path,
            set_output_path,
            clip,
            clip_range,
            upload_procedure,
            show_in_folder,
            get_qr,
            get_qr_status,
            open_live,
            get_accounts,
            add_account,
            remove_account,
            set_primary,
            get_room_info,
            get_archives,
            get_profile,
            delete_archive,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
    Ok(())
}
