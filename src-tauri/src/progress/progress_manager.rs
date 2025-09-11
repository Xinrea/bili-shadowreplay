use serde::{Deserialize, Serialize};

#[cfg(feature = "headless")]
use tokio::sync::broadcast;

#[derive(Clone, Serialize, Deserialize)]
pub enum Event {
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
        room: i64,
        ts: i64,
        content: String,
    },
}

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
