use crate::platforms::PlatformType;
use crate::RecorderInfo;

#[derive(Debug, Clone)]
pub enum RecorderEvent {
    LiveStart {
        recorder: RecorderInfo,
    },
    LiveEnd {
        room_id: String,
        platform: PlatformType,
        recorder: RecorderInfo,
    },
    RecordStart {
        recorder: RecorderInfo,
    },
    RecordEnd {
        recorder: RecorderInfo,
    },
    RecordUpdate {
        live_id: String,
        duration_secs: f64,
        cached_size_bytes: u64,
    },
    ProgressUpdate {
        id: String,
        content: String,
    },
    ProgressFinished {
        id: String,
        success: bool,
        message: String,
    },
    DanmuReceived {
        room: String,
        ts: i64,
        content: String,
        user_name: Option<String>,
    },
}

impl RecorderEvent {
    pub fn danmu_received(
        room: String,
        ts: i64,
        content: impl Into<String>,
        user_name: Option<&str>,
    ) -> Self {
        Self::DanmuReceived {
            room,
            ts,
            content: content.into(),
            user_name: optional_user_name(user_name),
        }
    }

    pub fn danmu_received_from_event(room: String, event: &danmu_stream::LiveEvent) -> Option<Self> {
        if event.event_type != "danmu" {
            return None;
        }
        Some(Self::danmu_received(
            room,
            event.ts,
            event
                .data
                .get("content")
                .and_then(|value| value.as_str())
                .unwrap_or_default(),
            event.data.get("user_name").and_then(|value| value.as_str()),
        ))
    }
}

fn optional_user_name(name: Option<&str>) -> Option<String> {
    name.map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use danmu_stream::LiveEvent;
    use serde_json::json;

    #[test]
    fn live_danmu_event_keeps_user_name_for_realtime_preview() {
        let event = LiveEvent {
            ts: 1_700_000_000_123,
            platform: "bilibili".into(),
            room_id: "123".into(),
            event_type: "danmu".into(),
            data: json!({
                "content": "hello",
                "user_name": "alice",
            }),
            raw: json!(null),
        };

        let received = RecorderEvent::danmu_received_from_event("123".into(), &event).unwrap();
        match received {
            RecorderEvent::DanmuReceived {
                room,
                ts,
                content,
                user_name,
            } => {
                assert_eq!(room, "123");
                assert_eq!(ts, 1_700_000_000_123);
                assert_eq!(content, "hello");
                assert_eq!(user_name.as_deref(), Some("alice"));
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn live_danmu_event_drops_blank_user_name() {
        let event = LiveEvent {
            ts: 1,
            platform: "bilibili".into(),
            room_id: "1".into(),
            event_type: "danmu".into(),
            data: json!({ "content": "hi", "user_name": "  " }),
            raw: json!(null),
        };
        let received = RecorderEvent::danmu_received_from_event("1".into(), &event).unwrap();
        match received {
            RecorderEvent::DanmuReceived { user_name, .. } => {
                assert_eq!(user_name, None);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn ignores_non_danmu_live_events() {
        let event = LiveEvent {
            ts: 1,
            platform: "bilibili".into(),
            room_id: "1".into(),
            event_type: "gift".into(),
            data: json!({ "user_name": "alice", "gift_name": "花束" }),
            raw: json!(null),
        };
        assert!(RecorderEvent::danmu_received_from_event("1".into(), &event).is_none());
    }
}
