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
        // Use the IM spotlight relation API to get user info
        let url = "https://www.douyin.com/aweme/v1/web/im/spotlight/relation/";
        let resp = self
            .client
            .get(url)
            .header("Referer", "https://www.douyin.com/")
            .header("User-Agent", USER_AGENT)
            .header("Cookie", self.cookies.clone())
            .send()
            .await?;

        if resp.status().is_success() {
            if let Ok(data) = resp.json::<super::response::DouyinRelationResponse>().await {
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
                        avatar_thumb: super::response::AvatarThumb {
                            url_list: vec![],
                        },
                        follow_info: super::response::FollowInfo::default(),
                        foreign_user: 0,
                        open_id_str: "".to_string(),
                    };
                    return Ok(user);
                }
            }
        }
        
        Err(DouyinClientError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Failed to get user info from Douyin relation API"
        )))
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
    async fn test_douyin_relation_response_parsing() {
        // This test verifies that our JSON parsing logic works correctly
        // with the new relation API response structure
        let sample_response_json = r#"{
            "extra": {
                "fatal_item_ids": [],
                "logid": "20250722225937443BF707B84858430847",
                "now": 1753196377000
            },
            "followings": [
                {
                    "account_cert_info": "{}",
                    "avatar_signature": "",
                    "avatar_small": {
                        "uri": "168x168/test",
                        "url_list": ["https://example.com/small_avatar.jpg"]
                    },
                    "avatar_thumb": {
                        "url_list": ["https://example.com/avatar.jpg"]
                    },
                    "birthday_hide_level": 0,
                    "commerce_user_level": 0,
                    "custom_verify": "",
                    "enterprise_verify_reason": "",
                    "follow_status": 0,
                    "follower_status": 0,
                    "has_e_account_role": false,
                    "im_activeness": 3,
                    "im_role_ids": [],
                    "is_im_oversea_user": 0,
                    "nickname": "测试用户",
                    "sec_uid": "MS4wLjABAAAACYbubi2lhyaRNn7xCsb0xG9AeBaM4g2yo_7JeoUoL3c",
                    "short_id": "3941301946",
                    "signature": "",
                    "social_relation_sub_type": 0,
                    "social_relation_type": 0,
                    "uid": "369055625381688",
                    "unique_id": "testuser",
                    "verification_type": 0,
                    "webcast_sp_info": {}
                }
            ],
            "owner_sec_uid": "MS4wLjABAAAACYbubi2lhyaRNn7xCsb0xG9AeBaM4g2yo_7JeoUoL3c",
            "status_code": 0,
            "log_pb": {
                "impr_id": "20250722225937443BF707B84858430847"
            }
        }"#;

        let response = serde_json::from_str::<super::response::DouyinRelationResponse>(sample_response_json).unwrap();
        assert_eq!(response.status_code, 0);
        assert_eq!(response.owner_sec_uid, "MS4wLjABAAAACYbubi2lhyaRNn7xCsb0xG9AeBaM4g2yo_7JeoUoL3c");
        assert!(response.followings.is_some());
        
        let followings = response.followings.unwrap();
        assert_eq!(followings.len(), 1);
        assert_eq!(followings[0].nickname, "测试用户");
        assert_eq!(followings[0].uid, "369055625381688");
        assert_eq!(followings[0].sec_uid, "MS4wLjABAAAACYbubi2lhyaRNn7xCsb0xG9AeBaM4g2yo_7JeoUoL3c");
    }

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
