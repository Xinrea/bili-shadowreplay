pub mod danmu_stream;
mod http_client;
pub mod provider;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;

/// Image metadata attached to a danmaku message. Text remains the export content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmojiContent {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emoticon_unique: Option<String>,
}

impl EmojiContent {
    pub fn from_value(value: &Value) -> Option<Self> {
        let url = value.get("url")?.as_str()?.to_string();
        if url.trim().is_empty() {
            return None;
        }
        Some(Self {
            url,
            width: value.get("width").and_then(Value::as_u64),
            height: value.get("height").and_then(Value::as_u64),
            emoticon_unique: value
                .get("emoticon_unique")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
    }

    /// Bilibili marks standalone image danmaku in the first info array.
    pub fn from_bilibili_raw(raw: &Value) -> Option<Self> {
        let header = raw.get("info")?.get(0)?.as_array()?;
        if header.get(12).and_then(Value::as_i64) != Some(1) {
            return None;
        }
        Self::from_value(header.get(13)?)
    }
}

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
    /// Also supports recordings written before image metadata was normalized.
    pub fn emoji_content(&self) -> Option<EmojiContent> {
        if self.event_type != "danmu" {
            return None;
        }
        self.data
            .get("emoji_content")
            .and_then(EmojiContent::from_value)
            .or_else(|| {
                (self.platform == "bilibili")
                    .then(|| EmojiContent::from_bilibili_raw(&self.raw))
                    .flatten()
            })
    }

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
