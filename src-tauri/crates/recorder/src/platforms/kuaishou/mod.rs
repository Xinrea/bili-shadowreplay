pub mod api;
pub mod response;

use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::{atomic, Arc};

use async_trait::async_trait;
use chrono::Utc;
use danmu_stream::provider::ProviderType;
use tokio::sync::{broadcast, RwLock};

use crate::account::Account;
use crate::errors::RecorderError;
use crate::platforms::common::{DanmuConfig, DanmuSpawn, PlatformApi, RoomPoll, StreamPull};
use crate::platforms::PlatformType;
use crate::traits::RecorderTrait;
use crate::{Recorder, UserInfo};

#[derive(Clone)]
pub struct KuaishouExtra {
    stream_url: Arc<RwLock<Option<String>>>,
}

pub type KuaishouRecorder = Recorder<KuaishouExtra>;

impl KuaishouRecorder {
    pub fn new(
        room_id: &str,
        account: &Account,
        cache_dir: PathBuf,
        event_channel: broadcast::Sender<crate::events::RecorderEvent>,
        update_interval: Arc<AtomicU64>,
        enabled: bool,
    ) -> Result<Self, RecorderError> {
        let recorder = Self::with_extra(
            PlatformType::Kuaishou,
            room_id,
            account,
            cache_dir,
            event_channel,
            update_interval,
            enabled,
            KuaishouExtra {
                stream_url: Arc::new(RwLock::new(None)),
            },
        );

        log::info!("[Kuaishou][{room_id}]Recorder created");

        Ok(recorder)
    }

    fn log_info(&self, message: &str) {
        log::info!("[Kuaishou][{}]{}", self.room_id, message);
    }

    fn log_error(&self, message: &str) {
        log::error!("[Kuaishou][{}]{}", self.room_id, message);
    }

    /// The room id is either a full page url or a plain room id.
    fn room_url(&self) -> String {
        if self.room_id.starts_with("http") {
            self.room_id.clone()
        } else {
            format!("https://live.kuaishou.com/u/{}", self.room_id)
        }
    }
}

#[async_trait]
impl PlatformApi for KuaishouRecorder {
    async fn poll_room(&self) -> Result<RoomPoll, RecorderError> {
        let url = self.room_url();
        let room_info = api::get_room_info(&self.client, &self.account, &url).await?;

        // Update user info
        let user = if self.user_info.read().await.user_id != room_info.user_id {
            Some(UserInfo {
                user_id: room_info.user_id.to_string(),
                user_name: room_info.user_name.clone(),
                user_avatar: room_info.user_avatar.clone(),
            })
        } else {
            None
        };

        Ok(RoomPoll {
            live: room_info.live_status,
            room_title: room_info.room_title,
            room_cover: room_info.room_cover_url,
            user,
            // Kuaishou has no persistent live id; keep a fresh timestamp.
            platform_live_id: Some(Utc::now().timestamp().to_string()),
        })
    }

    async fn poll_stream(&self) -> bool {
        let url = self.room_url();
        match api::get_stream_urls(&self.client, &self.account, &url).await {
            Ok(streams) => {
                let Some(selected) = streams
                    .iter()
                    .find(|stream| stream.url.contains(".m3u8"))
                    .map(|stream| stream.url.clone())
                else {
                    self.log_error("No stream URLs found");
                    return false;
                };

                let pre_stream = self.extra.stream_url.read().await.clone();
                *self.extra.stream_url.write().await = Some(selected.clone());
                self.last_update
                    .store(Utc::now().timestamp(), atomic::Ordering::Relaxed);

                self.log_info(&format!(
                    "Update to new stream: {:?} => {}",
                    pre_stream, selected
                ));

                true
            }
            Err(e) => {
                self.log_error(&format!("Fetch stream failed: {}", e));
                true
            }
        }
    }

    async fn open_pull(&self, live_id: &str) -> Result<StreamPull, RecorderError> {
        let Some(stream_url) = self.extra.stream_url.read().await.clone() else {
            return Err(RecorderError::NoStreamAvailable);
        };

        // Kuaishou stream URLs are direct m3u8 URLs
        StreamPull::hls(live_id, &stream_url, Some(self.account.cookies.clone())).await
    }

    async fn clear_stream(&self) {
        *self.extra.stream_url.write().await = None;
    }

    fn danmu_config(&self) -> Option<DanmuConfig> {
        Some(DanmuConfig {
            provider: ProviderType::Kuaishou,
            spawn: DanmuSpawn::PerRecording,
        })
    }

    fn is_throttled(&self, error: &RecorderError) -> bool {
        api::is_rate_limited_error(error)
    }
}

#[async_trait]
impl RecorderTrait for KuaishouRecorder {
    async fn run(&self) {
        self.run_recording_loop().await;
    }
}
