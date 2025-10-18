use thiserror::Error;

#[derive(Error, Debug)]
pub enum HuyaClientError {
    #[error("Invalid response")]
    InvalidResponse,
    #[error("Client init error")]
    InitClientError,
    #[error("Invalid response status: {status}")]
    InvalidResponseStatus { status: reqwest::StatusCode },
    #[error("Invalid response json: {resp}")]
    InvalidResponseJson { resp: serde_json::Value },
    #[error("Invalid message code: {code}")]
    InvalidMessageCode { code: u64 },
    #[error("Invalid value")]
    InvalidValue,
    #[error("Invalid url")]
    InvalidUrl,
    #[error("Invalid stream format")]
    InvalidFormat,
    #[error("Invalid stream")]
    InvalidStream,
    #[error("Invalid cookie")]
    InvalidCookie,
    #[error("Upload error: {err}")]
    UploadError { err: String },
    #[error("Upload was cancelled by user")]
    UploadCancelled,
    #[error("Empty cache")]
    EmptyCache,
    #[error("Client error: {0}")]
    ClientError(#[from] reqwest::Error),
    #[error("IO error: {0}")]
    IOError(#[from] std::io::Error),
    #[error("Security control error")]
    SecurityControlError,
    #[error("API error: {0}")]
    ApiError(String),
    #[error("Format not found: {0}")]
    FormatNotFound(String),
    #[error("Codec not found: {0}")]
    CodecNotFound(String),
    #[error("Extractor error: {0}")]
    ExtractorError(String),
}

impl From<HuyaClientError> for String {
    fn from(err: HuyaClientError) -> Self {
        err.to_string()
    }
}
