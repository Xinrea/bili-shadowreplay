#[cfg(feature = "headless")]
use tokio::sync::broadcast;

#[cfg(feature = "headless")]
pub struct ProgressManager {
    pub progress_sender: broadcast::Sender<Event>,
    pub progress_receiver: broadcast::Receiver<Event>,
}

#[cfg(feature = "headless")]
impl ProgressManager {
    pub fn new() -> Self {
        let (progress_sender, progress_receiver) = broadcast::channel(256);
        Self {
            progress_sender,
            progress_receiver,
        }
    }

    pub fn get_event_sender(&self) -> broadcast::Sender<Event> {
        self.progress_sender.clone()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.progress_receiver.resubscribe()
    }
}
