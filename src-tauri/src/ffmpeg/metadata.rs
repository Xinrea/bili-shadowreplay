use std::path::{Path, PathBuf};

use ffmpeg_utils::extract_video_metadata;

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
    ffmpeg_utils::write_jpeg(video_full_path, timestamp, None, target).await?;
    let metadata = tokio::fs::metadata(target)
        .await
        .map_err(|error| format!("读取缩略图失败: {error}"))?;
    log::info!(
        "生成缩略图完成: {} (文件大小: {} bytes)",
        target.display(),
        metadata.len()
    );
    Ok(())
}

/// Identify the linked FFmpeg SDK without requiring a command-line executable.
pub async fn check_ffmpeg() -> Result<String, String> {
    Ok(ffmpeg_utils::linked_ffmpeg_version())
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
    async fn thumbnail_recovers_from_a_seek_past_eof_and_reports_missing_frames() {
        let directory = tempfile::tempdir().unwrap();
        let video = directory.path().join("short.mp4");
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("crates/ffmpeg_utils/src/video_frame/fixtures/seek.mp4"),
            &video,
        )
        .unwrap();

        let missing_frame = directory.path().join("missing-frame.jpg");
        assert!(extract_frame(&video, 60.0, &missing_frame).await.is_err());

        let thumbnail = generate_thumbnail(&video, 60.0).await.unwrap();
        assert_eq!(
            std::fs::read(&thumbnail).unwrap(),
            ffmpeg_utils::extract_jpeg(&video, 0.0, None).await.unwrap()
        );
        let retained = std::fs::read(&thumbnail).unwrap();
        assert!(extract_frame(&video, 60.0, &thumbnail).await.is_err());
        assert_eq!(std::fs::read(&thumbnail).unwrap(), retained);

        // Neither the random position nor the first frame can read a missing video.
        assert!(
            generate_thumbnail(&directory.path().join("missing.mp4"), 60.0)
                .await
                .is_err()
        );
    }
}
