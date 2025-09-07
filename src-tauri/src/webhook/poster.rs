//! Webhook Event Poster
//!
//! This module provides functionality for posting webhook events to external URLs.
//! It includes retry logic, custom headers support, and proper error handling.
//!
//! # Examples
//!
//! ## Basic Usage
//! ```rust,no_run
//! use std::collections::HashMap;
//! use bili_shadowreplay::webhook::poster::create_webhook_poster_with_headers;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//! let mut headers = HashMap::new();
//! headers.insert("Authorization".to_string(), "Bearer token".to_string());
//!
//! let poster = create_webhook_poster_with_headers("https://api.example.com/webhook", headers)?;
//! // Use the poster...
//! # Ok(())
//! # }
//! ```
//!
//! ## Custom Configuration
//! ```rust,no_run
//! use std::time::Duration;
//! use bili_shadowreplay::webhook::poster::{WebhookPoster, WebhookConfig};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//! let config = WebhookConfig {
//!     url: "https://your-webhook-url.com/endpoint".to_string(),
//!     timeout: Duration::from_secs(60),
//!     retry_attempts: 5,
//!     retry_delay: Duration::from_secs(2),
//!     headers: None,
//! };
//!
//! let poster = WebhookPoster::new(config)?;
//! // Use the poster...
//! # Ok(())
//! # }
//! ```

use log::{error, info, warn};
use reqwest::Client;
use serde_json;
use std::{sync::Arc, time::Duration};
use tokio::{sync::RwLock, time::sleep};

use crate::webhook::events::WebhookEvent;

/// Configuration for webhook posting
#[derive(Debug, Clone)]
pub struct WebhookConfig {
    pub url: String,
    pub timeout: Duration,
    pub retry_attempts: u32,
    pub retry_delay: Duration,
    pub headers: Option<std::collections::HashMap<String, String>>,
}

impl Default for WebhookConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            timeout: Duration::from_secs(30),
            retry_attempts: 3,
            retry_delay: Duration::from_secs(1),
            headers: None,
        }
    }
}

/// Webhook event poster for sending events to specified URLs
/// All methods are thread-safe
#[derive(Clone)]
pub struct WebhookPoster {
    client: Arc<RwLock<Client>>,
    config: Arc<RwLock<WebhookConfig>>,
}

impl WebhookPoster {
    /// Create a new webhook poster with the given configuration
    pub fn new(config: WebhookConfig) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let client = Client::builder().timeout(config.timeout).build()?;

        Ok(Self {
            client: Arc::new(RwLock::new(client)),
            config: Arc::new(RwLock::new(config)),
        })
    }

    /// Post a webhook event to the configured URL
    pub async fn post_event(&self, event: &WebhookEvent) -> Result<(), WebhookPostError> {
        if self.config.read().await.url.is_empty() {
            log::debug!("Webhook URL is empty, skipping");
            return Ok(());
        }

        let serialized_event = serde_json::to_string(event)
            .map_err(|e| WebhookPostError::Serialization(e.to_string()))?;

        let self_clone = self.clone();
        tokio::task::spawn(async move {
            let result = self_clone.post_with_retry(&serialized_event).await;
            if let Err(e) = result {
                log::error!("Post webhook event error: {}", e);
            }
        });

        Ok(())
    }

    /// Post raw JSON data to the configured URL
    #[allow(dead_code)]
    pub async fn post_json(&self, json_data: &str) -> Result<(), WebhookPostError> {
        if self.config.read().await.url.is_empty() {
            log::debug!("Webhook URL is empty, skipping");
            return Ok(());
        }

        self.post_with_retry(json_data).await
    }

    /// Post data with retry logic
    async fn post_with_retry(&self, data: &str) -> Result<(), WebhookPostError> {
        if self.config.read().await.url.is_empty() {
            log::debug!("Webhook URL is empty, skipping");
            return Ok(());
        }

        let mut last_error = None;

        for attempt in 1..=self.config.read().await.retry_attempts {
            match self.send_request(data).await {
                Ok(_) => {
                    if attempt > 1 {
                        info!("Webhook posted successfully on attempt {}", attempt);
                    }
                    return Ok(());
                }
                Err(e) => {
                    last_error = Some(e);
                    if attempt < self.config.read().await.retry_attempts {
                        warn!(
                            "Webhook post attempt {} failed, retrying in {:?}",
                            attempt,
                            self.config.read().await.retry_delay
                        );
                        sleep(self.config.read().await.retry_delay).await;
                    }
                }
            }
        }

        error!("All webhook post attempts failed");
        Err(last_error.unwrap())
    }

    /// Send the actual HTTP request
    async fn send_request(&self, data: &str) -> Result<(), WebhookPostError> {
        let webhook_url = self.config.read().await.url.clone();
        let mut request = self.client.read().await.post(&webhook_url);

        // Add custom headers if configured
        if let Some(headers) = &self.config.read().await.headers {
            for (key, value) in headers {
                request = request.header(key, value);
            }
        }

        log::debug!("Sending webhook request to: {}", webhook_url);

        // Set content type to JSON
        request = request.header("Content-Type", "application/json");

        let response = request
            .body(data.to_string())
            .send()
            .await
            .map_err(|e| WebhookPostError::Network(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(WebhookPostError::Http {
                status: status.as_u16(),
                body,
            });
        }

        Ok(())
    }

    /// Update the webhook configuration
    pub async fn update_config(
        &self,
        config: WebhookConfig,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        *self.client.write().await = Client::builder().timeout(config.timeout).build()?;
        *self.config.write().await = config;
        Ok(())
    }
}

/// Errors that can occur during webhook posting
#[derive(Debug, thiserror::Error)]
pub enum WebhookPostError {
    #[error("Network error: {0}")]
    Network(String),

    #[error("HTTP error: status {status}, body: {body}")]
    Http { status: u16, body: String },

    #[error("Serialization error: {0}")]
    Serialization(String),
}

/// Convenience function to create a webhook poster with custom headers
pub fn create_webhook_poster(
    url: &str,
    headers: Option<std::collections::HashMap<String, String>>,
) -> Result<WebhookPoster, Box<dyn std::error::Error + Send + Sync>> {
    let config = WebhookConfig {
        url: url.to_string(),
        headers,
        ..Default::default()
    };
    log::info!("Creating webhook poster with URL: {}", url);
    WebhookPoster::new(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[tokio::test]
    async fn test_webhook_poster_creation() {
        let config = WebhookConfig {
            url: "https://httpbin.org/post".to_string(),
            ..Default::default()
        };

        let poster = WebhookPoster::new(config);
        assert!(poster.is_ok());
    }

    #[tokio::test]
    async fn test_webhook_poster_with_headers() {
        let mut headers = HashMap::new();
        headers.insert("Authorization".to_string(), "Bearer token".to_string());

        let poster = create_webhook_poster("https://httpbin.org/post", Some(headers));
        assert!(poster.is_ok());
    }
}
