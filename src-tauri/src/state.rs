use custom_error::custom_error;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::config::Config;
use crate::database::Database;
use crate::recorder::bilibili::client::BiliClient;
use crate::recorder_manager::RecorderManager;

#[cfg(feature = "headless")]
use crate::progress_manager::ProgressManager;

custom_error! {
    StateError
    RecorderAlreadyExists = "Recorder already exists",
    RecorderCreateError = "Recorder create error",
}

#[derive(Clone)]
pub struct State {
    pub db: Arc<Database>,
    pub client: Arc<BiliClient>,
    pub config: Arc<RwLock<Config>>,
    pub recorder_manager: Arc<RecorderManager>,
    #[cfg(not(feature = "headless"))]
    pub app_handle: tauri::AppHandle,
    #[cfg(feature = "headless")]
    pub progress_manager: Arc<ProgressManager>,
}
