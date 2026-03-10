use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

use async_ffmpeg_sidecar::{event::FfmpegEvent, log_parser::FfmpegLogParser};
use tokio::io::{AsyncWriteExt, BufReader};

use crate::{ffmpeg::hwaccel, progress::progress_reporter::ProgressReporterTrait};

use super::ffmpeg_path;

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
    // Only escape backslashes and single quotes
    // Do NOT escape square brackets - they work fine in single-quoted paths
    path_str.replace('\\', "\\\\").replace('\'', "'\\''")
}

pub async fn handle_ffmpeg_process(
    reporter: Option<&impl ProgressReporterTrait>,
    ffmpeg_process: &mut tokio::process::Command,
) -> Result<(), String> {
    log::info!("[FFmpeg] {:?}", ffmpeg_process);
    let child = ffmpeg_process
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn();
    if let Err(e) = child {
        return Err(e.to_string());
    }
    let mut child = child.unwrap();
    let stderr = child.stderr.take().unwrap();
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
    let mut ffmpeg_process = tokio::process::Command::new(ffmpeg_path());
    #[cfg(target_os = "windows")]
    ffmpeg_process.creation_flags(CREATE_NO_WINDOW);

    let output_folder = output_path.parent().unwrap();
    if !output_folder.exists() {
        std::fs::create_dir_all(output_folder).unwrap();
    }

    // If no transition or only one video, use simple concat
    if transition.is_none() || transition == Some("none") || videos.len() == 1 {
        let filelist_filename = format!("filelist_{}.txt", random_filename().await);

        let mut filelist = tokio::fs::File::create(&output_folder.join(&filelist_filename))
            .await
            .unwrap();
        for video in videos {
            let abs_path = std::fs::canonicalize(video).unwrap_or_else(|_| video.to_path_buf());
            let escaped_path = escape_concat_path(&abs_path);
            filelist
                .write_all(format!("file '{}'\n", escaped_path).as_bytes())
                .await
                .unwrap();
        }
        filelist.flush().await.unwrap();

        // Convert &[PathBuf] to &[&Path] for check_videos
        let video_refs: Vec<&Path> = videos.iter().map(|p| p.as_path()).collect();
        let should_encode = !super::check_videos(&video_refs).await;

        ffmpeg_process.args([
            "-f",
            "concat",
            "-safe",
            "0",
            "-i",
            output_folder.join(&filelist_filename).to_str().unwrap(),
        ]);
        if should_encode {
            let video_encoder = hwaccel::get_x264_encoder().await;
            ffmpeg_process.args(["-vf", "scale=1920:1080:force_original_aspect_ratio=decrease,pad=1920:1080:(ow-iw)/2:(oh-ih)/2"]);
            ffmpeg_process.args(["-r", "60"]);
            ffmpeg_process.args(["-c:v", video_encoder]);
            ffmpeg_process.args(["-c:a", "aac"]);
            ffmpeg_process.args(["-b:v", "6000k"]);
            ffmpeg_process.args(["-b:a", "128k"]);
            ffmpeg_process.args(["-threads", "0"]);
        } else {
            ffmpeg_process.args(["-c", "copy"]);
        }
        ffmpeg_process.args([output_path.to_str().unwrap()]);
        ffmpeg_process.args(["-progress", "pipe:2"]);
        ffmpeg_process.args(["-y"]);

        handle_ffmpeg_process(reporter, &mut ffmpeg_process).await?;

        // clean up filelist
        let _ = tokio::fs::remove_file(output_folder.join(&filelist_filename)).await;
    } else {
        // Use xfade filter for transitions
        let transition_duration = 1.0;
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
            ffmpeg_process.args(["-i", video.to_str().unwrap()]);
        }

        // Build xfade filter chain for video
        let mut filter_complex = String::new();

        for i in 0..(videos.len() - 1) {
            let is_last = i == videos.len() - 2;
            let offset = if i == 0 {
                durations[0] - transition_duration
            } else {
                durations.iter().take(i + 1).sum::<f64>() - (i as f64 + 1.0) * transition_duration
            };
            let input_left = if i == 0 {
                "[0:v]".to_string()
            } else {
                format!("[v{}]", i)
            };
            let output_label = if is_last {
                "[outv]".to_string()
            } else {
                format!("[v{}]", i + 1)
            };
            filter_complex.push_str(&format!(
                "{}[{}:v]xfade=transition={}:duration={}:offset={}{};",
                input_left,
                i + 1,
                transition_type,
                transition_duration,
                offset,
                output_label
            ));
        }

        // Build audio concat filter to merge all audio streams
        for i in 0..videos.len() {
            filter_complex.push_str(&format!("[{}:a]", i));
        }
        filter_complex.push_str(&format!("concat=n={}:v=0:a=1[outa]", videos.len()));

        ffmpeg_process.args(["-filter_complex", &filter_complex]);
        ffmpeg_process.args(["-map", "[outv]"]);
        ffmpeg_process.args(["-map", "[outa]"]);

        let video_encoder = hwaccel::get_x264_encoder().await;
        ffmpeg_process.args(["-c:v", video_encoder]);
        ffmpeg_process.args(["-preset", "medium"]);
        ffmpeg_process.args(["-crf", "23"]);
        ffmpeg_process.args(["-c:a", "aac"]);
        ffmpeg_process.args(["-progress", "pipe:2"]);
        ffmpeg_process.args(["-y"]);
        ffmpeg_process.args([output_path.to_str().unwrap()]);

        handle_ffmpeg_process(reporter, &mut ffmpeg_process).await?;
    }

    Ok(())
}
