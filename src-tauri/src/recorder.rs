pub mod bilibili;
pub mod douyin;
pub mod danmu;
pub mod errors;

use async_trait::async_trait;
use danmu::DanmuEntry;
use crate::recorder::bilibili::client::UserInfo;
use crate::recorder::bilibili::client::RoomInfo;

#[derive(Clone)]
pub struct TsEntry {
    pub url: String,
    pub offset: u64,
    pub sequence: u64,
    pub length: f64,
    pub size: u64,
}

pub enum RecorderType {
    BiliBili,
    Douyin,
    Huya,
    Youtube,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug)]
pub struct RecorderInfo {
    pub room_id: u64,
    pub room_info: RoomInfo,
    pub user_info: UserInfo,
    pub total_length: f64,
    pub current_ts: u64,
    pub live_status: bool,
}

#[async_trait]
pub trait Recorder: Send + Sync + 'static {
    fn recorder_type(&self) -> RecorderType;
    async fn run(&self);
    async fn stop(&self);
    async fn clip_range(&self, live_id: u64, x: f64, y: f64, output_path: &str) -> Result<String, errors::RecorderError>;
    async fn m3u8_content(&self, live_id: u64) -> String;
    async fn info(&self) -> RecorderInfo;
    async fn comments(&self, live_id: u64) -> Result<Vec<DanmuEntry>, errors::RecorderError>;
}