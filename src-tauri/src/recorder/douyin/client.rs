use crate::{database::account::AccountRow, recorder::user_agent_generator};
use m3u8_rs::{MediaPlaylist, Playlist};
use reqwest::{Client, Error as ReqwestError};

use super::response::DouyinRoomInfoResponse;
use std::{fmt, path::Path};

#[derive(Debug)]
pub enum DouyinClientError {
    Network(String),
    Io(std::io::Error),
    Playlist(String),
    H5NotLive(String),
}

impl fmt::Display for DouyinClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Network(e) => write!(f, "Network error: {}", e),
            Self::Io(e) => write!(f, "IO error: {}", e),
            Self::Playlist(e) => write!(f, "Playlist error: {}", e),
            Self::H5NotLive(e) => write!(f, "H5 live not started: {}", e),
        }
    }
}

impl From<ReqwestError> for DouyinClientError {
    fn from(err: ReqwestError) -> Self {
        DouyinClientError::Network(err.to_string())
    }
}

impl From<std::io::Error> for DouyinClientError {
    fn from(err: std::io::Error) -> Self {
        DouyinClientError::Io(err)
    }
}

#[derive(Debug, Clone)]
pub struct DouyinBasicRoomInfo {
    pub room_id_str: String,
    pub room_title: String,
    pub cover: Option<String>,
    pub status: i64,
    pub hls_url: String,
    pub stream_data: String,
    // user related
    pub user_name: String,
    pub user_avatar: String,
    pub sec_user_id: String,
}

#[derive(Clone)]
pub struct DouyinClient {
    client: Client,
    account: AccountRow,
}

impl DouyinClient {
    pub fn new(account: &AccountRow) -> Self {
        let client = Client::builder().build().unwrap();
        Self {
            client,
            account: account.clone(),
        }
    }

    pub fn generate_user_agent_header(&self) -> reqwest::header::HeaderMap {
        let user_agent = user_agent_generator::UserAgentGenerator::new().generate();
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("user-agent", user_agent.parse().unwrap());
        headers
    }

    pub async fn get_room_info(
        &self,
        room_id: u64,
        sec_user_id: &str,
    ) -> Result<DouyinBasicRoomInfo, DouyinClientError> {
        let url = format!(
            "https://live.douyin.com/webcast/room/web/enter/?aid=6383&app_name=douyin_web&live_id=1&device_platform=web&language=zh-CN&enter_from=web_live&a_bogus=0&cookie_enabled=true&screen_width=1920&screen_height=1080&browser_language=zh-CN&browser_platform=MacIntel&browser_name=Chrome&browser_version=122.0.0.0&web_rid={}",
            room_id
        );

        let mut headers = self.generate_user_agent_header();
        headers.insert("Referer", "https://live.douyin.com/".parse().unwrap());
        headers.insert("Cookie", self.account.cookies.clone().parse().unwrap());

        let resp = self.client.get(&url).headers(headers).send().await?;

        let status = resp.status();
        let text = resp.text().await?;

        if text.is_empty() {
            log::warn!("Empty room info response, trying H5 API");
            return self.get_room_info_h5(room_id, sec_user_id).await;
        }

        if status.is_success() {
            if let Ok(data) = serde_json::from_str::<DouyinRoomInfoResponse>(&text) {
                let cover = data
                    .data
                    .data
                    .first()
                    .and_then(|data| data.cover.as_ref())
                    .map(|cover| cover.url_list[0].clone());
                return Ok(DouyinBasicRoomInfo {
                    room_id_str: data.data.data[0].id_str.clone(),
                    sec_user_id: sec_user_id.to_string(),
                    cover,
                    room_title: data.data.data[0].title.clone(),
                    user_name: data.data.user.nickname.clone(),
                    user_avatar: data.data.user.avatar_thumb.url_list[0].clone(),
                    status: data.data.room_status,
                    hls_url: data.data.data[0]
                        .stream_url
                        .as_ref()
                        .map(|stream_url| stream_url.hls_pull_url.clone())
                        .unwrap_or_default(),
                    stream_data: data.data.data[0]
                        .stream_url
                        .as_ref()
                        .map(|s| s.live_core_sdk_data.pull_data.stream_data.clone())
                        .unwrap_or_default(),
                });
            } else {
                log::error!("Failed to parse room info response: {}", text);
                return self.get_room_info_h5(room_id, sec_user_id).await;
            }
        }

        log::error!("Failed to get room info: {}", status);
        return self.get_room_info_h5(room_id, sec_user_id).await;
    }

    pub async fn get_room_info_h5(
        &self,
        room_id: u64,
        sec_user_id: &str,
    ) -> Result<DouyinBasicRoomInfo, DouyinClientError> {
        // 参考biliup实现，构建完整的URL参数
        let room_id_str = room_id.to_string();
        // https://webcast.amemv.com/webcast/room/reflow/info/?type_id=0&live_id=1&version_code=99.99.99&app_id=1128&room_id=10000&sec_user_id=MS4wLjAB&aid=6383&device_platform=web&browser_language=zh-CN&browser_platform=Win32&browser_name=Mozilla&browser_version=5.0
        let url_params = [
            ("type_id", "0"),
            ("live_id", "1"),
            ("version_code", "99.99.99"),
            ("app_id", "1128"),
            ("room_id", &room_id_str),
            ("sec_user_id", sec_user_id),
            ("aid", "6383"),
            ("device_platform", "web"),
        ];

        // 构建URL
        let query_string = url_params
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join("&");
        let url = format!(
            "https://webcast.amemv.com/webcast/room/reflow/info/?{}",
            query_string
        );

        log::info!("get_room_info_h5: {}", url);

        let mut headers = self.generate_user_agent_header();
        headers.insert("Referer", "https://live.douyin.com/".parse().unwrap());
        headers.insert("Cookie", self.account.cookies.clone().parse().unwrap());

        let resp = self.client.get(&url).headers(headers).send().await?;

        let status = resp.status();
        let text = resp.text().await?;

        if status.is_success() {
            // Try to parse as H5 response format
            if let Ok(h5_data) =
                serde_json::from_str::<super::response::DouyinH5RoomInfoResponse>(&text)
            {
                // Extract RoomBasicInfo from H5 response
                let room = &h5_data.data.room;
                let owner = &room.owner;

                let cover = room
                    .cover
                    .as_ref()
                    .and_then(|c| c.url_list.first().cloned());
                let hls_url = room
                    .stream_url
                    .as_ref()
                    .map(|s| s.hls_pull_url.clone())
                    .unwrap_or_default();

                return Ok(DouyinBasicRoomInfo {
                    room_id_str: room.id_str.clone(),
                    room_title: room.title.clone(),
                    cover,
                    status: if room.status == 2 { 0 } else { 1 },
                    hls_url,
                    user_name: owner.nickname.clone(),
                    user_avatar: owner
                        .avatar_thumb
                        .url_list
                        .first()
                        .unwrap_or(&String::new())
                        .clone(),
                    sec_user_id: owner.sec_uid.clone(),
                    stream_data: room
                        .stream_url
                        .as_ref()
                        .map(|s| s.live_core_sdk_data.pull_data.stream_data.clone())
                        .unwrap_or_default(),
                });
            }

            // If that fails, try to parse as a generic JSON to see what we got
            if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(&text) {
                log::debug!(
                    "Unexpected response structure: {}",
                    serde_json::to_string_pretty(&json_value).unwrap_or_default()
                );

                // Check if it's an error response
                if let Some(status_code) = json_value.get("status_code").and_then(|v| v.as_i64()) {
                    if status_code != 0 {
                        let error_msg = json_value
                            .get("data")
                            .and_then(|v| v.get("message").and_then(|v| v.as_str()))
                            .unwrap_or("Unknown error");

                        if status_code == 10011 {
                            return Err(DouyinClientError::H5NotLive(error_msg.to_string()));
                        }

                        return Err(DouyinClientError::Network(format!(
                            "API returned error status_code: {} - {}",
                            status_code, error_msg
                        )));
                    }
                }

                // 检查是否是"invalid session"错误
                if let Some(status_message) =
                    json_value.get("status_message").and_then(|v| v.as_str())
                {
                    if status_message.contains("invalid session") {
                        return Err(DouyinClientError::Network(
                            "Invalid session - please check your cookies. Make sure you have valid sessionid, passport_csrf_token, and other authentication cookies from douyin.com".to_string(),
                        ));
                    }
                }

                return Err(DouyinClientError::Network(format!(
                    "Failed to parse h5 room info response: {}",
                    text
                )));
            } else {
                log::error!("Failed to parse h5 room info response: {}", text);
                return Err(DouyinClientError::Network(format!(
                    "Failed to parse h5 room info response: {}",
                    text
                )));
            }
        }

        log::error!("Failed to get h5 room info: {}", status);
        Err(DouyinClientError::Network(format!(
            "Failed to get h5 room info: {} {}",
            status, text
        )))
    }

    pub async fn get_user_info(&self) -> Result<super::response::User, DouyinClientError> {
        // Use the IM spotlight relation API to get user info
        let url = "https://www.douyin.com/aweme/v1/web/im/spotlight/relation/";
        let mut headers = self.generate_user_agent_header();
        headers.insert("Referer", "https://www.douyin.com/".parse().unwrap());
        headers.insert("Cookie", self.account.cookies.clone().parse().unwrap());

        let resp = self.client.get(url).headers(headers).send().await?;

        let status = resp.status();
        let text = resp.text().await?;

        if status.is_success() {
            if let Ok(data) = serde_json::from_str::<super::response::DouyinRelationResponse>(&text)
            {
                if data.status_code == 0 {
                    let owner_sec_uid = &data.owner_sec_uid;

                    // Find the user's own info in the followings list by matching sec_uid
                    if let Some(followings) = &data.followings {
                        for following in followings {
                            if following.sec_uid == *owner_sec_uid {
                                let user = super::response::User {
                                    id_str: following.uid.clone(),
                                    sec_uid: following.sec_uid.clone(),
                                    nickname: following.nickname.clone(),
                                    avatar_thumb: following.avatar_thumb.clone(),
                                    follow_info: super::response::FollowInfo::default(),
                                    foreign_user: 0,
                                    open_id_str: "".to_string(),
                                };
                                return Ok(user);
                            }
                        }
                    }

                    // If not found in followings, create a minimal user info from owner_sec_uid
                    let user = super::response::User {
                        id_str: "".to_string(), // We don't have the numeric UID
                        sec_uid: owner_sec_uid.clone(),
                        nickname: "抖音用户".to_string(), // Default nickname
                        avatar_thumb: super::response::AvatarThumb { url_list: vec![] },
                        follow_info: super::response::FollowInfo::default(),
                        foreign_user: 0,
                        open_id_str: "".to_string(),
                    };
                    return Ok(user);
                }
            } else {
                log::error!("Failed to parse user info response: {}", text);
                return Err(DouyinClientError::Network(format!(
                    "Failed to parse user info response: {}",
                    text
                )));
            }
        }

        log::error!("Failed to get user info: {}", status);

        Err(DouyinClientError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Failed to get user info from Douyin relation API",
        )))
    }

    /// Download file from url to path
    pub async fn download_file(&self, url: &str, path: &Path) -> Result<(), DouyinClientError> {
        if !path.parent().unwrap().exists() {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        }
        let response = self.client.get(url).send().await?;
        let bytes = response.bytes().await?;
        let mut file = tokio::fs::File::create(&path).await?;
        let mut content = std::io::Cursor::new(bytes);
        tokio::io::copy(&mut content, &mut file).await?;
        Ok(())
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
            log::info!("Master manifest with playlist URL: {}", url);
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
            let error = response.error_for_status().unwrap_err();
            log::error!("HTTP error: {} for URL: {}", error, url);
            return Err(DouyinClientError::Network(error.to_string()));
        }

        let mut file = tokio::fs::File::create(path).await?;
        let bytes = response.bytes().await?;
        let size = bytes.len() as u64;
        let mut content = std::io::Cursor::new(bytes);
        tokio::io::copy(&mut content, &mut file).await?;
        Ok(size)
    }
}
