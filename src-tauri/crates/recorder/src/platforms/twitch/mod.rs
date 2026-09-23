pub mod api;

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
pub struct TwitchExtra {
    stream_info: Arc<RwLock<Option<api::StreamInfo>>>,
}

pub type TwitchRecorder = Recorder<TwitchExtra>;

impl TwitchRecorder {
    pub fn new(
        room_id: &str,
        account: &Account,
        cache_dir: PathBuf,
        event_channel: broadcast::Sender<crate::events::RecorderEvent>,
        update_interval: Arc<AtomicU64>,
        enabled: bool,
    ) -> Result<Self, RecorderError> {
        let room_id = api::normalize_channel(room_id)?;
        let recorder = Self::with_extra(
            PlatformType::Twitch,
            &room_id,
            account,
            cache_dir,
            event_channel,
            update_interval,
            enabled,
            TwitchExtra {
                stream_info: Arc::new(RwLock::new(None)),
            },
        );

        log::info!("[Twitch][{room_id}] Recorder created");
        Ok(recorder)
    }

    fn log_error(&self, message: &str) {
        log::error!("[Twitch][{}]{}", self.room_id, message);
    }
}

#[async_trait]
impl PlatformApi for TwitchRecorder {
    async fn poll_room(&self) -> Result<RoomPoll, RecorderError> {
        let room_info = api::get_room_info(&self.client, &self.room_id).await?;
        let user = if self.user_info.read().await.user_id != room_info.user_id {
            Some(UserInfo {
                user_id: room_info.user_id,
                user_name: room_info.user_name,
                user_avatar: room_info.user_avatar,
            })
        } else {
            None
        };

        Ok(RoomPoll {
            live: room_info.live,
            room_title: room_info.title,
            room_cover: room_info.cover,
            user,
            platform_live_id: room_info.live_id,
        })
    }

    async fn poll_stream(&self) -> bool {
        match api::get_stream_url(&self.client, &self.room_id).await {
            Ok(stream_info) => {
                self.last_update
                    .store(Utc::now().timestamp(), atomic::Ordering::Relaxed);
                *self.extra.stream_info.write().await = Some(stream_info);
                true
            }
            Err(error) => {
                self.log_error(&format!("Fetch stream failed: {error}"));
                // Keep the cached stream until it expires. A transient GraphQL
                // failure should not turn an active room into an offline one.
                true
            }
        }
    }

    async fn open_pull(&self, live_id: &str) -> Result<StreamPull, RecorderError> {
        let stream_info = self
            .extra
            .stream_info
            .read()
            .await
            .clone()
            .ok_or(RecorderError::NoStreamAvailable)?;

        StreamPull::hls_with_expire(live_id, &stream_info.hls_url, None, stream_info.expires).await
    }

    async fn clear_stream(&self) {
        *self.extra.stream_info.write().await = None;
    }

    fn danmu_config(&self) -> Option<DanmuConfig> {
        Some(DanmuConfig {
            provider: ProviderType::Twitch,
            spawn: DanmuSpawn::LongLived,
        })
    }

    fn resume_on_update_timeout(&self) -> bool {
        true
    }
}

#[async_trait]
impl RecorderTrait for TwitchRecorder {
    async fn run(&self) {
        self.run_recording_loop().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::RecorderBasicTrait;

    #[test]
    fn recorder_normalizes_channel_names() {
        let (tx, _) = broadcast::channel(1);
        let recorder = TwitchRecorder::new(
            "https://www.twitch.tv/Ninja",
            &Account::default(),
            std::env::temp_dir(),
            tx,
            Arc::new(AtomicU64::new(30)),
            true,
        )
        .unwrap();
        assert_eq!(recorder.room_id(), "ninja");
        assert!(recorder.resume_on_update_timeout());
    }
}
