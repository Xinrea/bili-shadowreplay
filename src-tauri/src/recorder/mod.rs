pub mod bilibili;
pub mod danmu;
pub mod douyin;
pub mod errors;
pub mod huya;
mod user_agent_generator;

pub mod entry;

use async_trait::async_trait;
use danmu::DanmuEntry;
use m3u8_rs::MediaPlaylist;
use std::{
    fmt::Display,
    hash::{Hash, Hasher},
    path::PathBuf,
    sync::Arc,
};
use tokio::sync::RwLock;

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
    pub room_id: i64,
    pub room_info: RoomInfo,
    pub user_info: UserInfo,
    pub total_length: f64,
    pub current_live_id: String,
    pub live_status: bool,
    pub is_recording: bool,
    pub auto_start: bool,
    pub platform: String,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug)]
pub struct RoomInfo {
    pub room_id: i64,
    pub room_title: String,
    pub room_cover: String,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug)]
pub struct UserInfo {
    pub user_id: String,
    pub user_name: String,
    pub user_avatar: String,
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

#[async_trait]
pub trait Recorder: Send + Sync + 'static {
    async fn run(&self);
    async fn stop(&self);
    async fn playlist(&self, live_id: &str, start: i64, end: i64) -> MediaPlaylist;
    async fn get_related_playlists(&self, parent_id: &str) -> Vec<(String, String)>;
    async fn info(&self) -> RecorderInfo;
    async fn comments(&self, live_id: &str) -> Result<Vec<DanmuEntry>, errors::RecorderError>;
    async fn is_recording(&self, live_id: &str) -> bool;
    async fn get_archive_subtitle(&self, live_id: &str) -> Result<String, errors::RecorderError>;
    async fn generate_archive_subtitle(
        &self,
        live_id: &str,
    ) -> Result<String, errors::RecorderError>;
    async fn enable(&self);
    async fn disable(&self);
}

/// Cache path is relative to cache path in config
#[derive(Clone)]
pub struct CachePath {
    pub cache_path: String,
    pub platform: PlatformType,
    pub room_id: i64,
    pub live_id: String,
    pub file_name: Option<String>,
}

impl CachePath {
    pub fn new(cache_path: &str, platform: PlatformType, room_id: i64, live_id: &str) -> Self {
        Self {
            cache_path: cache_path.to_string(),
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
        PathBuf::from(self.cache_path.clone()).join(self.relative_path())
    }
}

impl Display for CachePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.full_path().display())
    }
}
