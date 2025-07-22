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

    pub async fn get_user_info(&self) -> Result<super::response::User, DouyinClientError> {
        // Try the API endpoint used for getting user info
        let url = "https://www.douyin.com/aweme/v1/web/aweme/personal/";
        let resp = self
            .client
            .get(url)
            .header("Referer", "https://www.douyin.com/")
            .header("User-Agent", USER_AGENT)
            .header("Cookie", self.cookies.clone())
            .send()
            .await?;

        if resp.status().is_success() {
            if let Ok(data) = resp.json::<serde_json::Value>().await {
                if let Some(user_info) = data["aweme_list"].as_array().and_then(|arr| arr.first()) {
                    if let Some(author) = user_info["author"].as_object() {
                        // Map the author info to our User struct
                        let user = super::response::User {
                            id_str: author["uid"].as_str().unwrap_or("").to_string(),
                            sec_uid: author["sec_uid"].as_str().unwrap_or("").to_string(),
                            nickname: author["nickname"].as_str().unwrap_or("").to_string(),
                            avatar_thumb: super::response::AvatarThumb {
                                url_list: author["avatar_thumb"]["url_list"]
                                    .as_array()
                                    .map(|arr| arr.iter().filter_map(|v| v.as_str()).map(|s| s.to_string()).collect())
                                    .unwrap_or_default(),
                            },
                            follow_info: super::response::FollowInfo::default(),
                            foreign_user: 0,
                            open_id_str: "".to_string(),
                        };
                        return Ok(user);
                    }
                }
            }
        }

        // Fallback: try to get user info from personal page
        let url = "https://www.douyin.com/user/self";
        let resp = self
            .client
            .get(url)
            .header("Referer", "https://www.douyin.com/")
            .header("User-Agent", USER_AGENT)
            .header("Cookie", self.cookies.clone())
            .send()
            .await?;

        let html = resp.text().await?;
        
        // Parse HTML to extract user info from SSR data
        if let Some(start) = html.find("window.__INITIAL_STATE__") {
            if let Some(eq_pos) = html[start..].find("=") {
                if let Some(script_end) = html[start + eq_pos + 1..].find("</script>") {
                    let json_str = html[start + eq_pos + 1..start + eq_pos + 1 + script_end].trim();
                    
                    if let Ok(data) = serde_json::from_str::<serde_json::Value>(json_str) {
                        if let Some(user_detail) = data["user"]["user"]["userDetail"].as_object() {
                            let user = super::response::User {
                                id_str: user_detail["user"]["uid"].as_str().unwrap_or("").to_string(),
                                sec_uid: user_detail["user"]["secUid"].as_str().unwrap_or("").to_string(),
                                nickname: user_detail["user"]["nickname"].as_str().unwrap_or("").to_string(),
                                avatar_thumb: super::response::AvatarThumb {
                                    url_list: user_detail["user"]["avatarThumb"]["urlList"]
                                        .as_array()
                                        .map(|arr| arr.iter().filter_map(|v| v.as_str()).map(|s| s.to_string()).collect())
                                        .unwrap_or_default(),
                                },
                                follow_info: super::response::FollowInfo::default(),
                                foreign_user: 0,
                                open_id_str: "".to_string(),
                            };
                            return Ok(user);
                        }
                    }
                }
            }
        }
        
        Err(DouyinClientError::Network(reqwest::Error::from(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "User info not found in response"
        ))))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::account::AccountRow;

    #[tokio::test]
    async fn test_douyin_user_info_parsing() {
        // This test verifies that our JSON parsing logic works correctly
        // with a sample user info structure
        let sample_user_json = r#"{
            "id_str": "12345678901234567",
            "sec_uid": "MS4wLjABAAAA...",
            "nickname": "测试用户",
            "avatar_thumb": {
                "url_list": ["https://example.com/avatar.jpg"]
            },
            "follow_info": {
                "follow_status": 0,
                "follow_status_str": "未关注"
            },
            "foreign_user": 0,
            "open_id_str": ""
        }"#;

        let user = serde_json::from_str::<super::response::User>(sample_user_json).unwrap();
        assert_eq!(user.id_str, "12345678901234567");
        assert_eq!(user.nickname, "测试用户");
        assert_eq!(user.avatar_thumb.url_list.len(), 1);
    }
}
