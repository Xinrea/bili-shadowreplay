use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;
#[cfg(test)]
use tokio::sync::broadcast;

use ffmpeg_utils::{record_to_hls, HlsRecordingOptions};
use m3u8_rs::MediaPlaylist;

use crate::core::playlist::HlsPlaylist;
use crate::errors::RecorderError;
use crate::events::RecorderEvent;

pub struct FlvRecorder {
    url: String,
    user_agent: Option<String>,
    http_headers: Vec<(String, String)>,
    read_timeout: Option<Duration>,
    work_dir: PathBuf,
    enabled: Arc<AtomicBool>,
    event_channel: crate::events::RecorderEventSender,
    live_id: String,
}

impl FlvRecorder {
    pub fn new(
        url: String,
        user_agent: Option<String>,
        http_headers: Vec<(String, String)>,
        work_dir: PathBuf,
        enabled: Arc<AtomicBool>,
        event_channel: impl Into<crate::events::RecorderEventSender>,
        live_id: String,
    ) -> Self {
        Self {
            url,
            user_agent,
            http_headers,
            read_timeout: None,
            work_dir,
            enabled,
            event_channel: event_channel.into(),
            live_id,
        }
    }

    pub fn with_read_timeout(mut self, timeout: Duration) -> Self {
        self.read_timeout = Some(timeout);
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
        if !self.work_dir.exists() {
            std::fs::create_dir_all(&self.work_dir)?;
        }

        let playlist_path = self.work_dir.join("playlist.m3u8");
        let segment_pattern = self.work_dir.join("%d.ts");

        if !playlist_path.exists() {
            let playlist = HlsPlaylist::new(playlist_path.clone()).await?;
            playlist.flush().await?;
        }

        let recording = record_to_hls(
            &self.url,
            &playlist_path,
            &segment_pattern,
            self.input_options(),
            self.enabled.clone(),
        );
        tokio::pin!(recording);
        let mut processed_segments = 0usize;
        let result = loop {
            tokio::select! {
                result = &mut recording => break result,
                _ = tokio::time::sleep(Duration::from_millis(500)) => {
                    self.publish_updates(&playlist_path, &mut processed_segments).await;
                }
            }
        };
        // The native worker can finish before the first poll and also closes
        // its pending final segment on a graceful stop. Account for it once.
        self.publish_updates(&playlist_path, &mut processed_segments)
            .await;
        result.map_err(RecorderError::FfmpegError)?;

        if playlist_path.exists() {
            let mut playlist = HlsPlaylist::new(playlist_path.clone()).await?;
            playlist.close().await?;
        }

        Ok(())
    }

    async fn publish_updates(&self, playlist_path: &Path, processed_segments: &mut usize) {
        let Ok(bytes) = tokio::fs::read(playlist_path).await else {
            return;
        };
        let Ok((_, playlist)) = m3u8_rs::parse_media_playlist(&bytes) else {
            return;
        };
        if let Some((duration_secs, cached_size_bytes)) =
            Self::update_from_playlist(&self.work_dir, &playlist, processed_segments).await
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
    }

    async fn update_from_playlist(
        work_dir: &Path,
        playlist: &MediaPlaylist,
        processed_segments: &mut usize,
    ) -> Option<(f64, u64)> {
        if playlist.segments.len() <= *processed_segments {
            return None;
        }

        let mut duration_delta: f64 = 0.0;
        let mut size_delta = 0u64;

        for segment in &playlist.segments[*processed_segments..] {
            duration_delta += f64::from(segment.duration);
            let segment_path = work_dir.join(segment.uri.as_str());
            if let Ok(metadata) = tokio::fs::metadata(&segment_path).await {
                size_delta += metadata.len();
            }
        }

        *processed_segments = playlist.segments.len();
        if duration_delta <= 0.0 && size_delta == 0 {
            return None;
        }
        Some((duration_delta, size_delta))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
