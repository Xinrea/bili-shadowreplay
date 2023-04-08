use custom_error::custom_error;

custom_error! {pub BiliClientError
    InvalidResponse = "Invalid response",
    InitClientError = "Client init error",
    InvalidValue = "Invalid value",
    InvalidIndex = "Invalid index",
    InvalidPlaylist = "Invalid playlist",
    InvalidUrl = "Invalid url",
    InvalidFormat = "Invalid stream format",
    EmptyCache = "Empty cache",
    ClientError{err: reqwest::Error} = "Client error",
}

impl From<reqwest::Error> for BiliClientError {
    fn from(e: reqwest::Error) -> Self {
        BiliClientError::ClientError { err: e }
    }
}
