#[cfg(test)]
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
#[cfg(test)]
use tokio::sync::broadcast;

use ffmpeg_utils::{record_to_hls_objects, HlsRecordingOptions};
use m3u8_rs::MediaPlaylist;
use recording_storage::RecordingStorage;

use crate::core::playlist::HlsPlaylist;
use crate::core::storage_io::{NativeHlsStorage, NATIVE_HLS_OBJECT_BYTES};
use crate::errors::RecorderError;
use crate::events::RecorderEvent;
use crate::startup::RecordingTrace;

pub struct FlvRecorder {
    url: String,
    user_agent: Option<String>,
    http_headers: Vec<(String, String)>,
    read_timeout: Option<Duration>,
    storage: RecordingStorage,
    enabled: Arc<AtomicBool>,
    event_channel: crate::events::RecorderEventSender,
    live_id: String,
    trace: Option<RecordingTrace>,
}

impl FlvRecorder {
    pub fn new(
        url: String,
        user_agent: Option<String>,
        http_headers: Vec<(String, String)>,
        work_dir: impl Into<RecordingStorage>,
        enabled: Arc<AtomicBool>,
        event_channel: impl Into<crate::events::RecorderEventSender>,
        live_id: String,
    ) -> Self {
        Self {
            url,
            user_agent,
            http_headers,
            read_timeout: None,
            storage: work_dir.into(),
            enabled,
            event_channel: event_channel.into(),
            live_id,
            trace: None,
        }
    }

    pub fn with_read_timeout(mut self, timeout: Duration) -> Self {
        self.read_timeout = Some(timeout);
        self
    }

    pub(crate) fn with_trace(mut self, trace: RecordingTrace) -> Self {
        self.trace = Some(trace);
        self
    }

    fn input_options(&self) -> HlsRecordingOptions {
        HlsRecordingOptions {
            user_agent: self.user_agent.clone(),
            http_headers: self.http_headers.clone(),
            read_timeout: self.read_timeout,
        }
    }

    pub async fn start(&self) -> Result<(), RecorderError> {
        let trace = self
            .trace
            .clone()
            .unwrap_or_else(|| RecordingTrace::new("flv", "unknown", &self.live_id));
        trace
            .stage("flv_storage_prepare", self.storage.prepare())
            .await?;
        trace
            .stage("flv_playlist_prepare", async {
                if !self.storage.exists("playlist.m3u8").await? {
                    let playlist =
                        HlsPlaylist::from_storage(self.storage.clone(), "playlist.m3u8").await?;
                    playlist.flush().await?;
                }
                Ok::<_, RecorderError>(())
            })
            .await?;

        // Storage failure stops this native worker, not the user's persistent
        // enabled flag shared by the platform recorder and its retry loop.
        let worker_enabled = Arc::new(AtomicBool::new(self.enabled.load(Ordering::Relaxed)));
        let native_storage =
            Arc::new(NativeHlsStorage::new(self.storage.clone()).with_trace(trace));
        let recording = record_to_hls_objects(
            &self.url,
            native_storage.clone(),
            self.input_options(),
            worker_enabled.clone(),
        );
        tokio::pin!(recording);
        let mut processed_segments = 0usize;
        let (result, update_error) = {
            let updates = async {
                let mut update_error = None;
                let result = loop {
                    tokio::select! {
                        result = &mut recording => break result,
                        _ = tokio::time::sleep(Duration::from_millis(500)), if update_error.is_none() => {
                            if let Err(error) = self.publish_updates(&mut processed_segments).await {
                                // Await worker shutdown rather than abandoning open files.
                                worker_enabled.store(false, Ordering::Relaxed);
                                update_error = Some(error);
                            }
                        }
                    }
                };
                (result, update_error)
            };
            tokio::pin!(updates);
            // Observe user cancellation even while publishing or delivering an
            // update is pending. No detached monitor may outlive this worker.
            let user_disabled = async {
                while self.enabled.load(Ordering::Relaxed) {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            };
            tokio::select! {
                result = &mut updates => result,
                _ = user_disabled => {
                    worker_enabled.store(false, Ordering::Relaxed);
                    updates.await
                }
            }
        };
        let result = match update_error.or_else(|| native_storage.take_error().map(Into::into)) {
            Some(error) => Err(error),
            None => result.map_err(RecorderError::FfmpegError),
        };
        // A later storage failure can leave committed segments before the first
        // progress poll. Always account for those before RecordEnd, which uses
        // the reported size to decide whether an archive is empty.
        let updates = self.publish_updates(&mut processed_segments).await;
        // Attempt both finalization steps even if one fails, without replacing
        // the original recording error with a secondary cleanup error.
        let close = async {
            if self.storage.exists("playlist.m3u8").await? {
                let mut playlist =
                    HlsPlaylist::from_storage(self.storage.clone(), "playlist.m3u8").await?;
                playlist.close().await?;
            }
            Ok(())
        }
        .await;
        result.and(updates).and(close)
    }

    async fn publish_updates(&self, processed_segments: &mut usize) -> Result<(), RecorderError> {
        // The native close callbacks already committed every referenced segment
        // before this manifest. Poll only for progress, never upload a local copy.
        let bytes = match self
            .storage
            .read_bounded("playlist.m3u8", NATIVE_HLS_OBJECT_BYTES)
            .await
        {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        let Ok((_, playlist)) = m3u8_rs::parse_media_playlist(&bytes) else {
            return Ok(());
        };
        if let Some((duration_secs, cached_size_bytes)) =
            Self::update_from_playlist(&self.storage, &playlist, processed_segments).await?
        {
            let _ = self
                .event_channel
                .send(RecorderEvent::RecordUpdate {
                    live_id: self.live_id.clone(),
                    duration_secs,
                    cached_size_bytes,
                })
                .await;
        }
        Ok(())
    }

    async fn update_from_playlist(
        storage: &RecordingStorage,
        playlist: &MediaPlaylist,
        processed_segments: &mut usize,
    ) -> Result<Option<(f64, u64)>, RecorderError> {
        if playlist.segments.len() <= *processed_segments {
            return Ok(None);
        }

        let mut duration_delta: f64 = 0.0;
        let mut size_delta = 0u64;

        for segment in &playlist.segments[*processed_segments..] {
            duration_delta += f64::from(segment.duration);
            size_delta += storage.size(&segment.uri).await?;
        }

        *processed_segments = playlist.segments.len();
        if duration_delta <= 0.0 && size_delta == 0 {
            return Ok(None);
        }
        Ok(Some((duration_delta, size_delta)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use recording_storage::{LocalWorkspace, ObjectMetadata, StorageBackend};
    use std::{collections::HashMap, io, sync::Mutex};

    #[derive(Default)]
    struct MemoryBackend {
        objects: Mutex<HashMap<String, Vec<u8>>>,
        reject_segments: bool,
        reject_after_first_segment: bool,
        reject_close: bool,
    }

    #[async_trait::async_trait]
    impl StorageBackend for MemoryBackend {
        async fn read_range(&self, key: &str, range: std::ops::Range<u64>) -> io::Result<Vec<u8>> {
            if range.start > range.end {
                return Err(io::ErrorKind::InvalidInput.into());
            }
            let objects = self.objects.lock().unwrap();
            let bytes = objects.get(key).ok_or(io::ErrorKind::NotFound)?;
            let start = range.start.min(bytes.len() as u64) as usize;
            let end = range.end.min(bytes.len() as u64) as usize;
            Ok(bytes[start..end].to_vec())
        }

        async fn list(
            &self,
            prefix: &str,
            cursor: Option<&str>,
            limit: usize,
        ) -> io::Result<recording_storage::ObjectPage> {
            let mut keys = self
                .objects
                .lock()
                .unwrap()
                .keys()
                .filter(|key| {
                    prefix.is_empty()
                        || key.as_str() == prefix
                        || key.starts_with(&format!("{prefix}/"))
                })
                .filter(|key| cursor.is_none_or(|cursor| key.as_str() > cursor))
                .cloned()
                .collect::<Vec<_>>();
            keys.sort();
            let more = keys.len() > limit;
            keys.truncate(limit);
            let next_cursor = more.then(|| keys.last().cloned()).flatten();
            Ok(recording_storage::ObjectPage { keys, next_cursor })
        }

        async fn read(&self, key: &str) -> io::Result<Vec<u8>> {
            self.objects
                .lock()
                .unwrap()
                .get(key)
                .cloned()
                .ok_or_else(|| io::ErrorKind::NotFound.into())
        }

        async fn read_bounded(&self, key: &str, max_bytes: usize) -> io::Result<Vec<u8>> {
            let objects = self.objects.lock().unwrap();
            let bytes = objects.get(key).ok_or(io::ErrorKind::NotFound)?;
            if bytes.len() > max_bytes {
                return Err(io::ErrorKind::InvalidData.into());
            }
            Ok(bytes.clone())
        }

        async fn write(&self, key: &str, bytes: &[u8]) -> io::Result<()> {
            if self.reject_segments && key.ends_with(".ts") {
                return Err(io::ErrorKind::PermissionDenied.into());
            }
            let mut objects = self.objects.lock().unwrap();
            if self.reject_after_first_segment
                && key.ends_with(".ts")
                && objects.keys().any(|key| key.ends_with(".ts"))
            {
                return Err(io::ErrorKind::PermissionDenied.into());
            }
            if key == "playlist.m3u8" {
                let (_, playlist) = m3u8_rs::parse_media_playlist(bytes)
                    .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
                if self.reject_close && playlist.end_list {
                    return Err(io::ErrorKind::ConnectionReset.into());
                }
                // Check EVERY snapshot at publication, not only the final VOD.
                for segment in playlist.segments {
                    assert!(
                        objects.contains_key(&segment.uri),
                        "uncommitted segment reference"
                    );
                }
            }
            objects.insert(key.to_owned(), bytes.to_vec());
            Ok(())
        }

        async fn remove(&self, key: &str) -> io::Result<()> {
            self.objects.lock().unwrap().remove(key);
            Ok(())
        }

        async fn stat(&self, key: &str) -> io::Result<ObjectMetadata> {
            self.objects
                .lock()
                .unwrap()
                .get(key)
                .map(|bytes| ObjectMetadata {
                    size: bytes.len() as u64,
                })
                .ok_or_else(|| io::ErrorKind::NotFound.into())
        }
    }

    #[tokio::test]
    async fn native_recording_and_resume_use_backend_without_a_local_output_directory() {
        let directory = tempfile::tempdir().unwrap();
        let workspace = directory.path().join("must-not-be-created");
        let backend = Arc::new(MemoryBackend::default());
        let storage =
            RecordingStorage::with_backend(backend.clone(), LocalWorkspace::new(&workspace));
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../ffmpeg_utils/src/hls/fixtures/live.flv");
        let (events, mut received) = broadcast::channel(16);
        let recorder = FlvRecorder::new(
            fixture.to_string_lossy().into_owned(),
            None,
            Vec::new(),
            storage.clone(),
            Arc::new(AtomicBool::new(true)),
            events,
            "backend-live".into(),
        );
        recorder.start().await.unwrap();
        let bytes = storage.read("playlist.m3u8").await.unwrap();
        let (_, first) = m3u8_rs::parse_media_playlist(&bytes).unwrap();
        assert!(first.end_list);
        assert!(first.segments.len() >= 2);
        let mut originals = Vec::new();
        for segment in &first.segments {
            let bytes = storage.read(&segment.uri).await.unwrap();
            let metadata = ffmpeg_utils::extract_video_metadata_from_bytes(bytes.clone())
                .await
                .unwrap();
            assert!(metadata.width > 0 && metadata.height > 0);
            originals.push((segment.uri.clone(), bytes));
        }
        assert!(matches!(
            received.try_recv(),
            Ok(RecorderEvent::RecordUpdate { .. })
        ));
        assert!(!workspace.exists());

        recorder.start().await.unwrap();
        let bytes = storage.read("playlist.m3u8").await.unwrap();
        let (_, resumed) = m3u8_rs::parse_media_playlist(&bytes).unwrap();
        assert!(resumed.end_list);
        assert!(resumed.segments.len() > first.segments.len());
        assert!(resumed.segments.iter().any(|segment| segment.discontinuity));
        for (key, bytes) in originals {
            assert_eq!(storage.read(&key).await.unwrap(), bytes);
        }
        assert!(!workspace.exists());
    }

    #[tokio::test]
    async fn rejected_native_segment_never_enters_the_backend_playlist() {
        let directory = tempfile::tempdir().unwrap();
        let workspace = directory.path().join("must-not-be-created");
        let backend = Arc::new(MemoryBackend {
            reject_segments: true,
            ..MemoryBackend::default()
        });
        let storage = RecordingStorage::with_backend(backend, LocalWorkspace::new(&workspace));
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../ffmpeg_utils/src/hls/fixtures/live.flv");
        let enabled = Arc::new(AtomicBool::new(true));
        let (events, _received) = broadcast::channel(16);
        let recorder = FlvRecorder::new(
            fixture.to_string_lossy().into_owned(),
            None,
            Vec::new(),
            storage.clone(),
            enabled.clone(),
            events,
            "backend-failure".into(),
        );
        let error = recorder.start().await.unwrap_err();
        assert!(matches!(
            error,
            RecorderError::IoError(error) if error.kind() == io::ErrorKind::PermissionDenied
        ));
        let bytes = storage.read("playlist.m3u8").await.unwrap();
        let (_, playlist) = m3u8_rs::parse_media_playlist(&bytes).unwrap();
        assert!(playlist.segments.is_empty());
        assert!(!workspace.exists());
        assert!(enabled.load(Ordering::Relaxed));
    }

    #[tokio::test]
    async fn later_storage_failure_reports_committed_media_even_when_close_also_fails() {
        for reject_close in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let backend = Arc::new(MemoryBackend {
                reject_after_first_segment: true,
                reject_close,
                ..Default::default()
            });
            let storage =
                RecordingStorage::with_backend(backend, LocalWorkspace::new(directory.path()));
            let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../ffmpeg_utils/src/hls/fixtures/live.flv");
            let (events, mut received) = broadcast::channel(16);
            let recorder = FlvRecorder::new(
                fixture.to_string_lossy().into_owned(),
                None,
                Vec::new(),
                storage.clone(),
                Arc::new(AtomicBool::new(true)),
                events,
                "partial-recording".into(),
            );
            assert!(matches!(
                recorder.start().await,
                Err(RecorderError::IoError(error))
                    if error.kind() == io::ErrorKind::PermissionDenied
            ));
            let bytes = storage.read("playlist.m3u8").await.unwrap();
            let (_, playlist) = m3u8_rs::parse_media_playlist(&bytes).unwrap();
            assert_eq!(playlist.segments.len(), 1);
            assert_eq!(playlist.end_list, !reject_close);
            let expected_size = storage.size(&playlist.segments[0].uri).await.unwrap();
            let mut reported_size = 0;
            let mut reported_duration = 0.0;
            while let Ok(event) = received.try_recv() {
                if let RecorderEvent::RecordUpdate {
                    cached_size_bytes,
                    duration_secs,
                    ..
                } = event
                {
                    reported_size += cached_size_bytes;
                    reported_duration += duration_secs;
                }
            }
            assert!(expected_size > 0);
            assert_eq!(
                reported_size, expected_size,
                "report committed bytes exactly once"
            );
            assert_eq!(reported_duration, f64::from(playlist.segments[0].duration));
        }
    }

    #[tokio::test]
    async fn storage_failure_stops_only_the_worker_not_the_recorders_enabled_flag() {
        use recording_storage::{LocalBackend, LocalWorkspace, ObjectMetadata, StorageBackend};
        use std::io;

        struct ReadOnly(LocalBackend);
        #[async_trait::async_trait]
        impl StorageBackend for ReadOnly {
            async fn read_range(
                &self,
                key: &str,
                range: std::ops::Range<u64>,
            ) -> io::Result<Vec<u8>> {
                self.0.read_range(key, range).await
            }
            async fn list(
                &self,
                prefix: &str,
                cursor: Option<&str>,
                limit: usize,
            ) -> io::Result<recording_storage::ObjectPage> {
                self.0.list(prefix, cursor, limit).await
            }
            async fn read(&self, key: &str) -> io::Result<Vec<u8>> {
                self.0.read(key).await
            }
            async fn read_bounded(&self, key: &str, max_bytes: usize) -> io::Result<Vec<u8>> {
                self.0.read_bounded(key, max_bytes).await
            }
            async fn write(&self, _: &str, _: &[u8]) -> io::Result<()> {
                Err(io::ErrorKind::PermissionDenied.into())
            }
            async fn remove(&self, key: &str) -> io::Result<()> {
                self.0.remove(key).await
            }
            async fn stat(&self, key: &str) -> io::Result<ObjectMetadata> {
                self.0.stat(key).await
            }
        }

        let directory = tempfile::tempdir().unwrap();
        let backend = LocalBackend::new(directory.path().join("objects"));
        let manifest = b"#EXTM3U\n#EXT-X-TARGETDURATION:4\n";
        backend.write("playlist.m3u8", manifest).await.unwrap();
        let workspace = LocalWorkspace::new(directory.path().join("workspace"));
        let storage = RecordingStorage::with_backend(Arc::new(ReadOnly(backend)), workspace);
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../ffmpeg_utils/src/hls/fixtures/live.flv");
        let enabled = Arc::new(AtomicBool::new(true));
        let (events, _received) = broadcast::channel(16);
        let recorder = FlvRecorder::new(
            fixture.to_string_lossy().into_owned(),
            None,
            Vec::new(),
            storage,
            enabled.clone(),
            events,
            "live".into(),
        )
        .with_read_timeout(Duration::from_secs(1));
        let result = tokio::time::timeout(Duration::from_secs(10), recorder.start())
            .await
            .expect("native worker must stop after publication fails");
        assert!(matches!(
            result,
            Err(RecorderError::IoError(error)) if error.kind() == io::ErrorKind::PermissionDenied
        ));
        assert!(enabled.load(Ordering::Relaxed));
        assert!(!directory.path().join("workspace").exists());
    }

    #[tokio::test]
    async fn native_recording_closes_the_playlist_and_reports_the_complete_tail() {
        let directory = tempfile::tempdir().unwrap();
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../ffmpeg_utils/src/hls/fixtures/live.flv");
        let (events, mut received) = broadcast::channel(16);
        let recorder = FlvRecorder::new(
            fixture.to_string_lossy().into_owned(),
            None,
            Vec::new(),
            directory.path().to_owned(),
            Arc::new(AtomicBool::new(true)),
            events,
            "native-live".into(),
        );
        recorder.start().await.unwrap();
        let bytes = std::fs::read(directory.path().join("playlist.m3u8")).unwrap();
        let (_, playlist) = m3u8_rs::parse_media_playlist(&bytes).unwrap();
        assert!(playlist.end_list);
        assert!(playlist.segments.len() >= 2);
        let expected_duration: f64 = playlist
            .segments
            .iter()
            .map(|s| f64::from(s.duration))
            .sum();
        let expected_size: u64 = playlist
            .segments
            .iter()
            .map(|s| {
                std::fs::metadata(directory.path().join(&s.uri))
                    .unwrap()
                    .len()
            })
            .sum();
        let mut duration = 0.0;
        let mut size = 0;
        while let Ok(event) = received.try_recv() {
            if let RecorderEvent::RecordUpdate {
                live_id,
                duration_secs,
                cached_size_bytes,
            } = event
            {
                assert_eq!(live_id, "native-live");
                duration += duration_secs;
                size += cached_size_bytes;
            }
        }
        assert!((duration - expected_duration).abs() < 0.0001);
        assert_eq!(size, expected_size);
        assert!((duration - 6.0).abs() < 0.2);
        assert!(size > 0);
    }

    #[tokio::test]
    async fn disabled_recording_does_not_open_the_url_and_closes_its_empty_playlist() {
        let directory = tempfile::tempdir().unwrap();
        let (events, _) = broadcast::channel(1);
        let recorder = FlvRecorder::new(
            "http://127.0.0.1:1/must-not-open".into(),
            Some("viewer".into()),
            vec![("Referer".into(), "https://www.douyu.com/".into())],
            directory.path().to_owned(),
            Arc::new(AtomicBool::new(false)),
            events,
            "disabled".into(),
        )
        .with_read_timeout(Duration::from_millis(50));
        tokio::time::timeout(Duration::from_secs(2), recorder.start())
            .await
            .unwrap()
            .unwrap();
        let bytes = std::fs::read(directory.path().join("playlist.m3u8")).unwrap();
        let (_, playlist) = m3u8_rs::parse_media_playlist(&bytes).unwrap();
        assert!(playlist.end_list);
        assert!(playlist.segments.is_empty());
    }
}
