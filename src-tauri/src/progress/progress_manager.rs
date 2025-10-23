use recorder::events::RecorderEvent;
use tokio::sync::broadcast;

pub struct ProgressManager {
    pub progress_sender: broadcast::Sender<RecorderEvent>,
    pub progress_receiver: broadcast::Receiver<RecorderEvent>,
}

impl ProgressManager {
    pub fn new() -> Self {
        let (progress_sender, progress_receiver) = broadcast::channel(256);
        Self {
            progress_sender,
            progress_receiver,
        }
    }

    pub fn get_event_sender(&self) -> broadcast::Sender<RecorderEvent> {
        self.progress_sender.clone()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RecorderEvent> {
        self.progress_receiver.resubscribe()
    }
}
