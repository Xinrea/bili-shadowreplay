use crate::config::Config;
use crate::danmu2ass;
use crate::database::recorder::RecorderRow;
use crate::database::video::VideoRow;
use crate::database::{account::AccountRow, record::RecordRow};
use crate::database::{Database, DatabaseError};
use crate::ffmpeg::{encode_video_danmu, transcode, Range};
use crate::progress::progress_reporter::{EventEmitter, ProgressReporter};
use crate::subtitle_generator::item_to_srt;
use crate::webhook::events::{self, Payload};
use crate::webhook::poster::WebhookPoster;
use chrono::DateTime;
use m3u8_rs::{MediaPlaylist, MediaPlaylistType};
use recorder::account::Account;
use recorder::danmu::{DanmuEntry, DanmuStorage};
use recorder::errors::RecorderError;
use recorder::events::RecorderEvent;
use recorder::platforms::bilibili::BiliRecorder;
use recorder::platforms::douyin::DouyinRecorder;
use recorder::platforms::PlatformType;
use recorder::traits::RecorderTrait;
use recorder::RoomInfo;
use recorder::UserInfo;
use recorder::{CachePath, RecorderInfo};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use thiserror::Error;
use tokio::fs::{remove_file, write, File};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::broadcast;
use tokio::sync::RwLock;

#[cfg(not(feature = "headless"))]
use tauri::AppHandle;

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug)]
pub struct RecorderList {
    pub count: usize,
    pub recorders: Vec<RecorderInfo>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ClipRangeParams {
    pub title: String,
    pub note: String,
    pub cover: String,
    pub platform: String,
    pub room_id: i64,
    pub live_id: String,
    pub range: Option<Range>,
    /// Encode danmu after clip
    pub danmu: bool,
    pub local_offset: i64,
    /// Fix encoding after clip
    pub fix_encoding: bool,
}

pub enum RecorderType {
    BiliBili(BiliRecorder),
    Douyin(DouyinRecorder),
}

impl RecorderType {
    async fn run(&self) {
        match self {
            RecorderType::BiliBili(recorder) => recorder.run().await,
            RecorderType::Douyin(recorder) => recorder.run().await,
        }
    }

    async fn stop(&self) {
        match self {
            RecorderType::BiliBili(recorder) => recorder.stop().await,
            RecorderType::Douyin(recorder) => recorder.stop().await,
        }
    }

    async fn info(&self) -> RecorderInfo {
        match self {
            RecorderType::BiliBili(recorder) => recorder.info().await,
            RecorderType::Douyin(recorder) => recorder.info().await,
        }
    }

    async fn enable(&self) {
        match self {
            RecorderType::BiliBili(recorder) => recorder.enable().await,
            RecorderType::Douyin(recorder) => recorder.enable().await,
        }
    }

    async fn disable(&self) {
        match self {
            RecorderType::BiliBili(recorder) => recorder.disable().await,
            RecorderType::Douyin(recorder) => recorder.disable().await,
        }
    }
}

#[derive(Clone)]
pub struct RecorderManager {
    #[cfg(not(feature = "headless"))]
    app_handle: AppHandle,
    emitter: EventEmitter,
    db: Arc<Database>,
    config: Arc<RwLock<Config>>,
    recorders: Arc<RwLock<HashMap<String, RecorderType>>>,
    to_remove: Arc<RwLock<HashSet<String>>>,
    event_tx: broadcast::Sender<RecorderEvent>,
    is_migrating: Arc<AtomicBool>,
    webhook_poster: WebhookPoster,
}

#[derive(Error, Debug)]
pub enum RecorderManagerError {
    #[error("Recorder already exists: {room_id}")]
    AlreadyExisted { room_id: i64 },
    #[error("Recorder not found: {room_id}")]
    NotFound { room_id: i64 },
    #[error("Invalid platform type: {platform}")]
    InvalidPlatformType { platform: String },
    #[error("Recorder error: {0}")]
    RecorderError(#[from] RecorderError),
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("HLS error: {err}")]
    HLSError { err: String },
    #[error("Database error: {0}")]
    DatabaseError(#[from] DatabaseError),
    #[error("Recording: {live_id}")]
    Recording { live_id: String },
    #[error("Clip error: {err}")]
    ClipError { err: String },
    #[error("M3u8 parse failed: {content}")]
    M3u8ParseFailed { content: String },
    #[error("Empty playlist")]
    EmptyPlaylist,
    #[error("Subtitle not found: {live_id}")]
    SubtitleNotFound { live_id: String },
    #[error("Subtitle generation failed: {error}")]
    SubtitleGenerationFailed { error: String },
    #[error("Invalid playlist without date time")]
    InvalidPlaylistWithoutDateTime,
}

impl From<RecorderManagerError> for String {
    fn from(err: RecorderManagerError) -> Self {
        err.to_string()
    }
}

impl RecorderManager {
    pub fn new(
        #[cfg(not(feature = "headless"))] app_handle: AppHandle,
        emitter: EventEmitter,
        db: Arc<Database>,
        config: Arc<RwLock<Config>>,
        webhook_poster: WebhookPoster,
    ) -> RecorderManager {
        let (event_tx, _) = broadcast::channel(100);
        let manager = RecorderManager {
            #[cfg(not(feature = "headless"))]
            app_handle,
            emitter,
            db,
            config,
            recorders: Arc::new(RwLock::new(HashMap::new())),
            to_remove: Arc::new(RwLock::new(HashSet::new())),
            event_tx,
            is_migrating: Arc::new(AtomicBool::new(false)),
            webhook_poster,
        };

        // Start event listener
        let manager_clone = manager.clone();
        tokio::spawn(async move {
            manager_clone.handle_events().await;
        });

        let manager_clone = manager.clone();
        tokio::spawn(async move {
            manager_clone.monitor_recorders().await;
        });

        manager
    }

    pub fn get_event_sender(&self) -> broadcast::Sender<RecorderEvent> {
        self.event_tx.clone()
    }

    async fn handle_events(&self) {
        let mut rx = self.event_tx.subscribe();
        while let Ok(event) = rx.recv().await {
            match event {
                RecorderEvent::LiveStart { recorder } => {
                    let event =
                        events::new_webhook_event(events::LIVE_STARTED, Payload::Room(recorder));
                    let _ = self.webhook_poster.post_event(&event).await;
                }
                RecorderEvent::LiveEnd {
                    platform,
                    room_id,
                    recorder,
                } => {
                    let event = events::new_webhook_event(
                        events::LIVE_ENDED,
                        Payload::Room(recorder.clone()),
                    );
                    let _ = self.webhook_poster.post_event(&event).await;
                    self.handle_live_end(platform, room_id, &recorder).await;
                }
                RecorderEvent::RecordStart { recorder } => {
                    // add record entry into db
                    let platform = PlatformType::from_str(&recorder.room_info.platform).unwrap();
                    let room_id = recorder.room_info.room_id.parse::<i64>().unwrap();
                    if let Err(e) = self
                        .db
                        .add_record(
                            platform,
                            &recorder.platform_live_id,
                            &recorder.live_id,
                            room_id,
                            &recorder.room_info.room_title,
                            None,
                        )
                        .await
                    {
                        log::error!("Failed to add record entry into db: {e}");
                    }
                    let event =
                        events::new_webhook_event(events::RECORD_STARTED, Payload::Room(recorder));
                    let _ = self.webhook_poster.post_event(&event).await;
                }
                RecorderEvent::RecordEnd { recorder } => {
                    let event =
                        events::new_webhook_event(events::RECORD_ENDED, Payload::Room(recorder));
                    let _ = self.webhook_poster.post_event(&event).await;
                }
                RecorderEvent::ProgressUpdate { id, content } => {
                    self.emitter
                        .emit(&RecorderEvent::ProgressUpdate { id, content });
                }
                RecorderEvent::ProgressFinished {
                    id,
                    success,
                    message,
                } => {
                    self.emitter.emit(&RecorderEvent::ProgressFinished {
                        id,
                        success,
                        message,
                    });
                }
                RecorderEvent::DanmuReceived { room, ts, content } => {
                    self.emitter
                        .emit(&RecorderEvent::DanmuReceived { room, ts, content });
                }
            }
        }
    }

    async fn handle_live_end(&self, platform: PlatformType, room_id: i64, recorder: &RecorderInfo) {
        if !self.config.read().await.auto_generate.enabled {
            return;
        }

        let recorder_id = format!("{}:{}", platform.as_str(), room_id);
        log::info!("Start auto generate for {recorder_id}");
        let live_id = recorder.live_id.clone();
        let live_record = self.db.get_record(room_id, &live_id).await;
        if live_record.is_err() {
            log::error!("Live not found in record: {room_id} {live_id}");
            return;
        }

        let live_record = live_record.unwrap();

        if let Err(e) = self
            .generate_whole_clip(
                None,
                platform.as_str().to_string(),
                room_id,
                live_record.parent_id,
            )
            .await
        {
            log::error!("Failed to generate whole clip: {e}");
        }
    }

    pub fn set_migrating(&self, migrating: bool) {
        self.is_migrating
            .store(migrating, std::sync::atomic::Ordering::Relaxed);
    }

    async fn monitor_recorders(&self) {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
        loop {
            if self.is_migrating.load(std::sync::atomic::Ordering::Relaxed) {
                interval.tick().await;
                continue;
            }
            // get a list of recorders in db, if not created yet, create them
            let recorders = self.db.get_recorders().await;
            if recorders.is_err() {
                log::error!(
                    "Failed to get recorders from db: {}",
                    recorders.err().unwrap()
                );
                return;
            }
            let recorders = recorders.unwrap();
            let mut recorder_map = HashMap::new();
            for recorder in recorders {
                let platform = PlatformType::from_str(&recorder.platform).unwrap();
                let room_id = recorder.room_id;
                let auto_start = recorder.auto_start;
                let extra = recorder.extra;
                recorder_map.insert((platform, room_id), (auto_start, extra));
            }
            let mut recorders_to_add = Vec::new();
            for (platform, room_id) in recorder_map.keys() {
                let recorder_id = format!("{}:{}", platform.as_str(), room_id);
                if !self.recorders.read().await.contains_key(&recorder_id)
                    && !self.to_remove.read().await.contains(&recorder_id)
                {
                    recorders_to_add.push((*platform, *room_id));
                }
            }
            for (platform, room_id) in recorders_to_add {
                if self.is_migrating.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
                let (auto_start, extra) = recorder_map.get(&(platform, room_id)).unwrap();
                let account = self
                    .db
                    .get_account_by_platform(platform.clone().as_str())
                    .await;
                if account.is_err() {
                    log::error!("Failed to get account: {}", account.err().unwrap());
                    continue;
                }
                let account = account.unwrap();

                if let Err(e) = self
                    .add_recorder(&account, platform, room_id, extra, *auto_start)
                    .await
                {
                    log::error!(
                        "Failed to add recorder: {} {} {}",
                        platform.as_str(),
                        room_id,
                        e
                    );
                }
            }
            interval.tick().await;
        }
    }

    pub async fn add_recorder(
        &self,
        account: &AccountRow,
        platform: PlatformType,
        room_id: i64,
        extra: &str,
        enabled: bool,
    ) -> Result<(), RecorderManagerError> {
        let recorder_id = format!("{}:{}", platform.as_str(), room_id);
        if self.recorders.read().await.contains_key(&recorder_id) {
            return Err(RecorderManagerError::AlreadyExisted { room_id });
        }

        let cache_dir = self.config.read().await.cache.clone();
        let cache_dir = PathBuf::from(&cache_dir);

        let recorder_account = Account {
            platform: platform.as_str().to_string(),
            id: if account.id_str.is_some() {
                account.id_str.as_ref().unwrap().clone()
            } else {
                account.uid.to_string()
            },
            name: account.name.clone(),
            avatar: account.avatar.clone(),
            csrf: account.csrf.clone(),
            cookies: account.cookies.clone(),
        };

        let event_tx = self.get_event_sender();
        let recorder: RecorderType = match platform {
            PlatformType::BiliBili => RecorderType::BiliBili(
                BiliRecorder::new(room_id, &recorder_account, cache_dir, event_tx, enabled).await?,
            ),
            PlatformType::Douyin => RecorderType::Douyin(
                DouyinRecorder::new(
                    room_id,
                    extra,
                    &recorder_account,
                    cache_dir,
                    event_tx,
                    enabled,
                )
                .await?,
            ),
            _ => {
                return Err(RecorderManagerError::InvalidPlatformType {
                    platform: platform.as_str().to_string(),
                })
            }
        };
        self.recorders
            .write()
            .await
            .insert(recorder_id.clone(), recorder);
        if let Some(recorder_ref) = self.recorders.read().await.get(&recorder_id) {
            recorder_ref.run().await;
        }
        Ok(())
    }

    pub async fn stop_all(&self) {
        for recorder_ref in self.recorders.read().await.values() {
            recorder_ref.stop().await;
        }

        // remove all recorders
        self.recorders.write().await.clear();
    }

    /// Remove a recorder from the manager
    ///
    /// This will stop the recorder and remove it from the manager
    /// and remove the related cache folder
    pub async fn remove_recorder(
        &self,
        platform: PlatformType,
        room_id: i64,
    ) -> Result<RecorderRow, RecorderManagerError> {
        // check recorder exists
        let recorder_id = format!("{}:{}", platform.as_str(), room_id);
        if !self.recorders.read().await.contains_key(&recorder_id) {
            return Err(RecorderManagerError::NotFound { room_id });
        }

        // remove from db
        let recorder = self.db.remove_recorder(room_id).await?;

        // add to to_remove
        log::debug!("Add to to_remove: {recorder_id}");
        self.to_remove.write().await.insert(recorder_id.clone());

        // stop recorder
        log::debug!("Stop recorder: {recorder_id}");
        if let Some(recorder_ref) = self.recorders.read().await.get(&recorder_id) {
            recorder_ref.stop().await;
        }

        // remove recorder
        log::debug!("Remove recorder from manager: {recorder_id}");
        self.recorders.write().await.remove(&recorder_id);

        // remove from to_remove
        log::debug!("Remove from to_remove: {recorder_id}");
        self.to_remove.write().await.remove(&recorder_id);

        // remove related cache folder
        let cache_folder = format!(
            "{}/{}/{}",
            self.config.read().await.cache,
            platform.as_str(),
            room_id
        );
        log::debug!("Remove cache folder: {cache_folder}");
        let _ = tokio::fs::remove_dir_all(cache_folder).await;
        log::info!("Recorder {room_id} cache folder removed");

        Ok(recorder)
    }

    async fn load_playlist_bytes(
        &self,
        platform: PlatformType,
        room_id: i64,
        live_id: &str,
    ) -> Result<Vec<u8>, RecorderManagerError> {
        let cache_path = self.config.read().await.cache.clone();
        let cache_path = Path::new(&cache_path);
        let playlist_path = cache_path
            .join(platform.as_str())
            .join(room_id.to_string())
            .join(live_id)
            .join("playlist.m3u8");
        if !playlist_path.exists() {
            return Err(RecorderManagerError::IoError(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Playlist file not found",
            )));
        }
        let mut bytes: Vec<u8> = Vec::new();
        tokio::fs::File::open(playlist_path)
            .await
            .unwrap()
            .read_to_end(&mut bytes)
            .await
            .unwrap();
        Ok(bytes)
    }

    async fn load_playlist(
        &self,
        platform: PlatformType,
        room_id: i64,
        live_id: &str,
    ) -> Result<MediaPlaylist, RecorderManagerError> {
        let bytes = self.load_playlist_bytes(platform, room_id, live_id).await?;
        if let Result::Ok((_, pl)) = m3u8_rs::parse_media_playlist(&bytes) {
            return Ok(pl);
        }
        Err(RecorderManagerError::M3u8ParseFailed {
            content: String::from_utf8(bytes).unwrap(),
        })
    }

    async fn playlist_range(
        &self,
        playlist: &MediaPlaylist,
        range: Option<Range>,
    ) -> Result<MediaPlaylist, RecorderManagerError> {
        let mut playlist = playlist.clone();
        if let Some(range) = range {
            let mut duration = 0.0f64;
            let mut segments = Vec::new();
            for s in playlist.segments {
                if range.is_in(duration) || range.is_in(duration + s.duration as f64) {
                    segments.push(s.clone());
                }
                duration += s.duration as f64;
            }
            playlist.segments = segments;
            playlist.end_list = true;
            playlist.playlist_type = Some(MediaPlaylistType::Vod);
        }

        Ok(playlist)
    }

    async fn first_segment_timestamp(
        &self,
        platform: PlatformType,
        room_id: i64,
        live_id: &str,
    ) -> Result<i64, RecorderManagerError> {
        let playlist = self.load_playlist(platform, room_id, live_id).await?;
        if playlist.segments.is_empty() {
            return Err(RecorderManagerError::EmptyPlaylist);
        }

        let first_segment = playlist.segments.first().unwrap();
        if let Some(program_date_time) = first_segment.program_date_time {
            return Ok(program_date_time.timestamp_millis());
        }

        // else, find in unknown tags
        let program_date_time = first_segment
            .unknown_tags
            .iter()
            .find(|t| t.tag == "X-PROGRAM-DATE-TIME");

        let Some(program_date_time) = program_date_time else {
            return Err(RecorderManagerError::InvalidPlaylistWithoutDateTime);
        };

        let Some(value) = &program_date_time.rest else {
            return Err(RecorderManagerError::InvalidPlaylistWithoutDateTime);
        };

        // example: "2025-10-18T17:18:17.004+0800"
        // convert to timestamp
        let timestamp = DateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.3f%z")
            .unwrap()
            .timestamp_millis();
        Ok(timestamp)
    }

    pub async fn load_danmus(
        &self,
        platform: PlatformType,
        room_id: i64,
        live_id: &str,
    ) -> Result<Vec<DanmuEntry>, RecorderManagerError> {
        let cache_path = self.config.read().await.cache.clone();
        let cache_path = Path::new(&cache_path);
        let danmus_path = cache_path
            .join(platform.as_str())
            .join(room_id.to_string())
            .join(live_id)
            .join("danmu.txt");
        if !danmus_path.exists() {
            return Ok(Vec::new());
        }
        let Some(storage) = DanmuStorage::new(&danmus_path).await else {
            log::error!("Failed to load danmu storage: {danmus_path:?}");
            return Ok(Vec::new());
        };
        Ok(storage.get_entries(0).await)
    }

    /// Get related playlists by parent id
    ///
    /// This will return a list of tuples, the first element is the title of the archive,
    /// the second element is the path of the playlist
    async fn get_related_playlists(
        &self,
        platform: &PlatformType,
        room_id: i64,
        parent_id: &str,
    ) -> Vec<(String, String)> {
        let cache_path = self.config.read().await.cache.clone();
        let cache_path = Path::new(&cache_path);
        let archives = self.db.get_archives_by_parent_id(room_id, parent_id).await;
        if let Err(e) = archives {
            log::error!(
                "[{}] Failed to get all related playlists: {} {}",
                room_id,
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
                let work_dir =
                    CachePath::new(cache_path.to_path_buf(), *platform, room_id, a.1.as_str());
                (
                    a.0.clone(),
                    work_dir
                        .with_filename("playlist.m3u8")
                        .full_path()
                        .to_str()
                        .unwrap()
                        .to_string(),
                )
            })
            .collect::<Vec<_>>();

        let playlists = futures::future::join_all(playlists).await;

        playlists
    }

    pub async fn clip_range(
        &self,
        reporter: Option<&ProgressReporter>,
        clip_file: PathBuf,
        params: &ClipRangeParams,
    ) -> Result<PathBuf, RecorderManagerError> {
        let cache_path = self.config.read().await.cache.clone();
        let cache_path = Path::new(&cache_path);
        let playlist_path = cache_path
            .join(params.platform.clone())
            .join(params.room_id.to_string())
            .join(params.live_id.clone())
            .join("playlist.m3u8");

        if !playlist_path.exists() {
            log::error!("Playlist file not found: {}", playlist_path.display());
            return Err(RecorderManagerError::ClipError {
                err: "Playlist file not found".to_string(),
            });
        }

        crate::ffmpeg::playlist::playlist_to_video(
            reporter,
            &playlist_path,
            &clip_file,
            params.range.clone(),
        )
        .await
        .map_err(|e| RecorderManagerError::ClipError { err: e.to_string() })?;

        if params.fix_encoding {
            // transcode clip_file
            let tmp_clip_file = clip_file.with_extension("tmp.mp4");
            if let Err(e) = transcode(reporter, &clip_file, &tmp_clip_file, false).await {
                log::error!("Failed to transcode clip file: {e}");
                return Err(RecorderManagerError::ClipError { err: e.to_string() });
            }

            // remove clip_file
            let _ = tokio::fs::remove_file(&clip_file).await;

            // rename tmp_clip_file to clip_file
            let _ = tokio::fs::rename(tmp_clip_file, &clip_file).await;
        }

        if !params.danmu {
            log::info!("Skip danmu encoding");
            return Ok(clip_file);
        }

        let Ok(platform) = PlatformType::from_str(&params.platform) else {
            return Err(RecorderManagerError::InvalidPlatformType {
                platform: params.platform.clone(),
            });
        };
        let stream_start_timestamp_milis = self
            .first_segment_timestamp(platform, params.room_id, &params.live_id)
            .await?;

        let danmus = self
            .load_danmus(platform, params.room_id, &params.live_id)
            .await;
        if danmus.is_err() {
            log::error!(
                "Failed to get danmus, skip danmu encoding: {}",
                danmus.err().unwrap()
            );
            return Ok(clip_file);
        }

        log::info!(
            "Filter danmus in range {} with local offset {}",
            params
                .range
                .as_ref()
                .map_or("None".to_string(), std::string::ToString::to_string),
            params.local_offset
        );
        let mut danmus = danmus.unwrap();
        log::debug!("First danmu entry: {:?}", danmus.first());
        log::debug!("Last danmu entry: {:?}", danmus.last());
        log::debug!("Stream start timestamp: {}", stream_start_timestamp_milis);
        log::debug!("Local offset: {}", params.local_offset);
        log::debug!("Range: {:?}", params.range);

        if let Some(range) = &params.range {
            // update entry ts to offset and filter danmus in range
            for d in &mut danmus {
                d.ts -= stream_start_timestamp_milis + params.local_offset * 1000;
            }
            if range.duration() > 0.0 {
                danmus.retain(|x| x.ts >= 0 && x.ts <= (range.duration() * 1000.0).round() as i64);
            }
        }

        if danmus.is_empty() {
            log::warn!("No danmus found, skip danmu encoding");

            return Ok(clip_file);
        }

        let ass_content = danmu2ass::danmu_to_ass(danmus);
        // dump ass_content into a temp file
        let ass_file_path = clip_file.with_extension("ass");
        if let Err(e) = write(&ass_file_path, ass_content).await {
            log::error!(
                "Failed to write temp ass file: {} {}",
                ass_file_path.display(),
                e
            );
            return Ok(clip_file);
        }

        let result = encode_video_danmu(reporter, &clip_file, &ass_file_path).await;
        // clean ass file
        let _ = remove_file(ass_file_path).await;
        let _ = remove_file(clip_file).await;

        result.map_err(|e| RecorderManagerError::ClipError { err: e })
    }

    pub async fn get_recorder_list(&self) -> RecorderList {
        let mut summary = RecorderList {
            count: 0,
            recorders: Vec::new(),
        };

        // initialized recorder set
        let mut recorder_set = HashSet::new();
        for recorder_ref in self.recorders.read().await.iter() {
            let recorder_info = recorder_ref.1.info().await;
            summary.recorders.push(recorder_info.clone());
            recorder_set.insert(recorder_info.room_info.room_id);
        }

        // get recorders from db
        let recorders = self.db.get_recorders().await;
        if recorders.is_err() {
            log::error!(
                "Failed to get recorders from db: {}",
                recorders.err().unwrap()
            );
            return summary;
        }
        let recorders = recorders.unwrap();
        summary.count = recorders.len();
        for recorder in recorders {
            // check if recorder is in recorder_set
            if !recorder_set.contains(&recorder.room_id.to_string()) {
                summary.recorders.push(RecorderInfo {
                    platform_live_id: "".to_string(),
                    live_id: "".to_string(),
                    recording: false,
                    enabled: false,
                    room_info: RoomInfo {
                        platform: recorder.platform.as_str().to_string(),
                        status: false,
                        room_id: recorder.room_id.to_string(),
                        room_title: recorder.room_id.to_string(),
                        room_cover: "".to_string(),
                    },
                    user_info: UserInfo {
                        user_id: "".to_string(),
                        user_name: "".to_string(),
                        user_avatar: "".to_string(),
                    },
                });
            }
        }

        summary
            .recorders
            .sort_by(|a, b| a.room_info.room_id.cmp(&b.room_info.room_id));
        summary
    }

    pub async fn get_recorder_info(
        &self,
        platform: PlatformType,
        room_id: i64,
    ) -> Option<RecorderInfo> {
        let recorder_id = format!("{}:{}", platform.as_str(), room_id);
        if let Some(recorder_ref) = self.recorders.read().await.get(&recorder_id) {
            let room_info = recorder_ref.info().await;
            Some(room_info)
        } else {
            None
        }
    }

    pub async fn get_archive_disk_usage(&self) -> Result<i64, RecorderManagerError> {
        Ok(self.db.get_record_disk_usage().await?)
    }

    pub async fn get_archives(
        &self,
        room_id: i64,
        offset: i64,
        limit: i64,
    ) -> Result<Vec<RecordRow>, RecorderManagerError> {
        Ok(self.db.get_records(room_id, offset, limit).await?)
    }

    pub async fn get_archive(
        &self,
        room_id: i64,
        live_id: &str,
    ) -> Result<RecordRow, RecorderManagerError> {
        Ok(self.db.get_record(room_id, live_id).await?)
    }

    pub async fn get_archive_subtitle(
        &self,
        platform: PlatformType,
        room_id: i64,
        live_id: &str,
    ) -> Result<String, RecorderManagerError> {
        // read subtitle file under work_dir
        let work_dir = CachePath::new(
            self.config.read().await.cache.clone().into(),
            platform,
            room_id,
            live_id,
        );
        let subtitle_file_path = work_dir.with_filename("subtitle.srt");
        let subtitle_file = File::open(subtitle_file_path.full_path()).await;
        if subtitle_file.is_err() {
            return Err(RecorderManagerError::SubtitleNotFound {
                live_id: live_id.to_string(),
            });
        }
        let subtitle_file = subtitle_file.unwrap();
        let mut subtitle_file = BufReader::new(subtitle_file);
        let mut subtitle_content = String::new();
        subtitle_file.read_to_string(&mut subtitle_content).await?;
        Ok(subtitle_content)
    }

    pub async fn generate_archive_subtitle(
        &self,
        platform: PlatformType,
        room_id: i64,
        live_id: &str,
    ) -> Result<String, RecorderManagerError> {
        // generate subtitle file under work_dir
        let work_dir = CachePath::new(
            self.config.read().await.cache.clone().into(),
            platform,
            room_id,
            live_id,
        );
        let subtitle_file_path = work_dir.with_filename("subtitle.srt");
        let mut subtitle_file = File::create(subtitle_file_path.full_path()).await?;
        // first generate a tmp clip file
        // generate a tmp m3u8 index file
        let m3u8_index_file_path = work_dir.with_filename("tmp.m3u8");
        let mut playlist = self.load_playlist(platform, room_id, live_id).await?;
        playlist.end_list = true;
        playlist.playlist_type = Some(MediaPlaylistType::Vod);

        let mut v: Vec<u8> = Vec::new();
        playlist.write_to(&mut v).unwrap();
        let m3u8_content: &str = std::str::from_utf8(&v).unwrap();
        tokio::fs::write(&m3u8_index_file_path.full_path(), m3u8_content).await?;
        log::info!(
            "[{}]M3U8 index file generated: {}",
            room_id,
            m3u8_index_file_path.full_path().display()
        );
        // generate a tmp clip file
        let clip_file_path = work_dir.with_filename("tmp.mp4");
        if let Err(e) = crate::ffmpeg::playlist::playlist_to_video(
            None::<&crate::progress::progress_reporter::ProgressReporter>,
            Path::new(&m3u8_index_file_path.full_path()),
            Path::new(&clip_file_path.full_path()),
            None,
        )
        .await
        {
            return Err(RecorderManagerError::SubtitleGenerationFailed {
                error: e.to_string(),
            });
        }
        log::info!("[{}]Temp clip file generated: {}", room_id, clip_file_path);
        // generate subtitle file
        let config = self.config.read().await;
        let result = crate::ffmpeg::generate_video_subtitle(
            None,
            Path::new(&clip_file_path.full_path()),
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
            return Err(RecorderManagerError::SubtitleGenerationFailed {
                error: e.to_string(),
            });
        }
        log::info!("[{room_id}]Subtitle generated");
        let result = result.unwrap();
        let subtitle_content = result
            .subtitle_content
            .iter()
            .map(item_to_srt)
            .collect::<String>();
        subtitle_file.write_all(subtitle_content.as_bytes()).await?;
        log::info!("[{room_id}]Subtitle file written");
        // remove tmp file
        tokio::fs::remove_file(&m3u8_index_file_path.full_path()).await?;
        tokio::fs::remove_file(&clip_file_path.full_path()).await?;
        log::info!("[{room_id}]Tmp file removed");
        Ok(subtitle_content)
    }

    pub async fn delete_archive(
        &self,
        platform: PlatformType,
        room_id: i64,
        live_id: &str,
    ) -> Result<RecordRow, RecorderManagerError> {
        log::info!("Deleting archive {room_id}:{live_id}");
        let to_delete = self.db.remove_record(live_id).await?;
        let cache_folder = Path::new(self.config.read().await.cache.as_str())
            .join(platform.as_str())
            .join(room_id.to_string())
            .join(live_id);
        let _ = tokio::fs::remove_dir_all(cache_folder).await;
        Ok(to_delete)
    }

    pub async fn delete_archives(
        &self,
        platform: PlatformType,
        room_id: i64,
        live_ids: &[&str],
    ) -> Result<Vec<RecordRow>, RecorderManagerError> {
        log::info!("Deleting archives in batch: {live_ids:?}");
        let mut to_deletes = Vec::new();
        for live_id in live_ids {
            let to_delete = self.delete_archive(platform, room_id, live_id).await?;
            to_deletes.push(to_delete);
        }
        Ok(to_deletes)
    }

    pub async fn handle_hls_request(&self, uri: &str) -> Result<Vec<u8>, RecorderManagerError> {
        let cache_path = self.config.read().await.cache.clone();
        let path = uri.split('?').next().unwrap_or(uri);
        let params = uri.split('?').nth(1).unwrap_or("");
        let path_segs: Vec<&str> = path.split('/').collect();

        if path_segs.len() != 4 {
            log::warn!("Invalid request path: {path}");
            return Err(RecorderManagerError::HLSError {
                err: "Invalid hls path".into(),
            });
        }
        // parse recorder type
        let platform = path_segs[0];
        // parse room id
        let room_id = path_segs[1].parse::<i64>().unwrap();
        // parse live id
        let live_id = path_segs[2];

        let params = Some(params);

        // parse params, example: start=10&end=20
        // start and end are optional
        // split params by &, and then split each param by =
        let params = if let Some(params) = params {
            let params = params
                .split('&')
                .map(|param| param.split('=').collect::<Vec<&str>>())
                .collect::<Vec<Vec<&str>>>();
            Some(params)
        } else {
            None
        };

        let start = if let Some(params) = &params {
            params
                .iter()
                .find(|param| param[0] == "start")
                .map_or(0, |param| param[1].parse::<i64>().unwrap())
        } else {
            0
        };
        let end = if let Some(params) = &params {
            params
                .iter()
                .find(|param| param[0] == "end")
                .map_or(0, |param| param[1].parse::<i64>().unwrap())
        } else {
            0
        };

        let platform = PlatformType::from_str(platform).map_err(|_| {
            RecorderManagerError::InvalidPlatformType {
                platform: platform.to_string(),
            }
        })?;

        let range = if start != 0 || end != 0 {
            Some(Range {
                start: start as f64,
                end: end as f64,
            })
        } else {
            None
        };

        if path_segs[3] == "playlist.m3u8" {
            let playlist = self.load_playlist(platform, room_id, live_id).await?;
            let playlist = self.playlist_range(&playlist, range).await?;
            let mut bytes: Vec<u8> = Vec::new();
            playlist.write_to(&mut bytes).unwrap();
            Ok(bytes)
        } else {
            // try to find requested ts file in recorder's cache
            // cache files are stored in {cache_dir}/{room_id}/{timestamp}/{ts_file}
            let ts_file = format!("{}/{}", cache_path, path.replace("%7C", "|"));
            let recorders = self.recorders.read().await;
            let recorder_id = format!("{}:{}", platform.as_str(), room_id);
            let recorder = recorders.get(&recorder_id);
            if recorder.is_none() {
                log::warn!("Recorder not found: {recorder_id}");
                return Err(RecorderManagerError::HLSError {
                    err: "Recorder not found".into(),
                });
            }
            let ts_file_content = tokio::fs::read(&ts_file).await;
            if ts_file_content.is_err() {
                log::warn!("Segment file not found: {ts_file}");
                return Err(RecorderManagerError::HLSError {
                    err: "Segment file not found".into(),
                });
            }

            Ok(ts_file_content.unwrap())
        }
    }

    pub async fn set_enable(&self, platform: PlatformType, room_id: i64, enabled: bool) {
        // update RecordRow auto_start field
        if let Err(e) = self.db.update_recorder(platform, room_id, enabled).await {
            log::error!("Failed to update recorder auto_start: {e}");
        }

        let recorder_id = format!("{}:{}", platform.as_str(), room_id);
        if let Some(recorder_ref) = self.recorders.read().await.get(&recorder_id) {
            if enabled {
                recorder_ref.enable().await;
            } else {
                recorder_ref.disable().await;
            }
        }
    }

    pub async fn generate_whole_clip(
        &self,
        reporter: Option<&ProgressReporter>,
        platform: String,
        room_id: i64,
        parent_id: String,
    ) -> Result<(), RecorderManagerError> {
        let platform = PlatformType::from_str(&platform).map_err(|_| {
            RecorderManagerError::InvalidPlatformType {
                platform: platform.to_string(),
            }
        })?;

        let playlists = self
            .get_related_playlists(&platform, room_id, &parent_id)
            .await;
        if playlists.is_empty() {
            log::error!("No related playlists found: {parent_id}");
            return Ok(());
        }

        let title = playlists.first().unwrap().0.clone();
        let playlists = playlists
            .iter()
            .map(|p| p.1.clone())
            .collect::<Vec<String>>();

        let sanitized_filename = sanitize_filename::sanitize(format!(
            "[full][{platform:?}][{room_id}][{parent_id}]{title}.mp4"
        ));
        let output_filename = Path::new(&sanitized_filename);
        let cover_filename = output_filename.with_extension("jpg");

        let output_path =
            Path::new(&self.config.read().await.output.as_str()).join(output_filename);

        log::info!("Concat playlists: {playlists:?}");
        log::info!("Output path: {output_path:?}");

        let owned_path_bufs: Vec<std::path::PathBuf> =
            playlists.iter().map(std::path::PathBuf::from).collect();

        let playlists_refs: Vec<&std::path::Path> = owned_path_bufs
            .iter()
            .map(std::path::PathBuf::as_path)
            .collect();

        if let Err(e) =
            crate::ffmpeg::playlist::playlists_to_video(reporter, &playlists_refs, &output_path)
                .await
        {
            log::error!("Failed to concat playlists: {e}");
            return Err(RecorderManagerError::HLSError {
                err: "Failed to concat playlists".into(),
            });
        }

        let metadata = std::fs::metadata(&output_path);
        if metadata.is_err() {
            return Err(RecorderManagerError::HLSError {
                err: "Failed to get file metadata".into(),
            });
        }
        let size = metadata.unwrap().len() as i64;

        let video_metadata = crate::ffmpeg::extract_video_metadata(Path::new(&output_path)).await;
        let mut length = 0;
        if let Ok(video_metadata) = video_metadata {
            length = video_metadata.duration as i64;
        } else {
            log::error!(
                "Failed to get video metadata: {}",
                video_metadata.err().unwrap()
            );
        }

        let _ = crate::ffmpeg::generate_thumbnail(Path::new(&output_path), 0.0).await;

        let video = self
            .db
            .add_video(&VideoRow {
                id: 0,
                status: 0,
                room_id,
                created_at: chrono::Local::now().to_rfc3339(),
                cover: cover_filename.to_string_lossy().to_string(),
                file: output_filename.to_string_lossy().to_string(),
                note: "".into(),
                length,
                size,
                bvid: String::new(),
                title: String::new(),
                desc: String::new(),
                tags: String::new(),
                area: 0,
                platform: platform.as_str().to_string(),
            })
            .await?;

        let event =
            events::new_webhook_event(events::CLIP_GENERATED, events::Payload::Clip(video.clone()));
        if let Err(e) = self.webhook_poster.post_event(&event).await {
            log::error!("Post webhook event error: {e}");
        }

        Ok(())
    }
}
