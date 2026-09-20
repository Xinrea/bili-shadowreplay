use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

use async_ffmpeg_sidecar::{event::FfmpegEvent, log_parser::FfmpegLogParser};
use tokio::io::BufReader;

use super::runner::FfmpegJob;
use ffmpeg_utils::{extract_video_metadata, ffmpeg_command};

/// Generate thumbnail file from video, capturing a frame at the specified timestamp.
pub async fn generate_thumbnail(video_full_path: &Path, timestamp: f64) -> Result<PathBuf, String> {
    let thumbnail_full_path = video_full_path.with_extension("jpg");
    FfmpegJob::new()
        .args([
            "-i".to_string(),
            video_full_path.to_string_lossy().into_owned(),
            "-ss".to_string(),
            timestamp.to_string(),
            "-vframes".to_string(),
            "1".to_string(),
            "-y".to_string(),
        ])
        .output(thumbnail_full_path.clone())
        .context("生成缩略图")
        .run_without_reporter()
        .await
        .map_err(|error| format!("ffmpeg生成缩略图失败: {error}"))?;

    if let Ok(metadata) = std::fs::metadata(&thumbnail_full_path) {
        log::info!(
            "生成缩略图完成: {} (文件大小: {} bytes)",
            thumbnail_full_path.display(),
            metadata.len()
        );
    } else {
        log::info!("生成缩略图完成: {}", thumbnail_full_path.display());
    }
    Ok(thumbnail_full_path)
}

/// Trying to run ffmpeg for version
pub async fn check_ffmpeg() -> Result<String, String> {
    let mut child = ffmpeg_command()
        .arg("-version")
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| {
            log::error!("Failed to spawn ffmpeg process: {e}");
            e.to_string()
        })?;

    let stdout = child.stdout.take().ok_or_else(|| {
        log::error!("Failed to take ffmpeg output");
        "Failed to take ffmpeg output".to_string()
    })?;
    let reader = BufReader::new(stdout);
    let mut parser = FfmpegLogParser::new(reader);

    let mut version = None;
    while let Ok(event) = parser.parse_next_event().await {
        match event {
            FfmpegEvent::ParsedVersion(v) => version = Some(v.version),
            FfmpegEvent::LogEOF => break,
            _ => {}
        }
    }

    let _ = child.wait().await;

    if let Some(version) = version {
        Ok(version)
    } else {
        Err("Failed to parse version from output".into())
    }
}

/// Check if all videos have same encoding and resolution
pub async fn check_videos(video_paths: &[&Path]) -> bool {
    // check if all playlist paths exist
    let mut video_codec = "".to_owned();
    let mut audio_codec = "".to_owned();
    let mut width = 0;
    let mut height = 0;
    for video_path in video_paths.iter() {
        if !Path::new(video_path).exists() {
            continue;
        }
        let metadata = match extract_video_metadata(Path::new(video_path)).await {
            Ok(metadata) => metadata,
            Err(error) => {
                log::error!("Failed to extract video metadata: {error}");
                return false;
            }
        };

        // check video codec
        if !video_codec.is_empty() && metadata.video_codec != video_codec {
            log::error!("Video codec does not match: {}", video_path.display());
            return false;
        } else {
            video_codec = metadata.video_codec;
        }

        // check audio codec
        if !audio_codec.is_empty() && metadata.audio_codec != audio_codec {
            log::error!("Audio codec does not match: {}", video_path.display());
            return false;
        } else {
            audio_codec = metadata.audio_codec;
        }

        // check width
        if width > 0 && metadata.width != width {
            log::error!("Video width does not match: {}", video_path.display());
            return false;
        } else {
            width = metadata.width;
        }

        // check height
        if height > 0 && metadata.height != height {
            log::error!("Video height does not match: {}", video_path.display());
            return false;
        } else {
            height = metadata.height;
        }
    }

    true
}
