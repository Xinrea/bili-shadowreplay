pub mod bilibili;
pub mod danmu;
pub mod douyin;
pub mod errors;

mod entry;

use std::fmt::Display;

use async_trait::async_trait;
use danmu::DanmuEntry;

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

impl Display for PlatformType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug)]
pub struct RecorderInfo {
    pub room_id: u64,
    pub room_info: RoomInfo,
    pub user_info: UserInfo,
    pub total_length: f32,
    pub current_live_id: String,
    pub live_status: bool,
    pub is_recording: bool,
    pub auto_start: bool,
    pub platform: String,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug)]
pub struct RoomInfo {
    pub room_id: u64,
    pub room_title: String,
    pub room_cover: String,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug)]
pub struct UserInfo {
    pub user_id: String,
    pub user_name: String,
    pub user_avatar: String,
}

#[async_trait]
pub trait Recorder: Send + Sync + 'static {
    async fn run(&self);
    async fn stop(&self);
    async fn m3u8_content(&self, live_id: &str, start: i64, end: i64) -> String;
    async fn info(&self) -> RecorderInfo;
    async fn comments(&self, live_id: &str) -> Result<Vec<DanmuEntry>, errors::RecorderError>;
    async fn is_recording(&self, live_id: &str) -> bool;
    async fn force_start(&self);
    async fn force_stop(&self);
    async fn set_auto_start(&self, auto_start: bool);
}
