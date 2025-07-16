pub mod client;
mod response;
mod stream_info;
use super::entry::{EntryStore, Range, TsEntry};
use super::{
    danmu::DanmuEntry, errors::RecorderError, PlatformType, Recorder, RecorderInfo, RoomInfo,
    UserInfo,
};
use crate::database::Database;
use crate::progress_manager::Event;
use crate::progress_reporter::EventEmitter;
use crate::recorder_manager::RecorderEvent;
use crate::{config::Config, database::account::AccountRow};
use async_trait::async_trait;
use chrono::Utc;
use client::DouyinClientError;
use danmu_stream::danmu_stream::DanmuStream;
use danmu_stream::provider::ProviderType;
use danmu_stream::DanmuMessageType;
use rand::random;
use std::sync::Arc;
use std::time::Duration;
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
    #[cfg(not(feature = "headless"))]
    app_handle: AppHandle,
    emitter: EventEmitter,
    client: client::DouyinClient,
    db: Arc<Database>,
    account: AccountRow,
    room_id: u64,
    room_info: Arc<RwLock<Option<response::DouyinRoomInfoResponse>>>,
    stream_url: Arc<RwLock<Option<String>>>,
    entry_store: Arc<RwLock<Option<EntryStore>>>,
    danmu_store: Arc<RwLock<Option<DanmuStorage>>>,
    live_id: Arc<RwLock<String>>,
    live_status: Arc<RwLock<LiveStatus>>,
    is_recording: Arc<RwLock<bool>>,
    running: Arc<RwLock<bool>>,
    last_update: Arc<RwLock<i64>>,
    config: Arc<RwLock<Config>>,
    live_end_channel: broadcast::Sender<RecorderEvent>,
    enabled: Arc<RwLock<bool>>,

    danmu_stream_task: Arc<Mutex<Option<JoinHandle<()>>>>,
    danmu_task: Arc<Mutex<Option<JoinHandle<()>>>>,
    record_task: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl DouyinRecorder {
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        #[cfg(not(feature = "headless"))] app_handle: AppHandle,
        emitter: EventEmitter,
        room_id: u64,
        config: Arc<RwLock<Config>>,
        account: &AccountRow,
        db: &Arc<Database>,
        enabled: bool,
        channel: broadcast::Sender<RecorderEvent>,
    ) -> Result<Self, super::errors::RecorderError> {
        let client = client::DouyinClient::new(account);
        let room_info = client.get_room_info(room_id).await?;
        let mut live_status = LiveStatus::Offline;
        if room_info.data.room_status == 0 {
            live_status = LiveStatus::Live;
        }

        Ok(Self {
            #[cfg(not(feature = "headless"))]
            app_handle,
            emitter,
            db: db.clone(),
            account: account.clone(),
            room_id,
            live_id: Arc::new(RwLock::new(String::new())),
            entry_store: Arc::new(RwLock::new(None)),
            danmu_store: Arc::new(RwLock::new(None)),
            client,
            room_info: Arc::new(RwLock::new(Some(room_info))),
            stream_url: Arc::new(RwLock::new(None)),
            live_status: Arc::new(RwLock::new(live_status)),
            running: Arc::new(RwLock::new(false)),
            is_recording: Arc::new(RwLock::new(false)),
            enabled: Arc::new(RwLock::new(enabled)),
            last_update: Arc::new(RwLock::new(Utc::now().timestamp())),
            config,
            live_end_channel: channel,

            danmu_stream_task: Arc::new(Mutex::new(None)),
            danmu_task: Arc::new(Mutex::new(None)),
            record_task: Arc::new(Mutex::new(None)),
        })
    }

    async fn should_record(&self) -> bool {
        if !*self.running.read().await {
            return false;
        }

        *self.enabled.read().await
    }

    async fn check_status(&self) -> bool {
        match self.client.get_room_info(self.room_id).await {
            Ok(info) => {
                let live_status = info.data.room_status == 0; // room_status == 0 表示正在直播

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
                                info.data.user.nickname, info.data.data[0].title
                            ))
                            .show()
                            .unwrap();
                    } else {
                        #[cfg(not(feature = "headless"))]
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
                        let _ = self.live_end_channel.send(RecorderEvent::LiveEnd {
                            platform: PlatformType::Douyin,
                            room_id: self.room_id,
                            live_id: self.live_id.read().await.clone(),
                        });
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

                    // setup danmu store
                    let danmu_file_path = format!("{}{}", work_dir, "danmu.txt");
                    let danmu_store = DanmuStorage::new(&danmu_file_path).await;
                    *self.danmu_store.write().await = danmu_store;

                    // start danmu task
                    if let Some(danmu_task) = self.danmu_task.lock().await.as_mut() {
                        danmu_task.abort();
                    }
                    if let Some(danmu_stream_task) = self.danmu_stream_task.lock().await.as_mut() {
                        danmu_stream_task.abort();
                    }
                    let live_id = self.live_id.read().await.clone();
                    let self_clone = self.clone();
                    *self.danmu_task.lock().await = Some(tokio::spawn(async move {
                        log::info!("Start fetching danmu for live {}", live_id);
                        let _ = self_clone.danmu().await;
                    }));
                }

                true
            }
            Err(e) => {
                log::error!("[{}]Update room status failed: {}", self.room_id, e);
                *self.live_status.read().await == LiveStatus::Live
            }
        }
    }

    async fn danmu(&self) -> Result<(), super::errors::RecorderError> {
        let cookies = self.account.cookies.clone();
        let live_id = self
            .live_id
            .read()
            .await
            .clone()
            .parse::<u64>()
            .unwrap_or(0);
        let danmu_stream = DanmuStream::new(ProviderType::Douyin, &cookies, live_id).await;
        if danmu_stream.is_err() {
            let err = danmu_stream.err().unwrap();
            log::error!("Failed to create danmu stream: {}", err);
            return Err(super::errors::RecorderError::DanmuStreamError { err });
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
                        self.emitter.emit(&Event::DanmuReceived {
                            room: self.room_id,
                            ts: danmu.timestamp,
                            content: danmu.message.clone(),
                        });
                        if let Some(storage) = self.danmu_store.read().await.as_ref() {
                            storage.add_line(danmu.timestamp, &danmu.message).await;
                        }
                    }
                }
            } else {
                log::error!("Failed to receive danmu message");
                return Err(super::errors::RecorderError::DanmuStreamError {
                    err: danmu_stream::DanmuStreamError::WebsocketError {
                        err: "Failed to receive danmu message".to_string(),
                    },
                });
            }
        }
    }

    async fn reset(&self) {
        *self.entry_store.write().await = None;
        *self.live_id.write().await = String::new();
        *self.last_update.write().await = Utc::now().timestamp();
        *self.stream_url.write().await = None;
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
        let stream_data = room_info.data.data[0]
            .stream_url
            .as_ref()
            .unwrap()
            .live_core_sdk_data
            .pull_data
            .stream_data
            .clone();
        // parse stream_data into stream_info
        let stream_info = serde_json::from_str::<stream_info::StreamInfo>(&stream_data);
        if let Ok(stream_info) = stream_info {
            // find the best stream url
            if stream_info.data.origin.main.hls.is_empty() {
                log::error!("No stream url found in stream_data: {}", stream_data);
                return None;
            }

            Some(stream_info.data.origin.main.hls)
        } else {
            let err = stream_info.unwrap_err();
            log::error!("Failed to parse stream data: {} {}", err, stream_data);
            None
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

        let last_timestamp_mili = self
            .entry_store
            .read()
            .await
            .as_ref()
            .unwrap()
            .last_timestamp_mili;

        for segment in playlist.segments.iter() {
            let formated_ts_name = segment.uri.clone();
            let timestamp_mili = extract_timestamp_from(&formated_ts_name)
                .expect("No timestamp found in ts filename");

            if timestamp_mili <= last_timestamp_mili {
                continue;
            }

            // using timetamp_mili as sequence
            let sequence = timestamp_mili;

            new_segment_fetched = true;
            // example: pull-l3.douyincdn.com_stream-405850027547689439_or4-1752675567719.ts
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
                        ts: timestamp_mili as i64,
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
                    *self.stream_url.write().await = None;
                    return Err(e.into());
                }
            }
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
        log::debug!("Generate m3u8 for {live_id}:{start}:{end}");
        let range = if start != 0 || end != 0 {
            Some(Range {
                x: start as f32,
                y: end as f32,
            })
        } else {
            None
        };

        // if requires a range, we need to filter entries and only use entries in the range, so m3u8 type is VOD.
        if live_id == *self.live_id.read().await {
            self.entry_store
                .read()
                .await
                .as_ref()
                .unwrap()
                .manifest(range.is_some(), false, range)
        } else {
            let work_dir = self.get_work_dir(live_id).await;
            EntryStore::new(&work_dir)
                .await
                .manifest(true, false, range)
        }
    }
}

fn extract_timestamp_from(name: &str) -> Option<u64> {
    use regex::Regex;
    let re = Regex::new(r"-(\d+)\.ts$").ok()?;
    let captures = re.captures(name)?;
    captures.get(1)?.as_str().parse().ok()
}

#[async_trait]
impl Recorder for DouyinRecorder {
    async fn run(&self) {
        *self.running.write().await = true;

        let self_clone = self.clone();
        *self.record_task.lock().await = Some(tokio::spawn(async move {
            while *self_clone.running.read().await {
                let mut connection_fail_count = 0;
                if self_clone.check_status().await {
                    // Live status is ok, start recording
                    while self_clone.should_record().await {
                        match self_clone.update_entries().await {
                            Ok(ms) => {
                                if ms < 1000 {
                                    tokio::time::sleep(Duration::from_millis(1000 - ms as u64))
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
                                if let RecorderError::DouyinClientError { err: _e } = e {
                                    connection_fail_count =
                                        std::cmp::min(5, connection_fail_count + 1);
                                }
                                break;
                            }
                        }
                    }
                    *self_clone.is_recording.write().await = false;
                    // Check status again after some seconds
                    let secs = random::<u64>() % 5;
                    tokio::time::sleep(Duration::from_secs(
                        secs + 2_u64.pow(connection_fail_count),
                    ))
                    .await;
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
            let _ = danmu_task.abort();
        }
        if let Some(danmu_stream_task) = self.danmu_stream_task.lock().await.as_mut() {
            let _ = danmu_stream_task.abort();
        }
        if let Some(record_task) = self.record_task.lock().await.as_mut() {
            let _ = record_task.abort();
        }
        log::info!("Recorder for room {} quit.", self.room_id);
    }

    async fn m3u8_content(&self, live_id: &str, start: i64, end: i64) -> String {
        self.generate_m3u8(live_id, start, end).await
    }

    async fn master_m3u8(&self, live_id: &str, start: i64, end: i64) -> String {
        let mut m3u8_content = "#EXTM3U\n".to_string();
        m3u8_content += "#EXT-X-VERSION:6\n";
        m3u8_content += format!(
            "#EXT-X-STREAM-INF:BANDWIDTH=1280000,RESOLUTION=1920x1080,CODECS=\"avc1.64001F,mp4a.40.2\",DANMU={}\n",
            self.first_segment_ts(live_id).await / 1000
        )
        .as_str();
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
            total_length: if let Some(store) = self.entry_store.read().await.as_ref() {
                store.total_duration()
            } else {
                0.0
            },
            current_live_id: self.live_id.read().await.clone(),
            live_status: *self.live_status.read().await == LiveStatus::Live,
            is_recording: *self.is_recording.read().await,
            auto_start: *self.enabled.read().await,
            platform: PlatformType::Douyin.as_str().to_string(),
        }
    }

    async fn comments(&self, live_id: &str) -> Result<Vec<DanmuEntry>, RecorderError> {
        Ok(if live_id == *self.live_id.read().await {
            // just return current cache content
            match self.danmu_store.read().await.as_ref() {
                Some(storage) => storage.get_entries().await,
                None => Vec::new(),
            }
        } else {
            // load disk cache
            let cache_file_path = format!(
                "{}/douyin/{}/{}/{}",
                self.config.read().await.cache,
                self.room_id,
                live_id,
                "danmu.txt"
            );
            log::debug!("loading danmu cache from {}", cache_file_path);
            let storage = DanmuStorage::new(&cache_file_path).await;
            if storage.is_none() {
                return Ok(Vec::new());
            }
            let storage = storage.unwrap();
            storage.get_entries().await
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
