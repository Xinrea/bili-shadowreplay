use std::time::{Duration, SystemTime};

use reqwest::header::HeaderMap;
use tokio::sync::RwLock;

use crate::DanmuStreamError;

pub struct ApiClient {
    client: reqwest::Client,
    base_cookie: String,
    buvid3: RwLock<String>,
    buvid3_updated_at: RwLock<SystemTime>,
}

impl ApiClient {
    pub fn new(cookies: &str) -> Self {
        let buvid3 = uuid::Uuid::new_v4().to_string();

        Self {
            client: reqwest::Client::new(),
            base_cookie: cookies.to_string(),
            buvid3: RwLock::new(buvid3),
            buvid3_updated_at: RwLock::new(SystemTime::now()),
        }
    }

    async fn get_current_cookie(&self) -> String {
        // Check if buvid3 needs to be refreshed (every 1 hour)
        let now = SystemTime::now();
        let last_updated = *self.buvid3_updated_at.read().await;

        if let Ok(elapsed) = now.duration_since(last_updated) {
            if elapsed >= Duration::from_secs(3600) {
                // Update buvid3
                let new_buvid3 = uuid::Uuid::new_v4().to_string();
                *self.buvid3.write().await = new_buvid3;
                *self.buvid3_updated_at.write().await = now;
            }
        }

        let buvid3 = self.buvid3.read().await.clone();
        format!("{};buvid3={}", self.base_cookie, buvid3)
    }

    pub async fn get(
        &self,
        url: &str,
        query: Option<&[(&str, &str)]>,
    ) -> Result<reqwest::Response, DanmuStreamError> {
        let cookie = self.get_current_cookie().await;
        let mut header = HeaderMap::new();
        header.insert("cookie", cookie.parse().unwrap());

        let resp = self
            .client
            .get(url)
            .query(query.unwrap_or_default())
            .headers(header)
            .timeout(Duration::from_secs(10))
            .send()
            .await?
            .error_for_status()?;

        Ok(resp)
    }
}
