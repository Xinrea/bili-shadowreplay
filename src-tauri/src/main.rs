// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod recorder;

use custom_error::custom_error;
use recorder::BiliRecorder;
use std::collections::HashMap;

use std::process::Command;
use std::sync::{Arc, Mutex};
use tauri::{CustomMenuItem, SystemTray, SystemTrayEvent, SystemTrayMenu, SystemTrayMenuItem};
use tauri::{Manager, WindowEvent};

use platform_dirs::AppDirs;

custom_error! {StateError
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
    config: Arc<Mutex<Config>>,
    recorders: Arc<Mutex<HashMap<u64, BiliRecorder>>>,
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
    pub fn get_summary(&self) -> Summary {
        let recorders = self.recorders.lock().unwrap();
        recorders.iter().fold(
            Summary {
                count: recorders.len(),
                rooms: Vec::new(),
            },
            |mut summary, (_, recorder)| {
                let room_info = RoomInfo {
                    room_id: recorder.room_id,
                    room_title: recorder.room_title.clone(),
                    room_cover: recorder.room_cover.clone(),
                    room_keyframe: recorder.room_keyframe.clone(),
                    user_id: recorder.user_id,
                    user_name: recorder.user_name.clone(),
                    user_sign: recorder.user_sign.clone(),
                    user_avatar: recorder.user_avatar.clone(),
                    total_length: *recorder.ts_length.read().unwrap(),
                    max_len: self.config.lock().unwrap().max_len,
                    live_status: *recorder.live_status.read().unwrap(),
                };
                summary.rooms.push(room_info);
                summary
            },
        )
    }

    pub fn add_recorder(&self, room_id: u64) -> Result<(), StateError> {
        let mut recorders = self.recorders.lock().unwrap();
        if recorders.get(&room_id).is_some() {
            Err(StateError::RecorderAlreadyExists)
        } else if let Ok(recorder) = BiliRecorder::new(room_id, self.config.clone()) {
            recorder.run();
            recorders.insert(room_id, recorder);
            Ok(())
        } else {
            Err(StateError::RecorderCreateError)
        }
    }

    pub fn remove_recorder(&self, room_id: u64) {
        let mut recorders = self.recorders.lock().unwrap();
        let recorder = recorders.get_mut(&room_id).unwrap();
        recorder.stop();
        recorders.remove(&room_id);
    }

    pub fn clip(&self, room_id: u64, len: f64) -> Result<String, String> {
        let recorders = self.recorders.lock().unwrap();
        let recorder = recorders.get(&room_id).unwrap();
        if let Ok(file) = recorder.clip(room_id, len) {
            Ok(file)
        } else {
            Err("Clip error".to_string())
        }
    }
}

#[tauri::command]
async fn get_summary(state: tauri::State<'_, State>) -> Result<Summary, ()> {
    Ok(state.get_summary())
}

#[tauri::command]
fn add_recorder(state: tauri::State<State>, room_id: u64) -> Result<(), String> {
    // Config update
    let mut config = state.config.lock().unwrap();
    if config.rooms.contains(&room_id) {
        return Err("Room already exists".to_string());
    }
    if let Err(e) = state.add_recorder(room_id) {
        Err(e.to_string())
    } else {
        config.add(room_id);
        Ok(())
    }
}

#[tauri::command]
fn remove_recorder(state: tauri::State<State>, room_id: u64) {
    // Config update
    let mut config = state.config.lock().unwrap();
    config.remove(room_id);
    state.remove_recorder(room_id)
}

#[tauri::command]
fn get_config(state: tauri::State<State>) -> Config {
    state.config.lock().unwrap().clone()
}

#[tauri::command]
fn set_max_len(state: tauri::State<State>, len: u64) {
    let mut config = state.config.lock().unwrap();
    config.set_max_len(len);
}

#[tauri::command]
fn set_cache_path(state: tauri::State<State>, cache_path: String) {
    let mut config = state.config.lock().unwrap();
    let old_cache_path = config.cache.clone();
    config.set_cache_path(&cache_path);
    drop(config);
    // Remove old cache
    if old_cache_path != cache_path {
        if let Err(e) = std::fs::remove_dir_all(old_cache_path) {
            println!("Remove old cache error: {}", e);
        }
    }
}

#[tauri::command]
fn set_output_path(state: tauri::State<State>, output_path: String) {
    let mut config = state.config.lock().unwrap();
    config.set_output_path(output_path);
}

#[tauri::command]
fn set_admins(state: tauri::State<State>, admins: Vec<u64>) {
    let mut config = state.config.lock().unwrap();
    config.set_admins(admins);
}

#[tauri::command]
fn clip(state: tauri::State<State>, room_id: u64, len: f64) -> Result<String, String> {
    state.clip(room_id, len)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Setup ffmpeg
    ffmpeg_sidecar::download::auto_download().unwrap();
    // Setup tray icon
    let quit = CustomMenuItem::new("quit".to_string(), "Quit");
    let hide = CustomMenuItem::new("hide".to_string(), "Hide");
    let tray_menu = SystemTrayMenu::new()
        .add_item(quit)
        .add_native_item(SystemTrayMenuItem::Separator)
        .add_item(hide);
    let tray = SystemTray::new().with_menu(tray_menu);
    // Setup initial state
    let state = State {
        config: Arc::new(Mutex::new(Config::load())),
        recorders: Arc::new(Mutex::new(HashMap::new())),
    };
    let conf = Config::load();
    for room_id in conf.rooms {
        std::fs::remove_dir_all(format!("{}/{}", conf.cache, room_id)).unwrap_or(());
        if state.add_recorder(room_id).is_err() {
            continue;
        }
    }
    // Tauri part
    tauri::Builder::default()
        .manage(state)
        .system_tray(tray)
        .on_window_event(|event| match event.event() {
            WindowEvent::CloseRequested { api, .. } => {
                event.window().hide().unwrap();
                api.prevent_close();
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            get_summary,
            add_recorder,
            remove_recorder,
            get_config,
            set_max_len,
            set_cache_path,
            set_output_path,
            set_admins,
            clip,
            show_in_folder
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
                "quit" => {
                    std::process::exit(0);
                }
                "hide" => {
                    let window = app.get_window("main").unwrap();
                    window.hide().unwrap();
                }
                _ => {}
            },
            _ => {}
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
    Ok(())
}
