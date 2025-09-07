pub mod danmu_stream;
mod http_client;
pub mod provider;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum DanmuStreamError {
    #[error("HttpError {0:?}")]
    HttpError(#[from] reqwest::Error),
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
    DanmuMessage(DanmuMessage),
}

#[derive(Debug, Clone)]
pub struct DanmuMessage {
    pub room_id: u64,
    pub user_id: u64,
    pub user_name: String,
    pub message: String,
    pub color: u32,
    /// timestamp in milliseconds
    pub timestamp: i64,
}
