pub mod api;
pub mod errors;
mod extractor;
pub mod url_builder;

use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::{broadcast, RwLock};

use crate::account::Account;
use crate::errors::RecorderError;
use crate::platforms::common::{PlatformApi, RoomPoll, StreamPull};
use crate::platforms::huya::extractor::StreamInfo;
use crate::platforms::PlatformType;
use crate::traits::RecorderTrait;
use crate::Recorder;

pub type HuyaRecorder = Recorder<HuyaExtra>;

#[derive(Clone)]
pub struct HuyaExtra {
    live_stream: Arc<RwLock<Option<StreamInfo>>>,
    /// Stream info of the latest poll, bridging `poll_room` to `poll_stream`.
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

        StreamPull::hls(live_id, &stream.hls_url, Some(self.account.cookies.clone())).await
    }

    async fn clear_stream(&self) {
        *self.extra.live_stream.write().await = None;
        *self.extra.pending.write().await = None;
    }
}

#[async_trait]
impl RecorderTrait for HuyaRecorder {
    async fn run(&self) {
        self.run_recording_loop().await;
    }
}
