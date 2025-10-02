pub mod bilibili;
pub mod danmu;
pub mod douyin;
pub mod errors;
mod user_agent_generator;

pub mod entry;

use async_trait::async_trait;
use danmu::DanmuEntry;
use m3u8_rs::MediaPlaylist;
use std::{
    hash::{Hash, Hasher},
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
}

impl Clone for FfmpegProgressHandler {
    fn clone(&self) -> Self {
        Self {
            db: self.db.clone(),
            live_id: self.live_id.clone(),
            total_duration: self.total_duration.clone(),
        }
    }
}

#[async_trait]
impl ProgressReporterTrait for FfmpegProgressHandler {
    fn update(&self, content: &str) {
        if let Ok(duration) = content.parse::<i64>() {
            let duration_secs = duration as f64 / 1000_000.0;
            let db = self.db.clone();
            let live_id = self.live_id.clone();
            let total_duration = self.total_duration.clone();
            tokio::spawn(async move {
                let _ = db
                    .update_record(live_id.read().await.as_str(), duration_secs as i64, 0)
                    .await;
                *total_duration.write().await = duration_secs;
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
