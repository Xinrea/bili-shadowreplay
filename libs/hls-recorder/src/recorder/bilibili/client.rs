use super::errors::BiliClientError;
use super::profile;
use super::profile::Profile;
use super::response;
use super::response::GeneralResponse;
use super::response::PostVideoMetaResponse;
use super::response::PreuploadResponse;
use super::response::VideoSubmitData;
use crate::database::account::AccountRow;
use crate::progress_reporter::ProgressReporter;
use crate::progress_reporter::ProgressReporterTrait;
use base64::Engine;
use pct_str::PctString;
use pct_str::URIReserved;
use regex::Regex;
use reqwest::Client;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;
use std::fmt;
use std::path::Path;
use std::time::Duration;
use std::time::SystemTime;
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use tokio::time::Instant;

/// BiliClient is thread safe
pub struct BiliClient {
    client: Client,
    headers: reqwest::header::HeaderMap,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StreamType {
    TS,
    FMP4,
}

#[derive(Clone, Debug)]
pub struct BiliStream {
    pub format: StreamType,
    pub host: String,
    pub path: String,
    pub extra: String,
    pub expire: i64,
}

impl fmt::Display for BiliStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "type: {:?}, host: {}, path: {}, extra: {}, expire: {}",
            self.format, self.host, self.path, self.extra, self.expire
        )
    }
}

impl BiliStream {
    pub fn new(format: StreamType, base_url: &str, host: &str, extra: &str) -> BiliStream {
        BiliStream {
            format,
            host: host.into(),
            path: BiliStream::get_path(base_url),
            extra: extra.into(),
            expire: BiliStream::get_expire(extra).unwrap_or(600000),
        }
    }

    pub fn index(&self) -> String {
        format!(
            "https://{}/{}/{}?{}",
            self.host, self.path, "index.m3u8", self.extra
        )
    }

    pub fn ts_url(&self, seg_name: &str) -> String {
        format!(
            "https://{}/{}/{}?{}",
            self.host, self.path, seg_name, self.extra
        )
    }

    pub fn get_path(base_url: &str) -> String {
        match base_url.rfind('/') {
            Some(pos) => base_url[..pos + 1].to_string(),
            None => base_url.to_string(),
        }
    }

    pub fn get_expire(extra: &str) -> Option<i64> {
        extra.split('&').find_map(|param| {
            if param.starts_with("expires=") {
                param.split('=').nth(1)?.parse().ok()
            } else {
                None
            }
        })
    }
}

impl BiliClient {
    pub async fn get_index_content(
        &self,
        account: &AccountRow,
        url: &String,
    ) -> Result<String, BiliClientError> {
        let mut headers = self.headers.clone();
        if let Ok(cookies) = account.cookies.parse() {
            headers.insert("cookie", cookies);
        } else {
            return Err(BiliClientError::InvalidCookie);
        }
        let response = self
            .client
            .get(url.to_owned())
            .headers(headers)
            .send()
            .await?;

        if response.status().is_success() {
            Ok(response.text().await?)
        } else {
            log::error!("get_index_content failed: {}", response.status());
            Err(BiliClientError::InvalidStream)
        }
    }
}
