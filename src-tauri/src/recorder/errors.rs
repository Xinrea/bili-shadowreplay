use custom_error::custom_error;
use super::bilibili::client::BiliStream;

custom_error! {pub RecorderError
    IndexNotFound {url: String} = "Index not found: {url}",
    ArchiveInUse {ts: u64} = "Can not delete current stream: {ts}",
    EmptyCache = "Cache is empty",
    M3u8ParseFailed {content: String } = "Parse m3u8 content failed: {content}",
    NoStreamAvailable = "No available stream provided",
    FreezedStream {stream: BiliStream} = "Stream is freezed: {stream}",
    InvalidStream {stream: BiliStream} = "Invalid stream: {stream}",
    SlowStream {stream: BiliStream} = "Stream is too slow: {stream}",
    EmptyHeader = "Header url is empty",
    InvalidTimestamp = "Header timestamp is invalid",
    InvalidDBOP {err: crate::database::DatabaseError } = "Database error: {err}",
    ClientError {err: super::bilibili::errors::BiliClientError} = "BiliClient error: {err}",
    ClipError {err: String} = "FFMPEG error: {err}",
    IoError {err: std::io::Error} = "IO error: {err}",
}
