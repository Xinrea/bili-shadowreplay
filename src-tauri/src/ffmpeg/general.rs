use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

use async_ffmpeg_sidecar::{event::FfmpegEvent, log_parser::FfmpegLogParser};
use tokio::io::{AsyncWriteExt, BufReader};

use crate::{ffmpeg::hwaccel, progress::progress_reporter::ProgressReporterTrait};

use super::ffmpeg_command;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;
#[cfg(target_os = "windows")]
#[allow(unused_imports)]
use std::os::windows::process::CommandExt;

/// Generate a random filename in hex
pub async fn random_filename() -> String {
    format!("{:x}", rand::random::<u64>())
}

/// Escape path for FFmpeg concat demuxer
/// According to FFmpeg docs, when using single quotes in concat files:
/// - Single quotes need special escaping: ' -> '\''
/// - Backslashes need escaping: \ -> \\
/// - Square brackets [] do NOT need escaping when inside single quotes
fn escape_concat_path(path: &Path) -> String {
    let path_str = path.to_string_lossy();

    // On Windows, canonicalize returns UNC paths like \\?\D:\path
    // FFmpeg doesn't handle these well, so we need to strip the UNC prefix
    #[cfg(target_os = "windows")]
    let path_str = {
        let s = path_str.as_ref();
        if s.starts_with(r"\\?\") {
            std::borrow::Cow::Borrowed(&s[4..])
        } else {
            path_str
        }
    };

    // Only escape backslashes and single quotes
    // Do NOT escape square brackets - they work fine in single-quoted paths
    path_str.replace('\\', "\\\\").replace('\'', "'\\''")
}

pub async fn handle_ffmpeg_process(
    reporter: Option<&impl ProgressReporterTrait>,
    ffmpeg_process: &mut tokio::process::Command,
) -> Result<(), String> {
    log::info!("[FFmpeg] {:?}", ffmpeg_process);
    let mut child = ffmpeg_process
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or("ffmpeg stderr pipe unavailable")?;
    let reader = BufReader::new(stderr);
    let mut parser = FfmpegLogParser::new(reader);
    while let Ok(event) = parser.parse_next_event().await {
        match event {
            FfmpegEvent::Log(_level, content) => {
                // if contains "out_time_ms=66654667", by the way, it's actually in us
                if content.starts_with("out_time_ms") {
                    let time_str = content.strip_prefix("out_time_ms=").unwrap_or_default();
                    if let Some(reporter) = reporter {
                        reporter.update(time_str).await;
                    }
                }
            }
            FfmpegEvent::LogEOF => break,
            FfmpegEvent::Error(e) => {
                log::error!("[FFmpeg Error] {}", e);
                return Err(e);
            }
            _ => {}
        }
    }
    let status = child.wait().await.map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("FFmpeg exited with status: {}", status));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_concat_path_plain() {
        let path = Path::new("/tmp/video.mp4");
        assert_eq!(escape_concat_path(path), "/tmp/video.mp4");
    }

    #[test]
    fn test_escape_concat_path_single_quote() {
        let path = Path::new("/tmp/it's a video.mp4");
        assert_eq!(escape_concat_path(path), "/tmp/it'\\''s a video.mp4");
    }

    #[test]
    fn test_escape_concat_path_square_brackets() {
        let path = Path::new("/tmp/video [1].mp4");
        assert_eq!(escape_concat_path(path), "/tmp/video [1].mp4");
    }

    #[test]
    fn test_escape_concat_path_spaces() {
        let path = Path::new("/tmp/my video file.mp4");
        assert_eq!(escape_concat_path(path), "/tmp/my video file.mp4");
    }

    #[tokio::test]
    async fn test_random_filename() {
        let name1 = random_filename().await;
        let name2 = random_filename().await;
        assert!(!name1.is_empty());
        assert!(!name2.is_empty());
        // Should be hex strings
        assert!(name1.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(name2.chars().all(|c| c.is_ascii_hexdigit()));
    }
}

fn build_transition_filter_complex(
    durations: &[f64],
    transition_type: &str,
    transition_duration: f64,
) -> String {
    let n = durations.len();
    let mut parts = Vec::with_capacity((n.saturating_sub(1)) * 2);

    for i in 0..(n.saturating_sub(1)) {
        let is_last = i + 2 == n;
        let offset = if i == 0 {
            durations[0] - transition_duration
        } else {
            durations.iter().take(i + 1).sum::<f64>() - (i as f64 + 1.0) * transition_duration
        };
        let input_left = if i == 0 {
            "[0:v]".to_string()
        } else {
            format!("[v{i}]")
        };
        let output_label = if is_last {
            "[outv]".to_string()
        } else {
            format!("[v{}]", i + 1)
        };
        parts.push(format!(
            "{input_left}[{}:v]xfade=transition={transition_type}:duration={transition_duration}:offset={offset}{output_label}",
            i + 1
        ));
    }

    for i in 0..(n.saturating_sub(1)) {
        let is_last = i + 2 == n;
        let input_left = if i == 0 {
            "[0:a]".to_string()
        } else {
            format!("[a{i}]")
        };
        let output_label = if is_last {
            "[outa]".to_string()
        } else {
            format!("[a{}]", i + 1)
        };
        parts.push(format!(
            "{input_left}[{}:a]acrossfade=d={transition_duration}{output_label}",
            i + 1
        ));
    }

    parts.join(";")
}

pub async fn concat_videos(
    reporter: Option<&impl ProgressReporterTrait>,
    videos: &[PathBuf],
    output_path: &Path,
) -> Result<(), String> {
    concat_videos_with_transition(reporter, videos, output_path, None).await
}

/// Concatenate videos with optional transition effects
pub async fn concat_videos_with_transition(
    reporter: Option<&impl ProgressReporterTrait>,
    videos: &[PathBuf],
    output_path: &Path,
    transition: Option<&str>,
) -> Result<(), String> {
    let mut ffmpeg_process = ffmpeg_command();
    #[cfg(target_os = "windows")]
    ffmpeg_process.creation_flags(CREATE_NO_WINDOW);

    let output_folder = output_path.parent().ok_or_else(|| {
        format!(
            "Invalid output path (no parent directory): {}",
            output_path.display()
        )
    })?;
    if !output_folder.exists() {
        std::fs::create_dir_all(output_folder).map_err(|e| {
            format!(
                "Failed to create output folder '{}': {e}",
                output_folder.display()
            )
        })?;
    }

    // If no transition or only one video, use simple concat
    if transition.is_none() || transition == Some("none") || videos.len() == 1 {
        let filelist_filename = format!("filelist_{}.txt", random_filename().await);

        let mut filelist = tokio::fs::File::create(&output_folder.join(&filelist_filename))
            .await
            .map_err(|e| format!("Failed to create filelist: {e}"))?;
        for video in videos {
            let abs_path = tokio::fs::canonicalize(video).await.unwrap_or_else(|e| {
                log::warn!("Failed to canonicalize path {}: {e}", video.display());
                video.to_path_buf()
            });
            let escaped_path = escape_concat_path(&abs_path);
            filelist
                .write_all(format!("file '{}'\n", escaped_path).as_bytes())
                .await
                .map_err(|e| format!("Failed to write to filelist: {e}"))?;
        }
        filelist
            .flush()
            .await
            .map_err(|e| format!("Failed to flush filelist: {e}"))?;

        // Convert &[PathBuf] to &[&Path] for check_videos
        let video_refs: Vec<&Path> = videos.iter().map(|p| p.as_path()).collect();
        let should_encode = !super::check_videos(&video_refs).await;

        let filelist_path = output_folder.join(&filelist_filename);
        ffmpeg_process.args([
            "-f",
            "concat",
            "-safe",
            "0",
            "-i",
            filelist_path.to_str().ok_or_else(|| {
                format!(
                    "Invalid filelist path (non-UTF8): {}",
                    filelist_path.display()
                )
            })?,
        ]);
        if should_encode {
            let video_encoder = hwaccel::get_x264_encoder().await;
            hwaccel::apply_x264_encoder_args(
                &mut ffmpeg_process,
                video_encoder,
                Some(hwaccel::H264_SCALE_PAD_FILTER),
            );
            ffmpeg_process.args(["-c:a", "aac"]);
            hwaccel::apply_x264_quality_args(&mut ffmpeg_process, video_encoder);
            ffmpeg_process.args(["-threads", "0"]);
        } else {
            ffmpeg_process.args(["-c", "copy"]);
        }
        ffmpeg_process.args([output_path.to_str().ok_or_else(|| {
            format!("Invalid output path (non-UTF8): {}", output_path.display())
        })?]);
        ffmpeg_process.args(["-progress", "pipe:2"]);
        ffmpeg_process.args(["-y"]);

        handle_ffmpeg_process(reporter, &mut ffmpeg_process).await?;

        // clean up filelist
        let _ = tokio::fs::remove_file(output_folder.join(&filelist_filename)).await;
    } else {
        // Use xfade + acrossfade so video and audio share the same overlap timeline
        let transition_duration = super::TRANSITION_DURATION_SECS;
        // At this point we know transition is Some and not "none"
        let transition_type = transition.unwrap_or("fade");

        // Get video durations
        let mut durations = Vec::new();
        for video in videos {
            let metadata = super::extract_video_metadata(video).await?;
            durations.push(metadata.duration);
        }

        // Add all input files
        for video in videos {
            ffmpeg_process.args([
                "-i",
                video
                    .to_str()
                    .ok_or_else(|| format!("Invalid video path (non-UTF8): {}", video.display()))?,
            ]);
        }

        let filter_complex =
            build_transition_filter_complex(&durations, transition_type, transition_duration);

        let video_encoder = hwaccel::get_x264_encoder().await;

        if hwaccel::is_vaapi_encoder(video_encoder) {
            let filter_complex = format!(
                "{filter_complex};[outv]{}[outv_hw]",
                hwaccel::vaapi_filter_suffix()
            );
            ffmpeg_process.args(["-filter_complex", filter_complex.as_str()]);
            ffmpeg_process.args(["-map", "[outv_hw]"]);
        } else {
            ffmpeg_process.args(["-filter_complex", &filter_complex]);
            ffmpeg_process.args(["-map", "[outv]"]);
        }
        ffmpeg_process.args(["-map", "[outa]"]);

        hwaccel::apply_x264_encoder_only(&mut ffmpeg_process, video_encoder);
        hwaccel::apply_x264_quality_args(&mut ffmpeg_process, video_encoder);
        ffmpeg_process.args(["-c:a", "aac"]);
        ffmpeg_process.args(["-progress", "pipe:2"]);
        ffmpeg_process.args(["-y"]);
        ffmpeg_process.args([output_path.to_str().ok_or("Invalid output path")?]);

        handle_ffmpeg_process(reporter, &mut ffmpeg_process).await?;
    }

    Ok(())
}

#[cfg(test)]
mod transition_filter_tests {
    use super::build_transition_filter_complex;

    #[test]
    fn two_clips_use_matching_xfade_and_acrossfade() {
        let filter = build_transition_filter_complex(&[8.0, 6.0], "dissolve", 1.0);
        assert!(
            filter.contains("[0:v][1:v]xfade=transition=dissolve:duration=1:offset=7[outv]"),
            "unexpected video filter: {filter}"
        );
        assert!(
            filter.contains("[0:a][1:a]acrossfade=d=1[outa]"),
            "audio should acrossfade to match xfade: {filter}"
        );
        assert!(
            !filter.contains("concat=n="),
            "audio concat would desync from xfade: {filter}"
        );
    }

    #[test]
    fn three_clips_accumulate_overlap_on_video_and_audio() {
        let filter = build_transition_filter_complex(&[10.0, 10.0, 10.0], "fade", 1.0);
        assert!(
            filter.contains("[0:v][1:v]xfade=transition=fade:duration=1:offset=9[v1]"),
            "unexpected first xfade: {filter}"
        );
        assert!(
            filter.contains("[v1][2:v]xfade=transition=fade:duration=1:offset=18[outv]"),
            "unexpected second xfade: {filter}"
        );
        assert!(
            filter.contains("[0:a][1:a]acrossfade=d=1[a1]"),
            "unexpected first acrossfade: {filter}"
        );
        assert!(
            filter.contains("[a1][2:a]acrossfade=d=1[outa]"),
            "unexpected second acrossfade: {filter}"
        );
    }
}

#[cfg(test)]
mod concat_videos_tests {
    use super::{concat_videos, concat_videos_with_transition, random_filename};
    use crate::ffmpeg::ffmpeg_path;
    use std::path::Path;

    #[cfg(target_os = "windows")]
    use super::CREATE_NO_WINDOW;

    /// Helper function to create a minimal valid MP4 file for testing
    async fn create_test_video(path: &Path, duration_secs: u32) -> Result<(), String> {
        // Create parent directory if it doesn't exist
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("Failed to create parent dir: {}", e))?;
        }

        // Use FFmpeg to generate a test video with color and audio
        let mut cmd = tokio::process::Command::new(ffmpeg_path());
        #[cfg(target_os = "windows")]
        cmd.creation_flags(CREATE_NO_WINDOW);

        cmd.args([
            "-f",
            "lavfi",
            "-i",
            &format!("color=c=blue:s=1280x720:d={}", duration_secs),
            "-f",
            "lavfi",
            "-i",
            &format!("anullsrc=r=44100:cl=stereo:d={}", duration_secs),
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-c:a",
            "aac",
            "-y",
            path.to_str().unwrap(),
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

        Ok(())
    }

    #[tokio::test]
    async fn test_concat_videos_with_chinese_and_brackets() {
        // Create a temporary directory for testing
        let temp_dir = std::env::temp_dir().join(format!("bili_test_{}", random_filename().await));
        tokio::fs::create_dir_all(&temp_dir).await.unwrap();

        // Create test videos with problematic filenames (Chinese characters and brackets)
        let video1_path =
            temp_dir.join("[22637261][1773410394380][电台久违的嚼嚼嚼][2026-03-14_11-11-41].0.mp4");
        let video2_path =
            temp_dir.join("[22637261][1773410394380][电台久违的嚼嚼嚼][2026-03-14_11-11-41].1.mp4");
        let output_path =
            temp_dir.join("[22637261][1773410394380][电台久违的嚼嚼嚼][2026-03-14_11-11-41].mp4");

        // Create test videos
        create_test_video(&video1_path, 2).await.unwrap();
        create_test_video(&video2_path, 2).await.unwrap();

        // Test concatenation
        let videos = vec![video1_path.clone(), video2_path.clone()];
        let result = concat_videos(
            None::<&crate::progress::progress_reporter::ProgressReporter>,
            &videos,
            &output_path,
        )
        .await;

        // Clean up
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;

        // Assert
        assert!(result.is_ok(), "Concat should succeed: {:?}", result);
    }

    #[tokio::test]
    async fn test_concat_videos_with_spaces_and_special_chars() {
        let temp_dir = std::env::temp_dir().join(format!("bili_test_{}", random_filename().await));
        tokio::fs::create_dir_all(&temp_dir).await.unwrap();

        let video1_path = temp_dir.join("video with spaces [1].mp4");
        let video2_path = temp_dir.join("video's file [2].mp4");
        let output_path = temp_dir.join("output [final].mp4");

        create_test_video(&video1_path, 2).await.unwrap();
        create_test_video(&video2_path, 2).await.unwrap();

        let videos = vec![video1_path, video2_path];
        let result = concat_videos(
            None::<&crate::progress::progress_reporter::ProgressReporter>,
            &videos,
            &output_path,
        )
        .await;

        let _ = tokio::fs::remove_dir_all(&temp_dir).await;

        assert!(result.is_ok(), "Concat should succeed: {:?}", result);
    }

    async fn probe_stream_duration(path: &Path, codec_type: &str) -> f64 {
        let output = tokio::process::Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-select_streams",
                codec_type,
                "-show_entries",
                "stream=duration",
                "-of",
                "default=noprint_wrappers=1:nokey=1",
                path.to_str().unwrap(),
            ])
            .output()
            .await
            .expect("ffprobe");
        String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse::<f64>()
            .unwrap_or(0.0)
    }

    #[tokio::test]
    async fn concat_with_transition_keeps_audio_in_sync_with_video() {
        let temp_dir = std::env::temp_dir().join(format!("bili_test_{}", random_filename().await));
        tokio::fs::create_dir_all(&temp_dir).await.unwrap();

        let video1_path = temp_dir.join("a.mp4");
        let video2_path = temp_dir.join("b.mp4");
        let video3_path = temp_dir.join("c.mp4");
        let output_path = temp_dir.join("out.mp4");

        create_test_video(&video1_path, 3).await.unwrap();
        create_test_video(&video2_path, 3).await.unwrap();
        create_test_video(&video3_path, 3).await.unwrap();

        let result = concat_videos_with_transition(
            None::<&crate::progress::progress_reporter::ProgressReporter>,
            &[video1_path, video2_path, video3_path],
            &output_path,
            Some("fade"),
        )
        .await;

        let video_duration = probe_stream_duration(&output_path, "v:0").await;
        let audio_duration = probe_stream_duration(&output_path, "a:0").await;
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;

        assert!(
            result.is_ok(),
            "transition concat should succeed: {:?}",
            result
        );
        // 3+3+3 minus two 1s overlaps ≈ 7s, not the 9s of naive audio concat
        assert!(
            (video_duration - 7.0).abs() < 0.3,
            "video duration {video_duration} should match xfade timeline"
        );
        assert!(
            (audio_duration - video_duration).abs() < 0.3,
            "audio duration {audio_duration} drifted from video {video_duration}"
        );
    }
}
