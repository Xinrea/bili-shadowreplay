use async_trait::async_trait;
use std::sync::Arc;

use recorder::events::RecorderEvent;

use crate::database::Database;

#[cfg(feature = "gui")]
use {
    recorder::danmu::DanmuEntry,
    serde::Serialize,
    tauri::{AppHandle, Emitter},
};

#[cfg(feature = "headless")]
use tokio::sync::broadcast;

#[derive(Clone)]
pub struct ProgressReporter {
    emitter: EventEmitter,
    pub event_id: String,
    db: Arc<Database>,
}

#[async_trait]
pub trait ProgressReporterTrait: Send + Sync + Clone {
    async fn update(&self, content: &str);
    async fn finish(&self, success: bool, message: &str);
}

#[derive(Clone)]
pub struct EventEmitter {
    #[cfg(feature = "gui")]
    app_handle: AppHandle,
    #[cfg(feature = "headless")]
    sender: broadcast::Sender<RecorderEvent>,
}

#[cfg(feature = "gui")]
#[derive(Clone, Serialize)]
struct UpdateEvent<'a> {
    id: &'a str,
    content: &'a str,
}

#[cfg(feature = "gui")]
#[derive(Clone, Serialize)]
struct FinishEvent<'a> {
    id: &'a str,
    success: bool,
    message: &'a str,
}

impl EventEmitter {
    pub fn new(
        #[cfg(feature = "gui")] app_handle: AppHandle,
        #[cfg(feature = "headless")] sender: broadcast::Sender<RecorderEvent>,
    ) -> Self {
        Self {
            #[cfg(feature = "gui")]
            app_handle,
            #[cfg(feature = "headless")]
            sender,
        }
    }

    pub fn emit(&self, event: &RecorderEvent) {
        #[cfg(feature = "gui")]
        {
            match event {
                RecorderEvent::ProgressUpdate { id, content } => {
                    self.app_handle
                        .emit(
                            &format!("progress-update:{}", id),
                            UpdateEvent { id, content },
                        )
                        .unwrap();
                }
                RecorderEvent::ProgressFinished {
                    id,
                    success,
                    message,
                } => {
                    self.app_handle
                        .emit(
                            &format!("progress-finished:{}", id),
                            FinishEvent {
                                id,
                                success: *success,
                                message,
                            },
                        )
                        .unwrap();
                }
                RecorderEvent::DanmuReceived { room, ts, content } => {
                    self.app_handle
                        .emit(
                            &format!("danmu:{room}"),
                            DanmuEntry {
                                ts: *ts,
                                content: content.clone(),
                            },
                        )
                        .unwrap();
                }
                _ => {}
            }
        }

        #[cfg(feature = "headless")]
        let _ = self.sender.send(event.clone());
    }
}
impl ProgressReporter {
    pub async fn new(
        db: Arc<Database>,
        emitter: &EventEmitter,
        event_id: &str,
    ) -> Result<Self, String> {
        Ok(Self {
            db,
            emitter: emitter.clone(),
            event_id: event_id.to_string(),
        })
    }
}

#[async_trait]
impl ProgressReporterTrait for ProgressReporter {
    async fn update(&self, content: &str) {
        self.emitter.emit(&RecorderEvent::ProgressUpdate {
            id: self.event_id.clone(),
            content: content.to_string(),
        });
        let _ = self
            .db
            .update_task(&self.event_id, "processing", content, None)
            .await;
    }

    async fn finish(&self, success: bool, message: &str) {
        self.emitter.emit(&RecorderEvent::ProgressFinished {
            id: self.event_id.clone(),
            success,
            message: message.to_string(),
        });
    }
}
