mod bilibili;

use super::http_client;
use custom_error::custom_error;
use tokio::sync::mpsc;

custom_error! {pub DanmuProviderError
    InvalidIdentifier {err: String} = "Invalid identifier: {err}",
    ApiError { err: http_client::ApiError } = "Failed to fetch api: {err}"
}

impl From<http_client::ApiError> for DanmuProviderError {
    fn from(value: http_client::ApiError) -> Self {
        Self::ApiError { err: value }
    }
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
