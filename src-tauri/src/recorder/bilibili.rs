pub mod client;
pub mod errors;
pub mod profile;
pub mod response;
use super::entry::{EntryStore, Range};
use super::errors::RecorderError;
use super::PlatformType;
use crate::database::account::AccountRow;
use crate::ffmpeg::get_video_resolution;
use crate::progress_manager::Event;
use crate::progress_reporter::EventEmitter;
use crate::recorder::Recorder;
use crate::recorder_manager::RecorderEvent;
use crate::subtitle_generator::item_to_srt;

use super::danmu::{DanmuEntry, DanmuStorage};
use super::entry::TsEntry;
use chrono::Utc;
use client::{BiliClient, BiliStream, RoomInfo, StreamType, UserInfo};
use danmu_stream::danmu_stream::DanmuStream;
use danmu_stream::provider::ProviderType;
use danmu_stream::DanmuMessageType;
use errors::BiliClientError;
use m3u8_rs::{Playlist, QuotedOrUnquoted, VariantStream};
use rand::seq::SliceRandom;
use regex::Regex;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{broadcast, Mutex, RwLock};
use tokio::task::JoinHandle;
use url::Url;

use crate::config::Config;
use crate::database::{Database, DatabaseError};

use async_trait::async_trait;

#[cfg(feature = "gui")]
use {tauri::AppHandle, tauri_plugin_notification::NotificationExt};

/// A recorder for BiliBili live streams
///
/// This recorder fetches, caches and serves TS entries, currently supporting only StreamType::FMP4.
/// As high-quality streams are accessible only to logged-in users, the use of a BiliClient, which manages cookies, is required.
// TODO implement StreamType::TS
#[derive(Clone)]
pub struct BiliRecorder {
    #[cfg(feature = "gui")]
    app_handle: AppHandle,
    emitter: EventEmitter,
    client: Arc<RwLock<BiliClient>>,
    db: Arc<Database>,
    account: AccountRow,
    config: Arc<RwLock<Config>>,
    room_id: u64,
    room_info: Arc<RwLock<RoomInfo>>,
    user_info: Arc<RwLock<UserInfo>>,
    live_status: Arc<RwLock<bool>>,
    live_id: Arc<RwLock<String>>,
    cover: Arc<RwLock<Option<String>>>,
    entry_store: Arc<RwLock<Option<EntryStore>>>,
    is_recording: Arc<RwLock<bool>>,
    last_update: Arc<RwLock<i64>>,
    quit: Arc<Mutex<bool>>,
    live_stream: Arc<RwLock<Option<BiliStream>>>,
    danmu_storage: Arc<RwLock<Option<DanmuStorage>>>,
    event_channel: broadcast::Sender<RecorderEvent>,
    enabled: Arc<RwLock<bool>>,
    last_segment_offset: Arc<RwLock<Option<i64>>>, // 保存上次处理的最后一个片段的偏移
    current_header_info: Arc<RwLock<Option<HeaderInfo>>>, // 保存当前的分辨率

    danmu_task: Arc<Mutex<Option<JoinHandle<()>>>>,
    record_task: Arc<Mutex<Option<JoinHandle<()>>>>,
    master_manifest: Arc<RwLock<Option<String>>>,
}

impl From<DatabaseError> for super::errors::RecorderError {
    fn from(value: DatabaseError) -> Self {
        super::errors::RecorderError::InvalidDBOP { err: value }
    }
}

impl From<BiliClientError> for super::errors::RecorderError {
    fn from(value: BiliClientError) -> Self {
        super::errors::RecorderError::BiliClientError { err: value }
    }
}

pub struct BiliRecorderOptions {
    #[cfg(feature = "gui")]
    pub app_handle: AppHandle,
    pub emitter: EventEmitter,
    pub db: Arc<Database>,
    pub room_id: u64,
    pub account: AccountRow,
    pub config: Arc<RwLock<Config>>,
    pub auto_start: bool,
    pub channel: broadcast::Sender<RecorderEvent>,
}

#[derive(Debug, Clone)]
struct HeaderInfo {
    url: String,
    resolution: String,
}

impl BiliRecorder {
    pub async fn new(options: BiliRecorderOptions) -> Result<Self, super::errors::RecorderError> {
        let client = BiliClient::new()?;
        let room_info = client
            .get_room_info(&options.account, options.room_id)
            .await?;
        let user_info = client
            .get_user_info(&options.account, room_info.user_id)
            .await?;
        let mut live_status = false;
        let mut cover = None;
        if room_info.live_status == 1 {
            live_status = true;

            let room_cover_path = Path::new(PlatformType::BiliBili.as_str())
                .join(options.room_id.to_string())
                .join("cover.jpg");
            let full_room_cover_path =
                Path::new(&options.config.read().await.cache).join(&room_cover_path);
            // Get cover image
            if (client
                .download_file(&room_info.room_cover_url, &full_room_cover_path)
                .await)
                .is_ok()
            {
                cover = Some(room_cover_path.to_str().unwrap().to_string());
            }
        }

        let recorder = Self {
            #[cfg(feature = "gui")]
            app_handle: options.app_handle,
            emitter: options.emitter,
            client: Arc::new(RwLock::new(client)),
            db: options.db.clone(),
            account: options.account.clone(),
            config: options.config.clone(),
            room_id: options.room_id,
            room_info: Arc::new(RwLock::new(room_info)),
            user_info: Arc::new(RwLock::new(user_info)),
            live_status: Arc::new(RwLock::new(live_status)),
            entry_store: Arc::new(RwLock::new(None)),
            is_recording: Arc::new(RwLock::new(false)),
            live_id: Arc::new(RwLock::new(String::new())),
            cover: Arc::new(RwLock::new(cover)),
            last_update: Arc::new(RwLock::new(Utc::now().timestamp())),
            quit: Arc::new(Mutex::new(false)),
            live_stream: Arc::new(RwLock::new(None)),
            danmu_storage: Arc::new(RwLock::new(None)),
            event_channel: options.channel,
            enabled: Arc::new(RwLock::new(options.auto_start)),
            last_segment_offset: Arc::new(RwLock::new(None)),
            current_header_info: Arc::new(RwLock::new(None)),
            danmu_task: Arc::new(Mutex::new(None)),
            record_task: Arc::new(Mutex::new(None)),
            master_manifest: Arc::new(RwLock::new(None)),
        };
        log::info!("Recorder for room {} created.", options.room_id);
        Ok(recorder)
    }

    pub async fn reset(&self) {
        *self.entry_store.write().await = None;
        *self.live_stream.write().await = None;
        *self.last_update.write().await = Utc::now().timestamp();
        *self.danmu_storage.write().await = None;
        *self.last_segment_offset.write().await = None;
        *self.current_header_info.write().await = None;
    }

    async fn should_record(&self) -> bool {
        if *self.quit.lock().await {
            return false;
        }

        *self.enabled.read().await
    }

    async fn check_status(&self) -> bool {
        match self
            .client
            .read()
            .await
            .get_room_info(&self.account, self.room_id)
            .await
        {
            Ok(room_info) => {
                *self.room_info.write().await = room_info.clone();
                let live_status = room_info.live_status == 1;

                // handle live notification
                if *self.live_status.read().await != live_status {
                    log::info!(
                        "[{}]Live status changed to {}, enabled: {}",
                        self.room_id,
                        live_status,
                        *self.enabled.read().await
                    );

                    if live_status {
                        if self.config.read().await.live_start_notify {
                            #[cfg(feature = "gui")]
                            self.app_handle
                                .notification()
                                .builder()
                                .title("BiliShadowReplay - 直播开始")
                                .body(format!(
                                    "{} 开启了直播：{}",
                                    self.user_info.read().await.user_name,
                                    room_info.room_title
                                ))
                                .show()
                                .unwrap();
                        }

                        // Get cover image
                        let room_cover_path = Path::new(PlatformType::BiliBili.as_str())
                            .join(self.room_id.to_string())
                            .join("cover.jpg");
                        let full_room_cover_path =
                            Path::new(&self.config.read().await.cache).join(&room_cover_path);
                        if (self
                            .client
                            .read()
                            .await
                            .download_file(&room_info.room_cover_url, &full_room_cover_path)
                            .await)
                            .is_ok()
                        {
                            *self.cover.write().await =
                                Some(room_cover_path.to_str().unwrap().to_string());
                        }
                        let _ = self.event_channel.send(RecorderEvent::LiveStart {
                            recorder: self.info().await,
                        });
                    } else {
                        if self.config.read().await.live_end_notify {
                            #[cfg(feature = "gui")]
                            self.app_handle
                                .notification()
                                .builder()
                                .title("BiliShadowReplay - 直播结束")
                                .body(format!(
                                    "{} 的直播结束了",
                                    self.user_info.read().await.user_name
                                ))
                                .show()
                                .unwrap();
                        }
                        let _ = self.event_channel.send(RecorderEvent::LiveEnd {
                            platform: PlatformType::BiliBili,
                            room_id: self.room_id,
                            recorder: self.info().await,
                        });
                        *self.live_id.write().await = String::new();
                    }

                    // just doing reset, cuz live status is changed
                    self.reset().await;
                }

                *self.live_status.write().await = live_status;

                if !live_status {
                    // reset cuz live is ended
                    self.reset().await;

                    return false;
                }

                // no need to check stream if should not record
                if !self.should_record().await {
                    return true;
                }

                // current_record => update stream
                // auto_start+is_new_stream => update stream and current_record=true
                let master_manifest = self.client.read().await.get_index_content(&self.account, &format!("https://api.live.bilibili.com/xlive/play-gateway/master/url?cid={}&pt=h5&p2p_type=-1&net=0&free_type=0&build=0&feature=2&qn=10000", self.room_id)).await;
                if master_manifest.is_err() {
                    log::error!(
                        "[{}]Fetch master manifest failed: {}",
                        self.room_id,
                        master_manifest.err().unwrap()
                    );
                    return true;
                }

                let master_manifest = master_manifest.unwrap();
                *self.master_manifest.write().await = Some(master_manifest.clone());

                let master_manifest = m3u8_rs::parse_playlist_res(master_manifest.as_bytes())
                    .map_err(|_| super::errors::RecorderError::M3u8ParseFailed {
                        content: master_manifest.clone(),
                    });
                if master_manifest.is_err() {
                    log::error!(
                        "[{}]Parse master manifest failed: {}",
                        self.room_id,
                        master_manifest.err().unwrap()
                    );
                    return true;
                }

                let master_manifest = master_manifest.unwrap();
                let variant = match master_manifest {
                    Playlist::MasterPlaylist(playlist) => {
                        let variants = playlist.variants.clone();
                        variants
                            .into_iter()
                            .filter(|variant| {
                                if let Some(other_attributes) = &variant.other_attributes {
                                    if let Some(QuotedOrUnquoted::Quoted(bili_display)) =
                                        other_attributes.get("BILI-DISPLAY")
                                    {
                                        bili_display == "原画"
                                    } else {
                                        false
                                    }
                                } else {
                                    false
                                }
                            })
                            .collect::<Vec<_>>()
                    }
                    _ => {
                        log::error!("[{}]Master manifest is not a media playlist", self.room_id);
                        vec![]
                    }
                };

                if variant.is_empty() {
                    log::error!("[{}]No variant found", self.room_id);
                    return true;
                }

                // random select a variant
                let variant = variant.choose(&mut rand::thread_rng()).unwrap();

                let new_stream = self.stream_from_variant(variant).await;
                if new_stream.is_err() {
                    log::error!(
                        "[{}]Fetch stream failed: {}",
                        self.room_id,
                        new_stream.err().unwrap()
                    );
                    return true;
                }

                let stream = new_stream.unwrap();

                let new_stream = self.fetch_real_stream(&stream).await;
                if new_stream.is_err() {
                    log::error!(
                        "[{}]Fetch real stream failed: {}",
                        self.room_id,
                        new_stream.err().unwrap()
                    );
                    return true;
                }

                let new_stream = new_stream.unwrap();
                *self.live_stream.write().await = Some(new_stream);
                *self.last_update.write().await = Utc::now().timestamp();

                log::info!(
                    "[{}]Update to a new stream: {:?} => {}",
                    self.room_id,
                    self.live_stream.read().await.clone(),
                    stream
                );

                true
            }
            Err(e) => {
                log::error!("[{}]Update room status failed: {}", self.room_id, e);
                // may encouter internet issues, not sure whether the stream is closed or started, just remain
                *self.live_status.read().await
            }
        }
    }

    async fn stream_from_variant(
        &self,
        variant: &VariantStream,
    ) -> Result<BiliStream, super::errors::RecorderError> {
        let url = variant.uri.clone();
        // example url: https://cn-hnld-ct-01-47.bilivideo.com/live-bvc/931676/live_1789460279_3538985/index.m3u8?expires=1745927098&len=0&oi=3729149990&pt=h5&qn=10000&trid=10075ceab17d4c9498264eb76d572b6810ad&sigparams=cdn,expires,len,oi,pt,qn,trid&cdn=cn-gotcha01&sign=686434f3ad01d33e001c80bfb7e1713d&site=3124fc9e0fabc664ace3d1b33638f7f2&free_type=0&mid=0&sche=ban&bvchls=1&sid=cn-hnld-ct-01-47&chash=0&bmt=1&sg=lr&trace=25&isp=ct&rg=East&pv=Shanghai&sk=28cc07215ff940102a1d60dade11467e&codec=0&pp=rtmp&hdr_type=0&hot_cdn=57345&suffix=origin&flvsk=c9154f5b3c6b14808bc5569329cf7f94&origin_bitrate=1281767&score=1&source=puv3_master&p2p_type=-1&deploy_env=prod&sl=1&info_source=origin&vd=nc&zoneid_l=151355393&sid_l=stream_name_cold&src=puv3&order=1
        // extract host: cn-hnld-ct-01-47.bilivideo.com
        let host = url.split('/').nth(2).unwrap_or_default();
        let extra = url.split('?').nth(1).unwrap_or_default();
        // extract base url: live-bvc/931676/live_1789460279_3538985/
        let base_url = url
            .split('/')
            .skip(3)
            .take(3)
            .collect::<Vec<&str>>()
            .join("/")
            + "/";
        let stream = BiliStream::new(StreamType::FMP4, base_url.as_str(), host, extra);
        Ok(stream)
    }

    async fn danmu(&self) -> Result<(), super::errors::RecorderError> {
        let cookies = self.account.cookies.clone();
        let room_id = self.room_id;
        let danmu_stream = DanmuStream::new(ProviderType::BiliBili, &cookies, room_id).await;
        if danmu_stream.is_err() {
            let err = danmu_stream.err().unwrap();
            log::error!("[{}]Failed to create danmu stream: {}", self.room_id, err);
            return Err(super::errors::RecorderError::DanmuStreamError { err });
        }
        let danmu_stream = danmu_stream.unwrap();

        // create a task to receive danmu message
        let danmu_stream_clone = danmu_stream.clone();
        tokio::spawn(async move {
            let _ = danmu_stream_clone.start().await;
        });

        loop {
            if let Ok(Some(msg)) = danmu_stream.recv().await {
                match msg {
                    DanmuMessageType::DanmuMessage(danmu) => {
                        let ts = Utc::now().timestamp_millis();
                        self.emitter.emit(&Event::DanmuReceived {
                            room: self.room_id,
                            ts,
                            content: danmu.message.clone(),
                        });
                        if let Some(storage) = self.danmu_storage.write().await.as_ref() {
                            storage.add_line(ts, &danmu.message).await;
                        }
                    }
                }
            } else {
                log::error!("[{}]Failed to receive danmu message", self.room_id);
                return Err(super::errors::RecorderError::DanmuStreamError {
                    err: danmu_stream::DanmuStreamError::WebsocketError {
                        err: "Failed to receive danmu message".to_string(),
                    },
                });
            }
        }
    }

    async fn get_playlist(&self) -> Result<Playlist, super::errors::RecorderError> {
        let stream = self.live_stream.read().await.clone();
        if stream.is_none() {
            return Err(super::errors::RecorderError::NoStreamAvailable);
        }
        let stream = stream.unwrap();
        match self
            .client
            .read()
            .await
            .get_index_content(&self.account, &stream.index())
            .await
        {
            Ok(index_content) => {
                if index_content.is_empty() {
                    return Err(super::errors::RecorderError::InvalidStream { stream });
                }
                if index_content.contains("Not Found") {
                    return Err(super::errors::RecorderError::IndexNotFound {
                        url: stream.index(),
                    });
                }
                m3u8_rs::parse_playlist_res(index_content.as_bytes()).map_err(|_| {
                    super::errors::RecorderError::M3u8ParseFailed {
                        content: index_content.clone(),
                    }
                })
            }
            Err(e) => {
                log::error!(
                    "[{}]Failed fetching index content from {}",
                    self.room_id,
                    stream.index()
                );
                log::error!(
                    "[{}]Master manifest: {}",
                    self.room_id,
                    self.master_manifest.read().await.as_ref().unwrap()
                );
                Err(super::errors::RecorderError::BiliClientError { err: e })
            }
        }
    }

    async fn get_header_url(&self) -> Result<String, super::errors::RecorderError> {
        let stream = self.live_stream.read().await.clone();
        if stream.is_none() {
            return Err(super::errors::RecorderError::NoStreamAvailable);
        }
        let stream = stream.unwrap();

        let index_content = self
            .client
            .read()
            .await
            .get_index_content(&self.account, &stream.index())
            .await?;
        if index_content.is_empty() {
            return Err(super::errors::RecorderError::InvalidStream { stream });
        }
        if index_content.contains("Not Found") {
            return Err(super::errors::RecorderError::IndexNotFound {
                url: stream.index(),
            });
        }

        let mut header_url = String::from("");
        let re = Regex::new(r"h.*\.m4s").unwrap();
        if let Some(captures) = re.captures(&index_content) {
            header_url = captures.get(0).unwrap().as_str().to_string();
        }
        if header_url.is_empty() {
            log::warn!(
                "[{}]Parse header url failed: {}",
                self.room_id,
                index_content
            );
        }

        Ok(header_url)
    }

    async fn get_resolution(
        &self,
        header_url: &str,
    ) -> Result<String, super::errors::RecorderError> {
        log::debug!("Get resolution from {}", header_url);
        let resolution = get_video_resolution(header_url)
            .await
            .map_err(|e| super::errors::RecorderError::FfmpegError { err: e })?;
        Ok(resolution)
    }

    async fn fetch_real_stream(
        &self,
        stream: &BiliStream,
    ) -> Result<BiliStream, super::errors::RecorderError> {
        let index_content = self
            .client
            .read()
            .await
            .get_index_content(&self.account, &stream.index())
            .await?;
        if index_content.is_empty() {
            return Err(super::errors::RecorderError::InvalidStream {
                stream: stream.clone(),
            });
        }
        if index_content.contains("Not Found") {
            return Err(super::errors::RecorderError::IndexNotFound {
                url: stream.index(),
            });
        }
        if index_content.contains("BANDWIDTH") {
            // this index content provides another m3u8 url
            // example: https://765b047cec3b099771d4b1851136046f.v.smtcdns.net/d1--cn-gotcha204-3.bilivideo.com/live-bvc/246284/live_1323355750_55526594/index.m3u8?expires=1741318366&len=0&oi=1961017843&pt=h5&qn=10000&trid=1007049a5300422eeffd2d6995d67b67ca5a&sigparams=cdn,expires,len,oi,pt,qn,trid&cdn=cn-gotcha204&sign=7ef1241439467ef27d3c804c1eda8d4d&site=1c89ef99adec13fab3a3592ee4db26d3&free_type=0&mid=475210&sche=ban&bvchls=1&trace=16&isp=ct&rg=East&pv=Shanghai&source=puv3_onetier&p2p_type=-1&score=1&suffix=origin&deploy_env=prod&flvsk=e5c4d6fb512ed7832b706f0a92f7a8c8&sk=246b3930727a89629f17520b1b551a2f&pp=rtmp&hot_cdn=57345&origin_bitrate=657300&sl=1&info_source=cache&vd=bc&src=puv3&order=1&TxLiveCode=cold_stream&TxDispType=3&svr_type=live_oc&tencent_test_client_ip=116.226.193.243&dispatch_from=OC_MGR61.170.74.11&utime=1741314857497
            let new_url = index_content.lines().last().unwrap();

            // extract host: cn-gotcha204-3.bilivideo.com
            let host = new_url.split('/').nth(2).unwrap_or_default();
            let extra = new_url.split('?').nth(1).unwrap_or_default();
            // extract base url: live-bvc/246284/live_1323355750_55526594/
            let base_url = new_url
                .split('/')
                .skip(3)
                .take_while(|&part| !part.contains('?') && part != "index.m3u8")
                .collect::<Vec<&str>>()
                .join("/")
                + "/";

            let new_stream = BiliStream::new(StreamType::FMP4, base_url.as_str(), host, extra);
            return Box::pin(self.fetch_real_stream(&new_stream)).await;
        }
        Ok(stream.clone())
    }

    async fn get_work_dir(&self, live_id: &str) -> String {
        format!(
            "{}/bilibili/{}/{}/",
            self.config.read().await.cache,
            self.room_id,
            live_id
        )
    }

    async fn update_entries(&self) -> Result<u128, super::errors::RecorderError> {
        let task_begin_time = std::time::Instant::now();
        let current_stream = self.live_stream.read().await.clone();
        if current_stream.is_none() {
            return Err(super::errors::RecorderError::NoStreamAvailable);
        }
        let current_stream = current_stream.unwrap();
        let parsed = self.get_playlist().await;
        if parsed.is_err() {
            return Err(parsed.err().unwrap());
        }

        let playlist = parsed.unwrap();

        let mut timestamp: i64 = self.live_id.read().await.parse::<i64>().unwrap_or(0);
        let mut work_dir;
        let mut is_first_record = false;

        // Get url from EXT-X-MAP
        let header_url = self.get_header_url().await?;
        if header_url.is_empty() {
            return Err(super::errors::RecorderError::EmptyHeader);
        }
        let full_header_url = current_stream.ts_url(&header_url);

        // Check header if None
        if (self.entry_store.read().await.as_ref().is_none()
            || self
                .entry_store
                .read()
                .await
                .as_ref()
                .unwrap()
                .get_header()
                .is_none())
            && current_stream.format == StreamType::FMP4
        {
            timestamp = Utc::now().timestamp_millis();
            *self.live_id.write().await = timestamp.to_string();
            work_dir = self.get_work_dir(timestamp.to_string().as_str()).await;
            is_first_record = true;

            let file_name = header_url.split('/').next_back().unwrap();
            let mut header = TsEntry {
                url: file_name.to_string(),
                sequence: 0,
                length: 0.0,
                size: 0,
                ts: timestamp,
                is_header: true,
            };

            // Create work directory before download
            tokio::fs::create_dir_all(&work_dir)
                .await
                .map_err(|e| super::errors::RecorderError::IoError { err: e })?;

            // Download header
            match self
                .client
                .read()
                .await
                .download_ts(&full_header_url, &format!("{}/{}", work_dir, file_name))
                .await
            {
                Ok(size) => {
                    if size == 0 {
                        log::error!(
                            "[{}]Download header failed: {}",
                            self.room_id,
                            full_header_url
                        );
                        // Clean up empty directory since header download failed
                        if let Err(cleanup_err) = tokio::fs::remove_dir_all(&work_dir).await {
                            log::warn!(
                                "[{}]Failed to cleanup empty work directory {}: {}",
                                self.room_id,
                                work_dir,
                                cleanup_err
                            );
                        }
                        return Err(super::errors::RecorderError::InvalidStream {
                            stream: current_stream,
                        });
                    }
                    header.size = size;

                    if self.cover.read().await.is_some() {
                        let current_cover_full_path = Path::new(&self.config.read().await.cache)
                            .join(self.cover.read().await.clone().unwrap());
                        // copy current cover to work_dir
                        let _ = tokio::fs::copy(
                            current_cover_full_path,
                            &format!("{}/{}", work_dir, "cover.jpg"),
                        )
                        .await;
                    }

                    // Now that download succeeded, create the record and setup stores
                    self.db
                        .add_record(
                            PlatformType::BiliBili,
                            timestamp.to_string().as_str(),
                            self.room_id,
                            &self.room_info.read().await.room_title,
                            format!(
                                "{}/{}/{}/{}",
                                PlatformType::BiliBili.as_str(),
                                self.room_id,
                                timestamp,
                                "cover.jpg"
                            )
                            .into(),
                            None,
                        )
                        .await?;

                    let entry_store = EntryStore::new(&work_dir).await;
                    *self.entry_store.write().await = Some(entry_store);

                    // danmu file
                    let danmu_file_path = format!("{}{}", work_dir, "danmu.txt");
                    *self.danmu_storage.write().await = DanmuStorage::new(&danmu_file_path).await;

                    self.entry_store
                        .write()
                        .await
                        .as_mut()
                        .unwrap()
                        .add_entry(header)
                        .await;

                    let new_resolution = self.get_resolution(&full_header_url).await?;

                    log::info!(
                        "[{}] Initial header resolution: {} {}",
                        self.room_id,
                        header_url,
                        new_resolution
                    );

                    *self.current_header_info.write().await = Some(HeaderInfo {
                        url: header_url.clone(),
                        resolution: new_resolution,
                    });

                    let _ = self.event_channel.send(RecorderEvent::RecordStart {
                        recorder: self.info().await,
                    });
                }
                Err(e) => {
                    log::error!("[{}]Download header failed: {}", self.room_id, e);
                    // Clean up empty directory since header download failed
                    if let Err(cleanup_err) = tokio::fs::remove_dir_all(&work_dir).await {
                        log::warn!(
                            "[{}]Failed to cleanup empty work directory {}: {}",
                            self.room_id,
                            work_dir,
                            cleanup_err
                        );
                    }
                    return Err(e.into());
                }
            }
        } else {
            work_dir = self.get_work_dir(timestamp.to_string().as_str()).await;
            // For non-FMP4 streams, check if we need to initialize
            if self.entry_store.read().await.as_ref().is_none() {
                timestamp = Utc::now().timestamp_millis();
                *self.live_id.write().await = timestamp.to_string();
                work_dir = self.get_work_dir(timestamp.to_string().as_str()).await;
                is_first_record = true;
            }
        }

        // check resolution change
        let current_header_info = self.current_header_info.read().await.clone();
        if current_header_info.is_some() {
            let current_header_info = current_header_info.unwrap();
            if current_header_info.url != header_url {
                let new_resolution = self.get_resolution(&full_header_url).await?;
                log::debug!(
                    "[{}] Header url changed: {} => {}, resolution: {} => {}",
                    self.room_id,
                    current_header_info.url,
                    header_url,
                    current_header_info.resolution,
                    new_resolution
                );
                if current_header_info.resolution != new_resolution {
                    self.reset().await;

                    return Err(super::errors::RecorderError::ResolutionChanged {
                        err: format!(
                            "Resolution changed: {} => {}",
                            current_header_info.resolution, new_resolution
                        ),
                    });
                }

                // update current header info
                *self.current_header_info.write().await = Some(HeaderInfo {
                    url: header_url,
                    resolution: new_resolution,
                });
            }
        }

        match playlist {
            Playlist::MasterPlaylist(pl) => {
                log::debug!("[{}]Master playlist:\n{:?}", self.room_id, pl)
            }
            Playlist::MediaPlaylist(pl) => {
                let mut new_segment_fetched = false;
                let last_sequence = self
                    .entry_store
                    .read()
                    .await
                    .as_ref()
                    .map(|store| store.last_sequence)
                    .unwrap_or(0); // For first-time recording, start from 0

                // Parse BILI-AUX offsets to calculate precise durations for FMP4
                let mut segment_offsets = Vec::new();
                for ts in pl.segments.iter() {
                    let mut seg_offset: i64 = 0;
                    for tag in &ts.unknown_tags {
                        if tag.tag == "BILI-AUX" {
                            if let Some(rest) = &tag.rest {
                                let parts: Vec<&str> = rest.split('|').collect();
                                if !parts.is_empty() {
                                    let offset_hex = parts.first().unwrap();
                                    if let Ok(offset) = i64::from_str_radix(offset_hex, 16) {
                                        seg_offset = offset;
                                    }
                                }
                            }
                            break;
                        }
                    }
                    segment_offsets.push(seg_offset);
                }

                // Extract stream start timestamp from header if available for FMP4
                let stream_start_timestamp = self.room_info.read().await.live_start_time;

                // Get the last segment offset from previous processing
                let mut last_offset = *self.last_segment_offset.read().await;

                for (i, ts) in pl.segments.iter().enumerate() {
                    let sequence = pl.media_sequence + i as u64;
                    if sequence <= last_sequence {
                        continue;
                    }

                    let ts_url = current_stream.ts_url(&ts.uri);
                    if Url::parse(&ts_url).is_err() {
                        log::error!(
                            "[{}]Ts url is invalid. ts_url={} original={}",
                            self.room_id,
                            ts_url,
                            ts.uri
                        );
                        continue;
                    }

                    // Calculate precise timestamp from stream start + BILI-AUX offset for FMP4
                    let ts_mili = if current_stream.format == StreamType::FMP4
                        && stream_start_timestamp > 0
                        && i < segment_offsets.len()
                    {
                        let seg_offset = segment_offsets[i];

                        stream_start_timestamp * 1000 + seg_offset
                    } else {
                        // Fallback to current time if parsing fails or not FMP4
                        Utc::now().timestamp_millis()
                    };

                    // encode segment offset into filename
                    let file_name = ts.uri.split('/').next_back().unwrap_or(&ts.uri);
                    let ts_length = pl.target_duration as f64;

                    // Calculate precise duration from BILI-AUX offsets for FMP4
                    let precise_length_from_aux =
                        if current_stream.format == StreamType::FMP4 && i < segment_offsets.len() {
                            let current_offset = segment_offsets[i];

                            // Get the previous offset for duration calculation
                            let prev_offset = if i > 0 {
                                // Use previous segment in current M3U8
                                Some(segment_offsets[i - 1])
                            } else {
                                // Use saved last offset from previous M3U8 processing
                                last_offset
                            };

                            if let Some(prev) = prev_offset {
                                let duration_ms = current_offset - prev;
                                if duration_ms > 0 {
                                    Some(duration_ms as f64 / 1000.0) // Convert ms to seconds
                                } else {
                                    None
                                }
                            } else {
                                // No previous offset available, use target duration
                                None
                            }
                        } else {
                            None
                        };
                    let client = self.client.clone();
                    let mut retry = 0;
                    let mut work_dir_created_for_non_fmp4 = false;

                    // For non-FMP4 streams, create record on first successful ts download
                    if is_first_record && current_stream.format != StreamType::FMP4 {
                        // Create work directory before first ts download
                        tokio::fs::create_dir_all(&work_dir)
                            .await
                            .map_err(|e| super::errors::RecorderError::IoError { err: e })?;
                        work_dir_created_for_non_fmp4 = true;
                    }

                    loop {
                        if retry > 3 {
                            log::error!("[{}]Download ts failed after retry", self.room_id);

                            // Clean up empty directory if first ts download failed for non-FMP4
                            if is_first_record
                                && current_stream.format != StreamType::FMP4
                                && work_dir_created_for_non_fmp4
                            {
                                if let Err(cleanup_err) = tokio::fs::remove_dir_all(&work_dir).await
                                {
                                    log::warn!(
                                        "[{}]Failed to cleanup empty work directory {}: {}",
                                        self.room_id,
                                        work_dir,
                                        cleanup_err
                                    );
                                }
                            }

                            break;
                        }
                        match client
                            .read()
                            .await
                            .download_ts(&ts_url, &format!("{}/{}", work_dir, file_name))
                            .await
                        {
                            Ok(size) => {
                                if size == 0 {
                                    log::error!(
                                        "[{}]Segment with size 0, stream might be corrupted",
                                        self.room_id
                                    );

                                    // Clean up empty directory if first ts download failed for non-FMP4
                                    if is_first_record
                                        && current_stream.format != StreamType::FMP4
                                        && work_dir_created_for_non_fmp4
                                    {
                                        if let Err(cleanup_err) =
                                            tokio::fs::remove_dir_all(&work_dir).await
                                        {
                                            log::warn!(
                                                "[{}]Failed to cleanup empty work directory {}: {}",
                                                self.room_id,
                                                work_dir,
                                                cleanup_err
                                            );
                                        }
                                    }

                                    return Err(super::errors::RecorderError::InvalidStream {
                                        stream: current_stream,
                                    });
                                }

                                // Create record and setup stores on first successful download for non-FMP4
                                if is_first_record && current_stream.format != StreamType::FMP4 {
                                    self.db
                                        .add_record(
                                            PlatformType::BiliBili,
                                            timestamp.to_string().as_str(),
                                            self.room_id,
                                            &self.room_info.read().await.room_title,
                                            self.cover.read().await.clone(),
                                            None,
                                        )
                                        .await?;

                                    let entry_store = EntryStore::new(&work_dir).await;
                                    *self.entry_store.write().await = Some(entry_store);

                                    // danmu file
                                    let danmu_file_path = format!("{}{}", work_dir, "danmu.txt");
                                    *self.danmu_storage.write().await =
                                        DanmuStorage::new(&danmu_file_path).await;

                                    is_first_record = false;
                                }

                                // Get precise duration - prioritize BILI-AUX for FMP4, fallback to ffprobe if needed
                                let precise_length = if let Some(aux_duration) =
                                    precise_length_from_aux
                                {
                                    aux_duration
                                } else if current_stream.format != StreamType::FMP4 {
                                    // For regular TS segments, use direct ffprobe
                                    let file_path = format!("{}/{}", work_dir, file_name);
                                    match crate::ffmpeg::get_segment_duration(std::path::Path::new(
                                        &file_path,
                                    ))
                                    .await
                                    {
                                        Ok(duration) => {
                                            log::debug!(
                                                "[{}]Precise TS segment duration: {}s (original: {}s)",
                                                self.room_id,
                                                duration,
                                                ts_length
                                            );
                                            duration
                                        }
                                        Err(e) => {
                                            log::warn!(
                                                "[{}]Failed to get precise TS duration for {}: {}, using fallback",
                                                self.room_id,
                                                file_name,
                                                e
                                            );
                                            ts_length
                                        }
                                    }
                                } else {
                                    // FMP4 segment without BILI-AUX info, use fallback
                                    log::debug!(
                                        "[{}]No BILI-AUX data available for FMP4 segment {}, using target duration",
                                        self.room_id,
                                        file_name
                                    );
                                    ts_length
                                };

                                self.entry_store
                                    .write()
                                    .await
                                    .as_mut()
                                    .unwrap()
                                    .add_entry(TsEntry {
                                        url: file_name.into(),
                                        sequence,
                                        length: precise_length,
                                        size,
                                        ts: ts_mili,
                                        is_header: false,
                                    })
                                    .await;

                                // Update last offset for next segment calculation
                                if current_stream.format == StreamType::FMP4
                                    && i < segment_offsets.len()
                                {
                                    last_offset = Some(segment_offsets[i]);
                                }

                                new_segment_fetched = true;
                                break;
                            }
                            Err(e) => {
                                retry += 1;
                                log::warn!(
                                    "[{}]Download ts failed, retry {}: {}",
                                    self.room_id,
                                    retry,
                                    e
                                );
                                log::warn!("[{}]file_name: {}", self.room_id, file_name);
                                log::warn!("[{}]ts_url: {}", self.room_id, ts_url);

                                // If this is the last retry and it's the first record for non-FMP4, clean up
                                if retry > 3
                                    && is_first_record
                                    && current_stream.format != StreamType::FMP4
                                    && work_dir_created_for_non_fmp4
                                {
                                    if let Err(cleanup_err) =
                                        tokio::fs::remove_dir_all(&work_dir).await
                                    {
                                        log::warn!(
                                            "[{}]Failed to cleanup empty work directory {}: {}",
                                            self.room_id,
                                            work_dir,
                                            cleanup_err
                                        );
                                    }
                                }
                            }
                        }
                    }
                }

                if new_segment_fetched {
                    *self.last_update.write().await = Utc::now().timestamp();

                    // Save the last offset for next M3U8 processing
                    if current_stream.format == StreamType::FMP4 {
                        *self.last_segment_offset.write().await = last_offset;
                    }

                    self.db
                        .update_record(
                            timestamp.to_string().as_str(),
                            self.entry_store
                                .read()
                                .await
                                .as_ref()
                                .unwrap()
                                .total_duration() as i64,
                            self.entry_store.read().await.as_ref().unwrap().total_size(),
                        )
                        .await?;
                } else {
                    // if index content is not changed for a long time, we should return a error to fetch a new stream
                    if *self.last_update.read().await < Utc::now().timestamp() - 10 {
                        log::error!(
                            "[{}]Stream content is not updating for 10s, maybe not started yet or not closed properly.",
                            self.room_id
                        );
                        return Err(super::errors::RecorderError::FreezedStream {
                            stream: current_stream,
                        });
                    }
                }
                // check the current stream is too slow or not
                if let Some(entry_store) = self.entry_store.read().await.as_ref() {
                    if let Some(last_ts) = entry_store.last_ts() {
                        if last_ts < Utc::now().timestamp() - 10 {
                            log::error!(
                                "[{}]Stream is too slow, last entry ts is at {}",
                                self.room_id,
                                last_ts
                            );
                            return Err(super::errors::RecorderError::SlowStream {
                                stream: current_stream,
                            });
                        }
                    }
                }
            }
        }

        // check stream is nearly expired
        // WHY: when program started, all stream is fetched nearly at the same time, so they will expire toggether,
        // this might meet server rate limit. So we add a random offset to make request spread over time.
        let pre_offset = rand::random::<u64>() % 181 + 120; // Random number between 120 and 300
                                                            // no need to update stream as it's not expired yet
        let current_stream = self.live_stream.read().await.clone();
        if current_stream
            .as_ref()
            .is_some_and(|s| s.expire - Utc::now().timestamp() < pre_offset as i64)
        {
            log::info!("[{}]Stream is nearly expired", self.room_id);
            return Err(super::errors::RecorderError::StreamExpired {
                stream: current_stream.unwrap(),
            });
        }

        Ok(task_begin_time.elapsed().as_millis())
    }

    async fn generate_archive_m3u8(&self, live_id: &str, start: i64, end: i64) -> String {
        let work_dir = self.get_work_dir(live_id).await;
        let entry_store = EntryStore::new(&work_dir).await;
        let mut range = None;
        if start != 0 || end != 0 {
            range = Some(Range {
                x: start as f32,
                y: end as f32,
            })
        }

        entry_store.manifest(true, true, range)
    }

    /// if fetching live/last stream m3u8, all entries are cached in memory, so it will be much faster than read_dir
    async fn generate_live_m3u8(&self, start: i64, end: i64) -> String {
        let live_status = *self.live_status.read().await;
        let range = if start != 0 || end != 0 {
            Some(Range {
                x: start as f32,
                y: end as f32,
            })
        } else {
            None
        };

        if let Some(entry_store) = self.entry_store.read().await.as_ref() {
            entry_store.manifest(!live_status || range.is_some(), true, range)
        } else {
            // Return empty manifest if entry_store is not initialized yet
            "#EXTM3U\n#EXT-X-VERSION:3\n".to_string()
        }
    }
}

#[async_trait]
impl super::Recorder for BiliRecorder {
    async fn run(&self) {
        let self_clone = self.clone();
        *self.danmu_task.lock().await = Some(tokio::spawn(async move {
            log::info!("[{}]Start fetching danmu", self_clone.room_id);
            let _ = self_clone.danmu().await;
        }));

        let self_clone = self.clone();
        *self.record_task.lock().await = Some(tokio::spawn(async move {
            log::info!("[{}]Start running recorder", self_clone.room_id);
            while !*self_clone.quit.lock().await {
                let mut connection_fail_count = 0;
                if self_clone.check_status().await {
                    // Live status is ok, start recording.
                    let mut continue_record = false;
                    while self_clone.should_record().await {
                        match self_clone.update_entries().await {
                            Ok(ms) => {
                                if ms < 1000 {
                                    tokio::time::sleep(Duration::from_millis((1000 - ms) as u64))
                                        .await;
                                }
                                if ms >= 3000 {
                                    log::warn!(
                                        "[{}]Update entries cost too long: {}ms",
                                        self_clone.room_id,
                                        ms
                                    );
                                }
                                *self_clone.is_recording.write().await = true;
                                connection_fail_count = 0;
                            }
                            Err(e) => {
                                log::error!("[{}]Update entries error: {}", self_clone.room_id, e);
                                if let RecorderError::BiliClientError { err: _ } = e {
                                    connection_fail_count =
                                        std::cmp::min(5, connection_fail_count + 1);
                                }
                                // if error is stream expired, we should not break, cuz we need to fetch a new stream
                                if let RecorderError::StreamExpired { stream: _ } = e {
                                    continue_record = true;
                                }
                                break;
                            }
                        }
                    }

                    if continue_record {
                        log::info!("[{}]Continue recording without reset", self_clone.room_id);
                        continue;
                    }

                    // whatever error happened during update entries, reset to start another recording.
                    if self_clone.current_header_info.read().await.is_some() {
                        let _ = self_clone.event_channel.send(RecorderEvent::RecordEnd {
                            recorder: self_clone.info().await,
                        });
                    }
                    *self_clone.is_recording.write().await = false;
                    self_clone.reset().await;
                    // go check status again after random 2-5 secs
                    let secs = rand::random::<u64>() % 4 + 2;
                    tokio::time::sleep(Duration::from_secs(
                        secs + 2_u64.pow(connection_fail_count),
                    ))
                    .await;
                    continue;
                }

                let interval = self_clone.config.read().await.status_check_interval;
                tokio::time::sleep(Duration::from_secs(interval)).await;
            }
        }));
    }

    async fn stop(&self) {
        log::debug!("[{}]Stop recorder", self.room_id);
        *self.quit.lock().await = true;
        if let Some(danmu_task) = self.danmu_task.lock().await.as_mut() {
            let _ = danmu_task.abort();
        }
        if let Some(record_task) = self.record_task.lock().await.as_mut() {
            let _ = record_task.abort();
        }
        log::info!("[{}]Recorder quit.", self.room_id);
    }

    /// timestamp is the id of live stream
    async fn m3u8_content(&self, live_id: &str, start: i64, end: i64) -> String {
        if *self.live_id.read().await == live_id && self.should_record().await {
            self.generate_live_m3u8(start, end).await
        } else {
            self.generate_archive_m3u8(live_id, start, end).await
        }
    }

    async fn master_m3u8(&self, live_id: &str, start: i64, end: i64) -> String {
        log::info!(
            "[{}]Master manifest for {live_id} {start}-{end}",
            self.room_id
        );
        let offset = self.first_segment_ts(live_id).await / 1000;
        let mut m3u8_content = "#EXTM3U\n".to_string();
        m3u8_content += "#EXT-X-VERSION:6\n";
        m3u8_content += &format!(
            "#EXT-X-STREAM-INF:BANDWIDTH=1280000,RESOLUTION=1920x1080,CODECS={},DANMU={}\n",
            "\"avc1.64001F,mp4a.40.2\"", offset
        );
        m3u8_content += &format!("playlist.m3u8?start={}&end={}\n", start, end);
        m3u8_content
    }

    async fn first_segment_ts(&self, live_id: &str) -> i64 {
        if *self.live_id.read().await == live_id {
            let entry_store = self.entry_store.read().await;
            if entry_store.is_some() {
                entry_store.as_ref().unwrap().first_ts().unwrap_or(0)
            } else {
                0
            }
        } else {
            let work_dir = self.get_work_dir(live_id).await;
            EntryStore::new(&work_dir).await.first_ts().unwrap_or(0)
        }
    }

    async fn info(&self) -> super::RecorderInfo {
        let room_info = self.room_info.read().await;
        let user_info = self.user_info.read().await;
        super::RecorderInfo {
            room_id: self.room_id,
            room_info: super::RoomInfo {
                room_id: self.room_id,
                room_title: room_info.room_title.clone(),
                room_cover: room_info.room_cover_url.clone(),
            },
            user_info: super::UserInfo {
                user_id: user_info.user_id.to_string(),
                user_name: user_info.user_name.clone(),
                user_avatar: user_info.user_avatar_url.clone(),
            },
            total_length: if let Some(store) = self.entry_store.read().await.as_ref() {
                store.total_duration()
            } else {
                0.0
            },
            current_live_id: self.live_id.read().await.clone(),
            live_status: *self.live_status.read().await,
            is_recording: *self.is_recording.read().await,
            auto_start: *self.enabled.read().await,
            platform: PlatformType::BiliBili.as_str().to_string(),
        }
    }

    async fn comments(
        &self,
        live_id: &str,
    ) -> Result<Vec<DanmuEntry>, super::errors::RecorderError> {
        Ok(if live_id == *self.live_id.read().await {
            // just return current cache content
            match self.danmu_storage.read().await.as_ref() {
                Some(storage) => {
                    storage
                        .get_entries(self.first_segment_ts(live_id).await)
                        .await
                }
                None => Vec::new(),
            }
        } else {
            // load disk cache
            let cache_file_path = format!(
                "{}/bilibili/{}/{}/{}",
                self.config.read().await.cache,
                self.room_id,
                live_id,
                "danmu.txt"
            );
            log::debug!(
                "[{}]loading danmu cache from {}",
                self.room_id,
                cache_file_path
            );
            let storage = DanmuStorage::new(&cache_file_path).await;
            if storage.is_none() {
                return Ok(Vec::new());
            }
            let storage = storage.unwrap();
            storage
                .get_entries(self.first_segment_ts(live_id).await)
                .await
        })
    }

    async fn is_recording(&self, live_id: &str) -> bool {
        *self.live_id.read().await == live_id && *self.live_status.read().await
    }

    async fn get_archive_subtitle(
        &self,
        live_id: &str,
    ) -> Result<String, super::errors::RecorderError> {
        // read subtitle file under work_dir
        let work_dir = self.get_work_dir(live_id).await;
        let subtitle_file_path = format!("{}/{}", work_dir, "subtitle.srt");
        let subtitle_file = File::open(subtitle_file_path).await;
        if subtitle_file.is_err() {
            return Err(super::errors::RecorderError::SubtitleNotFound {
                live_id: live_id.to_string(),
            });
        }
        let subtitle_file = subtitle_file.unwrap();
        let mut subtitle_file = BufReader::new(subtitle_file);
        let mut subtitle_content = String::new();
        subtitle_file.read_to_string(&mut subtitle_content).await?;
        Ok(subtitle_content)
    }

    async fn generate_archive_subtitle(
        &self,
        live_id: &str,
    ) -> Result<String, super::errors::RecorderError> {
        // generate subtitle file under work_dir
        let work_dir = self.get_work_dir(live_id).await;
        let subtitle_file_path = format!("{}/{}", work_dir, "subtitle.srt");
        let mut subtitle_file = File::create(subtitle_file_path).await?;
        // first generate a tmp clip file
        // generate a tmp m3u8 index file
        let m3u8_index_file_path = format!("{}/{}", work_dir, "tmp.m3u8");
        let m3u8_content = self.m3u8_content(live_id, 0, 0).await;
        tokio::fs::write(&m3u8_index_file_path, m3u8_content).await?;
        log::info!(
            "[{}]M3U8 index file generated: {}",
            self.room_id,
            m3u8_index_file_path
        );
        // generate a tmp clip file
        let clip_file_path = format!("{}/{}", work_dir, "tmp.mp4");
        if let Err(e) = crate::ffmpeg::clip_from_m3u8(
            None::<&crate::progress_reporter::ProgressReporter>,
            Path::new(&m3u8_index_file_path),
            Path::new(&clip_file_path),
            None,
            false,
        )
        .await
        {
            return Err(super::errors::RecorderError::SubtitleGenerationFailed {
                error: e.to_string(),
            });
        }
        log::info!(
            "[{}]Temp clip file generated: {}",
            self.room_id,
            clip_file_path
        );
        // generate subtitle file
        let config = self.config.read().await;
        let result = crate::ffmpeg::generate_video_subtitle(
            None,
            Path::new(&clip_file_path),
            "whisper",
            &config.whisper_model,
            &config.whisper_prompt,
            &config.openai_api_key,
            &config.openai_api_endpoint,
            &config.whisper_language,
        )
        .await;
        // write subtitle file
        if let Err(e) = result {
            return Err(super::errors::RecorderError::SubtitleGenerationFailed {
                error: e.to_string(),
            });
        }
        log::info!("[{}]Subtitle generated", self.room_id);
        let result = result.unwrap();
        let subtitle_content = result
            .subtitle_content
            .iter()
            .map(item_to_srt)
            .collect::<Vec<String>>()
            .join("");
        subtitle_file.write_all(subtitle_content.as_bytes()).await?;
        log::info!("[{}]Subtitle file written", self.room_id);
        // remove tmp file
        tokio::fs::remove_file(&m3u8_index_file_path).await?;
        tokio::fs::remove_file(&clip_file_path).await?;
        log::info!("[{}]Tmp file removed", self.room_id);
        Ok(subtitle_content)
    }

    async fn enable(&self) {
        *self.enabled.write().await = true;
    }

    async fn disable(&self) {
        *self.enabled.write().await = false;
    }
}
