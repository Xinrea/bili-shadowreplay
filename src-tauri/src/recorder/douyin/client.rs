use crate::database::account::AccountRow;
use base64::Engine;
use m3u8_rs::{MediaPlaylist, Playlist};
use reqwest::{Client, Error as ReqwestError};

use super::response::DouyinRoomInfoResponse;
use std::fmt;

const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36";

#[derive(Debug)]
pub enum DouyinClientError {
    Network(ReqwestError),
    Io(std::io::Error),
    Playlist(String),
}

impl fmt::Display for DouyinClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Network(e) => write!(f, "Network error: {}", e),
            Self::Io(e) => write!(f, "IO error: {}", e),
            Self::Playlist(e) => write!(f, "Playlist error: {}", e),
        }
    }
}

impl From<ReqwestError> for DouyinClientError {
    fn from(err: ReqwestError) -> Self {
        DouyinClientError::Network(err)
    }
}

impl From<std::io::Error> for DouyinClientError {
    fn from(err: std::io::Error) -> Self {
        DouyinClientError::Io(err)
    }
}

#[derive(Clone)]
pub struct DouyinClient {
    client: Client,
    cookies: String,
}

impl DouyinClient {
    pub fn new(account: &AccountRow) -> Self {
        let client = Client::builder().user_agent(USER_AGENT).build().unwrap();
        Self {
            client,
            cookies: account.cookies.clone(),
        }
    }

    pub async fn get_room_info(
        &self,
        room_id: u64,
    ) -> Result<DouyinRoomInfoResponse, DouyinClientError> {
        let url = format!(
            "https://live.douyin.com/webcast/room/web/enter/?aid=6383&app_name=douyin_web&live_id=1&device_platform=web&language=zh-CN&enter_from=web_live&cookie_enabled=true&screen_width=1920&screen_height=1080&browser_language=zh-CN&browser_platform=MacIntel&browser_name=Chrome&browser_version=122.0.0.0&web_rid={}",
            room_id
        );

        let resp = self
            .client
            .get(&url)
            .header("Referer", "https://live.douyin.com/")
            .header("User-Agent", USER_AGENT)
            .header("Cookie", self.cookies.clone())
            .send()
            .await?
            .json::<DouyinRoomInfoResponse>()
            .await?;

        Ok(resp)
    }

    pub async fn get_cover_base64(&self, url: &str) -> Result<String, DouyinClientError> {
        log::info!("get_cover_base64: {}", url);
        let response = self.client.get(url).send().await?;
        let bytes = response.bytes().await?;
        let base64 = base64::engine::general_purpose::STANDARD.encode(bytes);
        let mime_type = mime_guess::from_path(url)
            .first_or_octet_stream()
            .to_string();
        Ok(format!("data:{};base64,{}", mime_type, base64))
    }

    pub async fn get_m3u8_content(
        &self,
        url: &str,
    ) -> Result<(MediaPlaylist, String), DouyinClientError> {
        let content = self.client.get(url).send().await?.text().await?;
        // m3u8 content: #EXTM3U
        // #EXT-X-VERSION:3
        // #EXT-X-STREAM-INF:PROGRAM-ID=1,BANDWIDTH=2560000
        // http://7167739a741646b4651b6949b2f3eb8e.livehwc3.cn/pull-hls-l26.douyincdn.com/third/stream-693342996808860134_or4.m3u8?sub_m3u8=true&user_session_id=16090eb45ab8a2f042f7c46563936187&major_anchor_level=common&edge_slice=true&expire=67d944ec&sign=47b95cc6e8de20d82f3d404412fa8406
        if content.contains("BANDWIDTH") {
            let new_url = content.lines().last().unwrap();
            return Box::pin(self.get_m3u8_content(new_url)).await;
        }

        match m3u8_rs::parse_playlist_res(content.as_bytes()) {
            Ok(Playlist::MasterPlaylist(_)) => Err(DouyinClientError::Playlist(
                "Unexpected master playlist".to_string(),
            )),
            Ok(Playlist::MediaPlaylist(pl)) => Ok((pl, url.to_string())),
            Err(e) => Err(DouyinClientError::Playlist(e.to_string())),
        }
    }

    pub async fn download_ts(&self, url: &str, path: &str) -> Result<u64, DouyinClientError> {
        let response = self.client.get(url).send().await?;

        if response.status() != reqwest::StatusCode::OK {
            return Err(DouyinClientError::Network(
                response.error_for_status().unwrap_err(),
            ));
        }

        let mut file = tokio::fs::File::create(path).await?;
        let bytes = response.bytes().await?;
        let size = bytes.len() as u64;
        let mut content = std::io::Cursor::new(bytes);
        tokio::io::copy(&mut content, &mut file).await?;
        Ok(size)
    }
}
