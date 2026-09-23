pub mod api;
pub mod response;

use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::{atomic, Arc};

use async_trait::async_trait;
use chrono::Utc;
use tokio::sync::{broadcast, RwLock};

use crate::account::Account;
use crate::errors::RecorderError;
use crate::platforms::common::{PlatformApi, RoomPoll, StreamPull};
use crate::platforms::PlatformType;
use crate::traits::RecorderTrait;
use crate::{Recorder, UserInfo};

#[derive(Clone)]
pub struct TikTokExtra {
    stream_info: Arc<RwLock<Option<api::StreamInfo>>>,
}

pub type TikTokRecorder = Recorder<TikTokExtra>;

impl TikTokRecorder {
    pub fn new(
        room_id: &str,
        account: &Account,
        cache_dir: PathBuf,
        event_channel: broadcast::Sender<crate::events::RecorderEvent>,
        update_interval: Arc<AtomicU64>,
        enabled: bool,
    ) -> Result<Self, RecorderError> {
        let recorder = Self::with_extra(
            PlatformType::TikTok,
            room_id,
            account,
            cache_dir,
            event_channel,
            update_interval,
            enabled,
            TikTokExtra {
                stream_info: Arc::new(RwLock::new(None)),
            },
        );

        log::info!("[TikTok][{room_id}]Recorder created");

        Ok(recorder)
    }

    fn log_info(&self, message: &str) {
        log::info!("[TikTok][{}]{}", self.room_id, message);
    }

    fn log_error(&self, message: &str) {
        log::error!("[TikTok][{}]{}", self.room_id, message);
    }
}

#[async_trait]
impl PlatformApi for TikTokRecorder {
    async fn poll_room(&self) -> Result<RoomPoll, RecorderError> {
        log::info!("[TikTok][{}] Check status", self.room_id);
        let room_info = api::get_room_info(&self.client, &self.account, &self.room_id).await?;

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
            // TikTok has no persistent live id; keep a fresh timestamp.
            platform_live_id: Some(Utc::now().timestamp().to_string()),
        })
    }

    async fn poll_stream(&self) -> bool {
        match api::get_stream_url(&self.client, &self.account, &self.room_id).await {
            Ok(stream_info) => {
                let pre_stream = self.extra.stream_info.read().await.clone();
                *self.extra.stream_info.write().await = Some(stream_info.clone());
                self.last_update
                    .store(Utc::now().timestamp(), atomic::Ordering::Relaxed);

                self.log_info(&format!(
                    "Update to new stream: {:?} => {:?}",
                    pre_stream, stream_info
                ));

                true
            }
            Err(e) => {
                self.log_error(&format!("Fetch stream failed: {}", e));
                true
            }
        }
    }

    async fn open_pull(&self, _live_id: &str) -> Result<StreamPull, RecorderError> {
        let Some(stream_info) = self.extra.stream_info.read().await.clone() else {
            return Err(RecorderError::NoStreamAvailable);
        };
        let Some(rtmp_url) = stream_info.rtmp_url else {
            self.log_error("No HLS stream available");
            return Err(RecorderError::NoStreamAvailable);
        };

        // RTMP is not HTTP: no user agent or headers to align with a probe.
        Ok(StreamPull::Flv {
            url: rtmp_url,
            user_agent: None,
            http_headers: Vec::new(),
        })
    }

    async fn clear_stream(&self) {
        *self.extra.stream_info.write().await = None;
    }
}

#[async_trait]
impl RecorderTrait for TikTokRecorder {
    async fn run(&self) {
        self.run_recording_loop().await;
    }
}
