pub mod bilibili;
pub mod douyin;

use async_trait::async_trait;

use crate::{HlsSegment, HlsStreamError, StreamInfo};

#[derive(Debug, Clone, Copy)]
pub enum ProviderType {
    Bilibili,
    Douyin,
}

impl ProviderType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProviderType::Bilibili => "bilibili",
            ProviderType::Douyin => "douyin",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "bilibili" => Some(ProviderType::Bilibili),
            "douyin" => Some(ProviderType::Douyin),
            _ => None,
        }
    }
}

#[async_trait]
pub trait HlsProvider: Send + Sync {
    /// Fetch and parse playlist (returns segment metadata only)
    async fn fetch_playlist(&self) -> Result<Vec<HlsSegment>, HlsStreamError>;

    /// Get stream information
    async fn get_info(&self) -> Result<StreamInfo, HlsStreamError>;

    /// Change quality
    async fn change_quality(&mut self, quality: &str) -> Result<(), HlsStreamError>;

    /// Stop parsing
    async fn stop(&mut self) -> Result<(), HlsStreamError>;
}

pub async fn create_provider(
    provider_type: ProviderType,
    room_id: &str,
    auth: &str,
) -> Result<Box<dyn HlsProvider>, HlsStreamError> {
    match provider_type {
        ProviderType::Bilibili => Ok(Box::new(
            bilibili::BilibiliProvider::new(room_id, auth).await?,
        )),
        ProviderType::Douyin => Ok(Box::new(douyin::DouyinProvider::new(room_id, auth).await?)),
    }
}
