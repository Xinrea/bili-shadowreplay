use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Response structure for TikTok SIGI_STATE data
/// This is a simplified structure as TikTok's actual data structure is very complex
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SigiStateResponse {
    /// LiveRoom data
    #[serde(rename = "LiveRoom")]
    pub live_room: Option<Value>,

    /// RoomStore data
    #[serde(rename = "RoomStore")]
    pub room_store: Option<RoomStore>,

    /// UserModule data
    #[serde(rename = "UserModule")]
    pub user_module: Option<Value>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomStore {
    pub room_info: Option<RoomInfo>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomInfo {
    #[serde(
        default,
        alias = "room_id",
        alias = "liveRoomId",
        alias = "live_room_id"
    )]
    pub room_id: Option<String>,
    pub title: Option<String>,
    #[serde(default, alias = "liveStatus", alias = "live_status")]
    pub status: Option<i32>,
    #[serde(
        default,
        alias = "ownerInfo",
        alias = "user",
        alias = "userInfo",
        alias = "host",
        alias = "author"
    )]
    pub owner: Option<Owner>,
    #[serde(
        default,
        alias = "streamUrl",
        alias = "stream_url",
        alias = "streamUrlInfo",
        alias = "stream_url_info"
    )]
    pub stream_url: Option<StreamUrl>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Owner {
    #[serde(
        default,
        alias = "userId",
        alias = "user_id"
    )]
    pub id: Option<String>,
    #[serde(
         default,
         alias = "uniqueId",
         alias = "unique_id"
    )]
    pub unique_id: Option<String>,
    #[serde(
        default,
        alias = "nickName",
        alias = "userName",
        alias = "user_name"
    )]
    pub nickname: Option<String>,
    pub avatar_thumb: Option<Value>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamUrl {
    #[serde(
        default,
        alias = "rtmpPullUrl",
        alias = "rtmp_pull_url",
        alias = "rtmpPlayUrl"
    )]
    pub rtmp_pull_url: Option<String>,
    #[serde(
        default,
        alias = "hlsPullUrl",
        alias = "hls_pull_url",
        alias = "hlsPlayUrl"
    )]
    pub hls_pull_url: Option<String>,
    pub flv_pull_url: Option<Value>,
}
