use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::progress_reporter::{ProgressReporter, ProgressReporterTrait};
use crate::subtitle_generator::whisper_online;
use crate::subtitle_generator::{
    whisper_cpp, GenerateResult, SubtitleGenerator, SubtitleGeneratorType,
};
use async_ffmpeg_sidecar::event::{FfmpegEvent, LogLevel};
use async_ffmpeg_sidecar::log_parser::FfmpegLogParser;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};

// 视频元数据结构
#[derive(Debug)]
pub struct VideoMetadata {
    pub duration: f64,
    pub width: u32,
    pub height: u32,
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
}

pub async fn clip_from_m3u8(
    reporter: Option<&impl ProgressReporterTrait>,
    m3u8_index: &Path,
    output_path: &Path,
    range: Option<&Range>,
    fix_encoding: bool,
) -> Result<(), String> {
    // first check output folder exists
    let output_folder = output_path.parent().unwrap();
    if !output_folder.exists() {
        log::warn!(
            "Output folder does not exist, creating: {}",
            output_folder.display()
        );
        std::fs::create_dir_all(output_folder).unwrap();
    }

    let mut ffmpeg_process = tokio::process::Command::new(ffmpeg_path());
    #[cfg(target_os = "windows")]
    ffmpeg_process.creation_flags(CREATE_NO_WINDOW);

    let child_command = ffmpeg_process.args(["-i", &format!("{}", m3u8_index.display())]);

    if let Some(range) = range {
        child_command
            .args(["-ss", &range.start.to_string()])
            .args(["-t", &range.duration().to_string()]);
    }

    if fix_encoding {
        child_command
            .args(["-c:v", "libx264"])
            .args(["-c:a", "aac"])
            .args(["-preset", "fast"]);
    } else {
        child_command.args(["-c", "copy"]);
    }

    let child = child_command
        .args(["-y", output_path.to_str().unwrap()])
        .args(["-progress", "pipe:2"])
        .stderr(Stdio::piped())
        .spawn();

    if let Err(e) = child {
        return Err(format!("Spawn ffmpeg process failed: {}", e));
    }

    let mut child = child.unwrap();
    let stderr = child.stderr.take().unwrap();
    let reader = BufReader::new(stderr);
    let mut parser = FfmpegLogParser::new(reader);

    let mut clip_error = None;
    while let Ok(event) = parser.parse_next_event().await {
        match event {
            FfmpegEvent::Progress(p) => {
                if reporter.is_none() {
                    continue;
                }
                log::debug!("Clip progress: {}", p.time);
                reporter
                    .unwrap()
                    .update(format!("编码中：{}", p.time).as_str())
            }
            FfmpegEvent::LogEOF => break,
            FfmpegEvent::Log(level, content) => {
                // log error if content contains error
                if content.contains("error") || level == LogLevel::Error {
                    log::error!("Clip error: {}", content);
                }
            }
            FfmpegEvent::Error(e) => {
                log::error!("Clip error: {}", e);
                clip_error = Some(e.to_string());
            }
            _ => {}
        }
    }

    if let Err(e) = child.wait().await {
        log::error!("Clip error: {}", e);
        return Err(e.to_string());
    }

    if let Some(error) = clip_error {
        log::error!("Clip error: {}", error);
        Err(error)
    } else {
        log::info!("Clip task end: {}", output_path.display());
        Ok(())
    }
}

pub async fn extract_audio_chunks(file: &Path, format: &str) -> Result<PathBuf, String> {
    // ffmpeg -i fixed_\[30655190\]1742887114_0325084106_81.5.mp4 -ar 16000 test.wav
    log::info!("Extract audio task start: {}", file.display());
    let output_path = file.with_extension(format);
    let mut extract_error = None;

    // 降低采样率以提高处理速度，同时保持足够的音质用于语音识别
    let sample_rate = if format == "mp3" { "22050" } else { "16000" };

    // First, get the duration of the input file
    let duration = get_audio_duration(file).await?;
    log::info!("Audio duration: {} seconds", duration);

    // Split into chunks of 30 seconds
    let chunk_duration = 30;
    let chunk_count = (duration as f64 / chunk_duration as f64).ceil() as usize;
    log::info!(
        "Splitting into {} chunks of {} seconds each",
        chunk_count,
        chunk_duration
    );

    // Create output directory for chunks
    let output_dir = output_path.parent().unwrap();
    let base_name = output_path.file_stem().unwrap().to_str().unwrap();
    let chunk_dir = output_dir.join(format!("{}_chunks", base_name));

    if !chunk_dir.exists() {
        std::fs::create_dir_all(&chunk_dir)
            .map_err(|e| format!("Failed to create chunk directory: {}", e))?;
    }

    // Use ffmpeg segment feature to split audio into chunks
    let segment_pattern = chunk_dir.join(format!("{}_%03d.{}", base_name, format));

    // 构建优化的ffmpeg命令参数
    let file_str = file.to_str().unwrap();
    let chunk_duration_str = chunk_duration.to_string();
    let segment_pattern_str = segment_pattern.to_str().unwrap();

    let mut args = vec![
        "-i",
        file_str,
        "-ar",
        sample_rate,
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

    let mut ffmpeg_process = tokio::process::Command::new(ffmpeg_path());
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
                log::error!("Extract audio error: {}", e);
                extract_error = Some(e.to_string());
            }
            FfmpegEvent::LogEOF => break,
            FfmpegEvent::Log(_level, _content) => {}
            _ => {}
        }
    }

    if let Err(e) = child.wait().await {
        log::error!("Extract audio error: {}", e);
        return Err(e.to_string());
    }

    if let Some(error) = extract_error {
        log::error!("Extract audio error: {}", error);
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
        return Err(format!("Failed to spawn ffprobe process: {}", e));
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
                    log::debug!("Parsed duration: {} seconds", seconds_f64);
                }
            }
            _ => {}
        }
    }

    if let Err(e) = child.wait().await {
        log::error!("Failed to get duration: {}", e);
        return Err(e.to_string());
    }

    duration.ok_or_else(|| "Failed to parse duration".to_string())
}

/// Get the precise duration of a video segment (TS/MP4) in seconds
pub async fn get_segment_duration(file: &Path) -> Result<f64, String> {
    // Use ffprobe to get the exact duration of the segment
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
        return Err(format!(
            "Failed to spawn ffprobe process for segment: {}",
            e
        ));
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
                // Parse the exact duration as f64 for precise timing
                if let Ok(seconds_f64) = content.trim().parse::<f64>() {
                    duration = Some(seconds_f64);
                    log::debug!("Parsed segment duration: {} seconds", seconds_f64);
                }
            }
            _ => {}
        }
    }

    if let Err(e) = child.wait().await {
        log::error!("Failed to get segment duration: {}", e);
        return Err(e.to_string());
    }

    duration.ok_or_else(|| "Failed to parse segment duration".to_string())
}

pub async fn encode_video_subtitle(
    reporter: &impl ProgressReporterTrait,
    file: &Path,
    subtitle: &Path,
    srt_style: String,
) -> Result<String, String> {
    // ffmpeg -i fixed_\[30655190\]1742887114_0325084106_81.5.mp4 -vf "subtitles=test.srt:force_style='FontSize=24'" -c:v libx264 -c:a copy output.mp4
    log::info!("Encode video subtitle task start: {}", file.display());
    log::info!("srt_style: {}", srt_style);
    // output path is file with prefix [subtitle]
    let output_filename = format!("[subtitle]{}", file.file_name().unwrap().to_str().unwrap());
    let output_path = file.with_file_name(&output_filename);

    // check output path exists - log but allow overwrite
    if output_path.exists() {
        log::info!("Output path already exists, will overwrite: {}", output_path.display());
    }

    let mut command_error = None;

    // if windows
    let subtitle = if cfg!(target_os = "windows") {
        // escape characters in subtitle path
        let subtitle = subtitle
            .to_str()
            .unwrap()
            .replace("\\", "\\\\")
            .replace(":", "\\:");
        format!("'{}'", subtitle)
    } else {
        format!("'{}'", subtitle.display())
    };
    let vf = format!("subtitles={}:force_style='{}'", subtitle, srt_style);
    log::info!("vf: {}", vf);

    let mut ffmpeg_process = tokio::process::Command::new(ffmpeg_path());
    #[cfg(target_os = "windows")]
    ffmpeg_process.creation_flags(CREATE_NO_WINDOW);

    let child = ffmpeg_process
        .args(["-i", file.to_str().unwrap()])
        .args(["-vf", vf.as_str()])
        .args(["-c:v", "libx264"])
        .args(["-c:a", "copy"])
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
                log::error!("Encode video subtitle error: {}", e);
                command_error = Some(e.to_string());
            }
            FfmpegEvent::Progress(p) => {
                log::info!("Encode video subtitle progress: {}", p.time);
                reporter.update(format!("压制中：{}", p.time).as_str());
            }
            FfmpegEvent::LogEOF => break,
            FfmpegEvent::Log(_level, _content) => {}
            _ => {}
        }
    }

    if let Err(e) = child.wait().await {
        log::error!("Encode video subtitle error: {}", e);
        return Err(e.to_string());
    }

    if let Some(error) = command_error {
        log::error!("Encode video subtitle error: {}", error);
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
    let danmu_filename = format!("[danmu]{}", file.file_name().unwrap().to_str().unwrap());
    let output_path = file.with_file_name(danmu_filename);

    // check output path exists - log but allow overwrite
    if output_path.exists() {
        log::info!("Output path already exists, will overwrite: {}", output_path.display());
    }

    let mut command_error = None;

    // if windows
    let subtitle = if cfg!(target_os = "windows") {
        // escape characters in subtitle path
        let subtitle = subtitle
            .to_str()
            .unwrap()
            .replace("\\", "\\\\")
            .replace(":", "\\:");
        format!("'{}'", subtitle)
    } else {
        format!("'{}'", subtitle.display())
    };

    let mut ffmpeg_process = tokio::process::Command::new(ffmpeg_path());
    #[cfg(target_os = "windows")]
    ffmpeg_process.creation_flags(CREATE_NO_WINDOW);

    let child = ffmpeg_process
        .args(["-i", file.to_str().unwrap()])
        .args(["-vf", &format!("ass={}", subtitle)])
        .args(["-c:v", "libx264"])
        .args(["-c:a", "copy"])
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
                log::error!("Encode video danmu error: {}", e);
                command_error = Some(e.to_string());
            }
            FfmpegEvent::Progress(p) => {
                log::debug!("Encode video danmu progress: {}", p.time);
                if reporter.is_none() {
                    continue;
                }
                reporter
                    .unwrap()
                    .update(format!("压制中：{}", p.time).as_str());
            }
            FfmpegEvent::Log(_level, _content) => {}
            FfmpegEvent::LogEOF => break,
            _ => {}
        }
    }

    if let Err(e) = child.wait().await {
        log::error!("Encode video danmu error: {}", e);
        return Err(e.to_string());
    }

    if let Some(error) = command_error {
        log::error!("Encode video danmu error: {}", error);
        Err(error)
    } else {
        log::info!("Encode video danmu task end: {}", output_path.display());
        Ok(output_path)
    }
}

pub async fn generic_ffmpeg_command(args: &[&str]) -> Result<String, String> {
    let mut ffmpeg_process = tokio::process::Command::new(ffmpeg_path());
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
        log::error!("Generic ffmpeg command error: {}", e);
        return Err(e.to_string());
    }

    Ok(logs.join("\n"))
}

#[allow(clippy::too_many_arguments)]
pub async fn generate_video_subtitle(
    reporter: Option<&ProgressReporter>,
    file: &Path,
    generator_type: &str,
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
            if let Ok(generator) = whisper_cpp::new(Path::new(&whisper_model), whisper_prompt).await
            {
                let chunk_dir = extract_audio_chunks(file, "wav").await?;

                let mut full_result = GenerateResult {
                    subtitle_id: "".to_string(),
                    subtitle_content: vec![],
                    generator_type: SubtitleGeneratorType::Whisper,
                };

                let mut chunk_paths = vec![];
                for entry in std::fs::read_dir(&chunk_dir)
                    .map_err(|e| format!("Failed to read chunk directory: {}", e))?
                {
                    let entry =
                        entry.map_err(|e| format!("Failed to read directory entry: {}", e))?;
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
                Err("Failed to initialize Whisper model".to_string())
            }
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
                let chunk_dir = extract_audio_chunks(file, "mp3").await?;

                let mut full_result = GenerateResult {
                    subtitle_id: "".to_string(),
                    subtitle_content: vec![],
                    generator_type: SubtitleGeneratorType::WhisperOnline,
                };

                let mut chunk_paths = vec![];
                for entry in std::fs::read_dir(&chunk_dir)
                    .map_err(|e| format!("Failed to read chunk directory: {}", e))?
                {
                    let entry =
                        entry.map_err(|e| format!("Failed to read directory entry: {}", e))?;
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
        _ => Err(format!(
            "Unknown subtitle generator type: {}",
            generator_type
        )),
    }
}

/// Trying to run ffmpeg for version
pub async fn check_ffmpeg() -> Result<String, String> {
    let child = tokio::process::Command::new(ffmpeg_path())
        .arg("-version")
        .stdout(Stdio::piped())
        .spawn();
    if let Err(e) = child {
        log::error!("Faild to spwan ffmpeg process: {e}");
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

pub async fn get_video_resolution(file: &str) -> Result<String, String> {
    // ffprobe -v error -select_streams v:0 -show_entries stream=width,height -of csv=s=x:p=0 input.mp4
    let mut ffprobe_process = tokio::process::Command::new(ffprobe_path());
    #[cfg(target_os = "windows")]
    ffprobe_process.creation_flags(CREATE_NO_WINDOW);

    let child = ffprobe_process
        .arg("-i")
        .arg(file)
        .arg("-v")
        .arg("error")
        .arg("-select_streams")
        .arg("v:0")
        .arg("-show_entries")
        .arg("stream=width,height")
        .arg("-of")
        .arg("csv=s=x:p=0")
        .stdout(Stdio::piped())
        .spawn();
    if let Err(e) = child {
        log::error!("Faild to spwan ffprobe process: {e}");
        return Err(e.to_string());
    }

    let mut child = child.unwrap();
    let stdout = child.stdout.take();
    if stdout.is_none() {
        log::error!("Failed to take ffprobe output");
        return Err("Failed to take ffprobe output".into());
    }

    let stdout = stdout.unwrap();
    let reader = BufReader::new(stdout);
    let mut lines = reader.lines();
    let line = lines.next_line().await.unwrap();
    if line.is_none() {
        return Err("Failed to parse resolution from output".into());
    }
    let line = line.unwrap();
    let resolution = line.split("x").collect::<Vec<&str>>();
    if resolution.len() != 2 {
        return Err("Failed to parse resolution from output".into());
    }
    Ok(format!("{}x{}", resolution[0], resolution[1]))
}

fn ffmpeg_path() -> PathBuf {
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

// 解析 FFmpeg 时间字符串 (格式如 "00:01:23.45")
fn parse_time_string(time_str: &str) -> Result<f64, String> {
    let parts: Vec<&str> = time_str.split(':').collect();
    if parts.len() != 3 {
        return Err("Invalid time format".to_string());
    }
    
    let hours: f64 = parts[0].parse().map_err(|_| "Invalid hours")?;
    let minutes: f64 = parts[1].parse().map_err(|_| "Invalid minutes")?;
    let seconds: f64 = parts[2].parse().map_err(|_| "Invalid seconds")?;
    
    Ok(hours * 3600.0 + minutes * 60.0 + seconds)
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

    let mut ffmpeg_process = tokio::process::Command::new(ffmpeg_path());
    #[cfg(target_os = "windows")]
    ffmpeg_process.creation_flags(CREATE_NO_WINDOW);

    let child = ffmpeg_process
        .args(["-i", &format!("{}", input_path.display())])
        .args(["-ss", &start_time.to_string()])
        .args(["-t", &duration.to_string()])
        .args(["-c:v", "libx264"])
        .args(["-c:a", "aac"])
        .args(["-preset", "fast"])
        .args(["-crf", "23"])
        .args(["-avoid_negative_ts", "make_zero"])
        .args(["-y", output_path.to_str().unwrap()])
        .args(["-progress", "pipe:2"])
        .stderr(Stdio::piped())
        .spawn();

    if let Err(e) = child {
        return Err(format!("启动ffmpeg进程失败: {}", e));
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
                    // 解析时间字符串 (格式如 "00:01:23.45")
                    if let Ok(current_time) = parse_time_string(&p.time) {
                        let progress = (current_time / duration * 100.0).min(100.0);
                        reporter.update(&format!("切片进度: {:.1}%", progress));
                    }
                }
            }
            FfmpegEvent::LogEOF => break,
            FfmpegEvent::Log(level, content) => {
                if content.contains("error") || level == LogLevel::Error {
                    log::error!("切片错误: {}", content);
                }
            }
            FfmpegEvent::Error(e) => {
                log::error!("切片错误: {}", e);
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

// 获取视频元数据
pub async fn extract_video_metadata(file_path: &Path) -> Result<VideoMetadata, String> {
    let mut ffprobe_process = tokio::process::Command::new("ffprobe");
    #[cfg(target_os = "windows")]
    ffprobe_process.creation_flags(CREATE_NO_WINDOW);

    let output = ffprobe_process
        .args([
            "-v", "quiet",
            "-print_format", "json",
            "-show_format",
            "-show_streams",
            "-select_streams", "v:0",
            &format!("{}", file_path.display())
        ])
        .output()
        .await
        .map_err(|e| format!("执行ffprobe失败: {}", e))?;

    if !output.status.success() {
        return Err(format!("ffprobe执行失败: {}", String::from_utf8_lossy(&output.stderr)));
    }

    let json_str = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&json_str)
        .map_err(|e| format!("解析ffprobe输出失败: {}", e))?;

    // 解析视频流信息
    let streams = json["streams"].as_array()
        .ok_or("未找到视频流信息")?;
    
    if streams.is_empty() {
        return Err("未找到视频流".to_string());
    }

    let video_stream = &streams[0];
    let format = &json["format"];

    let duration = format["duration"].as_str()
        .and_then(|d| d.parse::<f64>().ok())
        .unwrap_or(0.0);

    let width = video_stream["width"].as_u64().unwrap_or(0) as u32;
    let height = video_stream["height"].as_u64().unwrap_or(0) as u32;

    Ok(VideoMetadata {
        duration,
        width,
        height,
    })
}

// 生成视频缩略图
pub async fn generate_thumbnail(
    video_path: &Path,
    output_path: &Path,
    timestamp: f64,
) -> Result<(), String> {
    let output_folder = output_path.parent().unwrap();
    if !output_folder.exists() {
        std::fs::create_dir_all(output_folder).unwrap();
    }

    let mut ffmpeg_process = tokio::process::Command::new(ffmpeg_path());
    #[cfg(target_os = "windows")]
    ffmpeg_process.creation_flags(CREATE_NO_WINDOW);

    let output = ffmpeg_process
        .args(["-i", &format!("{}", video_path.display())])
        .args(["-ss", &timestamp.to_string()])
        .args(["-vframes", "1"])
        .args(["-y", output_path.to_str().unwrap()])
        .output()
        .await
        .map_err(|e| format!("生成缩略图失败: {}", e))?;

    if !output.status.success() {
        return Err(format!("ffmpeg生成缩略图失败: {}", String::from_utf8_lossy(&output.stderr)));
    }

    // 记录生成的缩略图信息
    if let Ok(metadata) = std::fs::metadata(output_path) {
        log::info!("生成缩略图完成: {} (文件大小: {} bytes)", output_path.display(), metadata.len());
    } else {
        log::info!("生成缩略图完成: {}", output_path.display());
    }
    Ok(())
}

// tests
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_get_video_size() {
        let file = Path::new("/Users/xinreasuper/Desktop/shadowreplay-test/output2/[1789714684][1753965688317][摄像头被前夫抛妻弃子直播挣点奶粉][2025-07-31_12-58-14].mp4");
        let resolution = get_video_resolution(file.to_str().unwrap()).await.unwrap();
        println!("Resolution: {}", resolution);
    }
}
