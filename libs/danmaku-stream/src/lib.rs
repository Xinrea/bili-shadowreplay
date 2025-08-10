pub mod stream;
mod http_client;
pub mod provider;

use custom_error::custom_error;

custom_error! {pub DanmakuStreamError
    HttpError {err: reqwest::Error} = "HttpError {err}",
    ParseError {err: url::ParseError} = "ParseError {err}",
    WebsocketError {err: String } = "WebsocketError {err}",
    PackError {err: String} = "PackError {err}",
    UnsupportProto {proto: u16} = "UnsupportProto {proto}",
    MessageParseError {err: String} = "MessageParseError {err}",
    InvalidIdentifier {err: String} = "InvalidIdentifier {err}"
}

#[derive(Debug)]
pub enum DanmakuMessageType {
    DanmuMessage(DanmakuMessage),
}

#[derive(Debug, Clone)]
pub struct DanmakuMessage {
    pub room_id: u64,
    pub user_id: u64,
    pub user_name: String,
    pub message: String,
    pub color: u32,
    /// timestamp in milliseconds
    pub timestamp: i64,
}
