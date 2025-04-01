pub mod client;
mod response;
use super::entry::{EntryStore, TsEntry};
use super::{
    danmu::DanmuEntry, errors::RecorderError, PlatformType, Recorder, RecorderInfo, RoomInfo,
    UserInfo,
};
use crate::database::Database;
use crate::ffmpeg::{transcode, TranscodeConfig};
use crate::progress_event::{ProgressReporter, ProgressReporterTrait};
use crate::{config::Config, database::account::AccountRow};
use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use client::DouyinClientError;
use dashmap::DashMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;
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
    pub entry_store: Arc<RwLock<Option<EntryStore>>>,
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
            entry_store: Arc::new(RwLock::new(None)),
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
                        *self.live_id.write().await = info.data.data[0].id_str.clone();
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

                        // setup entry store
                        let work_dir = self.get_work_dir(self.live_id.read().await.as_str()).await;
                        let entry_store = EntryStore::new(&work_dir).await;
                        *self.entry_store.write().await = Some(entry_store);
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

    async fn reset(&self) {
        *self.entry_store.write().await = None;
        *self.live_id.write().await = String::new();
        *self.last_update.write().await = Utc::now().timestamp();
    }

    async fn get_work_dir(&self, live_id: &str) -> String {
        format!(
            "{}/douyin/{}/{}/",
            self.config.read().await.cache,
            self.room_id,
            live_id
        )
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

        let last_sequence = self
            .entry_store
            .read()
            .await
            .as_ref()
            .unwrap()
            .last_sequence();
        let continue_sequence = self
            .entry_store
            .read()
            .await
            .as_ref()
            .unwrap()
            .continue_sequence;
        let mut sequence = playlist.media_sequence + continue_sequence;

        for segment in playlist.segments {
            if sequence <= last_sequence {
                sequence += 1;
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

            let file_name = format!("{}.ts", sequence);

            // Download segment
            match self
                .client
                .download_ts(&ts_url, &format!("{}/{}", work_dir, file_name))
                .await
            {
                Ok(size) => {
                    let ts_entry = TsEntry {
                        url: file_name,
                        sequence,
                        length: segment.duration as f64,
                        size,
                        ts: Utc::now().timestamp(),
                        is_header: false,
                    };

                    self.entry_store
                        .write()
                        .await
                        .as_mut()
                        .unwrap()
                        .add_entry(ts_entry)
                        .await;
                }
                Err(e) => {
                    log::error!("Failed to download segment: {}", e);
                }
            }

            sequence += 1;
        }

        if new_segment_fetched {
            *self.last_update.write().await = Utc::now().timestamp();
            self.update_record().await;
        }

        Ok(task_begin_time.elapsed().as_millis())
    }

    async fn update_record(&self) {
        if let Err(e) = self
            .db
            .update_record(
                self.live_id.read().await.as_str(),
                self.entry_store
                    .read()
                    .await
                    .as_ref()
                    .unwrap()
                    .total_duration() as i64,
                self.entry_store.read().await.as_ref().unwrap().total_size(),
            )
            .await
        {
            log::error!("Failed to update record: {}", e);
        }
    }

    async fn generate_m3u8(&self, live_id: &str, start: i64, end: i64) -> String {
        let mut m3u8_content = "#EXTM3U\n".to_string();
        let range_required = start != 0 || end != 0;
        m3u8_content += "#EXT-X-VERSION:3\n";

        // if requires a range, we need to filter entries and only use entries in the range, so m3u8 type is VOD.
        let entries = if !range_required && live_id == *self.live_id.read().await {
            m3u8_content += "#EXT-X-PLAYLIST-TYPE:EVENT\n";
            self.entry_store
                .read()
                .await
                .as_ref()
                .unwrap()
                .get_entries()
                .clone()
        } else {
            m3u8_content += "#EXT-X-PLAYLIST-TYPE:VOD\n";
            let work_dir = self.get_work_dir(live_id).await;
            let entry_store = EntryStore::new(&work_dir).await;
            entry_store.get_entries().clone()
        };

        m3u8_content += "#EXT-X-OFFSET:0\n";

        if entries.is_empty() {
            return m3u8_content;
        }

        m3u8_content += &format!(
            "#EXT-X-TARGETDURATION:{}\n",
            entries.last().unwrap().length as u64
        );

        let mut previous_seq = entries.first().unwrap().sequence;
        let first_entry_ts = entries.first().unwrap().ts;
        for entry in entries {
            if range_required
                && (entry.ts - first_entry_ts < start || entry.ts - first_entry_ts > end)
            {
                continue;
            }
            if entry.sequence - previous_seq > 1 {
                m3u8_content += "#EXT-X-DISCONTINUITY\n";
            }
            previous_seq = entry.sequence;
            let date_str = Utc.timestamp_opt(entry.ts, 0).unwrap().to_rfc3339();
            m3u8_content += &format!("#EXT-X-PROGRAM-DATE-TIME:{}\n", date_str);
            m3u8_content += &format!("#EXTINF:{:.2},\n", entry.length);
            m3u8_content += &format!("/douyin/{}/{}/{}\n", self.room_id, live_id, entry.url);
        }

        if *self.live_status.read().await != LiveStatus::Live || range_required {
            m3u8_content += "#EXT-X-ENDLIST\n";
        }

        m3u8_content
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

    async fn clip_range(
        &self,
        reporter: &ProgressReporter,
        live_id: &str,
        x: f64,
        y: f64,
        clip_file: PathBuf,
    ) -> Result<PathBuf, RecorderError> {
        let work_dir = self.get_work_dir(live_id).await;
        let entries = if live_id == *self.live_id.read().await {
            self.entry_store
                .read()
                .await
                .as_ref()
                .unwrap()
                .get_entries()
                .clone()
        } else {
            let entry_store = EntryStore::new(&work_dir).await;
            entry_store.get_entries().clone()
        };

        if entries.is_empty() {
            return Err(RecorderError::EmptyCache);
        }

        let mut file_list = Vec::new();

        let mut offset = 0.0;
        for entry in entries {
            if offset >= x && offset <= y {
                file_list.push(format!("{}/{}", work_dir, entry.url));
            }
            offset += entry.length;

            if offset > y {
                break;
            }
        }

        let tmp_file = clip_file.with_extension("tmp");
        // Merge ts files
        let mut tmpf = tokio::fs::File::create(&tmp_file)
            .await
            .map_err(|e| RecorderError::IoError { err: e })?;
        for (i, file_path) in file_list.iter().enumerate() {
            if reporter.cancel.load(Ordering::Relaxed) {
                return Err(RecorderError::ClipError {
                    err: "Cancelled".to_string(),
                });
            }

            if let Ok(mut file) = tokio::fs::File::open(file_path).await {
                let mut buffer = Vec::new();
                if file.read_to_end(&mut buffer).await.is_ok() {
                    let _ = tmpf.write_all(&buffer).await;
                }
            }

            reporter.update(
                format!(
                    "生成中：{:.1}%",
                    (i + 1) as f64 * 100.0 / file_list.len() as f64
                )
                .as_str(),
            );
        }
        tmpf.flush()
            .await
            .map_err(|e| RecorderError::IoError { err: e })?;

        let transcode_config = TranscodeConfig {
            input_path: tmp_file.clone(),
            input_format: "mpegts".into(),
            output_path: clip_file,
        };

        let transcode_result = transcode(reporter, transcode_config).await;
        if let Err(e) = transcode_result {
            return Err(RecorderError::ClipError { err: e.to_string() });
        }

        // delete the original ts file
        tokio::fs::remove_file(tmp_file).await?;

        Ok(transcode_result.unwrap().output_path)
    }

    async fn m3u8_content(&self, live_id: &str, start: i64, end: i64) -> String {
        let cache_key = format!("{}:{}:{}", live_id, start, end);
        if let Some(cached) = self.m3u8_cache.get(&cache_key) {
            return cached.clone();
        }
        let m3u8_content = self.generate_m3u8(live_id, start, end).await;
        self.m3u8_cache.insert(cache_key, m3u8_content.clone());
        m3u8_content
    }

    async fn info(&self) -> RecorderInfo {
        let room_info = self.room_info.read().await;
        let room_cover_url = room_info
            .as_ref()
            .and_then(|info| info.data.data[0].cover.as_ref())
            .map(|cover| cover.url_list[0].clone())
            .unwrap_or_default();
        RecorderInfo {
            room_id: self.room_id,
            room_info: RoomInfo {
                room_id: self.room_id,
                room_title: room_info
                    .as_ref()
                    .map(|info| info.data.data[0].title.clone())
                    .unwrap_or_default(),
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
            total_length: if let Some(store) = self.entry_store.read().await.as_ref() {
                store.total_duration()
            } else {
                0.0
            },
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
