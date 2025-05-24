use reqwest::cookie::{self, CookieStore};
use serde::Deserialize;

use crate::http_client::{ApiClient, ApiError};

use super::DanmuProviderError;

pub struct BiliDanmu {
    client: ApiClient,
    room_id: u64,
    user_id: u64,
}

const BILI_ENDPOINT: &str = "https://api.live.bilibili.com";

impl BiliDanmu {
    async fn new(cookie: &str, room_id: u64) -> Result<Self, DanmuProviderError> {
        // find DedeUserID=<user_id> in cookie str
        let user_id = BiliDanmu::parse_user_id(cookie)?;
        let client = ApiClient::new(BILI_ENDPOINT, cookie);

        Ok(Self {
            client,
            user_id,
            room_id,
        })
    }
    async fn get_danmu_info(&self) -> Result<DanmuInfo, ApiError> {
        let resp = self
            .client
            .get(
                &format!(
                    "xlive/web-room/v1/index/getDanmuInfo?id={}&type=0",
                    self.room_id
                ),
                None,
            )
            .await?
            .json::<DanmuInfo>()
            .await?;

        Ok(resp)
    }

    async fn get_real_room(&self, room_id: u64) -> Result<u64, ApiError> {
        let resp = self
            .client
            .get(
                &format!("room/v1/Room/room_init?id={}?&from=room", room_id),
                None,
            )
            .await?
            .json::<RoomInit>()
            .await?
            .data
            .room_id;

        Ok(resp)
    }

    fn parse_user_id(cookie: &str) -> Result<u64, DanmuProviderError> {
        let mut user_id = None;

        cookie.split(";").into_iter().for_each(|e| {
            let kv = e
                .split("=")
                .into_iter()
                .map(|x| x.trim())
                .collect::<Vec<&str>>();
            let key = kv.first().map_or("", |v| v);
            if key == "DedeUserID" {
                if let Ok(id) = kv.last().map_or("0", |v| v).parse::<u64>() {
                    user_id = Some(id)
                }
            }
        });

        if user_id.is_none() {
            Err(DanmuProviderError::InvalidIdentifier {
                err: "Failed to find user_id in cookie".into(),
            })
        } else {
            Ok(user_id.unwrap())
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct DanmuInfo {
    pub data: DanmuInfoData,
}

#[derive(Debug, Deserialize)]
pub struct DanmuInfoData {
    pub token: String,
    pub host_list: Vec<WsHost>,
}

#[derive(Debug, Deserialize)]
pub struct WsHost {
    pub host: String,
}

#[derive(Debug, Deserialize)]
pub struct RoomInit {
    data: RoomInitData,
}

#[derive(Debug, Deserialize)]
pub struct RoomInitData {
    room_id: u64,
}
