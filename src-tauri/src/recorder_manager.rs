use crate::archive_storage::ArchiveStorage;
use crate::archive_timing::ArchiveTimingCache;
use crate::config::Config;
use crate::database::record::RecordRow;
use crate::database::recorder::RecorderRow;
use crate::database::video::VideoRow;
use crate::database::{Database, DatabaseError};
use crate::events::{DataChange, EventBus};
use crate::ffmpeg::{clip_timeline_anchors, transcode, Range};
use crate::progress::progress_reporter::{ProgressReporter, ProgressReporterTrait};
use crate::subtitle_generator::{item_to_srt, powerlive::PowerLiveOptions, SubtitleGeneratorType};
use crate::task::{Task, TaskManager, TaskPriority};
use crate::webhook::events::{self, Payload};
use crate::webhook::poster::WebhookPoster;
use chrono::DateTime;
use m3u8_rs::{MediaPlaylist, MediaPlaylistType};
use recorder::account::Account;
use recorder::core::playlist::normalize_archive_playlist;
use recorder::danmu::{DanmuEntry, DanmuStorage};
use recorder::errors::RecorderError;
use recorder::events::RecorderEvent;
use recorder::events::{recorder_event_channel, RecorderEventReceiver, RecorderEventSender};
use recorder::platforms::bilibili::BiliRecorder;
use recorder::platforms::douyin::DouyinRecorder;
use recorder::platforms::douyu::DouyuRecorder;
use recorder::platforms::huya::HuyaRecorder;
use recorder::platforms::kuaishou::KuaishouRecorder;
use recorder::platforms::rtmp::{RtmpConfig, RtmpRecorder};
use recorder::platforms::tiktok::TikTokRecorder;
use recorder::platforms::twitch::TwitchRecorder;
use recorder::platforms::youtube::YoutubeRecorder;
use recorder::platforms::PlatformType;
use recorder::timeline::{align_danmus_to_recording, load_skipped_ad_ranges_from_storage};
use recorder::traits::RecorderTrait;
use recorder::RoomInfo;
use recorder::UserInfo;
use recorder::{RecorderInfo, RoomCheckStatus};
use recording_storage::{LocalAssets, RecordingStorage};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
#[cfg(feature = "gui")]
use tauri_plugin_notification::NotificationExt;
use thiserror::Error;
use tokio::sync::RwLock;

#[cfg(not(feature = "headless"))]
use tauri::AppHandle;

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug)]
pub struct RecorderList {
    pub count: usize,
    pub recorders: Vec<RecorderInfo>,
}

#[derive(Serialize)]
pub struct RecorderStatus {
    pub recorder: RecorderInfo,
    pub active: bool,
    pub check: RoomCheckStatus,
    pub check_interval_secs: u64,
    pub current_record: Option<RecordRow>,
    /// Preference for the next recording; None inherits the DB default.
    /// `current_record.storage_backend_id` identifies an existing archive.
    pub storage_backend_id: Option<String>,
}

#[derive(Serialize)]
pub struct ArchiveRecord {
    #[serde(flatten)]
    pub record: RecordRow,
    /// Actual retained segment coverage in Unix milliseconds, when available.
    pub recording_started_at: Option<i64>,
    pub recording_ended_at: Option<i64>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ClipRangeParams {
    pub title: String,
    pub note: String,
    pub cover: String,
    pub platform: String,
    pub room_id: String,
    pub live_id: String,
    pub ranges: Vec<Range>,
    /// Burn the danmaku into the material instead of keeping it in a sidecar
    /// file for projects and uploads to burn later.
    #[serde(default)]
    pub danmu: bool,
    /// Video bitrate in kbps for a material with burned danmaku; none keeps
    /// the encoder's default quality. See [`ClipRangeParams::video_bitrate`].
    #[serde(default)]
    pub danmu_bitrate: Option<NonZeroU32>,
    /// Explicit encoding for newly generated materials. Omission preserves
    /// legacy stream-copy behavior and the danmu_bitrate override.
    #[serde(default)]
    pub render_settings: Option<crate::project::RenderSettings>,
    pub local_offset: i64,
    /// Re-encode the material; requests without render_settings use defaults.
    pub fix_encoding: bool,
    /// Transition effect between clips (for multiple ranges)
    pub transition: Option<String>,
}

impl ClipRangeParams {
    /// The bitrate every re-encode of this material targets: joining ranges,
    /// fixing the encoding and burning danmaku. Burning works from the earlier
    /// re-encodes, so they must not fall below the chosen bitrate either.
    pub fn video_bitrate(&self) -> Option<NonZeroU32> {
        self.danmu.then_some(self.danmu_bitrate).flatten()
    }

    /// Explicit settings already encode multiple ranges while joining.
    /// Burning also encodes, so either path satisfies a re-encode request.
    pub fn needs_final_encode(&self, danmu_embedded: bool) -> bool {
        self.render_settings.is_some() && !danmu_embedded && self.ranges.len() <= 1
    }
}

#[cfg(test)]
mod encoding_tests {
    use super::*;

    #[test]
    fn whole_clip_settings_use_camel_case_wire_and_legacy_defaults() {
        let mut wire = serde_json::json!({
            "encodeDanmu": false, "platform": "bilibili",
            "roomId": "room", "parentId": "parent"
        });
        let legacy: GenerateWholeClipParams = serde_json::from_value(wire.clone()).unwrap();
        assert!(legacy.render_settings.is_none());
        let settings = crate::project::RenderSettings::default();
        wire["renderSettings"] = serde_json::to_value(&settings).unwrap();
        let current: GenerateWholeClipParams = serde_json::from_value(wire).unwrap();
        assert_eq!(current.render_settings, Some(settings.clone()));
        assert_eq!(
            serde_json::to_value(current).unwrap()["renderSettings"],
            serde_json::to_value(settings).unwrap()
        );
    }

    fn params() -> ClipRangeParams {
        serde_json::from_value(serde_json::json!({
            "title": "", "note": "", "cover": "", "platform": "bilibili",
            "room_id": "", "live_id": "", "ranges": [],
            "local_offset": 0, "fix_encoding": false, "transition": null
        }))
        .unwrap()
    }

    #[test]
    fn omitted_material_settings_keep_legacy_copy_and_danmu_bitrate() {
        let mut params = params();
        assert!(params.render_settings.is_none());
        assert!(!params.needs_final_encode(false));
        params.danmu_bitrate = NonZeroU32::new(8000);
        assert_eq!(params.video_bitrate(), None);
        params.danmu = true;
        assert_eq!(params.video_bitrate(), NonZeroU32::new(8000));
        assert!(!params.needs_final_encode(false));
    }

    #[test]
    fn explicit_settings_guarantee_encoding_without_redundant_final_passes() {
        let mut params = params();
        params.render_settings = Some(crate::project::RenderSettings::default());
        for count in 0..=2 {
            params.ranges = vec![
                Range {
                    start: 0.0,
                    end: 1.0
                };
                count
            ];
            for danmu in [false, true] {
                params.danmu = danmu;
                for fix in [false, true] {
                    params.fix_encoding = fix;
                    // The requested danmu toggle alone is not proof of an encode:
                    // missing/empty/super-chat-only danmaku still need the guard.
                    assert_eq!(params.needs_final_encode(false), count <= 1);
                    assert!(!params.needs_final_encode(true));
                }
            }
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GenerateWholeClipParams {
    pub encode_danmu: bool,
    pub platform: String,
    pub room_id: String,
    pub parent_id: String,
    pub selected_live_ids: Option<Vec<String>>,
    pub output_name: Option<String>,
    #[serde(default)]
    pub render_settings: Option<crate::project::RenderSettings>,
}

pub struct RelatedPlaylist {
    pub live_id: String,
    pub title: String,
    pub path: PathBuf,
    // Native consumers borrow paths; retain the owned copies until they finish.
    _assets: LocalAssets,
}

/// Runs once a material the recorder manager made is saved, with the id of the
/// task that made it.
pub type MaterialHook =
    Arc<dyn Fn(String, VideoRow) -> futures::future::BoxFuture<'static, ()> + Send + Sync>;

#[derive(Clone)]
pub struct RecorderManager {
    #[cfg(not(feature = "headless"))]
    app_handle: AppHandle,
    events: EventBus,
    db: Arc<Database>,
    config: Arc<RwLock<Config>>,
    task_manager: Arc<TaskManager>,
    resource_dir: PathBuf,
    recorders: Arc<RwLock<HashMap<String, Arc<dyn RecorderTrait + Send + Sync>>>>,
    to_remove: Arc<RwLock<HashSet<String>>>,
    event_tx: RecorderEventSender,
    webhook_poster: WebhookPoster,
    material_hook: Arc<std::sync::OnceLock<MaterialHook>>,
    archive_timing: Arc<ArchiveTimingCache>,
    archive_storage: Arc<ArchiveStorage>,
}

#[derive(Error, Debug)]
pub enum RecorderManagerError {
    #[error("Recorder already exists: {room_id}")]
    AlreadyExisted { room_id: String },
    #[error("Recorder not found: {room_id}")]
    NotFound { room_id: String },
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
    #[error("Invalid live id, not timestamp str")]
    InvalidLiveID,
    #[error("Invalid program date time: {value}")]
    InvalidProgramDateTime { value: String },
}

impl From<RecorderManagerError> for String {
    fn from(err: RecorderManagerError) -> Self {
        err.to_string()
    }
}

async fn prepare_powerlive_audio(input: &Path, output: &Path) -> Result<(), RecorderManagerError> {
    ffmpeg_utils::encode_opus_upload(input, output)
        .await
        .map_err(|error| RecorderManagerError::SubtitleGenerationFailed {
            error: format!("Failed to extract opus audio: {error}"),
        })
}

/// Resolve the session even when the last recording attempt produced no archive.
async fn live_end_parent_id(
    db: &Database,
    platform: PlatformType,
    room_id: &str,
    recorder: &RecorderInfo,
) -> Result<Option<String>, DatabaseError> {
    let parent_id = if !recorder.platform_live_id.is_empty() {
        recorder.platform_live_id.clone()
    } else if !recorder.live_id.is_empty() {
        // Keep compatibility with recorders that only retain their current archive ID.
        db.get_record(room_id, &recorder.live_id).await?.parent_id
    } else {
        return Ok(None);
    };

    let records = db.get_archives_by_parent_id(room_id, &parent_id).await?;
    Ok(records
        .iter()
        .any(|record| record.platform == platform.as_str() && record.size > 0)
        .then_some(parent_id))
}

/// Room numbers are not unique across platforms. The live map already keys
/// `platform:room_id`; the catalog fallback must use the same identity or an
/// unloaded Bilibili room disappears behind a loaded Douyu room with the same number.
fn recorder_catalog_key(platform: &str, room_id: &str) -> String {
    format!("{platform}:{room_id}")
}

fn offline_recorder(recorder: RecorderRow) -> RecorderInfo {
    RecorderInfo {
        platform_live_id: String::new(),
        live_id: String::new(),
        storage_id: None,
        recording: false,
        enabled: false,
        room_info: RoomInfo {
            platform: recorder.platform,
            status: false,
            room_title: recorder.room_id.clone(),
            room_id: recorder.room_id,
            room_cover: String::new(),
        },
        user_info: UserInfo {
            user_id: String::new(),
            user_name: String::new(),
            user_avatar: String::new(),
        },
    }
}

fn merge_recorder_catalog(mut loaded: Vec<RecorderInfo>, stored: Vec<RecorderRow>) -> RecorderList {
    let mut seen = HashSet::new();
    for recorder in &loaded {
        seen.insert(recorder_catalog_key(
            &recorder.room_info.platform,
            &recorder.room_info.room_id,
        ));
    }
    let count = stored.len();
    for recorder in stored {
        if seen.insert(recorder_catalog_key(&recorder.platform, &recorder.room_id)) {
            loaded.push(offline_recorder(recorder));
        }
    }
    loaded.sort_by(|a, b| a.room_info.room_id.cmp(&b.room_info.room_id));
    RecorderList {
        count,
        recorders: loaded,
    }
}

impl RecorderManager {
    pub fn new(
        #[cfg(not(feature = "headless"))] app_handle: AppHandle,
        events: EventBus,
        db: Arc<Database>,
        config: Arc<RwLock<Config>>,
        task_manager: Arc<TaskManager>,
        resource_dir: PathBuf,
        webhook_poster: WebhookPoster,
    ) -> RecorderManager {
        let (event_tx, event_rx) = recorder_event_channel(512);
        let archive_storage = Arc::new(ArchiveStorage::new(db.clone(), config.clone()));
        let manager = RecorderManager {
            #[cfg(not(feature = "headless"))]
            app_handle,
            events,
            db,
            config,
            task_manager,
            resource_dir,
            recorders: Arc::new(RwLock::new(HashMap::new())),
            to_remove: Arc::new(RwLock::new(HashSet::new())),
            event_tx,
            webhook_poster,
            material_hook: Arc::new(std::sync::OnceLock::new()),
            archive_timing: Arc::new(ArchiveTimingCache::default()),
            archive_storage,
        };

        // Start event listener
        let manager_clone = manager.clone();
        tokio::spawn(async move {
            manager_clone.handle_events(event_rx).await;
        });

        let manager_clone = manager.clone();
        tokio::spawn(async move {
            manager_clone.monitor_recorders().await;
        });

        manager
    }

    pub(crate) fn archive_storage(&self) -> &Arc<ArchiveStorage> {
        &self.archive_storage
    }

    pub(crate) async fn recording_storage(
        &self,
        platform: PlatformType,
        room: &str,
        live: &str,
    ) -> Result<RecordingStorage, RecorderManagerError> {
        Ok(self
            .archive_storage
            .recording(platform.as_str(), room, live)
            .await?)
    }

    pub(crate) async fn materialize_recording(
        &self,
        platform: PlatformType,
        room: &str,
        live: &str,
    ) -> Result<LocalAssets, RecorderManagerError> {
        let storage = self.recording_storage(platform, room, live).await?;
        crate::recording_materialization::materialize(&storage, &[]).await
    }

    pub fn get_event_sender(&self) -> RecorderEventSender {
        self.event_tx.clone()
    }

    async fn handle_events(&self, mut rx: RecorderEventReceiver) {
        while let Some(event) = rx.recv().await {
            self.events.publish_recorder(&event);
            match event {
                RecorderEvent::RoomMetadataUpdated => {
                    self.db.notify_coalesced_change(DataChange::Recorders);
                }
                RecorderEvent::LiveStart { recorder } => {
                    self.db.notify_change(DataChange::Recorders);
                    let event = events::new_webhook_event(
                        events::LIVE_STARTED,
                        Payload::Room(recorder.clone()),
                    );
                    let _ = self.webhook_poster.post_event(&event).await;
                    if self.config.read().await.live_start_notify {
                        #[cfg(feature = "gui")]
                        self.app_handle
                            .notification()
                            .builder()
                            .title("BiliShadowReplay - 直播开始")
                            .body(format!(
                                "{} 开启了直播：{}",
                                recorder.user_info.user_name, recorder.room_info.room_title
                            ))
                            .show()
                            .unwrap_or_else(|e| {
                                log::warn!("Failed to show live start notification: {e}")
                            });
                    }
                }
                RecorderEvent::LiveEnd {
                    platform,
                    room_id,
                    recorder,
                } => {
                    self.db.notify_change(DataChange::Recorders);
                    let event = events::new_webhook_event(
                        events::LIVE_ENDED,
                        Payload::Room(recorder.clone()),
                    );
                    let _ = self.webhook_poster.post_event(&event).await;
                    self.handle_live_end(platform, &room_id, &recorder).await;
                    if self.config.read().await.live_end_notify {
                        #[cfg(feature = "gui")]
                        self.app_handle
                            .notification()
                            .builder()
                            .title("BiliShadowReplay - 直播结束")
                            .body(format!(
                                "{} 结束了直播：{}",
                                recorder.user_info.user_name, recorder.room_info.room_title
                            ))
                            .show()
                            .unwrap_or_else(|e| {
                                log::warn!("Failed to show live end notification: {e}")
                            });
                    }
                }
                RecorderEvent::RecordStart { recorder } => {
                    self.db.notify_change(DataChange::Recorders);
                    // add record entry into db
                    let platform = match PlatformType::from_str(&recorder.room_info.platform) {
                        Ok(platform) => platform,
                        Err(e) => {
                            log::error!("Invalid platform in RecordStart event: {e}");
                            continue;
                        }
                    };
                    let room_id = recorder.room_info.room_id.clone();
                    log::info!("Record start: {recorder:?}");
                    if let Err(error) = self
                        .archive_storage
                        .confirm_recording_start(
                            platform.as_str(),
                            &room_id,
                            &recorder.live_id,
                            recorder.storage_id.as_deref(),
                        )
                        .await
                    {
                        log::error!("Failed to bind recording storage: {error}");
                        self.discard_unpersisted_recorder(platform, &room_id).await;
                        continue;
                    }
                    if let Err(e) = self
                        .db
                        .add_record(
                            platform,
                            &recorder.platform_live_id,
                            &recorder.live_id,
                            &room_id,
                            &recorder.room_info.room_title,
                            None,
                        )
                        .await
                    {
                        log::error!("Failed to add record entry into db: {e}");
                        self.discard_unpersisted_recorder(platform, &room_id).await;
                        continue;
                    }
                    let event =
                        events::new_webhook_event(events::RECORD_STARTED, Payload::Room(recorder));
                    let _ = self.webhook_poster.post_event(&event).await;
                }
                RecorderEvent::RecordUpdate {
                    live_id,
                    duration_secs,
                    cached_size_bytes,
                } => {
                    let _ = self
                        .db
                        .update_record_delta(&live_id, duration_secs, cached_size_bytes)
                        .await;
                }
                RecorderEvent::RecordEnd { recorder } => {
                    self.db.notify_change(DataChange::Recorders);
                    log::info!("Record end: {recorder:?}");
                    let live_id = recorder.live_id.clone();
                    if live_id.is_empty() {
                        log::warn!(
                            "Ignoring record end without live id for room {}",
                            recorder.room_info.room_id
                        );
                        continue;
                    }
                    let event = events::new_webhook_event(
                        events::RECORD_ENDED,
                        Payload::Room(recorder.clone()),
                    );
                    let _ = self.webhook_poster.post_event(&event).await;
                    // check record in db, if length is 0, delete it
                    let room_id = recorder.room_info.room_id.clone();
                    let record = match self.db.get_record(&room_id, &live_id).await {
                        Ok(r) => r,
                        Err(e) => {
                            log::error!("Record not found in db: {recorder:?}, err={e:?}");
                            continue;
                        }
                    };
                    if record.size == 0 {
                        match PlatformType::from_str(&record.platform) {
                            Ok(platform) => {
                                if let Err(error) =
                                    self.delete_archive(platform, &room_id, &live_id).await
                                {
                                    log::error!(
                                        "Failed to remove empty recording {live_id}: {error}"
                                    );
                                }
                            }
                            Err(error) => log::error!("Invalid recording platform: {error}"),
                        }
                    }
                }
                RecorderEvent::ProgressUpdate { .. }
                | RecorderEvent::ProgressFinished { .. }
                | RecorderEvent::DanmuReceived { .. } => {}
            }
        }
    }

    async fn handle_live_end(
        &self,
        platform: PlatformType,
        room_id: &str,
        recorder: &RecorderInfo,
    ) {
        let auto_generate = self.config.read().await.auto_generate.clone();
        if !auto_generate.enabled {
            return;
        }
        if let Err(error) = auto_generate.render_settings.validate() {
            log::error!("Invalid automatic material encoding settings: {error}");
            return;
        }

        let recorder_id = format!("{}:{}", platform.as_str(), room_id);
        log::info!("Start auto generate for {recorder_id}");
        let parent_id = match live_end_parent_id(&self.db, platform, room_id, recorder).await {
            Ok(Some(parent_id)) => parent_id,
            Ok(None) => {
                log::info!("No recorded archives for ended live: {recorder_id}");
                return;
            }
            Err(error) => {
                log::error!("Failed to find ended live archives for {recorder_id}: {error}");
                return;
            }
        };

        // Capture the same immutable inputs for task metadata and its deferred
        // worker. Later settings changes apply only to newly scheduled jobs.
        let params = GenerateWholeClipParams {
            encode_danmu: auto_generate.encode_danmu,
            platform: platform.as_str().to_string(),
            room_id: room_id.to_string(),
            parent_id,
            selected_live_ids: None,
            output_name: None,
            render_settings: Some(auto_generate.render_settings),
        };
        let Ok(task) = self
            .db
            .generate_task(
                "generate_whole_clip",
                "",
                &serde_json::json!({
                    "platform": platform.as_str(),
                    "room_id": room_id,
                    "parent_id": params.parent_id,
                    "encode_danmu": params.encode_danmu,
                    "render_settings": params.render_settings,
                })
                .to_string(),
            )
            .await
        else {
            log::error!("Failed to generate task");
            return;
        };

        let Ok(reporter) = ProgressReporter::new(self.db.clone(), &self.events, &task.id).await
        else {
            log::error!("Failed to create reporter");
            let _ = self
                .db
                .update_task(&task.id, "failed", "Failed to create reporter", None)
                .await;
            return;
        };

        log::info!("Create task: {} {}", task.id, task.task_type);

        let self_clone = self.clone();
        let task_id = task.id.clone();
        let enqueue_result = self
            .task_manager
            .add_task(Task::new(
                task_id.clone(),
                TaskPriority::Normal,
                async move {
                    if let Err(e) = self_clone
                        .generate_whole_clip(Some(&reporter), params)
                        .await
                    {
                        log::error!("Failed to generate whole clip: {e}");
                        let message = format!("素材生成失败: {e}");
                        let _ = reporter.finish(false, &message).await;
                        let _ = self_clone
                            .db
                            .update_task(&task_id, "failed", &message, None)
                            .await;
                        return Err(message);
                    }

                    let _ = reporter.finish(true, "素材生成完成").await;
                    let _ = self_clone
                        .db
                        .update_task(&task_id, "success", "素材生成完成", None)
                        .await;
                    Ok(())
                },
            ))
            .await;
        if let Err(error) = enqueue_result {
            log::error!("Failed to queue whole clip for {recorder_id}: {error}");
            let _ = self
                .db
                .update_task(&task.id, "failed", &format!("加入队列失败: {error}"), None)
                .await;
        }
    }

    async fn monitor_recorders(&self) {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
        loop {
            // get a list of recorders in db, if not created yet, create them
            let recorders = match self.db.get_recorders().await {
                Ok(recorders) => recorders,
                Err(e) => {
                    log::error!("Failed to get recorders from db: {e}");
                    interval.tick().await;
                    continue;
                }
            };
            let mut recorder_map = HashMap::new();
            for recorder in recorders {
                let platform = match PlatformType::from_str(&recorder.platform) {
                    Ok(platform) => platform,
                    Err(e) => {
                        log::warn!("Skipping recorder with invalid platform: {e}");
                        continue;
                    }
                };
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
                    recorders_to_add.push((*platform, room_id.clone()));
                }
            }
            for (platform, room_id) in recorders_to_add {
                let Some((auto_start, extra)) = recorder_map.get(&(platform, room_id.clone()))
                else {
                    continue;
                };
                let account = self
                    .db
                    .get_account_by_platform(platform.clone().as_str())
                    .await;
                if platform != PlatformType::Huya
                    && platform != PlatformType::Douyu
                    && platform != PlatformType::Kuaishou
                    && platform != PlatformType::TikTok
                    && platform != PlatformType::Twitch
                    && platform != PlatformType::Youtube
                    && platform != PlatformType::Rtmp
                    && account.is_err()
                {
                    log::warn!("Failed to find an account for {platform:?} {room_id}");
                    continue;
                }
                let account = if let Ok(account) = account {
                    account.to_account()
                } else {
                    Account::default()
                };

                if let Err(e) = self
                    .add_recorder(&account, platform, &room_id, extra, *auto_start)
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

    /// Returns the resolved configuration to persist (including any allocated port).
    pub async fn add_recorder(
        &self,
        account: &Account,
        platform: PlatformType,
        room_id: &str,
        extra: &str,
        enabled: bool,
    ) -> Result<String, RecorderManagerError> {
        self.add_recorder_with_selection(account, platform, room_id, extra, enabled, None)
            .await
    }

    /// `None` loads the saved recorder choice; `Some(None)` explicitly inherits
    /// the current default for a new recorder whose DB row is not inserted yet.
    pub async fn add_recorder_with_selection(
        &self,
        account: &Account,
        platform: PlatformType,
        room_id: &str,
        extra: &str,
        enabled: bool,
        selection: Option<Option<String>>,
    ) -> Result<String, RecorderManagerError> {
        let normalized_room_id = match platform {
            PlatformType::Twitch => recorder::platforms::twitch::api::normalize_channel(room_id)
                .map_err(RecorderManagerError::RecorderError)?,
            PlatformType::Youtube => recorder::platforms::youtube::normalize_room_id(room_id)?,
            PlatformType::Rtmp => recorder::platforms::rtmp::normalize_room_id(room_id)
                .map_err(|error| RecorderError::ApiError { error })?,
            _ => room_id.to_string(),
        };
        let room_id = normalized_room_id.as_str();
        let recorder_id = format!("{}:{}", platform.as_str(), room_id);
        if self.recorders.read().await.contains_key(&recorder_id) {
            return Err(RecorderManagerError::AlreadyExisted {
                room_id: room_id.to_string(),
            });
        }

        // Validate an explicit choice at creation; actual recording writes resolve
        // the saved preference/default anew for each live ID.
        if let Some(id) = selection {
            self.archive_storage.selected_store(id.as_deref()).await?;
        }
        let storage = self.archive_storage.recording_writer();

        let event_tx = self.get_event_sender();
        let update_interval = self.config.read().await.update_interval.clone();
        let mut resolved_extra = extra.to_string();
        let recorder_boxed: Box<dyn RecorderTrait + Send + Sync> = match platform {
            PlatformType::Rtmp => {
                let recorder = RtmpRecorder::new(
                    room_id,
                    RtmpConfig::parse(extra).map_err(|error| RecorderError::ApiError { error })?,
                    storage,
                    event_tx,
                    enabled,
                )
                .await?;
                resolved_extra = serde_json::to_string(recorder.config()).map_err(|error| {
                    RecorderError::ApiError {
                        error: error.to_string(),
                    }
                })?;
                Box::new(recorder)
            }
            PlatformType::BiliBili => Box::new(BiliRecorder::new(
                room_id,
                account,
                storage,
                event_tx,
                update_interval,
                enabled,
                self.db.clone(),
            )?),
            PlatformType::Douyin => Box::new(DouyinRecorder::new(
                room_id,
                extra,
                account,
                storage,
                event_tx,
                update_interval,
                enabled,
            )?),
            PlatformType::Douyu => Box::new(DouyuRecorder::new(
                room_id,
                account,
                storage,
                event_tx,
                update_interval,
                enabled,
            )?),
            PlatformType::Huya => Box::new(HuyaRecorder::new(
                room_id,
                account,
                storage,
                event_tx,
                update_interval,
                enabled,
            )?),
            PlatformType::Kuaishou => Box::new(KuaishouRecorder::new(
                room_id,
                account,
                storage,
                event_tx,
                update_interval,
                enabled,
            )?),
            PlatformType::TikTok => Box::new(TikTokRecorder::new(
                room_id,
                account,
                storage,
                event_tx,
                update_interval,
                enabled,
            )?),
            PlatformType::Twitch => Box::new(TwitchRecorder::new(
                room_id,
                account,
                storage,
                event_tx,
                update_interval,
                enabled,
            )?),
            PlatformType::Youtube => Box::new(YoutubeRecorder::new(
                room_id,
                account,
                storage,
                event_tx,
                update_interval,
                enabled,
            )?),
            _ => {
                return Err(RecorderManagerError::InvalidPlatformType {
                    platform: platform.as_str().to_string(),
                })
            }
        };
        let events = self.events.clone();
        let topic = crate::events::RoomTopic::new(platform.as_str(), room_id);
        recorder_boxed.set_preview_sink(Arc::new(move |event| {
            events.publish_preview(&topic, &event);
        }));
        let recorder: Arc<dyn RecorderTrait + Send + Sync> = Arc::from(recorder_boxed);
        {
            // `to_remove` is taken before the recorder map, matching
            // `remove_recorder`. A catalog room can be deleted while this
            // constructor is still binding a port; inserting afterwards would
            // bring the removed room back.
            let removing = self.to_remove.read().await;
            let mut recorders = self.recorders.write().await;
            if removing.contains(&recorder_id) {
                return Err(RecorderManagerError::NotFound {
                    room_id: room_id.to_string(),
                });
            }
            // Construction can await a port bind. Another add may have won
            // while it was pending; never overwrite and orphan its listener.
            if recorders.contains_key(&recorder_id) {
                return Err(RecorderManagerError::AlreadyExisted {
                    room_id: room_id.to_string(),
                });
            }
            recorders.insert(recorder_id.clone(), recorder.clone());
        }
        // Run without holding the map's read lock: run() awaits internal locks.
        recorder.run().await;
        self.db.notify_change(DataChange::Recorders);
        // A concurrent remove_recorder/stop_all may have removed the recorder
        // while run() was starting its tasks; stop it so the tasks don't
        // outlive the removal.
        if !self.recorders.read().await.contains_key(&recorder_id) {
            recorder.stop().await;
        }
        Ok(resolved_extra)
    }

    pub async fn stop_all(&self) {
        // Deliberately hold the write lock across shutdown: it serializes
        // against add_recorder so a recorder inserted mid-shutdown can't be
        // dropped by clear() without being stopped.
        let mut recorders = self.recorders.write().await;
        for recorder in recorders.values().cloned().collect::<Vec<_>>() {
            recorder.stop().await;
        }
        recorders.clear();
    }

    pub(crate) async fn discard_unpersisted_recorder(&self, platform: PlatformType, room_id: &str) {
        let recorder = self
            .recorders
            .write()
            .await
            .remove(&format!("{}:{room_id}", platform.as_str()));
        if let Some(recorder) = recorder {
            recorder.stop().await;
        }
    }

    /// Remove a recorder from the manager
    ///
    /// This will stop the recorder and remove it from the manager
    /// and remove the related cache folder
    pub async fn remove_recorder(
        &self,
        platform: PlatformType,
        room_id: &str,
    ) -> Result<RecorderRow, RecorderManagerError> {
        // Catalog entries stay visible when startup could not construct the
        // recorder (for example the platform account is missing). Those rooms
        // are not in the live map, but their archives are real and the room
        // list still offers removal.
        let recorder_id = format!("{}:{}", platform.as_str(), room_id);
        let loaded = self.recorders.read().await.contains_key(&recorder_id);
        let in_catalog =
            self.db.get_recorders().await?.iter().any(|recorder| {
                recorder.platform == platform.as_str() && recorder.room_id == room_id
            });
        if !loaded && !in_catalog {
            return Err(RecorderManagerError::NotFound {
                room_id: room_id.to_string(),
            });
        }

        // add to to_remove before stopping, so a concurrent reload cannot
        // insert this room again while its files are being deleted.
        log::debug!("Add to to_remove: {recorder_id}");
        self.to_remove.write().await.insert(recorder_id.clone());

        // stop recorder
        log::debug!("Stop recorder: {recorder_id}");
        // Stop without holding the map's read lock: stop() awaits task shutdown.
        let recorder_ref = self.recorders.read().await.get(&recorder_id).cloned();
        if let Some(recorder_ref) = recorder_ref {
            recorder_ref.stop().await;
        }

        // Keep the recorder/index available for retry if any backend operation
        // fails. Historical archives can belong to different providers.
        let result = async {
            let room_stores = self
                .archive_storage
                .room_stores(platform.as_str(), room_id)
                .await?;
            let archives = self
                .db
                .get_room_records(platform.as_str(), room_id, 0, -1)
                .await?;
            let _mutations = archives
                .iter()
                .map(|archive| {
                    self.archive_storage.try_lock_recording(
                        platform.as_str(),
                        room_id,
                        &archive.live_id,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            for archive in &archives {
                self.recording_storage(platform, room_id, &archive.live_id)
                    .await?
                    .remove_all()
                    .await?;
            }
            for storage in room_stores {
                storage.remove_all().await?;
            }
            let recorder = self
                .db
                .remove_recorder_for_platform(platform.as_str(), room_id)
                .await?;
            for archive in archives {
                self.archive_storage
                    .forget_recording(platform.as_str(), room_id, &archive.live_id)
                    .await?;
            }
            Ok::<_, RecorderManagerError>(recorder)
        }
        .await;
        self.to_remove.write().await.remove(&recorder_id);
        let recorder = result?;

        // remove recorder
        log::debug!("Remove recorder from manager: {recorder_id}");
        self.recorders.write().await.remove(&recorder_id);
        self.db.notify_change(DataChange::Recorders);

        Ok(recorder)
    }

    async fn load_playlist_bytes(
        &self,
        platform: PlatformType,
        room_id: &str,
        live_id: &str,
    ) -> Result<Vec<u8>, RecorderManagerError> {
        Ok(self
            .recording_storage(platform, room_id, live_id)
            .await?
            .read("playlist.m3u8")
            .await?)
    }

    /// Check if the playlist is outdated
    ///
    /// This will check if the current recorder live id is the same as the live id
    /// and if the current recorder is recording
    /// and if the current recorder is recording, return false
    /// otherwise, return true
    async fn is_outdated_playlist(
        &self,
        platform: PlatformType,
        room_id: &str,
        live_id: &str,
    ) -> bool {
        // check current recorder live id is the same as the live id
        let recorder = self.get_recorder_info(platform, room_id).await;
        let Some(recorder) = recorder else {
            return true;
        };

        if recorder.live_id != live_id {
            return true;
        }

        false
    }

    async fn load_playlist(
        &self,
        platform: PlatformType,
        room_id: &str,
        live_id: &str,
    ) -> Result<MediaPlaylist, RecorderManagerError> {
        let bytes = self.load_playlist_bytes(platform, room_id, live_id).await?;
        if let Result::Ok((_, mut pl)) = m3u8_rs::parse_media_playlist(&bytes) {
            if self.is_outdated_playlist(platform, room_id, live_id).await {
                pl.end_list = true;
            }
            // Old on-disk archives may lack EVENT and a usable TARGETDURATION.
            // Repair the response too, without rewriting an actively recorded file.
            normalize_archive_playlist(&mut pl);
            return Ok(pl);
        }
        Err(RecorderManagerError::M3u8ParseFailed {
            content: String::from_utf8_lossy(&bytes).into_owned(),
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

    pub(crate) async fn first_segment_timestamp(
        &self,
        platform: PlatformType,
        room_id: &str,
        live_id: &str,
    ) -> Result<i64, RecorderManagerError> {
        let playlist = self.load_playlist(platform, room_id, live_id).await?;

        let first_segment = playlist
            .segments
            .first()
            .ok_or(RecorderManagerError::EmptyPlaylist)?;
        if let Some(program_date_time) = first_segment.program_date_time {
            return Ok(program_date_time.timestamp_millis());
        }

        // else, find in unknown tags
        let program_date_time = first_segment
            .unknown_tags
            .iter()
            .find(|t| t.tag == "X-PROGRAM-DATE-TIME");

        let Some(program_date_time) = program_date_time else {
            return live_id
                .parse::<i64>()
                .map_err(|_| RecorderManagerError::InvalidLiveID);
        };

        let Some(value) = &program_date_time.rest else {
            return live_id
                .parse::<i64>()
                .map_err(|_| RecorderManagerError::InvalidLiveID);
        };

        // example: "2025-10-18T17:18:17.004+0800"
        // convert to timestamp
        let timestamp = DateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.3f%z")
            .map_err(|_| RecorderManagerError::InvalidProgramDateTime {
                value: value.clone(),
            })?
            .timestamp_millis();
        Ok(timestamp)
    }

    pub async fn load_danmus(
        &self,
        platform: PlatformType,
        room_id: &str,
        live_id: &str,
    ) -> Result<Vec<DanmuEntry>, RecorderManagerError> {
        let storage = self.recording_storage(platform, room_id, live_id).await?;
        let key = if storage.log_exists("events.jsonl").await? {
            "events.jsonl"
        } else if storage.log_exists("danmu.txt").await? {
            "danmu.txt"
        } else {
            return Ok(Vec::new());
        };
        let danmus = DanmuStorage::read_entries(&storage, key).await?;
        if platform != PlatformType::Twitch {
            return Ok(danmus);
        }

        let skipped_ad_ranges = load_skipped_ad_ranges_from_storage(&storage).await?;
        if skipped_ad_ranges.is_empty() {
            return Ok(danmus);
        }
        let timeline_start_ms = self
            .first_segment_timestamp(platform, room_id, live_id)
            .await?;
        Ok(align_danmus_to_recording(
            danmus,
            timeline_start_ms,
            &skipped_ad_ranges,
        ))
    }

    /// The archive's danmaku with `ts` in milliseconds from the start of the
    /// recording, the time base of clip ranges. Recorded danmaku carry wall
    /// clock times; those sent before the recording started are dropped.
    pub async fn archive_danmus(
        &self,
        platform: PlatformType,
        room_id: &str,
        live_id: &str,
    ) -> Result<Vec<DanmuEntry>, RecorderManagerError> {
        let start = self
            .first_segment_timestamp(platform, room_id, live_id)
            .await?;
        let mut danmus = self.load_danmus(platform, room_id, live_id).await?;
        for danmu in &mut danmus {
            danmu.ts -= start;
        }
        danmus.retain(|danmu| danmu.ts >= 0);
        Ok(danmus)
    }

    /// Get related playlists by parent id
    ///
    /// This will return a list of tuples, the first element is the title of the archive,
    /// the second element is the path of the playlist
    async fn get_related_playlists(
        &self,
        platform: &PlatformType,
        room_id: &str,
        parent_id: &str,
        selected_live_ids: Option<&[String]>,
    ) -> Result<Vec<RelatedPlaylist>, RecorderManagerError> {
        let archives = self
            .db
            .get_archives_by_parent_id(room_id, parent_id)
            .await?;
        let mut playlists = Vec::new();
        for archive in archives
            .into_iter()
            .filter(|archive| archive.platform == platform.as_str())
            .filter(|archive| selected_live_ids.is_none_or(|ids| ids.contains(&archive.live_id)))
        {
            let assets = self
                .materialize_recording(*platform, room_id, &archive.live_id)
                .await
                .map_err(|error| RecorderManagerError::ClipError {
                    err: format!(
                        "Failed to materialize archive {}/{}/{} from playlist.m3u8: {error}",
                        platform.as_str(),
                        room_id,
                        archive.live_id
                    ),
                })?;
            playlists.push(RelatedPlaylist {
                live_id: archive.live_id,
                title: archive.title,
                path: assets.path("playlist.m3u8")?,
                _assets: assets,
            });
        }
        Ok(playlists)
    }

    /// Cut `params.ranges` into `clip_file`, returning it with the recording
    /// ranges it covers (see [`crate::ffmpeg::playlist::clip_multiple_from_playlist`]).
    pub async fn clip_range(
        &self,
        reporter: Option<&ProgressReporter>,
        clip_file: PathBuf,
        params: &ClipRangeParams,
    ) -> Result<(PathBuf, Vec<Range>), RecorderManagerError> {
        if let Some(settings) = &params.render_settings {
            settings
                .validate()
                .map_err(|err| RecorderManagerError::ClipError { err })?;
        }
        let platform = PlatformType::from_str(&params.platform).map_err(|_| {
            RecorderManagerError::InvalidPlatformType {
                platform: params.platform.clone(),
            }
        })?;
        let storage = self
            .recording_storage(platform, &params.room_id, &params.live_id)
            .await?;
        let assets =
            crate::recording_materialization::materialize(&storage, &params.ranges).await?;
        let playlist_path = assets.path("playlist.m3u8")?;

        let covered = if params.ranges.is_empty() {
            let extracting = reporter.map(|reporter| reporter.stage("提取片段"));
            crate::ffmpeg::playlist::clip_from_playlist(
                extracting.as_ref(),
                &playlist_path,
                &clip_file,
                None,
            )
            .await
            .map_err(|e| RecorderManagerError::ClipError { err: e.to_string() })?;
            Vec::new()
        } else {
            crate::ffmpeg::playlist::clip_multiple_with_settings(
                reporter,
                &playlist_path,
                &clip_file,
                &params.ranges,
                params.transition.as_deref(),
                params.video_bitrate(),
                params.render_settings.as_ref(),
            )
            .await
            .map_err(|e| RecorderManagerError::ClipError { err: e.to_string() })?
        };

        // Explicit settings defer fixing until after the optional danmaku burn,
        // avoiding an extra encode when burning already normalizes the picture.
        if params.fix_encoding && params.render_settings.is_none() {
            // transcode clip_file
            let tmp_clip_file = clip_file.with_extension("tmp.mp4");
            let fixing = reporter.map(|reporter| reporter.stage("重新编码"));
            if let Err(e) = transcode(
                fixing.as_ref(),
                &clip_file,
                &tmp_clip_file,
                false,
                params.video_bitrate(),
            )
            .await
            {
                log::error!("Failed to transcode clip file: {e}");
                return Err(RecorderManagerError::ClipError { err: e.to_string() });
            }

            // remove clip_file
            let _ = tokio::fs::remove_file(&clip_file).await;

            // rename tmp_clip_file to clip_file
            let _ = tokio::fs::rename(tmp_clip_file, &clip_file).await;
        }

        Ok((clip_file, covered))
    }

    /// Danmaku shown during the clipped ranges, with `ts` in milliseconds from
    /// the start of the resulting material. Materials keep this copy so that it
    /// survives the recording being cleaned up.
    pub async fn material_danmus(
        &self,
        params: &ClipRangeParams,
    ) -> Result<Vec<DanmuEntry>, RecorderManagerError> {
        let Ok(platform) = PlatformType::from_str(&params.platform) else {
            return Err(RecorderManagerError::InvalidPlatformType {
                platform: params.platform.clone(),
            });
        };
        let stream_start_timestamp_milis = self
            .first_segment_timestamp(platform, &params.room_id, &params.live_id)
            .await?;
        let mut danmus = self
            .load_danmus(platform, &params.room_id, &params.live_id)
            .await?;
        log::debug!("Stream start timestamp: {}", stream_start_timestamp_milis);
        log::debug!("Local offset: {}", params.local_offset);
        log::debug!("Range: {:?}", params.ranges);

        // update danmu entry ts to relative offset
        for d in &mut danmus {
            d.ts -= stream_start_timestamp_milis + params.local_offset * 1000;
        }

        if params.ranges.is_empty() {
            danmus.retain(|d| d.ts >= 0);
            return Ok(danmus);
        }

        let range_anchors = clip_timeline_anchors(&params.ranges, params.transition.as_deref());
        log::debug!("Range anchors: {:?}", range_anchors);
        let mut filtered = Vec::new();
        for (i, range) in params.ranges.iter().enumerate() {
            filtered.extend(self.filter_danmus_in_range(danmus.clone(), range, range_anchors[i]));
        }
        Ok(filtered)
    }

    fn filter_danmus_in_range(
        &self,
        mut danmus: Vec<DanmuEntry>,
        range: &Range,
        anchor: i64,
    ) -> Vec<DanmuEntry> {
        for d in &mut danmus {
            d.ts -= (range.start * 1000.0) as i64;
        }
        if range.duration() > 0.0 {
            danmus.retain(|x| x.ts >= 0 && x.ts <= (range.duration() * 1000.0).round() as i64);
        }

        for d in &mut danmus {
            d.ts += anchor;
        }

        danmus
    }

    async fn load_archive_danmus(
        &self,
        platform: PlatformType,
        room_id: &str,
        live_id: &str,
    ) -> Result<Vec<DanmuEntry>, RecorderManagerError> {
        log::info!(
            "Load archive danmu for {} {} {}",
            platform.as_str(),
            room_id,
            live_id
        );
        let first_segment_timestamp_milis = self
            .first_segment_timestamp(platform, room_id, live_id)
            .await?;
        let mut danmus = self.load_danmus(platform, room_id, live_id).await?;
        danmus.retain(|x| x.ts >= first_segment_timestamp_milis);
        for d in &mut danmus {
            d.ts -= first_segment_timestamp_milis;
        }
        Ok(danmus)
    }

    pub async fn get_recorder_list(&self) -> RecorderList {
        // Clone the Arcs out of the map so info().await doesn't hold the read lock.
        let recorders: Vec<_> = self.recorders.read().await.values().cloned().collect();
        let mut loaded = Vec::with_capacity(recorders.len());
        for recorder in recorders {
            loaded.push(recorder.info().await);
        }

        // get recorders from db
        let stored = match self.db.get_recorders().await {
            Ok(recorders) => recorders,
            Err(e) => {
                log::error!("Failed to get recorders from db: {e}");
                // Historical result on a catalog read failure: loaded rooms, count 0.
                return RecorderList {
                    count: 0,
                    recorders: loaded,
                };
            }
        };
        merge_recorder_catalog(loaded, stored)
    }

    pub async fn get_recorder_info(
        &self,
        platform: PlatformType,
        room_id: &str,
    ) -> Option<RecorderInfo> {
        let recorder_id = format!("{}:{}", platform.as_str(), room_id);
        let recorder = self.recorders.read().await.get(&recorder_id).cloned();
        if let Some(recorder) = recorder {
            Some(recorder.info().await)
        } else {
            None
        }
    }

    pub async fn get_archive_disk_usage(&self) -> Result<i64, RecorderManagerError> {
        Ok(self.db.get_record_disk_usage().await?)
    }

    pub async fn get_recorder_status(
        &self,
        platform: PlatformType,
        room_id: &str,
    ) -> Result<RecorderStatus, RecorderManagerError> {
        let recorder_id = format!("{}:{room_id}", platform.as_str());
        let recorder = self.recorders.read().await.get(&recorder_id).cloned();
        let (info, check, check_interval_secs, active) = if let Some(recorder) = recorder {
            (
                recorder.info().await,
                recorder.check_status().read().await.clone(),
                recorder
                    .update_interval()
                    .load(std::sync::atomic::Ordering::Relaxed),
                true,
            )
        } else {
            let info = self
                .get_recorder_list()
                .await
                .recorders
                .into_iter()
                .find(|info| {
                    info.room_info.platform == platform.as_str()
                        && info.room_info.room_id == room_id
                })
                .ok_or_else(|| RecorderManagerError::NotFound {
                    room_id: room_id.to_string(),
                })?;
            (info, RoomCheckStatus::default(), 0, false)
        };
        let current_record = if info.recording && !info.live_id.is_empty() {
            match self.db.get_record(room_id, &info.live_id).await {
                Ok(record) => Some(record),
                // RecordStart may still be queued when the status is requested.
                Err(DatabaseError::DB(sqlx::Error::RowNotFound)) => None,
                Err(error) => return Err(error.into()),
            }
        } else {
            None
        };
        Ok(RecorderStatus {
            recorder: info,
            active,
            check,
            check_interval_secs,
            current_record,
            storage_backend_id: self
                .db
                .recorder_storage_id(platform.as_str(), room_id)
                .await?,
        })
    }

    pub async fn archive_records(&self, records: Vec<RecordRow>) -> Vec<ArchiveRecord> {
        let mut archives = Vec::with_capacity(records.len());
        for record in records {
            let timing = match self
                .archive_storage
                .recording(&record.platform, &record.room_id, &record.live_id)
                .await
            {
                Ok(storage) => self.archive_timing.get_from_storage(&storage).await,
                Err(error) => {
                    log::warn!(
                        "Failed to resolve archive timing storage for {}: {error}",
                        record.live_id
                    );
                    None
                }
            };
            archives.push(ArchiveRecord {
                record,
                recording_started_at: timing.map(|timing| timing.started_at),
                recording_ended_at: timing.map(|timing| timing.ended_at),
            });
        }
        archives
    }

    pub async fn get_archives(
        &self,
        room_id: &str,
        offset: i64,
        limit: i64,
    ) -> Result<Vec<RecordRow>, RecorderManagerError> {
        Ok(self.db.get_records(room_id, offset, limit).await?)
    }

    pub async fn get_archive(
        &self,
        room_id: &str,
        live_id: &str,
    ) -> Result<RecordRow, RecorderManagerError> {
        Ok(self.db.get_record(room_id, live_id).await?)
    }

    pub async fn get_archive_subtitle(
        &self,
        platform: PlatformType,
        room_id: &str,
        live_id: &str,
    ) -> Result<String, RecorderManagerError> {
        let storage = self.recording_storage(platform, room_id, live_id).await?;
        let bytes = storage.read("subtitle.srt").await.map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                RecorderManagerError::SubtitleNotFound {
                    live_id: live_id.to_string(),
                }
            } else {
                error.into()
            }
        })?;
        String::from_utf8(bytes)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error).into())
    }

    pub async fn generate_archive_subtitle(
        &self,
        platform: PlatformType,
        room_id: &str,
        live_id: &str,
    ) -> Result<String, RecorderManagerError> {
        self.archive_storage.initialize().await?;
        let _mutation =
            self.archive_storage
                .try_lock_recording(platform.as_str(), room_id, live_id)?;
        let storage = self.recording_storage(platform, room_id, live_id).await?;
        let assets = crate::recording_materialization::materialize(&storage, &[]).await?;
        // Native processing reads the snapshot directly. Put outputs in a
        // separate lease so arbitrary archive names cannot collide with them.
        let temporary = storage.materialize(&[]).await?;
        let m3u8_index_file_path = assets.path("playlist.m3u8")?;
        let resources = vec![
            ffmpeg_utils::MediaResourceLease::new(assets.clone()),
            ffmpeg_utils::MediaResourceLease::new(temporary.clone()),
        ];

        // Generate a tmp mp4 clip file first
        let clip_file_path = temporary.path("tmp.mp4")?;
        let output = clip_file_path.clone();
        if let Err(e) =
            crate::ffmpeg::subtitle::run_native_with_resources(resources.clone(), async move {
                crate::ffmpeg::playlist::clip_from_playlist(
                    None::<&crate::progress::progress_reporter::ProgressReporter>,
                    &m3u8_index_file_path,
                    &output,
                    None,
                )
                .await
            })
            .await
        {
            return Err(RecorderManagerError::SubtitleGenerationFailed {
                error: e.to_string(),
            });
        }
        log::info!(
            "[{}]Temp clip file generated: {}",
            room_id,
            clip_file_path.display()
        );

        // Copy the settings out so no config guard is held across the ffmpeg
        // and subtitle jobs below.
        let (
            generator_type,
            whisper_model,
            whisper_prompt,
            openai_api_key,
            openai_api_endpoint,
            whisper_language,
            powerlive,
        ) = {
            let config = self.config.read().await;
            (
                SubtitleGeneratorType::parse(&config.subtitle_generator_type)
                    .map_err(|error| RecorderManagerError::SubtitleGenerationFailed { error })?,
                config.whisper_model.clone(),
                config.whisper_prompt.clone(),
                config.openai_api_key.clone(),
                config.openai_api_endpoint.clone(),
                config.whisper_language.clone(),
                PowerLiveOptions::for_room(&self.db, &config, platform.as_str(), room_id),
            )
        };

        // For third-party services (powerlive), extract opus audio from mp4
        let media_file_path = if generator_type.is_powerlive() {
            let opus_file_path = temporary.path("tmp.opus")?;
            log::info!("[{}]Extracting opus audio for third-party service", room_id);

            let input = clip_file_path.clone();
            let output = opus_file_path.clone();
            crate::ffmpeg::subtitle::run_native_with_resources(resources.clone(), async move {
                prepare_powerlive_audio(&input, &output)
                    .await
                    .map_err(|error| error.to_string())
            })
            .await
            .map_err(|error| RecorderManagerError::SubtitleGenerationFailed { error })?;

            log::info!(
                "[{}]Opus audio extracted: {}",
                room_id,
                opus_file_path.display()
            );
            opus_file_path
        } else {
            // For whisper/whisper_online, use mp4 directly
            clip_file_path
        };

        // generate subtitle file
        let resource_dir = self.resource_dir.clone();
        let result = crate::ffmpeg::subtitle::generate_video_subtitle_with_resources(
            None,
            &media_file_path,
            generator_type,
            &resource_dir,
            &whisper_model,
            &whisper_prompt,
            &openai_api_key,
            &openai_api_endpoint,
            &whisper_language,
            Some(powerlive),
            resources,
        )
        .await;
        // write subtitle file
        let result = match result {
            Ok(result) => result,
            Err(e) => {
                return Err(RecorderManagerError::SubtitleGenerationFailed {
                    error: e.to_string(),
                });
            }
        };
        log::info!("[{room_id}]Subtitle generated");
        let subtitle_content = result
            .subtitle_content
            .iter()
            .map(item_to_srt)
            .collect::<String>();
        storage
            .write("subtitle.srt", subtitle_content.as_bytes())
            .await?;
        log::info!("[{room_id}]Subtitle file written");
        // The lease cleans up all temporary files, including on failure.
        Ok(subtitle_content)
    }

    pub async fn delete_archive(
        &self,
        platform: PlatformType,
        room_id: &str,
        live_id: &str,
    ) -> Result<RecordRow, RecorderManagerError> {
        self.archive_storage.initialize().await?;
        let _mutation =
            self.archive_storage
                .try_lock_recording(platform.as_str(), room_id, live_id)?;
        log::info!("Deleting archive {room_id}:{live_id}");
        let record = self.db.get_record(room_id, live_id).await?;
        if record.platform != platform.as_str() || record.room_id != room_id {
            return Err(DatabaseError::NotFound.into());
        }
        if let Some(recorder) = self.get_recorder_info(platform, room_id).await {
            if recorder.live_id == live_id && recorder.recording {
                return Err(RecorderManagerError::Recording {
                    live_id: live_id.to_string(),
                });
            }
        }
        self.recording_storage(platform, room_id, live_id)
            .await?
            .remove_all()
            .await?;
        let to_delete = self.db.remove_record(live_id).await?;
        self.archive_storage
            .forget_recording(platform.as_str(), room_id, live_id)
            .await?;
        Ok(to_delete)
    }

    pub async fn delete_archives(
        &self,
        platform: PlatformType,
        room_id: &str,
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
        let path = uri.split('?').next().unwrap_or(uri);
        let params = uri.split('?').nth(1).unwrap_or("");
        let path_segs: Vec<&str> = path.split('/').collect();

        if path_segs.len() < 4 {
            log::warn!("Invalid request path: {path}");
            return Err(RecorderManagerError::HLSError {
                err: "Invalid hls path".into(),
            });
        }
        recording_storage::validate_key(path)?;
        // parse recorder type
        let platform = path_segs[0];
        // parse room id
        let room_id = path_segs[1];
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

        // Parse `start` / `end` query params defensively: a segment URI may
        // carry arbitrary or malformed query strings.
        let parse_param = |name: &str| -> i64 {
            let value = params.as_ref().and_then(|params| {
                params.iter().find_map(|param| match param.as_slice() {
                    [key, value] if *key == name => Some(*value),
                    _ => None,
                })
            });
            match value {
                Some(value) => value.parse::<i64>().unwrap_or_else(|e| {
                    log::warn!("Invalid {name} parameter value {value:?}: {e}");
                    0
                }),
                None => 0,
            }
        };
        let start = parse_param("start");
        let end = parse_param("end");

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

        // Check if this is a playlist request
        // The remaining path after platform/room_id/live_id could be:
        // - "playlist.m3u8" (4 segments total)
        // - "some_dir/playlist.m3u8" (5+ segments)
        // - "segment.ts" (4 segments total)
        // - "some_dir/segment.ts" (5+ segments)
        let remaining_path = path_segs[3..].join("/");

        if remaining_path == "playlist.m3u8" {
            let playlist = self.load_playlist(platform, room_id, live_id).await?;
            let playlist = self.playlist_range(&playlist, range).await?;
            let mut bytes: Vec<u8> = Vec::new();
            playlist.write_to(&mut bytes)?;
            Ok(bytes)
        } else {
            Ok(self
                .recording_storage(platform, room_id, live_id)
                .await?
                .read(&remaining_path)
                .await?)
        }
    }

    pub async fn set_recorder_storage(
        &self,
        platform: PlatformType,
        room_id: &str,
        storage_backend_id: Option<String>,
    ) -> Result<RecorderRow, RecorderManagerError> {
        self.archive_storage
            .selected_store(storage_backend_id.as_deref())
            .await?;
        // This is a preference for the next recording. Existing bindings and
        // running writers are deliberately unchanged.
        Ok(self
            .db
            .set_recorder_storage(platform.as_str(), room_id, storage_backend_id.as_deref())
            .await?)
    }

    pub async fn set_enable(&self, platform: PlatformType, room_id: &str, enabled: bool) {
        // update RecordRow auto_start field
        if let Err(e) = self.db.update_recorder(platform, room_id, enabled).await {
            log::error!("Failed to update recorder auto_start: {e}");
        }

        let recorder_id = format!("{}:{}", platform.as_str(), room_id);
        let recorder = self.recorders.read().await.get(&recorder_id).cloned();
        if let Some(recorder) = recorder {
            if enabled {
                recorder.enable().await;
            } else {
                recorder.disable().await;
            }
            self.db.notify_change(DataChange::Recorders);
        }
    }

    /// Set what runs after a whole-live material is saved. Only the first call
    /// takes effect.
    pub fn on_material_created(&self, hook: MaterialHook) {
        let _ = self.material_hook.set(hook);
    }

    pub async fn generate_whole_clip(
        &self,
        reporter: Option<&ProgressReporter>,
        params: GenerateWholeClipParams,
    ) -> Result<(), RecorderManagerError> {
        if let Some(settings) = &params.render_settings {
            settings
                .validate()
                .map_err(|err| RecorderManagerError::ClipError { err })?;
        }
        let GenerateWholeClipParams {
            encode_danmu,
            platform,
            room_id,
            parent_id,
            selected_live_ids,
            output_name,
            render_settings,
        } = params;

        let platform = PlatformType::from_str(&platform).map_err(|_| {
            RecorderManagerError::InvalidPlatformType {
                platform: platform.to_string(),
            }
        })?;

        let mut playlists = self
            .get_related_playlists(
                &platform,
                &room_id,
                &parent_id,
                selected_live_ids.as_deref(),
            )
            .await?;
        if playlists.is_empty() {
            log::error!("No related playlists found: {parent_id}");
            return Ok(());
        }

        if let Some(selected_live_ids) = selected_live_ids {
            let mut by_id = std::collections::HashMap::new();
            for playlist in playlists {
                by_id.insert(playlist.live_id.clone(), playlist);
            }
            let mut ordered = Vec::new();
            for live_id in selected_live_ids {
                if let Some(playlist) = by_id.remove(&live_id) {
                    ordered.push(playlist);
                }
            }
            playlists = ordered;
        }

        let Some(first_playlist) = playlists.first() else {
            log::error!("No selected playlists found: {parent_id}");
            return Ok(());
        };

        let title = first_playlist.title.clone();

        // Keep image metadata through concatenation; the common encoder prepares
        // each playlist's text and images together in its own temporary directory.
        let danmu_options = self.config.read().await.danmu_ass_options.clone();
        let danmu_tracks = if encode_danmu {
            let mut tracks = Vec::with_capacity(playlists.len());
            for playlist in &playlists {
                tracks.push(Some(
                    self.load_archive_danmus(platform, &room_id, &playlist.live_id)
                        .await?,
                ));
            }
            tracks
        } else {
            vec![None; playlists.len()]
        };

        let output_filename = if let Some(output_name) = output_name {
            let trimmed = output_name.trim();
            if trimmed.is_empty() {
                None
            } else {
                let mut sanitized = sanitize_filename::sanitize(trimmed);
                if !sanitized.to_lowercase().ends_with(".mp4") {
                    sanitized.push_str(".mp4");
                }
                Some(std::path::PathBuf::from(sanitized))
            }
        } else {
            None
        };

        let output_filename = if let Some(output_filename) = output_filename {
            output_filename
        } else {
            let timestamp = chrono::Local::now().format("%Y%m%d%H%M%S").to_string();
            let sanitized_filename = sanitize_filename::sanitize(format!(
                "[full][{platform:?}][{room_id}][{parent_id}][{timestamp}]{title}.mp4"
            ));
            std::path::PathBuf::from(sanitized_filename)
        };

        let cover_filename = output_filename.with_extension("jpg");

        let output_path =
            Path::new(&self.config.read().await.output.as_str()).join(&output_filename);

        let playlists_refs: Vec<&Path> = playlists.iter().map(|p| p.path.as_path()).collect();

        log::info!("Concat playlists: {playlists_refs:?}");
        log::info!("Output path: {output_path:?}");

        let danmu_embedded = match crate::ffmpeg::playlist::concat_playlists_to_video(
            reporter,
            &playlists_refs,
            danmu_tracks,
            &danmu_options,
            &output_path,
            render_settings.as_ref(),
        )
        .await
        {
            Ok(burned) => burned,
            Err(e) => {
                log::error!("Failed to concat playlists: {e}");
                return Err(RecorderManagerError::HLSError {
                    err: format!("Failed to concat playlists: {e}"),
                });
            }
        };

        let size = std::fs::metadata(&output_path)
            .map_err(|_| RecorderManagerError::HLSError {
                err: "Failed to get file metadata".into(),
            })?
            .len() as i64;

        let mut duration = 0.0;
        match ffmpeg_utils::extract_video_metadata(Path::new(&output_path)).await {
            Ok(video_metadata) => duration = video_metadata.duration,
            Err(e) => log::error!("Failed to get video metadata: {e}"),
        }

        let _ = crate::ffmpeg::generate_thumbnail(
            &output_path,
            crate::ffmpeg::get_random_thumbnail_timestamp(duration),
        )
        .await;
        let _ = crate::ffmpeg::extract_audio_sample(Path::new(&output_path)).await;

        let video = self
            .db
            .add_video(&VideoRow {
                id: 0,
                status: 0,
                room_id: room_id.to_string(),
                created_at: chrono::Local::now().to_rfc3339(),
                cover: cover_filename.to_string_lossy().to_string(),
                file: output_filename.to_string_lossy().to_string(),
                note: "".into(),
                length: duration as i64,
                size,
                bvid: String::new(),
                title: String::new(),
                desc: String::new(),
                tags: String::new(),
                area: 0,
                platform: platform.as_str().to_string(),
                live_id: String::new(),
                source: String::new(),
                duration,
                danmu_embedded,
            })
            .await?;

        let event =
            events::new_webhook_event(events::CLIP_GENERATED, events::Payload::Clip(video.clone()));
        if let Err(e) = self.webhook_poster.post_event(&event).await {
            log::error!("Post webhook event error: {e}");
        }

        if let Some(hook) = self.material_hook.get() {
            let task_id = reporter.map(|r| r.event_id.clone()).unwrap_or_default();
            hook(task_id, video).await;
        }

        Ok(())
    }
}

#[cfg(test)]
mod powerlive_audio_tests {
    use super::*;

    #[derive(Default)]
    struct Count(u64);
    impl ffmpeg_utils::AudioSink for Count {
        fn write(&mut self, samples: &[i16]) -> Result<(), String> {
            self.0 += samples.len() as u64;
            Ok(())
        }
    }

    fn assert_no_partial_opus(directory: &Path) {
        assert!(std::fs::read_dir(directory).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".opus-")));
    }

    #[tokio::test]
    async fn powerlive_preparation_keeps_stereo_and_publishes_the_complete_upload() {
        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join("录播 音频.wav");
        let output = directory.path().join("tmp.opus");
        let mut writer = hound::WavWriter::create(
            &input,
            hound::WavSpec {
                channels: 2,
                sample_rate: 48_000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for index in 0..48_017 {
            let sample = (12_000.0 * (index as f64 * 0.031).sin()) as i16;
            writer.write_sample(sample).unwrap();
            writer.write_sample(sample / 2).unwrap();
        }
        writer.finalize().unwrap();
        let original = std::fs::read(&input).unwrap();
        std::fs::write(&output, b"previous upload").unwrap();
        prepare_powerlive_audio(&input, &output).await.unwrap();
        let data = std::fs::read(&output).unwrap();
        assert_eq!(&data[..4], b"OggS");
        let head = data
            .windows(8)
            .position(|part| part == b"OpusHead")
            .unwrap();
        assert_eq!(data[head + 9], 2);
        assert_eq!(
            u32::from_le_bytes(data[head + 12..head + 16].try_into().unwrap()),
            48_000
        );
        let decoded = ffmpeg_utils::decode_audio(
            &output,
            ffmpeg_utils::AudioDecodeOptions {
                sample_rate: 48_000,
            },
            Count::default(),
        )
        .await
        .unwrap();
        assert!(decoded.has_audio);
        assert_eq!(decoded.samples, 48_017);
        assert_eq!(decoded.sink.0, decoded.samples);
        assert_eq!(std::fs::read(&input).unwrap(), original);
        assert_no_partial_opus(directory.path());
    }

    #[tokio::test]
    async fn failed_powerlive_preparation_reports_a_subtitle_error_and_keeps_previous_upload() {
        let directory = tempfile::tempdir().unwrap();
        let invalid = directory.path().join("invalid.wav");
        std::fs::write(&invalid, b"not media").unwrap();
        let output = directory.path().join("tmp.opus");
        std::fs::write(&output, b"previous upload").unwrap();
        let video = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/video/test.mp4");
        for source in [
            &invalid,
            &directory.path().join("missing.wav"),
            &video,
            &output,
        ] {
            let error = prepare_powerlive_audio(source, &output).await.unwrap_err();
            assert!(
                matches!(error, RecorderManagerError::SubtitleGenerationFailed { error } if error.starts_with("Failed to extract opus audio: "))
            );
            assert_eq!(std::fs::read(&output).unwrap(), b"previous upload");
            assert_no_partial_opus(directory.path());
        }
    }
}

#[cfg(test)]
mod live_end_tests {
    use super::*;

    async fn database() -> Database {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE records (
                live_id TEXT PRIMARY KEY, platform TEXT, parent_id TEXT,
                room_id TEXT, title TEXT, length REAL, size INTEGER,
                created_at TEXT, cover TEXT
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE tasks (
                id TEXT PRIMARY KEY, type TEXT, status TEXT, message TEXT,
                metadata TEXT, created_at TEXT
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        Database::new(
            pool,
            crate::database::credentials::CredentialCipher::new(&[0; 32]).unwrap(),
            EventBus::new(),
        )
    }

    fn ended_recorder(parent_id: &str, live_id: &str) -> RecorderInfo {
        RecorderInfo {
            platform_live_id: parent_id.into(),
            live_id: live_id.into(),
            storage_id: None,
            room_info: RoomInfo::default(),
            user_info: UserInfo::default(),
            recording: false,
            enabled: true,
        }
    }

    async fn add_archive(db: &Database, platform: PlatformType, parent: &str, live: &str) {
        db.add_record(platform, parent, live, "10220184", "test", None)
            .await
            .unwrap();
        db.update_record_delta(live, 4.0, 1024).await.unwrap();
    }

    #[cfg(feature = "headless")]
    #[tokio::test]
    async fn auto_generate_live_end_validates_and_snapshots_queued_settings() {
        use crate::project::{EncodingPriority, RenderSettings, VideoCodec};
        use crate::task::TaskManagerConfig;
        use crate::webhook::poster::WebhookConfig;
        use tokio::time::{timeout, Duration};

        let dir = tempfile::tempdir().unwrap();
        let db = Arc::new(database().await);
        add_archive(&db, PlatformType::BiliBili, "session", "archive").await;
        let config = Arc::new(RwLock::new(
            Config::load(&dir.path().join("config.toml"), dir.path(), dir.path()).unwrap(),
        ));
        let mut tasks = TaskManager::with_config(TaskManagerConfig {
            check_interval_ms: 5,
            ..TaskManagerConfig::default()
        });
        tasks.start();
        let tasks = Arc::new(tasks);
        // Occupy the only worker so settings can change after submission.
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        tasks
            .add_task(Task::new(
                "blocker".into(),
                TaskPriority::Normal,
                async move {
                    let _ = started_tx.send(());
                    let _ = release_rx.await;
                    Ok(())
                },
            ))
            .await
            .unwrap();
        timeout(Duration::from_secs(2), started_rx)
            .await
            .unwrap()
            .unwrap();
        let manager = RecorderManager::new(
            EventBus::new(),
            db.clone(),
            config.clone(),
            tasks.clone(),
            dir.path().into(),
            WebhookPoster::new(WebhookConfig::default()).unwrap(),
        );
        let ended = ended_recorder("session", "archive");
        manager
            .handle_live_end(PlatformType::BiliBili, "10220184", &ended)
            .await;
        assert!(db.get_tasks().await.unwrap().is_empty());
        {
            let mut config = config.write().await;
            config.auto_generate.enabled = true;
            config.auto_generate.render_settings.bitrate_kbps = 0;
        }
        manager
            .handle_live_end(PlatformType::BiliBili, "10220184", &ended)
            .await;
        assert!(db.get_tasks().await.unwrap().is_empty());
        let selected = RenderSettings {
            codec: VideoCodec::H264,
            priority: EncodingPriority::Quality,
            bitrate_kbps: 16_000,
            resolution: crate::project::RenderResolution::Original,
        };
        {
            let mut config = config.write().await;
            config.auto_generate.encode_danmu = true;
            config.auto_generate.render_settings = selected.clone();
        }
        manager
            .handle_live_end(PlatformType::BiliBili, "10220184", &ended)
            .await;
        let queued = db.get_tasks().await.unwrap().pop().unwrap();
        let metadata: serde_json::Value = serde_json::from_str(&queued.metadata).unwrap();
        assert_eq!(metadata["encode_danmu"], true);
        assert_eq!(
            metadata["render_settings"],
            serde_json::to_value(selected).unwrap()
        );
        {
            let mut config = config.write().await;
            config.auto_generate.enabled = false;
            config.auto_generate.encode_danmu = false;
            config.auto_generate.render_settings.bitrate_kbps = 0;
        }
        release_tx.send(()).unwrap();
        let finished = timeout(Duration::from_secs(3), async {
            loop {
                let task = db.get_task(&queued.id).await.unwrap();
                if task.status == "failed" || task.status == "success" {
                    break task;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        // This fixture deliberately has no playlist: reaching materialization proves
        // the worker kept valid scheduled settings instead of rereading zero.
        assert!(
            finished.message.contains("Failed to materialize archive"),
            "{}",
            finished.message
        );
        assert!(
            finished.message.contains("bilibili/10220184/archive")
                && finished.message.contains("playlist.m3u8")
                && finished.message.contains("IO error:"),
            "the task must retain the failing recording and its cause: {}",
            finished.message
        );
        assert_eq!(finished.metadata, queued.metadata);
    }

    #[tokio::test]
    async fn resolves_entire_session_without_a_last_archive() {
        let db = database().await;
        add_archive(&db, PlatformType::BiliBili, "session-1", "segment-1").await;
        add_archive(&db, PlatformType::BiliBili, "session-1", "segment-2").await;
        add_archive(&db, PlatformType::BiliBili, "old-session", "old-segment").await;

        // Covers both a reset current ID and a removed zero-byte final segment.
        for live_id in ["", "removed-empty-segment"] {
            let parent = live_end_parent_id(
                &db,
                PlatformType::BiliBili,
                "10220184",
                &ended_recorder("session-1", live_id),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(parent, "session-1");
            let archives = db
                .get_archives_by_parent_id("10220184", &parent)
                .await
                .unwrap();
            assert_eq!(archives.len(), 2);
            assert!(archives
                .iter()
                .all(|archive| archive.live_id.starts_with("segment-")));
        }
    }

    #[tokio::test]
    async fn skips_unrecorded_sessions_instead_of_using_another_live() {
        let db = database().await;
        add_archive(&db, PlatformType::BiliBili, "old-session", "old-segment").await;
        add_archive(&db, PlatformType::Douyin, "session-1", "other-platform").await;
        db.add_record(
            PlatformType::BiliBili,
            "session-1",
            "empty-segment",
            "10220184",
            "test",
            None,
        )
        .await
        .unwrap();

        for parent_id in ["", "session-1", "unrecorded-session"] {
            assert_eq!(
                live_end_parent_id(
                    &db,
                    PlatformType::BiliBili,
                    "10220184",
                    &ended_recorder(parent_id, ""),
                )
                .await
                .unwrap(),
                None
            );
        }
    }

    #[tokio::test]
    async fn supports_recorders_that_only_provide_an_archive_id() {
        let db = database().await;
        add_archive(&db, PlatformType::Douyin, "session-1", "segment-1").await;
        assert_eq!(
            live_end_parent_id(
                &db,
                PlatformType::Douyin,
                "10220184",
                &ended_recorder("", "segment-1"),
            )
            .await
            .unwrap(),
            Some("session-1".into())
        );
    }
}

#[cfg(test)]
mod hls_path_tests {
    #[test]
    fn accepts_scoped_segment_keys() {
        for path in [
            "bilibili/1/2/segment.ts",
            "bilibili/1/2/some_dir/segment.ts",
        ] {
            recording_storage::validate_key(path).unwrap();
        }
    }

    #[test]
    fn rejects_paths_escaping_the_recording_namespace() {
        for path in [
            "bilibili/1/2/../../../../secret.txt",
            "bilibili/../../secret.txt",
            "bilibili/1/2/./segment.ts",
            "bilibili/1/2//segment.ts",
            "bilibili/1/2/.. /.. /.. /.. /secret.txt",
            "bilibili/1/2/.. ./secret.txt",
            "bilibili/1/2/.../secret.txt",
            "/secret.txt",
            "bilibili\\..\\..\\secret.txt",
            "bilibili/1/2/C:",
            "bilibili/1/2/C:\\secret.txt",
            "bilibili/1/2/.. \\/.. \\/secret.txt",
        ] {
            assert!(recording_storage::validate_key(path).is_err(), "{path:?}");
        }
    }
}

#[cfg(test)]
mod recorder_catalog_tests {
    use super::{merge_recorder_catalog, RecorderInfo, RecorderRow, RoomInfo, UserInfo};

    fn loaded(platform: &str, room_id: &str) -> RecorderInfo {
        RecorderInfo {
            platform_live_id: "live".into(),
            live_id: "archive".into(),
            storage_id: None,
            recording: true,
            enabled: true,
            room_info: RoomInfo {
                platform: platform.into(),
                status: true,
                room_id: room_id.into(),
                room_title: format!("{platform} title"),
                room_cover: String::new(),
            },
            user_info: UserInfo {
                user_id: "user".into(),
                user_name: platform.into(),
                user_avatar: String::new(),
            },
        }
    }

    fn stored(platform: &str, room_id: &str) -> RecorderRow {
        RecorderRow {
            room_id: room_id.into(),
            created_at: String::new(),
            platform: platform.into(),
            auto_start: true,
            extra: String::new(),
            storage_backend_id: None,
        }
    }

    fn keys(list: &super::RecorderList) -> Vec<String> {
        list.recorders
            .iter()
            .map(|recorder| {
                format!(
                    "{}:{}",
                    recorder.room_info.platform, recorder.room_info.room_id
                )
            })
            .collect()
    }

    #[test]
    fn unloaded_room_stays_visible_when_another_platform_uses_the_same_number() {
        // Douyu 6 is running. Bilibili 6 is still in the catalog but failed to
        // load (for example its account is missing). Both must stay listed, or
        // the archive page never asks for the Bilibili recordings.
        let list = merge_recorder_catalog(
            vec![loaded("douyu", "6")],
            vec![stored("bilibili", "6"), stored("douyu", "6")],
        );

        assert_eq!(list.count, 2);
        let mut identities = keys(&list);
        identities.sort();
        assert_eq!(
            identities,
            vec!["bilibili:6".to_string(), "douyu:6".to_string()]
        );
        let bilibili = list
            .recorders
            .iter()
            .find(|recorder| recorder.room_info.platform == "bilibili")
            .unwrap();
        assert!(!bilibili.recording);
        assert!(!bilibili.enabled);
        assert_eq!(bilibili.room_info.room_title, "6");
        let douyu = list
            .recorders
            .iter()
            .find(|recorder| recorder.room_info.platform == "douyu")
            .unwrap();
        assert!(douyu.recording);
        assert_eq!(douyu.room_info.room_title, "douyu title");
    }

    #[test]
    fn loaded_room_is_not_listed_twice() {
        let list = merge_recorder_catalog(
            vec![loaded("bilibili", "6"), loaded("douyu", "7")],
            vec![stored("bilibili", "6"), stored("douyu", "7")],
        );

        assert_eq!(list.count, 2);
        assert_eq!(
            keys(&list),
            vec!["bilibili:6".to_string(), "douyu:7".to_string()]
        );
    }
}

#[cfg(all(test, feature = "headless"))]
mod archive_storage_tests {
    use super::*;
    use recording_storage::RecordingStore;

    async fn manager(dir: &Path) -> RecorderManager {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::raw_sql(
            "CREATE TABLE records (
                live_id TEXT PRIMARY KEY, platform TEXT, parent_id TEXT,
                room_id TEXT, title TEXT, length REAL, size INTEGER,
                created_at TEXT, cover TEXT
            );
            CREATE TABLE record_summaries (platform TEXT, room_id TEXT, live_id TEXT);
            CREATE TABLE recorders (
                room_id TEXT NOT NULL, platform TEXT NOT NULL, created_at TEXT,
                auto_start BOOLEAN, extra TEXT, storage_backend_id TEXT,
                PRIMARY KEY (platform, room_id)
            );",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::raw_sql(crate::database::storage::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        let db = Arc::new(Database::new(
            pool,
            crate::database::credentials::CredentialCipher::new(&[0; 32]).unwrap(),
            EventBus::new(),
        ));
        let config = Arc::new(RwLock::new(
            Config::load(&dir.join("config.toml"), dir, dir).unwrap(),
        ));
        let archive_storage = Arc::new(ArchiveStorage::new(db.clone(), config.clone()));
        let (event_tx, _) = recorder_event_channel(16);
        // No monitor/event tasks: tests control every mutation explicitly.
        RecorderManager {
            events: EventBus::new(),
            db,
            config,
            task_manager: Arc::new(TaskManager::new()),
            resource_dir: dir.into(),
            recorders: Arc::new(RwLock::new(HashMap::new())),
            to_remove: Arc::new(RwLock::new(HashSet::new())),
            event_tx,
            webhook_poster: WebhookPoster::new(crate::webhook::poster::WebhookConfig::default())
                .unwrap(),
            material_hook: Arc::new(std::sync::OnceLock::new()),
            archive_timing: Arc::new(ArchiveTimingCache::default()),
            archive_storage,
        }
    }

    #[tokio::test]
    async fn a_saved_preference_change_applies_to_the_next_archive_without_reconstruction() {
        let directory = tempfile::tempdir().unwrap();
        let manager = manager(directory.path()).await;
        for id in ["first", "second"] {
            manager
                .archive_storage
                .register_backend(id, RecordingStore::local(directory.path().join(id)))
                .await
                .unwrap();
        }
        manager
            .db
            .add_recorder_with_storage(PlatformType::BiliBili, "room", "", Some("first"))
            .await
            .unwrap();
        let writer = manager.archive_storage.recording_writer();
        let first = writer
            .resolve_recording("bilibili", "room", "first-live")
            .await
            .unwrap();
        manager
            .set_recorder_storage(PlatformType::BiliBili, "room", Some("second".into()))
            .await
            .unwrap();
        let second = writer
            .resolve_recording("bilibili", "room", "second-live")
            .await
            .unwrap();
        assert_eq!(first.identity(), Some("first"));
        drop(first);
        let resumed = writer
            .resolve_recording("bilibili", "room", "first-live")
            .await
            .unwrap();
        assert_eq!(resumed.identity(), Some("first"));
        assert_eq!(second.identity(), Some("second"));
    }

    #[tokio::test]
    async fn storage_preference_can_change_without_stopping_an_active_recording() {
        use recorder::traits::RecorderBasicTrait;
        let directory = tempfile::tempdir().unwrap();
        let manager = manager(directory.path()).await;
        manager
            .db
            .add_recorder(PlatformType::Rtmp, "rtmp-obs", "{}")
            .await
            .unwrap();
        let recorder = RtmpRecorder::new(
            "obs",
            RtmpConfig {
                listen_addr: "127.0.0.1".parse().unwrap(),
                ..RtmpConfig::default()
            },
            directory.path().to_path_buf(),
            manager.event_tx.clone(),
            true,
        )
        .await
        .unwrap();
        recorder
            .is_recording()
            .store(true, std::sync::atomic::Ordering::Relaxed);
        *recorder.live_id().write().await = "active".into();
        manager
            .recorders
            .write()
            .await
            .insert("rtmp:rtmp-obs".into(), Arc::new(recorder));
        manager
            .set_recorder_storage(PlatformType::Rtmp, "rtmp-obs", Some("default".into()))
            .await
            .unwrap();
        assert_eq!(
            manager
                .db
                .recorder_storage_id("rtmp", "rtmp-obs")
                .await
                .unwrap()
                .as_deref(),
            Some("default")
        );
        assert!(
            manager
                .get_recorder_info(PlatformType::Rtmp, "rtmp-obs")
                .await
                .unwrap()
                .recording
        );
    }

    async fn archive(
        manager: &RecorderManager,
        platform: PlatformType,
        live: &str,
        backend: &str,
    ) -> RecordingStorage {
        manager
            .archive_storage
            .bind_recording(platform.as_str(), "room", live, Some(backend))
            .await
            .unwrap();
        manager
            .db
            .add_record(platform, "session", live, "room", "archive", None)
            .await
            .unwrap();
        let storage = manager
            .recording_storage(platform, "room", live)
            .await
            .unwrap();
        storage
            .write("subtitle.srt", live.as_bytes())
            .await
            .unwrap();
        storage.write("segment.ts", live.as_bytes()).await.unwrap();
        storage
    }

    #[tokio::test]
    async fn reads_and_deletes_the_pinned_backend_not_the_current_default() {
        let dir = tempfile::tempdir().unwrap();
        let manager = manager(dir.path()).await;
        for id in ["first", "second"] {
            manager
                .archive_storage
                .register_backend(id, RecordingStore::local(dir.path().join(id)))
                .await
                .unwrap();
        }
        let first = archive(&manager, PlatformType::BiliBili, "old", "first").await;
        let second = archive(&manager, PlatformType::BiliBili, "new", "second").await;
        let other_platform = archive(&manager, PlatformType::Twitch, "twitch", "first").await;
        manager.config.write().await.storage = crate::config::StorageConfig::local(
            dir.path().join("unrelated").to_string_lossy().into_owned(),
        );

        assert_eq!(
            manager
                .get_archive_subtitle(PlatformType::BiliBili, "room", "old")
                .await
                .unwrap(),
            "old"
        );
        assert_eq!(
            manager
                .get_archive_subtitle(PlatformType::BiliBili, "room", "new")
                .await
                .unwrap(),
            "new"
        );
        assert_eq!(
            manager
                .handle_hls_request("bilibili/room/old/segment.ts")
                .await
                .unwrap(),
            b"old"
        );
        manager
            .delete_archive(PlatformType::BiliBili, "room", "old")
            .await
            .unwrap();
        assert!(!first.exists("segment.ts").await.unwrap());
        assert_eq!(second.read("segment.ts").await.unwrap(), b"new");
        assert_eq!(other_platform.read("segment.ts").await.unwrap(), b"twitch");
        assert!(manager.db.get_record("room", "old").await.is_err());
        assert!(manager.db.get_record("room", "new").await.is_ok());
    }

    #[tokio::test]
    async fn wrong_platform_or_room_cannot_delete_an_archive() {
        let dir = tempfile::tempdir().unwrap();
        let manager = manager(dir.path()).await;
        manager
            .archive_storage
            .register_backend("first", RecordingStore::local(dir.path().join("first")))
            .await
            .unwrap();
        let storage = archive(&manager, PlatformType::BiliBili, "old", "first").await;
        assert!(manager
            .delete_archive(PlatformType::Twitch, "room", "old")
            .await
            .is_err());
        assert!(manager
            .delete_archive(PlatformType::BiliBili, "another-room", "old")
            .await
            .is_err());
        assert_eq!(storage.read("segment.ts").await.unwrap(), b"old");
        assert!(manager.db.get_record("room", "old").await.is_ok());
    }

    #[tokio::test]
    async fn recording_setup_and_logs_block_deletion_and_deleted_resumes_cannot_rebind() {
        let dir = tempfile::tempdir().unwrap();
        let manager = manager(dir.path()).await;
        let original = archive(&manager, PlatformType::BiliBili, "old", "default").await;
        let writer = manager.archive_storage.recording_writer();
        // No running recorder/is_recording flag: this is the setup window.
        let setup = writer
            .resolve_recording("bilibili", "room", "old")
            .await
            .unwrap();
        assert!(matches!(
            manager.delete_archive(PlatformType::BiliBili, "room", "old").await,
            Err(RecorderManagerError::IoError(error)) if error.kind() == std::io::ErrorKind::WouldBlock
        ));
        let log = setup.open_log("events.jsonl").await.unwrap();
        drop(setup);
        assert!(manager
            .delete_archive(PlatformType::BiliBili, "room", "old")
            .await
            .is_err());
        log.flush().await.unwrap();
        drop(log);
        assert_eq!(original.read("subtitle.srt").await.unwrap(), b"old");

        // Capture a pre-delete initializer as well as the delayed resume resolver.
        let captured = manager
            .archive_storage
            .selected_store(None)
            .await
            .unwrap()
            .recording("bilibili", "room", "old")
            .unwrap();
        manager
            .delete_archive(PlatformType::BiliBili, "room", "old")
            .await
            .unwrap();
        assert_eq!(
            writer
                .resolve_recording("bilibili", "room", "old")
                .await
                .err()
                .unwrap()
                .kind(),
            std::io::ErrorKind::NotFound
        );
        assert_eq!(
            captured.prepare().await.unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
        assert!(manager.db.get_record("room", "old").await.is_err());
        assert!(manager
            .db
            .recording_storage_id("bilibili", "room", "old")
            .await
            .unwrap()
            .is_none());
        assert!(!original.exists("subtitle.srt").await.unwrap());

        // A fresh registry cannot forget the durable deletion marker.
        let restarted = Arc::new(ArchiveStorage::new(
            manager.db.clone(),
            manager.config.clone(),
        ));
        assert!(restarted
            .recording_writer()
            .resolve_recording("bilibili", "room", "old")
            .await
            .is_err());
        let fresh = restarted
            .recording_writer()
            .resolve_recording("bilibili", "room", "new")
            .await
            .unwrap();
        fresh.prepare().await.unwrap();
        assert_eq!(fresh.identity(), Some("default"));
    }

    #[tokio::test]
    async fn removed_storage_keeps_archive_lists_and_refuses_file_operations() {
        let dir = tempfile::tempdir().unwrap();
        let manager = manager(dir.path()).await;
        let mut config = manager.archive_storage.configuration().await.unwrap();
        config.backends.insert(
            "archive".into(),
            crate::config::RecordingBackendConfig {
                kind: "local".into(),
                root: dir.path().join("archive").to_string_lossy().into_owned(),
            },
        );
        manager
            .archive_storage
            .replace_configuration(config.clone())
            .await
            .unwrap();
        let original = archive(&manager, PlatformType::BiliBili, "old", "archive").await;
        assert!(
            manager
                .get_archive("room", "old")
                .await
                .unwrap()
                .storage_available
        );
        config.backends.remove("archive");
        manager
            .archive_storage
            .replace_configuration(config)
            .await
            .unwrap();

        let record = manager.get_archive("room", "old").await.unwrap();
        assert_eq!(record.storage_backend_id, "archive");
        assert!(!record.storage_available);
        assert!(record.storage_unavailable_reason.is_some());
        let lists = [
            manager.get_archives("room", 0, 10).await.unwrap(),
            manager
                .db
                .get_archives_by_parent_id("room", &record.parent_id)
                .await
                .unwrap(),
            manager.db.get_recent_record("", 0, 10).await.unwrap(),
            manager.db.get_recent_record("room", 0, 10).await.unwrap(),
            manager
                .db
                .get_room_records("bilibili", "room", 0, 10)
                .await
                .unwrap(),
        ];
        for list in lists {
            assert_eq!(list.len(), 1);
            assert_eq!(list[0].storage_backend_id, "archive");
            assert!(!list[0].storage_available);
            let enriched = manager.archive_records(list).await;
            let json = serde_json::to_value(&enriched[0]).unwrap();
            assert_eq!(json["storage_backend_id"], "archive");
            assert_eq!(json["storage_available"], false);
        }
        assert!(manager
            .get_archive_subtitle(PlatformType::BiliBili, "room", "old")
            .await
            .is_err());
        assert!(manager
            .delete_archive(PlatformType::BiliBili, "room", "old")
            .await
            .is_err());
        assert!(manager.db.get_record("room", "old").await.is_ok());
        assert_eq!(
            manager
                .db
                .recording_storage_id("bilibili", "room", "old")
                .await
                .unwrap()
                .as_deref(),
            Some("archive")
        );
        assert_eq!(original.read("subtitle.srt").await.unwrap(), b"old");
    }

    #[tokio::test]
    async fn missing_provider_never_falls_back_to_the_default_backend() {
        let dir = tempfile::tempdir().unwrap();
        let mut manager = manager(dir.path()).await;
        manager
            .archive_storage
            .register_backend("first", RecordingStore::local(dir.path().join("first")))
            .await
            .unwrap();
        let original = archive(&manager, PlatformType::BiliBili, "old", "first").await;
        let fallback =
            RecordingStore::local(&manager.config.read().await.storage.backends["default"].root)
                .recording("bilibili", "room", "old")
                .unwrap();
        fallback
            .write("subtitle.srt", b"wrong backend")
            .await
            .unwrap();
        // Simulate restarting without the programmatic provider registration.
        manager.archive_storage = Arc::new(ArchiveStorage::new(
            manager.db.clone(),
            manager.config.clone(),
        ));
        assert!(manager
            .get_archive_subtitle(PlatformType::BiliBili, "room", "old")
            .await
            .is_err());
        assert!(manager
            .delete_archive(PlatformType::BiliBili, "room", "old")
            .await
            .is_err());
        assert!(manager.db.get_record("room", "old").await.is_ok());
        assert_eq!(original.read("subtitle.srt").await.unwrap(), b"old");
        assert_eq!(
            fallback.read("subtitle.srt").await.unwrap(),
            b"wrong backend"
        );
    }

    #[tokio::test]
    async fn failed_subtitle_generation_preserves_the_previous_subtitle() {
        let dir = tempfile::tempdir().unwrap();
        let manager = manager(dir.path()).await;
        manager
            .archive_storage
            .register_backend("first", RecordingStore::local(dir.path().join("first")))
            .await
            .unwrap();
        let storage = archive(&manager, PlatformType::BiliBili, "old", "first").await;
        storage
            .write(
                "playlist.m3u8",
                b"#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXT-X-ENDLIST\n",
            )
            .await
            .unwrap();
        assert!(matches!(
            manager
                .generate_archive_subtitle(PlatformType::BiliBili, "room", "old")
                .await,
            Err(RecorderManagerError::SubtitleGenerationFailed { .. })
        ));
        assert_eq!(storage.read("subtitle.srt").await.unwrap(), b"old");
        assert!(!storage.exists("tmp.mp4").await.unwrap());
        assert!(!storage.exists("tmp.m3u8").await.unwrap());
    }

    #[tokio::test]
    async fn failed_storage_access_keeps_the_database_index_and_is_not_missing_data() {
        let dir = tempfile::tempdir().unwrap();
        let manager = manager(dir.path()).await;
        let root = dir.path().join("backend");
        manager
            .archive_storage
            .register_backend("first", RecordingStore::local(&root))
            .await
            .unwrap();
        archive(&manager, PlatformType::BiliBili, "old", "first").await;
        // A non-directory ancestor deterministically fails on every platform,
        // unlike permission-based tests run by an elevated CI account.
        std::fs::rename(&root, dir.path().join("saved")).unwrap();
        std::fs::write(&root, b"unavailable backend root").unwrap();
        assert!(manager
            .delete_archive(PlatformType::BiliBili, "room", "old")
            .await
            .is_err());
        assert!(manager.db.get_record("room", "old").await.is_ok());
        assert!(manager
            .load_danmus(PlatformType::BiliBili, "room", "old")
            .await
            .is_err());
        assert!(matches!(
            manager
                .get_archive_subtitle(PlatformType::BiliBili, "room", "old")
                .await,
            Err(RecorderManagerError::IoError(_))
        ));
    }

    #[tokio::test]
    async fn unloaded_catalog_room_removal_deletes_only_that_platform() {
        let dir = tempfile::tempdir().unwrap();
        let manager = manager(dir.path()).await;
        manager
            .archive_storage
            .register_backend("first", RecordingStore::local(dir.path().join("first")))
            .await
            .unwrap();
        // The room is in the catalog and on disk, but startup never constructed
        // it (its account is missing). The list still shows it.
        manager
            .db
            .add_recorder_with_storage(PlatformType::BiliBili, "room", "", Some("first"))
            .await
            .unwrap();
        manager
            .db
            .add_recorder(PlatformType::Douyu, "room", "kept")
            .await
            .unwrap();
        let removed = archive(&manager, PlatformType::BiliBili, "old", "first").await;
        let kept = archive(&manager, PlatformType::Douyu, "other", "first").await;
        assert!(manager.recorders.read().await.is_empty());
        assert!(matches!(
            manager
                .remove_recorder(PlatformType::BiliBili, "missing")
                .await,
            Err(RecorderManagerError::NotFound { .. })
        ));

        manager
            .remove_recorder(PlatformType::BiliBili, "room")
            .await
            .unwrap();

        assert!(manager.db.get_record("room", "old").await.is_err());
        assert!(!removed.exists("segment.ts").await.unwrap());
        assert_eq!(kept.read("segment.ts").await.unwrap(), b"other");
        assert!(manager.db.get_record("room", "other").await.is_ok());
        let remaining = manager.db.get_recorders().await.unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].platform, "douyu");
        assert_eq!(remaining[0].extra, "kept");
        assert!(manager.recorders.read().await.is_empty());
    }
}
