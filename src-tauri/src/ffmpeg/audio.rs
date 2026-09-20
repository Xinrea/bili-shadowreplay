use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

use async_ffmpeg_sidecar::{event::FfmpegEvent, log_parser::FfmpegLogParser};
use tempfile::{tempdir_in, TempDir};
use tokio::io::BufReader;

use super::runner::{FfmpegJob, ProgressMode};
use ffmpeg_utils::{ffprobe_command, path_str};

/// Extract an audio sample for waveform display.
pub async fn extract_audio_sample(file: &Path) -> Result<PathBuf, String> {
    log::info!("Extract audio sample task start: {}", file.display());
    let output_path = file.with_extension("opus");
    let job = FfmpegJob::new()
        .args([
            "-i".to_string(),
            file.to_string_lossy().into_owned(),
            "-c:a".to_string(),
            "libopus".to_string(),
            "-ar".to_string(),
            "16000".to_string(),
            "-ac".to_string(),
            "1".to_string(),
            "-vn".to_string(),
            "-b:a".to_string(),
            "64k".to_string(),
            "-vbr".to_string(),
            "on".to_string(),
            "-compression_level".to_string(),
            "10".to_string(),
            "-y".to_string(),
        ])
        .output(output_path.clone())
        .progress(ProgressMode::Time)
        .context("Extract audio sample");
    job.run_without_reporter()
        .await
        .map_err(|error| error.to_string())?;

    if output_path.exists() {
        log::info!("Extract audio sample task end: {}", output_path.display());
        Ok(output_path)
    } else {
        Err("Audio sample extraction failed: output file not found".to_string())
    }
}

/// Extract 16 kHz mono audio chunks into an RAII-managed temporary directory.
///
/// The directory is deleted when the returned [`TempDir`] is dropped, so a
/// failed or cancelled transcription cannot leave chunk files behind.
pub(crate) async fn extract_audio_chunks_guarded(
    file: &Path,
    format: &str,
) -> Result<TempDir, String> {
    log::info!("Extract audio task start: {}", file.display());
    let output_path = file.with_extension(format);
    let output_dir = output_path
        .parent()
        .ok_or_else(|| format!("Output path has no parent: {}", output_path.display()))?;
    std::fs::create_dir_all(output_dir)
        .map_err(|error| format!("Failed to create output directory: {error}"))?;

    let duration = get_audio_duration(file).await?;
    log::info!("Audio duration: {duration} seconds");
    let chunk_duration = 30;
    let chunk_count = (duration as f64 / f64::from(chunk_duration)).ceil() as usize;
    log::info!("Splitting into {chunk_count} chunks of {chunk_duration} seconds each");

    let chunk_dir = tempdir_in(output_dir)
        .map_err(|error| format!("Failed to create chunk directory: {error}"))?;
    let base_name = output_path
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("Invalid output file name: {}", output_path.display()))?;
    let segment_pattern = chunk_dir.path().join(format!("{base_name}_%03d.{format}"));

    let mut args = vec![
        "-i".to_string(),
        file.to_string_lossy().into_owned(),
        "-ar".to_string(),
        "16000".to_string(),
        "-ac".to_string(),
        "1".to_string(),
        "-vn".to_string(),
        "-f".to_string(),
        "segment".to_string(),
        "-segment_time".to_string(),
        chunk_duration.to_string(),
        "-reset_timestamps".to_string(),
        "1".to_string(),
        "-y".to_string(),
    ];
    if format == "mp3" {
        args.extend([
            "-c:a".to_string(),
            "mp3".to_string(),
            "-b:a".to_string(),
            "64k".to_string(),
            "-compression_level".to_string(),
            "0".to_string(),
        ]);
    } else {
        args.extend(["-c:a".to_string(), "pcm_s16le".to_string()]);
    }
    args.extend(["-threads".to_string(), "0".to_string()]);

    FfmpegJob::with_args(args)
        .output(segment_pattern)
        .progress(ProgressMode::Time)
        .context("Extract audio chunks")
        .run_without_reporter()
        .await
        .map_err(|error| error.to_string())?;

    log::info!(
        "Extract audio task end: {} chunks created in {}",
        chunk_count,
        chunk_dir.path().display()
    );
    Ok(chunk_dir)
}

/// Extract the full audio track as a single 16 kHz mono WAV file.
pub async fn extract_full_audio(file: &Path) -> Result<PathBuf, String> {
    log::info!("Extract full audio: {}", file.display());
    let output_path = file.with_extension("full.wav");
    let job = FfmpegJob::new()
        .args([
            "-i".to_string(),
            file.to_string_lossy().into_owned(),
            "-ar".to_string(),
            "16000".to_string(),
            "-ac".to_string(),
            "1".to_string(),
            "-c:a".to_string(),
            "pcm_s16le".to_string(),
            "-vn".to_string(),
            "-y".to_string(),
        ])
        .output(output_path.clone())
        .progress(ProgressMode::Time)
        .context("Extract full audio");

    if let Err(error) = job.run_without_reporter().await {
        let _ = tokio::fs::remove_file(&output_path).await;
        return Err(error.to_string());
    }
    if output_path.exists() {
        log::info!("Full audio extracted: {}", output_path.display());
        Ok(output_path)
    } else {
        Err("Full audio extraction failed: output file not found".to_string())
    }
}

/// Extract a time segment as a 16 kHz mono WAV file.
pub(crate) async fn extract_audio_segment(
    file: &Path,
    start_sec: f64,
    duration_sec: f64,
    output_path: &Path,
) -> Result<(), String> {
    let job = FfmpegJob::new()
        .args([
            "-ss".to_string(),
            start_sec.to_string(),
            "-i".to_string(),
            file.to_string_lossy().into_owned(),
            "-t".to_string(),
            duration_sec.to_string(),
            "-ar".to_string(),
            "16000".to_string(),
            "-ac".to_string(),
            "1".to_string(),
            "-c:a".to_string(),
            "pcm_s16le".to_string(),
            "-vn".to_string(),
            "-y".to_string(),
        ])
        .output(output_path.to_path_buf())
        .progress(ProgressMode::Time)
        .context("Extract audio segment");

    if let Err(error) = job.run_without_reporter().await {
        let _ = tokio::fs::remove_file(output_path).await;
        return Err(error.to_string());
    }
    if output_path.exists() {
        Ok(())
    } else {
        Err("Audio segment extraction failed: output file not found".to_string())
    }
}

/// Get the duration of an audio/video file in seconds using ffprobe.
pub(crate) async fn get_audio_duration(file: &Path) -> Result<u64, String> {
    let mut command = ffprobe_command();
    let mut child = command
        .args([
            "-v",
            "quiet",
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
        ])
        .arg("-i")
        .arg(path_str(file)?)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Failed to spawn ffprobe process: {error}"))?;

    let stdout = child
        .stdout
        .take()
        .ok_or("ffprobe stdout pipe unavailable")?;
    let mut parser = FfmpegLogParser::new(BufReader::new(stdout));
    let mut duration = None;
    loop {
        match parser.parse_next_event().await {
            Ok(FfmpegEvent::LogEOF) => break,
            Ok(FfmpegEvent::Log(_, content)) => {
                if let Ok(seconds) = content.trim().parse::<f64>() {
                    duration = Some(seconds.ceil() as u64);
                }
            }
            Ok(FfmpegEvent::Error(error)) => return Err(error),
            Ok(_) => {}
            Err(error) => return Err(format!("Failed to parse ffprobe output: {error}")),
        }
    }

    let status = child
        .wait()
        .await
        .map_err(|error| format!("Failed to wait for ffprobe: {error}"))?;
    if !status.success() {
        return Err(format!("ffprobe exited with status {status}"));
    }
    duration.ok_or_else(|| "Failed to parse duration".to_string())
}
