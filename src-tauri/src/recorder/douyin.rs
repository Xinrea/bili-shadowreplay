pub mod client;
mod response;
use super::entry::EntryStore;
use super::{
    danmu::DanmuEntry, errors::RecorderError, PlatformType, Recorder, RecorderInfo, RoomInfo,
    UserInfo,
};
use crate::database::Database;
use crate::playlist::HLSPlaylist;
use crate::{config::Config, database::account::AccountRow};
use async_trait::async_trait;
use chrono::Utc;
use client::DouyinClientError;
use dashmap::DashMap;
use m3u8_rs::Playlist;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::RwLock;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum LiveStatus {
    Live,
    Offline,
}

impl From<std::io::Error> for RecorderError {
    fn from(err: std::io::Error) -> Self {
        RecorderError::IoError { err }
    }
}

impl From<DouyinClientError> for RecorderError {
    fn from(err: DouyinClientError) -> Self {
        RecorderError::DouyinClientError { err }
    }
}

#[derive(Clone)]
pub struct DouyinRecorder {
    app_handle: AppHandle,
    client: client::DouyinClient,
    db: Arc<Database>,
    pub room_id: u64,
    pub room_info: Arc<RwLock<Option<response::DouyinRoomInfoResponse>>>,
    pub stream_url: Arc<RwLock<Option<String>>>,
    pub hls_playlist: Arc<RwLock<Option<HLSPlaylist>>>,
    pub live_id: Arc<RwLock<String>>,
    pub live_status: Arc<RwLock<LiveStatus>>,
    is_recording: Arc<RwLock<bool>>,
    auto_start: Arc<RwLock<bool>>,
    current_record: Arc<RwLock<bool>>,
    running: Arc<RwLock<bool>>,
    last_update: Arc<RwLock<i64>>,
    m3u8_cache: DashMap<String, String>,
    config: Arc<RwLock<Config>>,
}

impl DouyinRecorder {
    pub async fn new(
        app_handle: AppHandle,
        room_id: u64,
        config: Arc<RwLock<Config>>,
        douyin_account: &AccountRow,
        db: &Arc<Database>,
        auto_start: bool,
    ) -> Result<Self, super::errors::RecorderError> {
        let client = client::DouyinClient::new(douyin_account);
        let room_info = client.get_room_info(room_id).await?;
        let mut live_status = LiveStatus::Offline;
        if room_info.data.room_status == 0 {
            live_status = LiveStatus::Live;
        }

        Ok(Self {
            app_handle,
            db: db.clone(),
            room_id,
            live_id: Arc::new(RwLock::new(String::new())),
            hls_playlist: Arc::new(RwLock::new(None)),
            client,
            room_info: Arc::new(RwLock::new(Some(room_info))),
            stream_url: Arc::new(RwLock::new(None)),
            live_status: Arc::new(RwLock::new(live_status)),
            running: Arc::new(RwLock::new(false)),
            is_recording: Arc::new(RwLock::new(false)),
            auto_start: Arc::new(RwLock::new(auto_start)),
            current_record: Arc::new(RwLock::new(false)),
            last_update: Arc::new(RwLock::new(Utc::now().timestamp())),
            m3u8_cache: DashMap::new(),
            config,
        })
    }

    async fn should_record(&self) -> bool {
        if !*self.running.read().await {
            return false;
        }

        *self.current_record.read().await
    }

    async fn check_status(&self) -> bool {
        match self.client.get_room_info(self.room_id).await {
            Ok(info) => {
                let live_status = info.data.room_status == 0; // room_status == 0 表示正在直播
                let previous_liveid = self.live_id.read().await.clone();

                *self.room_info.write().await = Some(info.clone());

                if (*self.live_status.read().await == LiveStatus::Live) != live_status {
                    // live status changed, reset current record flag
                    *self.current_record.write().await = false;
                    self.reset().await;

                    log::info!(
                        "[{}]Live status changed to {}, current_record: {}, auto_start: {}",
                        self.room_id,
                        live_status,
                        *self.current_record.read().await,
                        *self.auto_start.read().await
                    );

                    if live_status {
                        self.app_handle
                            .notification()
                            .builder()
                            .title("BiliShadowReplay - 直播开始")
                            .body(format!(
                                "{} 开启了直播：{}",
                                info.data.user.nickname, info.data.data[0].title
                            ))
                            .show()
                            .unwrap();
                    } else {
                        self.app_handle
                            .notification()
                            .builder()
                            .title("BiliShadowReplay - 直播结束")
                            .body(format!(
                                "{} 关闭了直播：{}",
                                info.data.user.nickname, info.data.data[0].title
                            ))
                            .show()
                            .unwrap();
                    }
                }

                if live_status {
                    *self.live_status.write().await = LiveStatus::Live;
                } else {
                    *self.live_status.write().await = LiveStatus::Offline;
                }

                if !live_status {
                    *self.current_record.write().await = false;
                    self.reset().await;

                    return false;
                }

                if !*self.current_record.read().await && !*self.auto_start.read().await {
                    return true;
                }

                if *self.auto_start.read().await
                    && previous_liveid != info.data.data[0].id_str.clone()
                {
                    *self.current_record.write().await = true;
                }

                if *self.current_record.read().await {
                    // Get stream URL when live starts
                    if !info.data.data[0]
                        .stream_url
                        .as_ref()
                        .unwrap()
                        .hls_pull_url
                        .is_empty()
                    {
                        let live_id = info.data.data[0].id_str.clone();
                        *self.live_id.write().await = live_id.clone();
                        // create a new record
                        let cover_url = info.data.data[0]
                            .cover
                            .as_ref()
                            .map(|cover| cover.url_list[0].clone());
                        let cover = if let Some(url) = cover_url {
                            Some(self.client.get_cover_base64(&url).await.unwrap())
                        } else {
                            None
                        };

                        if let Err(e) = self
                            .db
                            .add_record(
                                PlatformType::Douyin,
                                self.live_id.read().await.as_str(),
                                self.room_id,
                                &info.data.data[0].title,
                                cover,
                                None,
                            )
                            .await
                        {
                            log::error!("Failed to add record: {}", e);
                        }

                        // setup playlist
                        let playlist = self.load_previous_playlist(&live_id).await;
                        *self.hls_playlist.write().await = playlist;
                    }

                    return true;
                }

                true
            }
            Err(e) => {
                log::error!("[{}]Update room status failed: {}", self.room_id, e);
                *self.live_status.read().await == LiveStatus::Live
            }
        }
    }

    async fn load_previous_playlist(&self, live_id: &str) -> Option<HLSPlaylist> {
        // first: check existed playlist file
        let work_dir = self.get_work_dir(live_id).await;
        let playlist_filepath = Path::new(&work_dir).join("index.m3u8");
        let file = File::open(&playlist_filepath).await;
        if let Err(e) = file {
            log::warn!("Local index.m3u8 open failed: {}", e);

            log::info!("Load playlist from entry store");
            let playlist = self.load_playlist_from_entrystore(live_id).await;
            if playlist.is_some() {
                // write playlist to file
                let mut file = File::create(&playlist_filepath).await.unwrap();
                let playlist = playlist.clone().unwrap();
                let playlist_content = playlist.to_string();
                file.write_all(playlist_content.as_bytes()).await.unwrap();
                file.flush().await.unwrap();
                // close file
                drop(file);
                log::info!(
                    "Generate entrystore playlist to file: {}",
                    playlist_filepath.display()
                );
            }

            return playlist;
        }

        let mut file = file.unwrap();
        let mut playlist_content = String::new();
        let _ = file.read_to_string(&mut playlist_content).await;

        let playlist = m3u8_rs::parse_playlist_res(playlist_content.as_bytes());
        if let Err(e) = playlist {
            log::error!("Parse local playlist failed: {}", e,);
            log::error!("Playlist content: {}", playlist_content);

            return None;
        }

        let playlist = playlist.unwrap();
        match playlist {
            Playlist::MediaPlaylist(p) => Some(HLSPlaylist::from(&p)),
            Playlist::MasterPlaylist(_) => None,
        }
    }

    async fn load_playlist_from_entrystore(&self, live_id: &str) -> Option<HLSPlaylist> {
        let work_dir = self.get_work_dir(live_id).await;
        let entry_store = EntryStore::new(&work_dir).await;
        if entry_store.is_none() {
            log::error!("Entry store not found");
            return None;
        }

        let entry_store = entry_store.unwrap();
        if entry_store.get_entries().is_empty() {
            None
        } else {
            Some(entry_store.to_hls_playlist(PlatformType::BiliBili, self.room_id, live_id, false))
        }
    }

    async fn reset(&self) {
        *self.hls_playlist.write().await = None;
        *self.live_id.write().await = String::new();
        *self.last_update.write().await = Utc::now().timestamp();
    }

    async fn get_work_dir(&self, live_id: &str) -> PathBuf {
        let cache_dir = self.config.read().await.cache.clone();
        Path::new(&cache_dir)
            .join("douyin")
            .join(self.room_id.to_string())
            .join(live_id)
    }

    async fn get_best_stream_url(
        &self,
        room_info: &response::DouyinRoomInfoResponse,
    ) -> Option<String> {
        let stream_url = room_info.data.data[0]
            .stream_url
            .as_ref()
            .unwrap()
            .hls_pull_url_map
            .clone();
        if let Some(url) = stream_url.full_hd1 {
            Some(url)
        } else if let Some(url) = stream_url.hd1 {
            Some(url)
        } else if let Some(url) = stream_url.sd1 {
            Some(url)
        } else {
            stream_url.sd2
        }
    }

    async fn update_entries(&self) -> Result<u128, RecorderError> {
        let task_begin_time = std::time::Instant::now();

        // Get current room info and stream URL
        let room_info = self.room_info.read().await;

        if room_info.is_none() {
            return Err(RecorderError::NoRoomInfo);
        }

        if self.stream_url.read().await.is_none() {
            let new_stream_url = self.get_best_stream_url(room_info.as_ref().unwrap()).await;
            if new_stream_url.is_none() {
                return Err(RecorderError::NoStreamAvailable);
            }
            log::info!("New douyin stream URL: {}", new_stream_url.clone().unwrap());
            *self.stream_url.write().await = Some(new_stream_url.unwrap());
        }
        let stream_url = self.stream_url.read().await.as_ref().unwrap().clone();

        // Get m3u8 playlist
        let (playlist, updated_stream_url) = self.client.get_m3u8_content(&stream_url).await?;

        *self.stream_url.write().await = Some(updated_stream_url);

        let mut new_segment_fetched = false;
        let work_dir = self.get_work_dir(self.live_id.read().await.as_str()).await;

        // Create work directory if not exists
        tokio::fs::create_dir_all(&work_dir).await?;

        if self.hls_playlist.read().await.is_none() {
            let mut new_playlist = HLSPlaylist::from(&playlist);
            new_playlist.segments.clear();
            *self.hls_playlist.write().await = Some(new_playlist);
        }

        let last_sequence = self
            .hls_playlist
            .read()
            .await
            .as_ref()
            .unwrap()
            .last_sequence()
            .unwrap_or(0);

        let mut new_segment_size = 0;
        for (i, segment) in playlist.segments.iter().enumerate() {
            let current_sequence = playlist.media_sequence + i as u64;
            if current_sequence <= last_sequence {
                continue;
            }

            new_segment_fetched = true;
            let mut uri = segment.uri.clone();
            // if uri contains ?params, remove it
            if let Some(pos) = uri.find('?') {
                uri = uri[..pos].to_string();
            }

            let ts_url = if uri.starts_with("http") {
                uri.clone()
            } else {
                // Get the base URL without the filename and query parameters
                let base_url = stream_url
                    .rfind('/')
                    .map(|i| &stream_url[..=i])
                    .unwrap_or(&stream_url);
                // Get the query parameters
                let query = stream_url.find('?').map(|i| &stream_url[i..]).unwrap_or("");
                // Combine: base_url + new_filename + query_params
                format!("{}{}{}", base_url, uri, query)
            };

            let file_name = format!("{}.ts", current_sequence);

            // Download segment
            match self
                .client
                .download_ts(&ts_url, &work_dir.join(&file_name))
                .await
            {
                Ok(size) => {
                    new_segment_size += size;

                    let mut new_segment = segment.clone();
                    new_segment.program_date_time = Some(Utc::now().into());
                    new_segment.uri = file_name;
                    self.hls_playlist
                        .write()
                        .await
                        .as_mut()
                        .unwrap()
                        .append_segement(new_segment);

                    self.hls_playlist
                        .write()
                        .await
                        .as_mut()
                        .unwrap()
                        .update_last_sequence(current_sequence);

                    self.save_playlist().await;
                }
                Err(e) => {
                    log::error!("Failed to download segment: {}", e);
                }
            }
        }

        if new_segment_fetched {
            *self.last_update.write().await = Utc::now().timestamp();
            let total_length = self
                .hls_playlist
                .read()
                .await
                .as_ref()
                .unwrap()
                .total_duration();
            // update record in database
            self.db
                .update_record(
                    self.live_id.read().await.as_str(),
                    total_length as i64,
                    new_segment_size,
                )
                .await?;
        }

        Ok(task_begin_time.elapsed().as_millis())
    }

    async fn save_playlist(&self) {
        let work_dir = self.get_work_dir(&self.live_id.read().await).await;
        let playlist_filepath = Path::new(&work_dir).join("index.m3u8");
        if let Some(playlist) = self.hls_playlist.read().await.as_ref() {
            let mut file = File::create(&playlist_filepath).await.unwrap();
            let playlist_content = playlist.output(true);
            file.write_all(playlist_content.as_bytes()).await.unwrap();
            file.flush().await.unwrap();
            // close file
            drop(file);
        }
    }
}

#[async_trait]
impl Recorder for DouyinRecorder {
    async fn run(&self) {
        *self.running.write().await = true;

        let self_clone = self.clone();
        tokio::spawn(async move {
            while *self_clone.running.read().await {
                if self_clone.check_status().await {
                    // Live status is ok, start recording
                    while self_clone.should_record().await {
                        match self_clone.update_entries().await {
                            Ok(ms) => {
                                if ms < 1000 {
                                    tokio::time::sleep(Duration::from_millis(1000 - ms as u64))
                                        .await;
                                } else {
                                    log::warn!(
                                        "[{}]Update entries cost too long: {}ms",
                                        self_clone.room_id,
                                        ms
                                    );
                                }
                                *self_clone.is_recording.write().await = true;
                            }
                            Err(e) => {
                                log::error!("[{}]Update entries error: {}", self_clone.room_id, e);
                                break;
                            }
                        }
                    }
                    *self_clone.is_recording.write().await = false;
                    // Check status again after 2-5 seconds
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue;
                }
                // Check live status every 10s
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
            log::info!("recording thread {} quit.", self_clone.room_id);
        });
    }

    async fn stop(&self) {
        *self.running.write().await = false;
    }

    async fn m3u8_content(&self, live_id: &str, start: i64, end: i64) -> String {
        if live_id != self.live_id.read().await.as_str() {
            self.load_previous_playlist(live_id)
                .await
                .unwrap_or_else(HLSPlaylist::new)
                .output(true)
        } else {
            self.hls_playlist
                .read()
                .await
                .as_ref()
                .unwrap_or(&HLSPlaylist::new())
                .output(false)
        }
    }

    async fn info(&self) -> RecorderInfo {
        let room_info = self.room_info.read().await;
        let room_cover_url = room_info
            .as_ref()
            .and_then(|info| {
                info.data
                    .data
                    .first()
                    .and_then(|data| data.cover.as_ref())
                    .map(|cover| cover.url_list[0].clone())
            })
            .unwrap_or_default();
        let room_title = room_info
            .as_ref()
            .and_then(|info| info.data.data.first().map(|data| data.title.clone()))
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
                    .map(|info| info.data.user.sec_uid.clone())
                    .unwrap_or_default(),
                user_name: room_info
                    .as_ref()
                    .map(|info| info.data.user.nickname.clone())
                    .unwrap_or_default(),
                user_avatar: room_info
                    .as_ref()
                    .map(|info| info.data.user.avatar_thumb.url_list[0].clone())
                    .unwrap_or_default(),
            },
            total_length: self
                .hls_playlist
                .read()
                .await
                .as_ref()
                .map_or(0.0, |p| p.total_duration()),
            current_live_id: self.live_id.read().await.clone(),
            live_status: *self.live_status.read().await == LiveStatus::Live,
            is_recording: *self.is_recording.read().await,
            auto_start: *self.auto_start.read().await,
            platform: PlatformType::Douyin.as_str().to_string(),
        }
    }

    async fn comments(&self, _live_id: &str) -> Result<Vec<DanmuEntry>, RecorderError> {
        Ok(vec![])
    }

    async fn is_recording(&self, live_id: &str) -> bool {
        *self.live_id.read().await == live_id && *self.live_status.read().await == LiveStatus::Live
    }

    async fn force_start(&self) {
        *self.current_record.write().await = true;
    }

    async fn force_stop(&self) {
        *self.current_record.write().await = false;
    }

    async fn set_auto_start(&self, auto_start: bool) {
        *self.auto_start.write().await = auto_start;
    }
}
