use uuid::Uuid;

use crate::{
    database::{account::AccountRow, record::RecordRow, recorder::RecorderRow, video::VideoRow},
    recorder::RecorderInfo,
};

pub const CLIP_GENERATED: &str = "clip.generated";
pub const CLIP_DELETED: &str = "clip.deleted";

pub const RECORD_STARTED: &str = "record.started";
pub const RECORD_ENDED: &str = "record.ended";

pub const LIVE_STARTED: &str = "live.started";
pub const LIVE_ENDED: &str = "live.ended";

pub const ARCHIVE_DELETED: &str = "archive.deleted";

pub const RECORDER_REMOVED: &str = "recorder.removed";
pub const RECORDER_ADDED: &str = "recorder.added";

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
    Account(AccountRow),
    Recorder(RecorderRow),
    Room(RecorderInfo),
    Clip(VideoRow),
    Archive(RecordRow),
}

pub fn new_webhook_event(event_type: &str, payload: Payload) -> WebhookEvent {
    WebhookEvent {
        id: Uuid::new_v4().to_string(),
        event: event_type.to_string(),
        payload,
        timestamp: chrono::Utc::now().timestamp(),
    }
}
