use std::sync::Arc;
use tokio::sync::RwLock;

use crate::config::Config;
use crate::database::Database;
use crate::recorder::bilibili::client::BiliClient;
use crate::recorder_manager::RecorderManager;
use crate::webhook::poster::WebhookPoster;

#[cfg(feature = "headless")]
use crate::progress::progress_manager::ProgressManager;

#[derive(Clone)]
pub struct State {
    pub db: Arc<Database>,
    pub client: Arc<BiliClient>,
    pub config: Arc<RwLock<Config>>,
    pub webhook_poster: WebhookPoster,
    pub recorder_manager: Arc<RecorderManager>,
    #[cfg(not(feature = "headless"))]
    pub app_handle: tauri::AppHandle,
    #[cfg(feature = "headless")]
    pub progress_manager: Arc<ProgressManager>,
    #[cfg(feature = "headless")]
    pub readonly: bool,
}
