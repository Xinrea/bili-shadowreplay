use std::path::{Path, PathBuf};

use tempfile::Builder;
use tokio::io::AsyncWriteExt;

use crate::{
    ffmpeg::{
        hwaccel,
        runner::{run_command, ProgressMode},
    },
    progress::progress_reporter::ProgressReporterTrait,
};
use ffmpeg_utils::ffmpeg_command;

/// Generate a random filename in hex
#[cfg(test)]
async fn random_filename() -> String {
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
    ffmpeg_process: tokio::process::Command,
) -> Result<(), String> {
    log::info!("[FFmpeg] {:?}", ffmpeg_process);
    run_command(ffmpeg_process, ProgressMode::OutTimeMs, "FFmpeg", reporter)
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
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

/// Hard-cut concat at the frame level so each input's MP4 edit list is applied
/// before joining. Concat demuxer + stream copy ignores those edit lists and
/// dumps AAC priming/pre-roll packets onto the join, which players hear as a
/// stutter.
///
/// When `normalize` is true, video is scaled/padded to a common 1920x1080 canvas
/// and audio is resampled first. Concat requires matching resolution; sample
/// rate and layout are converted automatically, but `aresample` also absorbs
/// leftover timestamp gaps on mismatched clips.
fn build_hardcut_concat_filter_complex(n: usize, normalize: bool) -> String {
    if !normalize {
        let mut inputs = String::new();
        for i in 0..n {
            inputs.push_str(&format!("[{i}:v][{i}:a]"));
        }
        return format!("{inputs}concat=n={n}:v=1:a=1[outv][outa]");
    }

    let mut parts = Vec::with_capacity(n * 2 + 1);
    let mut concat_inputs = String::new();
    for i in 0..n {
        parts.push(format!("[{i}:v]{}[v{i}]", hwaccel::H264_SCALE_PAD_FILTER));
        parts.push(format!("[{i}:a]aresample=async=1:first_pts=0[a{i}]"));
        concat_inputs.push_str(&format!("[v{i}][a{i}]"));
    }
    parts.push(format!("{concat_inputs}concat=n={n}:v=1:a=1[outv][outa]"));
    parts.join(";")
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

fn ensure_output_folder(output_path: &Path) -> Result<&Path, String> {
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
    Ok(output_folder)
}

/// Concatenate full recordings with stream copy when codecs match.
///
/// This path is for already-muxed files (e.g. whole playlists), not copy-trimmed
/// clip ranges. Copy-trimmed MP4s must go through [`concat_videos_with_transition`]
/// so edit lists are applied.
pub async fn concat_videos(
    reporter: Option<&impl ProgressReporterTrait>,
    videos: &[PathBuf],
    output_path: &Path,
) -> Result<(), String> {
    concat_videos_with_demuxer(reporter, videos, output_path).await
}

async fn concat_videos_with_demuxer(
    reporter: Option<&impl ProgressReporterTrait>,
    videos: &[PathBuf],
    output_path: &Path,
) -> Result<(), String> {
    let mut ffmpeg_process = ffmpeg_command();

    let output_folder = ensure_output_folder(output_path)?;
    let filelist = Builder::new()
        .prefix(".bili-filelist-")
        .suffix(".txt")
        .tempfile_in(output_folder)
        .map_err(|e| format!("Failed to create filelist: {e}"))?
        .into_temp_path();

    let mut filelist_writer = tokio::fs::File::create(&filelist)
        .await
        .map_err(|e| format!("Failed to open filelist: {e}"))?;
    for video in videos {
        let abs_path = tokio::fs::canonicalize(video).await.unwrap_or_else(|e| {
            log::warn!("Failed to canonicalize path {}: {e}", video.display());
            video.to_path_buf()
        });
        let escaped_path = escape_concat_path(&abs_path);
        filelist_writer
            .write_all(format!("file '{}'\n", escaped_path).as_bytes())
            .await
            .map_err(|e| format!("Failed to write to filelist: {e}"))?;
    }
    filelist_writer
        .flush()
        .await
        .map_err(|e| format!("Failed to flush filelist: {e}"))?;

    let video_refs: Vec<&Path> = videos.iter().map(|p| p.as_path()).collect();
    let should_encode = !super::check_videos(&video_refs).await;

    ffmpeg_process.args([
        "-f",
        "concat",
        "-safe",
        "0",
        "-i",
        filelist
            .to_str()
            .ok_or_else(|| format!("Invalid filelist path (non-UTF8): {}", filelist.display()))?,
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
    ffmpeg_process.args([output_path
        .to_str()
        .ok_or_else(|| format!("Invalid output path (non-UTF8): {}", output_path.display()))?]);
    ffmpeg_process.args(["-y"]);

    handle_ffmpeg_process(reporter, ffmpeg_process).await
}

async fn encode_filter_concat(
    reporter: Option<&impl ProgressReporterTrait>,
    videos: &[PathBuf],
    output_path: &Path,
    filter_complex: String,
) -> Result<(), String> {
    let mut ffmpeg_process = ffmpeg_command();

    ensure_output_folder(output_path)?;

    for video in videos {
        ffmpeg_process.args([
            "-i",
            video
                .to_str()
                .ok_or_else(|| format!("Invalid video path (non-UTF8): {}", video.display()))?,
        ]);
    }

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
    ffmpeg_process.args(["-y"]);
    ffmpeg_process.args([output_path
        .to_str()
        .ok_or_else(|| format!("Invalid output path (non-UTF8): {}", output_path.display()))?]);

    handle_ffmpeg_process(reporter, ffmpeg_process).await
}

/// Concatenate videos with optional transition effects
pub async fn concat_videos_with_transition(
    reporter: Option<&impl ProgressReporterTrait>,
    videos: &[PathBuf],
    output_path: &Path,
    transition: Option<&str>,
) -> Result<(), String> {
    if videos.is_empty() {
        return Err("No videos to concatenate".to_string());
    }

    // Single file: remux/copy, no join to fix
    if videos.len() == 1 {
        return concat_videos_with_demuxer(reporter, videos, output_path).await;
    }

    // No visual transition: still re-encode via concat filter so each clip's
    // MP4 edit list (AAC pre-roll from `-c copy` trim) is applied per input.
    // Concat demuxer + `-c copy` would dump those packets at every join.
    if transition.is_none() || transition == Some("none") {
        let video_refs: Vec<&Path> = videos.iter().map(|p| p.as_path()).collect();
        let normalize = !super::check_videos(&video_refs).await;
        let filter_complex = build_hardcut_concat_filter_complex(videos.len(), normalize);
        return encode_filter_concat(reporter, videos, output_path, filter_complex).await;
    }

    let transition_duration = super::TRANSITION_DURATION_SECS;
    let transition_type = transition.unwrap_or("fade");

    let mut durations = Vec::new();
    for video in videos {
        let metadata = ffmpeg_utils::extract_video_metadata(video).await?;
        durations.push(metadata.duration);
    }

    let filter_complex =
        build_transition_filter_complex(&durations, transition_type, transition_duration);
    encode_filter_concat(reporter, videos, output_path, filter_complex).await
}

#[cfg(test)]
mod transition_filter_tests {
    use super::{build_hardcut_concat_filter_complex, build_transition_filter_complex};

    #[test]
    fn hardcut_concat_maps_all_streams_in_order() {
        assert_eq!(
            build_hardcut_concat_filter_complex(2, false),
            "[0:v][0:a][1:v][1:a]concat=n=2:v=1:a=1[outv][outa]"
        );
        assert_eq!(
            build_hardcut_concat_filter_complex(3, false),
            "[0:v][0:a][1:v][1:a][2:v][2:a]concat=n=3:v=1:a=1[outv][outa]"
        );
    }

    #[test]
    fn hardcut_concat_normalizes_mismatched_inputs() {
        let filter = build_hardcut_concat_filter_complex(2, true);
        assert!(
            filter.contains("[0:v]scale=1920:1080:force_original_aspect_ratio=decrease,pad=1920:1080:(ow-iw)/2:(oh-ih)/2[v0]"),
            "video 0 should scale/pad: {filter}"
        );
        assert!(
            filter.contains("[1:v]scale=1920:1080:force_original_aspect_ratio=decrease,pad=1920:1080:(ow-iw)/2:(oh-ih)/2[v1]"),
            "video 1 should scale/pad: {filter}"
        );
        assert!(
            filter.contains("[0:a]aresample=async=1:first_pts=0[a0]"),
            "audio 0 should resample: {filter}"
        );
        assert!(
            filter.contains("[v0][a0][v1][a1]concat=n=2:v=1:a=1[outv][outa]"),
            "normalized streams should concat: {filter}"
        );
    }

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
    use super::{concat_videos, concat_videos_with_transition, ffmpeg_command, random_filename};
    use ffmpeg_utils::ffprobe_command;
    use std::path::Path;

    /// Helper function to create a minimal valid MP4 file for testing
    async fn create_test_video(path: &Path, duration_secs: u32) -> Result<(), String> {
        create_test_video_with_size(path, duration_secs, "1280x720").await
    }

    async fn create_test_video_with_size(
        path: &Path,
        duration_secs: u32,
        size: &str,
    ) -> Result<(), String> {
        // Create parent directory if it doesn't exist
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("Failed to create parent dir: {}", e))?;
        }

        // Use FFmpeg to generate a test video with color and audio
        let mut cmd = ffmpeg_command();

        cmd.args([
            "-f",
            "lavfi",
            "-i",
            &format!("color=c=blue:s={size}:d={duration_secs}"),
            "-f",
            "lavfi",
            "-i",
            &format!("anullsrc=r=44100:cl=stereo:d={duration_secs}"),
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
        let output = ffprobe_command()
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

    /// Source matching the clip pipeline: H.264 with a 1s GOP plus AAC, so
    /// `-c copy` trim writes an MP4 edit list instead of a clean cut.
    async fn create_gop_tone_video(path: &Path, duration_secs: u32) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("Failed to create parent dir: {e}"))?;
        }

        let mut cmd = ffmpeg_command();

        cmd.args([
            "-f",
            "lavfi",
            "-i",
            &format!("color=c=blue:s=1280x720:r=30:d={duration_secs}"),
            "-f",
            "lavfi",
            "-i",
            &format!("sine=frequency=440:sample_rate=44100:duration={duration_secs}"),
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-g",
            "30",
            "-keyint_min",
            "30",
            "-sc_threshold",
            "0",
            "-c:a",
            "aac",
            "-ar",
            "44100",
            "-ac",
            "2",
            "-y",
            path.to_str().unwrap(),
        ]);

        let output = cmd
            .output()
            .await
            .map_err(|e| format!("Failed to run ffmpeg: {e}"))?;
        if !output.status.success() {
            return Err(format!(
                "FFmpeg failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        Ok(())
    }

    fn audio_packet_durations(path: &Path) -> Vec<f64> {
        let output = std::process::Command::new(ffmpeg_utils::ffprobe_path())
            .args([
                "-v",
                "error",
                "-select_streams",
                "a:0",
                "-show_packets",
                "-show_entries",
                "packet=duration_time",
                "-of",
                "csv=p=0",
                path.to_str().unwrap(),
            ])
            .output()
            .expect("ffprobe packets");
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| line.split(',').next()?.parse::<f64>().ok())
            .collect()
    }

    async fn decoded_audio_duration(path: &Path) -> f64 {
        let pcm_path = path.with_extension("pcm");
        let mut cmd = ffmpeg_command();
        cmd.args([
            "-i",
            path.to_str().unwrap(),
            "-vn",
            "-ac",
            "1",
            "-ar",
            "44100",
            "-f",
            "f32le",
            "-y",
            pcm_path.to_str().unwrap(),
        ]);
        let output = cmd.output().await.expect("decode pcm");
        assert!(
            output.status.success(),
            "pcm decode failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let bytes = tokio::fs::metadata(&pcm_path)
            .await
            .expect("pcm size")
            .len();
        let _ = tokio::fs::remove_file(&pcm_path).await;
        bytes as f64 / 4.0 / 44100.0
    }

    #[tokio::test]
    async fn concat_without_transition_does_not_dump_edit_list_audio_at_join() {
        let temp_dir = std::env::temp_dir().join(format!("bili_test_{}", random_filename().await));
        tokio::fs::create_dir_all(&temp_dir).await.unwrap();

        let source_path = temp_dir.join("source.mp4");
        let clip0_path = temp_dir.join("clip0.mp4");
        let clip1_path = temp_dir.join("clip1.mp4");
        let output_path = temp_dir.join("out.mp4");

        create_gop_tone_video(&source_path, 12).await.unwrap();
        crate::ffmpeg::trim_video(
            None::<&crate::progress::progress_reporter::ProgressReporter>,
            &source_path,
            &clip0_path,
            1.37,
            3.11,
        )
        .await
        .unwrap();
        crate::ffmpeg::trim_video(
            None::<&crate::progress::progress_reporter::ProgressReporter>,
            &source_path,
            &clip1_path,
            6.83,
            3.07,
        )
        .await
        .unwrap();

        let result = concat_videos_with_transition(
            None::<&crate::progress::progress_reporter::ProgressReporter>,
            &[clip0_path, clip1_path],
            &output_path,
            None,
        )
        .await;
        assert!(
            result.is_ok(),
            "hard-cut concat should succeed: {:?}",
            result
        );

        let video_duration = probe_stream_duration(&output_path, "v:0").await;
        let audio_duration = probe_stream_duration(&output_path, "a:0").await;
        let tiny_packets = audio_packet_durations(&output_path)
            .into_iter()
            .filter(|d| *d < 0.001)
            .count();
        let decoded_duration = decoded_audio_duration(&output_path).await;
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
        assert_eq!(
            tiny_packets, 0,
            "copy-trim edit lists must not produce 1-sample audio packets at the join"
        );
        assert!(
            (audio_duration - video_duration).abs() < 0.15,
            "audio duration {audio_duration} drifted from video {video_duration}"
        );
        assert!(
            (decoded_duration - audio_duration).abs() < 0.15,
            "decoded audio {decoded_duration}s must match container {audio_duration}s (edit-list pre-roll was dumped)"
        );
    }

    #[tokio::test]
    async fn concat_without_transition_normalizes_mismatched_resolutions() {
        let temp_dir = std::env::temp_dir().join(format!("bili_test_{}", random_filename().await));
        tokio::fs::create_dir_all(&temp_dir).await.unwrap();

        let video1_path = temp_dir.join("small.mp4");
        let video2_path = temp_dir.join("large.mp4");
        let output_path = temp_dir.join("out.mp4");

        create_test_video_with_size(&video1_path, 2, "640x360")
            .await
            .unwrap();
        create_test_video_with_size(&video2_path, 2, "1280x720")
            .await
            .unwrap();

        let result = concat_videos_with_transition(
            None::<&crate::progress::progress_reporter::ProgressReporter>,
            &[video1_path, video2_path],
            &output_path,
            None,
        )
        .await;
        assert!(
            result.is_ok(),
            "mismatched hard-cut concat should succeed: {:?}",
            result
        );

        let metadata = ffmpeg_utils::extract_video_metadata(&output_path)
            .await
            .expect("output metadata");
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
        assert_eq!(metadata.width, 1920);
        assert_eq!(metadata.height, 1080);
    }
}
