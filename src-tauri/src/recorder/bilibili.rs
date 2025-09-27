pub mod client;
pub mod errors;
pub mod profile;
pub mod response;
use super::entry::Range;
use super::errors::RecorderError;
use super::PlatformType;
use crate::database::account::AccountRow;
use crate::ffmpeg::{extract_video_metadata, VideoMetadata};
use crate::progress::progress_manager::Event;
use crate::progress::progress_reporter::EventEmitter;
use crate::recorder::bilibili::client::{Codec, Protocol, Qn};
use crate::recorder::bilibili::errors::BiliClientError;
use crate::recorder::Recorder;
use crate::recorder_manager::RecorderEvent;
use crate::subtitle_generator::item_to_srt;

use super::danmu::{DanmuEntry, DanmuStorage};
use chrono::{DateTime, Utc};
use client::{BiliClient, BiliStream, Format, RoomInfo, UserInfo};
use danmu_stream::danmu_stream::DanmuStream;
use danmu_stream::provider::ProviderType;
use danmu_stream::DanmuMessageType;
use m3u8_rs::{Map, MediaPlaylist, MediaPlaylistType, MediaSegment, Playlist};
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
    platform_live_id: Arc<RwLock<String>>,
    live_id: Arc<RwLock<String>>,
    cover: Arc<RwLock<Option<String>>>,
    is_recording: Arc<RwLock<bool>>,
    last_update: Arc<RwLock<i64>>,
    quit: Arc<Mutex<bool>>,
    live_stream: Arc<RwLock<Option<BiliStream>>>,
    danmu_storage: Arc<RwLock<Option<DanmuStorage>>>,
    event_channel: broadcast::Sender<RecorderEvent>,
    enabled: Arc<RwLock<bool>>,
    current_metadata: Arc<RwLock<Option<VideoMetadata>>>,

    danmu_task: Arc<Mutex<Option<JoinHandle<()>>>>,
    record_task: Arc<Mutex<Option<JoinHandle<()>>>>,

    last_sequence: Arc<RwLock<u64>>,
    m3u8_playlist: Arc<RwLock<MediaPlaylist>>,
    total_duration: Arc<RwLock<f64>>,
    total_size: Arc<RwLock<u64>>,
}

struct SegmentBuffer {
    sequence: u64,
    representative_segment: MediaSegment,
    sub_segments: Vec<MediaSegment>,
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

fn default_m3u8_playlist() -> MediaPlaylist {
    MediaPlaylist {
        version: Some(6),
        target_duration: 4.0,
        end_list: true,
        playlist_type: Some(MediaPlaylistType::Vod),
        segments: Vec::new(),
        ..Default::default()
    }
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
            is_recording: Arc::new(RwLock::new(false)),
            platform_live_id: Arc::new(RwLock::new(String::new())),
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
            current_metadata: Arc::new(RwLock::new(None)),
            last_sequence: Arc::new(RwLock::new(0)),
            m3u8_playlist: Arc::new(RwLock::new(default_m3u8_playlist())),
            total_duration: Arc::new(RwLock::new(0.0)),
            total_size: Arc::new(RwLock::new(0)),
        };
        log::info!("Recorder for room {} created.", options.room_id);
        Ok(recorder)
    }

    pub async fn reset(&self) {
        // if record is ended, send event
        if !self.live_id.read().await.is_empty() && self.current_metadata.read().await.is_some() {
            self.m3u8_playlist.write().await.playlist_type = Some(MediaPlaylistType::Vod);
            self.m3u8_playlist.write().await.end_list = true;
            self.save_playlist().await;
            let _ = self.event_channel.send(RecorderEvent::RecordEnd {
                recorder: self.info().await,
            });
        }
        // if record is empty, remove record
        if !self.live_id.read().await.is_empty() && self.current_metadata.read().await.is_none() {
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
        *self.last_sequence.write().await = 0;
        *self.m3u8_playlist.write().await = default_m3u8_playlist();
        *self.live_stream.write().await = None;
        *self.last_update.write().await = Utc::now().timestamp();
        *self.danmu_storage.write().await = None;
        *self.current_metadata.write().await = None;
        *self.platform_live_id.write().await = String::new();
        *self.live_id.write().await = String::new();
        *self.total_duration.write().await = 0.0;
        *self.total_size.write().await = 0;
    }

    async fn should_record(&self) -> bool {
        if *self.quit.lock().await {
            return false;
        }

        *self.enabled.read().await
    }

    async fn add_segment(&self, sequence: u64, segment: MediaSegment) {
        let current_last_sequence = *self.last_sequence.read().await;
        let new_last_sequence = std::cmp::max(current_last_sequence, sequence);

        {
            let mut playlist = self.m3u8_playlist.write().await;
            playlist.segments.push(segment);
        }

        *self.last_sequence.write().await = new_last_sequence;

        self.save_playlist().await;
    }

    async fn load_playlist(
        &self,
        live_id: &str,
    ) -> Result<MediaPlaylist, super::errors::RecorderError> {
        let playlist_path = format!("{}/playlist.m3u8", self.get_work_dir(live_id).await);
        let playlist_full_path = self.get_full_path(&playlist_path).await;
        if !Path::new(&playlist_full_path).exists() {
            return Err(super::errors::RecorderError::IoError(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Playlist file not found",
            )));
        }
        let mut bytes: Vec<u8> = Vec::new();
        tokio::fs::File::open(playlist_full_path)
            .await
            .unwrap()
            .read_to_end(&mut bytes)
            .await
            .unwrap();
        if let Result::Ok((_, pl)) = m3u8_rs::parse_media_playlist(&bytes) {
            return Ok(pl);
        }
        Err(super::errors::RecorderError::M3u8ParseFailed {
            content: String::from_utf8(bytes).unwrap(),
        })
    }

    async fn save_playlist(&self) {
        let (playlist, live_id) = {
            let playlist = self.m3u8_playlist.read().await.clone();
            let live_id = self.live_id.read().await.clone();
            (playlist, live_id)
        };

        let playlist_path = format!("{}/playlist.m3u8", self.get_work_dir(&live_id).await);
        let playlist_full_path = self.get_full_path(&playlist_path).await;

        let mut bytes: Vec<u8> = Vec::new();
        playlist.write_to(&mut bytes).unwrap();

        match tokio::fs::File::create(&playlist_full_path).await {
            Ok(mut file) => {
                if let Err(e) = file.write_all(&bytes).await {
                    log::error!(
                        "Failed to write playlist file {}: {}",
                        playlist_full_path,
                        e
                    );
                }
            }
            Err(e) => {
                log::error!(
                    "Failed to create playlist file {}: {}",
                    playlist_full_path,
                    e
                );
            }
        }
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
                *self.platform_live_id.write().await = room_info.live_start_time.to_string();

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
                        Format::FMP4,
                        Codec::Avc,
                        Qn::Q4K,
                    )
                    .await;

                match new_stream {
                    Ok(stream) => {
                        *self.live_stream.write().await = Some(stream.clone());
                        *self.last_update.write().await = Utc::now().timestamp();

                        log::info!(
                            "[{}]Update to a new stream: {:?} => {}",
                            self.room_id,
                            self.live_stream.read().await.clone(),
                            stream
                        );

                        return true;
                    }
                    Err(e) => {
                        if let BiliClientError::FormatNotFound(format) = e {
                            log::error!(
                                "[{}]Format {} not found, try to fallback to ts",
                                self.room_id,
                                format
                            );
                        } else {
                            log::error!("[{}]Fetch stream failed: {}", self.room_id, e);

                            return true;
                        }
                    }
                }

                // fallback to ts
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

                match new_stream {
                    Ok(stream) => {
                        *self.live_stream.write().await = Some(stream.clone());
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
                        log::error!("[{}]Fetch stream failed: {}", self.room_id, e);

                        true
                    }
                }
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

    async fn get_metadata(
        &self,
        ts_path: &Path,
    ) -> Result<VideoMetadata, super::errors::RecorderError> {
        extract_video_metadata(ts_path)
            .await
            .map_err(super::errors::RecorderError::FfmpegError)
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
        let is_first_record = self.m3u8_playlist.read().await.segments.is_empty();

        if is_first_record {
            log::info!("[{}]New record started: {}", self.room_id, timestamp);
            *self.live_id.write().await = timestamp.to_string();
            tokio::fs::create_dir_all(self.get_full_path(&work_dir).await)
                .await
                .map_err(super::errors::RecorderError::IoError)?;

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
                    self.platform_live_id.read().await.as_str(),
                    timestamp.to_string().as_str(),
                    self.room_id,
                    &self.room_info.read().await.room_title,
                    Some(cover_path),
                )
                .await?;
        }

        match playlist {
            Playlist::MasterPlaylist(pl) => {
                log::debug!("[{}]Master playlist:\n{:?}", self.room_id, pl);
            }
            Playlist::MediaPlaylist(pl) => {
                if pl.segments.is_empty() {
                    log::warn!("[{}]Media playlist is empty", self.room_id);
                    return Err(super::errors::RecorderError::InvalidStream {
                        stream: current_stream,
                    });
                }
                let mut new_segment_fetched = false;
                let latest_sequence = *self.last_sequence.read().await;

                self.m3u8_playlist.write().await.target_duration = pl.target_duration;

                let is_fmp4 = current_stream.format == Format::FMP4;
                let mut header_data = Vec::new();

                if is_fmp4 {
                    // get fmp4 header from "#EXT-X-MAP:URI="h1758715459.m4s"
                    let first_segment = pl.segments.first().unwrap();
                    let mut header_url = first_segment
                        .unknown_tags
                        .iter()
                        .find(|t| t.tag == "X-MAP")
                        .map(|t| {
                            let rest = t.rest.clone().unwrap();
                            rest.split('=').nth(1).unwrap().replace("\\\"", "")
                        });
                    if header_url.is_none() {
                        // map: Some(Map { uri: "h1758725308.m4s"
                        if let Some(Map { uri, .. }) = &first_segment.map {
                            header_url = Some(uri.clone());
                        }
                    }

                    if header_url.is_none() {
                        log::error!("[{}]Fmp4 header not found", self.room_id);
                        return Err(super::errors::RecorderError::InvalidStream {
                            stream: current_stream,
                        });
                    }

                    let header_url = header_url.unwrap();
                    let header_url = current_stream.ts_url(&header_url);
                    header_data = self
                        .client
                        .read()
                        .await
                        .download_ts_raw(&header_url)
                        .await?;
                }

                let mut segment_buffers = Vec::new();
                let mut current_buffer = SegmentBuffer {
                    sequence: pl.media_sequence,
                    representative_segment: pl.segments.first().unwrap().clone(),
                    sub_segments: vec![pl.segments.first().unwrap().clone()],
                };

                let rest_segments = pl.segments.iter().skip(1).collect::<Vec<_>>();

                for (i, segment) in rest_segments.iter().enumerate() {
                    let is_key = segment
                        .unknown_tags
                        .iter()
                        .find(|t| t.tag == "BILI-AUX")
                        .map(|t| {
                            let rest = t.rest.clone().unwrap();
                            rest.split('|').nth(1).unwrap() == "K"
                        })
                        .unwrap_or(true);

                    if is_key {
                        // start a new buffer
                        segment_buffers.push(current_buffer);
                        current_buffer = SegmentBuffer {
                            sequence: pl.media_sequence + (i + 1) as u64,
                            representative_segment: (*segment).clone(),
                            sub_segments: vec![(*segment).clone()],
                        };
                    } else {
                        current_buffer.sub_segments.push((*segment).clone());
                    }
                }

                for buffer in segment_buffers {
                    if buffer.sequence <= latest_sequence {
                        continue;
                    }

                    let mut source_data = Vec::new();

                    for ts in buffer.sub_segments {
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

                        let mut retry = 0;
                        let client = self.client.clone();

                        loop {
                            if retry > 3 {
                                log::error!("[{}]Download ts failed after retry", self.room_id);

                                break;
                            }

                            match client.read().await.download_ts_raw(&ts_url).await {
                                Ok(data) => {
                                    if data.is_empty() {
                                        log::error!(
                                            "[{}]Segment with size 0, stream might be corrupted",
                                            self.room_id
                                        );

                                        return Err(super::errors::RecorderError::InvalidStream {
                                            stream: current_stream,
                                        });
                                    }

                                    source_data.extend_from_slice(&data);

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
                                }
                            }
                        }
                    }

                    let file_name = if is_fmp4 {
                        buffer
                            .representative_segment
                            .uri
                            .split('/')
                            .next_back()
                            .unwrap_or(&buffer.representative_segment.uri)
                            .to_string()
                            .replace("m4s", "ts")
                    } else {
                        buffer
                            .representative_segment
                            .uri
                            .split('/')
                            .next_back()
                            .unwrap_or(&buffer.representative_segment.uri)
                            .to_string()
                    };

                    let full_path = self.get_full_path(&format!("{work_dir}/{file_name}")).await;
                    let full_path = Path::new(&full_path);

                    let mut to_add_segment = buffer.representative_segment.clone();
                    to_add_segment.uri = file_name.clone();

                    if is_fmp4 {
                        crate::ffmpeg::convert_fmp4_to_ts_raw(
                            &header_data,
                            &source_data,
                            full_path,
                        )
                        .await
                        .map_err(super::errors::RecorderError::FfmpegError)?;
                    } else {
                        // just save the data
                        let mut file = tokio::fs::File::create(&full_path).await?;
                        file.write_all(&source_data).await?;
                    }

                    let metadata = self.get_metadata(full_path).await;
                    if metadata.is_err() {
                        return Err(metadata.err().unwrap());
                    }
                    let metadata = metadata.unwrap();
                    let current_metadata = self.current_metadata.read().await.clone();
                    // if Packet Interleaving Stream, video size might be 0, ignore it so that stream is not corrupted
                    if metadata.width != 0 && metadata.height != 0 {
                        if let Some(current_metadata) = current_metadata {
                            if current_metadata.width != metadata.width
                                || current_metadata.height != metadata.height
                            {
                                log::warn!(
                                    "[{}]Resolution changed: {:?} => {:?}",
                                    self.room_id,
                                    &current_metadata,
                                    &metadata
                                );
                                return Err(super::errors::RecorderError::ResolutionChanged {
                                    err: format!(
                                        "Resolution changed: {:?} => {:?}",
                                        &current_metadata, &metadata
                                    ),
                                });
                            }
                        } else {
                            // first segment, set current resolution
                            *self.current_metadata.write().await = Some(metadata.clone());

                            let _ = self.event_channel.send(RecorderEvent::RecordStart {
                                recorder: self.info().await,
                            });
                        }
                    }

                    to_add_segment.map = None;
                    to_add_segment.uri = file_name.clone();
                    to_add_segment.duration = metadata.duration as f32;

                    if is_fmp4 {
                        // date time is not provided, should be calculated by offset
                        let live_start_time = self.room_info.read().await.live_start_time;
                        let mut seg_offset: i64 = 0;
                        for tag in &to_add_segment.unknown_tags {
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
                        to_add_segment.program_date_time = Some(
                            DateTime::from_timestamp(live_start_time + seg_offset / 1000, 0)
                                .unwrap()
                                .into(),
                        );
                    }
                    self.add_segment(buffer.sequence, to_add_segment).await;

                    *self.total_duration.write().await += metadata.duration;
                    *self.total_size.write().await += source_data.len() as u64;
                    *self.last_sequence.write().await = buffer.sequence;

                    new_segment_fetched = true;
                }

                if new_segment_fetched {
                    *self.last_update.write().await = Utc::now().timestamp();

                    self.db
                        .update_record(
                            timestamp.to_string().as_str(),
                            *self.total_duration.read().await as i64,
                            *self.total_size.read().await,
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

    async fn generate_archive_playlist(
        &self,
        live_id: &str,
        start: i64,
        end: i64,
    ) -> MediaPlaylist {
        let mut range = None;
        if start != 0 || end != 0 {
            range = Some(Range {
                x: start as f32,
                y: end as f32,
            });
        }

        let playlist = self.load_playlist(live_id).await;
        if playlist.is_err() {
            return MediaPlaylist::default();
        }
        let mut playlist = playlist.unwrap();

        if let Some(range) = range {
            // accumulate duration, and filter segments in range
            let mut duration = 0.0;
            let mut segments = Vec::new();
            for s in playlist.segments {
                if range.is_in(duration) || range.is_in(duration + s.duration) {
                    segments.push(s.clone());
                }
                duration += s.duration;
            }
            playlist.segments = segments;
        }

        playlist.end_list = true;
        playlist.playlist_type = Some(MediaPlaylistType::Vod);

        playlist
    }

    /// if fetching live/last stream m3u8, all entries are cached in memory, so it will be much faster than `read_dir`
    async fn generate_live_playlist(&self, start: i64, end: i64) -> MediaPlaylist {
        let live_status = *self.live_status.read().await;
        let range = if start != 0 || end != 0 {
            Some(Range {
                x: start as f32,
                y: end as f32,
            })
        } else {
            None
        };

        let mut playlist = self.m3u8_playlist.read().await.clone();

        if playlist.segments.is_empty() {
            return MediaPlaylist::default();
        }

        if let Some(range) = range {
            // accumulate duration, and filter segments in range
            let mut duration = 0.0;
            let mut segments = Vec::new();
            for s in playlist.segments {
                if range.is_in(duration) || range.is_in(duration + s.duration) {
                    segments.push(s.clone());
                }
                duration += s.duration;
            }
            playlist.segments = segments;
        }

        (playlist.playlist_type, playlist.end_list) = if live_status && range.is_none() {
            (Some(MediaPlaylistType::Event), false)
        } else {
            (Some(MediaPlaylistType::Vod), true)
        };

        playlist
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

    async fn playlist(&self, live_id: &str, start: i64, end: i64) -> MediaPlaylist {
        let playlist = if *self.live_id.read().await == live_id && self.should_record().await {
            self.generate_live_playlist(start, end).await
        } else {
            self.generate_archive_playlist(live_id, start, end).await
        };

        playlist
    }

    async fn get_related_playlists(&self, parent_id: &str) -> Vec<(String, String)> {
        let archives = self
            .db
            .get_archives_by_parent_id(self.room_id, parent_id)
            .await;
        if let Err(e) = archives {
            log::error!(
                "[{}] Failed to get all related playlists: {} {}",
                self.room_id,
                parent_id,
                e
            );
            return Vec::new();
        }

        let archives: Vec<(String, String)> = archives
            .unwrap()
            .iter()
            .map(|a| (a.title.clone(), a.live_id.clone()))
            .collect();

        let playlists = archives
            .iter()
            .map(async |a| {
                let work_dir = self.get_work_dir(a.1.as_str()).await;
                (
                    a.0.clone(),
                    format!(
                        "{}/{}",
                        self.get_full_path(&work_dir).await,
                        "playlist.m3u8"
                    ),
                )
            })
            .collect::<Vec<_>>();

        let playlists = futures::future::join_all(playlists).await;

        return playlists;
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
            total_length: *self.total_duration.read().await,
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
                Some(storage) => storage.get_entries(0).await,
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
            storage.get_entries(0).await
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
        let playlist = self.playlist(live_id, 0, 0).await;
        let mut v: Vec<u8> = Vec::new();
        playlist.write_to(&mut v).unwrap();
        let m3u8_content: &str = std::str::from_utf8(&v).unwrap();
        tokio::fs::write(&m3u8_index_file_path, m3u8_content).await?;
        log::info!(
            "[{}]M3U8 index file generated: {}",
            self.room_id,
            m3u8_index_file_path
        );
        // generate a tmp clip file
        let clip_file_path = format!("{}/{}", work_dir, "tmp.mp4");
        if let Err(e) = crate::ffmpeg::playlist::playlist_to_video(
            None::<&crate::progress::progress_reporter::ProgressReporter>,
            Path::new(&m3u8_index_file_path),
            Path::new(&clip_file_path),
            None,
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
