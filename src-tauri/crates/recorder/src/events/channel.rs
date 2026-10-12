use super::RecorderEvent;
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};
use tokio::sync::{broadcast, mpsc, Notify};

#[derive(Default)]
struct PendingUpdates {
    deltas: HashMap<String, (f64, u64)>,
    ready: VecDeque<String>,
}
type Updates = Arc<Mutex<PendingUpdates>>;

/// Lifecycle events have bounded backpressure; recording deltas are coalesced
/// per archive so slow event handling never stalls segment downloads.
#[derive(Clone)]
pub struct RecorderEventSender {
    inner: SenderKind,
}

#[derive(Clone)]
enum SenderKind {
    Broadcast(broadcast::Sender<RecorderEvent>),
    Queued {
        control: mpsc::Sender<RecorderEvent>,
        updates: Updates,
        updated: Arc<Notify>,
    },
}

pub struct RecorderEventReceiver {
    control: mpsc::Receiver<RecorderEvent>,
    updates: Updates,
    updated: Arc<Notify>,
    pending: Option<RecorderEvent>,
}

pub fn recorder_event_channel(capacity: usize) -> (RecorderEventSender, RecorderEventReceiver) {
    let (tx, rx) = mpsc::channel(capacity);
    let updates = Updates::default();
    let updated = Arc::new(Notify::new());
    (
        RecorderEventSender {
            inner: SenderKind::Queued {
                control: tx,
                updates: updates.clone(),
                updated: updated.clone(),
            },
        },
        RecorderEventReceiver {
            control: rx,
            updates,
            updated,
            pending: None,
        },
    )
}

impl From<broadcast::Sender<RecorderEvent>> for RecorderEventSender {
    fn from(sender: broadcast::Sender<RecorderEvent>) -> Self {
        Self {
            inner: SenderKind::Broadcast(sender),
        }
    }
}

impl RecorderEventSender {
    pub async fn send(
        &self,
        event: RecorderEvent,
    ) -> Result<(), mpsc::error::SendError<RecorderEvent>> {
        match &self.inner {
            SenderKind::Broadcast(sender) => sender
                .send(event)
                .map(|_| ())
                .map_err(|error| mpsc::error::SendError(error.0)),
            SenderKind::Queued {
                control,
                updates,
                updated,
            } => {
                if let RecorderEvent::RecordUpdate {
                    live_id,
                    duration_secs,
                    cached_size_bytes,
                } = event
                {
                    if control.is_closed() {
                        return Err(mpsc::error::SendError(RecorderEvent::RecordUpdate {
                            live_id,
                            duration_secs,
                            cached_size_bytes,
                        }));
                    }
                    let mut updates = updates.lock().unwrap();
                    if !updates.deltas.contains_key(&live_id) {
                        updates.ready.push_back(live_id.clone());
                    }
                    let delta = updates.deltas.entry(live_id).or_default();
                    delta.0 += duration_secs;
                    delta.1 += cached_size_bytes;
                    updated.notify_one();
                    Ok(())
                } else {
                    control.send(event).await
                }
            }
        }
    }

    /// Legacy broadcast consumers can still opt into preview events. The
    /// bounded application channel never carries high-frequency preview data.
    pub fn send_preview(&self, event: RecorderEvent) {
        if let SenderKind::Broadcast(sender) = &self.inner {
            let _ = sender.send(event);
        }
    }
}

impl RecorderEventReceiver {
    fn take_update(&self, live_id: Option<&str>) -> Option<RecorderEvent> {
        let mut updates = self.updates.lock().unwrap();
        let live_id = match live_id {
            Some(id) => {
                updates.ready.retain(|queued| queued != id);
                id.to_owned()
            }
            None => updates.ready.pop_front()?,
        };
        let (duration_secs, cached_size_bytes) = updates.deltas.remove(&live_id)?;
        if !updates.ready.is_empty() {
            self.updated.notify_one();
        }
        Some(RecorderEvent::RecordUpdate {
            live_id,
            duration_secs,
            cached_size_bytes,
        })
    }

    pub async fn recv(&mut self) -> Option<RecorderEvent> {
        if let Some(event) = self.pending.take() {
            return Some(event);
        }
        loop {
            tokio::select! {
                biased;
                event = self.control.recv() => {
                    let Some(event) = event else { return self.take_update(None); };
                    // Persist the last accumulated deltas before RecordEnd
                    // checks archive size/duration or removes an empty record.
                    if let RecorderEvent::RecordEnd { recorder } | RecorderEvent::LiveEnd { recorder, .. } = &event {
                        if let Some(update) = self.take_update(Some(&recorder.live_id)) {
                            self.pending = Some(event);
                            return Some(update);
                        }
                    }
                    return Some(event);
                }
                _ = self.updated.notified() => {
                    if let Some(event) = self.take_update(None) { return Some(event); }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{platforms::PlatformType, RecorderInfo, RoomInfo, UserInfo};

    fn snapshot(id: &str) -> RecorderInfo {
        RecorderInfo {
            room_info: RoomInfo::default(),
            user_info: UserInfo::default(),
            platform_live_id: "session".into(),
            live_id: id.into(),
            recording: true,
            enabled: true,
        }
    }

    fn delta(id: &str) -> RecorderEvent {
        RecorderEvent::RecordUpdate {
            live_id: id.into(),
            duration_secs: 0.25,
            cached_size_bytes: 10,
        }
    }

    #[tokio::test]
    async fn full_lifecycle_queue_waits_without_losing_start_or_end() {
        let (sender, mut receiver) = recorder_event_channel(1);
        sender
            .send(RecorderEvent::RecordStart {
                recorder: snapshot("a"),
            })
            .await
            .unwrap();
        let second = sender.clone();
        let end = tokio::spawn(async move {
            second
                .send(RecorderEvent::RecordEnd {
                    recorder: snapshot("a"),
                })
                .await
                .unwrap();
        });
        tokio::task::yield_now().await;
        assert!(!end.is_finished());
        assert!(matches!(
            receiver.recv().await,
            Some(RecorderEvent::RecordStart { .. })
        ));
        end.await.unwrap();
        assert!(matches!(
            receiver.recv().await,
            Some(RecorderEvent::RecordEnd { .. })
        ));
    }

    #[tokio::test]
    async fn recording_deltas_coalesce_without_blocking_and_flush_before_end() {
        let (sender, mut receiver) = recorder_event_channel(1);
        sender
            .send(RecorderEvent::RecordStart {
                recorder: snapshot("a"),
            })
            .await
            .unwrap();
        for _ in 0..10_000 {
            sender.send(delta("a")).await.unwrap();
        }
        let queued = match &sender.inner {
            SenderKind::Queued { updates, .. } => updates.lock().unwrap().deltas.len(),
            _ => unreachable!(),
        };
        assert_eq!(queued, 1);
        assert!(matches!(
            receiver.recv().await,
            Some(RecorderEvent::RecordStart { .. })
        ));
        sender
            .send(RecorderEvent::RecordEnd {
                recorder: snapshot("a"),
            })
            .await
            .unwrap();
        match receiver.recv().await.unwrap() {
            RecorderEvent::RecordUpdate {
                duration_secs,
                cached_size_bytes,
                ..
            } => {
                assert_eq!(duration_secs, 2500.0);
                assert_eq!(cached_size_bytes, 100_000);
            }
            other => panic!("Expected accumulated deltas, got {other:?}"),
        }
        assert!(matches!(
            receiver.recv().await,
            Some(RecorderEvent::RecordEnd { .. })
        ));
    }

    #[tokio::test]
    async fn deltas_flush_before_live_end_and_hot_archives_do_not_starve_other_updates() {
        let (sender, mut receiver) = recorder_event_channel(1);
        sender.send(delta("hot")).await.unwrap();
        sender.send(delta("quiet")).await.unwrap();
        assert!(
            matches!(receiver.recv().await, Some(RecorderEvent::RecordUpdate { live_id, .. }) if live_id == "hot")
        );
        sender.send(delta("hot")).await.unwrap();
        assert!(
            matches!(receiver.recv().await, Some(RecorderEvent::RecordUpdate { live_id, .. }) if live_id == "quiet")
        );
        sender
            .send(RecorderEvent::LiveEnd {
                recorder: snapshot("hot"),
                room_id: "1".into(),
                platform: PlatformType::BiliBili,
            })
            .await
            .unwrap();
        assert!(
            matches!(receiver.recv().await, Some(RecorderEvent::RecordUpdate { live_id, .. }) if live_id == "hot")
        );
        assert!(matches!(
            receiver.recv().await,
            Some(RecorderEvent::LiveEnd { .. })
        ));
    }

    #[tokio::test]
    async fn preview_events_never_enter_the_bounded_lifecycle_queue() {
        let (sender, mut receiver) = recorder_event_channel(1);
        for _ in 0..10_000 {
            sender.send_preview(RecorderEvent::danmu_received("1".into(), 0, "hello", None));
        }
        sender
            .send(RecorderEvent::RecordStart {
                recorder: snapshot("a"),
            })
            .await
            .unwrap();
        assert!(matches!(
            receiver.recv().await,
            Some(RecorderEvent::RecordStart { .. })
        ));
        drop(sender);
        assert!(receiver.recv().await.is_none());
    }

    #[tokio::test]
    async fn closed_receiver_is_reported_and_pending_deltas_drain_on_shutdown() {
        let (sender, mut receiver) = recorder_event_channel(1);
        sender.send(delta("a")).await.unwrap();
        drop(sender);
        assert!(matches!(
            receiver.recv().await,
            Some(RecorderEvent::RecordUpdate { .. })
        ));
        assert!(receiver.recv().await.is_none());
        let (sender, receiver) = recorder_event_channel(1);
        drop(receiver);
        assert!(sender.send(delta("a")).await.is_err());
        assert!(sender
            .send(RecorderEvent::RecordStart {
                recorder: snapshot("a")
            })
            .await
            .is_err());
    }
}
