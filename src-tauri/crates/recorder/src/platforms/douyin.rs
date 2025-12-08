pub mod api;
mod response;
pub mod stream_info;
use crate::account::Account;
use crate::core::hls_recorder::{construct_stream_from_variant, HlsRecorder};
use crate::core::{Codec, Format};
use crate::errors::RecorderError;
use crate::events::RecorderEvent;
use crate::platforms::douyin::stream_info::DouyinStream;
use crate::traits::RecorderTrait;
use crate::{Recorder, RoomInfo, UserInfo};
use async_trait::async_trait;
use chrono::Utc;
use danmu_stream::danmu_stream::DanmuStream;
use danmu_stream::provider::ProviderType;
use danmu_stream::DanmuMessageType;
use rand::random;
use std::path::PathBuf;
use std::sync::{atomic, Arc};
use std::time::Duration;
use tokio::sync::{broadcast, Mutex, RwLock};

use crate::danmu::DanmuStorage;
use crate::platforms::PlatformType;

pub type DouyinRecorder = Recorder<DouyinExtra>;

#[derive(Clone)]
pub struct DouyinExtra {
    sec_user_id: String,
    live_stream: Arc<RwLock<Option<DouyinStream>>>,
}

fn get_best_stream_url(stream: &DouyinStream) -> Option<String> {
    // find the best stream url
    if stream.data.origin.main.hls.is_empty() {
        log::error!("No stream url found in stream_data: {stream:#?}");
        return None;
    }

    Some(stream.data.origin.main.hls.clone())
}

impl DouyinRecorder {
    pub async fn new(
        room_id: &str,
        sec_user_id: &str,
        account: &Account,
        cache_dir: PathBuf,
        channel: broadcast::Sender<RecorderEvent>,
        update_interval: Arc<atomic::AtomicU64>,
        enabled: bool,
    ) -> Result<Self, crate::errors::RecorderError> {
        Ok(Self {
            platform: PlatformType::Douyin,
            room_id: room_id.to_string(),
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
            update_interval,
            total_duration: Arc::new(atomic::AtomicU64::new(0)),
            total_size: Arc::new(atomic::AtomicU64::new(0)),
            extra: DouyinExtra {
                sec_user_id: sec_user_id.to_string(),
                live_stream: Arc::new(RwLock::new(None)),
            },
        })
    }

    async fn check_status(&self) -> bool {
        let pre_live_status = self.room_info.read().await.status;
        match api::get_room_info(
            &self.client,
            &self.account,
            &self.room_id,
            &self.extra.sec_user_id,
        )
        .await
        {
            Ok(info) => {
                let live_status = info.status == 0; // room_status == 0 表示正在直播

                *self.room_info.write().await = RoomInfo {
                    platform: PlatformType::Douyin.as_str().to_string(),
                    room_id: self.room_id.to_string(),
                    room_title: info.room_title.clone(),
                    room_cover: info.cover.clone().unwrap_or_default(),
                    status: live_status,
                };

                *self.user_info.write().await = UserInfo {
                    user_id: info.sec_user_id.clone(),
                    user_name: info.user_name.clone(),
                    user_avatar: info.user_avatar.clone(),
                };

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
                            room_id: self.room_id.clone(),
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

                // Get stream URL when live starts
                if !info.hls_url.is_empty() {
                    // Only set stream URL, don't create record yet
                    // Record will be created when first ts download succeeds
                    // parse info.stream_data into DouyinStream
                    let stream_data = info.stream_data.clone();
                    let Ok(stream) = serde_json::from_str::<DouyinStream>(&stream_data) else {
                        log::error!("Failed to parse stream data: {:#?}", &info);
                        return false;
                    };
                    let Some(new_stream_url) = get_best_stream_url(&stream) else {
                        log::error!("No stream url found in stream_data: {stream:#?}");
                        return false;
                    };

                    log::info!("New douyin stream URL: {}", new_stream_url.clone());
                    *self.extra.live_stream.write().await = Some(stream);
                    (*self.platform_live_id.write().await).clone_from(&info.room_id_str);
                }

                true
            }
            Err(e) => {
                log::warn!("[{}]Update room status failed: {}", &self.room_id, e);
                pre_live_status
            }
        }
    }

    async fn danmu(&self) -> Result<(), crate::errors::RecorderError> {
        let cookies = self.account.cookies.clone();
        let danmu_room_id = self
            .platform_live_id
            .read()
            .await
            .clone()
            .parse::<i64>()
            .unwrap_or(0);
        let danmu_stream =
            DanmuStream::new(ProviderType::Douyin, &cookies, &danmu_room_id.to_string()).await;
        if danmu_stream.is_err() {
            let err = danmu_stream.err().unwrap();
            log::error!("Failed to create danmu stream: {err}");
            return Err(crate::errors::RecorderError::DanmuStreamError(err));
        }
        let danmu_stream = danmu_stream.unwrap();

        let mut start_fut = Box::pin(danmu_stream.start());

        loop {
            tokio::select! {
                start_res = &mut start_fut => {
                    match start_res {
                        Ok(_) => {
                            log::info!("Danmu stream finished");
                            return Ok(());
                        }
                        Err(err) => {
                            log::error!("Danmu stream start error: {err}");
                            return Err(crate::errors::RecorderError::DanmuStreamError(err));
                        }
                    }
                }
                recv_res = danmu_stream.recv() => {
                    match recv_res {
                        Ok(Some(msg)) => {
                            match msg {
                                DanmuMessageType::DanmuMessage(danmu) => {
                                    let ts = Utc::now().timestamp_millis();
                                    let _ = self.event_channel.send(RecorderEvent::DanmuReceived {
                                        room: self.room_id.clone(),
                                        ts,
                                        content: danmu.message.clone(),
                                    });

                                    if let Some(danmu_storage) = self.danmu_storage.read().await.as_ref() {
                                        danmu_storage.add_line(ts, &danmu.message).await;
                                    }
                                }
                            }
                        }
                        Ok(None) => {
                            log::info!("Danmu stream closed");
                            return Ok(());
                        }
                        Err(err) => {
                            log::error!("Failed to receive danmu message: {err}");
                            return Err(crate::errors::RecorderError::DanmuStreamError(err));
                        }
                    }
                }
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
        if let Some(danmu_task) = self.danmu_task.lock().await.take() {
            danmu_task.abort();
            let _ = danmu_task.await;
            log::info!("Danmu task aborted");
        }
    }

    async fn update_entries(&self, live_id: &str) -> Result<(), RecorderError> {
        // Get current room info and stream URL
        let room_info = self.room_info.read().await.clone();
        let Some(stream) = self.extra.live_stream.read().await.clone() else {
            return Err(RecorderError::NoStreamAvailable);
        };
        let Some(stream_url) = get_best_stream_url(&stream) else {
            return Err(RecorderError::NoStreamAvailable);
        };

        let work_dir = self.work_dir(live_id).await;
        let _ = tokio::fs::create_dir_all(&work_dir.full_path()).await;

        // download cover
        let cover_url = room_info.room_cover.clone();
        let cover_path = work_dir.with_filename("cover.jpg");
        let _ = api::download_file(&self.client, &cover_url, &cover_path.full_path()).await;

        // Setup danmu store
        let danmu_file_path = work_dir.with_filename("danmu.txt");
        let danmu_storage = DanmuStorage::new(&danmu_file_path.full_path()).await;
        *self.danmu_storage.write().await = danmu_storage;

        // Start danmu task
        *self.live_id.write().await = live_id.to_string();

        let self_clone = self.clone();
        log::info!("Start fetching danmu for live {live_id}");
        *self.danmu_task.lock().await = Some(tokio::spawn(async move {
            let _ = self_clone.danmu().await;
        }));

        let _ = self.event_channel.send(RecorderEvent::RecordStart {
            recorder: self.info().await,
        });

        let hls_stream =
            construct_stream_from_variant(live_id, &stream_url, Format::TS, Codec::Avc)
                .await
                .map_err(|_| RecorderError::NoStreamAvailable)?;
        let hls_recorder = HlsRecorder::new(
            self.room_id.to_string(),
            Arc::new(hls_stream),
            self.client.clone(),
            None,
            self.event_channel.clone(),
            work_dir.full_path(),
            self.enabled.clone(),
        )
        .await;
        if let Err(e) = hls_recorder.start().await {
            log::error!("[{}]Error from hls recorder: {}", self.room_id, e);
            return Err(e);
        }

        Ok(())
    }
}

#[async_trait]
impl crate::traits::RecorderTrait<DouyinExtra> for DouyinRecorder {
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

                tokio::time::sleep(Duration::from_secs(
                    self_clone.update_interval.load(atomic::Ordering::Relaxed),
                ))
                .await;
            }
            log::info!("[{}]Recording thread quit.", self_clone.room_id);
        }));
    }
}
