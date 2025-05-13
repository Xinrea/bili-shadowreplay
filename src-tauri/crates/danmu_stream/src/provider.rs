mod bilibili;

use tokio::sync::mpsc;

use crate::{DanmmuStreamError, DanmuMessageType, provider::bilibili::BiliDanmu};

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
) -> Result<(), DanmmuStreamError> {
    let room_id = room_id
        .parse::<u64>()
        .map_err(|e| DanmmuStreamError::InvalidIdentifier { err: e.to_string() })?;
    match provider_type {
        ProviderType::BiliBili => {
            let bili = BiliDanmu::new(identifier, room_id).await?;
            bili.start(tx).await?;
            Ok(())
        }
        ProviderType::Douyin => {
            panic!("Douyin is not supported yet");
        }
    }
}
