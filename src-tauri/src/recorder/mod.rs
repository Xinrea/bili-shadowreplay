pub mod bilibili;
pub mod danmu;
pub mod douyin;
pub mod errors;
pub mod traits;
use crate::{
    database::account::AccountRow, recorder::danmu::DanmuStorage, recorder_manager::RecorderEvent,
};
mod user_agent_generator;

pub mod entry;

use async_trait::async_trait;
use std::{
    fmt::Display,
    hash::{Hash, Hasher},
    path::PathBuf,
    sync::{atomic, Arc},
};
use tokio::{
    sync::{broadcast, Mutex, RwLock},
    task::JoinHandle,
};

use crate::{database::Database, progress::progress_reporter::ProgressReporterTrait};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformType {
    BiliBili,
    Douyin,
    Huya,
    Youtube,
}

impl PlatformType {
    pub fn as_str(&self) -> &'static str {
        match self {
            PlatformType::BiliBili => "bilibili",
            PlatformType::Douyin => "douyin",
            PlatformType::Huya => "huya",
            PlatformType::Youtube => "youtube",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "bilibili" => Some(PlatformType::BiliBili),
            "douyin" => Some(PlatformType::Douyin),
            "huya" => Some(PlatformType::Huya),
            "youtube" => Some(PlatformType::Youtube),
            _ => None,
        }
    }
}

impl Hash for PlatformType {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
    }
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug)]
pub struct RecorderInfo {
    pub room_info: RoomInfo,
    pub user_info: UserInfo,
    pub platform_live_id: String,
    pub live_id: String,
    pub recording: bool,
    pub enabled: bool,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, Default)]
pub struct RoomInfo {
    pub platform: String,
    pub room_id: String,
    pub room_title: String,
    pub room_cover: String,
    /// Whether the room is live
    pub status: bool,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, Default)]
pub struct UserInfo {
    pub user_id: String,
    pub user_name: String,
    pub user_avatar: String,
}

/// `Recorder` is the base struct for all recorders
/// It contains the basic information for a recorder
/// and the extra information for the recorder
#[derive(Clone)]
pub struct Recorder<T>
where
    T: Send + Sync,
{
    platform: PlatformType,
    room_id: i64,
    /// The account for the recorder
    account: AccountRow,
    /// The client for the recorder
    client: reqwest::Client,
    /// The event channel for the recorder
    event_channel: broadcast::Sender<RecorderEvent>,
    /// The cache directory for the recorder
    cache_dir: PathBuf,
    /// Whether the recorder is quitting
    quit: Arc<atomic::AtomicBool>,
    /// Whether the recorder is enabled
    enabled: Arc<atomic::AtomicBool>,
    /// Whether the recorder is recording
    is_recording: Arc<atomic::AtomicBool>,
    /// The room info for the recorder
    room_info: Arc<RwLock<RoomInfo>>,
    /// The user info for the recorder
    user_info: Arc<RwLock<UserInfo>>,

    /// The platform live id for the current recording
    platform_live_id: Arc<RwLock<String>>,
    /// The live id for the current recording, generally is the timestamp of the recording start time
    live_id: Arc<RwLock<String>>,
    /// The danmu task for the current recording
    danmu_task: Arc<Mutex<Option<JoinHandle<()>>>>,
    /// The record task for the current recording
    record_task: Arc<Mutex<Option<JoinHandle<()>>>>,
    /// The danmu storage for the current recording
    danmu_storage: Arc<RwLock<Option<DanmuStorage>>>,
    /// The last update time of the current recording
    last_update: Arc<atomic::AtomicI64>,
    /// The last sequence of the current recording
    last_sequence: Arc<atomic::AtomicU64>,
    /// The total duration of the current recording in milliseconds
    total_duration: Arc<atomic::AtomicU64>,
    /// The total size of the current recording in bytes
    total_size: Arc<atomic::AtomicU64>,

    /// The extra information for the recorder
    extra: T,
}

impl<T: Send + Sync> traits::RecorderBasicTrait<T> for Recorder<T> {
    fn platform(&self) -> PlatformType {
        self.platform
    }

    fn room_id(&self) -> i64 {
        self.room_id
    }

    fn account(&self) -> &AccountRow {
        &self.account
    }

    fn client(&self) -> &reqwest::Client {
        &self.client
    }

    fn event_channel(&self) -> &broadcast::Sender<RecorderEvent> {
        &self.event_channel
    }

    fn cache_dir(&self) -> PathBuf {
        self.cache_dir.clone()
    }

    fn quit(&self) -> &atomic::AtomicBool {
        &self.quit
    }

    fn enabled(&self) -> &atomic::AtomicBool {
        &self.enabled
    }

    fn is_recording(&self) -> &atomic::AtomicBool {
        &self.is_recording
    }

    fn room_info(&self) -> Arc<RwLock<RoomInfo>> {
        self.room_info.clone()
    }

    fn user_info(&self) -> Arc<RwLock<UserInfo>> {
        self.user_info.clone()
    }

    fn platform_live_id(&self) -> Arc<RwLock<String>> {
        self.platform_live_id.clone()
    }

    fn live_id(&self) -> Arc<RwLock<String>> {
        self.live_id.clone()
    }

    fn danmu_task(&self) -> Arc<Mutex<Option<JoinHandle<()>>>> {
        self.danmu_task.clone()
    }

    fn record_task(&self) -> Arc<Mutex<Option<JoinHandle<()>>>> {
        self.record_task.clone()
    }

    fn danmu_storage(&self) -> Arc<RwLock<Option<DanmuStorage>>> {
        self.danmu_storage.clone()
    }

    fn last_update(&self) -> &atomic::AtomicI64 {
        &self.last_update
    }

    fn last_sequence(&self) -> &atomic::AtomicU64 {
        &self.last_sequence
    }

    fn total_duration(&self) -> &atomic::AtomicU64 {
        &self.total_duration
    }

    fn total_size(&self) -> &atomic::AtomicU64 {
        &self.total_size
    }

    fn extra(&self) -> &T {
        &self.extra
    }
}

pub struct FfmpegProgressHandler {
    db: Arc<Database>,
    live_id: Arc<RwLock<String>>,
    total_duration: Arc<RwLock<f64>>,
    total_size: Arc<RwLock<u64>>,
    work_dir: PathBuf,
}

impl Clone for FfmpegProgressHandler {
    fn clone(&self) -> Self {
        Self {
            db: self.db.clone(),
            live_id: self.live_id.clone(),
            total_duration: self.total_duration.clone(),
            total_size: self.total_size.clone(),
            work_dir: self.work_dir.clone(),
        }
    }
}

#[async_trait]
impl ProgressReporterTrait for FfmpegProgressHandler {
    fn update(&self, content: &str) {
        if let Ok(duration) = content.parse::<i64>() {
            let duration_secs = duration as f64 / 1_000_000.0;
            let db = self.db.clone();
            let live_id = self.live_id.clone();
            let total_duration = self.total_duration.clone();
            let total_size = self.total_size.clone();
            let work_dir = self.work_dir.clone();
            tokio::spawn(async move {
                // get all ts files in work_dir
                let mut entries = tokio::fs::read_dir(&work_dir).await.unwrap();
                let mut file_size: u64 = 0;
                while let Some(entry) = entries.next_entry().await.unwrap() {
                    if let Ok(metadata) = entry.metadata().await {
                        file_size += metadata.len();
                    }
                }
                let _ = db
                    .update_record(
                        live_id.read().await.as_str(),
                        duration_secs as i64,
                        file_size,
                    )
                    .await;
                *total_duration.write().await = duration_secs;
                *total_size.write().await = file_size;
            });
        }
    }

    async fn finish(&self, _success: bool, message: &str) {
        log::debug!("[FFmpeg Finish] {}", message);
    }
}

/// Cache path is relative to cache path in config
#[derive(Clone)]
pub struct CachePath {
    pub cache_path: PathBuf,
    pub platform: PlatformType,
    pub room_id: i64,
    pub live_id: String,
    pub file_name: Option<String>,
}

impl CachePath {
    pub fn new(cache_path: PathBuf, platform: PlatformType, room_id: i64, live_id: &str) -> Self {
        Self {
            cache_path,
            platform,
            room_id,
            live_id: live_id.to_string(),
            file_name: None,
        }
    }

    /// Sanitize filename and set it
    pub fn with_filename(&self, file_name: &str) -> Self {
        let sanitized_filename = sanitize_filename::sanitize(file_name);
        Self {
            file_name: Some(sanitized_filename),
            ..self.clone()
        }
    }

    /// Get relative path to cache path
    pub fn relative_path(&self) -> PathBuf {
        if let Some(file_name) = &self.file_name {
            return PathBuf::from(format!(
                "{}/{}/{}/{}",
                self.platform.as_str(),
                self.room_id,
                self.live_id,
                file_name
            ));
        }

        PathBuf::from(format!(
            "{}/{}/{}",
            self.platform.as_str(),
            self.room_id,
            self.live_id
        ))
    }

    pub fn full_path(&self) -> PathBuf {
        self.cache_path.clone().join(self.relative_path())
    }
}

impl Display for CachePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.full_path().display())
    }
}
