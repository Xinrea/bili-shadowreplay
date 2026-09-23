pub mod api;
pub mod response;

use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use async_trait::async_trait;
use danmu_stream::provider::ProviderType;
use tokio::sync::{broadcast, RwLock};

use crate::account::Account;
use crate::errors::RecorderError;
use crate::platforms::common::{DanmuConfig, DanmuSpawn, PlatformApi, RoomPoll, StreamPull};
use crate::platforms::PlatformType;
use crate::traits::RecorderTrait;
use crate::{Recorder, UserInfo};

pub type DouyuRecorder = Recorder<DouyuExtra>;

#[derive(Clone)]
pub struct DouyuExtra {
    stream_url: Arc<RwLock<Option<String>>>,
    encryption_key: api::EncryptionCache,
    resolved_room_id: Arc<RwLock<Option<u64>>>,
}

impl DouyuRecorder {
    async fn room_numeric_id(&self) -> Result<u64, RecorderError> {
        if let Some(room_id) = *self.extra.resolved_room_id.read().await {
            return Ok(room_id);
        }

        let room_id = api::resolve_room_id(&self.client, &self.account, &self.room_id)
            .await
            .map_err(api::DouyuApiError::into_recorder_error)?;
        *self.extra.resolved_room_id.write().await = Some(room_id);
        Ok(room_id)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        room_id: &str,
        account: &Account,
        cache_dir: PathBuf,
        channel: broadcast::Sender<crate::events::RecorderEvent>,
        update_interval: Arc<AtomicU64>,
        enabled: bool,
    ) -> Result<Self, RecorderError> {
        Ok(Self::with_extra(
            PlatformType::Douyu,
            room_id,
            account,
            cache_dir,
            channel,
            update_interval,
            enabled,
            DouyuExtra {
                stream_url: Arc::new(RwLock::new(None)),
                encryption_key: Arc::new(RwLock::new(None)),
                resolved_room_id: Arc::new(RwLock::new(None)),
            },
        ))
    }
}

#[async_trait]
impl PlatformApi for DouyuRecorder {
    async fn poll_room(&self) -> Result<RoomPoll, RecorderError> {
        let room_id = self.room_numeric_id().await?;
        let room = match api::get_room_info(&self.client, &self.account, room_id).await {
            Ok(room) => room,
            Err(api::DouyuApiError::Offline { detail }) => {
                log::info!(
                    "[Douyu][{}] RoomApi reports offline: {detail}",
                    self.room_id
                );
                let previous = self.room_info.read().await.clone();
                return Ok(RoomPoll {
                    live: false,
                    room_title: previous.room_title,
                    room_cover: previous.room_cover,
                    user: None,
                    platform_live_id: None,
                });
            }
            Err(error) => return Err(error.into_recorder_error()),
        };
        let live = response::room_is_live(&room);
        let platform_live_id = live.then(|| response::platform_live_id(&room, room_id));

        Ok(RoomPoll {
            live,
            room_title: room.room_name,
            room_cover: room.room_thumb,
            user: Some(UserInfo {
                // RoomApi does not always include the owner's uid. Keep it
                // empty rather than misreporting the room id as a user id.
                user_id: room.owner_uid,
                user_name: room.owner_name,
                user_avatar: room.avatar,
            }),
            platform_live_id,
        })
    }

    async fn poll_stream(&self) -> bool {
        let room_id = match self.room_numeric_id().await {
            Ok(room_id) => room_id,
            Err(error) => {
                log::error!("[Douyu][{}] Invalid room id: {error}", self.room_id);
                *self.extra.stream_url.write().await = None;
                return false;
            }
        };

        match api::get_stream_url(
            &self.client,
            &self.account,
            room_id,
            &self.extra.encryption_key,
        )
        .await
        {
            Ok(url) => {
                *self.extra.stream_url.write().await = Some(url);
                true
            }
            Err(error) => {
                *self.extra.stream_url.write().await = None;
                match &error {
                    api::DouyuApiError::Offline { detail } => {
                        log::info!("[Douyu][{}] Stream went offline: {detail}", self.room_id);
                    }
                    api::DouyuApiError::Authentication { detail } => {
                        log::warn!(
                            "[Douyu][{}] Stream authentication failed: {detail}",
                            self.room_id
                        );
                    }
                    _ => {
                        log::warn!("[Douyu][{}] Fetch stream failed: {error}", self.room_id);
                    }
                }
                false
            }
        }
    }

    async fn open_pull(&self, _live_id: &str) -> Result<StreamPull, RecorderError> {
        let url = self
            .extra
            .stream_url
            .read()
            .await
            .clone()
            .ok_or(RecorderError::NoStreamAvailable)?;
        let room_id = self.room_numeric_id().await?;
        let (user_agent, http_headers) = api::pull_http_identity(&self.account, room_id);

        Ok(StreamPull::Flv {
            url,
            user_agent: Some(user_agent),
            http_headers,
        })
    }

    async fn clear_stream(&self) {
        // Keep the encryption key across recording attempts while it is valid;
        // only the temporary FLV URL belongs to one stream session.
        *self.extra.stream_url.write().await = None;
    }

    fn danmu_config(&self) -> Option<DanmuConfig> {
        Some(DanmuConfig {
            provider: ProviderType::Douyu,
            spawn: DanmuSpawn::PerRecording,
        })
    }

    async fn danmu_room_id(&self) -> String {
        self.room_numeric_id()
            .await
            .map(|room_id| room_id.to_string())
            .unwrap_or_else(|_| self.room_id.clone())
    }
}

#[async_trait]
impl RecorderTrait for DouyuRecorder {
    async fn run(&self) {
        self.run_recording_loop().await;
    }
}
