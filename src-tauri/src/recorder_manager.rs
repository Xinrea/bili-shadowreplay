use crate::config::Config;
use crate::database::record::RecordRow;
use crate::database::recorder::RecorderRow;
use crate::database::video::VideoRow;
use crate::database::{Database, DatabaseError};
use crate::events::{DataChange, EventBus};
use crate::ffmpeg::{clip_timeline_anchors, transcode, Range};
use crate::progress::progress_reporter::{ProgressReporter, ProgressReporterTrait};
use crate::subtitle_generator::{item_to_srt, SubtitleGeneratorType};
use crate::task::{Task, TaskManager, TaskPriority};
use crate::webhook::events::{self, Payload};
use crate::webhook::poster::WebhookPoster;
use chrono::DateTime;
use m3u8_rs::{MediaPlaylist, MediaPlaylistType};
use recorder::account::Account;
use recorder::danmu::{DanmuEntry, DanmuStorage};
use recorder::errors::RecorderError;
use recorder::events::RecorderEvent;
use recorder::events::{recorder_event_channel, RecorderEventReceiver, RecorderEventSender};
use recorder::platforms::bilibili::BiliRecorder;
use recorder::platforms::douyin::DouyinRecorder;
use recorder::platforms::douyu::DouyuRecorder;
use recorder::platforms::huya::HuyaRecorder;
use recorder::platforms::kuaishou::KuaishouRecorder;
use recorder::platforms::tiktok::TikTokRecorder;
use recorder::platforms::twitch::TwitchRecorder;
use recorder::platforms::youtube::YoutubeRecorder;
use recorder::platforms::PlatformType;
use recorder::timeline::{align_danmus_to_recording, load_skipped_ad_ranges};
use recorder::traits::RecorderTrait;
use recorder::RoomInfo;
use recorder::UserInfo;
use recorder::{CachePath, RecorderInfo};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
#[cfg(feature = "gui")]
use tauri_plugin_notification::NotificationExt;
use thiserror::Error;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
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
    pub room_id: String,
    pub live_id: String,
    pub ranges: Vec<Range>,
    /// Ignored: materials keep their danmaku in a sidecar file instead, and
    /// projects burn it on export. Still accepted from older clients.
    #[serde(default)]
    pub danmu: bool,
    pub local_offset: i64,
    /// Fix encoding after clip
    pub fix_encoding: bool,
    /// Transition effect between clips (for multiple ranges)
    pub transition: Option<String>,
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
}

pub struct RelatedPlaylist {
    pub live_id: String,
    pub title: String,
    pub path: PathBuf,
}

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
    is_migrating: Arc<AtomicBool>,
    webhook_poster: WebhookPoster,
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

/// Map an HLS request path (`platform/room_id/live_id/...`) to a file in `cache_dir`.
///
/// Every `/`-separated segment must be a single plain file name: `..`, `.`, empty
/// segments, absolute paths and Windows prefixes or `\` separators are rejected,
/// so the result can never escape `cache_dir`.
fn resolve_hls_path(cache_dir: &Path, path: &str) -> Option<PathBuf> {
    let path = path.replace("%7C", "|");
    let mut resolved = cache_dir.to_path_buf();
    for segment in path.split('/') {
        let mut components = Path::new(segment).components();
        match (components.next(), components.next()) {
            (Some(Component::Normal(name)), None) => resolved.push(name),
            _ => return None,
        }
    }
    Some(resolved)
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
            is_migrating: Arc::new(AtomicBool::new(false)),
            webhook_poster,
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
                        let _ = self.db.remove_record(&live_id).await;
                        // remove record folder
                        let cache_folder = Path::new(self.config.read().await.cache.as_str())
                            .join(
                                PlatformType::from_str(&recorder.room_info.platform)
                                    .unwrap_or(PlatformType::BiliBili)
                                    .as_str(),
                            )
                            .join(room_id)
                            .join(live_id);
                        let _ = tokio::fs::remove_dir_all(&cache_folder).await;
                        log::info!("Record folder removed: {cache_folder:?}");
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
        if !self.config.read().await.auto_generate.enabled {
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

        let Ok(task) = self
            .db
            .generate_task(
                "generate_whole_clip",
                "",
                &serde_json::json!({
                    "platform": platform.as_str(),
                    "room_id": room_id,
                    "parent_id": parent_id,
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
        let room_id = room_id.to_string();
        let enqueue_result = self
            .task_manager
            .add_task(Task::new(
                task_id.clone(),
                TaskPriority::Normal,
                async move {
                    if let Err(e) = self_clone
                        .generate_whole_clip(
                            Some(&reporter),
                            GenerateWholeClipParams {
                                encode_danmu: self_clone
                                    .config
                                    .read()
                                    .await
                                    .auto_generate
                                    .encode_danmu,
                                platform: platform.as_str().to_string(),
                                room_id,
                                parent_id,
                                selected_live_ids: None,
                                output_name: None,
                            },
                        )
                        .await
                    {
                        log::error!("Failed to generate whole clip: {e}");
                        let _ = reporter
                            .finish(false, &format!("Failed to generate whole clip: {e}"))
                            .await;
                        let _ = self_clone
                            .db
                            .update_task(
                                &task_id,
                                "failed",
                                &format!("Failed to generate whole clip: {e}"),
                                None,
                            )
                            .await;
                        return Err(format!("Failed to generate whole clip: {e}"));
                    }

                    let _ = reporter
                        .finish(true, "Whole clip generated successfully")
                        .await;
                    let _ = self_clone
                        .db
                        .update_task(
                            &task_id,
                            "success",
                            "Whole clip generated successfully",
                            None,
                        )
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
                if self.is_migrating.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
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

    pub async fn add_recorder(
        &self,
        account: &Account,
        platform: PlatformType,
        room_id: &str,
        extra: &str,
        enabled: bool,
    ) -> Result<(), RecorderManagerError> {
        let normalized_room_id = match platform {
            PlatformType::Twitch => recorder::platforms::twitch::api::normalize_channel(room_id)
                .map_err(RecorderManagerError::RecorderError)?,
            PlatformType::Youtube => recorder::platforms::youtube::normalize_room_id(room_id)?,
            _ => room_id.to_string(),
        };
        let room_id = normalized_room_id.as_str();
        let recorder_id = format!("{}:{}", platform.as_str(), room_id);
        if self.recorders.read().await.contains_key(&recorder_id) {
            return Err(RecorderManagerError::AlreadyExisted {
                room_id: room_id.to_string(),
            });
        }

        let cache_dir = self.config.read().await.cache.clone();
        let cache_dir = PathBuf::from(&cache_dir);

        let event_tx = self.get_event_sender();
        let update_interval = self.config.read().await.update_interval.clone();
        let recorder_boxed: Box<dyn RecorderTrait + Send + Sync> = match platform {
            PlatformType::BiliBili => Box::new(BiliRecorder::new(
                room_id,
                account,
                cache_dir,
                event_tx,
                update_interval,
                enabled,
                self.db.clone(),
            )?),
            PlatformType::Douyin => Box::new(DouyinRecorder::new(
                room_id,
                extra,
                account,
                cache_dir,
                event_tx,
                update_interval,
                enabled,
            )?),
            PlatformType::Douyu => Box::new(DouyuRecorder::new(
                room_id,
                account,
                cache_dir,
                event_tx,
                update_interval,
                enabled,
            )?),
            PlatformType::Huya => Box::new(HuyaRecorder::new(
                room_id,
                account,
                cache_dir,
                event_tx,
                update_interval,
                enabled,
            )?),
            PlatformType::Kuaishou => Box::new(KuaishouRecorder::new(
                room_id,
                account,
                cache_dir,
                event_tx,
                update_interval,
                enabled,
            )?),
            PlatformType::TikTok => Box::new(TikTokRecorder::new(
                room_id,
                account,
                cache_dir,
                event_tx,
                update_interval,
                enabled,
            )?),
            PlatformType::Twitch => Box::new(TwitchRecorder::new(
                room_id,
                account,
                cache_dir,
                event_tx,
                update_interval,
                enabled,
            )?),
            PlatformType::Youtube => Box::new(YoutubeRecorder::new(
                room_id,
                account,
                cache_dir,
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
        self.recorders
            .write()
            .await
            .insert(recorder_id.clone(), recorder.clone());
        // Run without holding the map's read lock: run() awaits internal locks.
        recorder.run().await;
        self.db.notify_change(DataChange::Recorders);
        // A concurrent remove_recorder/stop_all may have removed the recorder
        // while run() was starting its tasks; stop it so the tasks don't
        // outlive the removal.
        if !self.recorders.read().await.contains_key(&recorder_id) {
            recorder.stop().await;
        }
        Ok(())
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

    /// Remove a recorder from the manager
    ///
    /// This will stop the recorder and remove it from the manager
    /// and remove the related cache folder
    pub async fn remove_recorder(
        &self,
        platform: PlatformType,
        room_id: &str,
    ) -> Result<RecorderRow, RecorderManagerError> {
        // check recorder exists
        let recorder_id = format!("{}:{}", platform.as_str(), room_id);
        if !self.recorders.read().await.contains_key(&recorder_id) {
            return Err(RecorderManagerError::NotFound {
                room_id: room_id.to_string(),
            });
        }

        // remove from db
        let recorder = self.db.remove_recorder(room_id).await?;

        // add to to_remove
        log::debug!("Add to to_remove: {recorder_id}");
        self.to_remove.write().await.insert(recorder_id.clone());

        // stop recorder
        log::debug!("Stop recorder: {recorder_id}");
        // Stop without holding the map's read lock: stop() awaits task shutdown.
        let recorder_ref = self.recorders.read().await.get(&recorder_id).cloned();
        if let Some(recorder_ref) = recorder_ref {
            recorder_ref.stop().await;
        }

        // remove recorder
        log::debug!("Remove recorder from manager: {recorder_id}");
        self.recorders.write().await.remove(&recorder_id);
        self.db.notify_change(DataChange::Recorders);

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
        room_id: &str,
        live_id: &str,
    ) -> Result<Vec<u8>, RecorderManagerError> {
        let cache_path = self.config.read().await.cache.clone();
        let cache_path = Path::new(&cache_path);
        let playlist_path = cache_path
            .join(platform.as_str())
            .join(room_id)
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
            .await?
            .read_to_end(&mut bytes)
            .await?;
        Ok(bytes)
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
                pl.playlist_type = Some(MediaPlaylistType::Vod);
            }
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
        let cache_path = self.config.read().await.cache.clone();
        let cache_path = Path::new(&cache_path);
        let mut danmus_path = cache_path
            .join(platform.as_str())
            .join(room_id)
            .join(live_id)
            .join("events.jsonl");
        if !danmus_path.exists() {
            // Keep existing archives readable after the storage format
            // migration. New recordings only create events.jsonl.
            let legacy_path = danmus_path.with_file_name("danmu.txt");
            if !legacy_path.exists() {
                return Ok(Vec::new());
            }
            danmus_path = legacy_path;
        }
        let Some(storage) = DanmuStorage::new(&danmus_path).await else {
            log::error!("Failed to load danmu storage: {danmus_path:?}");
            return Ok(Vec::new());
        };
        let danmus = storage.get_entries(0).await;
        if platform != PlatformType::Twitch {
            return Ok(danmus);
        }

        let Some(work_dir) = danmus_path.parent() else {
            return Ok(danmus);
        };
        let skipped_ad_ranges = match load_skipped_ad_ranges(work_dir).await {
            Ok(ranges) => ranges,
            Err(error) => {
                log::warn!("Failed to load skipped ad ranges for {live_id}: {error}");
                return Ok(danmus);
            }
        };
        if skipped_ad_ranges.is_empty() {
            return Ok(danmus);
        }
        let timeline_start_ms = match self
            .first_segment_timestamp(platform, room_id, live_id)
            .await
        {
            Ok(timestamp) => timestamp,
            Err(error) => {
                log::warn!("Failed to load archive timeline origin for {live_id}: {error}");
                match live_id.parse::<i64>() {
                    Ok(timestamp) => timestamp,
                    Err(_) => return Ok(danmus),
                }
            }
        };
        Ok(align_danmus_to_recording(
            danmus,
            timeline_start_ms,
            &skipped_ad_ranges,
        ))
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
    ) -> Vec<RelatedPlaylist> {
        let cache_path = self.config.read().await.cache.clone();
        let cache_path = Path::new(&cache_path);
        let archives = match self.db.get_archives_by_parent_id(room_id, parent_id).await {
            Ok(archives) => archives,
            Err(e) => {
                log::error!(
                    "[{}] Failed to get all related playlists: {} {}",
                    room_id,
                    parent_id,
                    e
                );
                return Vec::new();
            }
        };

        let archives: Vec<(String, String)> = archives
            .iter()
            .map(|a| (a.title.clone(), a.live_id.clone()))
            .collect();

        let playlists = archives
            .iter()
            .map(async |a| {
                let work_dir =
                    CachePath::new(cache_path.to_path_buf(), *platform, room_id, a.1.as_str());

                RelatedPlaylist {
                    live_id: a.1.clone(),
                    title: a.0.clone(),
                    path: work_dir.with_filename("playlist.m3u8").full_path(),
                }
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
            .join(params.room_id.clone())
            .join(params.live_id.clone())
            .join("playlist.m3u8");

        if !playlist_path.exists() {
            log::error!("Playlist file not found: {}", playlist_path.display());
            return Err(RecorderManagerError::ClipError {
                err: "Playlist file not found".to_string(),
            });
        }

        if params.ranges.is_empty() {
            let extracting = reporter.map(|reporter| reporter.stage("提取片段"));
            crate::ffmpeg::playlist::clip_from_playlist(
                extracting.as_ref(),
                &playlist_path,
                &clip_file,
                None,
            )
            .await
            .map_err(|e| RecorderManagerError::ClipError { err: e.to_string() })?;
        } else {
            crate::ffmpeg::playlist::clip_multiple_from_playlist(
                reporter,
                &playlist_path,
                &clip_file,
                &params.ranges,
                params.transition.as_deref(),
            )
            .await
            .map_err(|e| RecorderManagerError::ClipError { err: e.to_string() })?;
        }

        if params.fix_encoding {
            // transcode clip_file
            let tmp_clip_file = clip_file.with_extension("tmp.mp4");
            let fixing = reporter.map(|reporter| reporter.stage("修复编码"));
            if let Err(e) = transcode(fixing.as_ref(), &clip_file, &tmp_clip_file, false).await {
                log::error!("Failed to transcode clip file: {e}");
                return Err(RecorderManagerError::ClipError { err: e.to_string() });
            }

            // remove clip_file
            let _ = tokio::fs::remove_file(&clip_file).await;

            // rename tmp_clip_file to clip_file
            let _ = tokio::fs::rename(tmp_clip_file, &clip_file).await;
        }

        Ok(clip_file)
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
        let mut summary = RecorderList {
            count: 0,
            recorders: Vec::new(),
        };

        // initialized recorder set
        let mut recorder_set = HashSet::new();
        // Clone the Arcs out of the map so info().await doesn't hold the read lock.
        let recorders: Vec<_> = self.recorders.read().await.values().cloned().collect();
        for recorder in recorders {
            let recorder_info = recorder.info().await;
            summary.recorders.push(recorder_info.clone());
            recorder_set.insert(recorder_info.room_info.room_id);
        }

        // get recorders from db
        let recorders = match self.db.get_recorders().await {
            Ok(recorders) => recorders,
            Err(e) => {
                log::error!("Failed to get recorders from db: {e}");
                return summary;
            }
        };
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
        // read subtitle file under work_dir
        let work_dir = CachePath::new(
            self.config.read().await.cache.clone().into(),
            platform,
            room_id,
            live_id,
        );
        let subtitle_file_path = work_dir.with_filename("subtitle.srt");
        let subtitle_file = File::open(subtitle_file_path.full_path())
            .await
            .map_err(|_| RecorderManagerError::SubtitleNotFound {
                live_id: live_id.to_string(),
            })?;
        let mut subtitle_file = BufReader::new(subtitle_file);
        let mut subtitle_content = String::new();
        subtitle_file.read_to_string(&mut subtitle_content).await?;
        Ok(subtitle_content)
    }

    pub async fn generate_archive_subtitle(
        &self,
        platform: PlatformType,
        room_id: &str,
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
        playlist.write_to(&mut v)?;
        let m3u8_content = std::str::from_utf8(&v).map_err(|e| RecorderManagerError::HLSError {
            err: format!("Failed to encode playlist: {e}"),
        })?;
        tokio::fs::write(&m3u8_index_file_path.full_path(), m3u8_content).await?;
        log::info!(
            "[{}]M3U8 index file generated: {}",
            room_id,
            m3u8_index_file_path.full_path().display()
        );

        // Generate a tmp mp4 clip file first
        let clip_file_path = work_dir.with_filename("tmp.mp4");
        if let Err(e) = crate::ffmpeg::playlist::clip_from_playlist(
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

        // Copy the settings out so no config guard is held across the ffmpeg
        // and subtitle jobs below.
        let (
            generator_type,
            whisper_model,
            whisper_prompt,
            openai_api_key,
            openai_api_endpoint,
            whisper_language,
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
            )
        };

        // For third-party services (powerlive), extract opus audio from mp4
        let media_file_path = if generator_type.is_powerlive() {
            let opus_file_path = work_dir.with_filename("tmp.opus");
            log::info!("[{}]Extracting opus audio for third-party service", room_id);

            // Extract opus audio using FFmpeg
            let mut cmd = ffmpeg_utils::ffmpeg_command();
            cmd.arg("-i")
                .arg(clip_file_path.full_path())
                .args([
                    "-vn", // no video
                    "-acodec", "libopus", "-b:a", "128k", "-y",
                ])
                .arg(opus_file_path.full_path());

            let output =
                cmd.output()
                    .await
                    .map_err(|e| RecorderManagerError::SubtitleGenerationFailed {
                        error: format!("Failed to run FFmpeg: {}", e),
                    })?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(RecorderManagerError::SubtitleGenerationFailed {
                    error: format!("Failed to extract opus audio: {}", stderr),
                });
            }

            log::info!("[{}]Opus audio extracted: {}", room_id, opus_file_path);
            opus_file_path
        } else {
            // For whisper/whisper_online, use mp4 directly
            clip_file_path
        };

        // generate subtitle file
        let resource_dir = self.resource_dir.clone();
        let result = crate::ffmpeg::generate_video_subtitle(
            None,
            Path::new(&media_file_path.full_path()),
            generator_type,
            &resource_dir,
            &whisper_model,
            &whisper_prompt,
            &openai_api_key,
            &openai_api_endpoint,
            &whisper_language,
            Some(&self.db),
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
        subtitle_file.write_all(subtitle_content.as_bytes()).await?;
        log::info!("[{room_id}]Subtitle file written");
        // remove tmp files
        tokio::fs::remove_file(&m3u8_index_file_path.full_path()).await?;

        // Remove both mp4 and opus files if they exist
        let clip_file_path = work_dir.with_filename("tmp.mp4");
        let _ = tokio::fs::remove_file(&clip_file_path.full_path()).await;

        let opus_file_path = work_dir.with_filename("tmp.opus");
        let _ = tokio::fs::remove_file(&opus_file_path.full_path()).await;

        log::info!("[{room_id}]Tmp files removed");
        Ok(subtitle_content)
    }

    pub async fn delete_archive(
        &self,
        platform: PlatformType,
        room_id: &str,
        live_id: &str,
    ) -> Result<RecordRow, RecorderManagerError> {
        log::info!("Deleting archive {room_id}:{live_id}");
        let to_delete = self.db.remove_record(live_id).await?;
        let cache_folder = Path::new(self.config.read().await.cache.as_str())
            .join(platform.as_str())
            .join(room_id)
            .join(live_id);
        let _ = tokio::fs::remove_dir_all(cache_folder).await;
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
        let cache_path = self.config.read().await.cache.clone();
        let path = uri.split('?').next().unwrap_or(uri);
        let params = uri.split('?').nth(1).unwrap_or("");
        let path_segs: Vec<&str> = path.split('/').collect();

        if path_segs.len() < 4 {
            log::warn!("Invalid request path: {path}");
            return Err(RecorderManagerError::HLSError {
                err: "Invalid hls path".into(),
            });
        }
        // Resolve before branching so room_id / live_id are vetted for playlists too.
        let Some(file_path) = resolve_hls_path(Path::new(&cache_path), path) else {
            log::warn!("Rejected request path outside the cache dir: {path}");
            return Err(RecorderManagerError::HLSError {
                err: "Invalid hls path".into(),
            });
        };
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

        if remaining_path == "playlist.m3u8" || remaining_path.ends_with("/playlist.m3u8") {
            let playlist = self.load_playlist(platform, room_id, live_id).await?;
            let playlist = self.playlist_range(&playlist, range).await?;
            let mut bytes: Vec<u8> = Vec::new();
            playlist.write_to(&mut bytes)?;
            Ok(bytes)
        } else {
            // try to find requested ts file in recorder's cache
            // cache files are stored in {cache_dir}/{platform}/{room_id}/{live_id}/{ts_file}
            let ts_file_content = tokio::fs::read(&file_path).await.map_err(|e| {
                log::warn!("Failed to read segment file {}: {e}", file_path.display());
                RecorderManagerError::HLSError {
                    err: "Segment file not found".into(),
                }
            })?;

            Ok(ts_file_content)
        }
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

    pub async fn generate_whole_clip(
        &self,
        reporter: Option<&ProgressReporter>,
        params: GenerateWholeClipParams,
    ) -> Result<(), RecorderManagerError> {
        let GenerateWholeClipParams {
            encode_danmu,
            platform,
            room_id,
            parent_id,
            selected_live_ids,
            output_name,
        } = params;

        let platform = PlatformType::from_str(&platform).map_err(|_| {
            RecorderManagerError::InvalidPlatformType {
                platform: platform.to_string(),
            }
        })?;

        let mut playlists = self
            .get_related_playlists(&platform, &room_id, &parent_id)
            .await;
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
            let danmu_tracks = playlists
                .iter()
                .map(async |p| {
                    (self
                        .load_archive_danmus(platform, &room_id, &p.live_id)
                        .await)
                        .ok()
                })
                .collect::<Vec<_>>();

            futures::future::join_all(danmu_tracks).await
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

        if let Err(e) = crate::ffmpeg::playlist::concat_playlists_to_video(
            reporter,
            &playlists_refs,
            danmu_tracks,
            &danmu_options,
            &output_path,
        )
        .await
        {
            log::error!("Failed to concat playlists: {e}");
            return Err(RecorderManagerError::HLSError {
                err: "Failed to concat playlists".into(),
            });
        }

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
    use super::*;

    /// A temp dir holding `cache/` with a few segments, plus `secret.txt` just outside it.
    fn cache_with_outside_secret() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("cache");
        std::fs::create_dir_all(cache.join("bilibili/1/2/some_dir")).unwrap();
        std::fs::write(cache.join("bilibili/1/2/segment.ts"), "segment").unwrap();
        std::fs::write(cache.join("bilibili/1/2/some_dir/segment.ts"), "nested").unwrap();
        std::fs::write(dir.path().join("secret.txt"), "secret").unwrap();
        (dir, cache)
    }

    #[test]
    fn resolves_segment_paths_inside_the_cache_dir() {
        let (_dir, cache) = cache_with_outside_secret();
        for (path, content) in [
            ("bilibili/1/2/segment.ts", "segment"),
            ("bilibili/1/2/some_dir/segment.ts", "nested"),
        ] {
            let resolved = resolve_hls_path(&cache, path).unwrap();
            assert_eq!(std::fs::read_to_string(resolved).unwrap(), content);
        }
        assert_eq!(
            resolve_hls_path(&cache, "bilibili/1/2/a%7Cb.ts"),
            Some(cache.join("bilibili/1/2/a|b.ts"))
        );
    }

    #[test]
    fn rejects_paths_escaping_the_cache_dir() {
        let (dir, cache) = cache_with_outside_secret();
        let absolute = dir.path().join("secret.txt").to_string_lossy().into_owned();
        for path in [
            // What axum's `Path` extractor yields for `..%2F..%2F..%2F..%2Fsecret.txt`,
            // and what a raw `curl --path-as-is` request carries.
            "bilibili/1/2/../../../../secret.txt".to_string(),
            // Room and live ids are path segments too.
            "bilibili/../../secret.txt".to_string(),
            "bilibili/1/2/./segment.ts".to_string(),
            "bilibili/1/2//segment.ts".to_string(),
            absolute.clone(),
            format!("bilibili/1/2/{absolute}"),
        ] {
            let resolved = resolve_hls_path(&cache, &path);
            let leaked = resolved
                .as_ref()
                .and_then(|p| std::fs::read_to_string(p).ok());
            assert_eq!(
                resolved, None,
                "{path:?} must be rejected (reads {leaked:?})"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn rejects_windows_separators_and_prefixes() {
        let (_dir, cache) = cache_with_outside_secret();
        for path in [
            "bilibili\\..\\..\\secret.txt",
            "bilibili/1/2/C:",
            "bilibili/1/2/C:\\secret.txt",
        ] {
            assert_eq!(
                resolve_hls_path(&cache, path),
                None,
                "{path:?} must be rejected"
            );
        }
    }
}
