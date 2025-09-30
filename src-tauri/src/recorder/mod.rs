pub mod bilibili;
pub mod danmu;
pub mod douyin;
pub mod errors;
pub mod huya;
use crate::{recorder::danmu::DanmuStorage, recorder_manager::RecorderEvent};
mod user_agent_generator;

pub mod entry;

use async_trait::async_trait;
use m3u8_rs::MediaPlaylist;
use std::{
    hash::{Hash, Hasher},
    path::PathBuf,
    sync::{atomic, Arc},
};
use tokio::sync::{broadcast, Mutex, RwLock};
use tokio::task::JoinHandle;

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
    pub live_id: String,
    pub recording: bool,
    pub enabled: bool,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug)]
pub struct RoomInfo {
    pub platform: String,
    pub room_id: String,
    pub room_title: String,
    pub room_cover: String,
    /// Whether the room is live
    pub status: bool,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug)]
pub struct UserInfo {
    pub user_id: String,
    pub user_name: String,
    pub user_avatar: String,
}

/// `Recorder` is the base struct for all recorders
/// It contains the basic information for a recorder
/// and the extra information for the recorder
#[derive(Clone)]
pub struct Recorder<T> {
    platform: PlatformType,
    room_id: i64,
    /// The client for the recorder, cookies should be preset
    client: reqwest::Client,
    /// The event channel for the recorder
    event_channel: broadcast::Sender<RecorderEvent>,
    /// The cache directory for the recorder
    cache_dir: PathBuf,
    /// Whether the recorder is quitting
    quit: atomic::AtomicBool,
    /// Whether the recorder is enabled
    enabled: atomic::AtomicBool,
    /// Whether the recorder is recording
    is_recording: atomic::AtomicBool,
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
    /// The m3u8 playlist for the current recording
    m3u8_playlist: Arc<RwLock<MediaPlaylist>>,
    /// The last update time of the current recording
    last_update: atomic::AtomicI64,
    /// The last sequence of the current recording
    last_sequence: atomic::AtomicU64,
    /// The total duration of the current recording in milliseconds
    total_duration: atomic::AtomicU64,
    /// The total size of the current recording in bytes
    total_size: atomic::AtomicU64,
    /// The extra information for the recorder
    extra: T,
}

#[async_trait]
pub trait RecorderTrait: Send + Sync + 'static {
    async fn run(&self);
    async fn stop(&self) {
        self.quit.store(true, atomic::Ordering::Relaxed);
    }
    async fn info(&self) -> RecorderInfo;
    async fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, atomic::Ordering::Relaxed);
    }
}
