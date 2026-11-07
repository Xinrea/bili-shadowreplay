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

use crate::events::EventBus;

#[derive(Clone)]
pub struct State {
    pub mcp_server: Arc<crate::mcp::McpServer>,
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
    pub events: EventBus,
    #[cfg(feature = "headless")]
    pub readonly: bool,
}
