// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod recorder;

use custom_error::custom_error;
use recorder::bilibili::errors::BiliClientError;
use recorder::bilibili::{BiliClient, QrInfo, QrStatus};
use recorder::BiliRecorder;
use std::collections::HashMap;

use std::process::Command;
use std::sync::Arc;
use tauri::{CustomMenuItem, SystemTray, SystemTrayEvent, SystemTrayMenu, SystemTrayMenuItem};
use tauri::{Manager, WindowEvent};
use tokio::sync::{Mutex, RwLock};

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
    max_len: u64,
    cache: String,
    output: String,
    login: bool,
    uid: String,
    cookies: String,
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
            max_len: 300,
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

    pub fn set_max_len(&mut self, mut len: u64) {
        if len < 30 {
            len = 30;
        }
        self.max_len = len;
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
    client: Arc<Mutex<BiliClient>>,
    config: Arc<RwLock<Config>>,
    recorders: Arc<Mutex<HashMap<u64, Arc<RwLock<BiliRecorder>>>>>,
}

#[derive(serde::Deserialize, serde::Serialize, Clone)]
struct Summary {
    pub count: usize,
    pub rooms: Vec<RoomInfo>,
}

#[derive(serde::Deserialize, serde::Serialize, Clone)]
struct RoomInfo {
    pub room_id: u64,
    pub room_title: String,
    pub room_cover: String,
    pub room_keyframe: String,
    pub user_id: u64,
    pub user_name: String,
    pub user_sign: String,
    pub user_avatar: String,
    pub total_length: f64,
    pub max_len: u64,
    pub live_status: bool,
}

impl State {
    pub async fn get_summary(&self) -> Summary {
        let recorders = self.recorders.lock().await;
        let mut summary = Summary {
            count: recorders.len(),
            rooms: Vec::new(),
        };

        for (_, recorder) in recorders.iter() {
            let recorder = recorder.read().await;
            let room_info = RoomInfo {
                room_id: recorder.room_id,
                room_title: recorder.room_title.clone(),
                room_cover: recorder.room_cover.clone(),
                room_keyframe: recorder.room_keyframe.clone(),
                user_id: recorder.user_id,
                user_name: recorder.user_name.clone(),
                user_sign: recorder.user_sign.clone(),
                user_avatar: recorder.user_avatar.clone(),
                total_length: *recorder.ts_length.read().await,
                max_len: self.config.read().await.max_len,
                live_status: *recorder.live_status.read().await,
            };
            summary.rooms.push(room_info);
        }

        summary.rooms.sort_by(|a, b| a.room_id.cmp(&b.room_id));
        summary
    }

    pub async fn get_qr(client: Arc<Mutex<BiliClient>>) -> Result<QrInfo, BiliClientError> {
        client.lock().await.get_qr().await
    }

    pub async fn get_qr_status(&self, key: &str) -> Result<QrStatus, BiliClientError> {
        self.client.lock().await.get_qr_status(key).await
    }

    pub async fn add_recorder(&self, room_id: u64) -> Result<(), StateError> {
        if self.recorders.lock().await.get(&room_id).is_some() {
            return Err(StateError::RecorderAlreadyExists);
        }
        match BiliRecorder::new(room_id, self.config.clone()).await {
            Ok(recorder) => {
                recorder.run().await;
                let recorder = Arc::new(RwLock::new(recorder));
                self.recorders.lock().await.insert(room_id, recorder);
                self.config.write().await.add(room_id);
                Ok(())
            }
            Err(e) => {
                println!("create recorder failed: {:?}", e);
                Err(StateError::RecorderCreateError)
            }
        }
    }

    pub async fn remove_recorder(&self, room_id: u64) {
        let mut recorders = self.recorders.lock().await;
        let recorder = recorders.get_mut(&room_id).unwrap();
        recorder.read().await.stop().await;
        recorders.remove(&room_id);
        self.config.write().await.remove(room_id);
    }

    pub async fn clip(&self, room_id: u64, len: f64) -> Result<String, String> {
        let recorders = self.recorders.lock().await;
        let recorder = recorders.get(&room_id).unwrap().clone();
        if let Ok(file) = recorder.clone().read().await.clip(room_id, len).await {
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
    match State::get_qr(state.client.clone()).await {
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
async fn set_max_len(state: tauri::State<'_, State>, len: u64) -> Result<(), ()> {
    let mut config = state.config.write().await;
    config.set_max_len(len);
    Ok(())
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
async fn init_recorders(state: tauri::State<'_, State>) -> Result<(), ()> {
    println!("[invoke]init recorders");
    let cookies = state.config.read().await.cookies.clone();
    let rooms = state.config.read().await.rooms.clone();
    let mut client = state.client.lock().await;
    client.set_cookies(&cookies);
    for room_id in rooms.iter() {
        if let Err(e) = state.add_recorder(*room_id).await {
            println!("init recorder failed: {:?}", e);
        }
    }
    Ok(())
}

#[tauri::command]
async fn set_cookies(state: tauri::State<'_, State>, cookies: String) -> Result<(), String> {
    let mut client = state.client.lock().await;
    let mut config = state.config.write().await;
    config.set_cookies(&cookies);
    client.set_cookies(&cookies);
    let recorders = state.recorders.lock().await;
    for (_, recorder) in recorders.iter() {
        recorder.write().await.update_cookies(&cookies).await;
    }
    Ok(())
}

#[tauri::command]
async fn logout(state: tauri::State<'_, State>) -> Result<(), String> {
    let mut client = state.client.lock().await;
    let mut config = state.config.write().await;
    config.logout();
    client.logout();
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
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
    // Setup initial state
    let state = State {
        client: Arc::new(Mutex::new(BiliClient::new()?)),
        config: Arc::new(RwLock::new(Config::load())),
        recorders: Arc::new(Mutex::new(HashMap::new())),
    };

    // Tauri part
    tauri::Builder::default()
        .manage(state)
        .system_tray(tray)
        .on_window_event(|event| {
            if let WindowEvent::CloseRequested { api, .. } = event.event() {
                event.window().hide().unwrap();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            init_recorders,
            get_summary,
            add_recorder,
            remove_recorder,
            get_config,
            set_max_len,
            set_cache_path,
            set_output_path,
            set_admins,
            clip,
            show_in_folder,
            get_qr,
            get_qr_status,
            set_cookies,
            logout,
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
