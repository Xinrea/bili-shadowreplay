use custom_error::custom_error;

custom_error! {pub BiliClientError
    InvalidResponse = "Invalid response",
    InitClientError = "Client init error",
    InvalidCode = "Invalid Code",
    InvalidValue = "Invalid value",
    InvalidUrl = "Invalid url",
    InvalidFormat = "Invalid stream format",
    EmptyCache = "Empty cache",
    ClientError{err: reqwest::Error} = "Client error",
    IOError{err: std::io::Error} = "IO error",
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
