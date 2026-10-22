use crate::state::State;
use crate::state_type;
use base64::Engine;
use ffmpeg_utils::{extract_video_metadata, ffmpeg_command, path_str, VideoMetadata};
#[cfg(feature = "gui")]
use recorder::platforms::PlatformType;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
#[cfg(feature = "gui")]
use std::str::FromStr;

#[cfg(feature = "gui")]
use tauri::State as TauriState;

#[derive(Debug, Serialize, Deserialize)]
pub struct VideoFrame {
    pub timestamp: f64,
    pub image_base64: String,
}

#[cfg(feature = "gui")]
#[derive(Debug, Serialize, Deserialize)]
pub struct DanmuHighlight {
    pub start_time: f64,
    pub end_time: f64,
    pub comment_count: usize,
    pub density: f64,
    pub sample_comments: Vec<String>,
}

#[cfg(feature = "gui")]
#[derive(Debug, Serialize, Deserialize)]
pub struct DanmuKeywordMatch {
    pub timestamp: f64,
    pub content: String,
    pub keyword: String,
    pub context_start: f64,
    pub context_end: f64,
}

fn resolve_video_path(output_dir: &Path, file: &str) -> PathBuf {
    let path = PathBuf::from(file);
    if path.is_absolute() {
        path
    } else {
        output_dir.join(path)
    }
}

/// Extract frames from a video at specific timestamps or evenly distributed
#[cfg_attr(feature = "gui", tauri::command)]
pub async fn extract_video_frames(
    state: state_type!(),
    video_id: i64,
    timestamps: Vec<f64>,
    max_frames: usize,
) -> Result<Vec<VideoFrame>, String> {
    // Get video info
    let video = state
        .db
        .get_video(video_id)
        .await
        .map_err(|e| format!("Failed to get video: {}", e))?;

    let output_dir = PathBuf::from(&state.config.read().await.output);
    let video_path = resolve_video_path(&output_dir, &video.file);
    if !video_path.exists() {
        return Err(format!("Video file not found: {}", video_path.display()));
    }

    // Get video duration
    let metadata = get_video_metadata_internal(&video_path).await?;

    // Determine timestamps to extract
    let extract_timestamps = if timestamps.is_empty() {
        // Evenly distribute frames
        let count = max_frames.min(10);
        (0..count)
            .map(|i| (i as f64 / count as f64) * metadata.duration)
            .collect::<Vec<_>>()
    } else {
        timestamps.into_iter().take(max_frames).collect()
    };

    let mut frames = Vec::new();

    for ts in extract_timestamps {
        if ts > metadata.duration {
            continue;
        }

        // Extract frame using ffmpeg
        let frame_data = extract_frame_at_timestamp(&video_path, ts).await?;
        frames.push(VideoFrame {
            timestamp: ts,
            image_base64: frame_data,
        });
    }

    Ok(frames)
}

/// Extract a single frame at a specific timestamp
async fn extract_frame_at_timestamp(video_path: &Path, timestamp: f64) -> Result<String, String> {
    // Give every extraction its own directory: MCP clients may request the
    // same timestamp concurrently. No open file handle blocks FFmpeg on
    // Windows, and the directory is removed on both success and failure.
    let temporary_dir = tempfile::Builder::new()
        .prefix("bsr-frame-")
        .tempdir()
        .map_err(|error| format!("Failed to create frame directory: {error}"))?;
    let output_path = temporary_dir.path().join("frame.jpg");

    let mut cmd = ffmpeg_command();

    cmd.args([
        "-ss",
        &timestamp.to_string(),
        "-i",
        path_str(video_path)?,
        "-vframes",
        "1",
        "-q:v",
        "2",
        "-y",
        path_str(&output_path)?,
    ]);

    let output = cmd
        .output()
        .await
        .map_err(|e| format!("Failed to run ffmpeg: {}", e))?;

    if !output.status.success() {
        return Err(format!(
            "FFmpeg failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    // Read and encode to base64
    let image_data = tokio::fs::read(&output_path)
        .await
        .map_err(|e| format!("Failed to read frame: {}", e))?;

    let base64_data = base64::engine::general_purpose::STANDARD.encode(&image_data);

    Ok(base64_data)
}

/// Get detailed video metadata
#[cfg(feature = "gui")]
#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_video_metadata(
    state: state_type!(),
    video_id: i64,
) -> Result<VideoMetadata, String> {
    let video = state
        .db
        .get_video(video_id)
        .await
        .map_err(|e| format!("Failed to get video: {}", e))?;

    let output_dir = PathBuf::from(&state.config.read().await.output);
    let video_path = resolve_video_path(&output_dir, &video.file);
    get_video_metadata_internal(&video_path).await
}

/// Internal function to get video metadata
async fn get_video_metadata_internal(video_path: &Path) -> Result<VideoMetadata, String> {
    if !video_path.exists() {
        return Err("Video file not found".to_string());
    }

    extract_video_metadata(video_path).await
}

/// Analyze danmu to find highlight moments based on comment density
#[cfg(feature = "gui")]
#[cfg_attr(feature = "gui", tauri::command)]
pub async fn analyze_danmu_highlights(
    state: state_type!(),
    platform: String,
    room_id: String,
    live_id: String,
    time_window: f64,
    min_density: usize,
) -> Result<Vec<DanmuHighlight>, String> {
    // Get danmu records using recorder_manager
    let platform_type = PlatformType::from_str(&platform)?;
    let danmu_records = state
        .recorder_manager
        .load_danmus(platform_type, &room_id, &live_id)
        .await
        .map_err(|e| format!("Failed to get danmu: {}", e))?;

    if danmu_records.is_empty() {
        return Ok(Vec::new());
    }

    // Find max timestamp to determine total duration
    let max_ts = danmu_records.iter().map(|d| d.ts).max().unwrap_or(0);

    let duration = max_ts as f64 / 1000.0; // Convert ms to seconds
    let window_count = (duration / time_window).ceil() as usize;

    let mut highlights = Vec::new();

    for i in 0..window_count {
        let start_time = i as f64 * time_window;
        let end_time = ((i + 1) as f64 * time_window).min(duration);

        let start_ts = (start_time * 1000.0) as i64;
        let end_ts = (end_time * 1000.0) as i64;

        // Count comments in this window
        let comments_in_window: Vec<_> = danmu_records
            .iter()
            .filter(|d| d.ts >= start_ts && d.ts < end_ts)
            .collect();

        let count = comments_in_window.len();

        if count >= min_density {
            let density = count as f64 / time_window;

            // Sample some comments
            let sample_comments: Vec<String> = comments_in_window
                .iter()
                .take(5)
                .map(|d| d.content.clone())
                .collect();

            highlights.push(DanmuHighlight {
                start_time,
                end_time,
                comment_count: count,
                density,
                sample_comments,
            });
        }
    }

    Ok(highlights)
}

/// Search for specific keywords in danmu
#[cfg(feature = "gui")]
#[cfg_attr(feature = "gui", tauri::command)]
pub async fn search_danmu_keywords(
    state: state_type!(),
    platform: String,
    room_id: String,
    live_id: String,
    keywords: Vec<String>,
    context_seconds: f64,
) -> Result<Vec<DanmuKeywordMatch>, String> {
    let platform_type = PlatformType::from_str(&platform)?;
    let danmu_records = state
        .recorder_manager
        .load_danmus(platform_type, &room_id, &live_id)
        .await
        .map_err(|e| format!("Failed to get danmu: {}", e))?;

    let mut matches = Vec::new();

    for record in danmu_records {
        for keyword in &keywords {
            if record.content.contains(keyword) {
                let timestamp = record.ts as f64 / 1000.0;
                let context_start = (timestamp - context_seconds).max(0.0);
                let context_end = timestamp + context_seconds;

                matches.push(DanmuKeywordMatch {
                    timestamp,
                    content: record.content.clone(),
                    keyword: keyword.clone(),
                    context_start,
                    context_end,
                });
                break; // Only match once per comment
            }
        }
    }

    Ok(matches)
}

/// Merge multiple videos into one
#[cfg(feature = "gui")]
#[cfg_attr(feature = "gui", tauri::command)]
pub async fn merge_videos(
    state: state_type!(),
    video_ids: Vec<i64>,
    output_title: String,
    output_note: String,
    transition: Option<String>,
) -> Result<i64, String> {
    if video_ids.is_empty() {
        return Err("No videos to merge".to_string());
    }

    // Get all videos
    let mut videos = Vec::new();
    for id in &video_ids {
        let video = state
            .db
            .get_video(*id)
            .await
            .map_err(|e| format!("Failed to get video {}: {}", id, e))?;
        videos.push(video);
    }

    // Determine output path
    let output_dir = PathBuf::from(&state.config.read().await.output);
    let video_paths = videos
        .iter()
        .map(|video| resolve_video_path(&output_dir, &video.file))
        .collect::<Vec<_>>();
    let output_filename = format!(
        "merged_{}_{}.mp4",
        chrono::Local::now().format("%Y%m%d_%H%M%S"),
        uuid::Uuid::new_v4()
    );
    let output_path = output_dir.join(&output_filename);

    let transition_type = transition.as_deref().unwrap_or("none");

    // If no transition or only one video, use simple concat
    if transition_type == "none" || videos.len() == 1 {
        // Create concat file list
        let concat_file = std::env::temp_dir().join(format!("concat_{}.txt", uuid::Uuid::new_v4()));
        let mut concat_content = String::new();

        for video_path in &video_paths {
            // Escape path for FFmpeg concat demuxer
            // Only escape backslashes and single quotes
            // Square brackets work fine inside single quotes
            let path_str = video_path
                .to_string_lossy()
                .replace('\\', "\\\\")
                .replace('\'', "'\\''");
            concat_content.push_str(&format!("file '{}'\n", path_str));
        }

        tokio::fs::write(&concat_file, concat_content)
            .await
            .map_err(|e| format!("Failed to write concat file: {}", e))?;

        // Run ffmpeg concat
        let mut cmd = ffmpeg_command();

        cmd.args([
            "-f",
            "concat",
            "-safe",
            "0",
            "-i",
            path_str(&concat_file)?,
            "-c",
            "copy",
            "-y",
            path_str(&output_path)?,
        ]);

        let output = cmd
            .output()
            .await
            .map_err(|e| format!("Failed to run ffmpeg: {}", e))?;

        // Cleanup concat file
        let _ = tokio::fs::remove_file(&concat_file).await;

        if !output.status.success() {
            return Err(format!(
                "FFmpeg merge failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
    } else {
        // Only pass known xfade transitions into the filter graph
        let xfade_transition = match transition_type {
            "fade" => "fade",
            "dissolve" => "dissolve",
            "wipeleft" => "wipeleft",
            "wiperight" => "wiperight",
            "slideup" => "slideup",
            "slidedown" => "slidedown",
            _ => "fade",
        };

        // Shared with clip export: cross-fades the audio along with the video
        // and conforms inputs of different size or frame rate for xfade.
        crate::ffmpeg::general::concat_videos_with_transition(
            None::<&crate::progress::progress_reporter::ProgressReporter>,
            &video_paths,
            &output_path,
            Some(xfade_transition),
        )
        .await
        .map_err(|e| format!("FFmpeg merge with transition failed: {}", e))?;
    }

    // Get file size
    let file_size = tokio::fs::metadata(&output_path)
        .await
        .map_err(|e| format!("Failed to get file size: {}", e))?
        .len();

    // Transitions overlap neighbouring videos, so the merged length is shorter
    // than the sum of the inputs; read it from the result instead.
    let total_duration = match extract_video_metadata(&output_path).await {
        Ok(metadata) => metadata.duration as i64,
        Err(e) => {
            log::warn!("Failed to read merged video length: {e}");
            videos.iter().map(|v| v.length).sum()
        }
    };

    // Create new video row
    let new_video = crate::database::video::VideoRow {
        id: 0, // Will be auto-generated
        room_id: videos[0].room_id.clone(),
        cover: String::new(),
        // Relative to the output directory, like every other video row
        file: output_filename,
        note: output_note.clone(),
        length: total_duration,
        size: file_size as i64,
        status: 0,
        bvid: String::new(),
        title: output_title.clone(),
        desc: String::new(),
        tags: String::new(),
        area: 0,
        created_at: chrono::Utc::now().to_rfc3339(),
        platform: videos[0].platform.clone(),
        live_id: String::new(),
        source: String::new(),
        duration: 0.0,
        danmu_embedded: videos.iter().any(|video| video.danmu_embedded),
    };

    // Insert into database
    let result = state
        .db
        .add_video(&new_video)
        .await
        .map_err(|e| format!("Failed to insert video: {}", e))?;

    Ok(result.id)
}

/// Extract audio from video
#[cfg(feature = "gui")]
#[cfg_attr(feature = "gui", tauri::command)]
pub async fn extract_video_audio(state: state_type!(), video_id: i64) -> Result<String, String> {
    let video = state
        .db
        .get_video(video_id)
        .await
        .map_err(|e| format!("Failed to get video: {}", e))?;

    let output_dir = PathBuf::from(&state.config.read().await.output);
    let video_path = resolve_video_path(&output_dir, &video.file);
    if !video_path.exists() {
        return Err(format!("Video file not found: {}", video_path.display()));
    }

    // Determine output path
    let output_filename = format!(
        "audio_{}_{}.mp3",
        video.id,
        chrono::Local::now().format("%Y%m%d_%H%M%S")
    );
    let output_path = output_dir.join(&output_filename);

    // Extract audio using ffmpeg
    let mut cmd = ffmpeg_command();

    cmd.args([
        "-i",
        path_str(&video_path)?,
        "-vn",
        "-acodec",
        "libmp3lame",
        "-q:a",
        "2",
        "-y",
        path_str(&output_path)?,
    ]);

    let output = cmd
        .output()
        .await
        .map_err(|e| format!("Failed to run ffmpeg: {}", e))?;

    if !output.status.success() {
        return Err(format!(
            "FFmpeg audio extraction failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    Ok(output_path.to_string_lossy().to_string())
}

/// Get archive metadata
#[cfg(feature = "gui")]
#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_archive_metadata(
    state: state_type!(),
    platform: String,
    room_id: String,
    live_id: String,
) -> Result<serde_json::Value, String> {
    // Use get_record instead of get_archive
    let archive = state
        .db
        .get_record(&room_id, &live_id)
        .await
        .map_err(|e| format!("Failed to get archive: {}", e))?;

    // Get file path and metadata
    let cache_dir = PathBuf::from(&state.config.read().await.cache);
    let file_path = cache_dir
        .join(&platform)
        .join(&room_id)
        .join(&live_id)
        .join("output.mp4");

    let (file_size, video_metadata) = if file_path.exists() {
        let size = tokio::fs::metadata(&file_path)
            .await
            .map(|m| m.len())
            .unwrap_or(0);

        let metadata = get_video_metadata_internal(&file_path).await.ok();
        (size, metadata)
    } else {
        (0, None)
    };

    Ok(serde_json::json!({
        "live_id": archive.live_id,
        "room_id": archive.room_id,
        "platform": platform,
        "title": archive.title,
        "file_path": file_path.to_string_lossy(),
        "file_size": file_size,
        "created_at": archive.created_at,
        "video_metadata": video_metadata,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn frame_extraction_returns_base64_jpeg_without_gui() {
        // Match ffmpeg_utils tests: CI installs FFmpeg, but local contributors
        // can still run the test suite without the optional media executable.
        let available = ffmpeg_command()
            .arg("-version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await;
        if let Err(error) = &available {
            if error.kind() == std::io::ErrorKind::NotFound {
                eprintln!("Skipping frame extraction test: FFmpeg is not installed");
                return;
            }
        }
        assert!(available.unwrap().success(), "FFmpeg version check failed");
        let directory = tempfile::tempdir().unwrap();
        let video_path = directory.path().join("test.mp4");
        let output = ffmpeg_command()
            .args(["-f", "lavfi", "-i", "color=c=red:s=32x32:d=1", "-y"])
            .arg(&video_path)
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let data = extract_frame_at_timestamp(&video_path, 0.5).await.unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .unwrap();
        assert!(bytes.starts_with(&[0xff, 0xd8, 0xff]));
        assert!(bytes.ends_with(&[0xff, 0xd9]));
    }

    #[test]
    fn relative_video_paths_are_resolved_from_output_directory() {
        let output_dir = Path::new("/configured/output");
        assert_eq!(
            resolve_video_path(output_dir, "clip.mp4"),
            output_dir.join("clip.mp4")
        );
    }

    #[test]
    fn absolute_video_paths_are_preserved() {
        let video_path = std::env::temp_dir().join("clip.mp4");
        assert_eq!(
            resolve_video_path(
                Path::new("/configured/output"),
                video_path.to_str().unwrap()
            ),
            video_path
        );
    }
}
