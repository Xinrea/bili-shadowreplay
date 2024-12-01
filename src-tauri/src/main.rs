// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod database;
mod recorder;
mod recorder_manager;
mod tray;

use chrono::Utc;
use custom_error::custom_error;
use database::account::AccountRow;
use database::message::MessageRow;
use database::record::RecordRow;
use database::recorder::RecorderRow;
use database::video::VideoRow;
use database::Database;
use recorder::bilibili::errors::BiliClientError;
use recorder::bilibili::profile::Profile;
use recorder::bilibili::{BiliClient, QrInfo, QrStatus};
use recorder::danmu::DanmuEntry;
use recorder_manager::{RecorderInfo, RecorderList, RecorderManager};
use std::fs::File;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use tauri::utils::config::WindowEffectsConfig;
use tauri::{Manager, Theme, WindowEvent};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_sql::{Migration, MigrationKind};
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
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
    webid: String,
    webid_ts: i64,
    live_start_notify: bool,
    live_end_notify: bool,
    clip_notify: bool,
    post_notify: bool,
}

impl Config {
    pub fn load() -> Self {
        let app_dirs = AppDirs::new(Some("cn.vjoi.bili-shadowreplay"), false).unwrap();
        let config_path = app_dirs.config_dir.join("Conf.toml");
        if let Ok(content) = std::fs::read_to_string(config_path) {
            if let Ok(config) = toml::from_str(&content) {
                return config;
            }
        }
        let config = Config {
            webid: "".to_string(),
            webid_ts: 0,
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
            live_start_notify: true,
            live_end_notify: true,
            clip_notify: true,
            post_notify: true,
        };
        config.save();
        config
    }

    pub fn save(&self) {
        let content = toml::to_string(&self).unwrap();
        let app_dirs = AppDirs::new(Some("cn.vjoi.bili-shadowreplay"), false).unwrap();
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

    pub fn set_output_path(&mut self, path: &str) {
        self.output = path.into();
        self.save();
    }

    pub fn webid_expired(&self) -> bool {
        let now = chrono::Utc::now().timestamp();
        // expire in 20 hours
        now - self.webid_ts > 72000
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

#[derive(serde::Serialize)]
struct DiskInfo {
    disk: String,
    total: u64,
    free: u64,
}

#[tauri::command]
async fn get_disk_info(state: tauri::State<'_, State>) -> Result<DiskInfo, ()> {
    let cache = state.config.read().await.cache.clone();
    // check system disk info
    let disks = sysinfo::Disks::new_with_refreshed_list();
    // get cache disk info
    let mut disk_info = DiskInfo {
        disk: "".into(),
        total: 0,
        free: 0,
    };
    for disk in disks.list() {
        // if output is under disk mount point
        if cache.starts_with(disk.mount_point().to_str().unwrap()) {
            // if MacOS, using disk name
            #[cfg(target_os = "macos")]
            {
                disk_info.disk = disk.name().to_str().unwrap().into();
            }
            // if Windows, using disk mount point
            #[cfg(target_os = "windows")]
            {
                disk_info.disk = disk.mount_point().to_str().unwrap().into();
            }
            disk_info.total = disk.total_space();
            disk_info.free = disk.available_space();
            break;
        }
    }
    Ok(disk_info)
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
    if state.config.read().await.primary_uid == 0 || state.db.get_accounts().await?.is_empty() {
        is_primary = true;
    }
    let account = state.db.add_account(cookies).await?;
    if is_primary {
        state.config.write().await.webid = state.client.fetch_webid(&account).await?;
        state.config.write().await.webid_ts = chrono::Utc::now().timestamp();
        state.config.write().await.primary_uid = account.uid;
    }
    let account_info = state
        .client
        .get_user_info(&state.config.read().await.webid, &account, account.uid)
        .await?;
    state
        .db
        .update_account(
            account_info.user_id,
            &account_info.user_name,
            &account_info.user_avatar_url,
        )
        .await?;
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
    if (state.db.get_account(uid).await).is_ok() {
        state.config.write().await.primary_uid = uid;
        Ok(())
    } else {
        Err("Account not exist".into())
    }
}

#[tauri::command]
async fn add_recorder(state: tauri::State<'_, State>, room_id: u64) -> Result<RecorderRow, String> {
    let account = state
        .db
        .get_account(state.config.read().await.primary_uid)
        .await?;
    if state.config.read().await.webid_expired() {
        state.config.write().await.webid = state.client.fetch_webid(&account).await?;
        state.config.write().await.webid_ts = chrono::Utc::now().timestamp();
        log::info!("Webid expired, refetching");
    }
    match state
        .recorder_manager
        .add_recorder(
            &state.config.read().await.webid,
            &state.db,
            &account,
            room_id,
        )
        .await
    {
        Ok(()) => {
            let room = state.db.add_recorder(room_id).await?;
            state
                .db
                .new_message("添加直播间", &format!("添加了新直播间 {}", room_id))
                .await?;
            Ok(room)
        }
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
async fn remove_recorder(state: tauri::State<'_, State>, room_id: u64) -> Result<(), String> {
    match state.recorder_manager.remove_recorder(room_id).await {
        Ok(()) => {
            state
                .db
                .new_message("移除直播间", &format!("移除了直播间 {}", room_id))
                .await?;
            Ok(state.db.remove_recorder(room_id).await?)
        }
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
async fn get_config(state: tauri::State<'_, State>) -> Result<Config, ()> {
    Ok(state.config.read().await.clone())
}

#[tauri::command]
async fn set_cache_path(state: tauri::State<'_, State>, cache_path: String) -> Result<(), String> {
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
    if let Err(e) = copy_dir_all(&old_cache_path, &cache_path) {
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
async fn update_notify(
    state: tauri::State<'_, State>,
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
async fn set_output_path(state: tauri::State<'_, State>, output_path: String) -> Result<(), ()> {
    let mut config = state.config.write().await;
    let old_output_path = config.output.clone();
    if let Err(e) = copy_dir_all(&old_output_path, &output_path) {
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
async fn clip(state: tauri::State<'_, State>, room_id: u64, len: f64) -> Result<String, String> {
    println!("[invoke]clip room_id: {}, len: {}", room_id, len);
    state.clip(room_id, len).await
}

#[tauri::command]
async fn clip_range(
    state: tauri::State<'_, State>,
    cover: String,
    room_id: u64,
    ts: u64,
    x: f64,
    y: f64,
) -> Result<VideoRow, String> {
    log::info!(
        "Clip room_id: {}, ts: {}, start: {}, end: {}",
        room_id,
        ts,
        x,
        y
    );
    let file = state
        .recorder_manager
        .clip_range(&state.config.read().await.output, room_id, ts, x, y)
        .await?;
    // get file metadata from fs
    let metadata = std::fs::metadata(&file).map_err(|e| e.to_string())?;
    // get filename from path
    let filename = Path::new(&file)
        .file_name()
        .ok_or("Invalid file path")?
        .to_str()
        .ok_or("Invalid file path")?;
    // add video to db
    let video = state
        .db
        .add_video(&VideoRow {
            id: 0,
            status: 0,
            room_id,
            created_at: Utc::now().to_rfc3339(),
            cover: cover.clone(),
            file: filename.into(),
            length: (y - x) as i64,
            size: metadata.len() as i64,
            bvid: "".into(),
            title: "".into(),
            desc: "".into(),
            tags: "".into(),
            area: 0,
        })
        .await?;
    state
        .db
        .new_message(
            "生成新切片",
            &format!(
                "生成了房间 {} 的切片，长度 {:.1}s：{}",
                room_id,
                y - x,
                filename
            ),
        )
        .await?;
    if state.config.read().await.clip_notify {
        state
            .app_handle
            .notification()
            .builder()
            .title("BiliShadowReplay - 切片完成")
            .body(format!("生成了房间 {} 的切片: {}", room_id, filename))
            .show()
            .unwrap();
    }
    Ok(video)
}

#[tauri::command]
async fn upload_procedure(
    state: tauri::State<'_, State>,
    uid: u64,
    room_id: u64,
    video_id: i64,
    cover: String,
    mut profile: Profile,
) -> Result<String, String> {
    let account = state.db.get_account(uid).await?;
    // get video info from dbs
    let mut video_row = state.db.get_video(video_id).await?;
    // construct file path
    let output = state.config.read().await.output.clone();
    let file = format!("{}/{}", output, video_row.file);
    let path = Path::new(&file);
    let cover_url = state.client.upload_cover(&account, &cover);
    if let Ok(video) = state.client.prepare_video(&account, path).await {
        profile.cover = cover_url.await.unwrap_or("".to_string());
        if let Ok(ret) = state.client.submit_video(&account, &profile, &video).await {
            // update video status and details
            // 1 means uploaded
            video_row.status = 1;
            video_row.bvid = ret.bvid.clone();
            video_row.title = profile.title;
            video_row.desc = profile.desc;
            video_row.tags = profile.tag;
            video_row.area = profile.tid as i64;
            state.db.update_video(&video_row).await?;
            state
                .db
                .new_message(
                    "投稿成功",
                    &format!("投稿了房间 {} 的切片：{}", room_id, ret.bvid),
                )
                .await?;
            if state.config.read().await.post_notify {
                state
                    .app_handle
                    .notification()
                    .builder()
                    .title("BiliShadowReplay - 投稿成功")
                    .body(format!("投稿了房间 {} 的切片: {}", room_id, ret.bvid))
                    .show()
                    .unwrap();
            }
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
) -> Result<Vec<RecordRow>, String> {
    log::debug!("Get archives for {}", room_id);
    Ok(state.recorder_manager.get_archives(room_id).await?)
}

#[tauri::command]
async fn get_archive(
    state: tauri::State<'_, State>,
    room_id: u64,
    live_id: u64,
) -> Result<RecordRow, String> {
    Ok(state.recorder_manager.get_archive(room_id, live_id).await?)
}

#[tauri::command]
async fn delete_archive(
    state: tauri::State<'_, State>,
    room_id: u64,
    ts: u64,
) -> Result<(), String> {
    state.recorder_manager.delete_archive(room_id, ts).await?;
    state
        .db
        .new_message(
            "删除历史缓存",
            &format!("删除了房间 {} 的历史缓存 {}", room_id, ts),
        )
        .await?;
    Ok(())
}

#[tauri::command]
async fn send_danmaku(
    state: tauri::State<'_, State>,
    uid: u64,
    room_id: u64,
    message: String,
) -> Result<(), String> {
    let account = state.db.get_account(uid).await?;
    state
        .client
        .send_danmaku(&account, room_id, &message)
        .await?;
    Ok(())
}

#[tauri::command]
async fn get_danmu_record(
    state: tauri::State<'_, State>,
    room_id: u64,
    ts: u64,
) -> Result<Vec<DanmuEntry>, String> {
    Ok(state.recorder_manager.get_danmu(room_id, ts).await?)
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
    log::info!("Open player window: {} {}", room_id, ts);
    let addr = state.recorder_manager.get_hls_server_addr().await.unwrap();
    let recorder_info = state
        .recorder_manager
        .get_recorder_info(room_id)
        .await
        .unwrap();
    let handle = state.app_handle.clone();
    let builder = tauri::WebviewWindowBuilder::new(
        &handle,
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
    .inner_size(1200.0, 800.0)
    .effects(WindowEffectsConfig {
        effects: vec![
            tauri_utils::WindowEffect::Tabbed,
            tauri_utils::WindowEffect::Mica,
        ],
        state: None,
        radius: None,
        color: None,
    });
    #[cfg(target_os = "macos")]
    {
        if let Err(e) = builder.decorations(true).build() {
            log::error!("live window build failed: {}", e);
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Err(e) = builder.decorations(false).transparent(true).build() {
            log::error!("live window build failed: {}", e);
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Err(e) = builder.decorations(true).build() {
            log::error!("live window build failed: {}", e);
        }
    }
    Ok(())
}

#[tauri::command]
async fn get_messages(state: tauri::State<'_, State>) -> Result<Vec<MessageRow>, String> {
    Ok(state.db.get_messages().await?)
}

#[tauri::command]
async fn read_message(state: tauri::State<'_, State>, id: i64) -> Result<(), String> {
    Ok(state.db.read_message(id).await?)
}

#[tauri::command]
async fn delete_message(state: tauri::State<'_, State>, id: i64) -> Result<(), String> {
    Ok(state.db.delete_message(id).await?)
}

#[tauri::command]
async fn get_video(state: tauri::State<'_, State>, id: i64) -> Result<VideoRow, String> {
    Ok(state.db.get_video(id).await?)
}

#[tauri::command]
async fn get_videos(state: tauri::State<'_, State>, room_id: u64) -> Result<Vec<VideoRow>, String> {
    Ok(state.db.get_videos(room_id).await?)
}

#[tauri::command]
async fn delete_video(state: tauri::State<'_, State>, id: i64) -> Result<(), String> {
    // get video info from dbus
    let video = state.db.get_video(id).await?;
    // delete video files
    let filepath = format!("{}/{}", state.config.read().await.output, video.file);
    let file = Path::new(&filepath);
    if let Err(e) = std::fs::remove_file(file) {
        log::error!("Delete video file error: {}", e);
    }
    Ok(state.db.delete_video(id).await?)
}

#[tauri::command]
async fn get_video_typelist(
    state: tauri::State<'_, State>,
) -> Result<Vec<recorder::bilibili::response::Typelist>, String> {
    let account = state
        .db
        .get_account(state.config.read().await.primary_uid)
        .await?;
    Ok(state.client.get_video_typelist(&account).await?)
}

#[tauri::command]
async fn export_to_file(
    _state: tauri::State<'_, State>,
    file_name: &str,
    content: &str,
) -> Result<(), String> {
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(file_name)
        .await;
    if file.is_err() {
        return Err(format!("Open file failed: {}", file.err().unwrap()));
    }
    let mut file = file.unwrap();
    if let Err(e) = file.write_all(content.as_bytes()).await {
        return Err(format!("Write file failed: {}", e));
    }
    if let Err(e) = file.flush().await {
        return Err(format!("Flush file failed: {}", e));
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Setup log
    simplelog::CombinedLogger::init(vec![
        simplelog::TermLogger::new(
            simplelog::LevelFilter::Info,
            simplelog::Config::default(),
            simplelog::TerminalMode::Mixed,
            simplelog::ColorChoice::Auto,
        ),
        simplelog::WriteLogger::new(
            simplelog::LevelFilter::Info,
            simplelog::Config::default(),
            File::create("bsr.log").unwrap(),
        ),
    ])
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
            CREATE TABLE records (live_id INTEGER PRIMARY KEY, room_id INTEGER, title TEXT, length INTEGER, size INTEGER, created_at TEXT);
            CREATE TABLE danmu_statistics (live_id INTEGER PRIMARY KEY, room_id INTEGER, value INTEGER, time_point TEXT);
            CREATE TABLE messages (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT, content TEXT, read INTEGER, created_at TEXT);
            CREATE TABLE videos (id INTEGER PRIMARY KEY AUTOINCREMENT, room_id INTEGER, cover TEXT, file TEXT, length INTEGER, size INTEGER, status INTEGER, bvid TEXT, title TEXT, desc TEXT, tags TEXT, area INTEGER, created_at TEXT);
            "#,
        kind: MigrationKind::Up,
    }];

    // Tauri part
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            let _ = app
                .get_webview_window("main")
                .expect("no main window")
                .set_focus();
        }))
        .plugin(tauri_plugin_sql::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_os::init())
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
            let recorder_manager =
                Arc::new(RecorderManager::new(app.handle().clone(), config.clone()));
            let recorder_manager_clone = recorder_manager.clone();
            let dbs = app.state::<tauri_plugin_sql::DbInstances>().inner();
            let db = Arc::new(Database::new());
            let db_clone = db.clone();
            let client_clone = client.clone();
            tauri::async_runtime::block_on(async move {
                let _ = recorder_manager_clone.run_hls().await;
                let binding = dbs.0.read().await;
                let dbpool = binding.get("sqlite:data.db").unwrap();
                let sqlite_pool = match dbpool {
                    tauri_plugin_sql::DbPool::Sqlite(pool) => Some(pool),
                };
                db_clone.set(sqlite_pool.unwrap().clone()).await;
                let initial_rooms = db_clone.get_recorders().await.unwrap();
                let mut primary_uid = config_clone.read().await.primary_uid;
                let accounts = db_clone.get_accounts().await.unwrap();
                if accounts.is_empty() {
                    log::warn!("No account found");
                    return;
                }
                if primary_uid == 0 {
                    primary_uid = accounts.first().unwrap().uid;
                    config_clone.write().await.primary_uid = primary_uid;
                    config_clone.write().await.save();
                }
                let primary_account = accounts
                    .iter()
                    .find(|x| x.uid == primary_uid)
                    .unwrap()
                    .clone();
                let webid = client_clone.fetch_webid(&primary_account).await.unwrap();
                config_clone.write().await.webid = webid.clone();
                config_clone.write().await.webid_ts = chrono::Utc::now().timestamp();
                // update account infos
                for account in accounts {
                    match client_clone
                        .get_user_info(&webid, &primary_account, account.uid)
                        .await
                    {
                        Ok(account_info) => {
                            if let Err(e) = db_clone
                                .update_account(
                                    account_info.user_id,
                                    &account_info.user_name,
                                    &account_info.user_avatar_url,
                                )
                                .await
                            {
                                log::error!("Error when updating account info {}", e);
                            }
                        }
                        Err(e) => {
                            log::error!("Get user info failed {}", e);
                        }
                    }
                }
                let account = db_clone.get_account(primary_uid).await;
                if let Ok(account) = account {
                    for room in initial_rooms {
                        if let Err(e) = recorder_manager_clone
                            .add_recorder(&webid, &db_clone, &account, room.room_id)
                            .await
                        {
                            log::error!("error when adding initial rooms: {}", e);
                        }
                    }
                } else {
                    log::warn!("No available account found");
                }
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
            get_archive,
            get_archives,
            delete_archive,
            get_messages,
            read_message,
            delete_message,
            get_video,
            get_videos,
            delete_video,
            get_disk_info,
            send_danmaku,
            update_notify,
            get_danmu_record,
            get_video_typelist,
            export_to_file
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
    Ok(())
}
