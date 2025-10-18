pub mod account;
pub mod danmu;
pub mod entry;
pub mod errors;
pub mod events;
mod ffmpeg;
pub mod platforms;
pub mod traits;
use crate::danmu::DanmuStorage;
use crate::events::RecorderEvent;
use crate::{account::Account, platforms::PlatformType};
mod user_agent_generator;

use std::{
    fmt::Display,
    path::PathBuf,
    sync::{atomic, Arc},
};
use tokio::{
    sync::{broadcast, Mutex, RwLock},
    task::JoinHandle,
};

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
    account: Account,
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

    fn account(&self) -> &Account {
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
