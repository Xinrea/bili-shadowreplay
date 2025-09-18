pub mod client;
pub mod errors;
pub mod profile;
pub mod response;
use super::entry::{EntryStore, Range};
use super::errors::RecorderError;
use super::PlatformType;
use crate::database::account::AccountRow;
use crate::ffmpeg::get_video_resolution;
use crate::progress::progress_manager::Event;
use crate::progress::progress_reporter::EventEmitter;
use crate::recorder::bilibili::client::{Codec, Protocol, Qn};
use crate::recorder::Recorder;
use crate::recorder_manager::RecorderEvent;
use crate::subtitle_generator::item_to_srt;

use super::danmu::{DanmuEntry, DanmuStorage};
use super::entry::TsEntry;
use chrono::Utc;
use client::{BiliClient, BiliStream, Format, RoomInfo, UserInfo};
use danmu_stream::danmu_stream::DanmuStream;
use danmu_stream::provider::ProviderType;
use danmu_stream::DanmuMessageType;
use m3u8_rs::Playlist;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{broadcast, Mutex, RwLock};
use tokio::task::JoinHandle;
use url::Url;

use crate::config::Config;
use crate::database::Database;

use async_trait::async_trait;

#[cfg(feature = "gui")]
use {tauri::AppHandle, tauri_plugin_notification::NotificationExt};

/// A recorder for `BiliBili` live streams
///
/// This recorder fetches, caches and serves TS entries, currently supporting only `StreamType::FMP4`.
/// As high-quality streams are accessible only to logged-in users, the use of a `BiliClient`, which manages cookies, is required.
#[derive(Clone)]
pub struct BiliRecorder {
    #[cfg(feature = "gui")]
    app_handle: AppHandle,
    emitter: EventEmitter,
    client: Arc<RwLock<BiliClient>>,
    db: Arc<Database>,
    account: AccountRow,
    config: Arc<RwLock<Config>>,
    room_id: i64,
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
    current_resolution: Arc<RwLock<Option<String>>>,

    danmu_task: Arc<Mutex<Option<JoinHandle<()>>>>,
    record_task: Arc<Mutex<Option<JoinHandle<()>>>>,
}

pub struct BiliRecorderOptions {
    #[cfg(feature = "gui")]
    pub app_handle: AppHandle,
    pub emitter: EventEmitter,
    pub db: Arc<Database>,
    pub room_id: i64,
    pub account: AccountRow,
    pub config: Arc<RwLock<Config>>,
    pub auto_start: bool,
    pub channel: broadcast::Sender<RecorderEvent>,
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
            danmu_task: Arc::new(Mutex::new(None)),
            record_task: Arc::new(Mutex::new(None)),
            current_resolution: Arc::new(RwLock::new(None)),
        };
        log::info!("Recorder for room {} created.", options.room_id);
        Ok(recorder)
    }

    pub async fn reset(&self) {
        // if record is ended, send event
        if !self.live_id.read().await.is_empty() && self.current_resolution.read().await.is_some() {
            let _ = self.event_channel.send(RecorderEvent::RecordEnd {
                recorder: self.info().await,
            });
        }
        // if record is empty, remove record
        if !self.live_id.read().await.is_empty() && self.current_resolution.read().await.is_none() {
            // no entries, remove work dir
            log::warn!("[{}]No entries, remove empty record", self.room_id);
            *self.danmu_storage.write().await = None;
            if let Err(e) = self
                .db
                .remove_record(self.live_id.read().await.as_str())
                .await
            {
                log::warn!("[{}]Failed to remove empty record: {}", self.room_id, e);
            }
            let work_dir = self.get_work_dir(self.live_id.read().await.as_str()).await;

            if let Err(e) = tokio::fs::remove_dir_all(self.get_full_path(&work_dir).await).await {
                log::warn!("[{}]Failed to remove empty work dir: {}", self.room_id, e);
            }
        }
        *self.entry_store.write().await = None;
        *self.live_stream.write().await = None;
        *self.last_update.write().await = Utc::now().timestamp();
        *self.danmu_storage.write().await = None;
        *self.current_resolution.write().await = None;
        *self.live_id.write().await = String::new();
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
                let new_stream = self
                    .client
                    .read()
                    .await
                    .get_stream_info(
                        &self.account,
                        self.room_id,
                        Protocol::HttpHls,
                        Format::TS,
                        Codec::Avc,
                        Qn::Q4K,
                    )
                    .await;

                if new_stream.is_err() {
                    log::error!(
                        "[{}]Fetch stream failed: {}",
                        self.room_id,
                        new_stream.err().unwrap()
                    );
                    return true;
                }

                let new_stream = new_stream.unwrap();
                *self.live_stream.write().await = Some(new_stream.clone());
                *self.last_update.write().await = Utc::now().timestamp();

                log::info!(
                    "[{}]Update to a new stream: {:?} => {}",
                    self.room_id,
                    self.live_stream.read().await.clone(),
                    new_stream
                );

                true
            }
            Err(e) => {
                log::error!("[{}]Update room status failed: {}", self.room_id, e);
                // may encounter internet issues, not sure whether the stream is closed or started, just remain
                *self.live_status.read().await
            }
        }
    }

    async fn danmu(&self) -> Result<(), super::errors::RecorderError> {
        let cookies = self.account.cookies.clone();
        let room_id = self.room_id;
        let danmu_stream = DanmuStream::new(ProviderType::BiliBili, &cookies, room_id).await;
        if danmu_stream.is_err() {
            let err = danmu_stream.err().unwrap();
            log::error!("[{}]Failed to create danmu stream: {}", self.room_id, err);
            return Err(super::errors::RecorderError::DanmuStreamError(err));
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
                return Err(super::errors::RecorderError::DanmuStreamError(
                    danmu_stream::DanmuStreamError::WebsocketError {
                        err: "Failed to receive danmu message".to_string(),
                    },
                ));
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
                Err(super::errors::RecorderError::BiliClientError(e))
            }
        }
    }

    async fn get_resolution(&self, ts_path: &str) -> Result<String, super::errors::RecorderError> {
        let resolution = get_video_resolution(ts_path)
            .await
            .map_err(super::errors::RecorderError::FfmpegError)?;
        Ok(resolution)
    }

    async fn get_full_path(&self, relative_path: &str) -> String {
        format!("{}/{}", self.config.read().await.cache, relative_path)
    }

    async fn get_work_dir(&self, live_id: &str) -> String {
        format!("bilibili/{}/{}/", self.room_id, live_id)
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

        let mut timestamp = Utc::now().timestamp_millis();
        if !self.live_id.read().await.is_empty() {
            timestamp = self
                .live_id
                .read()
                .await
                .parse::<i64>()
                .unwrap_or(timestamp);
        }

        let work_dir = self.get_work_dir(&timestamp.to_string()).await;
        let is_first_record = self.entry_store.read().await.is_none();

        if is_first_record {
            log::info!("[{}]New record started: {}", self.room_id, timestamp);
            *self.live_id.write().await = timestamp.to_string();
            tokio::fs::create_dir_all(self.get_full_path(&work_dir).await)
                .await
                .map_err(super::errors::RecorderError::IoError)?;

            *self.entry_store.write().await =
                Some(EntryStore::new(&self.get_full_path(&work_dir).await).await);

            let danmu_path = format!("{work_dir}/danmu.txt");
            let danmu_full_path = self.get_full_path(&danmu_path).await;
            *self.danmu_storage.write().await = DanmuStorage::new(&danmu_full_path).await;

            let cover_path = format!("{work_dir}/cover.jpg");
            let cover_full_path = self.get_full_path(&cover_path).await;

            let room_cover = self.cover.read().await.clone().unwrap();
            let room_cover_full_path = format!("{}/{}", self.config.read().await.cache, room_cover);
            log::debug!(
                "[{}]Copy cover to: {} {}",
                self.room_id,
                room_cover_full_path,
                cover_full_path
            );
            tokio::fs::copy(room_cover_full_path, &cover_full_path)
                .await
                .map_err(super::errors::RecorderError::IoError)?;

            self.db
                .add_record(
                    PlatformType::BiliBili,
                    timestamp.to_string().as_str(),
                    self.room_id,
                    &self.room_info.read().await.room_title,
                    Some(cover_path),
                    None,
                )
                .await?;
        }

        match playlist {
            Playlist::MasterPlaylist(pl) => {
                log::debug!("[{}]Master playlist:\n{:?}", self.room_id, pl);
            }
            Playlist::MediaPlaylist(pl) => {
                let mut new_segment_fetched = false;
                let last_sequence = self
                    .entry_store
                    .read()
                    .await
                    .as_ref()
                    .map_or(0, |store| store.last_sequence); // For first-time recording, start from 0

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
                    let ts_mili = ts
                        .program_date_time
                        .map(|dt| dt.timestamp_millis())
                        .unwrap_or_else(|| Utc::now().timestamp_millis());

                    // encode segment offset into filename
                    let file_name = ts.uri.split('/').next_back().unwrap_or(&ts.uri);
                    let ts_length = f64::from(pl.target_duration);

                    let client = self.client.clone();
                    let mut retry = 0;

                    loop {
                        if retry > 3 {
                            log::error!("[{}]Download ts failed after retry", self.room_id);

                            break;
                        }
                        let full_path =
                            self.get_full_path(&format!("{work_dir}/{file_name}")).await;
                        match client.read().await.download_ts(&ts_url, &full_path).await {
                            Ok(size) => {
                                if size == 0 {
                                    log::error!(
                                        "[{}]Segment with size 0, stream might be corrupted",
                                        self.room_id
                                    );

                                    return Err(super::errors::RecorderError::InvalidStream {
                                        stream: current_stream,
                                    });
                                }

                                let resolution_result = self.get_resolution(&full_path).await;
                                if resolution_result.is_err() {
                                    return Err(resolution_result.err().unwrap());
                                }
                                let resolution = resolution_result.unwrap();
                                let current_resolution =
                                    self.current_resolution.read().await.clone();
                                if let Some(current_resolution) = current_resolution {
                                    if current_resolution != resolution {
                                        log::warn!(
                                            "[{}]Resolution changed: {} => {}",
                                            self.room_id,
                                            current_resolution,
                                            resolution
                                        );
                                        return Err(
                                            super::errors::RecorderError::ResolutionChanged {
                                                err: format!(
                                                    "Resolution changed: {} => {}",
                                                    current_resolution, resolution
                                                ),
                                            },
                                        );
                                    }
                                } else {
                                    // first segment, set current resolution
                                    *self.current_resolution.write().await = Some(resolution);

                                    let _ = self.event_channel.send(RecorderEvent::RecordStart {
                                        recorder: self.info().await,
                                    });
                                }

                                self.entry_store
                                    .write()
                                    .await
                                    .as_mut()
                                    .unwrap()
                                    .add_entry(TsEntry {
                                        url: file_name.into(),
                                        sequence,
                                        length: ts_length,
                                        size,
                                        ts: ts_mili,
                                        is_header: false,
                                    })
                                    .await;

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
                            }
                        }
                    }
                }

                if new_segment_fetched {
                    *self.last_update.write().await = Utc::now().timestamp();

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
        if current_stream.as_ref().is_some_and(|s| {
            s.get_expire().unwrap_or(0) - Utc::now().timestamp() < pre_offset as i64
        }) {
            log::info!("[{}]Stream is nearly expired", self.room_id);
            return Err(super::errors::RecorderError::StreamExpired {
                stream: current_stream.unwrap(),
            });
        }

        Ok(task_begin_time.elapsed().as_millis())
    }

    async fn generate_archive_m3u8(&self, live_id: &str, start: i64, end: i64) -> String {
        let work_dir = self.get_work_dir(live_id).await;
        let entry_store = EntryStore::new(&self.get_full_path(&work_dir).await).await;
        let mut range = None;
        if start != 0 || end != 0 {
            range = Some(Range {
                x: start as f32,
                y: end as f32,
            });
        }

        entry_store.manifest(true, true, range)
    }

    /// if fetching live/last stream m3u8, all entries are cached in memory, so it will be much faster than `read_dir`
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
                    let mut resolution_changed = false;
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
                                if let RecorderError::BiliClientError(_) = e {
                                    connection_fail_count =
                                        std::cmp::min(5, connection_fail_count + 1);
                                }
                                // if error is stream expired, we should not break, cuz we need to fetch a new stream
                                if let RecorderError::StreamExpired { stream: _ } = e {
                                    continue_record = true;
                                }

                                if let RecorderError::ResolutionChanged { err: _ } = e {
                                    resolution_changed = true;
                                }

                                break;
                            }
                        }
                    }

                    if continue_record {
                        log::info!("[{}]Continue recording without reset", self_clone.room_id);
                        continue;
                    }

                    *self_clone.is_recording.write().await = false;
                    self_clone.reset().await;
                    if resolution_changed {
                        log::info!("[{}]Resolution changed, reset recorder", self_clone.room_id);
                        continue;
                    }
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
            let () = danmu_task.abort();
        }
        if let Some(record_task) = self.record_task.lock().await.as_mut() {
            let () = record_task.abort();
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
        m3u8_content += &format!("playlist.m3u8?start={start}&end={end}\n");
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
            EntryStore::new(&self.get_full_path(&work_dir).await)
                .await
                .first_ts()
                .unwrap_or(0)
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
        let subtitle_file = File::open(self.get_full_path(&subtitle_file_path).await).await;
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
        let mut subtitle_file = File::create(self.get_full_path(&subtitle_file_path).await).await?;
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
            None::<&crate::progress::progress_reporter::ProgressReporter>,
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
            .collect::<String>();
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
