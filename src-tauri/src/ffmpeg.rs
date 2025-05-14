use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::progress_reporter::ProgressReporterTrait;
use async_ffmpeg_sidecar::event::FfmpegEvent;
use async_ffmpeg_sidecar::log_parser::FfmpegLogParser;
use tokio::io::BufReader;

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
            FfmpegEvent::Log(_level, content) => {
                log::info!("{}", content);
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

pub async fn extract_audio(file: &Path) -> Result<(), String> {
    // ffmpeg -i fixed_\[30655190\]1742887114_0325084106_81.5.mp4 -ar 16000 test.wav
    log::info!("Extract audio task start: {}", file.display());
    let output_path = file.with_extension("wav");
    let mut extract_error = None;

    let child = tokio::process::Command::new(ffmpeg_path())
        .args(["-i", file.to_str().unwrap()])
        .args(["-ar", "16000"])
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
                log::error!("Extract audio error: {}", e);
                extract_error = Some(e.to_string());
            }
            FfmpegEvent::LogEOF => break,
            FfmpegEvent::Log(_level, content) => {
                log::info!("{}", content);
            }
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
        log::info!("Extract audio task end: {}", output_path.display());
        Ok(())
    }
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
            FfmpegEvent::Log(_level, content) => {
                log::info!("{}", content);
            }
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
            FfmpegEvent::Log(_level, content) => {
                log::info!("{}", content);
            }
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

    if version.is_none() {
        Err("Failed to parse version from output".into())
    } else {
        Ok(version.unwrap())
    }
}

fn ffmpeg_path() -> PathBuf {
    let mut path = Path::new("ffmpeg").to_path_buf();
    if cfg!(windows) {
        path.set_extension("exe");
    }

    return path;
}
