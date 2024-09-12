// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod recorder;
mod recorder_manager;

use custom_error::custom_error;
use recorder::bilibili::errors::BiliClientError;
use recorder::bilibili::profile::Profile;
use recorder::bilibili::{BiliClient, QrInfo, QrStatus};
use recorder_manager::{RecorderManager, RoomInfo, Summary};

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use tauri::{
    CustomMenuItem, LogicalSize, SystemTray, SystemTrayEvent, SystemTrayMenu, SystemTrayMenuItem,
    Theme, Window,
};
use tauri::{Manager, WindowEvent};
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
    rooms: Vec<u64>,
    admin_uid: Vec<u64>,
    cache: String,
    output: String,
    login: bool,
    uid: String,
    cookies: String,
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
            rooms: Vec::new(),
            admin_uid: Vec::new(),
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
            login: false,
            uid: "".to_string(),
            cookies: "".to_string(),
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

    pub fn add(&mut self, room: u64) {
        if self.rooms.contains(&room) {
            return;
        }
        self.rooms.push(room);
        self.save();
    }

    pub fn remove(&mut self, room: u64) {
        self.rooms.retain(|&x| x != room);
        self.save();
    }

    pub fn set_admins(&mut self, admins: Vec<u64>) {
        self.admin_uid = admins;
        self.save();
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

    pub fn set_cookies(&mut self, cookies: &str) {
        self.cookies = cookies.to_string();
        // match(/DedeUserID=(\d+)/)[1
        self.uid = cookies
            .split("DedeUserID=")
            .collect::<Vec<&str>>()
            .get(1)
            .unwrap()
            .split(";")
            .collect::<Vec<&str>>()
            .first()
            .unwrap()
            .to_string();
        self.login = true;
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

    pub fn logout(&mut self) {
        self.cookies = "".to_string();
        self.uid = "".to_string();
        self.login = false;
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
    client: Arc<BiliClient>,
    post_window: Window,
    config: Arc<RwLock<Config>>,
    recorder_manager: Arc<RecorderManager>,
    app_handle: tauri::AppHandle,
}

impl State {
    pub async fn get_summary(&self) -> Summary {
        self.recorder_manager.get_summary().await
    }

    pub async fn get_qr(&self) -> Result<QrInfo, BiliClientError> {
        self.client.get_qr().await
    }

    pub async fn get_qr_status(&self, key: &str) -> Result<QrStatus, BiliClientError> {
        self.client.get_qr_status(key).await
    }

    pub async fn add_recorder(&self, room_id: u64) -> Result<(), String> {
        self.recorder_manager.add_recorder(room_id).await
    }

    pub async fn remove_recorder(&self, room_id: u64) {
        let _ = self.recorder_manager.remove_recorder(room_id).await;
    }

    pub async fn clip(&self, room_id: u64, len: f64) -> Result<String, String> {
        if let Ok(file) = self.recorder_manager.clip(room_id, len).await {
            Ok(file)
        } else {
            Err("Clip error".to_string())
        }
    }

    pub async fn clip_range(
        &self,
        room_id: u64,
        ts: u64,
        x: f64,
        y: f64,
    ) -> Result<String, String> {
        if let Ok(file) = self.recorder_manager.clip_range(room_id, ts, x, y).await {
            Ok(file)
        } else {
            Err("Clip error".to_string())
        }
    }
}

#[tauri::command]
async fn get_summary(state: tauri::State<'_, State>) -> Result<Summary, ()> {
    Ok(state.get_summary().await)
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
async fn add_recorder(state: tauri::State<'_, State>, room_id: u64) -> Result<(), String> {
    // Config update
    if let Err(e) = state.add_recorder(room_id).await {
        println!("add recorder failed: {:?}", e);
        Err(e.to_string())
    } else {
        println!("add recorder success: {}", room_id);
        Ok(())
    }
}

#[tauri::command]
async fn remove_recorder(state: tauri::State<'_, State>, room_id: u64) -> Result<(), ()> {
    // Config update
    state.remove_recorder(room_id).await;
    Ok(())
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
async fn set_admins(state: tauri::State<'_, State>, admins: Vec<u64>) -> Result<(), ()> {
    let mut config = state.config.write().await;
    config.set_admins(admins);
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

#[derive(serde::Serialize, Clone)]
struct UploadInfo {
    room_id: u64,
    file: String,
    cover: String,
    profile: Profile,
}

#[tauri::command]
async fn prepare_upload(
    state: tauri::State<'_, State>,
    room_id: u64,
    file: String,
    cover: String,
) -> Result<(), String> {
    state
        .post_window
        .emit(
            "init",
            UploadInfo {
                room_id,
                file,
                cover,
                profile: state
                    .config
                    .read()
                    .await
                    .get_profile(room_id)
                    .unwrap_or(Profile::new("视频标题", "描述", 27)),
            },
        )
        .unwrap();
    state.post_window.show().unwrap();
    Ok(())
}

#[tauri::command]
async fn upload_procedure(
    state: tauri::State<'_, State>,
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
    let path = Path::new(&file);
    let cover_url = state.client.upload_cover(&cover);
    if let Ok(video) = state.client.prepare_video(path).await {
        profile.cover = cover_url.await.unwrap_or("".to_string());
        if let Ok(ret) = state.client.submit_video(&profile, &video).await {
            Ok(ret.bvid)
        } else {
            Err("Submit video failed".to_string())
        }
    } else {
        Err("Preload video failed".to_string())
    }
}

#[tauri::command]
async fn set_cookies(state: tauri::State<'_, State>, cookies: String) -> Result<(), String> {
    let mut config = state.config.write().await;
    config.set_cookies(&cookies);
    state.client.set_cookies(&cookies).await;
    state.recorder_manager.update_cookies(&cookies).await;
    Ok(())
}

#[tauri::command]
async fn logout(state: tauri::State<'_, State>) -> Result<(), String> {
    let mut config = state.config.write().await;
    config.logout();
    state.client.logout().await;
    Ok(())
}

#[tauri::command]
async fn get_room_info(state: tauri::State<'_, State>, room_id: u64) -> Result<RoomInfo, String> {
    if let Some(info) = state.recorder_manager.get_room_info(room_id).await {
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
    pub login: bool,
    pub uid: String,
    pub name: String,
    pub sign: String,
    pub face: String,
}

#[tauri::command]
async fn get_accounts(state: tauri::State<'_, State>) -> Result<AccountInfo, String> {
    let config = state.config.read().await.clone();
    let mut account_info = AccountInfo {
        login: false,
        uid: "".to_string(),
        name: "".to_string(),
        sign: "".to_string(),
        face: "".to_string(),
    };
    // get user info
    if config.login {
        account_info.login = true;
        account_info.uid = config.uid.clone();
        match state.client.get_user_info(config.uid.as_str()).await {
            Ok(info) => {
                account_info.name = info.user_name;
                account_info.sign = info.user_sign;
                account_info.face = info.user_avatar_url;
            }
            Err(e) => {
                println!("{}", e);
            }
        }
    }
    Ok(account_info)
}

#[tauri::command]
async fn open_live(state: tauri::State<'_, State>, room_id: u64, ts: u64) -> Result<(), String> {
    let addr = state.recorder_manager.get_hls_server_addr().await.unwrap();
    let window = tauri::WindowBuilder::new(
        &state.app_handle.clone(),
        room_id.to_string(),
        tauri::WindowUrl::App(
            format!(
                "live_index.html?port={}&room_id={}&ts={}",
                addr.port(),
                room_id,
                ts
            )
            .into(),
        ),
    )
    .title(format!("Live {}", room_id))
    .theme(Some(Theme::Dark))
    .build()
    .unwrap();
    let window_clone = window.clone();
    window_clone.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { .. } = event {
            // close window
            window.close().unwrap();
        }
    });
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Setup log
    simplelog::CombinedLogger::init(vec![simplelog::TermLogger::new(
        simplelog::LevelFilter::Info,
        simplelog::Config::default(),
        simplelog::TerminalMode::Mixed,
        simplelog::ColorChoice::Auto,
    )])
    .unwrap();
    // Setup ffmpeg
    ffmpeg_sidecar::download::auto_download().unwrap();
    // Setup tray icon
    let quit = CustomMenuItem::new("quit".to_string(), "Quit");
    let hide = CustomMenuItem::new("hide".to_string(), "Hide");
    let tray_menu = SystemTrayMenu::new()
        .add_item(hide)
        .add_native_item(SystemTrayMenuItem::Separator)
        .add_item(quit);

    let tray = SystemTray::new().with_menu(tray_menu);
    let client = Arc::new(BiliClient::new().unwrap());
    let config = Arc::new(RwLock::new(Config::load()));
    let recorder_manager = Arc::new(RecorderManager::new(config.clone()));

    // Start recorder manager in tokio runtime
    // create a new tokio runtime
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();
    let recorder_manager_clone = recorder_manager.clone();
    let client_clone = client.clone();
    let config_clone = config.clone();
    rt.block_on(async move {
        client_clone
            .set_cookies(&config_clone.read().await.cookies)
            .await;
        recorder_manager_clone.init().await;
        recorder_manager_clone.run().await;
    });

    // Tauri part
    tauri::Builder::default()
        .setup(move |app| {
            let window = tauri::WindowBuilder::new(
                &app.app_handle(),
                "video-submit",
                tauri::WindowUrl::App("upload.html".into()),
            )
            .title("投稿配置")
            .visible(false)
            .inner_size(400.0, 800.0)
            .theme(Some(Theme::Light))
            .build()
            .unwrap();
            window
                .set_min_size(Some(LogicalSize {
                    width: 400,
                    height: 800,
                }))
                .unwrap();
            let state = State {
                client,
                post_window: window,
                config: config.clone(),
                recorder_manager: recorder_manager.clone(),
                app_handle: app.handle().clone(),
            };
            app.manage(state);
            Ok(())
        })
        .system_tray(tray)
        .on_window_event(|event| {
            if let WindowEvent::CloseRequested { api, .. } = event.event() {
                event.window().hide().unwrap();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_summary,
            add_recorder,
            remove_recorder,
            get_config,
            set_cache_path,
            set_output_path,
            set_admins,
            clip,
            clip_range,
            prepare_upload,
            upload_procedure,
            show_in_folder,
            get_qr,
            get_qr_status,
            set_cookies,
            logout,
            open_live,
            get_accounts,
            get_room_info,
            get_archives,
            delete_archive,
        ])
        .on_system_tray_event(|app, event| match event {
            SystemTrayEvent::LeftClick {
                position: _,
                size: _,
                ..
            } => {
                let window = app.get_window("main").unwrap();
                window.show().unwrap();
            }
            SystemTrayEvent::MenuItemClick { id, .. } => match id.as_str() {
                "hide" => {
                    let window = app.get_window("main").unwrap();
                    window.hide().unwrap();
                }
                "quit" => {
                    std::process::exit(0);
                }
                _ => {}
            },
            _ => {}
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
    Ok(())
}
