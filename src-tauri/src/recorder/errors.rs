use crate::api::errors::ApiError;
use crate::database::DatabaseError;
use custom_error::custom_error;

custom_error! {pub RecorderError
    IndexNotFound {url: String} = "Index not found: {url}",
    ArchiveInUse {ts: u64} = "Can not delete current stream: {ts}",
    EmptyCache = "Cache is empty",
    M3u8ParseFailed {content: String } = "Parse m3u8 content failed: {content}",
    NoStreamAvailable = "No available stream provided",
    FreezedStream {stream_info: String} = "Stream is freezed: {stream_info}",
    InvalidStream {stream_info: String} = "Invalid stream: {stream_info}",
    SlowStream {stream_info: String} = "Stream is too slow: {stream_info}",
    EmptyHeader = "Header url is empty",
    InvalidTimestamp = "Header timestamp is invalid",
    InvalidDBOP {err: DatabaseError } = "Database error: {err}",
    ClientError {err: ApiError} = "BiliClient error: {err}",
    ClipError {err: String} = "FFMPEG error: {err}",
    IoError {err: std::io::Error} = "IO error: {err}",
}
