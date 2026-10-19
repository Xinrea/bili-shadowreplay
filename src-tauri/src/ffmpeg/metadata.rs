use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

use async_ffmpeg_sidecar::{event::FfmpegEvent, log_parser::FfmpegLogParser};
use tokio::io::BufReader;

use super::runner::FfmpegJob;
use ffmpeg_utils::{extract_video_metadata, ffmpeg_command};

/// Pick a random point in the video's middle 80% to avoid intros and outros.
pub fn get_random_thumbnail_timestamp(duration: f64) -> f64 {
    if !duration.is_finite() || duration <= 0.0 {
        return 0.0;
    }
    duration * rand::random_range(0.1..0.9)
}

/// Generate a JPEG thumbnail, retrying at the start if the requested seek fails.
pub async fn generate_thumbnail(video_full_path: &Path, timestamp: f64) -> Result<PathBuf, String> {
    let thumbnail_full_path = video_full_path.with_extension("jpg");
    if let Err(error) = extract_frame(video_full_path, timestamp, &thumbnail_full_path).await {
        if timestamp == 0.0 {
            return Err(error);
        }
        log::warn!(
            "Failed to capture thumbnail at {timestamp}s for {}: {error}; retrying at the start",
            video_full_path.display()
        );
        extract_frame(video_full_path, 0.0, &thumbnail_full_path).await?;
    }
    Ok(thumbnail_full_path)
}

/// Save the frame of `video_full_path` at `timestamp` as the image `target`.
pub async fn extract_frame(
    video_full_path: &Path,
    timestamp: f64,
    target: &Path,
) -> Result<(), String> {
    let thumbnail_full_path = target.to_path_buf();
    FfmpegJob::new()
        .args([
            "-ss".to_string(),
            timestamp.to_string(),
            "-i".to_string(),
            video_full_path.to_string_lossy().into_owned(),
            "-vframes".to_string(),
            "1".to_string(),
            "-y".to_string(),
        ])
        .output(thumbnail_full_path.clone())
        .context("生成缩略图")
        .run_without_reporter()
        .await
        .map_err(|error| format!("ffmpeg生成缩略图失败: {error}"))?;

    // FFmpeg may exit successfully after seeking past EOF without writing a frame.
    let metadata = tokio::fs::metadata(&thumbnail_full_path)
        .await
        .map_err(|e| format!("缩略图文件未生成: {} ({e})", thumbnail_full_path.display()))?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err(format!("缩略图文件为空: {}", thumbnail_full_path.display()));
    }
    log::info!(
        "生成缩略图完成: {} (文件大小: {} bytes)",
        thumbnail_full_path.display(),
        metadata.len()
    );
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires FFmpeg with libx264"]
    async fn thumbnail_recovers_from_a_seek_past_eof_and_reports_missing_frames() {
        let directory = tempfile::tempdir().unwrap();
        let video = directory.path().join("short.mp4");
        let output = ffmpeg_command()
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=red:s=64x64:r=25:d=0.5",
                "-c:v",
                "libx264",
                "-y",
            ])
            .arg(&video)
            .output()
            .await
            .unwrap();
        assert!(output.status.success());

        let missing_frame = directory.path().join("missing-frame.jpg");
        assert!(extract_frame(&video, 60.0, &missing_frame).await.is_err());

        let thumbnail = generate_thumbnail(&video, 60.0).await.unwrap();
        assert!(std::fs::metadata(thumbnail).unwrap().len() > 0);

        // Neither the random position nor the first frame can read a missing video.
        assert!(
            generate_thumbnail(&directory.path().join("missing.mp4"), 60.0)
                .await
                .is_err()
        );
    }
}
