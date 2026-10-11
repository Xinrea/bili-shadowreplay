use super::{model::ClientEvent, EventBus, RoomTopic};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use tokio::task::JoinHandle;

/// One owner per connected socket/window. Dropping it stops all forwarding and
/// releases its subscriptions, including on an abrupt client disconnect.
#[derive(Default)]
pub struct RoomForwarder {
    tasks: HashMap<RoomTopic, JoinHandle<()>>,
}

pub const MAX_ROOM_TOPICS: usize = 32;

pub(super) fn validate_topics(topics: &[RoomTopic]) -> Result<(), String> {
    // Bound the input before deduplication, validation or allocating room tasks.
    if topics.len() > MAX_ROOM_TOPICS {
        return Err(format!(
            "At most {MAX_ROOM_TOPICS} room topics may be subscribed"
        ));
    }
    for topic in topics {
        topic.validate()?;
    }
    Ok(())
}

impl RoomForwarder {
    pub fn set_rooms(
        &mut self,
        bus: &EventBus,
        topics: Vec<RoomTopic>,
        emit: impl Fn(ClientEvent) -> Result<(), String> + Send + Sync + 'static,
    ) -> Result<(), String> {
        validate_topics(&topics)?;
        let topics: HashSet<_> = topics.into_iter().collect();
        self.tasks.retain(|topic, task| {
            if topics.contains(topic) && !task.is_finished() {
                true
            } else {
                task.abort();
                false
            }
        });
        let emit = Arc::new(emit);
        for topic in topics {
            self.tasks.entry(topic.clone()).or_insert_with(|| {
                let mut subscription = bus.subscribe_room(topic);
                let emit = emit.clone();
                tokio::spawn(async move {
                    while let Ok(event) = subscription.recv().await {
                        if let Err(error) = emit(event.client_event()) {
                            log::warn!("Failed to forward room preview: {error}");
                            break;
                        }
                    }
                })
            });
        }
        Ok(())
    }

    pub fn clear(&mut self) {
        for (_, task) in self.tasks.drain() {
            task.abort();
        }
    }
}

impl Drop for RoomForwarder {
    fn drop(&mut self) {
        self.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use recorder::events::RecorderEvent;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn topic_limit_rejects_atomically_without_allocating_buffers() {
        let bus = EventBus::new();
        let mut forwarder = RoomForwarder::default();
        let topics: Vec<_> = (0..MAX_ROOM_TOPICS)
            .map(|i| RoomTopic::new("bilibili", &i.to_string()))
            .collect();
        forwarder
            .set_rooms(&bus, topics.clone(), |_| Ok(()))
            .unwrap();
        assert_eq!(forwarder.tasks.len(), MAX_ROOM_TOPICS);
        let mut oversized = topics.clone();
        oversized.push(RoomTopic::new("twitch", "extra"));
        assert!(forwarder.set_rooms(&bus, oversized, |_| Ok(())).is_err());
        assert_eq!(forwarder.tasks.len(), MAX_ROOM_TOPICS);
        assert_eq!(bus.rooms.lock().unwrap().len(), MAX_ROOM_TOPICS);
        assert!(validate_topics(&vec![topics[0].clone(); MAX_ROOM_TOPICS + 1]).is_err());
        forwarder.clear();
        tokio::task::yield_now().await;
        assert!(bus.rooms.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn room_selection_replacement_and_disconnect_release_buffers() {
        let bus = EventBus::new();
        let first = RoomTopic::new("youtube", "@same");
        let other = RoomTopic::new("twitch", "@same");
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut forwarder = RoomForwarder::default();
        let emit = move |event: ClientEvent| tx.send(event).map_err(|error| error.to_string());
        forwarder
            .set_rooms(&bus, vec![first.clone(), first.clone()], emit.clone())
            .unwrap();
        assert_eq!(forwarder.tasks.len(), 1);
        bus.publish_preview(
            &other,
            &RecorderEvent::danmu_received(other.room.clone(), 1, "wrong platform", None),
        );
        bus.publish_preview(
            &first,
            &RecorderEvent::danmu_received(first.room.clone(), 2, "hello", None),
        );
        assert_eq!(rx.recv().await.unwrap().payload["content"], "hello");
        assert!(rx.try_recv().is_err());
        // Reject malformed topics atomically, keeping the valid subscriptions.
        assert!(forwarder
            .set_rooms(&bus, vec![RoomTopic::new("unknown", "1")], emit.clone())
            .is_err());
        assert_eq!(forwarder.tasks.len(), 1);
        forwarder
            .set_rooms(&bus, vec![other.clone()], emit)
            .unwrap();
        tokio::task::yield_now().await;
        assert!(!bus.rooms.lock().unwrap().contains_key(&first));
        bus.publish_preview(
            &other,
            &RecorderEvent::danmu_received(other.room.clone(), 3, "switched", None),
        );
        assert_eq!(rx.recv().await.unwrap().payload["content"], "switched");
        drop(forwarder);
        tokio::task::yield_now().await;
        assert!(bus.rooms.lock().unwrap().is_empty());
    }
}
