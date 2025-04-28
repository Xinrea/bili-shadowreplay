use async_trait::async_trait;
use serde::Serialize;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::sync::LazyLock;
use tauri::{AppHandle, Emitter};
use tokio::sync::RwLock;

type CancelFlagMap = std::collections::HashMap<String, Arc<AtomicBool>>;

static CANCEL_FLAG_MAP: LazyLock<Arc<RwLock<CancelFlagMap>>> =
    LazyLock::new(|| Arc::new(RwLock::new(CancelFlagMap::new())));

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressUpdate<'a> {
    pub id: &'a str,
    pub content: &'a str,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressFinished<'a> {
    pub id: &'a str,
    pub success: bool,
    pub message: &'a str,
}

#[derive(Clone)]
pub struct ProgressReporter {
    #[cfg(not(feature = "headless"))]
    app_handle: AppHandle,
    event_id: String,
    pub cancel: Arc<AtomicBool>,
}

#[async_trait]
pub trait ProgressReporterTrait: Send + Sync + Clone {
    fn update(&self, content: &str);
    async fn finish(&self, success: bool, message: &str);
}

impl ProgressReporter {
    pub async fn new(
        #[cfg(not(feature = "headless"))] app_handle: AppHandle,
        event_id: &str,
    ) -> Result<Self, String> {
        // if already exists, return
        if CANCEL_FLAG_MAP.read().await.get(event_id).is_some() {
            log::error!("Task already exists: {}", event_id);
            #[cfg(not(feature = "headless"))]
            let _ = app_handle.emit(
                "progress-finished",
                ProgressFinished {
                    id: event_id,
                    success: false,
                    message: "任务已经存在",
                },
            );
            return Err("任务已经存在".to_string());
        }

        let cancel = Arc::new(AtomicBool::new(false));
        CANCEL_FLAG_MAP
            .write()
            .await
            .insert(event_id.to_string(), cancel.clone());

        Ok(Self {
            #[cfg(not(feature = "headless"))]
            app_handle,
            event_id: event_id.to_string(),
            cancel,
        })
    }
}

#[async_trait]
impl ProgressReporterTrait for ProgressReporter {
    fn update(&self, content: &str) {
        #[cfg(not(feature = "headless"))]
        if let Err(e) = self.app_handle.emit(
            "progress-update",
            ProgressUpdate {
                id: &self.event_id,
                content,
            },
        ) {
            log::error!("Failed to emit progress update: {}", e);
        }
    }

    async fn finish(&self, success: bool, message: &str) {
        #[cfg(not(feature = "headless"))]
        if let Err(e) = self.app_handle.emit(
            "progress-finished",
            ProgressFinished {
                id: &self.event_id,
                success,
                message,
            },
        ) {
            log::error!("Failed to emit progress finished: {}", e);
        }
        CANCEL_FLAG_MAP.write().await.remove(&self.event_id);
    }
}

pub async fn cancel_progress(event_id: &str) {
    CANCEL_FLAG_MAP
        .write()
        .await
        .get_mut(event_id)
        .unwrap()
        .store(true, std::sync::atomic::Ordering::Relaxed);
}
