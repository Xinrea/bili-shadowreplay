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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display_messages() {
        let e = RecorderError::IndexNotFound {
            url: "https://example.com".to_string(),
        };
        assert_eq!(format!("{}", e), "Index not found: https://example.com");

        let e = RecorderError::EmptyCache;
        assert_eq!(format!("{}", e), "Cache is empty");

        let e = RecorderError::NoStreamAvailable;
        assert_eq!(format!("{}", e), "No available stream provided");

        let e = RecorderError::NoRoomInfo;
        assert_eq!(format!("{}", e), "No room info provided");

        let e = RecorderError::EmptyHeader;
        assert_eq!(format!("{}", e), "Header url is empty");

        let e = RecorderError::InvalidTimestamp;
        assert_eq!(format!("{}", e), "Header timestamp is invalid");

        let e = RecorderError::InvalidCookies;
        assert_eq!(format!("{}", e), "Invalid cookies");

        let e = RecorderError::InvalidValue;
        assert_eq!(format!("{}", e), "Invalid value");

        let e = RecorderError::InvalidResponse;
        assert_eq!(format!("{}", e), "Invalid response");

        let e = RecorderError::UploadCancelled;
        assert_eq!(format!("{}", e), "Upload cancelled");

        let e = RecorderError::SecurityControlError;
        assert_eq!(format!("{}", e), "Security control error");

        let e = RecorderError::UpdateTimeout;
        assert_eq!(format!("{}", e), "Update timeout");

        let e = RecorderError::UnsupportedStream;
        assert_eq!(format!("{}", e), "Unsupported stream");

        let e = RecorderError::EmptyRecord;
        assert_eq!(format!("{}", e), "Empty record");

        let e = RecorderError::NotLive;
        assert_eq!(format!("{}", e), "Not live");
    }

    #[test]
    fn test_error_display_with_fields() {
        let e = RecorderError::ArchiveInUse {
            live_id: "abc123".to_string(),
        };
        assert!(format!("{}", e).contains("abc123"));

        let e = RecorderError::M3u8ParseFailed {
            content: "bad content".to_string(),
        };
        assert!(format!("{}", e).contains("bad content"));

        let e = RecorderError::StreamExpired { expire: 1700000000 };
        assert!(format!("{}", e).contains("1700000000"));

        let e = RecorderError::ApiError {
            error: "rate limited".to_string(),
        };
        assert!(format!("{}", e).contains("rate limited"));

        let e = RecorderError::FfmpegError("codec error".to_string());
        assert!(format!("{}", e).contains("codec error"));

        let e = RecorderError::SubtitleNotFound {
            live_id: "live1".to_string(),
        };
        assert!(format!("{}", e).contains("live1"));

        let e = RecorderError::UploadError {
            err: "timeout".to_string(),
        };
        assert!(format!("{}", e).contains("timeout"));

        let e = RecorderError::JsRuntimeError("eval failed".to_string());
        assert!(format!("{}", e).contains("eval failed"));
    }

    #[test]
    fn test_error_from_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let e: RecorderError = io_err.into();
        assert!(format!("{}", e).contains("file not found"));
    }
}
