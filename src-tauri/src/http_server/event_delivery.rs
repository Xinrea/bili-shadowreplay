use socketioxide::{SendError, SocketError};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};

/// Socket.IO has another bounded output queue. Temporary congestion must not
/// terminate subscriptions; control deliveries pause preview traffic and retry.
#[derive(Clone, Default)]
pub(super) struct EventDelivery {
    control_pending: Arc<AtomicBool>,
    preview_dropped: Arc<AtomicU64>,
}

struct ControlGuard(Arc<AtomicBool>);
impl Drop for ControlGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl EventDelivery {
    pub async fn control(
        &self,
        mut emit: impl FnMut() -> Result<(), SendError>,
    ) -> Result<(), SendError> {
        let mut guard = None;
        loop {
            match emit() {
                Err(SendError::Socket(SocketError::InternalChannelFull)) => {
                    if guard.is_none() {
                        self.control_pending.store(true, Ordering::Release);
                        guard = Some(ControlGuard(self.control_pending.clone()));
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
                result => return result,
            }
        }
    }

    pub fn preview(&self, emit: impl FnOnce() -> Result<(), SendError>) -> Result<(), SendError> {
        let result = if self.control_pending.load(Ordering::Acquire) {
            Err(SendError::Socket(SocketError::InternalChannelFull))
        } else {
            emit()
        };
        match result {
            Err(SendError::Socket(SocketError::InternalChannelFull)) => {
                let dropped = self.preview_dropped.fetch_add(1, Ordering::Relaxed) + 1;
                // Aggregate loss without logging once per comment in a burst.
                if dropped.is_power_of_two() {
                    log::warn!(
                        "WebSocket preview congestion: {dropped} events dropped for this client"
                    );
                }
                Ok(())
            }
            result => result,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn successful_control_does_not_pause_preview() {
        let delivery = EventDelivery::default();
        delivery
            .control(|| {
                assert!(!delivery.control_pending.load(Ordering::Acquire));
                delivery.preview(|| Ok(())).unwrap();
                Ok(())
            })
            .await
            .unwrap();
        assert_eq!(delivery.preview_dropped.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn cancelled_control_releases_preview_pause() {
        let delivery = EventDelivery::default();
        {
            let control =
                delivery.control(|| Err(SendError::Socket(SocketError::InternalChannelFull)));
            tokio::pin!(control);
            tokio::select! {
                _ = &mut control => panic!("Control must wait for capacity"),
                _ = tokio::task::yield_now() => {}
            }
            assert!(delivery.control_pending.load(Ordering::Acquire));
        }
        assert!(!delivery.control_pending.load(Ordering::Acquire));
        delivery.preview(|| Ok(())).unwrap();
    }

    #[tokio::test]
    async fn full_socket_retries_control_and_pauses_preview_until_delivery() {
        let delivery = EventDelivery::default();
        let other = delivery.clone();
        let mut attempts = 0;
        let control = delivery.control(|| {
            attempts += 1;
            if attempts < 3 {
                Err(SendError::Socket(SocketError::InternalChannelFull))
            } else {
                Ok(())
            }
        });
        tokio::pin!(control);
        tokio::select! {
            _ = &mut control => panic!("Control must wait for output capacity"),
            _ = async { tokio::task::yield_now().await; } => {}
        }
        assert!(other.control_pending.load(Ordering::Acquire));
        other
            .preview(|| panic!("Preview must yield to pending control"))
            .unwrap();
        control.await.unwrap();
        assert!(!other.control_pending.load(Ordering::Acquire));
        other.preview(|| Ok(())).unwrap();
    }

    #[test]
    fn temporary_socket_overflow_does_not_end_a_preview_subscription() {
        let delivery = EventDelivery::default();
        delivery
            .preview(|| Err(SendError::Socket(SocketError::InternalChannelFull)))
            .unwrap();
        delivery.preview(|| Ok(())).unwrap();
        assert_eq!(delivery.preview_dropped.load(Ordering::Relaxed), 1);
    }
}
