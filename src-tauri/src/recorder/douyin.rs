pub mod client;
mod response;
mod stream_info;
use super::entry::Range;
use super::{
    danmu::DanmuEntry, errors::RecorderError, PlatformType, Recorder, RecorderInfo, RoomInfo,
    UserInfo,
};
use crate::database::Database;
use crate::progress::progress_manager::Event;
use crate::progress::progress_reporter::EventEmitter;
use crate::recorder::{CachePath, FfmpegProgressHandler};
use crate::recorder_manager::RecorderEvent;
use crate::subtitle_generator::item_to_srt;
use crate::{config::Config, database::account::AccountRow};
use async_trait::async_trait;
use chrono::Utc;
use client::DouyinClientError;
use danmu_stream::danmu_stream::DanmuStream;
use danmu_stream::provider::ProviderType;
use danmu_stream::DanmuMessageType;
use m3u8_rs::{MediaPlaylist, MediaPlaylistType};
use rand::random;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{broadcast, Mutex, RwLock};
use tokio::task::JoinHandle;

use super::danmu::DanmuStorage;

#[cfg(not(feature = "headless"))]
use {tauri::AppHandle, tauri_plugin_notification::NotificationExt};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum LiveStatus {
    Live,
    Offline,
}

#[derive(Clone)]
pub struct DouyinRecorder {
    #[cfg(not(feature = "headless"))]
    app_handle: AppHandle,
    emitter: EventEmitter,
    client: client::DouyinClient,
    db: Arc<Database>,
    account: AccountRow,
    room_id: i64,
    sec_user_id: String,
    room_info: Arc<RwLock<Option<client::DouyinBasicRoomInfo>>>,
    stream_url: Arc<RwLock<Option<String>>>,
    danmu_store: Arc<RwLock<Option<DanmuStorage>>>,
    live_id: Arc<RwLock<String>>,
    platform_live_id: Arc<RwLock<String>>,
    live_status: Arc<RwLock<LiveStatus>>,
    is_recording: Arc<RwLock<bool>>,
    running: Arc<RwLock<bool>>,
    config: Arc<RwLock<Config>>,
    event_channel: broadcast::Sender<RecorderEvent>,
    enabled: Arc<RwLock<bool>>,

    danmu_stream_task: Arc<Mutex<Option<JoinHandle<()>>>>,
    danmu_task: Arc<Mutex<Option<JoinHandle<()>>>>,
    record_task: Arc<Mutex<Option<JoinHandle<()>>>>,

    total_duration: Arc<RwLock<f64>>,
    total_size: Arc<RwLock<u64>>,
}

fn get_best_stream_url(room_info: &client::DouyinBasicRoomInfo) -> Option<String> {
    let stream_data = room_info.stream_data.clone();
    // parse stream_data into stream_info
    let stream_info = serde_json::from_str::<stream_info::StreamInfo>(&stream_data);
    if let Ok(stream_info) = stream_info {
        // find the best stream url
        if stream_info.data.origin.main.hls.is_empty() {
            log::error!("No stream url found in stream_data: {stream_data}");
            return None;
        }

        Some(stream_info.data.origin.main.hls)
    } else {
        let err = stream_info.unwrap_err();
        log::error!("Failed to parse stream data: {err} {stream_data}");
        None
    }
}

impl DouyinRecorder {
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        #[cfg(not(feature = "headless"))] app_handle: AppHandle,
        emitter: EventEmitter,
        room_id: i64,
        sec_user_id: &str,
        config: Arc<RwLock<Config>>,
        account: &AccountRow,
        db: &Arc<Database>,
        enabled: bool,
        channel: broadcast::Sender<RecorderEvent>,
    ) -> Result<Self, super::errors::RecorderError> {
        let client = client::DouyinClient::new(account);
        let room_info = client.get_room_info(room_id, sec_user_id).await?;
        let mut live_status = LiveStatus::Offline;
        if room_info.status == 0 {
            live_status = LiveStatus::Live;
        }

        Ok(Self {
            #[cfg(not(feature = "headless"))]
            app_handle,
            emitter,
            db: db.clone(),
            account: account.clone(),
            room_id,
            sec_user_id: sec_user_id.to_string(),
            live_id: Arc::new(RwLock::new(String::new())),
            platform_live_id: Arc::new(RwLock::new(String::new())),
            danmu_store: Arc::new(RwLock::new(None)),
            client,
            room_info: Arc::new(RwLock::new(Some(room_info))),
            stream_url: Arc::new(RwLock::new(None)),
            live_status: Arc::new(RwLock::new(live_status)),
            running: Arc::new(RwLock::new(false)),
            is_recording: Arc::new(RwLock::new(false)),
            enabled: Arc::new(RwLock::new(enabled)),
            config,
            event_channel: channel,

            danmu_stream_task: Arc::new(Mutex::new(None)),
            danmu_task: Arc::new(Mutex::new(None)),
            record_task: Arc::new(Mutex::new(None)),

            total_duration: Arc::new(RwLock::new(0.0)),
            total_size: Arc::new(RwLock::new(0)),
        })
    }

    async fn should_record(&self) -> bool {
        if !*self.running.read().await {
            return false;
        }

        *self.enabled.read().await
    }

    async fn check_status(&self) -> bool {
        match self
            .client
            .get_room_info(self.room_id, &self.sec_user_id)
            .await
        {
            Ok(info) => {
                let live_status = info.status == 0; // room_status == 0 表示正在直播

                *self.room_info.write().await = Some(info.clone());

                if (*self.live_status.read().await == LiveStatus::Live) != live_status {
                    // live status changed, reset current record flag
                    log::info!(
                        "[{}]Live status changed to {}, auto_start: {}",
                        self.room_id,
                        live_status,
                        *self.enabled.read().await
                    );

                    if live_status {
                        #[cfg(not(feature = "headless"))]
                        self.app_handle
                            .notification()
                            .builder()
                            .title("BiliShadowReplay - 直播开始")
                            .body(format!(
                                "{} 开启了直播：{}",
                                info.user_name, info.room_title
                            ))
                            .show()
                            .unwrap();

                        let _ = self.event_channel.send(RecorderEvent::LiveStart {
                            recorder: self.info().await,
                        });
                    } else {
                        #[cfg(not(feature = "headless"))]
                        self.app_handle
                            .notification()
                            .builder()
                            .title("BiliShadowReplay - 直播结束")
                            .body(format!(
                                "{} 关闭了直播：{}",
                                info.user_name, info.room_title
                            ))
                            .show()
                            .unwrap();
                        let _ = self.event_channel.send(RecorderEvent::LiveEnd {
                            platform: PlatformType::Douyin,
                            room_id: self.room_id,
                            recorder: self.info().await,
                        });
                        *self.live_id.write().await = String::new();
                    }

                    self.reset().await;
                }

                if live_status {
                    *self.live_status.write().await = LiveStatus::Live;
                } else {
                    *self.live_status.write().await = LiveStatus::Offline;
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
                    let new_stream_url = get_best_stream_url(&info);
                    if new_stream_url.is_none() {
                        log::error!("No stream url found in room_info: {info:#?}");
                        return false;
                    }

                    log::info!("New douyin stream URL: {}", new_stream_url.clone().unwrap());
                    *self.stream_url.write().await = Some(new_stream_url.unwrap());
                    (*self.platform_live_id.write().await).clone_from(&info.room_id_str);
                }

                true
            }
            Err(e) => {
                if let DouyinClientError::H5NotLive(e) = e {
                    log::debug!("[{}]Live maybe not started: {}", self.room_id, e);
                    return false;
                }
                log::error!("[{}]Update room status failed: {}", self.room_id, e);
                *self.live_status.read().await == LiveStatus::Live
            }
        }
    }

    async fn danmu(&self) -> Result<(), super::errors::RecorderError> {
        let cookies = self.account.cookies.clone();
        let danmu_room_id = self
            .platform_live_id
            .read()
            .await
            .clone()
            .parse::<i64>()
            .unwrap_or(0);
        let danmu_stream = DanmuStream::new(ProviderType::Douyin, &cookies, danmu_room_id).await;
        if danmu_stream.is_err() {
            let err = danmu_stream.err().unwrap();
            log::error!("Failed to create danmu stream: {err}");
            return Err(super::errors::RecorderError::DanmuStreamError(err));
        }
        let danmu_stream = danmu_stream.unwrap();

        let danmu_stream_clone = danmu_stream.clone();
        *self.danmu_stream_task.lock().await = Some(tokio::spawn(async move {
            let _ = danmu_stream_clone.start().await;
        }));

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
                        if let Some(storage) = self.danmu_store.read().await.as_ref() {
                            storage.add_line(ts, &danmu.message).await;
                        }
                    }
                }
            } else {
                log::error!("Failed to receive danmu message");
                return Err(super::errors::RecorderError::DanmuStreamError(
                    danmu_stream::DanmuStreamError::WebsocketError {
                        err: "Failed to receive danmu message".to_string(),
                    },
                ));
            }
        }
    }

    async fn reset(&self) {
        let live_id = self.live_id.read().await.clone();
        if !live_id.is_empty() && *self.total_duration.read().await == 0.0 {
            // previous recording is empty, work dir and record needs to be deleted
            let work_dir = self.work_dir(live_id.as_str()).await;
            if let Err(e) = tokio::fs::remove_dir_all(&work_dir.full_path()).await {
                log::error!("[{}]Failed to delete work dir: {}", self.room_id, e);
            };

            // delete record
            if let Err(e) = self.db.remove_record(live_id.as_str()).await {
                log::error!("[{}]Failed to delete record: {}", self.room_id, e);
            };
        }
        *self.platform_live_id.write().await = String::new();
        *self.stream_url.write().await = None;
        *self.total_duration.write().await = 0.0;
        *self.total_size.write().await = 0;
    }

    async fn work_dir(&self, live_id: &str) -> CachePath {
        CachePath::new(
            &self.config.read().await.cache,
            PlatformType::Douyin,
            self.room_id,
            live_id,
        )
    }

    async fn load_playlist(
        &self,
        live_id: &str,
    ) -> Result<MediaPlaylist, super::errors::RecorderError> {
        let playlist_file_path = format!("{}/{}", self.work_dir(live_id).await, "playlist.m3u8");
        match tokio::fs::read(&playlist_file_path).await {
            Ok(playlist_content) => {
                let playlist = m3u8_rs::parse_media_playlist(&playlist_content).unwrap().1;
                Ok(playlist)
            }
            Err(e) => Err(super::errors::RecorderError::IoError(e)),
        }
    }

    async fn update_entries(&self) -> Result<(), RecorderError> {
        // Get current room info and stream URL
        let room_info = self.room_info.read().await.clone();
        let Some(room_info) = room_info else {
            return Err(RecorderError::NoRoomInfo);
        };

        let Some(stream_url) = self.stream_url.read().await.clone() else {
            return Err(RecorderError::NoStreamAvailable);
        };

        let live_id = Utc::now().timestamp_millis().to_string();
        *self.live_id.write().await = live_id.clone();

        let work_dir = self.work_dir(&live_id).await;
        let _ = tokio::fs::create_dir_all(&work_dir.full_path()).await;

        // download cover
        if let Some(cover_url) = room_info.cover.clone() {
            let cover_path = format!("{work_dir}/cover.jpg");
            let _ = self
                .client
                .download_file(&cover_url, Path::new(&cover_path))
                .await;
        }

        // Setup danmu store
        let danmu_file_path = work_dir.with_filename("danmu.txt");
        let danmu_store = DanmuStorage::new(&danmu_file_path.full_path()).await;
        *self.danmu_store.write().await = danmu_store;

        // Start danmu task
        if let Some(danmu_task) = self.danmu_task.lock().await.as_mut() {
            danmu_task.abort();
        }
        if let Some(danmu_stream_task) = self.danmu_stream_task.lock().await.as_mut() {
            danmu_stream_task.abort();
        }

        let self_clone = self.clone();
        log::info!("Start fetching danmu for live {live_id}");
        *self.danmu_task.lock().await = Some(tokio::spawn(async move {
            let _ = self_clone.danmu().await;
        }));

        // add db record
        let _ = self
            .db
            .add_record(
                PlatformType::Douyin,
                self.platform_live_id.read().await.as_str(),
                live_id.as_str(),
                self.room_id,
                &room_info.room_title,
                Some(format!("douyin/{}/{}/cover.jpg", self.room_id, live_id)),
            )
            .await;

        let _ = self.event_channel.send(RecorderEvent::RecordStart {
            recorder: self.info().await,
        });

        let ffmpeg_progress_handler = FfmpegProgressHandler {
            db: self.db.clone(),
            live_id: self.live_id.clone(),
            total_duration: self.total_duration.clone(),
            total_size: self.total_size.clone(),
            work_dir: work_dir.full_path(),
        };

        if let Err(e) = crate::ffmpeg::playlist::cache_playlist(
            Some(&ffmpeg_progress_handler),
            &stream_url,
            Path::new(&work_dir.full_path()),
        )
        .await
        {
            log::error!("[{}]Failed to cache playlist: {}", self.room_id, e);
        }

        Ok(())
    }

    async fn generate_m3u8(&self, live_id: &str, start: i64, end: i64) -> MediaPlaylist {
        let range = if start != 0 || end != 0 {
            Some(Range {
                x: start as f32,
                y: end as f32,
            })
        } else {
            None
        };

        let playlist = self.load_playlist(live_id).await;
        if playlist.is_err() {
            return MediaPlaylist::default();
        }
        let mut playlist = playlist.unwrap();
        if let Some(range) = range {
            let mut duration = 0.0;
            let mut segments = Vec::new();
            for s in playlist.segments {
                if range.is_in(duration) || range.is_in(duration + s.duration) {
                    segments.push(s.clone());
                }
                duration += s.duration;
            }
            playlist.segments = segments;

            playlist.end_list = true;
            playlist.playlist_type = Some(MediaPlaylistType::Vod);

            return playlist;
        }

        if live_id == *self.live_id.read().await {
            playlist.end_list = false;
            playlist.playlist_type = Some(MediaPlaylistType::Event);

            return playlist;
        }

        playlist
    }
}

#[async_trait]
impl Recorder for DouyinRecorder {
    async fn run(&self) {
        *self.running.write().await = true;

        let self_clone = self.clone();
        *self.record_task.lock().await = Some(tokio::spawn(async move {
            while *self_clone.running.read().await {
                if self_clone.check_status().await {
                    // Live status is ok, start recording
                    if self_clone.should_record().await {
                        *self_clone.is_recording.write().await = true;
                        if let Err(e) = self_clone.update_entries().await {
                            log::error!("[{}]Update entries error: {}", self_clone.room_id, e);
                        }
                    }
                    if *self_clone.is_recording.read().await {
                        let _ = self_clone.event_channel.send(RecorderEvent::RecordEnd {
                            recorder: self_clone.info().await,
                        });
                    }
                    *self_clone.is_recording.write().await = false;
                    self_clone.reset().await;
                    // Check status again after some seconds
                    let secs = random::<u64>() % 5;
                    tokio::time::sleep(Duration::from_secs(secs)).await;
                    continue;
                }

                let interval = self_clone.config.read().await.status_check_interval;
                tokio::time::sleep(Duration::from_secs(interval)).await;
            }
            log::info!("recording thread {} quit.", self_clone.room_id);
        }));
    }

    async fn stop(&self) {
        *self.running.write().await = false;
        // stop 3 tasks
        if let Some(danmu_task) = self.danmu_task.lock().await.as_mut() {
            let () = danmu_task.abort();
        }
        if let Some(danmu_stream_task) = self.danmu_stream_task.lock().await.as_mut() {
            let () = danmu_stream_task.abort();
        }
        if let Some(record_task) = self.record_task.lock().await.as_mut() {
            let () = record_task.abort();
        }
        log::info!("Recorder for room {} quit.", self.room_id);
    }

    async fn playlist(&self, live_id: &str, start: i64, end: i64) -> MediaPlaylist {
        self.generate_m3u8(live_id, start, end).await
    }

    async fn get_archive_subtitle(
        &self,
        live_id: &str,
    ) -> Result<String, super::errors::RecorderError> {
        let work_dir = self.work_dir(live_id).await;
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
        let work_dir = self.work_dir(live_id).await;
        let subtitle_file_path = format!("{}/{}", work_dir, "subtitle.srt");
        let mut subtitle_file = File::create(subtitle_file_path).await?;
        // first generate a tmp clip file
        // generate a tmp m3u8 index file
        let m3u8_index_file_path = format!("{}/{}", work_dir, "tmp.m3u8");
        let playlist = self.playlist(live_id, 0, 0).await;
        let mut v: Vec<u8> = Vec::new();
        playlist.write_to(&mut v).unwrap();
        let m3u8_content: &str = std::str::from_utf8(&v).unwrap();
        tokio::fs::write(&m3u8_index_file_path, m3u8_content).await?;
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
        let result = result.unwrap();
        let subtitle_content = result
            .subtitle_content
            .iter()
            .map(item_to_srt)
            .collect::<String>();
        subtitle_file.write_all(subtitle_content.as_bytes()).await?;

        // remove tmp file
        tokio::fs::remove_file(&m3u8_index_file_path).await?;
        tokio::fs::remove_file(&clip_file_path).await?;
        Ok(subtitle_content)
    }

    async fn get_related_playlists(&self, parent_id: &str) -> Vec<(String, String)> {
        let playlists = self
            .db
            .get_archives_by_parent_id(self.room_id, parent_id)
            .await;
        if playlists.is_err() {
            return Vec::new();
        }
        let ids: Vec<(String, String)> = playlists
            .unwrap()
            .iter()
            .map(|a| (a.title.clone(), a.live_id.clone()))
            .collect();
        let playlists = ids
            .iter()
            .map(async |a| {
                (
                    a.0.clone(),
                    format!("{}/{}", self.work_dir(a.1.as_str()).await, "playlist.m3u8"),
                )
            })
            .collect::<Vec<_>>();
        let playlists = futures::future::join_all(playlists).await;
        return playlists;
    }

    async fn info(&self) -> RecorderInfo {
        let room_info = self.room_info.read().await;
        let room_cover_url = room_info
            .as_ref()
            .and_then(|info| info.cover.clone())
            .unwrap_or_default();
        let room_title = room_info
            .as_ref()
            .map(|info| info.room_title.clone())
            .unwrap_or_default();
        RecorderInfo {
            room_id: self.room_id,
            room_info: RoomInfo {
                room_id: self.room_id,
                room_title,
                room_cover: room_cover_url,
            },
            user_info: UserInfo {
                user_id: room_info
                    .as_ref()
                    .map(|info| info.sec_user_id.clone())
                    .unwrap_or_default(),
                user_name: room_info
                    .as_ref()
                    .map(|info| info.user_name.clone())
                    .unwrap_or_default(),
                user_avatar: room_info
                    .as_ref()
                    .map(|info| info.user_avatar.clone())
                    .unwrap_or_default(),
            },
            total_length: *self.total_duration.read().await,
            current_live_id: self.live_id.read().await.clone(),
            live_status: *self.live_status.read().await == LiveStatus::Live,
            is_recording: *self.is_recording.read().await,
            auto_start: *self.enabled.read().await,
            platform: PlatformType::Douyin.as_str().to_string(),
        }
    }

    async fn comments(&self, live_id: &str) -> Result<Vec<DanmuEntry>, RecorderError> {
        let work_dir = self.work_dir(live_id).await;
        Ok(if live_id == *self.live_id.read().await {
            // just return current cache content
            match self.danmu_store.read().await.as_ref() {
                Some(storage) => storage.get_entries(0).await,
                None => Vec::new(),
            }
        } else {
            // load disk cache
            let cache_file_path = work_dir.with_filename("danmu.txt");
            log::debug!("loading danmu cache from {cache_file_path}");
            let storage = DanmuStorage::new(&cache_file_path.full_path()).await;
            if storage.is_none() {
                return Ok(Vec::new());
            }
            let storage = storage.unwrap();
            storage.get_entries(0).await
        })
    }

    async fn is_recording(&self, live_id: &str) -> bool {
        *self.live_id.read().await == live_id && *self.live_status.read().await == LiveStatus::Live
    }

    async fn enable(&self) {
        *self.enabled.write().await = true;
    }

    async fn disable(&self) {
        *self.enabled.write().await = false;
    }
}
