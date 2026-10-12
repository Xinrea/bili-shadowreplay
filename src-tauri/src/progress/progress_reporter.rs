use async_trait::async_trait;
use std::sync::Arc;

use crate::events::{AppEvent, EventBus};

use crate::database::Database;

#[derive(Clone)]
pub struct ProgressReporter {
    events: EventBus,
    pub event_id: String,
    db: Arc<Database>,
}

// async_trait generates #[must_use] on methods returning an already must-use Future.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait ProgressReporterTrait: Send + Sync + Clone {
    async fn update(&self, content: &str);
    async fn finish(&self, success: bool, message: &str);
}

impl ProgressReporter {
    pub async fn new(db: Arc<Database>, events: &EventBus, event_id: &str) -> Result<Self, String> {
        Ok(Self {
            db,
            events: events.clone(),
            event_id: event_id.to_string(),
        })
    }
}

#[async_trait]
impl ProgressReporterTrait for ProgressReporter {
    async fn update(&self, content: &str) {
        self.events.publish(AppEvent::ProgressUpdate {
            id: self.event_id.clone(),
            content: content.to_string(),
        });
        let _ = self
            .db
            .update_task(&self.event_id, "processing", content, None)
            .await;
    }

    async fn finish(&self, success: bool, message: &str) {
        self.events.publish(AppEvent::ProgressFinished {
            id: self.event_id.clone(),
            success,
            message: message.to_string(),
        });
    }
}
