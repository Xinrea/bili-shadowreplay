use std::time::Duration;

use reqwest::header::HeaderMap;

use crate::DanmuStreamError;

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
    ) -> Result<reqwest::Response, DanmuStreamError> {
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
