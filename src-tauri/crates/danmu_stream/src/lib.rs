pub mod danmu_stream;
mod http_client;
pub mod provider;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DanmuStreamError {
    #[error("HttpError {0:?}")]
    HttpError(#[from] reqwest::Error),
    #[error("BilibiliApiError code={code}: {message}")]
    ApiError { code: i64, message: String },
    #[error("ParseError {0:?}")]
    ParseError(#[from] url::ParseError),
    #[error("WebsocketError {err}")]
    WebsocketError { err: String },
    #[error("PackError {err}")]
    PackError { err: String },
    #[error("UnsupportProto {proto}")]
    UnsupportProto { proto: u16 },
    #[error("MessageParseError {err}")]
    MessageParseError { err: String },
    #[error("InvalidIdentifier {err}")]
    InvalidIdentifier { err: String },
}

#[derive(Debug)]
pub enum DanmuMessageType {
    /// A normalized event received from a platform websocket.
    Event(LiveEvent),
    /// Kept for providers which have not yet migrated to `Event`.
    DanmuMessage(DanmuMessage),
}

/// The on-disk/websocket-independent representation of a live-room event.
///
/// `data` contains the normalized, platform-independent fields while `raw`
/// keeps the original provider payload.  Keeping the latter is intentional:
/// websocket protocols add fields frequently and an event must not be lost
/// just because this crate does not know how to interpret a new field yet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveEvent {
    pub ts: i64,
    pub platform: String,
    pub room_id: String,
    #[serde(rename = "type")]
    pub event_type: String,
    pub data: Value,
    pub raw: Value,
}

impl LiveEvent {
    pub fn new(platform: &str, room_id: &str, event_type: &str, data: Value) -> Self {
        Self {
            ts: chrono::Utc::now().timestamp_millis(),
            platform: platform.to_string(),
            room_id: room_id.to_string(),
            event_type: event_type.to_string(),
            data,
            raw: Value::Null,
        }
    }

    pub fn danmu(message: DanmuMessage, platform: &str) -> Self {
        Self {
            ts: message.timestamp,
            platform: platform.to_string(),
            room_id: message.room_id.clone(),
            event_type: "danmu".to_string(),
            data: json!({
                "user_id": message.user_id,
                "user_name": message.user_name,
                "content": message.message,
                "color": message.color,
            }),
            raw: Value::Null,
        }
    }

    pub fn with_raw(mut self, raw: Value) -> Self {
        self.raw = raw;
        self
    }
}

#[derive(Debug, Clone)]
pub struct DanmuMessage {
    pub room_id: String,
    pub user_id: u64,
    pub user_name: String,
    pub message: String,
    pub color: u32,
    /// timestamp in milliseconds
    pub timestamp: i64,
}
