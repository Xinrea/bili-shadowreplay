#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct WebhookEvent {
    pub id: String,
    pub event: String,
    pub payload: Payload,
    pub timestamp: i64,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub enum Payload {
    User(UserObject),
    Room(RoomObject),
    Live(LiveObject),
    Clip(ClipObject),
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
    pub room_name: String,
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
    pub start: i64,
    pub end: i64,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct ClipObject {
    pub clip_id: String,
    pub live: LiveObject,
    pub range: Range,
    pub note: String,
    pub cover: String,
    pub file: String,
    pub size: i64,
    pub length: i64,
    pub with_danmaku: bool,
    pub with_subtitle: bool,
}
