use uuid::Uuid;

use crate::database::video::VideoRow;

pub const CLIP_GENERATED: &str = "clip.generated";
pub const CLIP_DELETED: &str = "clip.deleted";

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct WebhookEvent {
    pub id: String,
    pub event: String,
    pub payload: Payload,
    pub timestamp: i64,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum Payload {
    User(UserObject),
    Room(RoomObject),
    Live(LiveObject),
    Clip(VideoRow),
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct UserObject {
    pub user_id: String,
    pub user_name: String,
    pub user_avatar: String,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct RoomObject {
    pub room_id: String,
    pub platform: String,
    pub room_title: String,
    pub room_cover: String,
    pub room_owner: UserObject,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct LiveObject {
    pub live_id: String,
    pub room: RoomObject,
    pub start_time: Option<i64>,
    pub end_time: Option<i64>,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct Range {
    pub start: f64,
    pub end: f64,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct ClipObject {
    pub clip_id: String,
    pub range: Range,
    pub note: String,
    pub cover: String,
    pub file: String,
    pub size: i64,
    pub length: i64,
    pub with_danmaku: bool,
    pub with_subtitle: bool,
}

pub fn new_webhook_event(event_type: &str, payload: Payload) -> WebhookEvent {
    WebhookEvent {
        id: Uuid::new_v4().to_string(),
        event: event_type.to_string(),
        payload,
        timestamp: chrono::Utc::now().timestamp(),
    }
}
