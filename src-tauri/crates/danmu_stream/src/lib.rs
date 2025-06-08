pub mod danmu_stream;
mod http_client;
pub mod provider;

use custom_error::custom_error;

custom_error! {pub DanmmuStreamError
    HttpError {err: reqwest::Error} = "HttpError {err}",
    ParseError {err: url::ParseError} = "ParseError {err}",
    WebsocketError {err: String } = "WebsocketError {err}",
    PackError {err: String} = "PackError {err}",
    UnsupportProto {proto: u16} = "UnsupportProto {proto}",
    MessageParseError {err: String} = "MessageParseError {err}",
    InvalidIdentifier {err: String} = "InvalidIdentifier {err}"
}

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
    pub timestamp: i64,
}
