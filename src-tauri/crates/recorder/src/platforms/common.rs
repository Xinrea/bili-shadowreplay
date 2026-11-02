//! Shared recording loop for the platform recorders.
//!
//! Every platform recorder is a [`Recorder`] plus a [`PlatformApi`] impl:
//! status transitions, event emission, the danmu lifecycle and the recording
//! session setup live here once, and each platform only describes how to talk
//! to its API and how to pull its stream.

use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use danmu_stream::danmu_stream::DanmuStream;
use danmu_stream::provider::ProviderType;
use danmu_stream::{DanmuMessageType, LiveEvent};

use crate::core::flv_recorder::FlvRecorder;
use crate::core::hls_recorder::{construct_stream_from_variant, HlsRecorder, HlsVariantSelection};
use crate::core::{Codec, Format, HlsStream};
use crate::danmu::DanmuStorage;
use crate::errors::RecorderError;
use crate::events::RecorderEvent;
use crate::traits::RecorderTrait;
use crate::{CachePath, RoomInfo, UserInfo};

/// Room metadata from one platform poll.
pub struct RoomPoll {
    /// Whether the room is currently live.
    pub live: bool,
    pub room_title: String,
    pub room_cover: String,
    /// `Some` overwrites the cached user info.
    pub user: Option<UserInfo>,
    /// Refreshes `platform_live_id` while live, e.g. the platform-side live id.
    pub platform_live_id: Option<String>,
}

/// How a platform's danmu task is spawned and torn down.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DanmuSpawn {
    /// Spawned for every recording attempt, aborted when it ends.
    PerRecording,
    /// Spawned once when the loop starts, kept alive across recordings.
    LongLived,
}

/// The platform's danmu subscription.
#[derive(Clone, Copy)]
pub struct DanmuConfig {
    pub provider: ProviderType,
    pub spawn: DanmuSpawn,
}

/// How a recording attempt pulls the live stream into its work directory.
pub enum StreamPull {
    Hls {
        stream: Arc<HlsStream>,
        cookies: Option<String>,
        variant_selection: HlsVariantSelection,
    },
    Flv {
        url: String,
        /// The viewer identity used when pulling this signed URL.
        user_agent: Option<String>,
        /// Extra HTTP headers for the pull request (e.g. `Referer`).
        http_headers: Vec<(String, String)>,
        /// Optional idle network read limit for HTTP-FLV inputs.
        read_timeout: Option<Duration>,
    },
}

impl StreamPull {
    /// Build an HLS pull from a plain m3u8 URL.
    pub(crate) async fn hls(
        live_id: &str,
        url: &str,
        cookies: Option<String>,
    ) -> Result<Self, RecorderError> {
        Self::hls_with_selection_and_expire(live_id, url, cookies, HlsVariantSelection::First, 0)
            .await
    }

    pub(crate) async fn hls_highest_bandwidth(
        live_id: &str,
        url: &str,
        cookies: Option<String>,
    ) -> Result<Self, RecorderError> {
        Self::hls_with_selection_and_expire(
            live_id,
            url,
            cookies,
            HlsVariantSelection::HighestBandwidth,
            0,
        )
        .await
    }

    /// Build an HLS pull with an explicit token expiry when a platform does
    /// not encode expiry in the playlist URL.
    pub(crate) async fn hls_with_expire(
        live_id: &str,
        url: &str,
        cookies: Option<String>,
        expire: i64,
    ) -> Result<Self, RecorderError> {
        Self::hls_with_selection_and_expire(
            live_id,
            url,
            cookies,
            HlsVariantSelection::First,
            expire,
        )
        .await
    }

    async fn hls_with_selection_and_expire(
        live_id: &str,
        url: &str,
        cookies: Option<String>,
        variant_selection: HlsVariantSelection,
        expire: i64,
    ) -> Result<Self, RecorderError> {
        let stream = construct_stream_from_variant(live_id, url, Format::TS, Codec::Avc)
            .await
            .map_err(|_| RecorderError::NoStreamAvailable)?
            .with_expire(expire);
        Ok(Self::Hls {
            stream: Arc::new(stream),
            cookies,
            variant_selection,
        })
    }
}

/// Platform-specific glue for the shared recording loop.
///
/// The loop owns status transitions, event emission, the danmu lifecycle and
/// the session setup; implementations describe how to reach the platform API
/// and how to pull its stream. All state lives in the shared [`Recorder`]
/// base, so implementations only keep their API responses in `extra`.
#[async_trait]
pub trait PlatformApi: RecorderTrait + Clone + Send + Sync + 'static {
    // ----- platform surface -------------------------------------------------

    /// Poll the platform for the room's live status and metadata.
    async fn poll_room(&self) -> Result<RoomPoll, RecorderError>;

    /// Refresh the cached stream for an ongoing live.
    ///
    /// Called only while live and recording is enabled. Returning `false`
    /// skips the recording attempt for this round.
    async fn poll_stream(&self) -> bool;

    /// Build the pull for the cached stream, or fail with
    /// [`RecorderError::NoStreamAvailable`] when there is none.
    async fn open_pull(&self, live_id: &str) -> Result<StreamPull, RecorderError>;

    /// Clear the cached stream (per-platform `extra` state).
    async fn clear_stream(&self);

    /// Prepare the cover image in `work_dir`.
    ///
    /// Returning an error aborts the recording attempt, which only makes
    /// sense when the cover is required; the default is deliberately
    /// best-effort — download the current room cover and swallow failures —
    /// so overrides that treat the cover as mandatory return their errors
    /// themselves.
    async fn prepare_cover(&self, work_dir: &CachePath) -> Result<(), RecorderError> {
        let room_info = self.room_info().read().await.clone();
        let cover_path = work_dir.with_filename("cover.jpg");
        let _ = download_file(
            self.client(),
            &room_info.room_cover,
            &cover_path.full_path(),
        )
        .await;
        Ok(())
    }

    /// The platform's danmu subscription, or `None` when it has none.
    fn danmu_config(&self) -> Option<DanmuConfig> {
        None
    }

    /// Room identifier used to subscribe to danmu. Default: the recorder's
    /// room id.
    async fn danmu_room_id(&self) -> String {
        self.room_id()
    }

    /// Run once right after a live starts, before `LiveStart` is emitted.
    async fn on_live_start(&self) {}

    /// Whether a poll error should back off to "not live" instead of keeping
    /// the previous status. Default: never.
    fn is_throttled(&self, _error: &RecorderError) -> bool {
        false
    }

    /// Whether an HLS source timeout should retry the same recording session.
    /// Most platforms treat a stalled playlist as the end of this recording;
    /// platforms with transient ad/transcode gaps can opt into resuming it.
    fn resume_on_update_timeout(&self) -> bool {
        false
    }

    /// Whether a resumed recording still belongs to the same platform live.
    /// Platforms without their own live-session ID can keep the default.
    async fn should_resume_same_recording(&self) -> bool {
        true
    }

    /// Whether a resolution change should start a new recording segment
    /// without the normal between-recordings delay.
    fn retry_immediately_after_resolution_change(&self) -> bool {
        false
    }

    // ----- shared lifecycle -------------------------------------------------

    /// One poll of the room status: store metadata, emit live start/end
    /// events, and report whether a recording attempt should be made.
    async fn check_live(&self) -> bool {
        let platform = self.platform().as_str();
        let room_id = self.room_id();
        let previous_room = self.room_info().read().await.clone();
        let pre_live_status = previous_room.status;
        let check_started = std::time::Instant::now();
        self.check_status().write().await.checking = true;

        match self.poll_room().await {
            Ok(poll) => {
                let mut metadata_changed = previous_room.room_title != poll.room_title
                    || previous_room.room_cover != poll.room_cover;
                *self.room_info().write().await = RoomInfo {
                    platform: platform.to_string(),
                    room_id: room_id.clone(),
                    room_title: poll.room_title.clone(),
                    room_cover: poll.room_cover,
                    status: poll.live,
                };
                if let Some(user) = poll.user {
                    let user_info = self.user_info();
                    let mut current = user_info.write().await;
                    metadata_changed |= current.user_id != user.user_id
                        || current.user_name != user.user_name
                        || current.user_avatar != user.user_avatar;
                    *current = user;
                }
                if poll.live && !pre_live_status {
                    // A new live session: drop the previous one's state.
                    self.reset_live().await;
                }
                if poll.live {
                    if let Some(platform_live_id) = poll.platform_live_id {
                        *self.platform_live_id().write().await = platform_live_id;
                    }
                }
                self.check_status()
                    .write()
                    .await
                    .finish(check_started, None);
                if metadata_changed && pre_live_status == poll.live {
                    let _ = self
                        .event_channel()
                        .send(RecorderEvent::RoomMetadataUpdated)
                        .await;
                }

                if pre_live_status != poll.live {
                    log::info!(
                        "[{platform}][{room_id}] Live status changed to {}, enabled: {}",
                        poll.live,
                        self.enabled().load(Ordering::Relaxed)
                    );

                    if poll.live {
                        self.on_live_start().await;
                        let _ = self
                            .event_channel()
                            .send(RecorderEvent::LiveStart {
                                recorder: self.info().await,
                            })
                            .await;
                    } else {
                        self.end_live().await;
                    }
                }

                if !poll.live {
                    self.reset_live().await;
                    return false;
                }

                if !self.should_record().await {
                    return true;
                }

                self.poll_stream().await
            }
            Err(error) => {
                self.check_status()
                    .write()
                    .await
                    .finish(check_started, Some(error.to_string()));
                if self.is_throttled(&error) {
                    log::info!("[{platform}][{room_id}] Throttled, backing off");
                    return false;
                }
                // The poll failed: the live may have started or ended, keep
                // the previous status until the next poll.
                log::warn!("[{platform}][{room_id}] Update room status failed: {error}");
                pre_live_status
            }
        }
    }

    /// Reset the state a recording attempt owns; session state that the live
    /// may still continue (`platform_live_id`, resume info) is kept.
    async fn reset_recording(&self) {
        self.is_recording().store(false, Ordering::Relaxed);
        self.clear_stream().await;
        self.last_update()
            .store(Utc::now().timestamp(), Ordering::Relaxed);
        self.last_sequence().store(0, Ordering::Relaxed);
        if self
            .danmu_config()
            .is_some_and(|config| config.spawn == DanmuSpawn::PerRecording)
        {
            self.abort_danmu_task().await;
        }
        *self.danmu_storage().write().await = None;
        *self.live_id().write().await = String::new();
    }

    /// Clear all state of the current live session.
    async fn reset_live(&self) {
        self.reset_recording().await;
        self.platform_live_id().write().await.clear();
        *self.pre_live_id().write().await = None;
        self.should_continue().store(false, Ordering::Relaxed);
    }

    /// Clear the session before publishing `LiveEnd`, preserving its IDs in
    /// the event snapshot for archive lookup and automatic clip generation.
    async fn end_live(&self) {
        let mut ended = self.info().await;
        self.reset_live().await;
        ended.recording = false;
        let _ = self
            .event_channel()
            .send(RecorderEvent::LiveEnd {
                platform: self.platform(),
                room_id: self.room_id(),
                recorder: ended,
            })
            .await;
    }

    /// Select the app recording id for this attempt. Reuse it only if a
    /// platform confirms the same live session; a new platform live must get a
    /// fresh work directory and media-sequence counter.
    async fn next_live_id(&self) -> String {
        if self.should_continue().swap(false, Ordering::Relaxed) {
            let previous = self.pre_live_id().read().await.clone();
            if let Some(previous) = previous {
                if self.should_resume_same_recording().await {
                    return previous;
                }
            }
            *self.pre_live_id().write().await = None;
            log::info!(
                "[{}][{}] Platform live changed; starting a new archive",
                self.platform().as_str(),
                self.room_id()
            );
        }

        let live_id = Utc::now().timestamp_millis().to_string();
        *self.pre_live_id().write().await = Some(live_id.clone());
        live_id
    }

    /// Set up and run one recording attempt for `live_id` until it ends.
    async fn start_recording(&self, live_id: &str) -> Result<(), RecorderError> {
        let platform = self.platform().as_str();
        let room_id = self.room_id();

        // Fail before any setup when there is no stream to record.
        let pull = self.open_pull(live_id).await?;

        let work_dir = self.work_dir(live_id).await;
        log::info!("[{platform}][{room_id}] New record started: {live_id}");
        let _ = tokio::fs::create_dir_all(work_dir.full_path()).await;

        self.prepare_cover(&work_dir).await?;

        // Setup danmu store
        let danmu_path = work_dir.with_filename("events.jsonl");
        *self.danmu_storage().write().await = DanmuStorage::new(&danmu_path.full_path()).await;

        if self
            .danmu_config()
            .is_some_and(|config| config.spawn == DanmuSpawn::PerRecording)
        {
            self.spawn_danmu_task().await;
        }

        *self.live_id().write().await = live_id.to_string();
        self.is_recording().store(true, Ordering::Relaxed);

        // Send record start event
        let _ = self
            .event_channel()
            .send(RecorderEvent::RecordStart {
                recorder: self.info().await,
            })
            .await;

        match pull {
            StreamPull::Hls {
                stream,
                cookies,
                variant_selection,
            } => {
                let hls_recorder = HlsRecorder::new_with_variant_selection(
                    self.room_id(),
                    stream,
                    self.client().clone(),
                    cookies,
                    self.event_channel().clone(),
                    work_dir.full_path(),
                    self.enabled().clone(),
                    variant_selection,
                )
                .await?;

                hls_recorder.start().await
            }
            StreamPull::Flv {
                url,
                user_agent,
                http_headers,
                read_timeout,
            } => {
                let mut flv_recorder = FlvRecorder::new(
                    url,
                    user_agent,
                    http_headers,
                    work_dir.full_path(),
                    self.enabled().clone(),
                    self.event_channel().clone(),
                    live_id.to_string(),
                );
                if let Some(timeout) = read_timeout {
                    flv_recorder = flv_recorder.with_read_timeout(timeout);
                }

                flv_recorder.start().await
            }
        }
    }

    /// Spawn the danmu task for the danmu configuration of this platform.
    async fn spawn_danmu_task(&self) {
        let danmu_recorder = self.clone();
        *self.danmu_task().lock().await = Some(tokio::spawn(async move {
            let _ = danmu_recorder.run_danmu().await;
        }));
    }

    /// Abort the running danmu task, if any.
    async fn abort_danmu_task(&self) {
        if let Some(task) = self.danmu_task().lock().await.take() {
            task.abort();
            let _ = task.await;
            log::info!(
                "[{platform}][{room}] Danmu task aborted",
                platform = self.platform().as_str(),
                room = self.room_id()
            );
        }
    }

    /// Run the platform's danmu stream until it closes, emitting and
    /// persisting every received event.
    async fn run_danmu(&self) -> Result<(), RecorderError> {
        let Some(config) = self.danmu_config() else {
            return Ok(());
        };
        let room_id = self.room_id();
        let danmu_stream = DanmuStream::new(
            config.provider,
            &self.account().cookies,
            &self.danmu_room_id().await,
        )
        .await
        .map_err(RecorderError::DanmuStreamError)?;

        let mut start_fut = Box::pin(danmu_stream.start());

        loop {
            tokio::select! {
                start_res = &mut start_fut => {
                    match start_res {
                        Ok(()) => {
                            log::info!("[{platform}][{room_id}] Danmu stream finished", platform = self.platform().as_str(), room_id = room_id);
                            return Ok(());
                        }
                        Err(err) => return Err(RecorderError::DanmuStreamError(err)),
                    }
                }
                recv_res = danmu_stream.recv() => {
                    match recv_res {
                        Ok(Some(message)) => match message {
                            DanmuMessageType::Event(event) => {
                                if let Some(received) =
                                    RecorderEvent::danmu_received_from_event(room_id.clone(), &event)
                                {
                                    self.emit_preview_event(received);
                                }
                                if let Some(storage) = self.danmu_storage().write().await.as_ref() {
                                    if let Err(error) = storage.add_event(event).await {
                                        log::error!("Failed to persist live event: {error}");
                                    }
                                }
                            }
                            DanmuMessageType::DanmuMessage(danmu) => {
                                let event = LiveEvent::danmu(danmu, self.platform().as_str());
                                if let Some(received) =
                                    RecorderEvent::danmu_received_from_event(room_id.clone(), &event)
                                {
                                    self.emit_preview_event(received);
                                }
                                if let Some(storage) = self.danmu_storage().write().await.as_ref() {
                                    if let Err(error) = storage.add_event(event).await {
                                        log::error!("Failed to persist danmu event: {error}");
                                    }
                                }
                            }
                        },
                        Ok(None) => {
                            log::info!("[{platform}][{room_id}] Danmu stream closed", platform = self.platform().as_str(), room_id = room_id);
                            return Ok(());
                        }
                        Err(err) => return Err(RecorderError::DanmuStreamError(err)),
                    }
                }
            }
        }
    }

    /// The recording loop: poll the room status, record while it is live, and
    /// poll again after a short delay. The task is stored in `record_task`.
    async fn run_recording_loop(&self) {
        if self
            .danmu_config()
            .is_some_and(|config| config.spawn == DanmuSpawn::LongLived)
        {
            self.spawn_danmu_task().await;
        }

        let recorder = self.clone();
        *self.record_task().lock().await = Some(tokio::spawn(async move {
            let platform = recorder.platform().as_str();
            let room_id = recorder.room_id();
            log::info!("[{platform}][{room_id}] Start running recorder");

            while !recorder.quit().load(Ordering::Relaxed) {
                if recorder.check_live().await {
                    // Live status is ok, start recording
                    let mut restart_immediately = false;
                    let mut ended_recording = None;
                    if recorder.should_record().await {
                        let live_id = recorder.next_live_id().await;
                        if let Err(error) = recorder.start_recording(&live_id).await {
                            match &error {
                                RecorderError::StreamExpired { expire } => {
                                    // Resume the same recording with a fresh stream.
                                    recorder.should_continue().store(true, Ordering::Relaxed);
                                    log::info!(
                                        "[{platform}][{room_id}] Stream expired at {expire}"
                                    );
                                }
                                RecorderError::UpdateTimeout
                                    if recorder.resume_on_update_timeout() =>
                                {
                                    // Some platforms briefly stop advancing their HLS
                                    // playlists (e.g. during ad transitions). Refresh
                                    // the source but keep the same archive/session.
                                    recorder.should_continue().store(true, Ordering::Relaxed);
                                    log::warn!(
                                        "[{platform}][{room_id}] HLS playlist stalled; resuming the same recording"
                                    );
                                }
                                RecorderError::ResolutionChanged { .. }
                                    if recorder.retry_immediately_after_resolution_change() =>
                                {
                                    // Close this segment and begin another at
                                    // the new resolution without idle polling.
                                    restart_immediately = true;
                                }
                                _ => {
                                    log::error!(
                                        "[{platform}][{room_id}] Update entries error: {error}"
                                    );
                                }
                            }
                        }

                        // Close the session only when it actually started:
                        // `live_id` is set right before `RecordStart` and
                        // cleared again by `reset_recording`, so a failed
                        // setup emits no `RecordEnd` without its `RecordStart`.
                        if !recorder.live_id().read().await.is_empty() {
                            ended_recording = Some(recorder.info().await);
                        }
                    }

                    recorder.reset_recording().await;
                    if let Some(mut ended) = ended_recording {
                        // Keep the ended archive's ID in the event, but publish
                        // only after the queryable recorder state is reset.
                        ended.recording = false;
                        let _ = recorder
                            .event_channel()
                            .send(RecorderEvent::RecordEnd { recorder: ended })
                            .await;
                    }

                    // An expired stream resumes immediately with a fresh one.
                    if recorder.should_continue().load(Ordering::Relaxed) || restart_immediately {
                        continue;
                    }
                    // Check status again after a short random delay.
                    let secs = rand::random::<u64>() % 4 + 2;
                    tokio::time::sleep(Duration::from_secs(secs)).await;
                    continue;
                }

                let interval = recorder.update_interval().load(Ordering::Relaxed);
                tokio::time::sleep(status_poll_delay(interval)).await;
            }
            log::info!("[{platform}][{room_id}] Recording thread quit.");
        }));
    }
}

/// Delay before the next poll while the room is not live.
///
/// Room polling hits each platform's public API, so keep a floor even for
/// small configured intervals to stay under rate limits, and jitter larger
/// intervals so the polling cadence is not exactly predictable.
pub(crate) fn status_poll_delay(interval_secs: u64) -> Duration {
    if interval_secs <= 10 {
        Duration::from_secs(rand::random::<u64>() % 11 + 10)
    } else {
        Duration::from_secs(interval_secs + rand::random::<u64>() % 5)
    }
}

/// Download `url` to `path`. An empty url is a no-op, matching the platform
/// APIs whose cover fields are optional.
pub(crate) async fn download_file(
    client: &reqwest::Client,
    url: &str,
    path: &Path,
) -> Result<(), RecorderError> {
    if url.is_empty() {
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent).map_err(RecorderError::IoError)?;
        }
    }

    let response = client.get(url).send().await?;
    let bytes = response.bytes().await?;
    let mut file = tokio::fs::File::create(path).await?;
    tokio::io::AsyncWriteExt::write_all(&mut file, &bytes).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        account::Account, events::recorder_event_channel, platforms::PlatformType, Recorder,
    };
    use std::{path::PathBuf, sync::atomic::AtomicU64};
    use tokio::sync::RwLock;

    #[derive(Clone)]
    struct Metadata(
        Arc<RwLock<(String, String, Option<UserInfo>)>>,
        Arc<std::sync::atomic::AtomicBool>,
    );

    #[async_trait]
    impl RecorderTrait for Recorder<Metadata> {
        async fn run(&self) {}
    }

    #[async_trait]
    impl PlatformApi for Recorder<Metadata> {
        async fn poll_room(&self) -> Result<RoomPoll, RecorderError> {
            if self.extra.1.load(Ordering::Relaxed) {
                return Err(RecorderError::NoStreamAvailable);
            }
            let (title, cover, user) = self.extra.0.read().await.clone();
            Ok(RoomPoll {
                live: true,
                room_title: title,
                room_cover: cover,
                user,
                platform_live_id: None,
            })
        }
        async fn poll_stream(&self) -> bool {
            false
        }
        async fn open_pull(&self, _: &str) -> Result<StreamPull, RecorderError> {
            Err(RecorderError::NoStreamAvailable)
        }
        async fn clear_stream(&self) {}
    }

    #[tokio::test]
    async fn metadata_changes_notify_after_state_is_queryable_without_live_transition() {
        let (tx, mut rx) = recorder_event_channel(4);
        let metadata = Metadata(
            Arc::new(RwLock::new(("title".into(), "cover".into(), None))),
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
        );
        let recorder = Recorder::with_extra(
            PlatformType::Twitch,
            "room",
            &Account::default(),
            PathBuf::new(),
            tx,
            Arc::new(AtomicU64::new(10)),
            false,
            metadata.clone(),
        );
        recorder.room_info.write().await.status = true;
        recorder.check_live().await;
        assert!(matches!(
            rx.recv().await,
            Some(RecorderEvent::RoomMetadataUpdated)
        ));
        recorder.check_live().await;
        assert!(tokio::time::timeout(Duration::from_millis(20), rx.recv())
            .await
            .is_err());
        *metadata.0.write().await = ("new title".into(), "new cover".into(), None);
        recorder.check_live().await;
        assert!(matches!(
            rx.recv().await,
            Some(RecorderEvent::RoomMetadataUpdated)
        ));
        let info = recorder.info().await;
        assert!(info.room_info.status);
        assert_eq!(info.room_info.room_title, "new title");
        assert_eq!(info.room_info.room_cover, "new cover");
        metadata.0.write().await.2 = Some(UserInfo {
            user_id: "user".into(),
            user_name: "new name".into(),
            user_avatar: "new avatar".into(),
        });
        recorder.check_live().await;
        assert!(matches!(
            rx.recv().await,
            Some(RecorderEvent::RoomMetadataUpdated)
        ));
        let info = recorder.info().await;
        assert_eq!(info.user_info.user_name, "new name");
        assert_eq!(info.user_info.user_avatar, "new avatar");
        recorder.check_live().await;
        assert!(tokio::time::timeout(Duration::from_millis(20), rx.recv())
            .await
            .is_err());
        metadata.0.write().await.2 = None;
        recorder.check_live().await;
        assert!(tokio::time::timeout(Duration::from_millis(20), rx.recv())
            .await
            .is_err());
        assert_eq!(recorder.info().await.user_info.user_name, "new name");
    }

    #[tokio::test]
    async fn room_checks_report_failures_keep_cached_metadata_and_recover() {
        let (tx, _rx) = recorder_event_channel(4);
        let metadata = Metadata(
            Arc::new(RwLock::new(("latest title".into(), "cover".into(), None))),
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
        );
        let recorder = Recorder::with_extra(
            PlatformType::Twitch,
            "room",
            &Account::default(),
            PathBuf::new(),
            tx,
            Arc::new(AtomicU64::new(10)),
            false,
            metadata.clone(),
        );
        assert!(recorder.check_status.read().await.checked_at.is_none());
        assert!(recorder.check_live().await);
        let success = recorder.check_status.read().await.clone();
        assert!(success.checked_at.is_some());
        assert_eq!(success.last_success_at, success.checked_at);
        assert!(success.duration_ms.is_some());
        assert!(!success.checking);
        assert!(success.error.is_none());
        assert_eq!(success.history.len(), 1);

        metadata.1.store(true, Ordering::Relaxed);
        metadata.0.write().await.0 = "unavailable title".into();
        assert!(recorder.check_live().await);
        let failed = recorder.check_status.read().await.clone();
        assert!(failed.error.is_some());
        assert!(!failed.checking);
        assert_eq!(failed.last_success_at, success.last_success_at);
        assert_eq!(failed.history.len(), 2);
        assert!(failed.history.back().unwrap().error.is_some());
        assert_eq!(recorder.info().await.room_info.room_title, "latest title");

        metadata.1.store(false, Ordering::Relaxed);
        assert!(recorder.check_live().await);
        let recovered = recorder.check_status.read().await.clone();
        assert!(recovered.error.is_none());
        assert_eq!(recovered.last_success_at, recovered.checked_at);
        assert_eq!(recovered.history.len(), 3);
        assert!(recovered.history[1].error.is_some());
        assert!(recovered.history.back().unwrap().error.is_none());
        assert_eq!(
            recorder.info().await.room_info.room_title,
            "unavailable title"
        );
    }
}
