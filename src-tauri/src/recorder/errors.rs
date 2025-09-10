use super::bilibili::client::BiliStream;
use super::douyin::client::DouyinClientError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum RecorderError {
    #[error("Index not found: {url}")]
    IndexNotFound { url: String },
    #[error("Can not delete current stream: {live_id}")]
    ArchiveInUse { live_id: String },
    #[error("Cache is empty")]
    EmptyCache,
    #[error("Parse m3u8 content failed: {content}")]
    M3u8ParseFailed { content: String },
    #[error("No available stream provided")]
    NoStreamAvailable,
    #[error("Stream is freezed: {stream}")]
    FreezedStream { stream: BiliStream },
    #[error("Stream is nearly expired: {stream}")]
    StreamExpired { stream: BiliStream },
    #[error("No room info provided")]
    NoRoomInfo,
    #[error("Invalid stream: {stream}")]
    InvalidStream { stream: BiliStream },
    #[error("Stream is too slow: {stream}")]
    SlowStream { stream: BiliStream },
    #[error("Header url is empty")]
    EmptyHeader,
    #[error("Header timestamp is invalid")]
    InvalidTimestamp,
    #[error("Database error: {0}")]
    InvalidDBOP(#[from] crate::database::DatabaseError),
    #[error("BiliClient error: {0}")]
    BiliClientError(#[from] super::bilibili::errors::BiliClientError),
    #[error("DouyinClient error: {0}")]
    DouyinClientError(#[from] DouyinClientError),
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Danmu stream error: {0}")]
    DanmuStreamError(#[from] danmu_stream::DanmuStreamError),
    #[error("Subtitle not found: {live_id}")]
    SubtitleNotFound { live_id: String },
    #[error("Subtitle generation failed: {error}")]
    SubtitleGenerationFailed { error: String },
    #[error("Resolution changed: {err}")]
    ResolutionChanged { err: String },
    #[error("Ffmpeg error: {0}")]
    FfmpegError(String),
}
