pub mod api;
pub mod response;

use crate::account::Account;
use crate::core::flv_recorder::FlvRecorder;
use crate::core::hls_recorder::{construct_stream_from_variant, HlsRecorder};
use crate::core::{Codec, Format};
use crate::danmu::DanmuStorage;
use crate::errors::RecorderError;
use crate::events::RecorderEvent;
use crate::platforms::PlatformType;
use crate::traits::RecorderTrait;
use crate::{Recorder, RoomInfo, UserInfo};
use async_trait::async_trait;
use chrono::Utc;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{atomic, Arc};
use std::time::Duration;
use tokio::sync::{broadcast, Mutex, RwLock};

const TIKTOK_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

#[derive(Clone)]
pub struct TikTokExtra {
    stream_info: Arc<RwLock<Option<api::StreamInfo>>>,
    pre_live_id: Arc<RwLock<Option<String>>>,
    should_continue: Arc<AtomicBool>,
}

pub type TikTokRecorder = Recorder<TikTokExtra>;

impl TikTokRecorder {
    pub async fn new(
        room_id: &str,
        account: &Account,
        cache_dir: PathBuf,
        event_channel: broadcast::Sender<RecorderEvent>,
        update_interval: Arc<atomic::AtomicU64>,
        enabled: bool,
    ) -> Result<Self, RecorderError> {
        let mut default_headers = reqwest::header::HeaderMap::new();
        default_headers.insert("Referer", "https://www.tiktok.com/".parse().unwrap());
        default_headers.insert(
            "User-Agent",
            TIKTOK_USER_AGENT.parse().unwrap(),
        );

        let proxy_url = api::proxy_url_from_env();
        let client = if let Some(proxy_url) = proxy_url.as_deref() {
            api::build_proxy_client(proxy_url)?
        } else {
            reqwest::Client::builder()
                .default_headers(default_headers)
                .build()
                .map_err(|e| RecorderError::ApiError {
                    error: e.to_string(),
                })?
        };
        let extra = TikTokExtra {
            stream_info: Arc::new(RwLock::new(None)),
            pre_live_id: Arc::new(RwLock::new(None)),
            should_continue: Arc::new(AtomicBool::new(false)),
        };

        let recorder = Self {
            platform: PlatformType::TikTok,
            room_id: room_id.to_string(),
            account: account.clone(),
            client,
            event_channel,
            cache_dir,
            quit: Arc::new(atomic::AtomicBool::new(false)),
            enabled: Arc::new(atomic::AtomicBool::new(enabled)),
            update_interval,
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
            extra,
        };

        log::info!("[TikTok][{}]Recorder created", room_id);

        Ok(recorder)
    }

    fn log_info(&self, message: &str) {
        log::info!("[TikTok][{}]{}", self.room_id, message);
    }

    fn log_error(&self, message: &str) {
        log::error!("[TikTok][{}]{}", self.room_id, message);
    }

    fn prefer_flv() -> bool {
        std::env::var("BSR_TIKTOK_PREFER_FLV")
            .map(|v| {
                let v = v.to_ascii_lowercase();
                v == "1" || v == "true" || v == "yes" || v == "on"
            })
            .unwrap_or(false)
    }

    fn prefer_hls() -> bool {
        std::env::var("BSR_TIKTOK_PREFER_HLS")
            .map(|v| {
                let v = v.to_ascii_lowercase();
                v == "1" || v == "true" || v == "yes" || v == "on"
            })
            .unwrap_or(false)
    }

    pub async fn reset(&self) {
        *self.extra.stream_info.write().await = None;
        self.last_update
            .store(Utc::now().timestamp(), atomic::Ordering::Relaxed);
        *self.danmu_storage.write().await = None;
        *self.platform_live_id.write().await = String::new();
        *self.live_id.write().await = String::new();
    }

    async fn check_status(&self) -> bool {
        let pre_live_status = self.room_info.read().await.status;

        // TikTok URLs are typically like: https://www.tiktok.com/@username/live
        let url = if self.room_id.starts_with("http") {
            self.room_id.clone()
        } else {
            format!("https://www.tiktok.com/{}/live", self.room_id)
        };

        match api::get_room_info(&self.client, &self.account, &url).await {
            Ok(room_info) => {
                *self.room_info.write().await = RoomInfo {
                    platform: "tiktok".to_string(),
                    room_id: self.room_id.to_string(),
                    room_title: room_info.room_title.clone(),
                    room_cover: room_info.room_cover_url.clone(),
                    status: room_info.live_status,
                };

                if self.user_info.read().await.user_id != room_info.user_id {
                    *self.user_info.write().await = UserInfo {
                        user_id: room_info.user_id.to_string(),
                        user_name: room_info.user_name.clone(),
                        user_avatar: room_info.user_avatar.clone(),
                    }
                }

                let live_status = room_info.live_status;

                if pre_live_status != live_status {
                    self.log_info(&format!(
                        "Live status changed to {}, enabled: {}",
                        live_status,
                        self.enabled.load(atomic::Ordering::Relaxed)
                    ));

                    if live_status {
                        let _ = self.event_channel.send(RecorderEvent::LiveStart {
                            recorder: self.info().await,
                        });
                    } else {
                        let _ = self.event_channel.send(RecorderEvent::LiveEnd {
                            platform: PlatformType::TikTok,
                            room_id: self.room_id.to_string(),
                            recorder: self.info().await,
                        });
                        *self.live_id.write().await = String::new();
                    }

                    self.reset().await;
                }

                *self.platform_live_id.write().await = Utc::now().timestamp().to_string();

                if !live_status {
                    return false;
                }

                if !self.should_record().await {
                    return true;
                }

                let new_stream = api::get_stream_url(&self.client, &self.account, &url).await;

                match new_stream {
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
            Err(e) => {
                self.log_error(&format!("Update room status failed: {}", e));
                pre_live_status
            }
        }
    }

    async fn update_entries(&self, live_id: &str) -> Result<(), RecorderError> {
        let current_stream = self.extra.stream_info.read().await.clone();
        let Some(stream_info) = current_stream else {
            return Err(RecorderError::NoStreamAvailable);
        };

        let work_dir = self.work_dir(live_id).await;
        self.log_info(&format!("New record started: {}", live_id));

        let _ = tokio::fs::create_dir_all(&work_dir.full_path()).await;

        // Download cover
        let room_info = self.room_info.read().await.clone();
        let cover_url = room_info.room_cover.clone();
        let cover_path = work_dir.with_filename("cover.jpg");
        let _ = api::download_file(&self.client, &cover_url, &cover_path.full_path()).await;

        let danmu_path = work_dir.with_filename("danmu.txt");
        *self.danmu_storage.write().await = DanmuStorage::new(&danmu_path.full_path()).await;

        *self.live_id.write().await = live_id.to_string();

        let _ = self.event_channel.send(RecorderEvent::RecordStart {
            recorder: self.info().await,
        });

        self.is_recording.store(true, atomic::Ordering::Relaxed);

        let hls_url = stream_info.hls_url.clone();
        let rtmp_url = stream_info.rtmp_url.clone();

        let flv_url = rtmp_url
            .clone()
            .filter(|url| url.contains(".flv"));

        if flv_url.is_some() && !Self::prefer_hls() {
            if let Some(url) = flv_url.clone() {
                self.log_info("Using FLV recorder (prefer flv by default)");
                let mut headers = reqwest::header::HeaderMap::new();
                headers.insert("Referer", "https://www.tiktok.com/".parse().unwrap());
                headers.insert("Origin", "https://www.tiktok.com".parse().unwrap());
                headers.insert("User-Agent", TIKTOK_USER_AGENT.parse().unwrap());
                if !self.account.cookies.is_empty() {
                    headers.insert("Cookie", self.account.cookies.parse().unwrap());
                }
                let flv_recorder = FlvRecorder::new(
                    url,
                    headers,
                    work_dir.full_path(),
                    self.enabled.clone(),
                    self.event_channel.clone(),
                    live_id.to_string(),
                );
                if let Err(e) = flv_recorder.start().await {
                    self.log_error(&format!("Flv recorder quit with error: {}", e));
                    return Err(e);
                }
                return Ok(());
            }
        }

        if hls_url.is_none() {
            if let Some(url) = flv_url.clone() {
                self.log_info("Using FLV recorder (HLS unavailable)");
                let mut headers = reqwest::header::HeaderMap::new();
                headers.insert("Referer", "https://www.tiktok.com/".parse().unwrap());
                headers.insert("Origin", "https://www.tiktok.com".parse().unwrap());
                headers.insert("User-Agent", TIKTOK_USER_AGENT.parse().unwrap());
                if !self.account.cookies.is_empty() {
                    headers.insert("Cookie", self.account.cookies.parse().unwrap());
                }
                let flv_recorder = FlvRecorder::new(
                    url,
                    headers,
                    work_dir.full_path(),
                    self.enabled.clone(),
                    self.event_channel.clone(),
                    live_id.to_string(),
                );
                if let Err(e) = flv_recorder.start().await {
                    self.log_error(&format!("Flv recorder quit with error: {}", e));
                    return Err(e);
                }
                return Ok(());
            }
        }

        if Self::prefer_flv() {
            if let Some(url) = flv_url.clone() {
                self.log_info("Using FLV recorder (prefer_flv)");
                let mut headers = reqwest::header::HeaderMap::new();
                headers.insert("Referer", "https://www.tiktok.com/".parse().unwrap());
                headers.insert("Origin", "https://www.tiktok.com".parse().unwrap());
                headers.insert("User-Agent", TIKTOK_USER_AGENT.parse().unwrap());
                if !self.account.cookies.is_empty() {
                    headers.insert("Cookie", self.account.cookies.parse().unwrap());
                }
                let flv_recorder = FlvRecorder::new(
                    url,
                    headers,
                    work_dir.full_path(),
                    self.enabled.clone(),
                    self.event_channel.clone(),
                    live_id.to_string(),
                );
                if let Err(e) = flv_recorder.start().await {
                    self.log_error(&format!("Flv recorder quit with error: {}", e));
                    return Err(e);
                }
                return Ok(());
            }
        }

        // Prefer HLS stream if available
        let stream_url = hls_url
            .clone()
            .or(rtmp_url.clone())
            .ok_or(RecorderError::NoStreamAvailable)?;

        let hls_stream = construct_stream_from_variant(
            live_id,
            &stream_url,
            Format::TS,
            Codec::Avc,
        )
        .await
        .map_err(|_| RecorderError::NoStreamAvailable)?;

        let hls_recorder = HlsRecorder::new(
            self.room_id.to_string(),
            Arc::new(hls_stream),
            self.client.clone(),
            if self.account.cookies.is_empty() {
                None
            } else {
                Some(self.account.cookies.clone())
            },
            Some({
                let mut headers = reqwest::header::HeaderMap::new();
                headers.insert("Referer", "https://www.tiktok.com/".parse().unwrap());
                headers.insert("Origin", "https://www.tiktok.com".parse().unwrap());
                headers.insert("User-Agent", TIKTOK_USER_AGENT.parse().unwrap());
                headers
            }),
            self.event_channel.clone(),
            work_dir.full_path(),
            self.enabled.clone(),
        )
        .await?;

        if let Err(e) = hls_recorder.start().await {
            self.log_error(&format!("Hls recorder quit with error: {}", e));
            if let Some(url) = flv_url {
                self.log_info("HLS failed, fallback to FLV recorder");
                let mut headers = reqwest::header::HeaderMap::new();
                headers.insert("Referer", "https://www.tiktok.com/".parse().unwrap());
                headers.insert("Origin", "https://www.tiktok.com".parse().unwrap());
                headers.insert("User-Agent", TIKTOK_USER_AGENT.parse().unwrap());
                if !self.account.cookies.is_empty() {
                    headers.insert("Cookie", self.account.cookies.parse().unwrap());
                }
                let flv_recorder = FlvRecorder::new(
                    url,
                    headers,
                    work_dir.full_path(),
                    self.enabled.clone(),
                    self.event_channel.clone(),
                    live_id.to_string(),
                );
                if let Err(err) = flv_recorder.start().await {
                    self.log_error(&format!("Flv recorder quit with error: {}", err));
                    return Err(err);
                }
                return Ok(());
            }
            return Err(e);
        }

        Ok(())
    }
}

#[async_trait]
impl RecorderTrait<TikTokExtra> for TikTokRecorder {
    async fn run(&self) {
        let self_clone = self.clone();
        *self.record_task.lock().await = Some(tokio::spawn(async move {
            self_clone.log_info("Start running recorder");
            while !self_clone.quit.load(atomic::Ordering::Relaxed) {
                if self_clone.check_status().await {
                    if self_clone.should_record().await {
                        let live_id;
                        if self_clone.extra.should_continue.load(Ordering::Relaxed)
                            && self_clone.extra.pre_live_id.read().await.is_some()
                        {
                            live_id = self_clone.extra.pre_live_id.read().await.clone().unwrap();
                            self_clone
                                .extra
                                .should_continue
                                .store(false, Ordering::Relaxed);
                        } else {
                            live_id = Utc::now().timestamp_millis().to_string();
                            self_clone
                                .extra
                                .pre_live_id
                                .write()
                                .await
                                .replace(live_id.clone());
                        }

                        if let Err(e) = self_clone.update_entries(&live_id).await {
                            match e {
                                RecorderError::StreamExpired { expire } => {
                                    self_clone
                                        .extra
                                        .should_continue
                                        .store(true, Ordering::Relaxed);
                                    self_clone
                                        .log_info(&format!("Stream expired at {}", expire));
                                }
                                _ => {
                                    self_clone.log_error(&format!("Update entries error: {}", e));
                                }
                            }
                        }

                        let _ = self_clone.event_channel.send(RecorderEvent::RecordEnd {
                            recorder: self_clone.info().await,
                        });
                    }

                    self_clone
                        .is_recording
                        .store(false, atomic::Ordering::Relaxed);

                    self_clone.reset().await;
                    if self_clone.extra.should_continue.load(Ordering::Relaxed) {
                        continue;
                    }
                    let secs = rand::random::<u64>() % 4 + 2;
                    tokio::time::sleep(Duration::from_secs(secs)).await;
                    continue;
                }

                let interval = self_clone.update_interval.load(atomic::Ordering::Relaxed);
                let sleep_secs = if interval <= 10 {
                    rand::random::<u64>() % 11 + 10
                } else {
                    interval + rand::random::<u64>() % 5
                };
                tokio::time::sleep(Duration::from_secs(sleep_secs)).await;
            }
        }));
    }
}
