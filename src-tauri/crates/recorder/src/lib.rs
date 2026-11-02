pub mod account;
pub mod core;
pub mod danmu;
pub mod entry;
pub mod errors;
pub mod events;
pub mod platforms;
pub mod timeline;
pub mod traits;
pub mod utils;
use crate::danmu::DanmuStorage;
use crate::events::RecorderEvent;
use crate::{account::Account, platforms::PlatformType};

use chrono::Utc;
use std::{
    collections::VecDeque,
    fmt::Display,
    path::PathBuf,
    sync::{atomic, Arc},
};
use tokio::{
    sync::{Mutex, RwLock},
    task::JoinHandle,
};

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug)]
pub struct RecorderInfo {
    pub room_info: RoomInfo,
    pub user_info: UserInfo,
    pub platform_live_id: String,
    pub live_id: String,
    pub recording: bool,
    pub enabled: bool,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug)]
pub struct RoomCheckSample {
    pub checked_at: i64,
    pub duration_ms: u64,
    pub error: Option<String>,
}

/// Poll results are independent of cached metadata, including when a check fails.
#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, Default)]
pub struct RoomCheckStatus {
    /// Unix timestamps in milliseconds.
    pub checked_at: Option<i64>,
    pub last_success_at: Option<i64>,
    pub duration_ms: Option<u64>,
    pub error: Option<String>,
    pub checking: bool,
    /// Recent completed checks, retained even while the status dialog is closed.
    #[serde(default)]
    pub history: VecDeque<RoomCheckSample>,
}

impl RoomCheckStatus {
    pub(crate) fn finish(&mut self, started: std::time::Instant, error: Option<String>) {
        self.finish_at(
            Utc::now().timestamp_millis(),
            started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
            error,
        );
    }

    fn finish_at(&mut self, checked_at: i64, duration_ms: u64, error: Option<String>) {
        const HISTORY_WINDOW_MS: i64 = 5 * 60 * 1000;
        const HISTORY_LIMIT: usize = 300;

        self.checked_at = Some(checked_at);
        self.duration_ms = Some(duration_ms);
        self.checking = false;
        if error.is_none() {
            self.last_success_at = self.checked_at;
        }
        self.error = error.clone();
        while self.history.len() >= HISTORY_LIMIT
            || self.history.front().is_some_and(|sample| {
                sample.checked_at < checked_at.saturating_sub(HISTORY_WINDOW_MS)
            })
        {
            self.history.pop_front();
        }
        self.history.push_back(RoomCheckSample {
            checked_at,
            duration_ms,
            error,
        });
    }
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, Default)]
pub struct RoomInfo {
    pub platform: String,
    pub room_id: String,
    pub room_title: String,
    pub room_cover: String,
    /// Whether the room is live
    pub status: bool,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, Default)]
pub struct UserInfo {
    pub user_id: String,
    pub user_name: String,
    pub user_avatar: String,
}

/// `Recorder` is the base struct for all recorders
/// It contains the basic information for a recorder
/// and the extra information for the recorder
#[derive(Clone)]
pub struct Recorder<T>
where
    T: Send + Sync,
{
    platform: PlatformType,
    room_id: String,
    /// The account for the recorder
    account: Account,
    /// The client for the recorder
    client: reqwest::Client,
    /// The event channel for the recorder
    event_channel: crate::events::RecorderEventSender,
    preview_sink: Arc<std::sync::RwLock<Option<events::PreviewEventSink>>>,
    /// The cache directory for the recorder
    cache_dir: PathBuf,
    /// Whether the recorder is quitting
    quit: Arc<atomic::AtomicBool>,
    /// Whether the recorder is enabled
    enabled: Arc<atomic::AtomicBool>,
    /// Whether the recorder is recording
    is_recording: Arc<atomic::AtomicBool>,
    /// The room info for the recorder
    room_info: Arc<RwLock<RoomInfo>>,
    /// The user info for the recorder
    user_info: Arc<RwLock<UserInfo>>,
    /// Result of the latest room metadata check, including failures.
    check_status: Arc<RwLock<RoomCheckStatus>>,
    /// The update interval for room status
    update_interval: Arc<atomic::AtomicU64>,

    /// The platform live id for the current recording
    platform_live_id: Arc<RwLock<String>>,
    /// The live id for the current recording, generally is the timestamp of the recording start time
    live_id: Arc<RwLock<String>>,
    /// The danmu task for the current recording
    danmu_task: Arc<Mutex<Option<JoinHandle<()>>>>,
    /// The record task for the current recording
    record_task: Arc<Mutex<Option<JoinHandle<()>>>>,
    /// The danmu storage for the current recording
    danmu_storage: Arc<RwLock<Option<DanmuStorage>>>,
    /// The last update time of the current recording
    last_update: Arc<atomic::AtomicI64>,
    /// The last sequence of the current recording
    last_sequence: Arc<atomic::AtomicU64>,
    /// The live id of the previous recording attempt, kept so a stream expiry
    /// can resume the same recording
    pre_live_id: Arc<RwLock<Option<String>>>,
    /// Whether the next recording attempt should reuse `pre_live_id`
    should_continue: Arc<atomic::AtomicBool>,

    /// The extra information for the recorder
    extra: T,
}

impl<T: Send + Sync> Recorder<T> {
    /// Create a recorder from the state shared by every platform, plus the
    /// platform-specific `extra` state.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn with_extra(
        platform: PlatformType,
        room_id: &str,
        account: &Account,
        cache_dir: PathBuf,
        event_channel: impl Into<crate::events::RecorderEventSender>,
        update_interval: Arc<atomic::AtomicU64>,
        enabled: bool,
        extra: T,
    ) -> Self {
        Self {
            platform,
            room_id: room_id.to_string(),
            account: account.clone(),
            client: reqwest::Client::new(),
            event_channel: event_channel.into(),
            preview_sink: Arc::new(std::sync::RwLock::new(None)),
            cache_dir,
            quit: Arc::new(atomic::AtomicBool::new(false)),
            enabled: Arc::new(atomic::AtomicBool::new(enabled)),
            is_recording: Arc::new(atomic::AtomicBool::new(false)),
            room_info: Arc::new(RwLock::new(RoomInfo::default())),
            user_info: Arc::new(RwLock::new(UserInfo::default())),
            check_status: Arc::new(RwLock::new(RoomCheckStatus::default())),
            platform_live_id: Arc::new(RwLock::new(String::new())),
            live_id: Arc::new(RwLock::new(String::new())),
            danmu_storage: Arc::new(RwLock::new(None)),
            last_update: Arc::new(atomic::AtomicI64::new(Utc::now().timestamp())),
            last_sequence: Arc::new(atomic::AtomicU64::new(0)),
            danmu_task: Arc::new(Mutex::new(None)),
            record_task: Arc::new(Mutex::new(None)),
            pre_live_id: Arc::new(RwLock::new(None)),
            should_continue: Arc::new(atomic::AtomicBool::new(false)),
            update_interval,
            extra,
        }
    }
}

impl<T: Send + Sync> traits::RecorderBasicTrait for Recorder<T> {
    fn platform(&self) -> PlatformType {
        self.platform
    }

    fn room_id(&self) -> String {
        self.room_id.clone()
    }

    fn account(&self) -> &Account {
        &self.account
    }

    fn client(&self) -> &reqwest::Client {
        &self.client
    }

    fn event_channel(&self) -> &crate::events::RecorderEventSender {
        &self.event_channel
    }

    fn set_preview_sink(&self, sink: events::PreviewEventSink) {
        *self.preview_sink.write().unwrap() = Some(sink);
    }

    fn emit_preview_event(&self, event: RecorderEvent) {
        let sink = self.preview_sink.read().unwrap().clone();
        if let Some(sink) = sink {
            sink(event);
        } else {
            // Preserve the library API for standalone consumers.
            self.event_channel.send_preview(event);
        }
    }

    fn cache_dir(&self) -> PathBuf {
        self.cache_dir.clone()
    }

    fn quit(&self) -> Arc<atomic::AtomicBool> {
        self.quit.clone()
    }

    fn enabled(&self) -> Arc<atomic::AtomicBool> {
        self.enabled.clone()
    }

    fn is_recording(&self) -> Arc<atomic::AtomicBool> {
        self.is_recording.clone()
    }

    fn room_info(&self) -> Arc<RwLock<RoomInfo>> {
        self.room_info.clone()
    }

    fn user_info(&self) -> Arc<RwLock<UserInfo>> {
        self.user_info.clone()
    }

    fn check_status(&self) -> Arc<RwLock<RoomCheckStatus>> {
        self.check_status.clone()
    }

    fn platform_live_id(&self) -> Arc<RwLock<String>> {
        self.platform_live_id.clone()
    }

    fn live_id(&self) -> Arc<RwLock<String>> {
        self.live_id.clone()
    }

    fn danmu_task(&self) -> Arc<Mutex<Option<JoinHandle<()>>>> {
        self.danmu_task.clone()
    }

    fn record_task(&self) -> Arc<Mutex<Option<JoinHandle<()>>>> {
        self.record_task.clone()
    }

    fn danmu_storage(&self) -> Arc<RwLock<Option<DanmuStorage>>> {
        self.danmu_storage.clone()
    }

    fn last_update(&self) -> &atomic::AtomicI64 {
        &self.last_update
    }

    fn last_sequence(&self) -> &atomic::AtomicU64 {
        &self.last_sequence
    }

    fn update_interval(&self) -> Arc<atomic::AtomicU64> {
        self.update_interval.clone()
    }

    fn pre_live_id(&self) -> Arc<RwLock<Option<String>>> {
        self.pre_live_id.clone()
    }

    fn should_continue(&self) -> &atomic::AtomicBool {
        &self.should_continue
    }
}

/// Cache path is relative to cache path in config
#[derive(Clone)]
pub struct CachePath {
    pub cache_path: PathBuf,
    pub platform: PlatformType,
    pub room_id: String,
    pub live_id: String,
    pub file_name: Option<String>,
}

impl CachePath {
    pub fn new(cache_path: PathBuf, platform: PlatformType, room_id: &str, live_id: &str) -> Self {
        Self {
            cache_path,
            platform,
            room_id: room_id.to_string(),
            live_id: live_id.to_string(),
            file_name: None,
        }
    }

    /// Sanitize filename and set it
    pub fn with_filename(&self, file_name: &str) -> Self {
        let sanitized_filename = sanitize_filename::sanitize(file_name);
        Self {
            file_name: Some(sanitized_filename),
            ..self.clone()
        }
    }

    /// Get relative path to cache path
    pub fn relative_path(&self) -> PathBuf {
        if let Some(file_name) = &self.file_name {
            return PathBuf::from(format!(
                "{}/{}/{}/{}",
                self.platform.as_str(),
                self.room_id,
                self.live_id,
                file_name
            ));
        }

        PathBuf::from(format!(
            "{}/{}/{}",
            self.platform.as_str(),
            self.room_id,
            self.live_id
        ))
    }

    pub fn full_path(&self) -> PathBuf {
        self.cache_path.clone().join(self.relative_path())
    }
}

impl Display for CachePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.full_path().display())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::RecorderBasicTrait;

    #[test]
    fn room_check_history_keeps_failures_after_recovery() {
        let mut status = RoomCheckStatus::default();
        status.finish_at(1_000, 120, None);
        status.checking = true;
        status.finish_at(2_000, 940, Some("request failed".into()));
        assert_eq!(status.last_success_at, Some(1_000));
        assert!(!status.checking);
        status.finish_at(3_000, 178, None);

        assert_eq!(status.checked_at, Some(3_000));
        assert_eq!(status.last_success_at, Some(3_000));
        assert!(status.error.is_none());
        assert_eq!(status.history.len(), 3);
        assert_eq!(status.history[1].checked_at, 2_000);
        assert_eq!(status.history[1].duration_ms, 940);
        assert_eq!(status.history[1].error.as_deref(), Some("request failed"));
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["history"][1]["error"], "request failed");
    }

    #[test]
    fn room_check_history_expires_old_samples_and_bounds_rapid_polls() {
        let mut status = RoomCheckStatus::default();
        status.finish_at(0, 1, None);
        status.finish_at(1, 2, None);
        status.finish_at(300_001, 3, None);
        assert_eq!(status.history.len(), 2);
        assert_eq!(status.history.front().unwrap().checked_at, 1);

        for checked_at in 400_000..400_400 {
            status.finish_at(checked_at, 10, None);
        }
        assert_eq!(status.history.len(), 300);
        assert_eq!(status.history.front().unwrap().checked_at, 400_100);
        assert_eq!(status.history.back().unwrap().checked_at, 400_399);
    }

    #[test]
    fn room_check_history_accepts_legacy_status_snapshots() {
        let status: RoomCheckStatus = serde_json::from_value(serde_json::json!({
            "checked_at": 1000, "last_success_at": 1000, "duration_ms": 178,
            "error": null, "checking": false,
        }))
        .unwrap();
        assert!(status.history.is_empty());
    }

    #[tokio::test]
    async fn preview_sink_is_shared_by_recorder_clones_and_bypasses_lifecycle_queue() {
        let (sender, mut receiver) = events::recorder_event_channel(1);
        let recorder = Recorder::with_extra(
            PlatformType::BiliBili,
            "1",
            &Account::default(),
            PathBuf::from("/tmp"),
            sender,
            Arc::new(atomic::AtomicU64::new(30)),
            true,
            (),
        );
        let cloned = recorder.clone();
        let count = Arc::new(atomic::AtomicUsize::new(0));
        let observed = count.clone();
        recorder.set_preview_sink(Arc::new(move |event| {
            assert!(matches!(event, RecorderEvent::DanmuReceived { .. }));
            observed.fetch_add(1, atomic::Ordering::Relaxed);
        }));
        for _ in 0..10_000 {
            cloned.emit_preview_event(RecorderEvent::danmu_received("1".into(), 1, "hello", None));
        }
        assert_eq!(count.load(atomic::Ordering::Relaxed), 10_000);
        cloned
            .event_channel()
            .send(RecorderEvent::ProgressFinished {
                id: "task".into(),
                success: true,
                message: "done".into(),
            })
            .await
            .unwrap();
        assert!(matches!(
            receiver.recv().await,
            Some(RecorderEvent::ProgressFinished { .. })
        ));
        drop(cloned);
        drop(recorder);
        assert!(receiver.recv().await.is_none());
    }

    #[test]
    fn test_cache_path_relative_without_filename() {
        let cp = CachePath::new(
            PathBuf::from("/cache"),
            PlatformType::BiliBili,
            "12345",
            "live_001",
        );
        assert_eq!(cp.relative_path(), PathBuf::from("bilibili/12345/live_001"));
    }

    #[test]
    fn test_cache_path_relative_with_filename() {
        let cp = CachePath::new(
            PathBuf::from("/cache"),
            PlatformType::BiliBili,
            "12345",
            "live_001",
        )
        .with_filename("video.ts");
        assert_eq!(
            cp.relative_path(),
            PathBuf::from("bilibili/12345/live_001/video.ts")
        );
    }

    #[test]
    fn test_cache_path_full_path() {
        let cp = CachePath::new(
            PathBuf::from("/cache"),
            PlatformType::Douyin,
            "room1",
            "live1",
        );
        assert_eq!(cp.full_path(), PathBuf::from("/cache/douyin/room1/live1"));
    }

    #[test]
    fn test_cache_path_full_path_with_filename() {
        let cp = CachePath::new(
            PathBuf::from("/cache"),
            PlatformType::Huya,
            "room1",
            "live1",
        )
        .with_filename("seg.ts");
        assert_eq!(
            cp.full_path(),
            PathBuf::from("/cache/huya/room1/live1/seg.ts")
        );
    }

    #[test]
    fn test_cache_path_with_filename_sanitizes() {
        let cp = CachePath::new(
            PathBuf::from("/cache"),
            PlatformType::BiliBili,
            "12345",
            "live_001",
        )
        .with_filename("bad/file:name?.ts");
        let rel = cp.relative_path();
        let filename = rel.file_name().unwrap().to_str().unwrap();
        // sanitize_filename should remove invalid chars
        assert!(!filename.contains('/'));
        assert!(!filename.contains(':'));
        assert!(!filename.contains('?'));
    }

    #[test]
    fn test_cache_path_display() {
        let cp = CachePath::new(
            PathBuf::from("/cache"),
            PlatformType::TikTok,
            "room",
            "live",
        );
        let display = format!("{}", cp);
        assert!(display.contains("tiktok"));
        assert!(display.contains("room"));
        assert!(display.contains("live"));
    }

    #[test]
    fn test_room_info_default() {
        let ri = RoomInfo::default();
        assert_eq!(ri.platform, "");
        assert_eq!(ri.room_id, "");
        assert!(!ri.status);
    }

    #[test]
    fn test_user_info_default() {
        let ui = UserInfo::default();
        assert_eq!(ui.user_id, "");
        assert_eq!(ui.user_name, "");
        assert_eq!(ui.user_avatar, "");
    }
}
