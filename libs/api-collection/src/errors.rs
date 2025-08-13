use custom_error::custom_error;

custom_error! {pub ApiCollectionError
    RequestError {err: String} = "Request error: {err}",
    InvalidValue {key: String, value: String} = "Invalid value: {key}: {value}",
    RiskControlError = "Risk control error",
    IOError {err: String} = "IO error: {err}",
    UploadError {err: String} = "Upload error: {err}",
}

impl From<reqwest::Error> for ApiCollectionError {
    fn from(err: reqwest::Error) -> Self {
        ApiCollectionError::RequestError {
            err: err.to_string(),
        }
    }
}

impl From<std::io::Error> for ApiCollectionError {
    fn from(err: std::io::Error) -> Self {
        ApiCollectionError::IOError {
            err: err.to_string(),
        }
    }
}
