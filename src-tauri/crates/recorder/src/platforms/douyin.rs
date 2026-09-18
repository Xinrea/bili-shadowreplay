pub mod api;
mod response;
pub mod stream_info;

use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use async_trait::async_trait;
use danmu_stream::provider::ProviderType;
use tokio::sync::{broadcast, RwLock};

use crate::account::Account;
use crate::errors::RecorderError;
use crate::platforms::common::{DanmuConfig, DanmuSpawn, PlatformApi, RoomPoll, StreamPull};
use crate::platforms::douyin::stream_info::{
    collect_available_streams, AvailableStream, DouyinStream,
};
use crate::platforms::PlatformType;
use crate::traits::RecorderTrait;
use crate::{Recorder, UserInfo};

pub type DouyinRecorder = Recorder<DouyinExtra>;

#[derive(Clone)]
pub struct DouyinExtra {
    sec_user_id: String,
    live_stream: Arc<RwLock<Option<DouyinStream>>>,
    /// Room info of the latest poll, bridging `poll_room` to `poll_stream`.
    pending: Arc<RwLock<Option<api::DouyinBasicRoomInfo>>>,
}

fn get_best_stream_url(stream: &DouyinStream) -> Option<String> {
    // find the best stream url
    if stream.data.origin.main.hls.is_empty() {
        log::error!("No stream url found in stream_data: {stream:#?}");
        return None;
    }

    Some(stream.data.origin.main.hls.clone())
}

fn log_douyin_stream_choice(room_id: &str, stream_data: &str, hls_pull_url: &str, selected: &str) {
    let mut available = collect_available_streams(stream_data);
    if !hls_pull_url.is_empty() && !available.iter().any(|stream| stream.url == hls_pull_url) {
        available.push(AvailableStream {
            quality: "hls_pull_url".to_string(),
            variant: "default".to_string(),
            format: "hls",
            url: hls_pull_url.to_string(),
        });
    }

    if available.is_empty() {
        log::info!("[{room_id}] Douyin available streams: (none)");
    } else {
        log::info!("[{room_id}] Douyin available streams:");
        for stream in &available {
            log::info!("[{room_id}]   {}: {}", stream.label(), stream.url);
        }
    }

    let selected_label = available
        .iter()
        .find(|stream| stream.url == selected)
        .map(AvailableStream::label)
        .unwrap_or_else(|| "origin/main hls".to_string());
    log::info!("[{room_id}] Douyin selected stream: {selected_label} {selected}");
}

impl DouyinRecorder {
    pub fn new(
        room_id: &str,
        sec_user_id: &str,
        account: &Account,
        cache_dir: PathBuf,
        channel: broadcast::Sender<crate::events::RecorderEvent>,
        update_interval: Arc<AtomicU64>,
        enabled: bool,
    ) -> Result<Self, crate::errors::RecorderError> {
        Ok(Self::with_extra(
            PlatformType::Douyin,
            room_id,
            account,
            cache_dir,
            channel,
            update_interval,
            enabled,
            DouyinExtra {
                sec_user_id: sec_user_id.to_string(),
                live_stream: Arc::new(RwLock::new(None)),
                pending: Arc::new(RwLock::new(None)),
            },
        ))
    }
}

#[async_trait]
impl PlatformApi for DouyinRecorder {
    async fn poll_room(&self) -> Result<RoomPoll, RecorderError> {
        let info = api::get_room_info(
            &self.client,
            &self.account,
            &self.room_id,
            &self.extra.sec_user_id,
        )
        .await?;
        *self.extra.pending.write().await = Some(info.clone());

        Ok(RoomPoll {
            // room_status == 0 表示正在直播
            live: info.status == 0,
            room_title: info.room_title.clone(),
            room_cover: info.cover.clone().unwrap_or_default(),
            user: Some(UserInfo {
                user_id: info.sec_user_id.clone(),
                user_name: info.user_name.clone(),
                user_avatar: info.user_avatar.clone(),
            }),
            // The douyin live id is only known together with the stream.
            platform_live_id: None,
        })
    }

    async fn poll_stream(&self) -> bool {
        let Some(info) = self.extra.pending.write().await.take() else {
            return true;
        };

        // An empty hls url keeps the stream of the previous poll.
        if info.hls_url.is_empty() {
            return true;
        }

        let Ok(stream) = serde_json::from_str::<DouyinStream>(&info.stream_data) else {
            log::error!("Failed to parse stream data: {info:#?}");
            return false;
        };
        let Some(new_stream_url) = get_best_stream_url(&stream) else {
            return false;
        };

        log_douyin_stream_choice(
            &self.room_id,
            &info.stream_data,
            &info.hls_url,
            &new_stream_url,
        );
        *self.extra.live_stream.write().await = Some(stream);
        *self.platform_live_id.write().await = info.room_id_str;

        true
    }

    async fn open_pull(&self, live_id: &str) -> Result<StreamPull, RecorderError> {
        let Some(stream) = self.extra.live_stream.read().await.clone() else {
            return Err(RecorderError::NoStreamAvailable);
        };
        let Some(stream_url) = get_best_stream_url(&stream) else {
            return Err(RecorderError::NoStreamAvailable);
        };

        StreamPull::hls(live_id, &stream_url, None).await
    }

    async fn clear_stream(&self) {
        *self.extra.live_stream.write().await = None;
        *self.extra.pending.write().await = None;
    }

    fn danmu_config(&self) -> Option<DanmuConfig> {
        Some(DanmuConfig {
            provider: ProviderType::Douyin,
            spawn: DanmuSpawn::PerRecording,
        })
    }

    async fn danmu_room_id(&self) -> String {
        // The danmu service subscribes by numeric room id, which is only set
        // together with the stream.
        self.platform_live_id
            .read()
            .await
            .clone()
            .parse::<i64>()
            .unwrap_or(0)
            .to_string()
    }
}

#[async_trait]
impl RecorderTrait for DouyinRecorder {
    async fn run(&self) {
        self.run_recording_loop().await;
    }
}
