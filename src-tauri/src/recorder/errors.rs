use custom_error::custom_error;
use super::bilibili::client::BiliStream;
use super::douyin::client::DouyinClientError;

custom_error! {pub RecorderError
    IndexNotFound {url: String} = "Index not found: {url}",
    ArchiveInUse {live_id: String} = "Can not delete current stream: {live_id}",
    EmptyCache = "Cache is empty",
    M3u8ParseFailed {content: String } = "Parse m3u8 content failed: {content}",
    NoStreamAvailable = "No available stream provided",
    FreezedStream {stream: BiliStream} = "Stream is freezed: {stream}",
    NoRoomInfo = "No room info provided",
    InvalidStream {stream: BiliStream} = "Invalid stream: {stream}",
    SlowStream {stream: BiliStream} = "Stream is too slow: {stream}",
    EmptyHeader = "Header url is empty",
    InvalidTimestamp = "Header timestamp is invalid",
    InvalidDBOP {err: crate::database::DatabaseError } = "Database error: {err}",
    BiliClientError {err: super::bilibili::errors::BiliClientError} = "BiliClient error: {err}",
    DouyinClientError {err: DouyinClientError} = "DouyinClient error: {err}",
    ClipError {err: String} = "FFMPEG error: {err}",
    IoError {err: std::io::Error} = "IO error: {err}",
}
