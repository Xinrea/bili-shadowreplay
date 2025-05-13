use async_trait::async_trait;
use custom_error::custom_error;
use tokio::sync::mpsc;

custom_error! {pub DanmuProviderError
    ConnectionError = "Failed to establish a connection"
}

pub enum DanmuMessageType {}

pub enum ProviderType {
    BiliBili,
    Douyin,
}

/// Construct a new provider to fetch danmu stream.
/// _tx_ : receiving danmu messages from channel;
/// _identifier_: user validation info(cookies, etc), might be required by platform.
pub async fn new(
    tx: mpsc::UnboundedSender<DanmuMessageType>,
    provider_type: ProviderType,
    identifier: &str,
    room_id: &str,
) -> Result<(), DanmuProviderError> {
    todo!("")
}
