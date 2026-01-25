use super::platforms::bilibili::api::BiliStream;
use super::platforms::douyin::stream_info::DouyinStream;
use thiserror::Error;

#[derive(Debug, Clone)]
pub enum Stream {
    BiliBili(BiliStream),
    Douyin(DouyinStream),
}

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
    #[error("Stream is freezed: {stream:#?}")]
    FreezedStream { stream: Stream },
    #[error("Stream is nearly expired: {expire}")]
    StreamExpired { expire: i64 },
    #[error("No room info provided")]
    NoRoomInfo,
    #[error("Invalid stream: {stream:#?}")]
    InvalidStream { stream: Stream },
    #[error("Stream is too slow: {stream:#?}")]
    SlowStream { stream: Stream },
    #[error("Header url is empty")]
    EmptyHeader,
    #[error("Header timestamp is invalid")]
    InvalidTimestamp,
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
    #[error("Format not found: {format}")]
    FormatNotFound { format: String },
    #[error("Codec not found: {codecs}")]
    CodecNotFound { codecs: String },
    #[error("Invalid cookies")]
    InvalidCookies,
    #[error("API error: {error}")]
    ApiError { error: String },
    #[error("Invalid value")]
    InvalidValue,
    #[error("Invalid response")]
    InvalidResponse,
    #[error("Invalid response json: {resp}")]
    InvalidResponseJson { resp: serde_json::Value },
    #[error("Invalid response status: {status}")]
    InvalidResponseStatus { status: reqwest::StatusCode },
    #[error("Upload cancelled")]
    UploadCancelled,
    #[error("Upload error: {err}")]
    UploadError { err: String },
    #[error("Client error: {0}")]
    ClientError(#[from] reqwest::Error),
    #[error("Security control error")]
    SecurityControlError,
    #[error("JavaScript runtime error: {0}")]
    JsRuntimeError(String),
    #[error("Update timeout")]
    UpdateTimeout,
    #[error("Unsupported stream")]
    UnsupportedStream,
    #[error("Empty record")]
    EmptyRecord,
    #[error("Not live")]
    NotLive,
}
