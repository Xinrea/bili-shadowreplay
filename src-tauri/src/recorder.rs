pub mod bilibili;
pub mod danmu;
use async_std::{fs, stream::StreamExt};
use bilibili::{errors::BiliClientError, RoomInfo};
use bilibili::{BiliClient, BiliStream, StreamType, UserInfo};
use chrono::prelude::*;
use custom_error::custom_error;
use danmu::{DanmuEntry, DanmuStorage};
use dashmap::DashMap;
use felgens::{ws_socket_object, FelgensError, WsStreamMessageType};
use ffmpeg_sidecar::{
    command::FfmpegCommand,
    event::{FfmpegEvent, LogLevel},
};
use futures::future::join_all;
use m3u8_rs::Playlist;
use regex::Regex;
use std::sync::Arc;
use std::thread;
use tauri::{AppHandle, Emitter, Url};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::mpsc::{self, UnboundedReceiver};
use tokio::sync::{Mutex, RwLock};

use crate::db::{AccountRow, Database, DatabaseError, RecordRow};
use crate::Config;

#[derive(Clone)]
pub struct TsEntry {
    pub url: String,
    pub offset: u64,
    pub sequence: u64,
    pub length: f64,
    pub size: u64,
}

/// A recorder for BiliBili live streams
///
/// This recorder fetches, caches and serves TS entries, currently supporting only StreamType::FMP4.
/// As high-quality streams are accessible only to logged-in users, the use of a BiliClient, which manages cookies, is required.
// TODO implement StreamType::TS
#[derive(Clone)]
pub struct BiliRecorder {
    app_handle: AppHandle,
    client: Arc<RwLock<BiliClient>>,
    db: Arc<Database>,
    account: AccountRow,
    config: Arc<RwLock<Config>>,
    pub room_id: u64,
    pub room_info: Arc<RwLock<RoomInfo>>,
    pub user_info: Arc<RwLock<UserInfo>>,
    pub live_status: Arc<RwLock<bool>>,
    pub last_sequence: Arc<RwLock<u64>>,
    pub ts_length: Arc<RwLock<f64>>,
    pub timestamp: Arc<RwLock<u64>>,
    ts_entries: Arc<Mutex<Vec<TsEntry>>>,
    quit: Arc<Mutex<bool>>,
    header: Arc<RwLock<Option<TsEntry>>>,
    pub live_stream: Arc<RwLock<Option<BiliStream>>>,
    cache_size: Arc<RwLock<u64>>,
    danmu_storage: Arc<RwLock<Option<DanmuStorage>>>,
    m3u8_cache: DashMap<u64, String>,
}

custom_error! {pub RecorderError
    IndexNotFound {url: String} = "Index not found: {url}",
    EmptyCache = "Cache is empty",
    M3u8ParseFailed {content: String } = "Parse m3u8 content failed: {content}",
    NoStreamAvailable = "No available stream provided",
    InvalidStream {stream: BiliStream} = "Invalid stream: {stream}",
    EmptyHeader = "Header url is empty",
    InvalidTimestamp = "Header timestamp is invalid",
    InvalidDBOP {err: DatabaseError } = "Database error: {err}",
    ClientError {err: BiliClientError} = "BiliClient error: {err}",
}

impl From<DatabaseError> for RecorderError {
    fn from(value: DatabaseError) -> Self {
        RecorderError::InvalidDBOP { err: value }
    }
}

impl From<BiliClientError> for RecorderError {
    fn from(value: BiliClientError) -> Self {
        RecorderError::ClientError { err: value }
    }
}

impl BiliRecorder {
    pub async fn new(
        app_handle: AppHandle,
        webid: &str,
        db: &Arc<Database>,
        room_id: u64,
        account: &AccountRow,
        config: Arc<RwLock<Config>>,
    ) -> Result<Self, RecorderError> {
        let client = BiliClient::new()?;
        let room_info = client.get_room_info(account, room_id).await?;
        let user_info = client
            .get_user_info(webid, account, room_info.user_id)
            .await?;
        let mut live_status = false;
        let mut live_stream = None;
        if room_info.live_status == 1 {
            live_status = true;
            if let Ok(stream) = client.get_play_url(account, room_info.room_id).await {
                live_stream = Some(stream);
            } else {
                log::error!("[{}]Room is online but fetching stream failed", room_id);
            }
        }

        let recorder = Self {
            app_handle,
            client: Arc::new(RwLock::new(client)),
            db: db.clone(),
            account: account.clone(),
            config,
            room_id,
            room_info: Arc::new(RwLock::new(room_info)),
            user_info: Arc::new(RwLock::new(user_info)),
            live_status: Arc::new(RwLock::new(live_status)),
            last_sequence: Arc::new(RwLock::new(0)),
            ts_length: Arc::new(RwLock::new(0.0)),
            ts_entries: Arc::new(Mutex::new(Vec::new())),
            timestamp: Arc::new(RwLock::new(0)),
            quit: Arc::new(Mutex::new(false)),
            header: Arc::new(RwLock::new(None)),
            live_stream: Arc::new(RwLock::new(live_stream)),
            cache_size: Arc::new(RwLock::new(0)),
            danmu_storage: Arc::new(RwLock::new(None)),
            m3u8_cache: DashMap::new(),
        };
        log::info!("Recorder for room {} created.", room_id);
        Ok(recorder)
    }

    pub async fn reset(&self) {
        *self.ts_length.write().await = 0.0;
        *self.last_sequence.write().await = 0;
        self.ts_entries.lock().await.clear();
        *self.header.write().await = None;
        *self.timestamp.write().await = 0;
        *self.danmu_storage.write().await = None;
    }

    async fn check_status(&self) -> bool {
        if let Ok(room_info) = self
            .client
            .read()
            .await
            .get_room_info(&self.account, self.room_id)
            .await
        {
            *self.room_info.write().await = room_info.clone();
            let live_status = room_info.live_status == 1;

            // handle live notification
            if *self.live_status.read().await != live_status {
                if live_status {
                    if self.config.read().await.live_start_notify {
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
                } else if self.config.read().await.live_end_notify {
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
            }

            // if stream is confirmed to be closed, live stream cache is cleaned.
            // all request will go through fs
            if live_status {
                // no need to update stream as it's not expired yet
                if self
                    .live_stream
                    .read()
                    .await
                    .as_ref()
                    .is_some_and(|s| s.expire > Utc::now().timestamp())
                {
                    return live_status;
                }
                log::info!("[{}]Stream is empty or expired, updating", self.room_id);
                if let Ok(stream) = self
                    .client
                    .read()
                    .await
                    .get_play_url(&self.account, self.room_id)
                    .await
                {
                    *self.live_stream.write().await = Some(stream);
                }
            } else {
                self.reset().await;
            }
            *self.live_status.write().await = live_status;
            live_status
        } else {
            log::error!("[{}]Update room status failed", self.room_id);
            *self.live_status.write().await = true;
            // may encouter internet issues, not sure whether the stream is closed
            true
        }
    }

    pub async fn get_archives(&self) -> Result<Vec<RecordRow>, RecorderError> {
        Ok(self.db.get_records(self.room_id).await?)
    }

    pub async fn get_archive(&self, live_id: u64) -> Result<RecordRow, RecorderError> {
        Ok(self.db.get_record(self.room_id, live_id).await?)
    }

    pub async fn delete_archive(&self, ts: u64) {
        if let Err(e) = self.db.remove_record(ts).await {
            log::error!("remove archive failed: {}", e);
        } else {
            let target_dir = format!("{}/{}/{}", self.config.read().await.cache, self.room_id, ts);
            if fs::remove_dir_all(target_dir).await.is_err() {
                log::error!("remove archive failed [{}]{}", self.room_id, ts);
            }
        }
    }

    pub async fn run(&self) {
        let self_clone = self.clone();
        thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async move {
                while !*self_clone.quit.lock().await {
                    if self_clone.check_status().await {
                        // Live status is ok, start recording.
                        while !*self_clone.quit.lock().await {
                            if let Err(e) = self_clone.update_entries().await {
                                log::error!("[{}]Update entries error: {}", self_clone.room_id, e);
                                break;
                            }
                            thread::sleep(std::time::Duration::from_secs(1));
                        }
                        // go check status again
                        continue;
                    }
                    // Every 10s check live status.
                    thread::sleep(std::time::Duration::from_secs(10));
                }
                log::info!("recording thread {} quit.", self_clone.room_id);
            });
        });
        // Thread for danmaku
        let self_clone = self.clone();
        thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async move {
                self_clone.danmu().await;
            });
        });
    }

    async fn danmu(&self) {
        let (tx, rx) = mpsc::unbounded_channel();
        let cookies = self.account.cookies.clone();
        let uid: u64 = self.account.uid;
        let ws = ws_socket_object(tx, uid, self.room_id, cookies.as_str());
        if let Err(e) = tokio::select! {v = ws => v, v = self.recv(self.room_id,rx) => v} {
            log::debug!("{}", e);
        }
    }

    async fn recv(
        &self,
        room: u64,
        mut rx: UnboundedReceiver<WsStreamMessageType>,
    ) -> Result<(), FelgensError> {
        while let Some(msg) = rx.recv().await {
            if let WsStreamMessageType::DanmuMsg(msg) = msg {
                self.app_handle
                    .emit(
                        &format!("danmu:{}", room),
                        DanmuEntry {
                            ts: msg.timestamp,
                            content: msg.msg.clone(),
                        },
                    )
                    .unwrap();
                if *self.live_status.read().await {
                    // save danmu
                    if let Some(storage) = self.danmu_storage.write().await.as_ref() {
                        storage.add_line(msg.timestamp, &msg.msg).await;
                    }
                }
            }
        }
        Ok(())
    }

    async fn get_playlist(&self) -> Result<Playlist, RecorderError> {
        let stream = self.live_stream.read().await.clone();
        if stream.is_none() {
            return Err(RecorderError::NoStreamAvailable);
        }
        let stream = stream.unwrap();
        match self
            .client
            .read()
            .await
            .get_index_content(&stream.index())
            .await
        {
            Ok(index_content) => {
                if index_content.is_empty() {
                    return Err(RecorderError::InvalidStream { stream });
                }
                if index_content.contains("Not Found") {
                    return Err(RecorderError::IndexNotFound {
                        url: stream.index(),
                    });
                }
                m3u8_rs::parse_playlist_res(index_content.as_bytes()).map_err(|_| {
                    RecorderError::M3u8ParseFailed {
                        content: index_content.clone(),
                    }
                })
            }
            Err(e) => {
                log::error!("Failed fetching index content from {}", stream.index());
                return Err(RecorderError::ClientError { err: e });
            }
        }
    }

    async fn get_header_url(&self) -> Result<String, RecorderError> {
        let stream = self.live_stream.read().await.clone();
        if stream.is_none() {
            return Err(RecorderError::NoStreamAvailable);
        }
        let stream = stream.unwrap();
        let index_content = self
            .client
            .read()
            .await
            .get_index_content(&stream.index())
            .await?;
        if index_content.is_empty() {
            return Err(RecorderError::InvalidStream { stream });
        }
        if index_content.contains("Not Found") {
            return Err(RecorderError::IndexNotFound {
                url: stream.index(),
            });
        }
        if index_content.contains("BANDWIDTH") {
            // // this index content provides another m3u8 url
            // let new_url = index_content.lines().last().unwrap();
            // *self.m3u8_url.write().await = String::from(new_url);
            // return Box::pin(self.get_header_url()).await;
            log::error!("BANDWIDTH index content: {}", index_content);
            return Err(RecorderError::InvalidStream { stream });
        }
        let mut header_url = String::from("");
        let re = Regex::new(r"h.*\.m4s").unwrap();
        if let Some(captures) = re.captures(&index_content) {
            header_url = captures.get(0).unwrap().as_str().to_string();
        }
        if header_url.is_empty() {
            log::warn!("Parse header url failed: {}", index_content);
        }
        Ok(header_url)
    }

    async fn extract_timestamp(&self, header_url: &str) -> u64 {
        log::debug!("[{}]Extract timestamp from {}", self.room_id, header_url);
        let re = Regex::new(r"h(\d+).m4s").unwrap();
        if let Some(cap) = re.captures(header_url) {
            let ts = cap.get(1).unwrap().as_str().parse().unwrap();
            *self.timestamp.write().await = ts;
            ts
        } else {
            log::error!("Extract timestamp failed: {}", header_url);
            0
        }
    }

    async fn update_entries(&self) -> Result<(), RecorderError> {
        let current_stream = self.live_stream.read().await.clone();
        if current_stream.is_none() {
            return Err(RecorderError::NoStreamAvailable);
        }
        let current_stream = current_stream.unwrap();
        let parsed = self.get_playlist().await;
        let mut timestamp = *self.timestamp.read().await;
        let mut work_dir = format!(
            "{}/{}/{}/",
            self.config.read().await.cache,
            self.room_id,
            timestamp
        );
        // Check header if None
        if self.header.read().await.is_none() && current_stream.format == StreamType::FMP4 {
            // Get url from EXT-X-MAP
            let header_url = self.get_header_url().await?;
            if header_url.is_empty() {
                return Err(RecorderError::EmptyHeader);
            }
            timestamp = self.extract_timestamp(&header_url).await;
            if timestamp == 0 {
                log::error!("[{}]Parse timestamp failed: {}", self.room_id, header_url);
                return Err(RecorderError::InvalidTimestamp);
            }
            self.db
                .add_record(
                    timestamp,
                    self.room_id,
                    &self.room_info.read().await.room_title,
                )
                .await?;
            // now work dir is confirmed
            work_dir = format!(
                "{}/{}/{}/",
                self.config.read().await.cache,
                self.room_id,
                timestamp
            );
            // if folder is exisited, need to load previous data into cache
            if let Ok(meta) = fs::metadata(&work_dir).await {
                if meta.is_dir() {
                    log::warn!("Live {} is already cached. Try to restore", timestamp);
                    self.restore(&work_dir).await;
                } else {
                    // make sure work_dir is created
                    fs::create_dir_all(&work_dir).await.unwrap();
                }
            } else {
                // make sure work_dir is created
                fs::create_dir_all(&work_dir).await.unwrap();
            }
            // danmau file
            let danmu_file_path = format!("{}{}", work_dir, "danmu.txt");
            *self.danmu_storage.write().await = DanmuStorage::new(&danmu_file_path).await;
            let full_header_url = current_stream.ts_url(&header_url);
            let file_name = header_url.split('/').last().unwrap();
            let mut header = TsEntry {
                url: file_name.to_string(),
                offset: 0,
                sequence: 0,
                length: 0.0,
                size: 0,
            };
            // Download header
            match self
                .client
                .read()
                .await
                .download_ts(&full_header_url, &format!("{}/{}", work_dir, file_name))
                .await
            {
                Ok(size) => {
                    header.size = size;
                    *self.header.write().await = Some(header);
                    // add size into cache_size
                    *self.cache_size.write().await += size;
                }
                Err(e) => {
                    log::error!("Download header failed: {}", e);
                }
            }
        }
        match parsed {
            Ok(Playlist::MasterPlaylist(pl)) => log::debug!("Master playlist:\n{:?}", pl),
            Ok(Playlist::MediaPlaylist(pl)) => {
                let mut sequence = pl.media_sequence;
                let mut handles = Vec::new();
                for ts in pl.segments {
                    if sequence <= *self.last_sequence.read().await {
                        sequence += 1;
                        continue;
                    }
                    let mut offset_hex: String = "".into();
                    let mut seg_offset: u64 = 0;
                    for tag in ts.unknown_tags {
                        if tag.tag == "BILI-AUX" {
                            if let Some(rest) = tag.rest {
                                let parts: Vec<&str> = rest.split('|').collect();
                                if parts.len() == 0 {
                                    continue;
                                }
                                offset_hex = parts.get(0).unwrap().to_string();
                                seg_offset = u64::from_str_radix(&offset_hex, 16).unwrap();
                            }
                            break;
                        }
                    }
                    let ts_url = current_stream.ts_url(&ts.uri);
                    if !Url::parse(&ts_url).is_ok() {
                        log::error!("Ts url is invalid. ts_url={} original={}", ts_url, ts.uri);
                        continue;
                    }
                    // encode segment offset into filename
                    let mut entries = self.ts_entries.lock().await;
                    let file_name =
                        format!("{}-{}", &offset_hex, ts.uri.split('/').last().unwrap());
                    let mut ts_length = 1.0;
                    // calculate entry length using offset
                    // the default #EXTINF is 1.0, which is not accurate
                    if !entries.is_empty() {
                        // skip this entry as it is already in cache or stream changed
                        if seg_offset <= entries.last().unwrap().offset {
                            continue;
                        }
                        ts_length = (seg_offset - entries.last().unwrap().offset) as f64 / 1000.0;
                    }
                    let ts_entry = TsEntry {
                        url: file_name.clone(),
                        offset: seg_offset,
                        sequence,
                        length: ts_length,
                        size: 0,
                    };
                    let client = self.client.clone();
                    let work_dir = work_dir.clone();
                    let cache_size_clone = self.cache_size.clone();
                    handles.push(tokio::task::spawn(async move {
                        let file_name_clone = file_name.clone();
                        match client
                            .read()
                            .await
                            .download_ts(&ts_url, &format!("{}/{}", work_dir, file_name_clone))
                            .await
                        {
                            Ok(size) => {
                                *cache_size_clone.write().await += size;
                            }
                            Err(e) => {
                                log::error!("Download ts failed: {}", e);
                            }
                        }
                    }));
                    entries.push(ts_entry);
                    *self.last_sequence.write().await = sequence;
                    let mut total_length = self.ts_length.write().await;
                    *total_length += ts.duration as f64;
                    sequence += 1;
                }
                join_all(handles).await.into_iter().for_each(|e| {
                    if let Err(e) = e {
                        log::error!("Download ts failed: {:?}", e);
                    }
                });
                // currently we take every segement's length as 1.0s.
                self.db
                    .update_record(
                        timestamp,
                        self.ts_entries.lock().await.len() as i64,
                        *self.cache_size.read().await,
                    )
                    .await?;
            }
            Err(e) => {
                return Err(e);
            }
        }
        Ok(())
    }

    async fn restore(&self, work_dir: &str) {
        // by the way, header will be set after restore, so we don't need to restore it.
        let entries = self.get_fs_entries(work_dir).await;
        if entries.is_empty() {
            return;
        }
        self.ts_entries.lock().await.extend_from_slice(&entries);
        *self.ts_length.write().await = entries.len() as f64;
        *self.cache_size.write().await = entries.iter().map(|e| e.size).sum();
        *self.last_sequence.write().await = entries.last().unwrap().sequence;
        log::info!("Restore {} entries from local file", entries.len());
    }

    pub async fn clip(&self, ts: u64, d: f64, output_path: &str) -> Result<String, RecorderError> {
        let total_length = *self.ts_length.read().await;
        self.clip_range(ts, total_length - d, total_length, output_path)
            .await
    }

    /// x and y are relative to first sequence
    pub async fn clip_range(
        &self,
        ts: u64,
        x: f64,
        y: f64,
        output_path: &str,
    ) -> Result<String, RecorderError> {
        if *self.timestamp.read().await == ts {
            self.clip_live_range(x, y, output_path).await
        } else {
            self.clip_archive_range(ts, x, y, output_path).await
        }
    }

    pub async fn clip_archive_range(
        &self,
        ts: u64,
        x: f64,
        y: f64,
        output_path: &str,
    ) -> Result<String, RecorderError> {
        log::info!("Create archive clip for range [{}, {}]", x, y);
        let work_dir = format!("{}/{}/{}", self.config.read().await.cache, self.room_id, ts);
        let entries = self.get_fs_entries(&work_dir).await;
        if entries.is_empty() {
            return Err(RecorderError::EmptyCache);
        }
        let mut file_list = String::new();
        // header fist
        file_list += &format!("{}/h{}.m4s", work_dir, ts);
        file_list += "|";
        // add body entries
        // seconds to ms
        let begin = (x * 1000.0) as u64;
        let end = (y * 1000.0) as u64;
        let offset = entries.first().unwrap().offset;
        if !entries.is_empty() {
            for e in entries {
                if e.offset - offset < begin {
                    continue;
                }
                file_list += &format!("{}/{}", work_dir, e.url);
                file_list += "|";
                if e.offset - offset > end {
                    break;
                }
            }
        }

        std::fs::create_dir_all(output_path).expect("create clips folder failed");
        let file_name = format!(
            "{}/[{}]{}_{}_{:.1}.mp4",
            output_path,
            self.room_id,
            ts,
            Utc::now().format("%m%d%H%M%S"),
            y - x
        );
        log::info!("{}", file_name);
        let args = format!("-i concat:{} -c copy", file_list);
        FfmpegCommand::new()
            .args(args.split(' '))
            .output(file_name.clone())
            .spawn()
            .unwrap()
            .iter()
            .unwrap()
            .for_each(|e| match e {
                FfmpegEvent::Log(LogLevel::Error, e) => log::error!("Error: {}", e),
                FfmpegEvent::Progress(p) => log::info!("Progress: {}", p.time),
                _ => {}
            });
        Ok(file_name)
    }

    pub async fn clip_live_range(
        &self,
        x: f64,
        y: f64,
        output_path: &str,
    ) -> Result<String, RecorderError> {
        log::info!("Create live clip for range [{}, {}]", x, y);
        let mut to_combine = Vec::new();
        let header_copy = self.header.read().await.clone();
        let entry_copy = self.ts_entries.lock().await.clone();
        if entry_copy.is_empty() {
            return Err(RecorderError::EmptyCache);
        }
        let begin = (x * 1000.0) as u64;
        let end = (y * 1000.0) as u64;
        let offset = entry_copy.first().unwrap().offset;
        // TODO using binary search
        for e in entry_copy.iter() {
            if e.offset - offset < begin {
                continue;
            }
            to_combine.push(e);
            if e.offset - offset > end {
                break;
            }
        }
        if self
            .live_stream
            .read()
            .await
            .as_ref()
            .is_some_and(|s| s.format == StreamType::FMP4)
        {
            // add header to vec
            let header = header_copy.as_ref().unwrap();
            to_combine.insert(0, header);
        }
        let mut file_list = String::new();
        let timestamp = *self.timestamp.read().await;
        for e in to_combine {
            let file_name = e.url.split('/').last().unwrap();
            let file_path = format!(
                "{}/{}/{}/{}",
                self.config.read().await.cache,
                self.room_id,
                timestamp,
                file_name
            );
            file_list += &file_path;
            file_list += "|";
        }
        let title = self.room_info.read().await.room_title.clone();
        let title: String = title.chars().take(5).collect();
        std::fs::create_dir_all(output_path).expect("create clips folder failed");
        let file_name = format!(
            "{}/[{}]{}_{}_{:.1}.mp4",
            output_path,
            self.room_id,
            title,
            Utc::now().format("%m%d%H%M%S"),
            y - x
        );
        log::info!("{}", file_name);
        let args = format!("-i concat:{} -c copy", file_list);
        FfmpegCommand::new()
            .args(args.split(' '))
            .output(file_name.clone())
            .spawn()
            .unwrap()
            .iter()
            .unwrap()
            .for_each(|e| match e {
                FfmpegEvent::Log(LogLevel::Error, e) => log::error!("Error: {}", e),
                FfmpegEvent::Progress(p) => log::info!("Progress: {}", p.time),
                _ => {}
            });
        Ok(file_name)
    }

    /// timestamp is the id of live stream
    pub async fn generate_m3u8(&self, timestamp: u64) -> String {
        if *self.timestamp.read().await == timestamp {
            self.generate_live_m3u8().await
        } else {
            self.generate_archive_m3u8(timestamp).await
        }
    }

    async fn generate_archive_m3u8(&self, timestamp: u64) -> String {
        if self.m3u8_cache.contains_key(&timestamp) {
            return self.m3u8_cache.get(&timestamp).unwrap().clone();
        }
        let mut m3u8_content = "#EXTM3U\n".to_string();
        m3u8_content += "#EXT-X-VERSION:6\n";
        m3u8_content += "#EXT-X-TARGETDURATION:1\n";
        m3u8_content += "#EXT-X-PLAYLIST-TYPE:VOD\n";
        // add header, FMP4 need this
        // TODO handle StreamType::TS
        let header_url = format!("/{}/{}/h{}.m4s", self.room_id, timestamp, timestamp);
        m3u8_content += &format!("#EXT-X-MAP:URI=\"{}\"\n", header_url);
        // add entries from read_dir
        let work_dir = format!(
            "{}/{}/{}",
            self.config.read().await.cache,
            self.room_id,
            timestamp
        );
        let entries = self.get_fs_entries(&work_dir).await;
        if entries.is_empty() {
            return m3u8_content;
        }
        let mut last_sequence = entries.first().unwrap().sequence;
        m3u8_content += &format!("#EXT-X-OFFSET:{}\n", entries.first().unwrap().offset);
        for e in entries {
            let current_seq = e.sequence;
            if current_seq - last_sequence > 1 {
                m3u8_content += "#EXT-X-DISCONTINUITY\n"
            }
            // add #EXT-X-PROGRAM-DATE-TIME with ISO 8601 date
            let ts = timestamp + e.offset / 1000;
            let date_str = Utc.timestamp_opt(ts as i64, 0).unwrap().to_rfc3339();
            m3u8_content += &format!("#EXT-X-PROGRAM-DATE-TIME:{}\n", date_str);
            m3u8_content += &format!("#EXTINF:{:.2},\n", e.length);
            m3u8_content += &format!("/{}/{}/{}\n", self.room_id, timestamp, e.url);

            last_sequence = current_seq;
        }
        m3u8_content += "#EXT-X-ENDLIST";
        // cache this
        self.m3u8_cache.insert(timestamp, m3u8_content.clone());
        m3u8_content
    }

    /// Fetch HLS segments from local cached file, header is excluded
    async fn get_fs_entries(&self, path: &str) -> Vec<TsEntry> {
        let mut ret = Vec::new();
        let direntry = fs::read_dir(path).await;
        if direntry.is_err() {
            return ret;
        }
        let mut direntry = direntry.unwrap();
        while let Some(e) = direntry.next().await {
            if e.is_err() {
                continue;
            }
            let e = e.unwrap();
            let etype = e.file_type().await;
            if etype.is_err() {
                continue;
            }
            let etype = etype.unwrap();
            if !etype.is_file() {
                continue;
            }
            if let Some(file_ext) = e.path().extension() {
                let file_ext = file_ext.to_str().unwrap().to_string();
                // need to exclude other files, such as danmu file
                if file_ext != "m4s" {
                    continue;
                }
            } else {
                continue;
            }
            let file_name = e.file_name().to_str().unwrap().to_string();
            if file_name.starts_with("h") {
                continue;
            }
            let meta_info: &str = file_name.split('.').next().unwrap();
            let infos: Vec<&str> = meta_info.split('-').collect();
            let offset: u64;
            let sequence: u64;
            // BREAKCHANGE do not support legacy files that not named with offset
            if infos.len() == 1 {
                continue;
            } else {
                if let Ok(parsed_offset) = u64::from_str_radix(infos.get(0).unwrap(), 16) {
                    offset = parsed_offset;
                } else {
                    continue;
                }
                sequence = infos.get(1).unwrap().parse().unwrap();
            }
            ret.push(TsEntry {
                url: file_name.clone(),
                offset,
                sequence,
                length: 1.0,
                size: e.metadata().await.unwrap().len(),
            });
        }
        ret.sort_by(|a, b| a.sequence.cmp(&b.sequence));
        if ret.is_empty() {
            return ret;
        }
        let mut last_offset = ret.first().unwrap().offset;
        for (i, entry) in ret.iter_mut().enumerate() {
            if i == 0 {
                continue;
            }
            entry.length = (entry.offset - last_offset) as f64 / 1000.0;
            last_offset = entry.offset;
        }
        ret
    }

    /// if fetching live/last stream m3u8, all entries are cached in memory, so it will be much faster than read_dir
    async fn generate_live_m3u8(&self) -> String {
        let live_status = *self.live_status.read().await;
        let mut m3u8_content = "#EXTM3U\n".to_string();
        m3u8_content += "#EXT-X-VERSION:6\n";
        m3u8_content += "#EXT-X-TARGETDURATION:1\n";
        // if stream is closed, switch to VOD
        if live_status {
            m3u8_content += "#EXT-X-PLAYLIST-TYPE:EVENT\n";
        } else {
            m3u8_content += "#EXT-X-PLAYLIST-TYPE:VOD\n";
        }
        let timestamp = *self.timestamp.read().await;
        // initial segment for fmp4, info from self.header
        if let Some(header) = self.header.read().await.as_ref() {
            let file_name = header.url.split('/').last().unwrap();
            let local_url = format!("/{}/{}/{}", self.room_id, timestamp, file_name);
            m3u8_content += &format!("#EXT-X-MAP:URI=\"{}\"\n", local_url);
        }
        let entries = self.ts_entries.lock().await.clone();
        if entries.is_empty() {
            m3u8_content += "#EXT-X-OFFSET:0\n";
            return m3u8_content;
        }
        let timestamp = *self.timestamp.read().await;
        let mut last_sequence = entries.first().unwrap().sequence;
        m3u8_content += &format!("#EXT-X-OFFSET:{}\n", entries.first().unwrap().offset);
        for entry in entries.iter() {
            if entry.sequence - last_sequence > 1 {
                // discontinuity happens
                m3u8_content += "#EXT-X-DISCONTINUITY\n"
            }
            // add #EXT-X-PROGRAM-DATE-TIME with ISO 8601 date
            let ts = timestamp + entry.offset / 1000;
            let date_str = Utc.timestamp_opt(ts as i64, 0).unwrap().to_rfc3339();
            m3u8_content += &format!("#EXT-X-PROGRAM-DATE-TIME:{}\n", date_str);
            m3u8_content += &format!("#EXTINF:{:.2},\n", entry.length,);
            last_sequence = entry.sequence;
            let file_name = entry.url.split('/').last().unwrap();
            let local_url = format!("/{}/{}/{}", self.room_id, timestamp, file_name);
            m3u8_content += &format!("{}\n", local_url);
        }
        // let player know stream is closed
        if !live_status {
            m3u8_content += "#EXT-X-ENDLIST";
        }
        m3u8_content
    }

    pub async fn get_danmu_record(&self, ts: u64) -> Vec<DanmuEntry> {
        if ts == *self.timestamp.read().await {
            // just return current cache content
            match self.danmu_storage.read().await.as_ref() {
                Some(storage) => {
                    return storage.get_entries().await;
                }
                None => {
                    return Vec::new();
                }
            }
        } else {
            // load disk cache
            let cache_file_path = format!(
                "{}/{}/{}/{}",
                self.config.read().await.cache,
                self.room_id,
                ts,
                "danmu.txt"
            );
            log::info!("loading danmu cache from {}", cache_file_path);
            let storage = DanmuStorage::new(&cache_file_path).await;
            if storage.is_none() {
                return Vec::new();
            }
            let storage = storage.unwrap();
            return storage.get_entries().await;
        }
    }
}
