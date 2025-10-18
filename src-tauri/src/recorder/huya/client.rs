use super::errors::HuyaClientError;

use crate::database::account::AccountRow;

use crate::recorder::user_agent_generator;
use crate::recorder::RoomInfo;
use crate::recorder::UserInfo;

use reqwest::Client;
use scraper::Html;
use scraper::Selector;
use serde::Deserialize;
use serde::Serialize;
use std::path::Path;
use std::time::Duration;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct HuyaUserInfo {
    pub user_id: i64,
    pub user_name: String,
    pub user_sign: String,
    pub user_avatar_url: String,
}

pub struct HuyaClient {
    client: Client,
}

#[derive(Clone, Debug)]
pub struct StreamInfo {
    pub stream_url: String,
    pub stream_type: String,
    pub stream_quality: String,
    pub stream_codec: String,
}

impl HuyaClient {
    pub fn new() -> HuyaClient {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap();

        HuyaClient { client }
    }

    fn generate_user_agent_header(&self) -> reqwest::header::HeaderMap {
        let user_agent = user_agent_generator::UserAgentGenerator::new().generate();
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("user-agent", user_agent.parse().unwrap());
        headers
    }

    pub async fn get_user_info(
        &self,
        account: &AccountRow,
    ) -> Result<HuyaUserInfo, HuyaClientError> {
        // https://m.huya.com/video/u/2246697169
        let mut headers = self.generate_user_agent_header();
        if let Ok(cookies) = account.cookies.parse() {
            headers.insert("cookie", cookies);
        } else {
            return Err(HuyaClientError::InvalidCookie);
        }
        let url = format!("https://m.huya.com/video/u/{}", account.uid);
        let response = self.client.get(url).headers(headers).send().await?;
        let raw_content = response.text().await?;
        // <div class="video-list-info">
        //     <div class="podcast-box clearfix">
        //         <img src="http://huyaimg.msstatic.com/avatar/1060/3f/0e6c0694867ef98e9f869589608ce3_180_135.jpg" alt="">
        //         <div class="podcast-info-intro">
        //             <h2>X inrea  丶</h2>
        //             <p></p>
        //         </div>
        //     </div>
        // </div>
        let document = Html::parse_document(&raw_content);

        let avatar_selector = Selector::parse(".video-list-info .podcast-box img").unwrap();
        let name_selector = Selector::parse(".video-list-info .podcast-info-intro h2").unwrap();
        let sign_selector = Selector::parse(".video-list-info .podcast-info-intro p").unwrap();

        // 提取 avatar (img src)
        let avatar = document
            .select(&avatar_selector)
            .next()
            .and_then(|img| img.value().attr("src"))
            .map(|src| src.to_string());

        // 提取 name (h2 text)
        let name = document
            .select(&name_selector)
            .next()
            .map(|h2| h2.text().collect::<String>().trim().to_string())
            .filter(|s| !s.is_empty());

        // 提取 sign (p text)
        let sign = document
            .select(&sign_selector)
            .next()
            .map(|p| p.text().collect::<String>().trim().to_string())
            .filter(|s| !s.is_empty());

        Ok(HuyaUserInfo {
            user_id: account.uid,
            user_name: name.unwrap_or_default(),
            user_sign: sign.unwrap_or_default(),
            user_avatar_url: avatar.unwrap_or_default(),
        })
    }

    pub async fn get_room_info(
        &self,
        account: &AccountRow,
        room_id: i64,
    ) -> Result<(UserInfo, RoomInfo, StreamInfo), HuyaClientError> {
        let mut headers = self.generate_user_agent_header();
        if let Ok(cookies) = account.cookies.parse() {
            headers.insert("cookie", cookies);
        } else {
            return Err(HuyaClientError::InvalidCookie);
        }
        headers.insert("Referer", "https://huya.com/".parse().unwrap());
        let url = format!("https://huya.com/{}", room_id);
        let response = self.client.get(url).headers(headers).send().await?;
        let raw_content = response.text().await?;
        let (tt_room_data, tt_profile_info, hy_player_config) =
            super::extractor::LiveStreamExtractor::extract_variables(&raw_content)?;
        Ok((
            UserInfo {
                user_id: tt_profile_info
                    .get("uid")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0)
                    .to_string(),
                user_name: tt_profile_info
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                user_avatar: tt_profile_info
                    .get("avatar")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            },
            RoomInfo {
                room_id: tt_room_data
                    .get("room_id")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0),
                room_title: tt_room_data
                    .get("room_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                room_cover: tt_room_data
                    .get("room_cover")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            },
            StreamInfo {
                stream_url: hy_player_config
                    .get("stream_url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                stream_type: hy_player_config
                    .get("stream_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                stream_quality: hy_player_config
                    .get("stream_quality")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                stream_codec: hy_player_config
                    .get("stream_codec")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            },
        ))
    }

    /// Download file from url to path
    pub async fn download_file(&self, url: &str, path: &Path) -> Result<(), HuyaClientError> {
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

    pub async fn get_index_content(
        &self,
        account: &AccountRow,
        url: &String,
    ) -> Result<String, HuyaClientError> {
        let mut headers = self.generate_user_agent_header();
        if let Ok(cookies) = account.cookies.parse() {
            headers.insert("cookie", cookies);
        } else {
            return Err(HuyaClientError::InvalidCookie);
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
            Err(HuyaClientError::InvalidStream)
        }
    }

    pub async fn download_ts(&self, url: &str, file_path: &str) -> Result<u64, HuyaClientError> {
        let res = self
            .client
            .get(url)
            .headers(self.generate_user_agent_header())
            .send()
            .await?;
        let mut file = tokio::fs::File::create(file_path).await?;
        let bytes = res.bytes().await?;
        let size = bytes.len() as u64;
        let mut content = std::io::Cursor::new(bytes);
        tokio::io::copy(&mut content, &mut file).await?;
        Ok(size)
    }
}
