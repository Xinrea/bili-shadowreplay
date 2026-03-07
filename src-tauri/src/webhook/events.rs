use uuid::Uuid;

use crate::database::{
    account::AccountRow, record::RecordRow, recorder::RecorderRow, video::VideoRow,
};

use recorder::RecorderInfo;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_type_constants() {
        assert_eq!(CLIP_GENERATED, "clip.generated");
        assert_eq!(CLIP_DELETED, "clip.deleted");
        assert_eq!(RECORD_STARTED, "record.started");
        assert_eq!(RECORD_ENDED, "record.ended");
        assert_eq!(LIVE_STARTED, "live.started");
        assert_eq!(LIVE_ENDED, "live.ended");
        assert_eq!(ARCHIVE_DELETED, "archive.deleted");
        assert_eq!(RECORDER_REMOVED, "recorder.removed");
        assert_eq!(RECORDER_ADDED, "recorder.added");
    }

    #[test]
    fn test_new_webhook_event() {
        let payload = Payload::Room(RecorderInfo {
            room_info: recorder::RoomInfo::default(),
            user_info: recorder::UserInfo::default(),
            platform_live_id: "plid".to_string(),
            live_id: "lid".to_string(),
            recording: true,
            enabled: true,
        });
        let event = new_webhook_event(LIVE_STARTED, payload);
        assert_eq!(event.event, "live.started");
        assert!(!event.id.is_empty());
        assert!(event.timestamp > 0);
    }

    #[test]
    fn test_webhook_event_unique_ids() {
        let payload1 = Payload::Room(RecorderInfo {
            room_info: recorder::RoomInfo::default(),
            user_info: recorder::UserInfo::default(),
            platform_live_id: "".to_string(),
            live_id: "".to_string(),
            recording: false,
            enabled: false,
        });
        let payload2 = Payload::Room(RecorderInfo {
            room_info: recorder::RoomInfo::default(),
            user_info: recorder::UserInfo::default(),
            platform_live_id: "".to_string(),
            live_id: "".to_string(),
            recording: false,
            enabled: false,
        });
        let e1 = new_webhook_event(LIVE_STARTED, payload1);
        let e2 = new_webhook_event(LIVE_STARTED, payload2);
        assert_ne!(e1.id, e2.id);
    }

    #[test]
    fn test_webhook_event_serialization() {
        let payload = Payload::Room(RecorderInfo {
            room_info: recorder::RoomInfo::default(),
            user_info: recorder::UserInfo::default(),
            platform_live_id: "".to_string(),
            live_id: "".to_string(),
            recording: false,
            enabled: false,
        });
        let event = new_webhook_event(RECORD_STARTED, payload);
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("record.started"));
        // Deserialize back
        let deserialized: WebhookEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.event, "record.started");
        assert_eq!(deserialized.id, event.id);
    }
}
