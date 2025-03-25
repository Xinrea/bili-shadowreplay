use custom_error::custom_error;

custom_error! {pub BiliClientError
    InvalidResponse = "Invalid response",
    InitClientError = "Client init error",
    InvalidCode = "Invalid Code",
    InvalidValue = "Invalid value",
    InvalidUrl = "Invalid url",
    InvalidFormat = "Invalid stream format",
    UploadError{err: String} = "Upload error: {err}",
    EmptyCache = "Empty cache",
    ClientError{err: reqwest::Error} = "Client error: {err}",
    IOError{err: std::io::Error} = "IO error: {err}",
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
