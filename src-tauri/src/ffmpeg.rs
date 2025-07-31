use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::progress_reporter::{ProgressReporter, ProgressReporterTrait};
use crate::subtitle_generator::whisper_online;
use crate::subtitle_generator::{
    whisper_cpp, GenerateResult, SubtitleGenerator, SubtitleGeneratorType,
};
use async_ffmpeg_sidecar::event::{FfmpegEvent, LogLevel};
use async_ffmpeg_sidecar::log_parser::FfmpegLogParser;
use tokio::io::{AsyncBufReadExt, BufReader};

pub async fn clip_from_m3u8(
    reporter: Option<&impl ProgressReporterTrait>,
    m3u8_index: &Path,
    output_path: &Path,
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

    let child = tokio::process::Command::new(ffmpeg_path())
        .args(["-i", &format!("{}", m3u8_index.display())])
        .args(["-c", "copy"])
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

    let child = tokio::process::Command::new(ffmpeg_path())
        .args(&args)
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
    let child = tokio::process::Command::new(ffprobe_path())
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
    let child = tokio::process::Command::new(ffprobe_path())
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

    // check output path exists
    if output_path.exists() {
        log::info!("Output path already exists: {}", output_path.display());
        return Err("Output path already exists".to_string());
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

    let child = tokio::process::Command::new(ffmpeg_path())
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

    // check output path exists
    if output_path.exists() {
        log::info!("Output path already exists: {}", output_path.display());
        return Err("Output path already exists".to_string());
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

    let child = tokio::process::Command::new(ffmpeg_path())
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
                log::info!("Encode video danmu progress: {}", p.time);
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
    let child = tokio::process::Command::new(ffmpeg_path())
        .args(args)
        .stderr(Stdio::piped())
        .spawn();
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
    let child = tokio::process::Command::new(ffprobe_path())
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
