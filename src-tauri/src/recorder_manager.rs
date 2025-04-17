use crate::config::Config;
use crate::danmu2ass;
use crate::database::video::VideoRow;
use crate::database::DatabaseError;
use crate::database::{account::AccountRow, record::RecordRow, Database};
use crate::ffmpeg::{clip_from_m3u8, encode_video_danmu};
use crate::progress_event::ProgressReporter;
use crate::recorder::bilibili::BiliRecorder;
use crate::recorder::danmu::DanmuEntry;
use crate::recorder::douyin::DouyinRecorder;
use crate::recorder::errors::RecorderError;
use crate::recorder::PlatformType;
use crate::recorder::Recorder;
use crate::recorder::RecorderInfo;
use chrono::Utc;
use custom_error::custom_error;
use hyper::Uri;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use tauri::AppHandle;
use tokio::fs::{remove_file, write};
use tokio::sync::broadcast;
use tokio::sync::RwLock;

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug)]
pub struct RecorderList {
    pub count: usize,
    pub recorders: Vec<RecorderInfo>,
}

#[derive(Debug, Deserialize)]
pub struct ClipRangeParams {
    pub title: String,
    pub cover: String,
    pub platform: String,
    pub room_id: u64,
    pub live_id: String,
    /// Clip range start in seconds
    pub x: i64,
    /// Clip range end in seconds
    pub y: i64,
    /// Timestamp of first stream segment in seconds
    pub offset: i64,
    /// Encode danmu after clip
    pub danmu: bool,
}

#[derive(Debug, Clone)]
pub enum RecorderEvent {
    LiveEnd {
        platform: PlatformType,
        room_id: u64,
        live_id: String,
    },
}

pub struct RecorderManager {
    app_handle: AppHandle,
    db: Arc<Database>,
    config: Arc<RwLock<Config>>,
    recorders: Arc<RwLock<HashMap<String, Box<dyn Recorder>>>>,
    event_tx: broadcast::Sender<RecorderEvent>,
}

custom_error! {pub RecorderManagerError
    AlreadyExisted { room_id: u64 } = "房间 {room_id} 已存在",
    NotFound {room_id: u64 } = "房间 {room_id} 不存在",
    InvalidPlatformType { platform: String } = "不支持的平台: {platform}",
    RecorderError { err: RecorderError } = "录播器错误: {err}",
    IOError {err: std::io::Error } = "IO 错误: {err}",
    HLSError { err: String } = "HLS 服务器错误: {err}",
    DatabaseError { err: DatabaseError } = "数据库错误: {err}",
    Recording { live_id: String } = "无法删除正在录制的直播 {live_id}",
    ClipError { err: String } = "切片错误: {err}",
}

impl From<std::io::Error> for RecorderManagerError {
    fn from(value: std::io::Error) -> Self {
        RecorderManagerError::IOError { err: value }
    }
}

impl From<RecorderError> for RecorderManagerError {
    fn from(value: RecorderError) -> Self {
        RecorderManagerError::RecorderError { err: value }
    }
}

impl From<DatabaseError> for RecorderManagerError {
    fn from(value: DatabaseError) -> Self {
        RecorderManagerError::DatabaseError { err: value }
    }
}

impl From<RecorderManagerError> for String {
    fn from(value: RecorderManagerError) -> Self {
        value.to_string()
    }
}

impl RecorderManager {
    pub fn new(
        app_handle: AppHandle,
        db: Arc<Database>,
        config: Arc<RwLock<Config>>,
    ) -> RecorderManager {
        let (event_tx, _) = broadcast::channel(100);
        let manager = RecorderManager {
            app_handle,
            db,
            config,
            recorders: Arc::new(RwLock::new(HashMap::new())),
            event_tx,
        };

        // Start event listener
        let manager_clone = manager.clone();
        tokio::spawn(async move {
            manager_clone.handle_events().await;
        });

        manager
    }

    pub fn clone(&self) -> Self {
        RecorderManager {
            app_handle: self.app_handle.clone(),
            db: self.db.clone(),
            config: self.config.clone(),
            recorders: self.recorders.clone(),
            event_tx: self.event_tx.clone(),
        }
    }

    pub fn get_event_sender(&self) -> broadcast::Sender<RecorderEvent> {
        self.event_tx.clone()
    }

    async fn handle_events(&self) {
        let mut rx = self.event_tx.subscribe();
        while let Ok(event) = rx.recv().await {
            match event {
                RecorderEvent::LiveEnd {
                    platform,
                    room_id,
                    live_id,
                } => {
                    self.handle_live_end(platform, room_id, &live_id).await;
                }
            }
        }
    }

    async fn handle_live_end(&self, platform: PlatformType, room_id: u64, live_id: &str) {
        if !self.config.read().await.auto_generate.enabled {
            return;
        }

        let recorder_id = format!("{}:{}", platform.as_str(), room_id);
        log::info!("Start auto generate for {}", recorder_id);
        let live_record = self.db.get_record(room_id, live_id).await;
        if live_record.is_err() {
            log::error!("Live not found in record: {} {}", room_id, live_id);
            return;
        }

        let recorders = self.recorders.read().await;
        let recorder = match recorders.get(&recorder_id) {
            Some(recorder) => recorder,
            None => {
                log::error!("Recorder not found: {}", recorder_id);
                return;
            }
        };

        let live_record = live_record.unwrap();
        let encode_danmu = self.config.read().await.auto_generate.encode_danmu;

        let clip_config = ClipRangeParams {
            title: live_record.title,
            cover: "".into(),
            platform: live_record.platform,
            room_id,
            live_id: live_id.to_string(),
            x: 0,
            y: 0,
            offset: recorder.first_segment_ts(live_id).await,
            danmu: encode_danmu,
        };

        let clip_filename = self.config.read().await.generate_clip_name(&clip_config);

        // add prefix [full] for clip_filename
        let name_with_prefix = format!(
            "[full]{}",
            clip_filename.file_name().unwrap().to_str().unwrap()
        );
        let _ = clip_filename.with_file_name(name_with_prefix);

        match self
            .clip_range_on_recorder(&**recorder, None, clip_filename, &clip_config)
            .await
        {
            Ok(f) => {
                let metadata = match std::fs::metadata(&f) {
                    Ok(metadata) => metadata,
                    Err(e) => {
                        log::error!("Failed to detect auto generated clip: {}", e);
                        return;
                    }
                };
                match self
                    .db
                    .add_video(&VideoRow {
                        id: 0,
                        status: 0,
                        room_id,
                        created_at: Utc::now().to_rfc3339(),
                        cover: "".into(),
                        file: f.file_name().unwrap().to_str().unwrap().to_string(),
                        length: live_record.length,
                        size: metadata.len() as i64,
                        bvid: "".into(),
                        title: "".into(),
                        desc: "".into(),
                        tags: "".into(),
                        area: 0,
                    })
                    .await
                {
                    Ok(_) => {}
                    Err(e) => {
                        log::error!("Add auto generate clip record failed: {}", e)
                    }
                };
            }
            Err(e) => {
                log::error!("Auto generate clip failed: {}", e)
            }
        }
    }

    pub async fn add_recorder(
        &self,
        webid: &str,
        account: &AccountRow,
        platform: PlatformType,
        room_id: u64,
        auto_start: bool,
    ) -> Result<(), RecorderManagerError> {
        let recorder_id = format!("{}:{}", platform.as_str(), room_id);
        if self.recorders.read().await.contains_key(&recorder_id) {
            return Err(RecorderManagerError::AlreadyExisted { room_id });
        }

        let event_tx = self.get_event_sender();
        let recorder: Box<dyn Recorder + 'static> = match platform {
            PlatformType::BiliBili => Box::new(
                BiliRecorder::new(
                    self.app_handle.clone(),
                    webid,
                    &self.db,
                    room_id,
                    account,
                    self.config.clone(),
                    auto_start,
                    event_tx,
                )
                .await?,
            ),
            PlatformType::Douyin => Box::new(
                DouyinRecorder::new(
                    self.app_handle.clone(),
                    room_id,
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

    pub async fn remove_recorder(
        &self,
        platform: PlatformType,
        room_id: u64,
    ) -> Result<(), RecorderManagerError> {
        // check recorder exists
        let recorder_id = format!("{}:{}", platform.as_str(), room_id);
        if !self.recorders.read().await.contains_key(&recorder_id) {
            return Err(RecorderManagerError::NotFound { room_id });
        }

        // stop recorder
        if let Some(recorder_ref) = self.recorders.read().await.get(&recorder_id) {
            recorder_ref.stop().await;
        }

        // remove recorder
        self.recorders.write().await.remove(&recorder_id);

        // remove related cache folder
        let cache_folder = format!(
            "{}/{}/{}",
            self.config.read().await.cache,
            platform.as_str(),
            room_id
        );
        let _ = tokio::fs::remove_dir_all(cache_folder).await;
        log::info!("Recorder {} cache folder removed", room_id);

        Ok(())
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
            log::error!("Recorder {} not found", recorder_id);
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
        let range_m3u8 = format!(
            "http://127.0.0.1/{}/{}/{}/playlist.m3u8?start={}&end={}",
            params.platform, params.room_id, params.live_id, params.x, params.y
        );

        let manifest_content = self.handle_hls_request(&range_m3u8).await?;
        let manifest_content = String::from_utf8(manifest_content)
            .map_err(|e| RecorderManagerError::ClipError { err: e.to_string() })?;

        let cache_path = self.config.read().await.cache.clone();
        let cache_path = Path::new(&cache_path);
        let random_filename = format!("manifest_{}.m3u8", uuid::Uuid::new_v4());
        let tmp_manifest_file_path = cache_path
            .join(&params.platform)
            .join(params.room_id.to_string())
            .join(&params.live_id)
            .join(random_filename);

        // Write manifest content to temporary file
        tokio::fs::write(&tmp_manifest_file_path, manifest_content.as_bytes())
            .await
            .map_err(|e| RecorderManagerError::ClipError { err: e.to_string() })?;

        if let Err(e) = clip_from_m3u8(reporter, &tmp_manifest_file_path, &clip_file).await {
            log::error!("Failed to generate clip file: {}", e);
            return Err(RecorderManagerError::ClipError { err: e.to_string() });
        }

        // remove temp file
        let _ = tokio::fs::remove_file(tmp_manifest_file_path).await;

        if !params.danmu {
            return Ok(clip_file);
        }

        let danmus = recorder.comments(&params.live_id).await;
        if danmus.is_err() {
            log::error!("Failed to get danmus");
            return Ok(clip_file);
        }

        log::info!(
            "Filter danmus in range [{}, {}] with offset {}",
            params.x,
            params.y,
            params.offset
        );
        let mut danmus = danmus.unwrap();
        log::debug!("First danmu entry: {:?}", danmus.first());
        // update entry ts to offset
        for d in &mut danmus {
            d.ts -= (params.x + params.offset) * 1000;
        }
        if params.x != 0 || params.y != 0 {
            danmus.retain(|x| x.ts >= 0 && x.ts <= (params.y - params.x) * 1000);
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
            count: self.recorders.read().await.len(),
            recorders: Vec::new(),
        };

        for recorder_ref in self.recorders.read().await.iter() {
            let room_info = recorder_ref.1.info().await;
            summary.recorders.push(room_info);
        }

        summary.recorders.sort_by(|a, b| a.room_id.cmp(&b.room_id));
        summary
    }

    pub async fn get_recorder_info(
        &self,
        platform: PlatformType,
        room_id: u64,
    ) -> Option<RecorderInfo> {
        let recorder_id = format!("{}:{}", platform.as_str(), room_id);
        if let Some(recorder_ref) = self.recorders.read().await.get(&recorder_id) {
            let room_info = recorder_ref.info().await;
            Some(room_info)
        } else {
            None
        }
    }

    pub async fn get_archives(&self, room_id: u64) -> Result<Vec<RecordRow>, RecorderManagerError> {
        Ok(self.db.get_records(room_id).await?)
    }

    pub async fn get_archive(
        &self,
        room_id: u64,
        live_id: &str,
    ) -> Result<RecordRow, RecorderManagerError> {
        Ok(self.db.get_record(room_id, live_id).await?)
    }

    pub async fn delete_archive(
        &self,
        platform: PlatformType,
        room_id: u64,
        live_id: &str,
    ) -> Result<(), RecorderManagerError> {
        log::info!("Deleting {}:{}", room_id, live_id);
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
        self.db.remove_record(live_id).await?;
        let cache_folder = Path::new(self.config.read().await.cache.as_str())
            .join(platform.as_str())
            .join(room_id.to_string())
            .join(live_id);
        let _ = tokio::fs::remove_dir_all(cache_folder).await;
        Ok(())
    }

    pub async fn get_danmu(
        &self,
        platform: PlatformType,
        room_id: u64,
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
        let uri = Uri::from_str(uri)
            .map_err(|e| RecorderManagerError::HLSError { err: e.to_string() })?;
        let path = uri.path();
        let path_segs: Vec<&str> = path.split('/').collect();

        if path_segs.len() != 5 {
            log::warn!("Invalid request path: {}", path);
            return Err(RecorderManagerError::HLSError {
                err: "Invalid hls path".into(),
            });
        }
        // parse recorder type
        let platform = path_segs[1];
        // parse room id
        let room_id = path_segs[2].parse::<u64>().unwrap();
        // parse live id
        let live_id = path_segs[3];

        if path_segs[4] == "playlist.m3u8" {
            // get recorder
            let recorder_key = format!("{}:{}", platform, room_id);
            let recorders = self.recorders.read().await;
            let recorder = recorders.get(&recorder_key);
            if recorder.is_none() {
                return Err(RecorderManagerError::HLSError {
                    err: "Recorder not found".into(),
                });
            }
            let recorder = recorder.unwrap();
            let params = uri.query();
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
                    .map(|param| param[1].parse::<i64>().unwrap())
                    .unwrap_or(0)
            } else {
                0
            };
            let end = if let Some(params) = &params {
                params
                    .iter()
                    .find(|param| param[0] == "end")
                    .map(|param| param[1].parse::<i64>().unwrap())
                    .unwrap_or(0)
            } else {
                0
            };

            // response with recorder generated m3u8, which contains ts entries that cached in local
            let m3u8_content = recorder.m3u8_content(live_id, start, end).await;

            Ok(m3u8_content.into())
        } else {
            // try to find requested ts file in recorder's cache
            // cache files are stored in {cache_dir}/{room_id}/{timestamp}/{ts_file}
            let ts_file = format!("{}/{}", cache_path, path.replace("%7C", "|"));
            let recorders = self.recorders.read().await;
            let recorder_id = format!("{}:{}", platform, room_id);
            let recorder = recorders.get(&recorder_id);
            if recorder.is_none() {
                log::warn!("Recorder not found: {}", recorder_id);
                return Err(RecorderManagerError::HLSError {
                    err: "Recorder not found".into(),
                });
            }
            let ts_file_content = tokio::fs::read(&ts_file).await;
            if ts_file_content.is_err() {
                log::warn!("Segment file not found: {}", ts_file);
                return Err(RecorderManagerError::HLSError {
                    err: "Segment file not found".into(),
                });
            }

            Ok(ts_file_content.unwrap())
        }
    }

    pub async fn set_auto_start(&self, platform: PlatformType, room_id: u64, auto_start: bool) {
        // update RecordRow auto_start field
        if let Err(e) = self.db.update_recorder(platform, room_id, auto_start).await {
            log::error!("Failed to update recorder auto_start: {}", e);
        }

        let recorder_id = format!("{}:{}", platform.as_str(), room_id);
        if let Some(recorder_ref) = self.recorders.read().await.get(&recorder_id) {
            recorder_ref.set_auto_start(auto_start).await;
        }
    }

    pub async fn force_start(&self, platform: PlatformType, room_id: u64) {
        let recorder_id = format!("{}:{}", platform.as_str(), room_id);
        if let Some(recorder_ref) = self.recorders.read().await.get(&recorder_id) {
            recorder_ref.force_start().await;
        }
    }

    pub async fn force_stop(&self, platform: PlatformType, room_id: u64) {
        let recorder_id = format!("{}:{}", platform.as_str(), room_id);
        if let Some(recorder_ref) = self.recorders.read().await.get(&recorder_id) {
            recorder_ref.force_stop().await;
        }
    }
}
