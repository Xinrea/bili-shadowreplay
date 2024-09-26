pub mod bilibili;
use async_std::{fs, stream::StreamExt};
use bilibili::{errors::BiliClientError, RoomInfo};
use bilibili::{BiliClient, UserInfo};
use chrono::prelude::*;
use custom_error::custom_error;
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
use tokio::sync::mpsc::{self, UnboundedReceiver};
use tokio::sync::{Mutex, RwLock};

use crate::db::AccountRow;

#[derive(Clone)]
pub struct TsEntry {
    pub url: String,
    pub sequence: u64,
    pub length: f64,
}

/// A recorder for BiliBili live streams
///
/// This recorder fetches, caches and serves TS entries, currently supporting only StreamType::FMP4.
/// As high-quality streams are accessible only to logged-in users, the use of a BiliClient, which manages cookies, is required.
// TODO implement StreamType::TS
#[derive(Clone)]
pub struct BiliRecorder {
    client: Arc<RwLock<BiliClient>>,
    account: AccountRow,
    cache_path: String,
    pub room_id: u64,
    pub room_info: Arc<RwLock<RoomInfo>>,
    pub user_info: Arc<RwLock<UserInfo>>,
    pub m3u8_url: Arc<RwLock<String>>,
    pub live_status: Arc<RwLock<bool>>,
    pub last_sequence: Arc<RwLock<u64>>,
    pub ts_length: Arc<RwLock<f64>>,
    pub timestamp: Arc<RwLock<u64>>,
    ts_entries: Arc<Mutex<Vec<TsEntry>>>,
    quit: Arc<Mutex<bool>>,
    header: Arc<RwLock<Option<TsEntry>>>,
    stream_type: Arc<RwLock<StreamType>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StreamType {
    TS,
    FMP4,
}

custom_error! {pub RecorderError
    NotStarted = "Room is offline",
    EmptyCache = "Cache is empty",
    M3u8ParseFailed = "Parse m3u8 content failed",
    InvalidM3u8Url {url: String} = "Invalid m3u8 url: {url}",
    EmptyHeader = "Header url is empty",
    InvalidTimestamp = "Header timestamp is invalid",
    InvalidPlaylist = "Invalid m3u8 playlist",
    ClientError {err: BiliClientError} = "BiliClient fetch failed",
}

impl From<BiliClientError> for RecorderError {
    fn from(value: BiliClientError) -> Self {
        RecorderError::ClientError { err: value }
    }
}

impl BiliRecorder {
    pub async fn new(
        room_id: u64,
        account: &AccountRow,
        cache_path: &str,
    ) -> Result<Self, RecorderError> {
        let client = BiliClient::new()?;
        let room_info = client.get_room_info(account, room_id).await?;
        let user_info = client.get_user_info(account, room_info.user_id).await?;
        let mut m3u8_url = String::from("");
        let mut live_status = false;
        let mut stream_type = StreamType::FMP4;
        if room_info.live_status == 1 {
            live_status = true;
            if let Ok((index_url, stream_type_now)) =
                client.get_play_url(account, room_info.room_id).await
            {
                m3u8_url = index_url;
                stream_type = stream_type_now;
            }
        }

        let recorder = Self {
            client: Arc::new(RwLock::new(client)),
            account: account.clone(),
            cache_path: cache_path.into(),
            room_id,
            room_info: Arc::new(RwLock::new(room_info)),
            user_info: Arc::new(RwLock::new(user_info)),
            m3u8_url: Arc::new(RwLock::new(m3u8_url)),
            live_status: Arc::new(RwLock::new(live_status)),
            last_sequence: Arc::new(RwLock::new(0)),
            ts_length: Arc::new(RwLock::new(0.0)),
            ts_entries: Arc::new(Mutex::new(Vec::new())),
            timestamp: Arc::new(RwLock::new(0)),
            quit: Arc::new(Mutex::new(false)),
            header: Arc::new(RwLock::new(None)),
            stream_type: Arc::new(RwLock::new(stream_type)),
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
            // if stream is confirmed to be closed, live stream cache is cleaned.
            // all request will go through fs
            if live_status {
                if let Ok((index_url, stream_type)) = self
                    .client
                    .read()
                    .await
                    .get_play_url(&self.account, self.room_id)
                    .await
                {
                    self.m3u8_url.write().await.replace_range(.., &index_url);
                    *self.stream_type.write().await = stream_type;
                }
            } else {
                self.reset().await;
            }
            *self.live_status.write().await = live_status;
            live_status
        } else {
            *self.live_status.write().await = true;
            // may encouter internet issues, not sure whether the stream is closed
            true
        }
    }

    pub async fn get_archives(&self) -> Vec<u64> {
        let work_dir = format!("{}/{}", self.cache_path, self.room_id);
        log::debug!(
            "[recorder:{}]Finding archives under {}",
            self.room_id,
            work_dir
        );
        let mut ret = Vec::new();
        if let Ok(mut entries) = fs::read_dir(work_dir).await {
            while let Some(e) = entries.next().await {
                if e.is_err() {
                    continue;
                }
                let e = e.unwrap();
                // get file type
                let ftype = e.file_type().await;
                if ftype.is_err() {
                    continue;
                }
                let ftype = ftype.unwrap();
                // check dir
                if ftype.is_dir() {
                    if let Ok(name) = e.file_name().into_string() {
                        // folder name should be timestamp
                        log::debug!(
                            "[recorder:{}]find a folder with name: {}",
                            self.room_id,
                            name
                        );
                        if let Ok(ts) = name.parse::<u64>() {
                            // current stream is not archived yet
                            if *self.timestamp.read().await != ts {
                                ret.push(ts);
                            }
                        }
                    }
                }
            }
        } else {
            log::error!("[recorder:{}]fs::read_dir failed", self.room_id);
        }
        ret
    }

    pub async fn delete_archive(&self, ts: u64) {
        let target_dir = format!("{}/{}/{}", self.cache_path, self.room_id, ts);
        if fs::remove_dir_all(target_dir).await.is_err() {
            log::error!("remove archive failed [{}]{}", self.room_id, ts);
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
                                log::error!("update entries error: {}", e);
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
            log::error!("{}", e);
        }
    }

    async fn recv(
        &self,
        room: u64,
        mut rx: UnboundedReceiver<WsStreamMessageType>,
    ) -> Result<(), FelgensError> {
        while let Some(msg) = rx.recv().await {
            if let WsStreamMessageType::DanmuMsg(msg) = msg {
                // if self.config.read().await.admin_uid.contains(&msg.uid) {
                //     let content: String = msg.msg;
                //     if content.starts_with("/clip") {
                //         let mut duration = 60.0;
                //         if content.len() > 5 {
                //             let num_part = content.strip_prefix("/clip ").unwrap_or("60");
                //             duration = num_part.parse::<u64>().unwrap_or(60) as f64;
                //         }
                //         if let Err(e) = self.clip(room, duration).await {
                //             if let Err(e) = Notification::new()
                //                 .summary("BiliBili ShadowReplay")
                //                 .body(format!("生成切片失败: {} - {}s", room, duration).as_str())
                //                 .icon("bili-shadowreplay")
                //                 .show()
                //             {
                //                 log::error!("notification error: {}", e);
                //             }
                //             log::error!("clip error: {}", e);
                //         } else if let Err(e) = Notification::new()
                //             .summary("BiliBili ShadowReplay")
                //             .body(format!("生成切片成功: {} - {}s", room, duration).as_str())
                //             .icon("bili-shadowreplay")
                //             .show()
                //         {
                //             log::error!("notification error: {}", e);
                //         }
                //     }
                // }
            }
        }
        Ok(())
    }

    pub async fn stop(&self) {
        *self.quit.lock().await = false;
    }

    async fn get_playlist(&self) -> Result<Playlist, RecorderError> {
        let url = self.m3u8_url.read().await.clone();
        let mut index_content = self.client.read().await.get_index_content(&url).await?;
        if index_content.contains("Not Found") {
            // 404 try another time after update
            if self.check_status().await {
                index_content = self.client.read().await.get_index_content(&url).await?;
            } else {
                return Err(RecorderError::NotStarted);
            }
        }
        m3u8_rs::parse_playlist_res(index_content.as_bytes())
            .map_err(|_| RecorderError::M3u8ParseFailed)
    }

    async fn get_header_url(&self) -> Result<String, RecorderError> {
        let url = self.m3u8_url.read().await.clone();
        let mut index_content = self.client.read().await.get_index_content(&url).await?;
        if index_content.contains("Not Found") {
            // 404 try another time after update
            log::warn!("Index content not found: {}", index_content);
            if self.check_status().await {
                index_content = self.client.read().await.get_index_content(&url).await?;
            } else {
                return Err(RecorderError::NotStarted);
            }
        }
        if index_content.contains("BANDWIDTH") {
            // this index content provides another m3u8 url
            let new_url = index_content.lines().last().unwrap();
            *self.m3u8_url.write().await = String::from(new_url);
            return Box::pin(self.get_header_url()).await;
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

    async fn ts_url(&self, ts_url: &String) -> Result<String, RecorderError> {
        // Construct url for ts and fmp4 stream.
        match *self.stream_type.read().await {
            StreamType::TS => {
                let url = self.m3u8_url.read().await.clone();
                if let Some(pos) = url.rfind("index.m3u8") {
                    Ok(format!("{}{}", &url[..pos], ts_url))
                } else {
                    Err(RecorderError::InvalidM3u8Url { url })
                }
            }
            StreamType::FMP4 => {
                let url = self.m3u8_url.read().await.clone();
                if let Some(pos) = url.rfind("index.m3u8") {
                    Ok(format!("{}{}", &url[..pos], ts_url))
                } else {
                    Err(RecorderError::InvalidM3u8Url { url })
                }
            }
        }
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
        let parsed = self.get_playlist().await;
        let mut timestamp = *self.timestamp.read().await;
        let mut work_dir = format!("{}/{}/{}/", self.cache_path, self.room_id, timestamp);
        // Check header if None
        if self.header.read().await.is_none() && *self.stream_type.read().await == StreamType::FMP4
        {
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
            // now work dir is confirmed
            work_dir = format!("{}/{}/{}/", self.cache_path, self.room_id, timestamp);
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
            let full_header_url = self.ts_url(&header_url).await?;
            let header = TsEntry {
                url: full_header_url.clone(),
                sequence: 0,
                length: 0.0,
            };
            let file_name = header_url.split('/').last().unwrap();
            // Download header
            if let Err(e) = self
                .client
                .read()
                .await
                .download_ts(&full_header_url, &format!("{}/{}", work_dir, file_name))
                .await
            {
                log::error!("Error downloading header: {:?}", e);
            } else {
                *self.header.write().await = Some(header);
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
                    let mut ts_entry = TsEntry {
                        url: ts.uri,
                        sequence,
                        length: ts.duration as f64,
                    };
                    let client = self.client.clone();
                    let ts_url = self.ts_url(&ts_entry.url).await?;
                    ts_entry.url = ts_url.clone();
                    if ts_url.is_empty() {
                        continue;
                    }
                    let work_dir = work_dir.clone();
                    handles.push(tokio::task::spawn(async move {
                        let ts_url_clone = ts_url.clone();
                        let file_name = ts_url_clone.split('/').last().unwrap();
                        if let Err(e) = client
                            .read()
                            .await
                            .download_ts(&ts_url, &format!("{}/{}", work_dir, file_name))
                            .await
                        {
                            log::error!("download ts failed: {}", e);
                        }
                    }));
                    let mut entries = self.ts_entries.lock().await;
                    entries.push(ts_entry);
                    *self.last_sequence.write().await = sequence;
                    let mut total_length = self.ts_length.write().await;
                    *total_length += ts.duration as f64;
                    sequence += 1;
                }
                join_all(handles).await.into_iter().for_each(|e| {
                    if let Err(e) = e {
                        log::error!("download ts failed: {:?}", e);
                    }
                });
            }
            Err(_) => {
                return Err(RecorderError::InvalidPlaylist);
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
        log::info!("create archive clip for range [{}, {}]", x, y);
        let work_dir = format!("{}/{}/{}", self.cache_path, self.room_id, ts);
        let entries = self.get_fs_entries(&work_dir).await;
        if entries.is_empty() {
            return Err(RecorderError::EmptyCache);
        }
        let mut file_list = String::new();
        // header fist
        file_list += &format!("{}/h{}.m4s", work_dir, ts);
        file_list += "|";
        // add body entries
        let mut offset = 0.0;
        if !entries.is_empty() {
            for e in entries {
                if offset < x {
                    offset += 1.0;
                    continue;
                }
                file_list += &format!("{}/{}", work_dir, e.url);
                file_list += "|";
                if offset > y {
                    break;
                }
                offset += 1.0;
            }
        }

        std::fs::create_dir_all(&output_path).expect("create clips folder failed");
        let file_name = format!(
            "{}/[{}]{}_{}_{:.1}.mp4",
            output_path,
            self.room_id,
            ts,
            Utc::now().format("%m%d%H%M%S"),
            y - x
        );
        log::info!("{}", file_name);
        let args = format!("-i concat:{} -c:v libx264 -c:a aac", file_list);
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
        log::info!("create live clip for range [{}, {}]", x, y);
        let mut to_combine = Vec::new();
        let header_copy = self.header.read().await.clone();
        let entry_copy = self.ts_entries.lock().await.clone();
        if entry_copy.is_empty() {
            return Err(RecorderError::EmptyCache);
        }
        let mut start = x;
        let mut end = y;
        if start > end {
            std::mem::swap(&mut start, &mut end);
        }
        let first_sequence = entry_copy.first().unwrap().sequence;
        for e in entry_copy.iter() {
            let offset = e.sequence - first_sequence;
            if (offset as f64) < start {
                continue;
            }
            to_combine.push(e);
            if (offset as f64) >= end {
                break;
            }
        }
        if *self.stream_type.read().await == StreamType::FMP4 {
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
                self.cache_path, self.room_id, timestamp, file_name
            );
            file_list += &file_path;
            file_list += "|";
        }
        let title = self.room_info.read().await.room_title.clone();
        let title: String = title.chars().take(5).collect();
        std::fs::create_dir_all(&output_path).expect("create clips folder failed");
        let file_name = format!(
            "{}/[{}]{}_{}_{:.1}.mp4",
            output_path,
            self.room_id,
            title,
            Utc::now().format("%m%d%H%M%S"),
            end - start
        );
        log::info!("{}", file_name);
        let args = format!("-i concat:{} -c:v libx264 -c:a aac", file_list);
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
        let mut m3u8_content = "#EXTM3U\n".to_string();
        m3u8_content += "#EXT-X-VERSION:6\n";
        m3u8_content += "#EXT-X-TARGETDURATION:1\n";
        m3u8_content += "#EXT-X-PLAYLIST-TYPE:VOD\n";
        // add header, FMP4 need this
        // TODO handle StreamType::TS
        let header_url = format!("/{}/{}/h{}.m4s", self.room_id, timestamp, timestamp);
        m3u8_content += &format!("#EXT-X-MAP:URI=\"{}\"\n", header_url);
        // add entries from read_dir
        let work_dir = format!("{}/{}/{}", self.cache_path, self.room_id, timestamp);
        let entries = self.get_fs_entries(&work_dir).await;
        if entries.is_empty() {
            return m3u8_content;
        }
        let mut last_sequence = entries.first().unwrap().sequence;
        for e in entries {
            let current_seq = e.sequence;
            if current_seq - last_sequence > 1 {
                m3u8_content += "#EXT-X-DISCONTINUITY\n"
            }
            last_sequence = current_seq;
            m3u8_content += "#EXTINF:1,\n";
            m3u8_content += &format!("/{}/{}/{}\n", self.room_id, timestamp, e.url);
        }
        m3u8_content += "#EXT-X-ENDLIST";
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
            let file_name = e.file_name().to_str().unwrap().to_string();
            if file_name.starts_with("h") {
                continue;
            }
            ret.push(TsEntry {
                url: file_name.clone(),
                sequence: file_name.split('.').next().unwrap().parse().unwrap(),
                length: 1.0,
            });
        }
        ret.sort_by(|a, b| a.sequence.cmp(&b.sequence));
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
            return m3u8_content;
        }
        let mut last_sequence = entries.first().unwrap().sequence;
        for entry in entries.iter() {
            if entry.sequence - last_sequence > 1 {
                // discontinuity happens
                m3u8_content += "#EXT-X-DISCONTINUITY\n"
            }
            last_sequence = entry.sequence;
            m3u8_content += "#EXTINF:1,\n";
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
}
