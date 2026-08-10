use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Stdio;

pub mod general;
pub mod hwaccel;
pub mod playlist;

use crate::constants;
use crate::progress::progress_reporter::{ProgressReporter, ProgressReporterTrait};
use crate::subtitle_generator::{powerlive, whisper_online};
use crate::subtitle_generator::{
    whisper_cpp, GenerateResult, SubtitleGenerator, SubtitleGeneratorType,
};
use async_ffmpeg_sidecar::event::{FfmpegEvent, LogLevel};
use async_ffmpeg_sidecar::log_parser::FfmpegLogParser;
use serde::{Deserialize, Serialize};
use tokio::io::BufReader;

// 视频元数据结构
#[derive(Debug, Clone, PartialEq)]
pub struct VideoMetadata {
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub video_codec: String,
    pub audio_codec: String,
}

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;
#[cfg(target_os = "windows")]
#[allow(unused_imports)]
use std::os::windows::process::CommandExt;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Range {
    pub start: f64,
    pub end: f64,
}

impl fmt::Display for Range {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}, {}]", self.start, self.end)
    }
}

impl Range {
    pub fn duration(&self) -> f64 {
        self.end - self.start
    }

    pub fn is_in(&self, v: f64) -> bool {
        v >= self.start && v <= self.end
    }
}

pub async fn transcode(
    reporter: Option<&impl ProgressReporterTrait>,
    file: &Path,
    output_path: &Path,
    copy_codecs: bool,
) -> Result<(), String> {
    // ffmpeg -i fixed_\[30655190\]1742887114_0325084106_81.5.mp4 -c:v libx264 -c:a aac -b:v 6000k -b:a 64k -compression_level 0 -threads 0 output.mp3
    log::info!("Transcode: {} copy: {}", file.display(), copy_codecs);
    let mut ffmpeg_process = ffmpeg_command();
    #[cfg(target_os = "windows")]
    ffmpeg_process.creation_flags(CREATE_NO_WINDOW);

    ffmpeg_process.args(["-i", file.to_str().unwrap()]);

    if copy_codecs {
        ffmpeg_process.args(["-c:v", "copy"]).args(["-c:a", "copy"]);
    } else {
        let video_encoder = hwaccel::get_x264_encoder().await;
        hwaccel::apply_x264_encoder_args(
            &mut ffmpeg_process,
            video_encoder,
            Some(hwaccel::H264_SCALE_PAD_FILTER),
        );
        ffmpeg_process.args(["-c:a", "aac"]);
        hwaccel::apply_x264_quality_args(&mut ffmpeg_process, video_encoder);
        ffmpeg_process.args(["-threads", "0"]);
    }

    let child = ffmpeg_process
        .args([output_path.to_str().unwrap()])
        .args(["-y"])
        .args(["-progress", "pipe:2"])
        .stderr(Stdio::piped())
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
            FfmpegEvent::Progress(p) => {
                if reporter.is_none() {
                    continue;
                }
                reporter
                    .unwrap()
                    .update(format!("压制中：{}", p.time).as_str())
                    .await;
            }
            FfmpegEvent::LogEOF => break,
            FfmpegEvent::Error(e) => {
                log::error!("Transcode error: {e}");
                return Err(e.to_string());
            }
            _ => {}
        }
    }

    if let Err(e) = child.wait().await {
        return Err(e.to_string());
    }

    Ok(())
}

pub async fn trim_video(
    reporter: Option<&impl ProgressReporterTrait>,
    file: &Path,
    output_path: &Path,
    start_time: f64,
    duration: f64,
) -> Result<(), String> {
    // ffmpeg -i fixed_\[30655190\]1742887114_0325084106_81.5.mp4 -ss 0 -t 10 output.mp4
    log::info!("Trim video task start: {}", file.display());
    let mut ffmpeg_process = ffmpeg_command();
    #[cfg(target_os = "windows")]
    ffmpeg_process.creation_flags(CREATE_NO_WINDOW);

    ffmpeg_process.args(["-ss", &start_time.to_string()]);
    ffmpeg_process.args(["-i", file.to_str().unwrap()]);
    ffmpeg_process.args(["-t", &duration.to_string()]);
    ffmpeg_process.args(["-c", "copy"]);
    ffmpeg_process.args([output_path.to_str().unwrap()]);
    ffmpeg_process.args(["-y"]);
    ffmpeg_process.args(["-progress", "pipe:2"]);
    ffmpeg_process.stderr(Stdio::piped());
    let child = ffmpeg_process.spawn();
    if let Err(e) = child {
        return Err(e.to_string());
    }

    let mut child = child.unwrap();
    let stderr = child.stderr.take().unwrap();
    let reader = BufReader::new(stderr);
    let mut parser = FfmpegLogParser::new(reader);

    while let Ok(event) = parser.parse_next_event().await {
        match event {
            FfmpegEvent::Progress(p) => {
                if reporter.is_none() {
                    continue;
                }
                reporter
                    .unwrap()
                    .update(format!("切片中：{}", p.time).as_str())
                    .await;
            }
            FfmpegEvent::LogEOF => break,
            FfmpegEvent::Error(e) => {
                log::error!("Trim video error: {e}");
                return Err(e.to_string());
            }
            _ => {}
        }
    }

    if let Err(e) = child.wait().await {
        log::error!("Trim video error: {e}");
        return Err(e.to_string());
    }

    log::info!("Trim video task end: {}", output_path.display());
    Ok(())
}

/// Extract a sample audio from the video file for waveform display
pub async fn extract_audio_sample(file: &Path) -> Result<PathBuf, String> {
    // ffmpeg -i fixed_\[30655592\]1742887114_0325084106_81.5.mp4 -ar 16000 test.wav
    log::info!("Extract audio sample task start: {}", file.display());
    let output_path = file.with_extension("opus");
    let mut extract_error = None;

    let mut ffmpeg_process = ffmpeg_command();
    #[cfg(target_os = "windows")]
    ffmpeg_process.creation_flags(CREATE_NO_WINDOW);
    ffmpeg_process.kill_on_drop(true);

    let child = ffmpeg_process
        .args(["-i", file.to_str().unwrap()])
        .args(["-c:a", "libopus"])
        .args(["-ar", "16000"])
        .args(["-ac", "1"])
        .args(["-vn"])
        .args(["-b:a", "64k"])
        .args(["-vbr", "on"])
        .args(["-compression_level", "10"])
        .args([output_path.to_str().unwrap()])
        .args(["-y"])
        .args(["-progress", "pipe:2"])
        .stderr(Stdio::piped())
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
            FfmpegEvent::Error(e) => {
                log::error!("Extract audio sample error: {e}");
                extract_error = Some(e.to_string());
            }
            FfmpegEvent::LogEOF => break,
            FfmpegEvent::Progress(p) => {
                log::info!("Extract audio sample progress: {}", p.time);
            }
            FfmpegEvent::Log(_level, _content) => {}
            _ => {}
        }
    }

    if let Err(e) = child.wait().await {
        log::error!("Extract audio sample error: {e}");
        return Err(e.to_string());
    }

    if let Some(error) = extract_error {
        log::error!("Extract audio sample error: {error}");
        Err(error)
    } else {
        log::info!("Extract audio sample task end: {}", output_path.display());
        Ok(output_path)
    }
}

/// Return the conservative byte estimate used by the submission preflight.
/// The estimate intentionally includes a small container margin so a clip at
/// the 5 MB boundary is not admitted only to be rejected after encoding.
pub fn estimate_submission_audio_size(duration_seconds: f64) -> u64 {
    let seconds = duration_seconds.max(0.0);
    (seconds * 192_000.0 / 8.0).ceil() as u64 + 4096
}

/// Encode the first audio stream for a joi-button submission.
///
/// This is deliberately separate from the Whisper waveform path above: the
/// upload contract is 192 kbps / 44.1 kHz and preserves the source channel
/// count. If the local ffmpeg lacks libmp3lame, AAC in an m4a container is the
/// documented fallback accepted by the server.
pub async fn extract_submission_audio(file: &Path, output_path: &Path) -> Result<PathBuf, String> {
    match encode_submission_audio(file, output_path, "libmp3lame").await {
        Ok(path) => Ok(path),
        Err(mp3_error) => {
            // A failed encoder may have created a partial file before ffmpeg
            // reported the error. Remove it before trying the fallback so a
            // failed submission can never leave an alternate artifact behind.
            let _ = tokio::fs::remove_file(output_path).await;
            let fallback_path = output_path.with_extension("m4a");
            match encode_submission_audio(file, &fallback_path, "aac").await {
                Ok(path) => Ok(path),
                Err(m4a_error) => {
                    let _ = tokio::fs::remove_file(output_path).await;
                    let _ = tokio::fs::remove_file(&fallback_path).await;
                    Err(format!(
                        "submission audio encoding failed (mp3: {mp3_error}; m4a: {m4a_error})"
                    ))
                }
            }
        }
    }
}

async fn encode_submission_audio(
    file: &Path,
    output_path: &Path,
    encoder: &str,
) -> Result<PathBuf, String> {
    let mut ffmpeg_process = ffmpeg_command();
    #[cfg(target_os = "windows")]
    ffmpeg_process.creation_flags(CREATE_NO_WINDOW);
    ffmpeg_process.kill_on_drop(true);

    let child = ffmpeg_process
        .args(["-i", file.to_str().ok_or("invalid input path")?])
        .args(["-map", "0:a:0", "-vn"])
        .args(["-c:a", encoder, "-b:a", "192k", "-ar", "44100"])
        .args(["-y", "-progress", "pipe:2"])
        .arg(output_path)
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to spawn ffmpeg: {e}"))?;

    let mut child = child;
    let stderr = child.stderr.take().ok_or("ffmpeg stderr unavailable")?;
    let reader = BufReader::new(stderr);
    let mut parser = FfmpegLogParser::new(reader);
    let mut parser_error = None;
    while let Ok(event) = parser.parse_next_event().await {
        match event {
            FfmpegEvent::Error(error) => parser_error = Some(error.to_string()),
            FfmpegEvent::LogEOF => break,
            FfmpegEvent::Progress(progress) => {
                log::debug!("Submission audio progress: {}", progress.time);
            }
            _ => {}
        }
    }

    let status = child
        .wait()
        .await
        .map_err(|e| format!("ffmpeg wait error: {e}"))?;
    if !status.success() {
        let _ = tokio::fs::remove_file(output_path).await;
        return Err(parser_error.unwrap_or_else(|| format!("ffmpeg exited with {status}")));
    }
    if !output_path.exists() {
        return Err("ffmpeg produced no submission audio".to_string());
    }
    Ok(output_path.to_path_buf())
}
pub async fn extract_audio_chunks(file: &Path, format: &str) -> Result<PathBuf, String> {
    // ffmpeg -i fixed_\[30655190\]1742887114_0325084106_81.5.mp4 -ar 16000 test.wav
    log::info!("Extract audio task start: {}", file.display());
    let output_path = file.with_extension(format);
    let mut extract_error = None;

    // Whisper consumes 16 kHz mono audio. Keep every upload path consistent
    // and avoid spending bitrate on a second channel.
    let sample_rate = "16000";

    // First, get the duration of the input file
    let duration = get_audio_duration(file).await?;
    log::info!("Audio duration: {duration} seconds");

    // Split into chunks of 30 seconds
    let chunk_duration = 30;
    let chunk_count = (duration as f64 / f64::from(chunk_duration)).ceil() as usize;
    log::info!("Splitting into {chunk_count} chunks of {chunk_duration} seconds each");

    // Create output directory for chunks
    let output_dir = output_path.parent().unwrap();
    let base_name = output_path.file_stem().unwrap().to_str().unwrap();
    let chunk_dir = output_dir.join(format!("{base_name}_chunks"));

    if !chunk_dir.exists() {
        std::fs::create_dir_all(&chunk_dir)
            .map_err(|e| format!("Failed to create chunk directory: {e}"))?;
    }

    // Use ffmpeg segment feature to split audio into chunks
    let segment_pattern = chunk_dir.join(format!("{base_name}_%03d.{format}"));

    // 构建优化的ffmpeg命令参数
    let file_str = file.to_str().unwrap();
    let chunk_duration_str = chunk_duration.to_string();
    let segment_pattern_str = segment_pattern.to_str().unwrap();

    let mut args = vec![
        "-i",
        file_str,
        "-ar",
        sample_rate,
        "-ac",
        "1",
        "-vn",
        "-f",
        "segment",
        "-segment_time",
        &chunk_duration_str,
        "-reset_timestamps",
        "1",
        "-y",
        "-progress",
        "pipe:2",
    ];

    // 根据格式添加优化的编码参数
    if format == "mp3" {
        args.extend_from_slice(&[
            "-c:a",
            "mp3",
            "-b:a",
            "64k", // 降低比特率以提高速度
            "-compression_level",
            "0", // 最快压缩
        ]);
    } else {
        args.extend_from_slice(&[
            "-c:a",
            "pcm_s16le", // 使用PCM编码，速度更快
        ]);
    }

    // 添加性能优化参数
    args.extend_from_slice(&[
        "-threads", "0", // 使用所有可用CPU核心
    ]);

    args.push(segment_pattern_str);

    let mut ffmpeg_process = ffmpeg_command();
    #[cfg(target_os = "windows")]
    ffmpeg_process.creation_flags(CREATE_NO_WINDOW);

    let child = ffmpeg_process.args(&args).stderr(Stdio::piped()).spawn();

    if let Err(e) = child {
        return Err(e.to_string());
    }

    let mut child = child.unwrap();
    let stderr = child.stderr.take().unwrap();
    let reader = BufReader::new(stderr);
    let mut parser = FfmpegLogParser::new(reader);

    while let Ok(event) = parser.parse_next_event().await {
        match event {
            FfmpegEvent::Error(e) => {
                log::error!("Extract audio error: {e}");
                extract_error = Some(e.to_string());
            }
            FfmpegEvent::LogEOF => break,
            FfmpegEvent::Log(_level, _content) => {}
            _ => {}
        }
    }

    if let Err(e) = child.wait().await {
        log::error!("Extract audio error: {e}");
        return Err(e.to_string());
    }

    if let Some(error) = extract_error {
        log::error!("Extract audio error: {error}");
        Err(error)
    } else {
        log::info!(
            "Extract audio task end: {} chunks created in {}",
            chunk_count,
            chunk_dir.display()
        );
        Ok(chunk_dir)
    }
}

/// Extract the full audio track as a single 16kHz mono WAV file.
/// Returns the path to the extracted WAV.
pub async fn extract_full_audio(file: &Path) -> Result<PathBuf, String> {
    log::info!("Extract full audio: {}", file.display());
    let output_path = file.with_extension("full.wav");

    let mut ffmpeg_process = ffmpeg_command();
    #[cfg(target_os = "windows")]
    ffmpeg_process.creation_flags(CREATE_NO_WINDOW);
    ffmpeg_process.kill_on_drop(true);

    let child = ffmpeg_process
        .arg("-i")
        .arg(file)
        .args(["-ar", "16000"])
        .args(["-ac", "1"]) // mono for VAD
        .args(["-c:a", "pcm_s16le"])
        .args(["-vn"])
        .args(["-y"])
        .args(["-progress", "pipe:2"])
        .arg(&output_path)
        .stderr(Stdio::piped())
        .spawn();

    let mut child = child.map_err(|e| format!("Failed to spawn ffmpeg: {e}"))?;
    let stderr = child.stderr.take().unwrap();
    let reader = BufReader::new(stderr);
    let mut parser = FfmpegLogParser::new(reader);

    while let Ok(event) = parser.parse_next_event().await {
        match event {
            FfmpegEvent::Error(e) => {
                log::error!("Extract full audio error: {e}");
            }
            FfmpegEvent::LogEOF => break,
            _ => {}
        }
    }

    let status = child
        .wait()
        .await
        .map_err(|e| format!("ffmpeg wait error: {e}"))?;

    if !status.success() {
        let _ = tokio::fs::remove_file(&output_path).await;
        return Err(format!("Full audio extraction failed with status {status}"));
    }

    if output_path.exists() {
        log::info!("Full audio extracted: {}", output_path.display());
        Ok(output_path)
    } else {
        Err("Full audio extraction failed: output file not found".to_string())
    }
}

/// Extract a time segment from a video as a 16kHz mono WAV file.
pub async fn extract_audio_segment(
    file: &Path,
    start_sec: f64,
    duration_sec: f64,
    output_path: &Path,
) -> Result<(), String> {
    let mut ffmpeg_process = ffmpeg_command();
    #[cfg(target_os = "windows")]
    ffmpeg_process.creation_flags(CREATE_NO_WINDOW);

    let child = ffmpeg_process
        .args(["-ss", &start_sec.to_string()])
        .arg("-i")
        .arg(file)
        .args(["-t", &duration_sec.to_string()])
        .args(["-ar", "16000"])
        .args(["-ac", "1"])
        .args(["-c:a", "pcm_s16le"])
        .args(["-vn"])
        .args(["-y"])
        .args(["-progress", "pipe:2"])
        .arg(output_path)
        .stderr(Stdio::piped())
        .spawn();

    let mut child = child.map_err(|e| format!("Failed to spawn ffmpeg: {e}"))?;
    let stderr = child.stderr.take().unwrap();
    let reader = BufReader::new(stderr);
    let mut parser = FfmpegLogParser::new(reader);

    while let Ok(event) = parser.parse_next_event().await {
        match event {
            FfmpegEvent::Error(e) => {
                log::error!("Extract audio segment error: {e}");
            }
            FfmpegEvent::LogEOF => break,
            _ => {}
        }
    }

    child
        .wait()
        .await
        .map_err(|e| format!("ffmpeg wait error: {e}"))?;

    if output_path.exists() {
        Ok(())
    } else {
        Err("Audio segment extraction failed: output file not found".to_string())
    }
}

/// Get the duration of an audio/video file in seconds
async fn get_audio_duration(file: &Path) -> Result<u64, String> {
    // Use ffprobe with format option to get duration
    let mut ffprobe_process = tokio::process::Command::new(ffprobe_path());
    #[cfg(target_os = "windows")]
    ffprobe_process.creation_flags(CREATE_NO_WINDOW);

    let child = ffprobe_process
        .args(["-v", "quiet"])
        .args(["-show_entries", "format=duration"])
        .args(["-of", "csv=p=0"])
        .args(["-i", file.to_str().unwrap()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();

    if let Err(e) = child {
        return Err(format!("Failed to spawn ffprobe process: {e}"));
    }

    let mut child = child.unwrap();
    let stdout = child.stdout.take().unwrap();
    let reader = BufReader::new(stdout);
    let mut parser = FfmpegLogParser::new(reader);

    let mut duration = None;
    while let Ok(event) = parser.parse_next_event().await {
        match event {
            FfmpegEvent::LogEOF => break,
            FfmpegEvent::Log(_level, content) => {
                // The new command outputs duration directly as a float
                if let Ok(seconds_f64) = content.trim().parse::<f64>() {
                    duration = Some(seconds_f64.ceil() as u64);
                    log::debug!("Parsed duration: {seconds_f64} seconds");
                }
            }
            _ => {}
        }
    }

    if let Err(e) = child.wait().await {
        log::error!("Failed to get duration: {e}");
        return Err(e.to_string());
    }

    duration.ok_or_else(|| "Failed to parse duration".to_string())
}

/// Encode video subtitle using ffmpeg, output is file name with prefix [subtitle]
pub async fn encode_video_subtitle(
    reporter: &impl ProgressReporterTrait,
    file: &Path,
    subtitle: &Path,
    srt_style: String,
) -> Result<String, String> {
    // ffmpeg -i fixed_\[30655190\]1742887114_0325084106_81.5.mp4 -vf "subtitles=test.srt:force_style='FontSize=24'" -c:v libx264 -c:a copy output.mp4
    log::info!("Encode video subtitle task start: {}", file.display());
    log::info!("SRT style: {srt_style}");
    // output path is file with prefix [subtitle]
    let output_filename = format!(
        "{}{}",
        constants::PREFIX_SUBTITLE,
        file.file_name().unwrap().to_str().unwrap()
    );
    let output_path = file.with_file_name(&output_filename);

    // check output path exists - log but allow overwrite
    if output_path.exists() {
        log::info!(
            "Output path already exists, will overwrite: {}",
            output_path.display()
        );
    }

    let mut command_error = None;

    // if windows
    let subtitle = if cfg!(target_os = "windows") {
        // escape characters in subtitle path
        let subtitle = subtitle
            .to_str()
            .unwrap()
            .replace('\\', "\\\\")
            .replace(':', "\\:");
        format!("'{subtitle}'")
    } else {
        format!("'{}'", subtitle.display())
    };
    let vf = format!(
        "{},subtitles={subtitle}:force_style='{srt_style}'",
        hwaccel::H264_SCALE_PAD_FILTER
    );
    log::info!("vf: {vf}");

    let mut ffmpeg_process = ffmpeg_command();
    #[cfg(target_os = "windows")]
    ffmpeg_process.creation_flags(CREATE_NO_WINDOW);

    let video_encoder = hwaccel::get_x264_encoder().await;

    ffmpeg_process.args(["-i", file.to_str().unwrap()]);
    hwaccel::apply_x264_encoder_args(&mut ffmpeg_process, video_encoder, Some(vf.as_str()));
    ffmpeg_process.args(["-c:a", "copy"]);
    hwaccel::apply_x264_quality_args(&mut ffmpeg_process, video_encoder);
    let child = ffmpeg_process
        .args([output_path.to_str().unwrap()])
        .args(["-y"])
        .args(["-progress", "pipe:2"])
        .stderr(Stdio::piped())
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
            FfmpegEvent::Error(e) => {
                log::error!("Encode video subtitle error: {e}");
                command_error = Some(e.to_string());
            }
            FfmpegEvent::Progress(p) => {
                log::info!("Encode video subtitle progress: {}", p.time);
                reporter
                    .update(format!("压制中：{}", p.time).as_str())
                    .await;
            }
            FfmpegEvent::LogEOF => break,
            FfmpegEvent::Log(_level, _content) => {}
            _ => {}
        }
    }

    if let Err(e) = child.wait().await {
        log::error!("Encode video subtitle error: {e}");
        return Err(e.to_string());
    }

    if let Some(error) = command_error {
        log::error!("Encode video subtitle error: {error}");
        Err(error)
    } else {
        log::info!("Encode video subtitle task end: {}", output_path.display());
        Ok(output_filename)
    }
}

pub async fn encode_video_danmu(
    reporter: Option<&impl ProgressReporterTrait>,
    file: &Path,
    subtitle: &Path,
) -> Result<PathBuf, String> {
    // ffmpeg -i fixed_\[30655190\]1742887114_0325084106_81.5.mp4 -vf ass=subtitle.ass -c:v libx264 -c:a copy output.mp4
    log::info!("Encode video danmu task start: {}", file.display());
    let danmu_filename = format!(
        "{}{}",
        constants::PREFIX_DANMAKU,
        file.file_name().unwrap().to_str().unwrap()
    );
    let output_file_path = file.with_file_name(danmu_filename);

    // check output path exists - log but allow overwrite
    if output_file_path.exists() {
        log::info!(
            "Output path already exists, will overwrite: {}",
            output_file_path.display()
        );
    }

    let mut command_error = None;

    // if windows
    let subtitle = if cfg!(target_os = "windows") {
        // escape characters in subtitle path
        let subtitle = subtitle
            .to_str()
            .unwrap()
            .replace('\\', "\\\\")
            .replace(':', "\\:");
        format!("'{subtitle}'")
    } else {
        format!("'{}'", subtitle.display())
    };

    let mut ffmpeg_process = ffmpeg_command();
    #[cfg(target_os = "windows")]
    ffmpeg_process.creation_flags(CREATE_NO_WINDOW);

    let video_encoder = hwaccel::get_x264_encoder().await;

    let vf = format!("{},ass={subtitle}", hwaccel::H264_SCALE_PAD_FILTER);
    ffmpeg_process.args(["-i", file.to_str().unwrap()]);
    hwaccel::apply_x264_encoder_args(&mut ffmpeg_process, video_encoder, Some(vf.as_str()));
    ffmpeg_process.args(["-c:a", "copy"]);
    hwaccel::apply_x264_quality_args(&mut ffmpeg_process, video_encoder);
    let child = ffmpeg_process
        .args([output_file_path.to_str().unwrap()])
        .args(["-y"])
        .args(["-progress", "pipe:2"])
        .stderr(Stdio::piped())
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
            FfmpegEvent::Error(e) => {
                log::error!("Encode video danmu error: {e}");
                command_error = Some(e.to_string());
            }
            FfmpegEvent::Progress(p) => {
                log::debug!("Encode video danmu progress: {}", p.time);
                if reporter.is_none() {
                    continue;
                }
                reporter
                    .unwrap()
                    .update(format!("压制中：{}", p.time).as_str())
                    .await;
            }
            FfmpegEvent::Log(_level, _content) => {}
            FfmpegEvent::LogEOF => break,
            _ => {}
        }
    }

    if let Err(e) = child.wait().await {
        log::error!("Encode video danmu error: {e}");
        return Err(e.to_string());
    }

    if let Some(error) = command_error {
        log::error!("Encode video danmu error: {error}");
        Err(error)
    } else {
        log::info!(
            "Encode video danmu task end: {}",
            output_file_path.display()
        );
        Ok(output_file_path)
    }
}

pub async fn generic_ffmpeg_command(args: &[&str]) -> Result<String, String> {
    let mut ffmpeg_process = ffmpeg_command();
    #[cfg(target_os = "windows")]
    ffmpeg_process.creation_flags(CREATE_NO_WINDOW);

    let child = ffmpeg_process.args(args).stderr(Stdio::piped()).spawn();
    if let Err(e) = child {
        return Err(e.to_string());
    }

    let mut child = child.unwrap();
    let stderr = child.stderr.take().unwrap();
    let reader = BufReader::new(stderr);
    let mut parser = FfmpegLogParser::new(reader);

    let mut logs = Vec::new();

    while let Ok(event) = parser.parse_next_event().await {
        match event {
            FfmpegEvent::Log(_level, content) => {
                logs.push(content);
            }
            FfmpegEvent::LogEOF => break,
            _ => {}
        }
    }

    if let Err(e) = child.wait().await {
        log::error!("Generic ffmpeg command error: {e}");
        return Err(e.to_string());
    }

    Ok(logs.join("\n"))
}

#[allow(clippy::too_many_arguments)]
pub async fn generate_video_subtitle(
    reporter: Option<&ProgressReporter>,
    file: &Path,
    generator_type: &str,
    resource_dir: &Path,
    whisper_model: &str,
    whisper_prompt: &str,
    openai_api_key: &str,
    openai_api_endpoint: &str,
    language_hint: &str,
) -> Result<GenerateResult, String> {
    match generator_type {
        "whisper" => {
            if whisper_model.is_empty() {
                return Err("Whisper model not configured".to_string());
            }
            let vad_model = resource_dir.join("silero_vad.onnx");
            if !vad_model.is_file() {
                return Err(format!(
                    "Bundled Silero VAD model not found: {}",
                    vad_model.display()
                ));
            }
            let generator = match whisper_cpp::new(Path::new(whisper_model), whisper_prompt).await {
                Ok(g) => g,
                Err(e) => return Err(format!("Failed to initialize Whisper model: {e}")),
            };

            // Extract full audio as single 16kHz mono WAV
            if let Some(reporter) = reporter {
                reporter.update("提取完整音频中").await;
            }
            let full_wav = extract_full_audio(file).await?;

            // Read samples for VAD
            let audio = hound::WavReader::open(&full_wav).map_err(|e| e.to_string())?;
            let spec = audio.spec();
            let sample_rate = spec.sample_rate;
            let raw_samples: Vec<i16> = audio
                .into_samples::<i16>()
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("Failed to decode WAV samples: {e}"))?;
            let mut f32_samples = vec![0.0f32; raw_samples.len()];
            whisper_cpp_rs::convert_integer_to_float_audio(&raw_samples, &mut f32_samples)
                .map_err(|e| format!("Audio conversion error: {e}"))?;

            // Silero VAD: find speech segments
            if let Some(reporter) = reporter {
                reporter.update("检测语音片段中").await;
            }
            let mut speech_segments =
                crate::audio_utils::silero_vad(&f32_samples, sample_rate, &vad_model)?;
            let audio_duration = f32_samples.len() as f64 / f64::from(sample_rate);
            if speech_segments.is_empty() && audio_duration > 0.0 {
                log::warn!("Silero VAD detected no speech; falling back to the full audio");
                speech_segments.push(crate::audio_utils::SpeechSegment {
                    start: 0.0,
                    end: audio_duration,
                });
            }
            let (energies, frame_sec) = crate::audio_utils::rms_energies(&f32_samples, sample_rate);
            log::info!(
                "Silero VAD detected {} speech segments from {:.1}s audio",
                speech_segments.len(),
                audio_duration
            );

            // Cut & Merge: normalize to ≤30s chunks
            let max_chunk = 30.0;
            let chunks = crate::audio_utils::cut_and_merge(
                &speech_segments,
                &energies,
                frame_sec,
                max_chunk, // cut_max: split segments >30s
                10.0,      // merge_max: merge adjacent segments ≤10s
            );
            log::info!(
                "Cut & Merge: {} speech segments → {} chunks (≤{}s each)",
                speech_segments.len(),
                chunks.len(),
                max_chunk
            );

            // Process each chunk
            let mut results = Vec::new();
            let mut rolling_context = String::new();
            let temp_dir =
                std::env::temp_dir().join(format!("bsr_whisper_{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&temp_dir)
                .map_err(|e| format!("Failed to create temp dir: {e}"))?;

            let chunk_padding = 0.35;
            for (i, chunk) in chunks.iter().enumerate() {
                let segment_path = temp_dir.join(format!("seg_{:03}.wav", i));
                let padded_start = (chunk.start - chunk_padding).max(0.0);
                let padded_end = chunk.end + chunk_padding;
                let duration = padded_end - padded_start;
                if let Some(reporter) = reporter {
                    reporter
                        .update(&format!(
                            "字幕生成中 ({}/{}, {:.0}s-{:.0}s)",
                            i + 1,
                            chunks.len(),
                            padded_start,
                            padded_end
                        ))
                        .await;
                }
                // Trim the original video to get this segment's audio
                match extract_audio_segment(file, padded_start, duration, &segment_path).await {
                    Ok(()) => {
                        let chunk_generator = generator.with_previous_context(&rolling_context);
                        let result = chunk_generator
                            .generate_subtitle_with_confidence(
                                reporter,
                                &segment_path,
                                language_hint,
                            )
                            .await;
                        if let Ok(generated) = &result {
                            log::info!(
                                "Whisper chunk {} quality: confidence={:.3}, low_token_ratio={:.1}%, retried={}, eligible_for_prompt={}",
                                i,
                                generated.confidence.geometric_mean,
                                generated.confidence.low_token_ratio * 100.0,
                                generated.retried,
                                generated.eligible_for_prompt
                            );
                            if generated.eligible_for_prompt {
                                rolling_context = whisper_cpp::update_rolling_context(
                                    &rolling_context,
                                    &generated.result,
                                );
                                log::debug!(
                                    "Whisper rolling context updated to {} characters",
                                    rolling_context.chars().count()
                                );
                            } else {
                                log::warn!(
                                    "Whisper chunk {} excluded from rolling prompt: confidence={:.3}, low_token_ratio={:.1}%",
                                    i,
                                    generated.confidence.geometric_mean,
                                    generated.confidence.low_token_ratio * 100.0
                                );
                            }
                        }
                        results.push((
                            (padded_start * 1000.0).round() as u64,
                            result.map(|generated| generated.result),
                        ));
                    }
                    Err(e) => {
                        log::error!("Failed to extract segment {}: {e}", i);
                        continue;
                    }
                }
            }

            // Clean up temp files
            let _ = tokio::fs::remove_dir_all(&temp_dir).await;
            let _ = tokio::fs::remove_file(&full_wav).await;

            // Stitch results with time offsets
            let mut full_result = GenerateResult {
                subtitle_id: String::new(),
                subtitle_content: vec![],
                generator_type: SubtitleGeneratorType::Whisper,
            };

            for (offset_ms, result) in &results {
                if let Ok(result) = result {
                    full_result.subtitle_id = result.subtitle_id.clone();
                    full_result.concat_with_offset_ms(result, *offset_ms);
                }
            }
            full_result.clamp_overlaps();

            Ok(full_result)
        }
        "whisper_online" => {
            if openai_api_key.is_empty() {
                return Err("API key not configured".to_string());
            }
            if let Ok(generator) = whisper_online::new(
                Some(openai_api_endpoint),
                Some(openai_api_key),
                Some(whisper_prompt),
            )
            .await
            {
                // Thirty seconds of 16 kHz mono PCM is under 1 MB, so WAV
                // avoids an unnecessary lossy AAC/Opus -> MP3 transcode while
                // staying far below the transcription API upload limit.
                let chunk_dir = extract_audio_chunks(file, "wav").await?;

                let mut full_result = GenerateResult {
                    subtitle_id: String::new(),
                    subtitle_content: vec![],
                    generator_type: SubtitleGeneratorType::WhisperOnline,
                };

                let mut chunk_paths = vec![];
                for entry in std::fs::read_dir(&chunk_dir)
                    .map_err(|e| format!("Failed to read chunk directory: {e}"))?
                {
                    let entry =
                        entry.map_err(|e| format!("Failed to read directory entry: {e}"))?;
                    let path = entry.path();
                    chunk_paths.push(path);
                }
                // sort chunk paths by name
                chunk_paths
                    .sort_by_key(|path| path.file_name().unwrap().to_str().unwrap().to_string());

                let mut results = Vec::new();
                for path in chunk_paths {
                    let result = generator
                        .generate_subtitle(reporter, &path, language_hint)
                        .await;
                    results.push(result);
                }

                for (i, result) in results.iter().enumerate() {
                    if let Ok(result) = result {
                        full_result.subtitle_id = result.subtitle_id.clone();
                        full_result.concat(result, 30 * i as u64);
                    }
                }

                // delete chunk directory
                let _ = tokio::fs::remove_dir_all(chunk_dir).await;

                Ok(full_result)
            } else {
                Err("Failed to initialize Whisper Online".to_string())
            }
        }
        "powerlive" => {
            if let Ok(generator) = powerlive::new(
                "pk_d2755cd38ef03f7ed3a92be1f1471e4adea90a1a5d4b3900345298a68fba0821",
            )
            .await
            {
                let extension = file
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .unwrap_or_default();
                let audio_file = if matches!(extension, "opus" | "wav" | "mp3" | "m4a" | "flac") {
                    file.to_path_buf()
                } else {
                    file.with_extension("opus")
                };
                if !audio_file.exists() {
                    return Err("Audio file not found".to_string());
                }
                let result = generator
                    .generate_subtitle(reporter, &audio_file, language_hint)
                    .await;
                match result {
                    Ok(result) => Ok(result),
                    Err(e) => Err(e),
                }
            } else {
                Err("Failed to initialize PowerLive".to_string())
            }
        }
        _ => Err(format!("Unknown subtitle generator type: {generator_type}")),
    }
}

/// Trying to run ffmpeg for version
pub async fn check_ffmpeg() -> Result<String, String> {
    let child = tokio::process::Command::new(ffmpeg_path())
        .arg("-version")
        .stdout(Stdio::piped())
        .spawn();
    if let Err(e) = child {
        log::error!("Failed to spawn ffmpeg process: {e}");
        return Err(e.to_string());
    }

    let mut child = child.unwrap();

    let stdout = child.stdout.take();
    if stdout.is_none() {
        log::error!("Failed to take ffmpeg output");
        return Err("Failed to take ffmpeg output".into());
    }

    let stdout = stdout.unwrap();
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

    if let Some(version) = version {
        Ok(version)
    } else {
        Err("Failed to parse version from output".into())
    }
}

pub fn ffmpeg_command() -> tokio::process::Command {
    let mut command = tokio::process::Command::new(ffmpeg_path());
    command.kill_on_drop(true);
    command
}

pub fn ffmpeg_path() -> PathBuf {
    let mut path = Path::new("ffmpeg").to_path_buf();
    if cfg!(windows) {
        path.set_extension("exe");
    }

    path
}

fn ffprobe_path() -> PathBuf {
    let mut path = Path::new("ffprobe").to_path_buf();
    if cfg!(windows) {
        path.set_extension("exe");
    }

    path
}

// 从视频文件切片
pub async fn clip_from_video_file(
    reporter: Option<&impl ProgressReporterTrait>,
    input_path: &Path,
    output_path: &Path,
    start_time: f64,
    duration: f64,
) -> Result<(), String> {
    let output_folder = output_path.parent().unwrap();
    if !output_folder.exists() {
        std::fs::create_dir_all(output_folder).unwrap();
    }

    let mut ffmpeg_process = ffmpeg_command();
    #[cfg(target_os = "windows")]
    ffmpeg_process.creation_flags(CREATE_NO_WINDOW);

    let video_encoder = hwaccel::get_x264_encoder().await;

    ffmpeg_process
        .args(["-i", &format!("{}", input_path.display())])
        .args(["-ss", &start_time.to_string()])
        .args(["-t", &duration.to_string()]);
    hwaccel::apply_x264_encoder_args(&mut ffmpeg_process, video_encoder, None);
    ffmpeg_process.args(["-c:a", "aac"]);
    hwaccel::apply_x264_quality_args(&mut ffmpeg_process, video_encoder);
    let child = ffmpeg_process
        .args(["-avoid_negative_ts", "make_zero"])
        .args(["-y", output_path.to_str().unwrap()])
        .args(["-progress", "pipe:2"])
        .stderr(Stdio::piped())
        .spawn();

    if let Err(e) = child {
        return Err(format!("启动ffmpeg进程失败: {e}"));
    }

    let mut child = child.unwrap();
    let stderr = child.stderr.take().unwrap();
    let reader = BufReader::new(stderr);
    let mut parser = FfmpegLogParser::new(reader);

    let mut clip_error = None;
    while let Ok(event) = parser.parse_next_event().await {
        match event {
            FfmpegEvent::Progress(p) => {
                if let Some(reporter) = reporter {
                    reporter.update(&format!("切片进度: {}", p.time)).await;
                }
            }
            FfmpegEvent::LogEOF => break,
            FfmpegEvent::Log(level, content) => {
                if content.contains("error") || level == LogLevel::Error {
                    log::error!("切片错误: {content}");
                }
            }
            FfmpegEvent::Error(e) => {
                log::error!("切片错误: {e}");
                clip_error = Some(e.to_string());
            }
            _ => {}
        }
    }

    if let Err(e) = child.wait().await {
        return Err(e.to_string());
    }

    if let Some(error) = clip_error {
        Err(error)
    } else {
        log::info!("切片任务完成: {}", output_path.display());
        Ok(())
    }
}

/// Extract basic information from a video file.
///
/// # Arguments
/// * `file_path` - The path to the video file.
///
/// # Returns
/// A `Result` containing the video metadata or an error message.
pub async fn extract_video_metadata(file_path: &Path) -> Result<VideoMetadata, String> {
    let mut ffprobe_process = tokio::process::Command::new("ffprobe");
    #[cfg(target_os = "windows")]
    ffprobe_process.creation_flags(CREATE_NO_WINDOW);

    let output = ffprobe_process
        .args([
            "-v",
            "quiet",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
            &format!("{}", file_path.display()),
        ])
        .output()
        .await
        .map_err(|e| format!("执行ffprobe失败: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "ffprobe执行失败: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let json_str = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value =
        serde_json::from_str(&json_str).map_err(|e| format!("解析ffprobe输出失败: {e}"))?;

    // 解析视频流信息
    let streams = json["streams"].as_array().ok_or("未找到视频流信息")?;

    if streams.is_empty() {
        return Err("未找到视频流".to_string());
    }

    let mut metadata = VideoMetadata {
        duration: 0.0,
        width: 0,
        height: 0,
        video_codec: String::new(),
        audio_codec: String::new(),
    };

    for stream in streams {
        let codec_name = stream["codec_type"].as_str().unwrap_or("");
        if codec_name == "video" {
            metadata.video_codec = stream["codec_name"].as_str().unwrap_or("").to_owned();
            metadata.width = stream["width"].as_u64().unwrap_or(0) as u32;
            metadata.height = stream["height"].as_u64().unwrap_or(0) as u32;
            metadata.duration = stream["duration"]
                .as_str()
                .unwrap_or("0.0")
                .parse::<f64>()
                .unwrap_or(0.0);
        } else if codec_name == "audio" {
            metadata.audio_codec = stream["codec_name"].as_str().unwrap_or("").to_owned();
        }
    }
    Ok(metadata)
}

/// Generate thumbnail file from video, capturing a frame at the specified timestamp.
///
/// # Arguments
/// * `video_full_path` - The full path to the video file.
/// * `timestamp` - The timestamp (in seconds) to capture the thumbnail.
///
/// # Returns
/// The path to the generated thumbnail image.
pub async fn generate_thumbnail(video_full_path: &Path, timestamp: f64) -> Result<PathBuf, String> {
    let mut ffmpeg_process = ffmpeg_command();
    #[cfg(target_os = "windows")]
    ffmpeg_process.creation_flags(CREATE_NO_WINDOW);

    let thumbnail_full_path = video_full_path.with_extension("jpg");

    let output = ffmpeg_process
        .args(["-i", &format!("{}", video_full_path.display())])
        .args(["-ss", &timestamp.to_string()])
        .args(["-vframes", "1"])
        .args(["-y", thumbnail_full_path.to_str().unwrap()])
        .output()
        .await
        .map_err(|e| format!("生成缩略图失败: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "ffmpeg生成缩略图失败: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    // 记录生成的缩略图信息
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

// 执行FFmpeg转换的通用函数
pub async fn execute_ffmpeg_conversion(
    mut cmd: tokio::process::Command,
    reporter: &ProgressReporter,
    mode_name: &str,
) -> Result<(), String> {
    use async_ffmpeg_sidecar::event::FfmpegEvent;
    use async_ffmpeg_sidecar::log_parser::FfmpegLogParser;
    use std::process::Stdio;
    use tokio::io::BufReader;

    let mut child = cmd
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("启动FFmpeg进程失败: {e}"))?;

    let stderr = child.stderr.take().unwrap();
    let reader = BufReader::new(stderr);
    let mut parser = FfmpegLogParser::new(reader);

    let mut conversion_error = None;
    while let Ok(event) = parser.parse_next_event().await {
        match event {
            FfmpegEvent::Progress(p) => {
                reporter
                    .update(&format!("正在转换视频格式... {} ({})", p.time, mode_name))
                    .await;
            }
            FfmpegEvent::LogEOF => break,
            FfmpegEvent::Log(level, content) => {
                if matches!(level, async_ffmpeg_sidecar::event::LogLevel::Error)
                    && content.contains("Error")
                {
                    conversion_error = Some(content);
                }
            }
            FfmpegEvent::Error(e) => {
                conversion_error = Some(e);
            }
            _ => {} // 忽略其他事件类型
        }
    }

    let status = child
        .wait()
        .await
        .map_err(|e| format!("等待FFmpeg进程失败: {e}"))?;

    if !status.success() {
        let error_msg = conversion_error
            .unwrap_or_else(|| format!("FFmpeg退出码: {}", status.code().unwrap_or(-1)));
        return Err(format!("视频格式转换失败 ({mode_name}): {error_msg}"));
    }

    reporter
        .update(&format!("视频格式转换完成 100% ({mode_name})"))
        .await;
    Ok(())
}

// 尝试流复制转换（无损，速度快）
pub async fn try_stream_copy_conversion(
    source: &Path,
    dest: &Path,
    reporter: &ProgressReporter,
) -> Result<(), String> {
    reporter.update("正在转换视频格式... 0% (无损模式)").await;

    // 构建ffmpeg命令 - 流复制模式
    let mut cmd = tokio::process::Command::new(ffmpeg_path());
    #[cfg(target_os = "windows")]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW

    cmd.args([
        "-i",
        &source.to_string_lossy(),
        "-c:v",
        "copy", // 直接复制视频流，零损失
        "-c:a",
        "copy", // 直接复制音频流，零损失
        "-avoid_negative_ts",
        "make_zero", // 修复时间戳问题
        "-movflags",
        "+faststart", // 优化web播放
        "-progress",
        "pipe:2", // 输出进度到stderr
        "-y",     // 覆盖输出文件
        &dest.to_string_lossy(),
    ]);

    execute_ffmpeg_conversion(cmd, reporter, "无损转换").await
}

// 高质量重编码转换（兼容性好，质量高）
pub async fn try_high_quality_conversion(
    source: &Path,
    dest: &Path,
    reporter: &ProgressReporter,
) -> Result<(), String> {
    reporter.update("正在转换视频格式... 0% (高质量模式)").await;

    // 构建ffmpeg命令 - 高质量重编码
    let mut cmd = tokio::process::Command::new(ffmpeg_path());
    #[cfg(target_os = "windows")]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW

    cmd.args([
        "-i",
        &source.to_string_lossy(),
        "-c:v",
        "libx264", // H.264编码器
        "-preset",
        "slow", // 慢速预设，更好的压缩效率
        "-crf",
        "18", // 高质量设置 (18-23范围，越小质量越高)
        "-c:a",
        "aac", // AAC音频编码器
        "-b:a",
        "192k", // 高音频码率
        "-avoid_negative_ts",
        "make_zero", // 修复时间戳问题
        "-movflags",
        "+faststart", // 优化web播放
        "-progress",
        "pipe:2", // 输出进度到stderr
        "-y",     // 覆盖输出文件
        &dest.to_string_lossy(),
    ]);

    execute_ffmpeg_conversion(cmd, reporter, "高质量转换").await
}

// 带进度的视频格式转换函数（智能质量保持策略）
pub async fn convert_video_format(
    source: &Path,
    dest: &Path,
    reporter: &ProgressReporter,
) -> Result<(), String> {
    // 先尝试stream copy（无损转换），如果失败则使用高质量重编码
    match try_stream_copy_conversion(source, dest, reporter).await {
        Ok(()) => Ok(()),
        Err(stream_copy_error) => {
            reporter.update("流复制失败，使用高质量重编码模式...").await;
            log::warn!("Stream copy failed: {stream_copy_error}, falling back to re-encoding");
            try_high_quality_conversion(source, dest, reporter).await
        }
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

// tests
#[cfg(test)]
mod tests {
    use super::*;

    // 测试 Range 结构体
    #[test]
    fn test_range_creation() {
        let range = Range {
            start: 10.0,
            end: 30.0,
        };
        assert_eq!(range.start, 10.0);
        assert_eq!(range.end, 30.0);
        assert_eq!(range.duration(), 20.0);
    }

    #[test]
    fn test_range_duration() {
        let range = Range {
            start: 0.0,
            end: 60.0,
        };
        assert_eq!(range.duration(), 60.0);

        let range2 = Range {
            start: 15.5,
            end: 45.5,
        };
        assert_eq!(range2.duration(), 30.0);
    }

    #[test]
    fn test_range_display() {
        let range = Range {
            start: 5.0,
            end: 25.0,
        };
        assert_eq!(range.to_string(), "[5, 25]");
    }

    #[test]
    fn test_range_edge_cases() {
        let zero_range = Range {
            start: 0.0,
            end: 0.0,
        };
        assert_eq!(zero_range.duration(), 0.0);

        let negative_start = Range {
            start: -5.0,
            end: 10.0,
        };
        assert_eq!(negative_start.duration(), 15.0);

        let large_range = Range {
            start: 1000.0,
            end: 2000.0,
        };
        assert_eq!(large_range.duration(), 1000.0);
    }

    // 测试视频元数据提取
    #[tokio::test]
    async fn test_extract_video_metadata() {
        let test_video = Path::new("tests/video/test.mp4");
        if test_video.exists() {
            let metadata = extract_video_metadata(test_video).await.unwrap();
            println!("metadata: {:?}", metadata);
            assert!(metadata.duration > 0.0);
            assert!(metadata.width > 0);
            assert!(metadata.height > 0);
        }
    }

    // 测试音频时长获取
    #[tokio::test]
    async fn test_get_audio_duration() {
        let test_audio = Path::new("tests/audio/test.wav");
        if test_audio.exists() {
            let duration = get_audio_duration(test_audio).await.unwrap();
            assert!(duration > 0);
        }
    }

    // 测试缩略图生成
    #[tokio::test]
    async fn test_generate_thumbnail() {
        let file = Path::new("tests/video/test.mp4");
        if file.exists() {
            let thumbnail_file = generate_thumbnail(file, 0.0).await.unwrap();
            assert!(thumbnail_file.exists());
            assert_eq!(thumbnail_file.extension().unwrap(), "jpg");
            // clean up
            let _ = std::fs::remove_file(thumbnail_file);
        }
    }

    // 测试 FFmpeg 版本检查
    #[tokio::test]
    async fn test_check_ffmpeg() {
        let result = check_ffmpeg().await;
        match result {
            Ok(version) => {
                assert!(!version.is_empty());
                // FFmpeg 版本字符串可能不包含 "ffmpeg" 这个词，所以检查是否包含数字
                assert!(version.chars().any(|c| c.is_ascii_digit()));
            }
            Err(_) => {
                // FFmpeg 可能没有安装，这是正常的
                println!("FFmpeg not available for testing");
            }
        }
    }

    // 测试通用 FFmpeg 命令
    #[tokio::test]
    async fn test_generic_ffmpeg_command() {
        let result = generic_ffmpeg_command(&["-version"]).await;
        match result {
            Ok(_output) => {
                // 输出可能为空或者不包含 "ffmpeg" 字符串，我们只检查函数能正常执行
                println!("FFmpeg command executed successfully");
            }
            Err(_) => {
                // FFmpeg 可能没有安装，这是正常的
                println!("FFmpeg not available for testing");
            }
        }
    }

    // 测试硬件加速能力探测
    #[tokio::test]
    async fn test_list_supported_hwaccels() {
        match super::hwaccel::list_supported_hwaccels().await {
            Ok(hwaccels) => {
                println!("hwaccels: {:?}", hwaccels);
                let mut sorted = hwaccels.clone();
                sorted.sort();
                sorted.dedup();
                assert_eq!(sorted.len(), hwaccels.len());
            }
            Err(_) => {
                println!("FFmpeg hardware acceleration query not available for testing");
            }
        }
    }

    // 测试字幕生成错误处理
    #[tokio::test]
    async fn test_generate_video_subtitle_errors() {
        let test_file = Path::new("tests/video/test.mp4");

        // 测试 Whisper 类型 - 模型未配置
        let result = generate_video_subtitle(
            None,
            test_file,
            "whisper",
            Path::new(""),
            "",
            "",
            "",
            "",
            "zh",
        )
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Whisper model not configured"));

        // 测试 Whisper Online 类型 - API key 未配置
        let result = generate_video_subtitle(
            None,
            test_file,
            "whisper_online",
            Path::new(""),
            "",
            "",
            "",
            "",
            "zh",
        )
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("API key not configured"));

        // 测试未知类型
        let result = generate_video_subtitle(
            None,
            test_file,
            "unknown_type",
            Path::new(""),
            "",
            "",
            "",
            "",
            "",
        )
        .await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("Unknown subtitle generator type"));
    }

    // 测试路径构建函数
    #[test]
    fn test_ffmpeg_paths() {
        let ffmpeg_path = ffmpeg_path();
        let ffprobe_path = ffprobe_path();

        #[cfg(windows)]
        {
            assert_eq!(ffmpeg_path.extension().unwrap(), "exe");
            assert_eq!(ffprobe_path.extension().unwrap(), "exe");
        }

        #[cfg(not(windows))]
        {
            assert_eq!(ffmpeg_path.file_name().unwrap(), "ffmpeg");
            assert_eq!(ffprobe_path.file_name().unwrap(), "ffprobe");
        }
    }

    // 测试文件名和路径处理
    #[test]
    fn test_filename_processing() {
        let test_file = Path::new("tests/video/test.mp4");

        // 测试字幕文件名生成
        let subtitle_filename = format!(
            "{}{}",
            constants::PREFIX_SUBTITLE,
            test_file.file_name().unwrap().to_str().unwrap()
        );
        assert!(subtitle_filename.starts_with(constants::PREFIX_SUBTITLE));
        assert!(subtitle_filename.contains("test.mp4"));

        // 测试弹幕文件名生成
        let danmu_filename = format!(
            "{}{}",
            constants::PREFIX_DANMAKU,
            test_file.file_name().unwrap().to_str().unwrap()
        );
        assert!(danmu_filename.starts_with(constants::PREFIX_DANMAKU));
        assert!(danmu_filename.contains("test.mp4"));
    }

    // 测试音频分块目录结构
    #[test]
    fn test_audio_chunk_directory_structure() {
        let test_file = Path::new("tests/audio/test.wav");
        let output_path = test_file.with_extension("wav");
        let output_dir = output_path.parent().unwrap();
        let base_name = output_path.file_stem().unwrap().to_str().unwrap();
        let chunk_dir = output_dir.join(format!("{base_name}_chunks"));

        assert!(chunk_dir.to_string_lossy().contains("_chunks"));
        assert!(chunk_dir.to_string_lossy().contains("test"));
    }

    #[test]
    fn test_range_is_in_inside() {
        let r = Range {
            start: 1.0,
            end: 5.0,
        };
        assert!(r.is_in(3.0));
    }

    #[test]
    fn test_range_is_in_at_boundaries() {
        let r = Range {
            start: 1.0,
            end: 5.0,
        };
        assert!(r.is_in(1.0));
        assert!(r.is_in(5.0));
    }

    #[test]
    fn test_range_is_in_outside() {
        let r = Range {
            start: 1.0,
            end: 5.0,
        };
        assert!(!r.is_in(0.9));
        assert!(!r.is_in(5.1));
    }

    #[test]
    fn test_video_metadata_equality() {
        let m1 = VideoMetadata {
            duration: 10.0,
            width: 1920,
            height: 1080,
            video_codec: "h264".to_string(),
            audio_codec: "aac".to_string(),
        };
        let m2 = m1.clone();
        assert_eq!(m1, m2);
    }

    #[test]
    fn test_video_metadata_different_resolution() {
        let m1 = VideoMetadata {
            duration: 10.0,
            width: 1920,
            height: 1080,
            video_codec: "h264".to_string(),
            audio_codec: "aac".to_string(),
        };
        let m2 = VideoMetadata {
            duration: 10.0,
            width: 1280,
            height: 720,
            video_codec: "h264".to_string(),
            audio_codec: "aac".to_string(),
        };
        assert_ne!(m1, m2);
    }

    #[test]
    fn test_video_metadata_different_codec() {
        let m1 = VideoMetadata {
            duration: 10.0,
            width: 1920,
            height: 1080,
            video_codec: "h264".to_string(),
            audio_codec: "aac".to_string(),
        };
        let m2 = VideoMetadata {
            duration: 10.0,
            width: 1920,
            height: 1080,
            video_codec: "hevc".to_string(),
            audio_codec: "aac".to_string(),
        };
        assert_ne!(m1, m2);
    }
}
