use crate::state::State;
use crate::state_type;
use base64::Engine;
use recorder::platforms::PlatformType;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::str::FromStr;

#[cfg(feature = "gui")]
use tauri::State as TauriState;

#[derive(Debug, Serialize, Deserialize)]
pub struct VideoFrame {
    pub timestamp: f64,
    pub image_base64: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VideoMetadata {
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub video_codec: String,
    pub audio_codec: String,
    pub bitrate: u64,
    pub fps: f64,
    pub file_size: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DanmuHighlight {
    pub start_time: f64,
    pub end_time: f64,
    pub comment_count: usize,
    pub density: f64,
    pub sample_comments: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DanmuKeywordMatch {
    pub timestamp: f64,
    pub content: String,
    pub keyword: String,
    pub context_start: f64,
    pub context_end: f64,
}

// Helper function to get ffmpeg path
fn get_ffmpeg_path() -> PathBuf {
    let mut path = Path::new("ffmpeg").to_path_buf();
    if cfg!(windows) {
        path.set_extension("exe");
    }
    path
}

// Helper function to get ffprobe path
fn get_ffprobe_path() -> PathBuf {
    let mut path = Path::new("ffprobe").to_path_buf();
    if cfg!(windows) {
        path.set_extension("exe");
    }
    path
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

    let video_path = PathBuf::from(&video.file);
    if !video_path.exists() {
        return Err("Video file not found".to_string());
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
    let output_path = std::env::temp_dir().join(format!("frame_{}.jpg", timestamp));

    let ffmpeg_path = get_ffmpeg_path();
    let mut cmd = tokio::process::Command::new(ffmpeg_path);

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    cmd.args([
        "-ss",
        &timestamp.to_string(),
        "-i",
        video_path.to_str().unwrap(),
        "-vframes",
        "1",
        "-q:v",
        "2",
        "-y",
        output_path.to_str().unwrap(),
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

    // Cleanup
    let _ = tokio::fs::remove_file(&output_path).await;

    Ok(base64_data)
}

/// Get detailed video metadata
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

    let video_path = PathBuf::from(&video.file);
    get_video_metadata_internal(&video_path).await
}

/// Internal function to get video metadata
async fn get_video_metadata_internal(video_path: &Path) -> Result<VideoMetadata, String> {
    if !video_path.exists() {
        return Err("Video file not found".to_string());
    }

    let ffprobe_path = get_ffprobe_path();
    let mut cmd = tokio::process::Command::new(ffprobe_path);

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    cmd.args([
        "-v",
        "quiet",
        "-print_format",
        "json",
        "-show_format",
        "-show_streams",
        video_path.to_str().unwrap(),
    ]);

    let output = cmd
        .output()
        .await
        .map_err(|e| format!("Failed to run ffprobe: {}", e))?;

    if !output.status.success() {
        return Err("FFprobe failed".to_string());
    }

    let json_str = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&json_str)
        .map_err(|e| format!("Failed to parse ffprobe output: {}", e))?;

    // Extract metadata
    let format = json.get("format").ok_or("No format info")?;
    let streams = json
        .get("streams")
        .and_then(|s| s.as_array())
        .ok_or("No streams")?;

    let duration = format
        .get("duration")
        .and_then(|d| d.as_str())
        .and_then(|d| d.parse::<f64>().ok())
        .unwrap_or(0.0);

    let file_size = format
        .get("size")
        .and_then(|s| s.as_str())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);

    let bitrate = format
        .get("bit_rate")
        .and_then(|b| b.as_str())
        .and_then(|b| b.parse::<u64>().ok())
        .unwrap_or(0);

    // Find video stream
    let video_stream = streams
        .iter()
        .find(|s| s.get("codec_type").and_then(|t| t.as_str()) == Some("video"))
        .ok_or("No video stream found")?;

    let width = video_stream
        .get("width")
        .and_then(|w| w.as_u64())
        .unwrap_or(0) as u32;
    let height = video_stream
        .get("height")
        .and_then(|h| h.as_u64())
        .unwrap_or(0) as u32;
    let video_codec = video_stream
        .get("codec_name")
        .and_then(|c| c.as_str())
        .unwrap_or("unknown")
        .to_string();

    // Calculate FPS
    let fps = video_stream
        .get("r_frame_rate")
        .and_then(|r| r.as_str())
        .and_then(|r| {
            let parts: Vec<&str> = r.split('/').collect();
            if parts.len() == 2 {
                let num = parts[0].parse::<f64>().ok()?;
                let den = parts[1].parse::<f64>().ok()?;
                Some(num / den)
            } else {
                None
            }
        })
        .unwrap_or(0.0);

    // Find audio stream
    let audio_stream = streams
        .iter()
        .find(|s| s.get("codec_type").and_then(|t| t.as_str()) == Some("audio"));

    let audio_codec = audio_stream
        .and_then(|s| s.get("codec_name"))
        .and_then(|c| c.as_str())
        .unwrap_or("none")
        .to_string();

    Ok(VideoMetadata {
        duration,
        width,
        height,
        video_codec,
        audio_codec,
        bitrate,
        fps,
        file_size,
    })
}

/// Analyze danmu to find highlight moments based on comment density
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
    let output_filename = format!(
        "merged_{}_{}.mp4",
        chrono::Local::now().format("%Y%m%d_%H%M%S"),
        uuid::Uuid::new_v4()
    );
    let output_path = output_dir.join(&output_filename);

    let ffmpeg_path = get_ffmpeg_path();
    let transition_type = transition.as_deref().unwrap_or("none");

    // If no transition or only one video, use simple concat
    if transition_type == "none" || videos.len() == 1 {
        // Create concat file list
        let concat_file = std::env::temp_dir().join(format!("concat_{}.txt", uuid::Uuid::new_v4()));
        let mut concat_content = String::new();

        for video in &videos {
            // Escape path for FFmpeg concat demuxer
            // Only escape backslashes and single quotes
            // Square brackets work fine inside single quotes
            let path_str = video.file.replace('\\', "\\\\").replace('\'', "'\\''");
            concat_content.push_str(&format!("file '{}'\n", path_str));
        }

        tokio::fs::write(&concat_file, concat_content)
            .await
            .map_err(|e| format!("Failed to write concat file: {}", e))?;

        // Run ffmpeg concat
        let mut cmd = tokio::process::Command::new(ffmpeg_path);

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        cmd.args([
            "-f",
            "concat",
            "-safe",
            "0",
            "-i",
            concat_file.to_str().unwrap(),
            "-c",
            "copy",
            "-y",
            output_path.to_str().unwrap(),
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
        // Use xfade filter for transitions
        let transition_duration = 1.0; // 1 second transition

        // Build complex filter for xfade transitions
        let mut filter_complex = String::new();

        // Load all videos
        for (i, _video) in videos.iter().enumerate() {
            filter_complex.push_str(&format!("[{}:v]", i));
        }

        // Chain xfade filters
        let xfade_transition = match transition_type {
            "fade" => "fade",
            "dissolve" => "dissolve",
            "wipeleft" => "wipeleft",
            "wiperight" => "wiperight",
            "slideup" => "slideup",
            "slidedown" => "slidedown",
            _ => "fade",
        };

        for i in 0..(videos.len() - 1) {
            if i == 0 {
                filter_complex.push_str(&format!(
                    "[0:v][1:v]xfade=transition={}:duration={}:offset={}[v{}];",
                    xfade_transition,
                    transition_duration,
                    videos[0].length as f64 - transition_duration,
                    i + 1
                ));
            } else if i < videos.len() - 2 {
                let offset: f64 = videos
                    .iter()
                    .take(i + 1)
                    .map(|v| v.length as f64)
                    .sum::<f64>()
                    - (i as f64 + 1.0) * transition_duration;
                filter_complex.push_str(&format!(
                    "[v{}][{}:v]xfade=transition={}:duration={}:offset={}[v{}];",
                    i,
                    i + 1,
                    xfade_transition,
                    transition_duration,
                    offset,
                    i + 1
                ));
            } else {
                let offset: f64 = videos
                    .iter()
                    .take(i + 1)
                    .map(|v| v.length as f64)
                    .sum::<f64>()
                    - (i as f64 + 1.0) * transition_duration;
                filter_complex.push_str(&format!(
                    "[v{}][{}:v]xfade=transition={}:duration={}:offset={}[outv]",
                    i,
                    i + 1,
                    xfade_transition,
                    transition_duration,
                    offset
                ));
            }
        }

        // Build ffmpeg command with multiple inputs
        let mut cmd = tokio::process::Command::new(&ffmpeg_path);

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        // Add all input files
        for video in &videos {
            cmd.args(["-i", &video.file]);
        }

        // Add filter complex
        cmd.args([
            "-filter_complex",
            &filter_complex,
            "-map",
            "[outv]",
            "-map",
            "0:a",
            "-c:v",
            "libx264",
            "-preset",
            "medium",
            "-crf",
            "23",
            "-c:a",
            "aac",
            "-y",
            output_path.to_str().unwrap(),
        ]);

        let output = cmd
            .output()
            .await
            .map_err(|e| format!("Failed to run ffmpeg: {}", e))?;

        if !output.status.success() {
            return Err(format!(
                "FFmpeg merge with transition failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
    }

    // Get file size
    let file_size = tokio::fs::metadata(&output_path)
        .await
        .map_err(|e| format!("Failed to get file size: {}", e))?
        .len();

    // Calculate total duration
    let total_duration: i64 = videos.iter().map(|v| v.length).sum();

    // Create new video row
    let new_video = crate::database::video::VideoRow {
        id: 0, // Will be auto-generated
        room_id: videos[0].room_id.clone(),
        cover: String::new(),
        file: output_path.to_string_lossy().to_string(),
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
#[cfg_attr(feature = "gui", tauri::command)]
pub async fn extract_video_audio(state: state_type!(), video_id: i64) -> Result<String, String> {
    let video = state
        .db
        .get_video(video_id)
        .await
        .map_err(|e| format!("Failed to get video: {}", e))?;

    let video_path = PathBuf::from(&video.file);
    if !video_path.exists() {
        return Err("Video file not found".to_string());
    }

    // Determine output path
    let output_dir = PathBuf::from(&state.config.read().await.output);
    let output_filename = format!(
        "audio_{}_{}.mp3",
        video.id,
        chrono::Local::now().format("%Y%m%d_%H%M%S")
    );
    let output_path = output_dir.join(&output_filename);

    // Extract audio using ffmpeg
    let ffmpeg_path = get_ffmpeg_path();
    let mut cmd = tokio::process::Command::new(ffmpeg_path);

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    cmd.args([
        "-i",
        video_path.to_str().unwrap(),
        "-vn",
        "-acodec",
        "libmp3lame",
        "-q:a",
        "2",
        "-y",
        output_path.to_str().unwrap(),
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
