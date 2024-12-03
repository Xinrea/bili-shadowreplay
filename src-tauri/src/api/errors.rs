use custom_error::custom_error;

custom_error! {pub ApiError
    InvalidResponse = "Invalid response",
    InitClientError = "Client init error",
    InvalidCode = "Invalid Code",
    InvalidValue = "Invalid value",
    InvalidUrl = "Invalid url",
    InvalidFormat = "Invalid stream format",
    EmptyCache = "Empty cache",
    ClientError{err: reqwest::Error} = "Client error: {err}",
    IOError{err: std::io::Error} = "IO error: {err}",
}

impl From<reqwest::Error> for ApiError {
    fn from(e: reqwest::Error) -> Self {
        ApiError::ClientError { err: e }
    }
}

impl From<std::io::Error> for ApiError {
    fn from(e: std::io::Error) -> Self {
        ApiError::IOError { err: e }
    }
}

impl From<ApiError> for String {
    fn from(value: ApiError) -> Self {
        value.to_string()
    }
}
