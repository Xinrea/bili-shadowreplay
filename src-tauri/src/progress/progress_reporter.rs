use async_trait::async_trait;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::sync::LazyLock;
use tokio::sync::RwLock;

use crate::recorder_manager::RecorderEvent;

#[cfg(feature = "gui")]
use {
    crate::recorder::danmu::DanmuEntry,
    serde::Serialize,
    tauri::{AppHandle, Emitter},
};

#[cfg(feature = "headless")]
use tokio::sync::broadcast;

type CancelFlagMap = std::collections::HashMap<String, Arc<AtomicBool>>;

static CANCEL_FLAG_MAP: LazyLock<Arc<RwLock<CancelFlagMap>>> =
    LazyLock::new(|| Arc::new(RwLock::new(CancelFlagMap::new())));

#[derive(Clone)]
pub struct ProgressReporter {
    emitter: EventEmitter,
    pub event_id: String,
    pub cancel: Arc<AtomicBool>,
}

#[async_trait]
pub trait ProgressReporterTrait: Send + Sync + Clone {
    fn update(&self, content: &str);
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
    pub async fn new(emitter: &EventEmitter, event_id: &str) -> Result<Self, String> {
        // if already exists, return
        if CANCEL_FLAG_MAP.read().await.get(event_id).is_some() {
            log::error!("Task already exists: {event_id}");
            emitter.emit(&RecorderEvent::ProgressFinished {
                id: event_id.to_string(),
                success: false,
                message: "任务已经存在".to_string(),
            });
            return Err("任务已经存在".to_string());
        }

        let cancel = Arc::new(AtomicBool::new(false));
        CANCEL_FLAG_MAP
            .write()
            .await
            .insert(event_id.to_string(), cancel.clone());

        Ok(Self {
            emitter: emitter.clone(),
            event_id: event_id.to_string(),
            cancel,
        })
    }
}

#[async_trait]
impl ProgressReporterTrait for ProgressReporter {
    fn update(&self, content: &str) {
        self.emitter.emit(&RecorderEvent::ProgressUpdate {
            id: self.event_id.clone(),
            content: content.to_string(),
        });
    }

    async fn finish(&self, success: bool, message: &str) {
        self.emitter.emit(&RecorderEvent::ProgressFinished {
            id: self.event_id.clone(),
            success,
            message: message.to_string(),
        });
        CANCEL_FLAG_MAP.write().await.remove(&self.event_id);
    }
}

pub async fn cancel_progress(event_id: &str) {
    let mut cancel_flag_map = CANCEL_FLAG_MAP.write().await;
    if let Some(cancel_flag) = cancel_flag_map.get_mut(event_id) {
        cancel_flag.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}
