use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::config::Config;
use crate::database::Database;
use crate::recorder_manager::RecorderManager;
#[cfg(feature = "gui")]
use crate::static_server::StaticServer;
use crate::task::TaskManager;
use crate::webhook::poster::WebhookPoster;

#[cfg(feature = "headless")]
use crate::progress::progress_manager::ProgressManager;

#[derive(Clone)]
pub struct State {
    pub db: Arc<Database>,
    pub config: Arc<RwLock<Config>>,
    pub webhook_poster: WebhookPoster,
    pub recorder_manager: Arc<RecorderManager>,
    pub task_manager: Arc<TaskManager>,
    pub resource_dir: PathBuf,
    #[cfg(feature = "gui")]
    pub static_server: Arc<StaticServer>,
    #[cfg(not(feature = "headless"))]
    pub app_handle: tauri::AppHandle,
    #[cfg(feature = "headless")]
    pub progress_manager: Arc<ProgressManager>,
    #[cfg(feature = "headless")]
    pub readonly: bool,
}
