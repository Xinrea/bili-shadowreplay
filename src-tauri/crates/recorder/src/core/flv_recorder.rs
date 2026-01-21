use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use m3u8_rs::MediaPlaylist;
use reqwest::header::HeaderMap;
use tokio::process::Command;
use tokio::sync::broadcast;

use crate::core::playlist::HlsPlaylist;
use crate::errors::RecorderError;
use crate::events::RecorderEvent;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;
#[cfg(target_os = "windows")]
#[allow(unused_imports)]
use std::os::windows::process::CommandExt;

pub struct FlvRecorder {
    url: String,
    headers: HeaderMap,
    work_dir: PathBuf,
    enabled: Arc<AtomicBool>,
    event_channel: broadcast::Sender<RecorderEvent>,
    live_id: String,
}

impl FlvRecorder {
    pub fn new(
        url: String,
        headers: HeaderMap,
        work_dir: PathBuf,
        enabled: Arc<AtomicBool>,
        event_channel: broadcast::Sender<RecorderEvent>,
        live_id: String,
    ) -> Self {
        Self {
            url,
            headers,
            work_dir,
            enabled,
            event_channel,
            live_id,
        }
    }

    fn build_header_string(&self) -> String {
        let mut parts = Vec::new();
        for (key, value) in self.headers.iter() {
            if let Ok(val) = value.to_str() {
                parts.push(format!("{}: {}", key.as_str(), val));
            }
        }
        // ffmpeg expects CRLF between headers
        format!("{}\r\n", parts.join("\r\n"))
    }

    pub async fn start(&self) -> Result<(), RecorderError> {
        if !self.work_dir.exists() {
            std::fs::create_dir_all(&self.work_dir)?;
        }

        let playlist_path = self.work_dir.join("playlist.m3u8");
        let segment_pattern = self.work_dir.join("%d.ts");

        if !playlist_path.exists() {
            let playlist = HlsPlaylist::new(playlist_path.clone()).await;
            playlist.flush().await?;
        }

        let mut cmd = Command::new("ffmpeg");
        #[cfg(target_os = "windows")]
        cmd.creation_flags(CREATE_NO_WINDOW);

        cmd.args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-headers",
            &self.build_header_string(),
            "-i",
            &self.url,
            "-c",
            "copy",
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

        let mut child = cmd.spawn().map_err(|e| RecorderError::FfmpegError(e.to_string()))?;
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

            if let Some(status) = child.try_wait().map_err(|e| {
                RecorderError::FfmpegError(format!("ffmpeg wait failed: {e}"))
            })? {
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
            let mut playlist = HlsPlaylist::new(playlist_path).await;
            let _ = playlist.close().await;
        }

        Ok(())
    }

    async fn update_from_playlist(
        work_dir: &PathBuf,
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
