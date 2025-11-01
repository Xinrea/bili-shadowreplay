pub mod api;
pub mod profile;
pub mod response;
use crate::account::Account;
use crate::core::hls_recorder::HlsRecorder;
use crate::events::RecorderEvent;
use crate::platforms::bilibili::api::{Protocol, Qn};
use crate::platforms::PlatformType;
use crate::traits::RecorderTrait;
use crate::{Recorder, RoomInfo, UserInfo};

use crate::core::Format;
use crate::core::{Codec, HlsStream};
use crate::danmu::DanmuStorage;
use crate::platforms::bilibili::api::BiliStream;
use chrono::Utc;
use danmu_stream::danmu_stream::DanmuStream;
use danmu_stream::provider::ProviderType;
use danmu_stream::DanmuMessageType;
use std::path::{Path, PathBuf};
use std::sync::{atomic, Arc};
use std::time::Duration;
use tokio::sync::{broadcast, Mutex, RwLock};

use async_trait::async_trait;

/// A recorder for `BiliBili` live streams
///
/// This recorder fetches, caches and serves TS entries, currently supporting only `StreamType::FMP4`.
/// As high-quality streams are accessible only to logged-in users, the use of a `BiliClient`, which manages cookies, is required.
#[derive(Clone)]
pub struct BiliExtra {
    cover: Arc<RwLock<Option<String>>>,
    live_stream: Arc<RwLock<Option<BiliStream>>>,
}

pub type BiliRecorder = Recorder<BiliExtra>;

impl BiliRecorder {
    pub async fn new(
        room_id: &str,
        account: &Account,
        cache_dir: PathBuf,
        event_channel: broadcast::Sender<RecorderEvent>,
        update_interval: Arc<atomic::AtomicU64>,
        enabled: bool,
    ) -> Result<Self, crate::errors::RecorderError> {
        let client = reqwest::Client::new();
        let extra = BiliExtra {
            cover: Arc::new(RwLock::new(None)),
            live_stream: Arc::new(RwLock::new(None)),
        };

        let recorder = Self {
            platform: PlatformType::BiliBili,
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
            total_duration: Arc::new(atomic::AtomicU64::new(0)),
            total_size: Arc::new(atomic::AtomicU64::new(0)),
            extra,
        };

        log::info!("[{}]Recorder for room {} created.", room_id, room_id);

        Ok(recorder)
    }

    fn log_info(&self, message: &str) {
        log::info!("[{}]{}", self.room_id, message);
    }

    fn log_error(&self, message: &str) {
        log::error!("[{}]{}", self.room_id, message);
    }

    pub async fn reset(&self) {
        *self.extra.live_stream.write().await = None;
        self.last_update
            .store(Utc::now().timestamp(), atomic::Ordering::Relaxed);
        *self.danmu_storage.write().await = None;
        *self.platform_live_id.write().await = String::new();
        *self.live_id.write().await = String::new();
        self.total_duration.store(0, atomic::Ordering::Relaxed);
        self.total_size.store(0, atomic::Ordering::Relaxed);
    }

    async fn check_status(&self) -> bool {
        let pre_live_status = self.room_info.read().await.status;
        match api::get_room_info(&self.client, &self.account, &self.room_id).await {
            Ok(room_info) => {
                *self.room_info.write().await = RoomInfo {
                    platform: "bilibili".to_string(),
                    room_id: self.room_id.to_string(),
                    room_title: room_info.room_title,
                    room_cover: room_info.room_cover_url.clone(),
                    status: room_info.live_status == 1,
                };
                // Only update user info once
                if self.user_info.read().await.user_id != room_info.user_id {
                    let user_id = room_info.user_id;
                    let user_info = api::get_user_info(&self.client, &self.account, &user_id).await;
                    if let Ok(user_info) = user_info {
                        *self.user_info.write().await = UserInfo {
                            user_id: user_id.to_string(),
                            user_name: user_info.user_name,
                            user_avatar: user_info.user_avatar_url,
                        }
                    } else {
                        self.log_error(&format!(
                            "Failed to get user info: {}",
                            user_info.err().unwrap()
                        ));
                    }
                }
                let live_status = room_info.live_status == 1;

                // handle live notification
                if pre_live_status != live_status {
                    self.log_info(&format!(
                        "Live status changed to {}, enabled: {}",
                        live_status,
                        self.enabled.load(atomic::Ordering::Relaxed)
                    ));

                    if live_status {
                        // Get cover image
                        let room_cover_path = Path::new(PlatformType::BiliBili.as_str())
                            .join(&self.room_id)
                            .join("cover.jpg");
                        let full_room_cover_path = self.cache_dir.join(&room_cover_path);
                        if (api::download_file(
                            &self.client,
                            &room_info.room_cover_url,
                            &full_room_cover_path,
                        )
                        .await)
                            .is_ok()
                        {
                            *self.extra.cover.write().await =
                                Some(room_cover_path.to_str().unwrap().to_string());
                        }
                        let _ = self.event_channel.send(RecorderEvent::LiveStart {
                            recorder: self.info().await,
                        });
                    } else {
                        let _ = self.event_channel.send(RecorderEvent::LiveEnd {
                            platform: PlatformType::BiliBili,
                            room_id: self.room_id.to_string(),
                            recorder: self.info().await,
                        });
                        *self.live_id.write().await = String::new();
                    }

                    // just doing reset, cuz live status is changed
                    self.reset().await;
                }

                *self.platform_live_id.write().await = room_info.live_start_time.to_string();

                if !live_status {
                    return false;
                }

                // no need to check stream if should not record
                if !self.should_record().await {
                    return true;
                }

                // current_record => update stream
                // auto_start+is_new_stream => update stream and current_record=true
                let new_stream = api::get_stream_info(
                    &self.client,
                    &self.account,
                    &self.room_id,
                    Protocol::HttpHls,
                    Format::TS,
                    &[Codec::Avc, Codec::Hevc],
                    Qn::Q4K,
                )
                .await;

                match new_stream {
                    Ok(stream) => {
                        let pre_live_stream = self.extra.live_stream.read().await.clone();
                        *self.extra.live_stream.write().await = Some(stream.clone());
                        self.last_update
                            .store(Utc::now().timestamp(), atomic::Ordering::Relaxed);

                        log::info!(
                            "[{}]Update to a new stream: {:#?} => {:#?}",
                            &self.room_id,
                            pre_live_stream,
                            stream
                        );

                        true
                    }
                    Err(e) => {
                        if let crate::errors::RecorderError::FormatNotFound { format } = e {
                            log::error!("[{}]Format {} not found", &self.room_id, format);

                            true
                        } else {
                            log::error!("[{}]Fetch stream failed: {}", &self.room_id, e);

                            true
                        }
                    }
                }
            }
            Err(e) => {
                log::error!("[{}]Update room status failed: {}", &self.room_id, e);
                // may encounter internet issues, not sure whether the stream is closed or started, just remain
                pre_live_status
            }
        }
    }

    async fn danmu(&self) -> Result<(), crate::errors::RecorderError> {
        let cookies = self.account.cookies.clone();
        let room_id = self.room_id.clone();
        let danmu_stream = DanmuStream::new(ProviderType::BiliBili, &cookies, &room_id).await;
        if danmu_stream.is_err() {
            let err = danmu_stream.err().unwrap();
            log::error!("[{}]Failed to create danmu stream: {}", &self.room_id, err);
            return Err(crate::errors::RecorderError::DanmuStreamError(err));
        }
        let danmu_stream = danmu_stream.unwrap();

        let mut start_fut = Box::pin(danmu_stream.start());

        loop {
            tokio::select! {
                start_res = &mut start_fut => {
                    match start_res {
                        Ok(_) => {
                            log::info!("[{}]Danmu stream finished", &self.room_id);
                            return Ok(());
                        }
                        Err(err) => {
                            log::error!("[{}]Danmu stream start error: {}", &self.room_id, err);
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
                                    if let Some(storage) = self.danmu_storage.write().await.as_ref() {
                                        storage.add_line(ts, &danmu.message).await;
                                    }
                                }
                            }
                        }
                        Ok(None) => {
                            log::info!("[{}]Danmu stream closed", &self.room_id);
                            return Ok(());
                        }
                        Err(err) => {
                            log::error!("[{}]Failed to receive danmu message: {}", &self.room_id, err);
                            return Err(crate::errors::RecorderError::DanmuStreamError(err));
                        }
                    }
                }
            }
        }
    }

    /// Update entries for a new live
    async fn update_entries(&self, live_id: &str) -> Result<(), crate::errors::RecorderError> {
        let current_stream = self.extra.live_stream.read().await.clone();
        let Some(current_stream) = current_stream else {
            return Err(crate::errors::RecorderError::NoStreamAvailable);
        };

        let work_dir = self.work_dir(live_id).await;
        log::info!("[{}]New record started: {}", self.room_id, live_id);

        let _ = tokio::fs::create_dir_all(&work_dir.full_path()).await;

        let danmu_path = work_dir.with_filename("danmu.txt");
        *self.danmu_storage.write().await = DanmuStorage::new(&danmu_path.full_path()).await;

        let cover_path = work_dir.with_filename("cover.jpg");
        let room_cover_path = self
            .cache_dir
            .join(PlatformType::BiliBili.as_str())
            .join(&self.room_id)
            .join("cover.jpg");

        tokio::fs::copy(room_cover_path, &cover_path.full_path())
            .await
            .map_err(crate::errors::RecorderError::IoError)?;

        *self.live_id.write().await = live_id.to_string();

        // send record start event
        let _ = self.event_channel.send(RecorderEvent::RecordStart {
            recorder: self.info().await,
        });

        self.is_recording.store(true, atomic::Ordering::Relaxed);

        let stream = Arc::new(HlsStream::new(
            live_id.to_string(),
            current_stream.url_info.first().unwrap().host.clone(),
            current_stream.base_url.clone(),
            current_stream.url_info.first().unwrap().extra.clone(),
            current_stream.format,
            current_stream.codec,
        ));
        let hls_recorder = HlsRecorder::new(
            self.room_id.to_string(),
            stream,
            self.client.clone(),
            None,
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
impl crate::traits::RecorderTrait<BiliExtra> for BiliRecorder {
    async fn run(&self) {
        let self_clone = self.clone();
        let danmu_task = tokio::spawn(async move {
            let _ = self_clone.danmu().await;
        });
        *self.danmu_task.lock().await = Some(danmu_task);

        let self_clone = self.clone();
        *self.record_task.lock().await = Some(tokio::spawn(async move {
            log::info!("[{}]Start running recorder", self_clone.room_id);
            while !self_clone.quit.load(atomic::Ordering::Relaxed) {
                if self_clone.check_status().await {
                    // Live status is ok, start recording.
                    if self_clone.should_record().await {
                        let live_id = Utc::now().timestamp_millis().to_string();

                        if let Err(e) = self_clone.update_entries(&live_id).await {
                            log::error!("[{}]Update entries error: {}", self_clone.room_id, e);
                        }

                        let _ = self_clone.event_channel.send(RecorderEvent::RecordEnd {
                            recorder: self_clone.info().await,
                        });
                    }

                    self_clone
                        .is_recording
                        .store(false, atomic::Ordering::Relaxed);

                    self_clone.reset().await;
                    // go check status again after random 2-5 secs
                    let secs = rand::random::<u64>() % 4 + 2;
                    tokio::time::sleep(Duration::from_secs(secs)).await;
                    continue;
                }

                tokio::time::sleep(Duration::from_secs(
                    self_clone.update_interval.load(atomic::Ordering::Relaxed),
                ))
                .await;
            }
        }));
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn parse_fmp4_playlist() {
        let content = r#"#EXTM3U
        #EXT-X-VERSION:7
        #EXT-X-START:TIME-OFFSET=0
        #EXT-X-MEDIA-SEQUENCE:323066244
        #EXT-X-TARGETDURATION:1
        #EXT-X-MAP:URI=\"h1758715459.m4s\"
        #EXT-BILI-AUX:97d350|K|7d1e3|fe1425ab
        #EXTINF:1.00,7d1e3|fe1425ab
        323066244.m4s
        #EXT-BILI-AUX:97d706|N|757d4|c9094969
        #EXTINF:1.00,757d4|c9094969
        323066245.m4s
        #EXT-BILI-AUX:97daee|N|8223d|f307566a
        #EXTINF:1.00,8223d|f307566a
        323066246.m4s
        #EXT-BILI-AUX:97dee7|N|775cc|428d567
        #EXTINF:1.00,775cc|428d567
        323066247.m4s
        #EXT-BILI-AUX:97e2df|N|10410|9a62fe61
        #EXTINF:0.17,10410|9a62fe61
        323066248.m4s
        #EXT-BILI-AUX:97e397|K|679d2|8fbee7df
        #EXTINF:1.00,679d2|8fbee7df
        323066249.m4s
        #EXT-BILI-AUX:97e74d|N|8907b|67d1c6ad
        #EXTINF:1.00,8907b|67d1c6ad
        323066250.m4s
        #EXT-BILI-AUX:97eb35|N|87374|f6406797
        #EXTINF:1.00,87374|f6406797
        323066251.m4s
        #EXT-BILI-AUX:97ef2d|N|6b792|b8125097
        #EXTINF:1.00,6b792|b8125097
        323066252.m4s
        #EXT-BILI-AUX:97f326|N|e213|b30c02c6
        #EXTINF:0.17,e213|b30c02c6
        323066253.m4s
        #EXT-BILI-AUX:97f3de|K|65754|7ea6dcc8
        #EXTINF:1.00,65754|7ea6dcc8
        323066254.m4s
        "#;
        let (_, pl) = m3u8_rs::parse_media_playlist(content.as_bytes()).unwrap();
        // ExtTag { tag: "X-MAP", rest: Some("URI=\\\"h1758715459.m4s\\\"") }
        let header_url = pl
            .segments
            .first()
            .unwrap()
            .unknown_tags
            .iter()
            .find(|t| t.tag == "X-MAP")
            .map(|t| {
                let rest = t.rest.clone().unwrap();
                rest.split('=').nth(1).unwrap().replace("\\\"", "")
            });
        // #EXT-BILI-AUX:a5e4e0|K|79b3e|ebde469e
        let is_key = pl
            .segments
            .first()
            .unwrap()
            .unknown_tags
            .iter()
            .find(|t| t.tag == "BILI-AUX")
            .map(|t| {
                let rest = t.rest.clone().unwrap();
                rest.split('|').nth(1).unwrap() == "K"
            });
        assert_eq!(is_key, Some(true));
        assert_eq!(header_url, Some("h1758715459.m4s".to_string()));
    }
}
