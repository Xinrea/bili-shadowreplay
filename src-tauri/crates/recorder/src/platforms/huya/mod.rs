pub mod api;
pub mod errors;
mod extractor;
pub mod url_builder;

use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use async_trait::async_trait;
use danmu_stream::provider::ProviderType;
use tokio::sync::{broadcast, RwLock};

use crate::account::Account;
use crate::errors::RecorderError;
use crate::platforms::common::{DanmuConfig, DanmuSpawn, PlatformApi, RoomPoll, StreamPull};
use crate::platforms::huya::extractor::{PullUrl, StreamInfo};
use crate::platforms::PlatformType;
use crate::traits::RecorderTrait;
use crate::Recorder;

pub type HuyaRecorder = Recorder<HuyaExtra>;

#[derive(Clone)]
pub struct HuyaExtra {
    live_stream: Arc<RwLock<Option<StreamInfo>>>,
    /// Stream info of the latest poll, bridging `poll_room` to `poll_stream`.
    ///
    /// Per-poll state, not session state: every successful `poll_room`
    /// overwrites it and `poll_stream` is its only consumer, so it is
    /// deliberately kept out of `clear_stream` — clearing it there would wipe
    /// the just-polled stream on the poll that first sees the live.
    pending: Arc<RwLock<Option<StreamInfo>>>,
}

impl HuyaRecorder {
    pub fn new(
        room_id: &str,
        account: &Account,
        cache_dir: PathBuf,
        channel: broadcast::Sender<crate::events::RecorderEvent>,
        update_interval: Arc<AtomicU64>,
        enabled: bool,
    ) -> Result<Self, RecorderError> {
        Ok(Self::with_extra(
            PlatformType::Huya,
            room_id,
            account,
            cache_dir,
            channel,
            update_interval,
            enabled,
            HuyaExtra {
                live_stream: Arc::new(RwLock::new(None)),
                pending: Arc::new(RwLock::new(None)),
            },
        ))
    }
}

#[async_trait]
impl PlatformApi for HuyaRecorder {
    async fn poll_room(&self) -> Result<RoomPoll, RecorderError> {
        let (user_info, room_info, stream_info) =
            api::get_room_info(&self.client, &self.account, &self.room_id)
                .await
                .map_err(|error| RecorderError::ApiError {
                    error: error.to_string(),
                })?;
        *self.extra.pending.write().await = Some(stream_info);

        Ok(RoomPoll {
            live: room_info.status,
            room_title: room_info.room_title,
            room_cover: room_info.room_cover,
            user: Some(user_info),
            // The huya live id comes with the stream info.
            platform_live_id: None,
        })
    }

    async fn poll_stream(&self) -> bool {
        if let Some(stream) = self.extra.pending.write().await.take() {
            *self.platform_live_id.write().await = stream.id();
            *self.extra.live_stream.write().await = Some(stream);
        }

        true
    }

    async fn open_pull(&self, live_id: &str) -> Result<StreamPull, RecorderError> {
        let Some(stream) = self.extra.live_stream.read().await.clone() else {
            return Err(RecorderError::NoStreamAvailable);
        };

        let pull_url = api::pick_pull_url(&self.client, &stream)
            .await
            .map_err(|error| RecorderError::ApiError {
                error: error.to_string(),
            })?;
        match pull_url {
            PullUrl::Flv(url) => Ok(StreamPull::Flv { url }),
            PullUrl::Hls(url) => {
                StreamPull::hls(live_id, &url, Some(self.account.cookies.clone())).await
            }
        }
    }

    async fn clear_stream(&self) {
        // `pending` carries the fresh poll into `poll_stream` and must survive
        // the live-transition reset.
        *self.extra.live_stream.write().await = None;
    }

    fn danmu_config(&self) -> Option<DanmuConfig> {
        Some(DanmuConfig {
            provider: ProviderType::Huya,
            spawn: DanmuSpawn::PerRecording,
        })
    }
}

#[async_trait]
impl RecorderTrait for HuyaRecorder {
    async fn run(&self) {
        self.run_recording_loop().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    #[tokio::test]
    async fn clear_stream_keeps_the_fresh_polls_bridge() {
        let (tx, _rx) = broadcast::channel(1);
        let recorder = HuyaRecorder::new(
            "room",
            &Account::default(),
            std::env::temp_dir().join(format!("bsr-huya-{}", uuid::Uuid::new_v4())),
            tx,
            Arc::new(AtomicU64::new(30)),
            true,
        )
        .unwrap();
        *recorder.extra.pending.write().await = Some(StreamInfo {
            candidates: vec![PullUrl::Hls("https://example.com/live.m3u8".to_string())],
        });

        // The live-transition reset runs between `poll_room` and
        // `poll_stream`: it must clear the session stream but keep the fresh
        // poll's bridge, or the first recording attempt finds no stream.
        recorder.clear_stream().await;

        assert!(recorder.extra.pending.read().await.is_some());
        assert!(recorder.extra.live_stream.read().await.is_none());
    }
}
