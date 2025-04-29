use async_trait::async_trait;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::sync::LazyLock;
use tauri::AppHandle;
use tauri::Emitter;
use tokio::sync::broadcast;
use tokio::sync::RwLock;

use crate::progress_manager::Event;
use crate::recorder::danmu::DanmuEntry;

type CancelFlagMap = std::collections::HashMap<String, Arc<AtomicBool>>;

static CANCEL_FLAG_MAP: LazyLock<Arc<RwLock<CancelFlagMap>>> =
    LazyLock::new(|| Arc::new(RwLock::new(CancelFlagMap::new())));

#[derive(Clone)]
pub struct ProgressReporter {
    emitter: EventEmitter,
    event_id: String,
    pub cancel: Arc<AtomicBool>,
}

#[async_trait]
pub trait ProgressReporterTrait: Send + Sync + Clone {
    fn update(&self, content: &str);
    async fn finish(&self, success: bool, message: &str);
}

#[derive(Clone)]
pub struct EventEmitter {
    #[cfg(not(feature = "headless"))]
    app_handle: AppHandle,
    #[cfg(feature = "headless")]
    sender: broadcast::Sender<Event>,
}

impl EventEmitter {
    pub fn new(
        #[cfg(not(feature = "headless"))] app_handle: AppHandle,
        #[cfg(feature = "headless")] sender: broadcast::Sender<Event>,
    ) -> Self {
        Self {
            #[cfg(not(feature = "headless"))]
            app_handle,
            #[cfg(feature = "headless")]
            sender,
        }
    }

    pub fn emit(&self, event: Event) {
        #[cfg(not(feature = "headless"))]
        {
            match event {
                Event::ProgressUpdate { id, content } => {
                    self.app_handle.emit("progress_event", event).unwrap();
                }
                Event::ProgressFinished {
                    id,
                    success,
                    message,
                } => {
                    self.app_handle.emit("progress_event", event).unwrap();
                }
                Event::DanmuReceived { room, ts, content } => {
                    self.app_handle
                        .emit(&format!("danmu:{}", room), DanmuEntry { ts, content })
                        .unwrap();
                }
                _ => {}
            }
        }

        #[cfg(feature = "headless")]
        let _ = self.sender.send(event);
    }
}
impl ProgressReporter {
    pub async fn new(emitter: &EventEmitter, event_id: &str) -> Result<Self, String> {
        // if already exists, return
        if CANCEL_FLAG_MAP.read().await.get(event_id).is_some() {
            log::error!("Task already exists: {}", event_id);
            emitter.emit(Event::ProgressFinished {
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
        self.emitter.emit(Event::ProgressUpdate {
            id: self.event_id.clone(),
            content: content.to_string(),
        });
    }

    async fn finish(&self, success: bool, message: &str) {
        self.emitter.emit(Event::ProgressFinished {
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
