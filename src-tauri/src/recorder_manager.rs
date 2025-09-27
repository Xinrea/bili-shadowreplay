use crate::config::Config;
use crate::danmu2ass;
use crate::database::recorder::RecorderRow;
use crate::database::video::VideoRow;
use crate::database::{account::AccountRow, record::RecordRow};
use crate::database::{Database, DatabaseError};
use crate::ffmpeg::{encode_video_danmu, transcode, Range};
use crate::progress::progress_reporter::{EventEmitter, ProgressReporter};
use crate::recorder::bilibili::{BiliRecorder, BiliRecorderOptions};
use crate::recorder::danmu::DanmuEntry;
use crate::recorder::douyin::DouyinRecorder;
use crate::recorder::errors::RecorderError;
use crate::recorder::RecorderInfo;
use crate::recorder::{PlatformType, RoomInfo};
use crate::recorder::{Recorder, UserInfo};
use crate::webhook::events::{self, Payload};
use crate::webhook::poster::WebhookPoster;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use thiserror::Error;
use tokio::fs::{remove_file, write};
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

#[derive(Debug, Clone)]
pub enum RecorderEvent {
    LiveStart {
        recorder: RecorderInfo,
    },
    LiveEnd {
        room_id: i64,
        platform: PlatformType,
        recorder: RecorderInfo,
    },
    RecordStart {
        recorder: RecorderInfo,
    },
    RecordEnd {
        recorder: RecorderInfo,
    },
}

#[derive(Clone)]
pub struct RecorderManager {
    #[cfg(not(feature = "headless"))]
    app_handle: AppHandle,
    emitter: EventEmitter,
    db: Arc<Database>,
    config: Arc<RwLock<Config>>,
    recorders: Arc<RwLock<HashMap<String, Box<dyn Recorder>>>>,
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
    IOError(#[from] std::io::Error),
    #[error("HLS error: {err}")]
    HLSError { err: String },
    #[error("Database error: {0}")]
    DatabaseError(#[from] DatabaseError),
    #[error("Recording: {live_id}")]
    Recording { live_id: String },
    #[error("Clip error: {err}")]
    ClipError { err: String },
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
                    let event =
                        events::new_webhook_event(events::RECORD_STARTED, Payload::Room(recorder));
                    let _ = self.webhook_poster.post_event(&event).await;
                }
                RecorderEvent::RecordEnd { recorder } => {
                    let event =
                        events::new_webhook_event(events::RECORD_ENDED, Payload::Room(recorder));
                    let _ = self.webhook_poster.post_event(&event).await;
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
        let live_id = recorder.current_live_id.clone();
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
        auto_start: bool,
    ) -> Result<(), RecorderManagerError> {
        let recorder_id = format!("{}:{}", platform.as_str(), room_id);
        if self.recorders.read().await.contains_key(&recorder_id) {
            return Err(RecorderManagerError::AlreadyExisted { room_id });
        }

        let event_tx = self.get_event_sender();
        let recorder: Box<dyn Recorder + 'static> = match platform {
            PlatformType::BiliBili => Box::new(
                BiliRecorder::new(BiliRecorderOptions {
                    #[cfg(feature = "gui")]
                    app_handle: self.app_handle.clone(),
                    emitter: self.emitter.clone(),
                    db: self.db.clone(),
                    room_id,
                    account: account.clone(),
                    config: self.config.clone(),
                    auto_start,
                    channel: event_tx,
                })
                .await?,
            ),
            PlatformType::Douyin => Box::new(
                DouyinRecorder::new(
                    #[cfg(feature = "gui")]
                    self.app_handle.clone(),
                    self.emitter.clone(),
                    room_id,
                    extra,
                    self.config.clone(),
                    account,
                    &self.db,
                    auto_start,
                    event_tx,
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

    pub async fn clip_range(
        &self,
        reporter: &ProgressReporter,
        clip_file: PathBuf,
        params: &ClipRangeParams,
    ) -> Result<PathBuf, RecorderManagerError> {
        let recorders = self.recorders.read().await;
        let recorder_id = format!("{}:{}", params.platform, params.room_id);
        if !recorders.contains_key(&recorder_id) {
            log::error!("Recorder {recorder_id} not found");
            return Err(RecorderManagerError::NotFound {
                room_id: params.room_id,
            });
        }

        let recorder = recorders.get(&recorder_id).unwrap();

        self.clip_range_on_recorder(&**recorder, Some(reporter), clip_file, params)
            .await
    }

    pub async fn clip_range_on_recorder(
        &self,
        recorder: &dyn Recorder,
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

        let stream_start_timestamp_milis = recorder
            .playlist(
                &params.live_id,
                params.range.as_ref().unwrap().start as i64,
                params.range.as_ref().unwrap().end as i64,
            )
            .await
            .segments
            .first()
            .unwrap()
            .program_date_time
            .unwrap()
            .timestamp_millis();

        let danmus = recorder.comments(&params.live_id).await;
        if danmus.is_err() {
            log::error!("Failed to get danmus");
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
            let room_info = recorder_ref.1.info().await;
            summary.recorders.push(room_info.clone());
            recorder_set.insert(room_info.room_id);
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
            if !recorder_set.contains(&recorder.room_id) {
                summary.recorders.push(RecorderInfo {
                    room_id: recorder.room_id,
                    platform: recorder.platform,
                    auto_start: recorder.auto_start,
                    live_status: false,
                    is_recording: false,
                    total_length: 0.0,
                    current_live_id: "".to_string(),
                    room_info: RoomInfo {
                        room_id: recorder.room_id,
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

        summary.recorders.sort_by(|a, b| a.room_id.cmp(&b.room_id));
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
        let recorder_id = format!("{}:{}", platform.as_str(), room_id);
        if let Some(recorder_ref) = self.recorders.read().await.get(&recorder_id) {
            let recorder = recorder_ref.as_ref();
            Ok(recorder.get_archive_subtitle(live_id).await?)
        } else {
            Err(RecorderManagerError::NotFound { room_id })
        }
    }

    pub async fn generate_archive_subtitle(
        &self,
        platform: PlatformType,
        room_id: i64,
        live_id: &str,
    ) -> Result<String, RecorderManagerError> {
        let recorder_id = format!("{}:{}", platform.as_str(), room_id);
        if let Some(recorder_ref) = self.recorders.read().await.get(&recorder_id) {
            let recorder = recorder_ref.as_ref();
            Ok(recorder.generate_archive_subtitle(live_id).await?)
        } else {
            Err(RecorderManagerError::NotFound { room_id })
        }
    }

    pub async fn delete_archive(
        &self,
        platform: PlatformType,
        room_id: i64,
        live_id: &str,
    ) -> Result<RecordRow, RecorderManagerError> {
        log::info!("Deleting {room_id}:{live_id}");
        // check if this is still recording
        let recorder_id = format!("{}:{}", platform.as_str(), room_id);
        if let Some(recorder_ref) = self.recorders.read().await.get(&recorder_id) {
            let recorder = recorder_ref.as_ref();
            if recorder.is_recording(live_id).await {
                return Err(RecorderManagerError::Recording {
                    live_id: live_id.to_string(),
                });
            }
        }
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

    pub async fn get_danmu(
        &self,
        platform: PlatformType,
        room_id: i64,
        live_id: &str,
    ) -> Result<Vec<DanmuEntry>, RecorderManagerError> {
        let recorder_id = format!("{}:{}", platform.as_str(), room_id);
        if let Some(recorder_ref) = self.recorders.read().await.get(&recorder_id) {
            Ok(recorder_ref.comments(live_id).await?)
        } else {
            Err(RecorderManagerError::NotFound { room_id })
        }
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

        if path_segs[3] == "playlist.m3u8" {
            // get recorder
            let recorder_key = format!("{platform}:{room_id}");
            let recorders = self.recorders.read().await;
            let recorder = recorders.get(&recorder_key);
            if recorder.is_none() {
                log::warn!("Recorder not found: {recorder_key}");
                return Err(RecorderManagerError::HLSError {
                    err: "Recorder not found".into(),
                });
            }
            let recorder = recorder.unwrap();

            // response with recorder generated m3u8, which contains ts entries that cached in local
            log::debug!("Generating m3u8 for {live_id} with start {start} and end {end}");
            let playlist = recorder.playlist(live_id, start, end).await;
            let mut v: Vec<u8> = Vec::new();
            playlist.write_to(&mut v).unwrap();
            let m3u8_content: &str = std::str::from_utf8(&v).unwrap();

            Ok(m3u8_content.into())
        } else {
            // try to find requested ts file in recorder's cache
            // cache files are stored in {cache_dir}/{room_id}/{timestamp}/{ts_file}
            let ts_file = format!("{}/{}", cache_path, path.replace("%7C", "|"));
            let recorders = self.recorders.read().await;
            let recorder_id = format!("{platform}:{room_id}");
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
        let recorder_id = format!("{}:{}", platform, room_id);
        let recorders = self.recorders.read().await;
        let recorder_ref = recorders.get(&recorder_id);
        if recorder_ref.is_none() {
            return Err(RecorderManagerError::NotFound { room_id });
        };

        let recorder_ref = recorder_ref.unwrap();
        let playlists = recorder_ref.get_related_playlists(&parent_id).await;
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
            "[full][{platform}][{room_id}][{parent_id}]{title}.mp4"
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
                platform: platform.clone(),
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
