mod bilibili;
mod douyin;
mod kuaishou;

use async_trait::async_trait;
use tokio::sync::mpsc;

use self::bilibili::BiliDanmu;
use self::douyin::DouyinDanmu;
use self::kuaishou::KuaishouDanmu;

use crate::{DanmuMessageType, DanmuStreamError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderType {
    BiliBili,
    Douyin,
    Kuaishou,
}

#[async_trait]
pub trait DanmuProvider: Send + Sync {
    async fn new(identifier: &str, room_id: &str) -> Result<Self, DanmuStreamError>
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
/// * `provider_type` - The type of platform to fetch danmu from (BiliBili or Douyin)
/// * `identifier` - User validation information (e.g., cookies) required by the platform
/// * `room_id` - The unique identifier of the room/channel to fetch danmu from. Notice that douyin room_id is more like a live_id, it changes every time the live starts.
///
/// # Returns
///
/// Returns a provider whose `start` method initializes the websocket:
/// * `Ok(...)` indicates successful provider initialization
/// * `Err(DanmuStreamError)` indicates an error occurred during initialization
///
/// # Examples
///
/// ```no_run
/// use danmu_stream::provider::{new, ProviderType};
///
/// #[tokio::main]
/// async fn main() {
///     let _provider = new(ProviderType::BiliBili, "your_cookie", "123456")
///         .await
///         .expect("provider initialization failed");
/// }
/// ```
pub async fn new(
    provider_type: ProviderType,
    identifier: &str,
    room_id: &str,
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
        ProviderType::Kuaishou => {
            let kuaishou = KuaishouDanmu::new(identifier, room_id).await?;
            Ok(Box::new(kuaishou))
        }
    }
}
