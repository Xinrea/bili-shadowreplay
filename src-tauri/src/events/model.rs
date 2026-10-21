use recorder::{danmu::DanmuEntry, events::RecorderEvent};
use serde_json::{json, Value};

use crate::progress::progress_reporter::StageProgress;

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct RoomTopic {
    pub platform: String,
    pub room: String,
}

impl RoomTopic {
    pub fn new(platform: &str, room: &str) -> Self {
        Self {
            platform: platform.into(),
            room: room.into(),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        self.platform.parse::<recorder::platforms::PlatformType>()?;
        if self.room.is_empty() || self.room.len() > 512 {
            return Err("Invalid event room".into());
        }
        Ok(())
    }

    pub fn event_name(&self) -> String {
        // Tauri event names cannot contain '@' or arbitrary Unicode. Hex also
        // keeps delimiters in a room ID from colliding with another topic.
        use std::fmt::Write;
        let mut name = format!("danmu:{}:", self.platform);
        for byte in self.room.as_bytes() {
            write!(name, "{byte:02x}").unwrap();
        }
        name
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DataChange {
    Videos,
    Projects,
    Uploads,
    Tasks,
    Recorders,
    Records,
    Audio,
    Resync,
}

impl DataChange {
    fn event_name(self) -> &'static str {
        match self {
            Self::Videos => "videos-changed",
            Self::Projects => "projects-changed",
            Self::Uploads => "uploads-changed",
            Self::Tasks => "tasks-changed",
            Self::Recorders => "recorders-changed",
            Self::Records => "records-changed",
            Self::Audio => "audio-changed",
            Self::Resync => "state-resync",
        }
    }
}

/// All application notifications use this model, regardless of their producer
/// or whether the consumer connects through Tauri or Socket.IO.
#[derive(Debug, Clone)]
pub enum AppEvent {
    ProgressUpdate {
        id: String,
        content: String,
        /// Structured form of `content` for clients that draw a progress bar.
        progress: Option<StageProgress>,
    },
    ProgressFinished {
        id: String,
        success: bool,
        message: String,
    },
    Danmu {
        topic: RoomTopic,
        entry: DanmuEntry,
    },
    DataChanged(DataChange),
    /// A preview window asks the main window to start an upload of a
    /// `material` or `project`.
    UploadRequested {
        source: String,
        id: i64,
    },
}

pub struct ClientEvent {
    pub name: String,
    pub payload: Value,
}

impl AppEvent {
    /// Canonical client event names and payloads shared by both transports.
    pub fn client_event(&self) -> ClientEvent {
        let (name, payload) = match self {
            Self::ProgressUpdate {
                id,
                content,
                progress,
            } => {
                let mut payload = json!({ "id": id, "content": content });
                if let Some(progress) = progress {
                    payload["progress"] = json!(progress);
                }
                (format!("progress-update:{id}"), payload)
            }
            Self::ProgressFinished {
                id,
                success,
                message,
            } => (
                format!("progress-finished:{id}"),
                json!({ "id": id, "success": success, "message": message }),
            ),
            Self::Danmu { topic, entry } => {
                let mut payload = json!(entry);
                payload["room"] = json!(topic.room);
                payload["platform"] = json!(topic.platform);
                (topic.event_name(), payload)
            }
            Self::DataChanged(change) => (change.event_name().into(), Value::Null),
            Self::UploadRequested { source, id } => (
                "upload-requested".into(),
                json!({ "source": source, "id": id }),
            ),
        };
        ClientEvent { name, payload }
    }

    /// Recorder lifecycle events stay in the recorder domain; only its client
    /// notifications enter the application event bus.
    pub fn from_recorder(event: &RecorderEvent) -> Option<Self> {
        match event {
            RecorderEvent::ProgressUpdate { id, content } => Some(Self::ProgressUpdate {
                id: id.clone(),
                content: content.clone(),
                progress: None,
            }),
            RecorderEvent::ProgressFinished {
                id,
                success,
                message,
            } => Some(Self::ProgressFinished {
                id: id.clone(),
                success: *success,
                message: message.clone(),
            }),
            _ => None,
        }
    }

    pub fn from_preview(topic: &RoomTopic, event: &RecorderEvent) -> Option<Self> {
        match event {
            RecorderEvent::DanmuReceived {
                room: _,
                ts,
                event_type,
                content,
                emoji_content,
                user_name,
                price,
                sc_duration,
            } => Some(Self::Danmu {
                topic: topic.clone(),
                entry: DanmuEntry {
                    ts: *ts,
                    event_type: event_type.clone(),
                    content: content.clone(),
                    emoji_content: emoji_content.clone(),
                    user_name: user_name.clone(),
                    price: *price,
                    sc_duration: *sc_duration,
                },
            }),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_names_and_payloads_are_scoped_to_the_task() {
        let update = AppEvent::ProgressUpdate {
            id: "task-1".into(),
            content: "encoding".into(),
            progress: None,
        }
        .client_event();
        assert_eq!(update.name, "progress-update:task-1");
        assert_eq!(
            update.payload,
            json!({"id": "task-1", "content": "encoding"})
        );
        let measured = AppEvent::ProgressUpdate {
            id: "task-1".into(),
            content: "压制弹幕 40%".into(),
            progress: Some(StageProgress {
                stage: "压制弹幕".into(),
                processed: 12.0,
                total: Some(30.0),
                speed: None,
            }),
        }
        .client_event();
        assert_eq!(
            measured.payload,
            json!({
                "id": "task-1", "content": "压制弹幕 40%",
                "progress": {"stage": "压制弹幕", "processed": 12.0, "total": 30.0},
            })
        );
        let finish = AppEvent::ProgressFinished {
            id: "task-1".into(),
            success: false,
            message: "cancelled".into(),
        }
        .client_event();
        assert_eq!(finish.name, "progress-finished:task-1");
        assert_eq!(
            finish.payload,
            json!({"id": "task-1", "success": false, "message": "cancelled"})
        );
    }

    #[test]
    fn preview_client_payload_preserves_image_metadata_and_text() {
        let live = danmu_stream::LiveEvent::new(
            "bilibili",
            "123",
            "danmu",
            json!({
                "content": "[emoji]",
                "emoji_content": {"url": "https://example.com/image.webp", "width": 80}
            }),
        );
        let recorder = RecorderEvent::danmu_received_from_event("123".into(), &live).unwrap();
        let client = AppEvent::from_preview(&RoomTopic::new("bilibili", "123"), &recorder)
            .unwrap()
            .client_event();
        assert_eq!(client.payload["content"], "[emoji]");
        assert_eq!(client.payload["emoji_content"], live.data["emoji_content"]);
    }

    #[test]
    fn recorder_danmu_preserves_room_names_and_super_chat_fields() {
        let event = RecorderEvent::DanmuReceived {
            room: "@EliraPendora".into(),
            ts: 123,
            event_type: "super_chat".into(),
            content: "hello".into(),
            emoji_content: None,
            user_name: Some("alice".into()),
            price: Some(300),
            sc_duration: Some(60),
        };
        let topic = RoomTopic::new("youtube", "@EliraPendora");
        let client = AppEvent::from_preview(&topic, &event)
            .unwrap()
            .client_event();
        assert_eq!(client.name, "danmu:youtube:40456c69726150656e646f7261");
        assert_eq!(
            client.payload,
            json!({
                "platform": "youtube", "room": "@EliraPendora", "ts": 123, "type": "super_chat", "content": "hello",
                "user_name": "alice", "price": 300, "sc_duration": 60,
                "emoji_content": null,
            })
        );
    }
}
