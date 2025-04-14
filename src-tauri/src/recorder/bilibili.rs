pub mod client;
pub mod errors;
pub mod profile;
pub mod response;
use super::entry::EntryStore;
use super::PlatformType;
use crate::database::account::AccountRow;
use crate::playlist::HLSPlaylist;

use super::danmu::{DanmuEntry, DanmuStorage};
use chrono::{TimeZone, Utc};
use client::{BiliClient, BiliStream, RoomInfo, StreamType, UserInfo};
use errors::BiliClientError;
use felgens::{ws_socket_object, FelgensError, WsStreamMessageType};
use m3u8_rs::Playlist;
use rand::Rng;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Url};
use tauri_plugin_notification::NotificationExt;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc::{self, UnboundedReceiver};
use tokio::sync::{Mutex, RwLock};

use crate::config::Config;
use crate::database::{Database, DatabaseError};

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
    pub hls_playlist: Arc<RwLock<Option<HLSPlaylist>>>,
    pub is_recording: Arc<RwLock<bool>>,
    pub auto_start: Arc<RwLock<bool>>,
    pub current_record: Arc<RwLock<bool>>,
    force_update: Arc<AtomicBool>,
    last_update: Arc<RwLock<i64>>,
    quit: Arc<Mutex<bool>>,
    pub live_stream: Arc<RwLock<Option<BiliStream>>>,
    danmu_storage: Arc<RwLock<Option<DanmuStorage>>>,
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
            hls_playlist: Arc::new(RwLock::new(None)),
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
        };
        log::info!("Recorder for room {} created.", room_id);
        Ok(recorder)
    }

    pub async fn reset(&self) {
        *self.hls_playlist.write().await = None;
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
        log::debug!("[{}]Check status", self.room_id);
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
                    let live_id = stream.live_time.to_string();
                    let real_stream = self.fetch_real_stream(stream).await;
                    match real_stream {
                        Ok(stream) => {
                            log::info!("[{}]Fetched stream: {}", self.room_id, stream);
                            *self.live_stream.write().await = Some(stream.clone());
                        }
                        Err(e) => {
                            log::error!("[{}]Fetch stream failed: {}", self.room_id, e);
                            *self.live_stream.write().await = None;

                            return true;
                        }
                    }

                    *self.last_update.write().await = Utc::now().timestamp();

                    let _ = self
                        .db
                        .add_record(
                            PlatformType::BiliBili,
                            &live_id,
                            self.room_id,
                            &self.room_info.read().await.room_title,
                            self.cover.read().await.clone(),
                            None,
                        )
                        .await;

                    *self.live_id.write().await = live_id.to_string();

                    let playlist = self.load_previous_playlist(&live_id).await;
                    if let Some(playlist) = &playlist {
                        if let Err(e) = self.update_stream_header(playlist).await {
                            log::error!("[{}]Update stream header failed: {}", self.room_id, e);
                        }
                    }
                    *self.hls_playlist.write().await = playlist;

                    let work_dir = self.get_work_dir(&live_id).await;
                    let danmu_file_path = Path::new(&work_dir).join("danmu.txt");
                    *self.danmu_storage.write().await = DanmuStorage::new(&danmu_file_path).await;

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
        if entry_store.get_entries().is_empty() {
            None
        } else {
            Some(entry_store.to_hls_playlist(PlatformType::BiliBili, self.room_id, live_id, false))
        }
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
                    } else {
                        log::error!("Danmu storage not privided");
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

    async fn fetch_real_stream(
        &self,
        stream: BiliStream,
    ) -> Result<BiliStream, super::errors::RecorderError> {
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
            let new_url = index_content.lines().last().unwrap();
            let base_url = new_url.split('/').next().unwrap();
            let host = base_url.split('/').next().unwrap();
            // extra is params after index.m3u8
            let extra = new_url.split(base_url).last().unwrap();
            let stream = BiliStream::new(stream.live_time, StreamType::FMP4, base_url, host, extra);
            log::info!("Update stream: {}", stream);
            return Box::pin(self.fetch_real_stream(stream)).await;
        }
        Ok(stream)
    }

    async fn get_work_dir(&self, live_id: &str) -> PathBuf {
        let cache_dir = self.config.read().await.cache.clone();
        Path::new(&cache_dir)
            .join("bilibili")
            .join(self.room_id.to_string())
            .join(live_id)
    }

    async fn update_stream_header(
        &self,
        playlist: &HLSPlaylist,
    ) -> Result<(), super::errors::RecorderError> {
        let header_path = playlist.get_header();
        if header_path.is_none() {
            return Ok(());
        }
        let header_path = header_path.unwrap();
        let current_stream = self.live_stream.read().await.clone();
        if current_stream.is_none() {
            return Ok(());
        }
        let current_stream = current_stream.unwrap();
        let timestamp: i64 = self.live_id.read().await.parse::<i64>().unwrap_or(0);
        let work_dir = self.get_work_dir(timestamp.to_string().as_str()).await;
        let header_url = current_stream.ts_url(&header_path);
        if Url::parse(&header_url).is_err() {
            log::error!("Header url is invalid. header_url={}", header_url);
            return Err(super::errors::RecorderError::EmptyHeader);
        }
        let header_filename = header_path.split('/').last().unwrap_or(&header_path);
        let header_full_path = work_dir.join(header_filename);
        if header_full_path.exists() {
            log::info!("Header file already exists: {}", header_full_path.display());
            return Ok(());
        }
        log::info!("Download header file: {}", header_full_path.display());
        self.client
            .read()
            .await
            .download_ts(&header_url, &header_full_path)
            .await?;

        Ok(())
    }

    async fn update_entries(&self) -> Result<u128, super::errors::RecorderError> {
        let task_begin_time = std::time::Instant::now();
        let current_stream = self.live_stream.read().await.clone();
        if current_stream.is_none() {
            return Err(super::errors::RecorderError::NoStreamAvailable);
        }
        let current_stream = current_stream.unwrap();
        let parsed = self.get_playlist().await;
        let timestamp: i64 = self.live_id.read().await.parse::<i64>().unwrap_or(0);
        let work_dir = self.get_work_dir(timestamp.to_string().as_str()).await;

        match parsed {
            Ok(Playlist::MasterPlaylist(pl)) => log::debug!("Master playlist:\n{:?}", pl),
            Ok(Playlist::MediaPlaylist(pl)) => {
                let mut new_segment_size = 0;
                if self.hls_playlist.read().await.is_none() {
                    let mut new_playlist = HLSPlaylist::from(&pl);
                    self.update_stream_header(&new_playlist).await?;
                    new_playlist.segments.clear();
                    *self.hls_playlist.write().await = Some(new_playlist);
                    log::info!("New playlist created");
                    self.save_playlist().await;
                }

                let last_sequence = self
                    .hls_playlist
                    .read()
                    .await
                    .as_ref()
                    .unwrap()
                    .last_sequence()
                    .unwrap_or(0);

                let media_sequence = pl.media_sequence;

                for (i, ts) in pl.segments.iter().enumerate() {
                    let current_sequence = media_sequence + i as u64;
                    // skip this entry if it is already in cache
                    if current_sequence <= last_sequence {
                        continue;
                    }
                    let mut seg_offset: i64 = 0;
                    for tag in ts.unknown_tags.clone() {
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

                    let ts_filename = ts.uri.split('/').last().unwrap_or(&ts.uri);
                    let ts_timestamp = timestamp * 1000 + seg_offset;

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
                            .download_ts(&ts_url, &work_dir.join(ts_filename))
                            .await
                        {
                            Ok(size) => {
                                new_segment_size += size;

                                log::debug!(
                                    "[{}]Download segment: {}",
                                    self.room_id,
                                    current_sequence
                                );
                                let mut new_segment = ts.clone();
                                new_segment.program_date_time =
                                    Some(Utc.timestamp_opt(ts_timestamp / 1000, 0).unwrap().into());
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

                                break;
                            }
                            Err(e) => {
                                retry += 1;
                                log::warn!("Download ts failed, retry {}: {}", retry, e);
                            }
                        }
                    }
                }

                if new_segment_size > 0 {
                    *self.last_update.write().await = Utc::now().timestamp();
                    // total length is the offset of the last segment - the offset of the first segment
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
                            timestamp.to_string().as_str(),
                            total_length as i64,
                            new_segment_size,
                        )
                        .await?;

                    self.save_playlist().await;
                } else {
                    // if index content is not changed for a long time, we should return a error to fetch a new stream
                    if *self.last_update.read().await < Utc::now().timestamp() - 10 {
                        log::error!("Stream content is not updating for 10s, maybe not started yet or not closed properly.");
                        return Err(super::errors::RecorderError::FreezedStream {
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

    /// timestamp is the id of live stream
    async fn m3u8_content(&self, live_id: &str, start: i64, end: i64) -> String {
        if *self.live_id.read().await == live_id {
            let paused = !*self.current_record.read().await;
            self.hls_playlist
                .read()
                .await
                .clone()
                .unwrap_or_else(HLSPlaylist::new)
                .output(paused)
        } else {
            self.load_previous_playlist(live_id)
                .await
                .unwrap_or_else(HLSPlaylist::new)
                .output(true)
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
            total_length: self
                .hls_playlist
                .read()
                .await
                .as_ref()
                .map_or(0.0, |p| p.total_duration()),
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
            let work_dir = self.get_work_dir(live_id).await;
            let cache_file_path = Path::new(&work_dir).join("danmu.txt");
            log::info!("loading danmu cache from {:?}", cache_file_path);
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
