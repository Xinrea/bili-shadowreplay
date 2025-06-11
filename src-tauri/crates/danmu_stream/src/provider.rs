mod bilibili;
mod douyin;

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::{
    provider::bilibili::BiliDanmu, provider::douyin::DouyinDanmu, DanmuMessageType,
    DanmuStreamError,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderType {
    BiliBili,
    Douyin,
}

#[async_trait]
pub trait DanmuProvider: Send + Sync {
    async fn new(identifier: &str, room_id: u64) -> Result<Self, DanmuStreamError>
    where
        Self: Sized;

    async fn start(
        &self,
        tx: mpsc::UnboundedSender<DanmuMessageType>,
    ) -> Result<(), DanmuStreamError>;

    async fn stop(&self) -> Result<(), DanmuStreamError>;
}

/// Creates a new danmu stream provider for the specified platform.
///
/// This function initializes and starts a danmu stream provider based on the specified platform type.
/// The provider will fetch danmu messages and send them through the provided channel.
///
/// # Arguments
///
/// * `tx` - An unbounded sender channel that will receive danmu messages
/// * `provider_type` - The type of platform to fetch danmu from (BiliBili or Douyin)
/// * `identifier` - User validation information (e.g., cookies) required by the platform
/// * `room_id` - The unique identifier of the room/channel to fetch danmu from. Notice that douyin room_id is more like a live_id, it changes every time the live starts.
///
/// # Returns
///
/// Returns `Result<(), DanmmuStreamError>` where:
/// * `Ok(())` indicates successful initialization and start of the provider, only return after disconnect
/// * `Err(DanmmuStreamError)` indicates an error occurred during initialization or startup
///
/// # Examples
///
/// ```rust
/// use tokio::sync::mpsc;
/// let (tx, mut rx) = mpsc::unbounded_channel();
/// new(tx, ProviderType::BiliBili, "your_cookie", 123456).await?;
/// ```
pub async fn new(
    provider_type: ProviderType,
    identifier: &str,
    room_id: u64,
) -> Result<Box<dyn DanmuProvider>, DanmuStreamError> {
    match provider_type {
        ProviderType::BiliBili => {
            let bili = BiliDanmu::new(identifier, room_id).await?;
            Ok(Box::new(bili))
        }
        ProviderType::Douyin => {
            let douyin = DouyinDanmu::new(identifier, room_id).await?;
            Ok(Box::new(douyin))
        }
    }
}
