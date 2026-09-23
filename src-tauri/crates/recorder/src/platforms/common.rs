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
use crate::core::hls_recorder::{construct_stream_from_variant, HlsRecorder};
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
    },
    Flv {
        url: String,
        /// The viewer identity used while probing this URL, so ffmpeg speaks
        /// the same HTTP as the probe that validated it.
        user_agent: Option<String>,
        /// Extra HTTP headers for the pull request (e.g. `Referer`).
        http_headers: Vec<(String, String)>,
    },
}

impl StreamPull {
    /// Build an HLS pull from a plain m3u8 URL.
    pub(crate) async fn hls(
        live_id: &str,
        url: &str,
        cookies: Option<String>,
    ) -> Result<Self, RecorderError> {
        let stream = construct_stream_from_variant(live_id, url, Format::TS, Codec::Avc)
            .await
            .map_err(|_| RecorderError::NoStreamAvailable)?;
        Ok(Self::Hls {
            stream: Arc::new(stream),
            cookies,
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

    // ----- shared lifecycle -------------------------------------------------

    /// One poll of the room status: store metadata, emit live start/end
    /// events, and report whether a recording attempt should be made.
    async fn check_live(&self) -> bool {
        let platform = self.platform().as_str();
        let room_id = self.room_id();
        let pre_live_status = self.room_info().read().await.status;

        match self.poll_room().await {
            Ok(poll) => {
                *self.room_info().write().await = RoomInfo {
                    platform: platform.to_string(),
                    room_id: room_id.clone(),
                    room_title: poll.room_title.clone(),
                    room_cover: poll.room_cover,
                    status: poll.live,
                };
                if let Some(user) = poll.user {
                    *self.user_info().write().await = user;
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

                if pre_live_status != poll.live {
                    log::info!(
                        "[{platform}][{room_id}] Live status changed to {}, enabled: {}",
                        poll.live,
                        self.enabled().load(Ordering::Relaxed)
                    );

                    if poll.live {
                        self.on_live_start().await;
                        let _ = self.event_channel().send(RecorderEvent::LiveStart {
                            recorder: self.info().await,
                        });
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

    /// Emit `LiveEnd` and clear the session state. The event owns a snapshot,
    /// so the session can be cleared after sending it.
    async fn end_live(&self) {
        let _ = self.event_channel().send(RecorderEvent::LiveEnd {
            platform: self.platform(),
            room_id: self.room_id(),
            recorder: self.info().await,
        });
        self.reset_live().await;
    }

    /// The live id for a recording attempt: resumed after a stream expiry,
    /// otherwise a fresh timestamp.
    async fn next_live_id(&self) -> String {
        let previous = self.pre_live_id().read().await.clone();
        if let Some(previous) = previous {
            if self.should_continue().load(Ordering::Relaxed) {
                self.should_continue().store(false, Ordering::Relaxed);
                return previous;
            }
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

        // Send record start event
        let _ = self.event_channel().send(RecorderEvent::RecordStart {
            recorder: self.info().await,
        });

        self.is_recording().store(true, Ordering::Relaxed);

        match pull {
            StreamPull::Hls { stream, cookies } => {
                let hls_recorder = HlsRecorder::new(
                    self.room_id(),
                    stream,
                    self.client().clone(),
                    cookies,
                    self.event_channel().clone(),
                    work_dir.full_path(),
                    self.enabled().clone(),
                )
                .await?;

                hls_recorder.start().await
            }
            StreamPull::Flv {
                url,
                user_agent,
                http_headers,
            } => {
                let flv_recorder = FlvRecorder::new(
                    url,
                    user_agent,
                    http_headers,
                    work_dir.full_path(),
                    self.enabled().clone(),
                    self.event_channel().clone(),
                    live_id.to_string(),
                );

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
                                    let _ = self.event_channel().send(received);
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
                                    let _ = self.event_channel().send(received);
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
                    if recorder.should_record().await {
                        let live_id = recorder.next_live_id().await;
                        if let Err(error) = recorder.start_recording(&live_id).await {
                            match error {
                                RecorderError::StreamExpired { expire } => {
                                    // Resume the same recording with a fresh stream.
                                    recorder.should_continue().store(true, Ordering::Relaxed);
                                    log::info!(
                                        "[{platform}][{room_id}] Stream expired at {expire}"
                                    );
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
                            let _ = recorder.event_channel().send(RecorderEvent::RecordEnd {
                                recorder: recorder.info().await,
                            });
                        }
                    }

                    recorder.reset_recording().await;

                    // An expired stream resumes immediately with a fresh one.
                    if recorder.should_continue().load(Ordering::Relaxed) {
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
