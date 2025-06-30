use custom_error::custom_error;

custom_error! {pub BiliClientError
    InvalidResponse = "Invalid response",
    InitClientError = "Client init error",
    InvalidResponseStatus{ status: reqwest::StatusCode } = "Invalid response status: {status}",
    InvalidResponseJson{ resp: serde_json::Value } = "Invalid response json: {resp}",
    InvalidMessageCode{ code: u64 } = "Invalid message code: {code}",
    InvalidValue = "Invalid value",
    InvalidUrl = "Invalid url",
    InvalidFormat = "Invalid stream format",
    InvalidStream = "Invalid stream",
    InvalidCookie = "Invalid cookie",
    UploadError{err: String} = "Upload error: {err}",
    UploadCancelled = "Upload was cancelled by user",
    EmptyCache = "Empty cache",
    ClientError{err: reqwest::Error} = "Client error: {err}",
    IOError{err: std::io::Error} = "IO error: {err}",
    SecurityControlError = "Security control error",
}

impl From<reqwest::Error> for BiliClientError {
    fn from(e: reqwest::Error) -> Self {
        BiliClientError::ClientError { err: e }
    }
}

impl From<std::io::Error> for BiliClientError {
    fn from(e: std::io::Error) -> Self {
        BiliClientError::IOError { err: e }
    }
}

impl From<BiliClientError> for String {
    fn from(value: BiliClientError) -> Self {
        value.to_string()
    }
}
