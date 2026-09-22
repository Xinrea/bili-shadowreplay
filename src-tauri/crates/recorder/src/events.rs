use crate::danmu::PREVIEW_EVENT_TYPES;
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
        /// Event type, see [`crate::danmu::PREVIEW_EVENT_TYPES`].
        event_type: String,
        content: String,
        user_name: Option<String>,
        /// Super chat price in CNY (battery). `None` for regular danmaku.
        price: Option<u32>,
        /// Super chat pinned duration in seconds.
        sc_duration: Option<u32>,
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
            event_type: "danmu".to_string(),
            content: content.into(),
            user_name: optional_user_name(user_name),
            price: None,
            sc_duration: None,
        }
    }

    /// Map a recorded live event into the preview pipeline. Only event types
    /// listed in [`crate::danmu::PREVIEW_EVENT_TYPES`] are forwarded; super
    /// chats additionally carry price/duration.
    pub fn danmu_received_from_event(
        room: String,
        event: &danmu_stream::LiveEvent,
    ) -> Option<Self> {
        if !PREVIEW_EVENT_TYPES.contains(&event.event_type.as_str()) {
            return None;
        }
        let is_super_chat = event.event_type == "super_chat";
        Some(Self::DanmuReceived {
            room,
            ts: event.ts,
            event_type: event.event_type.clone(),
            content: event
                .data
                .get("content")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string(),
            user_name: optional_user_name(
                event.data.get("user_name").and_then(|value| value.as_str()),
            ),
            price: if is_super_chat {
                event
                    .data
                    .get("price")
                    .and_then(|value| value.as_u64())
                    .map(|value| value as u32)
            } else {
                None
            },
            sc_duration: if is_super_chat {
                event
                    .data
                    .get("duration")
                    .and_then(|value| value.as_u64())
                    .map(|value| value as u32)
            } else {
                None
            },
        })
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
                event_type,
                content,
                user_name,
                price,
                sc_duration,
            } => {
                assert_eq!(room, "123");
                assert_eq!(ts, 1_700_000_000_123);
                assert_eq!(event_type, "danmu");
                assert_eq!(content, "hello");
                assert_eq!(user_name.as_deref(), Some("alice"));
                assert_eq!(price, None);
                assert_eq!(sc_duration, None);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn live_super_chat_event_keeps_price_and_duration() {
        let event = LiveEvent {
            ts: 1_700_000_001_000,
            platform: "bilibili".into(),
            room_id: "123".into(),
            event_type: "super_chat".into(),
            data: json!({
                "content": "hello sc",
                "user_name": "alice",
                "price": 30,
                "duration": 60,
            }),
            raw: json!(null),
        };

        let received = RecorderEvent::danmu_received_from_event("123".into(), &event).unwrap();
        match received {
            RecorderEvent::DanmuReceived {
                event_type,
                content,
                price,
                sc_duration,
                ..
            } => {
                assert_eq!(event_type, "super_chat");
                assert_eq!(content, "hello sc");
                assert_eq!(price, Some(30));
                assert_eq!(sc_duration, Some(60));
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
