use crate::errors::ApiCollectionError;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RoomInfo {
    pub live_status: u8,
    pub room_cover_url: String,
    pub room_id: u64,
    pub room_keyframe_url: String,
    pub room_title: String,
    pub user_id: u64,
}

pub async fn get_room_info(
    client: &reqwest::Client,
    room_id: u64,
) -> Result<RoomInfo, ApiCollectionError> {
    let response = client
        .get(format!(
            "https://api.live.bilibili.com/room/v1/Room/get_info?room_id={}",
            room_id
        ))
        .send()
        .await?;

    if !response.status().is_success() {
        if response.status() == reqwest::StatusCode::PRECONDITION_FAILED {
            return Err(ApiCollectionError::RiskControlError);
        }
        return Err(ApiCollectionError::RequestError {
            err: response.status().to_string(),
        });
    }

    let res: serde_json::Value = response.json().await?;
    let code = res["code"]
        .as_u64()
        .ok_or(ApiCollectionError::InvalidValue {
            key: "code".to_string(),
            value: "".to_string(),
        })?;
    if code != 0 {
        return Err(ApiCollectionError::InvalidValue {
            key: "code".to_string(),
            value: code.to_string(),
        });
    }

    let room_id = res["data"]["room_id"]
        .as_u64()
        .ok_or(ApiCollectionError::InvalidValue {
            key: "room_id".to_string(),
            value: "".to_string(),
        })?;
    let room_title = res["data"]["title"]
        .as_str()
        .ok_or(ApiCollectionError::InvalidValue {
            key: "room_title".to_string(),
            value: "".to_string(),
        })?
        .to_string();
    let room_cover_url = res["data"]["user_cover"]
        .as_str()
        .ok_or(ApiCollectionError::InvalidValue {
            key: "room_cover_url".to_string(),
            value: "".to_string(),
        })?
        .to_string();
    let room_keyframe_url = res["data"]["keyframe"]
        .as_str()
        .ok_or(ApiCollectionError::InvalidValue {
            key: "room_keyframe_url".to_string(),
            value: "".to_string(),
        })?
        .to_string();
    let user_id = res["data"]["uid"]
        .as_u64()
        .ok_or(ApiCollectionError::InvalidValue {
            key: "user_id".to_string(),
            value: "".to_string(),
        })?;
    let live_status =
        res["data"]["live_status"]
            .as_u64()
            .ok_or(ApiCollectionError::InvalidValue {
                key: "live_status".to_string(),
                value: "".to_string(),
            })? as u8;
    Ok(RoomInfo {
        room_id,
        room_title,
        room_cover_url,
        room_keyframe_url,
        user_id,
        live_status,
    })
}
