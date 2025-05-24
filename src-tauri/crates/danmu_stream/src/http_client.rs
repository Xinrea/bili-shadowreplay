use std::time::Duration;

use custom_error::custom_error;
use reqwest::header::HeaderMap;

custom_error! {pub ApiError
    HttpError {err: reqwest::Error} = "HttpError {err}",
    ParseError {err: url::ParseError} = "ParseError {err}"
}

impl From<reqwest::Error> for ApiError {
    fn from(value: reqwest::Error) -> Self {
        Self::HttpError { err: value }
    }
}

impl From<url::ParseError> for ApiError {
    fn from(value: url::ParseError) -> Self {
        Self::ParseError { err: value }
    }
}

pub struct ApiClient {
    client: reqwest::Client,
    base_url: url::Url,
    header: HeaderMap,
}

impl ApiClient {
    pub fn new(base_url: &str, cookies: &str) -> Self {
        let mut header = HeaderMap::new();
        header.insert("cookie", cookies.parse().unwrap());

        Self {
            client: reqwest::Client::new(),
            base_url: base_url.parse().unwrap(),
            header,
        }
    }

    pub async fn get(
        &self,
        path: &str,
        query: Option<&[(&str, &str)]>,
    ) -> Result<reqwest::Response, ApiError> {
        let resp = self
            .client
            .get(self.base_url.join(path)?)
            .query(query.unwrap_or_default())
            .headers(self.header.clone())
            .timeout(Duration::from_secs(10))
            .send()
            .await?
            .error_for_status()?;

        Ok(resp)
    }
}
