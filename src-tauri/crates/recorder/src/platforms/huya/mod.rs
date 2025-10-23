pub mod api;
pub mod errors;
mod extractor;
pub mod url_builder;
use crate::account::Account;
use crate::core::hls_recorder::{construct_stream_from_variant, HlsRecorder};
use crate::core::{Codec, Format};
use crate::errors::RecorderError;
use crate::events::RecorderEvent;
use crate::platforms::huya::extractor::StreamInfo;
use crate::traits::RecorderTrait;
use crate::{Recorder, RoomInfo, UserInfo};
use async_trait::async_trait;
use chrono::Utc;
use rand::random;
use std::path::PathBuf;
use std::sync::{atomic, Arc};
use std::time::Duration;
use tokio::sync::{broadcast, Mutex, RwLock};

use crate::danmu::DanmuStorage;
use crate::platforms::PlatformType;

pub type HuyaRecorder = Recorder<HuyaExtra>;

#[derive(Clone)]
pub struct HuyaExtra {
    live_stream: Arc<RwLock<Option<StreamInfo>>>,
}

impl HuyaRecorder {
    pub async fn new(
        room_id: i64,
        account: &Account,
        cache_dir: PathBuf,
        channel: broadcast::Sender<RecorderEvent>,
        enabled: bool,
    ) -> Result<Self, crate::errors::RecorderError> {
        Ok(Self {
            platform: PlatformType::Huya,
            room_id,
            account: account.clone(),
            client: reqwest::Client::new(),
            event_channel: channel,
            cache_dir,
            quit: Arc::new(atomic::AtomicBool::new(false)),
            enabled: Arc::new(atomic::AtomicBool::new(enabled)),
            is_recording: Arc::new(atomic::AtomicBool::new(false)),
            room_info: Arc::new(RwLock::new(RoomInfo::default())),
            user_info: Arc::new(RwLock::new(UserInfo::default())),
            platform_live_id: Arc::new(RwLock::new(String::new())),
            live_id: Arc::new(RwLock::new(String::new())),
            danmu_storage: Arc::new(RwLock::new(None)),
            last_update: Arc::new(atomic::AtomicI64::new(Utc::now().timestamp())),
            last_sequence: Arc::new(atomic::AtomicU64::new(0)),
            danmu_task: Arc::new(Mutex::new(None)),
            record_task: Arc::new(Mutex::new(None)),
            total_duration: Arc::new(atomic::AtomicU64::new(0)),
            total_size: Arc::new(atomic::AtomicU64::new(0)),
            extra: HuyaExtra {
                live_stream: Arc::new(RwLock::new(None)),
            },
        })
    }

    async fn check_status(&self) -> bool {
        let pre_live_status = self.room_info.read().await.status;
        match api::get_room_info(&self.client, &self.account, self.room_id).await {
            Ok((user_info, room_info, stream_info)) => {
                let live_status = room_info.status;

                *self.room_info.write().await = room_info;

                *self.user_info.write().await = user_info;

                if pre_live_status != live_status {
                    // live status changed, reset current record flag
                    log::info!(
                        "[{}]Live status changed to {}, auto_start: {}",
                        self.room_id,
                        live_status,
                        self.enabled.load(atomic::Ordering::Relaxed)
                    );

                    if live_status {
                        let _ = self.event_channel.send(RecorderEvent::LiveStart {
                            recorder: self.info().await,
                        });
                    } else {
                        let _ = self.event_channel.send(RecorderEvent::LiveEnd {
                            platform: PlatformType::Douyin,
                            room_id: self.room_id,
                            recorder: self.info().await,
                        });
                    }

                    self.reset().await;
                }

                if !live_status {
                    self.reset().await;

                    return false;
                }

                let should_record = self.should_record().await;

                if !should_record {
                    return true;
                }

                *self.extra.live_stream.write().await = Some(stream_info.clone());
                let platform_live_id = stream_info.id();
                *self.platform_live_id.write().await = platform_live_id;

                true
            }
            Err(e) => {
                log::warn!("[{}]Update room status failed: {}", self.room_id, e);
                pre_live_status
            }
        }
    }

    async fn reset(&self) {
        *self.platform_live_id.write().await = String::new();
        self.last_update
            .store(Utc::now().timestamp(), atomic::Ordering::Relaxed);
        self.last_sequence.store(0, atomic::Ordering::Relaxed);
        self.total_duration.store(0, atomic::Ordering::Relaxed);
        self.total_size.store(0, atomic::Ordering::Relaxed);
        *self.extra.live_stream.write().await = None;
    }

    async fn update_entries(&self, live_id: &str) -> Result<(), RecorderError> {
        // Get current room info and stream URL
        let room_info = self.room_info.read().await.clone();
        let Some(stream) = self.extra.live_stream.read().await.clone() else {
            return Err(RecorderError::NoStreamAvailable);
        };

        let work_dir = self.work_dir(live_id).await;
        let _ = tokio::fs::create_dir_all(&work_dir.full_path()).await;

        // download cover
        let cover_url = room_info.room_cover.clone();
        let cover_path = work_dir.with_filename("cover.jpg");
        let _ = api::download_file(&self.client, &cover_url, &cover_path.full_path()).await;

        *self.live_id.write().await = live_id.to_string();

        // Setup danmu store
        let danmu_file_path = work_dir.with_filename("danmu.txt");
        let danmu_storage = DanmuStorage::new(&danmu_file_path.full_path()).await;
        *self.danmu_storage.write().await = danmu_storage;

        // Start danmu task
        if let Some(danmu_task) = self.danmu_task.lock().await.as_mut() {
            danmu_task.abort();
        }
        if let Some(danmu_stream_task) = self.danmu_task.lock().await.as_mut() {
            danmu_stream_task.abort();
        }

        let _ = self.event_channel.send(RecorderEvent::RecordStart {
            recorder: self.info().await,
        });

        log::debug!("[{}]Stream URL: {}", self.room_id, stream.hls_url);

        let hls_stream =
            construct_stream_from_variant(live_id, &stream.hls_url, Format::TS, Codec::Avc)
                .await
                .map_err(|_| RecorderError::NoStreamAvailable)?;
        let hls_recorder = HlsRecorder::new(
            self.room_id.to_string(),
            Arc::new(hls_stream),
            self.client.clone(),
            Some(self.account.cookies.clone()),
            self.event_channel.clone(),
            work_dir.full_path(),
            self.enabled.clone(),
        )
        .await;

        if let Err(e) = hls_recorder.start().await {
            log::error!("[{}]Failed to start hls recorder: {}", self.room_id, e);
            return Err(e);
        }

        Ok(())
    }
}

#[async_trait]
impl crate::traits::RecorderTrait<HuyaExtra> for HuyaRecorder {
    async fn run(&self) {
        let self_clone = self.clone();
        *self.record_task.lock().await = Some(tokio::spawn(async move {
            while !self_clone.quit.load(atomic::Ordering::Relaxed) {
                if self_clone.check_status().await {
                    // Live status is ok, start recording
                    if self_clone.should_record().await {
                        self_clone
                            .is_recording
                            .store(true, atomic::Ordering::Relaxed);
                        let live_id = Utc::now().timestamp_millis().to_string();
                        if let Err(e) = self_clone.update_entries(&live_id).await {
                            log::error!("[{}]Update entries error: {}", self_clone.room_id, e);
                        }
                    }
                    if self_clone.is_recording.load(atomic::Ordering::Relaxed) {
                        let _ = self_clone.event_channel.send(RecorderEvent::RecordEnd {
                            recorder: self_clone.info().await,
                        });
                    }
                    self_clone
                        .is_recording
                        .store(false, atomic::Ordering::Relaxed);
                    self_clone.reset().await;
                    // Check status again after some seconds
                    let secs = random::<u64>() % 5;
                    tokio::time::sleep(Duration::from_secs(secs)).await;
                    continue;
                }

                tokio::time::sleep(Duration::from_secs(15)).await;
            }
            log::info!("[{}]Recording thread quit.", self_clone.room_id);
        }));
    }
}
