pub mod client;
pub mod errors;
pub mod profile;
pub mod response;
use super::entry::EntryStore;
use super::PlatformType;
use crate::database::account::AccountRow;
use crate::ffmpeg::{transcode, TranscodeConfig};

use super::danmu::{DanmuEntry, DanmuStorage};
use super::entry::TsEntry;
use chrono::{TimeZone, Utc};
use client::{BiliClient, BiliStream, RoomInfo, StreamType, UserInfo};
use dashmap::DashMap;
use errors::BiliClientError;
use felgens::{ws_socket_object, FelgensError, WsStreamMessageType};
use m3u8_rs::Playlist;
use rand::Rng;
use regex::Regex;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Url};
use tauri_plugin_notification::NotificationExt;
use tokio::fs::OpenOptions;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc::{self, UnboundedReceiver};
use tokio::sync::{Mutex, RwLock};

use crate::config::Config;
use crate::database::{Database, DatabaseError};
use crate::progress_event::emit_progress_update;

use async_trait::async_trait;

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
    pub live_id: Arc<RwLock<String>>,
    pub cover: Arc<RwLock<Option<String>>>,
    pub entry_store: Arc<RwLock<Option<EntryStore>>>,
    pub is_recording: Arc<RwLock<bool>>,
    pub auto_start: Arc<RwLock<bool>>,
    pub current_record: Arc<RwLock<bool>>,
    force_update: Arc<AtomicBool>,
    last_update: Arc<RwLock<i64>>,
    quit: Arc<Mutex<bool>>,
    pub live_stream: Arc<RwLock<Option<BiliStream>>>,
    danmu_storage: Arc<RwLock<Option<DanmuStorage>>>,
    m3u8_cache: DashMap<String, String>,
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

impl BiliRecorder {
    pub async fn new(
        app_handle: AppHandle,
        webid: &str,
        db: &Arc<Database>,
        room_id: u64,
        account: &AccountRow,
        config: Arc<RwLock<Config>>,
        auto_start: bool,
    ) -> Result<Self, super::errors::RecorderError> {
        let client = BiliClient::new()?;
        let room_info = client.get_room_info(account, room_id).await?;
        let user_info = client
            .get_user_info(webid, account, room_info.user_id)
            .await?;
        let mut live_status = false;
        let mut cover = None;
        if room_info.live_status == 1 {
            live_status = true;

            // Get cover image
            if let Ok(cover_base64) = client.get_cover_base64(&room_info.room_cover_url).await {
                cover = Some(cover_base64);
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
            entry_store: Arc::new(RwLock::new(None)),
            is_recording: Arc::new(RwLock::new(false)),
            auto_start: Arc::new(RwLock::new(auto_start)),
            current_record: Arc::new(RwLock::new(false)),
            live_id: Arc::new(RwLock::new(String::new())),
            cover: Arc::new(RwLock::new(cover)),
            last_update: Arc::new(RwLock::new(Utc::now().timestamp())),
            force_update: Arc::new(AtomicBool::new(false)),
            quit: Arc::new(Mutex::new(false)),
            live_stream: Arc::new(RwLock::new(None)),
            danmu_storage: Arc::new(RwLock::new(None)),
            m3u8_cache: DashMap::new(),
        };
        log::info!("Recorder for room {} created.", room_id);
        Ok(recorder)
    }

    pub async fn reset(&self) {
        *self.entry_store.write().await = None;
        *self.live_id.write().await = String::new();
        *self.live_stream.write().await = None;
        *self.last_update.write().await = Utc::now().timestamp();
        *self.danmu_storage.write().await = None;
    }

    async fn should_record(&self) -> bool {
        if *self.quit.lock().await {
            return false;
        }

        *self.current_record.read().await
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
                        "[{}]Live status changed to {}, current_record: {}, auto_start: {}",
                        self.room_id,
                        live_status,
                        *self.current_record.read().await,
                        *self.auto_start.read().await
                    );
                    // just doing reset
                    self.reset().await;

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

                        // Get cover image
                        if let Ok(cover_base64) = self
                            .client
                            .read()
                            .await
                            .get_cover_base64(&room_info.room_cover_url)
                            .await
                        {
                            *self.cover.write().await = Some(cover_base64);
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

                *self.live_status.write().await = live_status;

                if !live_status {
                    self.reset().await;
                    *self.current_record.write().await = false;

                    return false;
                }

                // no need to check stream if current_record is false and auto_start is false
                if !*self.current_record.read().await && !*self.auto_start.read().await {
                    return true;
                }

                // current_record => update stream
                // auto_start+is_new_stream => update stream and current_record=true
                let new_stream = match self
                    .client
                    .read()
                    .await
                    .get_play_url(&self.account, self.room_id)
                    .await
                {
                    Ok(stream) => Some(stream),
                    Err(e) => {
                        log::error!("[{}]Fetch stream failed: {}", self.room_id, e);
                        None
                    }
                };

                if new_stream.is_none() {
                    return true;
                }

                let stream = new_stream.unwrap();

                // auto start must be true here, if what fetched is a new stream, set current_record=true to auto start recording
                if self.live_stream.read().await.is_none()
                    || !self
                        .live_stream
                        .read()
                        .await
                        .as_ref()
                        .unwrap()
                        .is_same(&stream)
                    || self.force_update.load(Ordering::Relaxed)
                {
                    log::info!(
                        "[{}]Fetched a new stream: {:?} => {}",
                        self.room_id,
                        self.live_stream.read().await.clone(),
                        stream
                    );
                    *self.current_record.write().await = true;
                    self.force_update.store(false, Ordering::Relaxed);
                }

                if *self.current_record.read().await {
                    *self.live_stream.write().await = Some(stream);

                    return true;
                }

                true
            }
            Err(e) => {
                log::error!("[{}]Update room status failed: {}", self.room_id, e);
                // may encouter internet issues, not sure whether the stream is closed or started, just remain
                *self.live_status.read().await
            }
        }
    }

    async fn danmu(&self) {
        let cookies = self.account.cookies.clone();
        let uid: u64 = self.account.uid;
        while !*self.quit.lock().await {
            let (tx, rx) = mpsc::unbounded_channel();
            let ws = ws_socket_object(tx, uid, self.room_id, cookies.as_str());
            if let Err(e) = tokio::select! {v = ws => v, v = self.recv(self.room_id,rx) => v} {
                log::error!("danmu error: {}", e);
            }
            // reconnect after 3s
            log::warn!("danmu will reconnect after 3s");
            tokio::time::sleep(Duration::from_secs(3)).await;
        }

        log::info!("danmu thread {} quit.", self.room_id);
    }

    async fn recv(
        &self,
        room: u64,
        mut rx: UnboundedReceiver<WsStreamMessageType>,
    ) -> Result<(), FelgensError> {
        while let Some(msg) = rx.recv().await {
            if *self.quit.lock().await {
                break;
            }
            if let WsStreamMessageType::DanmuMsg(msg) = msg {
                let _ = self.app_handle.emit(
                    &format!("danmu:{}", room),
                    DanmuEntry {
                        ts: msg.timestamp,
                        content: msg.msg.clone(),
                    },
                );
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
            .get_index_content(&stream.index())
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
                log::error!("Failed fetching index content from {}", stream.index());
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
            .get_index_content(&stream.index())
            .await?;
        if index_content.is_empty() {
            return Err(super::errors::RecorderError::InvalidStream { stream });
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
            let base_url = new_url.split('/').next().unwrap();
            let host = base_url.split('/').next().unwrap();
            // extra is params after index.m3u8
            let extra = new_url.split(base_url).last().unwrap();
            let stream = BiliStream::new(StreamType::FMP4, base_url, host, extra);
            log::info!("Update stream: {}", stream);
            *self.live_stream.write().await = Some(stream);
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

    async fn extract_liveid(&self, header_url: &str) -> i64 {
        log::debug!("[{}]Extract liveid from {}", self.room_id, header_url);
        let re = Regex::new(r"h(\d+).m4s").unwrap();
        if let Some(cap) = re.captures(header_url) {
            let liveid: i64 = cap.get(1).unwrap().as_str().parse().unwrap();
            *self.live_id.write().await = liveid.to_string();
            liveid
        } else {
            log::error!("Extract liveid failed: {}", header_url);
            0
        }
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
        let mut timestamp: i64 = self.live_id.read().await.parse::<i64>().unwrap_or(0);
        let mut work_dir = self.get_work_dir(timestamp.to_string().as_str()).await;
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
            // Get url from EXT-X-MAP
            let header_url = self.get_header_url().await?;
            if header_url.is_empty() {
                return Err(super::errors::RecorderError::EmptyHeader);
            }
            timestamp = self.extract_liveid(&header_url).await;
            if timestamp == 0 {
                log::error!("[{}]Parse timestamp failed: {}", self.room_id, header_url);
                return Err(super::errors::RecorderError::InvalidTimestamp);
            }
            self.db
                .add_record(
                    PlatformType::BiliBili,
                    timestamp.to_string().as_str(),
                    self.room_id,
                    &self.room_info.read().await.room_title,
                    self.cover.read().await.clone(),
                )
                .await?;
            // now work dir is confirmed
            work_dir = self.get_work_dir(timestamp.to_string().as_str()).await;

            let entry_store = EntryStore::new(&work_dir).await;
            *self.entry_store.write().await = Some(entry_store);

            // danmau file
            let danmu_file_path = format!("{}{}", work_dir, "danmu.txt");
            *self.danmu_storage.write().await = DanmuStorage::new(&danmu_file_path).await;
            let full_header_url = current_stream.ts_url(&header_url);
            let file_name = header_url.split('/').last().unwrap();
            let mut header = TsEntry {
                url: file_name.to_string(),
                sequence: 0,
                length: 0.0,
                size: 0,
                ts: timestamp,
                is_header: true,
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
                    self.entry_store
                        .write()
                        .await
                        .as_mut()
                        .unwrap()
                        .add_entry(header)
                        .await;
                }
                Err(e) => {
                    log::error!("Download header failed: {}", e);
                }
            }
        }
        match parsed {
            Ok(Playlist::MasterPlaylist(pl)) => log::debug!("Master playlist:\n{:?}", pl),
            Ok(Playlist::MediaPlaylist(pl)) => {
                let mut new_segment_fetched = false;
                let mut sequence = pl.media_sequence;
                let last_sequence = self
                    .entry_store
                    .read()
                    .await
                    .as_ref()
                    .unwrap()
                    .last_sequence();
                for ts in pl.segments {
                    if sequence <= last_sequence {
                        sequence += 1;
                        continue;
                    }
                    new_segment_fetched = true;
                    let mut seg_offset: i64 = 0;
                    for tag in ts.unknown_tags {
                        if tag.tag == "BILI-AUX" {
                            if let Some(rest) = tag.rest {
                                let parts: Vec<&str> = rest.split('|').collect();
                                if parts.is_empty() {
                                    continue;
                                }
                                let offset_hex = parts.first().unwrap().to_string();
                                seg_offset = i64::from_str_radix(&offset_hex, 16).unwrap();
                            }
                            break;
                        }
                    }
                    let ts_url = current_stream.ts_url(&ts.uri);
                    if Url::parse(&ts_url).is_err() {
                        log::error!("Ts url is invalid. ts_url={} original={}", ts_url, ts.uri);
                        continue;
                    }
                    // encode segment offset into filename
                    let file_name = ts.uri.split('/').last().unwrap_or(&ts.uri);
                    let mut ts_length = pl.target_duration as f64;
                    let ts = timestamp * 1000 + seg_offset;
                    // calculate entry length using offset
                    // the default #EXTINF is 1.0, which is not accurate
                    if let Some(last) = self.entry_store.read().await.as_ref().unwrap().last_ts() {
                        // skip this entry as it is already in cache or stream changed
                        if ts <= last {
                            continue;
                        }
                        ts_length = (ts - last) as f64 / 1000.0;
                    }

                    let client = self.client.clone();
                    let mut retry = 0;
                    loop {
                        if retry > 3 {
                            log::error!("Download ts failed after retry");
                            break;
                        }
                        match client
                            .read()
                            .await
                            .download_ts(&ts_url, &format!("{}/{}", work_dir, file_name))
                            .await
                        {
                            Ok(size) => {
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
                                        ts,
                                        is_header: false,
                                    })
                                    .await;
                                break;
                            }
                            Err(e) => {
                                retry += 1;
                                log::warn!("Download ts failed, retry {}: {}", retry, e);
                            }
                        }
                    }

                    sequence += 1;
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
                        log::error!("Stream content is not updating for 10s, maybe not started yet or not closed properly.");
                        return Err(super::errors::RecorderError::FreezedStream {
                            stream: current_stream,
                        });
                    }
                }
                // check the current stream is too slow or not
                if let Some(last_ts) = self.entry_store.read().await.as_ref().unwrap().last_ts() {
                    if last_ts < Utc::now().timestamp() - 10 {
                        log::error!("Stream is too slow, last entry ts is at {}", last_ts);
                        return Err(super::errors::RecorderError::SlowStream {
                            stream: current_stream,
                        });
                    }
                }
            }
            Err(e) => {
                return Err(e);
            }
        }

        // check stream is nearly expired
        // WHY: when program started, all stream is fetched nearly at the same time, so they will expire toggether,
        // this might meet server rate limit. So we add a random offset to make request spread over time.
        let mut rng = rand::thread_rng();
        let pre_offset = rng.gen_range(5..=120);
        // no need to update stream as it's not expired yet
        let current_stream = self.live_stream.read().await.clone();
        if current_stream
            .as_ref()
            .is_some_and(|s| s.expire - Utc::now().timestamp() < pre_offset)
        {
            log::info!("Stream is nearly expired, force update");
            self.force_update.store(true, Ordering::Relaxed);
            return Err(super::errors::RecorderError::StreamExpired {
                stream: current_stream.unwrap(),
            });
        }

        Ok(task_begin_time.elapsed().as_millis())
    }

    pub async fn clip_archive_range(
        &self,
        app_handle: AppHandle,
        live_id: &str,
        x: f64,
        y: f64,
        clip_file: PathBuf,
    ) -> Result<PathBuf, super::errors::RecorderError> {
        log::info!("Create archive clip for range [{}, {}]", x, y);
        let work_dir = self.get_work_dir(live_id).await;
        let entries = EntryStore::new(&work_dir).await.get_entries().clone();
        if entries.is_empty() {
            return Err(super::errors::RecorderError::EmptyCache);
        }
        let mut file_list = Vec::new();
        // header fist
        file_list.push(format!("{}/h{}.m4s", work_dir, live_id));
        // add body entries
        // seconds to ms
        let begin = (x * 1000.0) as i64;
        let end = (y * 1000.0) as i64;
        let ts = entries.first().unwrap().ts;
        if !entries.is_empty() {
            for e in entries {
                if e.ts - ts < begin {
                    continue;
                }
                file_list.push(format!("{}/{}", work_dir, e.url));
                if e.ts - ts > end {
                    break;
                }
            }
        }

        self.generate_clip(app_handle, &file_list, clip_file).await
    }

    pub async fn clip_live_range(
        &self,
        app_handle: AppHandle,
        x: f64,
        y: f64,
        clip_file: PathBuf,
    ) -> Result<PathBuf, super::errors::RecorderError> {
        log::info!("Create live clip for range [{}, {}]", x, y);
        let work_dir = self.get_work_dir(self.live_id.read().await.as_str()).await;
        let mut to_combine = Vec::new();
        let header_copy = self
            .entry_store
            .read()
            .await
            .as_ref()
            .unwrap()
            .get_header()
            .unwrap()
            .clone();
        let entry_copy = self
            .entry_store
            .read()
            .await
            .as_ref()
            .unwrap()
            .get_entries()
            .clone();
        if entry_copy.is_empty() {
            return Err(super::errors::RecorderError::EmptyCache);
        }
        let begin = (x * 1000.0) as i64;
        let end = (y * 1000.0) as i64;
        let ts = entry_copy.first().unwrap().ts;
        // TODO using binary search
        for e in entry_copy.iter() {
            if e.ts - ts < begin {
                continue;
            }
            to_combine.push(e);
            if e.ts - ts > end {
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
            to_combine.insert(0, &header_copy);
        }
        let mut file_list = Vec::new();
        for e in to_combine {
            let file_name = e.url.split('/').last().unwrap();
            let file_path = format!("{}/{}", work_dir, file_name);
            file_list.push(file_path);
        }
        self.generate_clip(app_handle, &file_list, clip_file).await
    }

    async fn generate_clip(
        &self,
        app_handle: AppHandle,
        file_list: &[String],
        clip_file: PathBuf,
    ) -> Result<PathBuf, super::errors::RecorderError> {
        let event_id = format!("clip_{}", self.room_id);

        let tmp_file = clip_file.with_extension("tmp");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp_file)
            .await;
        if file.is_err() {
            let err = file.err().unwrap();
            log::error!("Open clip file failed: {}", err.to_string());
            return Err(super::errors::RecorderError::ClipError {
                err: err.to_string(),
            });
        }
        let mut file = file.unwrap();
        let total_files = file_list.len();
        // write file content in file_list into file
        for (i, f) in file_list.iter().enumerate() {
            let seg_file = OpenOptions::new().read(true).open(f).await;
            if seg_file.is_err() {
                log::error!("Reading {} failed, skip", f);
                continue;
            }
            let mut seg_file = seg_file.unwrap();
            let mut buffer = Vec::new();
            seg_file.read_to_end(&mut buffer).await.unwrap();
            file.write_all(&buffer).await.unwrap();

            emit_progress_update(
                &app_handle,
                event_id.as_str(),
                format!(
                    "生成中：{:.1}%",
                    (i + 1) as f64 * 100.0 / total_files as f64
                )
                .as_str(),
                "",
            );
        }

        file.flush().await.unwrap();

        let transcode_config = TranscodeConfig {
            input_path: tmp_file.clone(),
            input_format: "mp4".to_string(),
            output_path: clip_file,
        };

        let transcode_result = transcode(&app_handle, event_id.as_str(), transcode_config);

        // delete the original ts file
        if let Err(e) = tokio::fs::remove_file(tmp_file).await {
            log::error!("Delete temp clip file failed: {}", e.to_string());
        }

        Ok(transcode_result.unwrap().output_path)
    }

    async fn generate_archive_m3u8(&self, live_id: &str) -> String {
        if self.m3u8_cache.contains_key(live_id) {
            return self.m3u8_cache.get(live_id).unwrap().clone();
        }
        let mut m3u8_content = "#EXTM3U\n".to_string();
        m3u8_content += "#EXT-X-VERSION:6\n";
        m3u8_content += "#EXT-X-TARGETDURATION:1\n";
        m3u8_content += "#EXT-X-PLAYLIST-TYPE:VOD\n";
        // add header, FMP4 need this
        // TODO handle StreamType::TS
        let header_url = format!("/bilibili/{}/{}/h{}.m4s", self.room_id, live_id, live_id);
        m3u8_content += &format!("#EXT-X-MAP:URI=\"{}\"\n", header_url);
        // add entries from read_dir
        let work_dir = self.get_work_dir(live_id).await;
        let entries = EntryStore::new(&work_dir).await.get_entries().clone();
        if entries.is_empty() {
            return m3u8_content;
        }
        let mut last_sequence = entries.first().unwrap().sequence;

        let live_ts = live_id.parse::<i64>().unwrap();
        m3u8_content += &format!(
            "#EXT-X-OFFSET:{}\n",
            (entries.first().unwrap().ts - live_ts * 1000) / 1000
        );

        for e in entries {
            // ignore header, cause it's already in EXT-X-MAP
            if e.is_header {
                continue;
            }
            let current_seq = e.sequence;
            if current_seq - last_sequence > 1 {
                m3u8_content += "#EXT-X-DISCONTINUITY\n"
            }
            // add #EXT-X-PROGRAM-DATE-TIME with ISO 8601 date
            let ts = e.ts / 1000;
            let date_str = Utc.timestamp_opt(ts, 0).unwrap().to_rfc3339();
            m3u8_content += &format!("#EXT-X-PROGRAM-DATE-TIME:{}\n", date_str);
            m3u8_content += &format!("#EXTINF:{:.2},\n", e.length);
            m3u8_content += &format!("/bilibili/{}/{}/{}\n", self.room_id, live_id, e.url);

            last_sequence = current_seq;
        }
        m3u8_content += "#EXT-X-ENDLIST";
        // cache this
        self.m3u8_cache
            .insert(live_id.to_string(), m3u8_content.clone());
        m3u8_content
    }

    /// if fetching live/last stream m3u8, all entries are cached in memory, so it will be much faster than read_dir
    async fn generate_live_m3u8(&self) -> String {
        let live_status = *self.live_status.read().await;
        let mut m3u8_content = "#EXTM3U\n".to_string();
        m3u8_content += "#EXT-X-VERSION:6\n";
        m3u8_content += "#EXT-X-TARGETDURATION:1\n";
        m3u8_content += "#EXT-X-SERVER-CONTROL:HOLD-BACK:3\n";
        // if stream is closed, switch to VOD
        if live_status {
            m3u8_content += "#EXT-X-PLAYLIST-TYPE:EVENT\n";
        } else {
            m3u8_content += "#EXT-X-PLAYLIST-TYPE:VOD\n";
        }
        let live_id = self.live_id.read().await.clone();
        // initial segment for fmp4, info from self.header
        if let Some(header) = self.entry_store.read().await.as_ref().unwrap().get_header() {
            let file_name = header.url.split('/').last().unwrap();
            let local_url = format!("/bilibili/{}/{}/{}", self.room_id, live_id, file_name);
            m3u8_content += &format!("#EXT-X-MAP:URI=\"{}\"\n", local_url);
        }
        let entries = self
            .entry_store
            .read()
            .await
            .as_ref()
            .unwrap()
            .get_entries()
            .clone();
        if entries.is_empty() {
            m3u8_content += "#EXT-X-OFFSET:0\n";
            return m3u8_content;
        }

        let mut last_sequence = entries.first().unwrap().sequence;

        // this does nothing, but privide first entry ts for player
        let live_ts = live_id.parse::<i64>().unwrap();
        m3u8_content += &format!(
            "#EXT-X-OFFSET:{}\n",
            (entries.first().unwrap().ts - live_ts * 1000) / 1000
        );

        for entry in entries.iter() {
            if entry.sequence - last_sequence > 1 {
                // discontinuity happens
                m3u8_content += "#EXT-X-DISCONTINUITY\n"
            }
            // add #EXT-X-PROGRAM-DATE-TIME with ISO 8601 date
            let ts = entry.ts / 1000;
            let date_str = Utc.timestamp_opt(ts, 0).unwrap().to_rfc3339();
            m3u8_content += &format!("#EXT-X-PROGRAM-DATE-TIME:{}\n", date_str);
            m3u8_content += &format!("#EXTINF:{:.2},\n", entry.length);
            last_sequence = entry.sequence;
            let file_name = entry.url.split('/').last().unwrap();
            let local_url = format!("/bilibili/{}/{}/{}", self.room_id, live_id, file_name);
            m3u8_content += &format!("{}\n", local_url);
        }
        // let player know stream is closed
        if !live_status {
            m3u8_content += "#EXT-X-ENDLIST";
        }
        m3u8_content
    }
}

#[async_trait]
impl super::Recorder for BiliRecorder {
    async fn run(&self) {
        let self_clone = self.clone();
        thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async move {
                while !*self_clone.quit.lock().await {
                    if self_clone.check_status().await {
                        // Live status is ok, start recording.
                        while self_clone.should_record().await {
                            match self_clone.update_entries().await {
                                Ok(ms) => {
                                    if ms < 1000 {
                                        thread::sleep(std::time::Duration::from_millis(
                                            (1000 - ms) as u64,
                                        ));
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
                                    log::error!(
                                        "[{}]Update entries error: {}",
                                        self_clone.room_id,
                                        e
                                    );
                                    break;
                                }
                            }
                        }
                        *self_clone.is_recording.write().await = false;
                        // go check status again after random 2-5 secs
                        let mut rng = rand::thread_rng();
                        let secs = rng.gen_range(2..=5);
                        thread::sleep(std::time::Duration::from_secs(secs));
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

    async fn stop(&self) {
        *self.quit.lock().await = true;
    }

    /// x and y are relative to first sequence
    async fn clip_range(
        &self,
        app_handle: AppHandle,
        live_id: &str,
        x: f64,
        y: f64,
        output_path: PathBuf,
    ) -> Result<PathBuf, super::errors::RecorderError> {
        if *self.live_id.read().await == live_id {
            self.clip_live_range(app_handle, x, y, output_path).await
        } else {
            self.clip_archive_range(app_handle, live_id, x, y, output_path)
                .await
        }
    }

    /// timestamp is the id of live stream
    async fn m3u8_content(&self, live_id: &str) -> String {
        if *self.live_id.read().await == live_id {
            self.generate_live_m3u8().await
        } else {
            self.generate_archive_m3u8(live_id).await
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
            auto_start: *self.auto_start.read().await,
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
                Some(storage) => storage.get_entries().await,
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
            log::info!("loading danmu cache from {}", cache_file_path);
            let storage = DanmuStorage::new(&cache_file_path).await;
            if storage.is_none() {
                return Ok(Vec::new());
            }
            let storage = storage.unwrap();
            storage.get_entries().await
        })
    }

    async fn is_recording(&self, live_id: &str) -> bool {
        *self.live_id.read().await == live_id && *self.live_status.read().await
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
