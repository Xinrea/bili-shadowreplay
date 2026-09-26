use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use ffmpeg_utils::ffmpeg_command;
use m3u8_rs::MediaPlaylist;
use tokio::sync::broadcast;

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
    event_channel: broadcast::Sender<RecorderEvent>,
    live_id: String,
}

impl FlvRecorder {
    pub fn new(
        url: String,
        user_agent: Option<String>,
        http_headers: Vec<(String, String)>,
        work_dir: PathBuf,
        enabled: Arc<AtomicBool>,
        event_channel: broadcast::Sender<RecorderEvent>,
        live_id: String,
    ) -> Self {
        Self {
            url,
            user_agent,
            http_headers,
            read_timeout: None,
            work_dir,
            enabled,
            event_channel,
            live_id,
        }
    }

    pub fn with_read_timeout(mut self, timeout: Duration) -> Self {
        self.read_timeout = Some(timeout);
        self
    }

    fn configure_input(&self, cmd: &mut tokio::process::Command) {
        if let Some(timeout) = self.read_timeout {
            let micros = timeout.as_micros().min(i64::MAX as u128).to_string();
            cmd.args(["-rw_timeout", &micros]);
        }
        if let Some(user_agent) = &self.user_agent {
            cmd.args(["-user_agent", user_agent]);
        }
        if !self.http_headers.is_empty() {
            let headers: String = self
                .http_headers
                .iter()
                .map(|(name, value)| format!("{name}: {value}\r\n"))
                .collect();
            cmd.args(["-headers", &headers]);
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

        let mut cmd = ffmpeg_command();

        // FFmpeg's network options must precede the input. Douyu opts into
        // an idle read limit so a dead CDN cannot stall the recorder forever.
        self.configure_input(&mut cmd);

        cmd.args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-i",
            &self.url,
            "-c",
            "copy",
            // Huya's FLV carries a webvtt-like subtitle track that the HLS
            // muxer rejects with `-c copy`; drop subtitle streams entirely.
            "-sn",
            "-f",
            "hls",
            "-hls_time",
            "4",
            "-hls_list_size",
            "0",
            "-hls_flags",
            "append_list+omit_endlist",
            "-hls_segment_filename",
            segment_pattern.to_string_lossy().as_ref(),
            playlist_path.to_string_lossy().as_ref(),
            "-y",
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

        let mut child = cmd
            .spawn()
            .map_err(|e| RecorderError::FfmpegError(e.to_string()))?;
        let mut processed_segments = 0usize;

        loop {
            if !self.enabled.load(Ordering::Relaxed) {
                let _ = child.kill().await;
                break;
            }

            if let Ok(bytes) = tokio::fs::read(&playlist_path).await {
                if let Ok((_, playlist)) = m3u8_rs::parse_media_playlist(&bytes) {
                    let duration_delta = Self::update_from_playlist(
                        &self.work_dir,
                        &playlist,
                        &mut processed_segments,
                    )
                    .await;
                    if let Some((duration_secs, cached_size_bytes)) = duration_delta {
                        let _ = self.event_channel.send(RecorderEvent::RecordUpdate {
                            live_id: self.live_id.clone(),
                            duration_secs,
                            cached_size_bytes,
                        });
                    }
                }
            }

            if let Some(status) = child
                .try_wait()
                .map_err(|e| RecorderError::FfmpegError(format!("ffmpeg wait failed: {e}")))?
            {
                if !status.success() {
                    return Err(RecorderError::FfmpegError(format!(
                        "ffmpeg exited with status: {status}"
                    )));
                }
                break;
            }

            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        if playlist_path.exists() {
            let mut playlist = HlsPlaylist::new(playlist_path).await?;
            playlist.close().await?;
        }

        Ok(())
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

    #[test]
    fn douyu_read_timeout_precedes_the_ffmpeg_input() {
        let (events, _) = broadcast::channel(1);
        let recorder = FlvRecorder::new(
            "https://cdn.example/live.flv".into(),
            Some("viewer".into()),
            vec![("Referer".into(), "https://www.douyu.com/".into())],
            PathBuf::new(),
            Arc::new(AtomicBool::new(true)),
            events,
            "test".into(),
        )
        .with_read_timeout(Duration::from_secs(15));
        let mut command = ffmpeg_command();
        recorder.configure_input(&mut command);
        command.args(["-i", "https://cdn.example/live.flv"]);
        let args: Vec<_> = command
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "-rw_timeout",
                "15000000",
                "-user_agent",
                "viewer",
                "-headers",
                "Referer: https://www.douyu.com/\r\n",
                "-i",
                "https://cdn.example/live.flv"
            ]
        );
    }
}
