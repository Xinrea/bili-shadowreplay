mod model;
pub use model::{AppEvent, DataChange};

use recorder::events::RecorderEvent;
use tokio::sync::broadcast;

/// One publishing/subscription API for all application notifications. Streaming
/// danmu has its own bounded buffer so it cannot evict task or state events.
#[derive(Clone)]
pub struct EventBus {
    control: broadcast::Sender<AppEvent>,
    stream: broadcast::Sender<AppEvent>,
}

pub struct EventSubscription {
    control: broadcast::Receiver<AppEvent>,
    stream: broadcast::Receiver<AppEvent>,
}

impl EventBus {
    pub fn new() -> Self {
        let (control, _) = broadcast::channel(1024);
        let (stream, _) = broadcast::channel(256);
        Self { control, stream }
    }

    pub fn publish(&self, event: AppEvent) {
        let sender = if matches!(event, AppEvent::Danmu { .. }) {
            &self.stream
        } else {
            &self.control
        };
        // No connected subscribers must never make a business operation fail.
        let _ = sender.send(event);
    }

    pub fn publish_recorder(&self, event: &RecorderEvent) {
        if let Some(event) = AppEvent::from_recorder(event) {
            self.publish(event);
        }
    }

    pub fn subscribe(&self) -> EventSubscription {
        EventSubscription {
            control: self.control.subscribe(),
            stream: self.stream.subscribe(),
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
}

impl EventSubscription {
    pub async fn recv(&mut self) -> Result<AppEvent, broadcast::error::RecvError> {
        let result = tokio::select! {
            biased;
            result = self.control.recv() => result,
            result = self.stream.recv() => result,
        };
        match result {
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                log::warn!("Application event subscriber lagged, skipped {skipped} events");
                Ok(AppEvent::DataChanged(DataChange::Resync))
            }
            result => result,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use recorder::danmu::DanmuEntry;

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
    async fn danmu_overflow_cannot_evict_task_completion_or_state_changes() {
        let bus = EventBus::new();
        let mut subscription = bus.subscribe();
        for i in 0..300 {
            bus.publish(AppEvent::Danmu {
                room: "1".into(),
                entry: DanmuEntry {
                    ts: i,
                    event_type: "danmu".into(),
                    content: "test".into(),
                    user_name: None,
                    price: None,
                    sc_duration: None,
                },
            });
        }
        bus.publish(AppEvent::DataChanged(DataChange::Videos));
        bus.publish(AppEvent::ProgressFinished {
            id: "task".into(),
            success: true,
            message: "done".into(),
        });
        assert_eq!(
            subscription.recv().await.unwrap().client_event().name,
            "videos-changed"
        );
        assert_eq!(
            subscription.recv().await.unwrap().client_event().name,
            "progress-finished:task"
        );
        assert_eq!(
            subscription.recv().await.unwrap().client_event().name,
            "state-resync"
        );
        assert_eq!(
            subscription.recv().await.unwrap().client_event().name,
            "danmu"
        );
    }

    #[tokio::test]
    async fn shutdown_closes_subscriptions() {
        let bus = EventBus::new();
        let mut subscription = bus.subscribe();
        drop(bus);
        assert!(matches!(
            subscription.recv().await,
            Err(broadcast::error::RecvError::Closed)
        ));
    }
}
