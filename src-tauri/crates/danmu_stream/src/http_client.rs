use std::time::Duration;

use crate::DanmmuStreamError;
use reqwest::header::HeaderMap;

impl From<reqwest::Error> for DanmmuStreamError {
    fn from(value: reqwest::Error) -> Self {
        Self::HttpError { err: value }
    }
}

impl From<url::ParseError> for DanmmuStreamError {
    fn from(value: url::ParseError) -> Self {
        Self::ParseError { err: value }
    }
}

pub struct ApiClient {
    client: reqwest::Client,
    header: HeaderMap,
}

impl ApiClient {
    pub fn new(cookies: &str) -> Self {
        let mut header = HeaderMap::new();
        header.insert("cookie", cookies.parse().unwrap());

        Self {
            client: reqwest::Client::new(),
            header,
        }
    }

    pub async fn get(
        &self,
        url: &str,
        query: Option<&[(&str, &str)]>,
    ) -> Result<reqwest::Response, DanmmuStreamError> {
        let resp = self
            .client
            .get(url)
            .query(query.unwrap_or_default())
            .headers(self.header.clone())
            .timeout(Duration::from_secs(10))
            .send()
            .await?
            .error_for_status()?;

        Ok(resp)
    }
}
