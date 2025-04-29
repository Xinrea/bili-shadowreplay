use serde::{Deserialize, Serialize};
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
        room: u64,
        ts: i64,
        content: String,
    },
}

pub struct ProgressManager {
    pub progress_sender: broadcast::Sender<Event>,
    pub progress_receiver: broadcast::Receiver<Event>,
}

impl ProgressManager {
    pub fn new() -> Self {
        let (progress_sender, progress_receiver) = broadcast::channel(16);
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
