mod model;
mod rooms;
pub use model::{AppEvent, DataChange, RoomTopic};
pub use rooms::RoomForwarder;

use recorder::events::RecorderEvent;
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};
use tokio::sync::broadcast;

// Bounded best-effort buffers, not a duration or a durable event log. A room's
// preview budget is independent of the number of other recorded rooms.
const CONTROL_CAPACITY: usize = 1024;
const ROOM_STREAM_CAPACITY: usize = 256;
type RoomRegistry = Arc<Mutex<HashMap<RoomTopic, broadcast::WeakSender<AppEvent>>>>;

#[derive(Clone)]
pub struct EventBus {
    control: broadcast::Sender<AppEvent>,
    rooms: RoomRegistry,
    pending_changes: Arc<Mutex<HashSet<DataChange>>>,
    #[cfg(feature = "gui")]
    windows: Arc<Mutex<HashMap<String, RoomForwarder>>>,
}

pub struct EventSubscription {
    control: broadcast::Receiver<AppEvent>,
}

pub struct RoomSubscription {
    receiver: broadcast::Receiver<AppEvent>,
    sender: Option<broadcast::Sender<AppEvent>>,
    rooms: RoomRegistry,
    topic: RoomTopic,
}

impl EventBus {
    pub fn new() -> Self {
        let (control, _) = broadcast::channel(CONTROL_CAPACITY);
        Self {
            control,
            rooms: Arc::new(Mutex::new(HashMap::new())),
            pending_changes: Arc::new(Mutex::new(HashSet::new())),
            #[cfg(feature = "gui")]
            windows: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn publish(&self, event: AppEvent) {
        if let AppEvent::Danmu { topic, .. } = &event {
            let rooms = self.rooms.lock().unwrap();
            if let Some(sender) = rooms.get(topic).and_then(broadcast::WeakSender::upgrade) {
                let _ = sender.send(event);
            }
        } else {
            // No subscribers must never make a business operation fail.
            let _ = self.control.send(event);
        }
    }

    pub fn publish_recorder(&self, event: &RecorderEvent) {
        if let Some(event) = AppEvent::from_recorder(event) {
            self.publish(event);
        }
    }

    /// Aggregate frequent statistics/progress mutations across all producers.
    /// A fixed one-second window delivers the latest committed snapshot even
    /// during continuous updates; lifecycle changes still publish immediately.
    pub fn publish_coalesced_change(&self, change: DataChange) {
        if !self.pending_changes.lock().unwrap().insert(change) {
            return;
        }
        let bus = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            bus.pending_changes.lock().unwrap().remove(&change);
            bus.publish(AppEvent::DataChanged(change));
        });
    }

    pub fn publish_preview(&self, topic: &RoomTopic, event: &RecorderEvent) {
        // Do not allocate a preview payload for rooms nobody is watching.
        if !self.rooms.lock().unwrap().contains_key(topic) {
            return;
        }
        if let Some(event) = AppEvent::from_preview(topic, event) {
            self.publish(event);
        }
    }

    pub fn subscribe(&self) -> EventSubscription {
        EventSubscription {
            control: self.control.subscribe(),
        }
    }

    pub fn subscribe_room(&self, topic: RoomTopic) -> RoomSubscription {
        let mut rooms = self.rooms.lock().unwrap();
        let sender = rooms
            .get(&topic)
            .and_then(broadcast::WeakSender::upgrade)
            .unwrap_or_else(|| {
                let (sender, _) = broadcast::channel(ROOM_STREAM_CAPACITY);
                rooms.insert(topic.clone(), sender.downgrade());
                sender
            });
        RoomSubscription {
            receiver: sender.subscribe(),
            sender: Some(sender),
            topic,
            rooms: self.rooms.clone(),
        }
    }

    #[cfg(feature = "gui")]
    pub fn forward_to_app(&self, app: tauri::AppHandle) {
        use tauri::Emitter;
        let mut events = self.subscribe();
        tokio::spawn(async move {
            while let Ok(event) = events.recv().await {
                let event = event.client_event();
                if let Err(error) = app.emit(&event.name, &event.payload) {
                    log::warn!("Failed to emit application event: {error}");
                }
            }
        });
    }

    #[cfg(feature = "gui")]
    pub fn set_window_rooms(
        &self,
        window: &tauri::WebviewWindow,
        topics: Vec<RoomTopic>,
    ) -> Result<(), String> {
        use tauri::Emitter;
        rooms::validate_topics(&topics)?;
        let mut windows = self.windows.lock().unwrap();
        if topics.is_empty() {
            windows.remove(window.label());
        } else {
            let window = window.clone();
            windows
                .entry(window.label().to_owned())
                .or_default()
                .set_rooms(self, topics, move |event| {
                    window
                        .emit_to(
                            tauri::EventTarget::webview_window(window.label()),
                            &event.name,
                            &event.payload,
                        )
                        .map_err(|error| error.to_string())
                })?;
        }
        Ok(())
    }

    #[cfg(feature = "gui")]
    pub fn remove_window(&self, label: &str) {
        self.windows.lock().unwrap().remove(label);
    }
}

impl EventSubscription {
    pub async fn recv(&mut self) -> Result<AppEvent, broadcast::error::RecvError> {
        match self.control.recv().await {
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                log::warn!("Control subscriber lagged, skipped {skipped} events; resync required");
                Ok(AppEvent::DataChanged(DataChange::Resync))
            }
            result => result,
        }
    }
}

impl RoomSubscription {
    pub async fn recv(&mut self) -> Result<AppEvent, broadcast::error::RecvError> {
        loop {
            match self.receiver.recv().await {
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    log::warn!(
                        "Preview subscriber lagged for {}:{}, skipped {skipped} events",
                        self.topic.platform,
                        self.topic.room
                    );
                    // Preview loss never invalidates a database snapshot.
                }
                result => return result,
            }
        }
    }
}

impl Drop for RoomSubscription {
    fn drop(&mut self) {
        // Keep lookup/last-owner removal atomic with concurrent subscriptions.
        let mut rooms = self.rooms.lock().unwrap();
        if self.sender.as_ref().unwrap().strong_count() == 1 {
            rooms.remove(&self.topic);
        }
        drop(self.sender.take());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use recorder::danmu::DanmuEntry;

    fn preview(topic: &RoomTopic, ts: i64) -> AppEvent {
        AppEvent::Danmu {
            topic: topic.clone(),
            entry: DanmuEntry {
                ts,
                event_type: "danmu".into(),
                content: "test".into(),
                emoji_content: None,
                user_name: None,
                price: None,
                sc_duration: None,
            },
        }
    }

    #[tokio::test]
    async fn subscribers_receive_the_same_task_and_state_notifications() {
        let bus = EventBus::new();
        let mut first = bus.subscribe();
        let mut second = bus.subscribe();
        bus.publish(AppEvent::ProgressUpdate {
            id: "task".into(),
            content: "working".into(),
        });
        bus.publish(AppEvent::DataChanged(DataChange::Videos));
        bus.publish(AppEvent::ProgressFinished {
            id: "task".into(),
            success: true,
            message: "done".into(),
        });
        for expected in [
            "progress-update:task",
            "videos-changed",
            "progress-finished:task",
        ] {
            let a = first.recv().await.unwrap().client_event();
            let b = second.recv().await.unwrap().client_event();
            assert_eq!(a.name, expected);
            assert_eq!(a.name, b.name);
            assert_eq!(a.payload, b.payload);
        }
    }

    #[tokio::test]
    async fn hot_rooms_cannot_evict_other_rooms_or_control_events() {
        let bus = EventBus::new();
        let mut control = bus.subscribe();
        let hot = RoomTopic::new("bilibili", "1");
        let quiet = RoomTopic::new("twitch", "1");
        let mut hot_sub = bus.subscribe_room(hot.clone());
        let mut quiet_sub = bus.subscribe_room(quiet.clone());
        bus.publish(preview(&quiet, 42));
        for i in 0..10_000 {
            bus.publish(preview(&hot, i));
        }
        bus.publish(AppEvent::DataChanged(DataChange::Videos));
        bus.publish(AppEvent::ProgressFinished {
            id: "task".into(),
            success: true,
            message: "done".into(),
        });
        assert_eq!(
            control.recv().await.unwrap().client_event().name,
            "videos-changed"
        );
        assert_eq!(
            control.recv().await.unwrap().client_event().name,
            "progress-finished:task"
        );
        let quiet_event = quiet_sub.recv().await.unwrap().client_event();
        assert_eq!(quiet_event.payload["ts"], 42);
        assert_eq!(quiet_event.payload["platform"], "twitch");
        let hot_event = hot_sub.recv().await.unwrap();
        assert!(matches!(hot_event, AppEvent::Danmu { .. }));
        assert_eq!(
            hot_event.client_event().payload["ts"],
            10_000 - ROOM_STREAM_CAPACITY
        );
        assert!(matches!(
            control.control.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
    }

    #[tokio::test]
    async fn control_overflow_still_requests_snapshot_reconciliation() {
        let bus = EventBus::new();
        let mut control = bus.subscribe();
        for _ in 0..CONTROL_CAPACITY + 1 {
            bus.publish(AppEvent::DataChanged(DataChange::Videos));
        }
        assert!(matches!(
            control.recv().await.unwrap(),
            AppEvent::DataChanged(DataChange::Resync)
        ));
        assert!(matches!(
            control.recv().await.unwrap(),
            AppEvent::DataChanged(DataChange::Videos)
        ));
    }

    #[test]
    fn room_buffers_exist_only_while_watched() {
        let bus = EventBus::new();
        let topic = RoomTopic::new("youtube", "@test");
        bus.publish(preview(&topic, 1));
        assert!(bus.rooms.lock().unwrap().is_empty());
        let first = bus.subscribe_room(topic.clone());
        let second = bus.subscribe_room(topic);
        assert_eq!(bus.rooms.lock().unwrap().len(), 1);
        drop(first);
        assert_eq!(bus.rooms.lock().unwrap().len(), 1);
        drop(second);
        assert!(bus.rooms.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn frequent_changes_are_coalesced_without_delaying_completion() {
        let bus = EventBus::new();
        let mut subscription = bus.subscribe();
        for _ in 0..1_000 {
            bus.publish_coalesced_change(DataChange::Records);
            bus.publish_coalesced_change(DataChange::Tasks);
        }
        assert_eq!(bus.pending_changes.lock().unwrap().len(), 2);
        bus.publish(AppEvent::ProgressFinished {
            id: "task".into(),
            success: true,
            message: "done".into(),
        });
        assert_eq!(
            subscription.recv().await.unwrap().client_event().name,
            "progress-finished:task"
        );
        let first = subscription.recv().await.unwrap().client_event().name;
        let second = subscription.recv().await.unwrap().client_event().name;
        assert_ne!(first, second);
        assert!(["records-changed", "tasks-changed"].contains(&first.as_str()));
        assert!(["records-changed", "tasks-changed"].contains(&second.as_str()));
        assert!(subscription.control.try_recv().is_err());
        bus.publish_coalesced_change(DataChange::Records);
        assert_eq!(
            subscription.recv().await.unwrap().client_event().name,
            "records-changed"
        );
    }

    #[tokio::test]
    async fn shutdown_closes_control_subscriptions() {
        let bus = EventBus::new();
        let mut subscription = bus.subscribe();
        drop(bus);
        assert!(matches!(
            subscription.recv().await,
            Err(broadcast::error::RecvError::Closed)
        ));
    }
}
