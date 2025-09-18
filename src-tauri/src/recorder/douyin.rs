pub mod client;
mod response;
mod stream_info;
use super::entry::{EntryStore, Range, TsEntry};
use super::{
    danmu::DanmuEntry, errors::RecorderError, PlatformType, Recorder, RecorderInfo, RoomInfo,
    UserInfo,
};
use crate::database::Database;
use crate::progress::progress_manager::Event;
use crate::progress::progress_reporter::EventEmitter;
use crate::recorder_manager::RecorderEvent;
use crate::subtitle_generator::item_to_srt;
use crate::{config::Config, database::account::AccountRow};
use async_trait::async_trait;
use chrono::Utc;
use client::DouyinClientError;
use danmu_stream::danmu_stream::DanmuStream;
use danmu_stream::provider::ProviderType;
use danmu_stream::DanmuMessageType;
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
    entry_store: Arc<RwLock<Option<EntryStore>>>,
    danmu_store: Arc<RwLock<Option<DanmuStorage>>>,
    live_id: Arc<RwLock<String>>,
    danmu_room_id: Arc<RwLock<String>>,
    live_status: Arc<RwLock<LiveStatus>>,
    is_recording: Arc<RwLock<bool>>,
    running: Arc<RwLock<bool>>,
    last_update: Arc<RwLock<i64>>,
    config: Arc<RwLock<Config>>,
    event_channel: broadcast::Sender<RecorderEvent>,
    enabled: Arc<RwLock<bool>>,

    danmu_stream_task: Arc<Mutex<Option<JoinHandle<()>>>>,
    danmu_task: Arc<Mutex<Option<JoinHandle<()>>>>,
    record_task: Arc<Mutex<Option<JoinHandle<()>>>>,
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

fn parse_stream_url(stream_url: &str) -> (String, String) {
    // Parse stream URL to extract base URL and query parameters
    // Example: http://7167739a741646b4651b6949b2f3eb8e.livehwc3.cn/pull-hls-l26.douyincdn.com/third/stream-693342996808860134_or4.m3u8?sub_m3u8=true&user_session_id=16090eb45ab8a2f042f7c46563936187&major_anchor_level=common&edge_slice=true&expire=67d944ec&sign=47b95cc6e8de20d82f3d404412fa8406

    let base_url = stream_url
        .rfind('/')
        .map_or(stream_url, |i| &stream_url[..=i])
        .to_string();

    let query_params = stream_url
        .find('?')
        .map_or("", |i| &stream_url[i..])
        .to_string();

    (base_url, query_params)
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
            danmu_room_id: Arc::new(RwLock::new(String::new())),
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
            event_channel: channel,

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
                    (*self.danmu_room_id.write().await).clone_from(&info.room_id_str);
                }

                true
            }
            Err(e) => {
                if let DouyinClientError::H5NotLive(e) = e {
                    log::warn!("[{}]Live maybe not started: {}", self.room_id, e);
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
            .danmu_room_id
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
        *self.entry_store.write().await = None;
        *self.danmu_room_id.write().await = String::new();
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

    async fn update_entries(&self) -> Result<u128, RecorderError> {
        let task_begin_time = std::time::Instant::now();

        // Get current room info and stream URL
        let room_info = self.room_info.read().await;

        if room_info.is_none() {
            return Err(RecorderError::NoRoomInfo);
        }

        if self.stream_url.read().await.is_none() {
            return Err(RecorderError::NoStreamAvailable);
        }

        let mut stream_url = self.stream_url.read().await.as_ref().unwrap().clone();

        // Get m3u8 playlist
        let (playlist, updated_stream_url) = self.client.get_m3u8_content(&stream_url).await?;

        *self.stream_url.write().await = Some(updated_stream_url.clone());
        stream_url = updated_stream_url;

        let mut new_segment_fetched = false;
        let mut is_first_segment = self.entry_store.read().await.is_none();
        let work_dir;

        // If this is the first segment, prepare but don't create directories yet
        if is_first_segment {
            // Generate live_id for potential use
            let live_id = Utc::now().timestamp_millis().to_string();
            *self.live_id.write().await = live_id.clone();
            work_dir = self.get_work_dir(&live_id).await;
        } else {
            work_dir = self.get_work_dir(self.live_id.read().await.as_str()).await;
        }

        let last_sequence = if is_first_segment {
            0
        } else {
            self.entry_store
                .read()
                .await
                .as_ref()
                .unwrap()
                .last_sequence
        };

        for segment in &playlist.segments {
            let formatted_ts_name = segment.uri.clone();
            let sequence = extract_sequence_from(&formatted_ts_name);
            if sequence.is_none() {
                log::error!("No timestamp extracted from douyin ts name: {formatted_ts_name}");
                continue;
            }

            let sequence = sequence.unwrap();
            if sequence <= last_sequence {
                continue;
            }

            // example: pull-l3.douyincdn.com_stream-405850027547689439_or4-1752675567719.ts
            let uri = segment.uri.clone();

            let ts_url = if uri.starts_with("http") {
                uri.clone()
            } else {
                // Parse the stream URL to extract base URL and query parameters
                let (base_url, query_params) = parse_stream_url(&stream_url);

                // Check if the segment URI already has query parameters
                if uri.contains('?') {
                    // If segment URI has query params, append m3u8 query params with &
                    format!("{}{}&{}", base_url, uri, &query_params[1..]) // Remove leading ? from query_params
                } else {
                    // If segment URI has no query params, append m3u8 query params with ?
                    format!("{base_url}{uri}{query_params}")
                }
            };

            // Download segment with retry mechanism
            let mut retry_count = 0;
            let max_retries = 3;
            let mut download_success = false;
            let mut work_dir_created = false;

            while retry_count < max_retries && !download_success {
                let file_name = format!("{sequence}.ts");
                let file_path = format!("{work_dir}/{file_name}");

                // If this is the first segment, create work directory before first download attempt
                if is_first_segment && !work_dir_created {
                    // Create work directory only when we're about to download
                    if let Err(e) = tokio::fs::create_dir_all(&work_dir).await {
                        log::error!("Failed to create work directory: {e}");
                        return Err(e.into());
                    }
                    work_dir_created = true;
                }

                match self.client.download_ts(&ts_url, &file_path).await {
                    Ok(size) => {
                        if size == 0 {
                            log::error!("Download segment failed (empty response): {ts_url}");
                            retry_count += 1;
                            if retry_count < max_retries {
                                tokio::time::sleep(Duration::from_millis(500)).await;
                                continue;
                            }
                            break;
                        }

                        // If this is the first successful download, create record and initialize stores
                        if is_first_segment {
                            // Create database record
                            let room_info = room_info.as_ref().unwrap();
                            let cover_url = room_info.cover.clone();
                            let room_cover_path = Path::new(PlatformType::Douyin.as_str())
                                .join(self.room_id.to_string())
                                .join("cover.jpg");
                            if let Some(url) = cover_url {
                                let full_room_cover_path =
                                    Path::new(&self.config.read().await.cache)
                                        .join(&room_cover_path);
                                let _ =
                                    self.client.download_file(&url, &full_room_cover_path).await;
                            }

                            if let Err(e) = self
                                .db
                                .add_record(
                                    PlatformType::Douyin,
                                    self.live_id.read().await.as_str(),
                                    self.room_id,
                                    &room_info.room_title,
                                    Some(room_cover_path.to_str().unwrap().to_string()),
                                    None,
                                )
                                .await
                            {
                                log::error!("Failed to add record: {e}");
                            }

                            let _ = self.event_channel.send(RecorderEvent::RecordStart {
                                recorder: self.info().await,
                            });

                            // Setup entry store
                            let entry_store = EntryStore::new(&work_dir).await;
                            *self.entry_store.write().await = Some(entry_store);

                            // Setup danmu store
                            let danmu_file_path = format!("{}{}", work_dir, "danmu.txt");
                            let danmu_store = DanmuStorage::new(&danmu_file_path).await;
                            *self.danmu_store.write().await = danmu_store;

                            // Start danmu task
                            if let Some(danmu_task) = self.danmu_task.lock().await.as_mut() {
                                danmu_task.abort();
                            }
                            if let Some(danmu_stream_task) =
                                self.danmu_stream_task.lock().await.as_mut()
                            {
                                danmu_stream_task.abort();
                            }
                            let live_id = self.live_id.read().await.clone();
                            let self_clone = self.clone();
                            *self.danmu_task.lock().await = Some(tokio::spawn(async move {
                                log::info!("Start fetching danmu for live {live_id}");
                                let _ = self_clone.danmu().await;
                            }));

                            is_first_segment = false;
                        }

                        let ts_entry = TsEntry {
                            url: file_name,
                            sequence,
                            length: f64::from(segment.duration),
                            size,
                            ts: Utc::now().timestamp_millis(),
                            is_header: false,
                        };

                        self.entry_store
                            .write()
                            .await
                            .as_mut()
                            .unwrap()
                            .add_entry(ts_entry)
                            .await;

                        new_segment_fetched = true;
                        download_success = true;
                    }
                    Err(e) => {
                        log::warn!(
                            "Failed to download segment (attempt {}/{}): {} - URL: {}",
                            retry_count + 1,
                            max_retries,
                            e,
                            ts_url
                        );
                        retry_count += 1;
                        if retry_count < max_retries {
                            tokio::time::sleep(Duration::from_millis(1000 * retry_count as u64))
                                .await;
                            continue;
                        }
                        // If all retries failed, check if it's a 400 error
                        if e.to_string().contains("400") {
                            log::error!(
                                "HTTP 400 error for segment, stream URL may be expired: {ts_url}"
                            );
                            *self.stream_url.write().await = None;

                            // Clean up empty directory if first segment failed
                            if is_first_segment && work_dir_created {
                                if let Err(cleanup_err) = tokio::fs::remove_dir_all(&work_dir).await
                                {
                                    log::warn!(
                                        "Failed to cleanup empty work directory {work_dir}: {cleanup_err}"
                                    );
                                }
                            }

                            return Err(RecorderError::NoStreamAvailable);
                        }

                        // Clean up empty directory if first segment failed
                        if is_first_segment && work_dir_created {
                            if let Err(cleanup_err) = tokio::fs::remove_dir_all(&work_dir).await {
                                log::warn!(
                                    "Failed to cleanup empty work directory {work_dir}: {cleanup_err}"
                                );
                            }
                        }

                        return Err(e.into());
                    }
                }
            }

            if !download_success {
                log::error!("Failed to download segment after {max_retries} retries: {ts_url}");

                // Clean up empty directory if first segment failed after all retries
                if is_first_segment && work_dir_created {
                    if let Err(cleanup_err) = tokio::fs::remove_dir_all(&work_dir).await {
                        log::warn!(
                            "Failed to cleanup empty work directory {work_dir}: {cleanup_err}"
                        );
                    }
                }
            }
        }

        if new_segment_fetched {
            *self.last_update.write().await = Utc::now().timestamp();
            self.update_record().await;
        }

        // if no new segment fetched for 10 seconds
        if *self.last_update.read().await + 10 < Utc::now().timestamp() {
            log::warn!("No new segment fetched for 10 seconds");
            *self.stream_url.write().await = None;
            *self.last_update.write().await = Utc::now().timestamp();
            return Err(RecorderError::NoStreamAvailable);
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
            log::error!("Failed to update record: {e}");
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

fn extract_sequence_from(name: &str) -> Option<u64> {
    use regex::Regex;
    let re = Regex::new(r"(\d+)\.ts").ok()?;
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
                                    tokio::time::sleep(Duration::from_millis(
                                        (1000 - ms).try_into().unwrap(),
                                    ))
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
                                if let RecorderError::DouyinClientError(_) = e {
                                    connection_fail_count =
                                        std::cmp::min(5, connection_fail_count + 1);
                                }
                                break;
                            }
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
        use std::fmt::Write;
        writeln!(&mut m3u8_content, "playlist.m3u8?start={start}&end={end}").unwrap();
        m3u8_content
    }

    async fn get_archive_subtitle(
        &self,
        live_id: &str,
    ) -> Result<String, super::errors::RecorderError> {
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
                "{}/douyin/{}/{}/{}",
                self.config.read().await.cache,
                self.room_id,
                live_id,
                "danmu.txt"
            );
            log::debug!("loading danmu cache from {cache_file_path}");
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
        *self.live_id.read().await == live_id && *self.live_status.read().await == LiveStatus::Live
    }

    async fn enable(&self) {
        *self.enabled.write().await = true;
    }

    async fn disable(&self) {
        *self.enabled.write().await = false;
    }
}
