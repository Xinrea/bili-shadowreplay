use std::path::{Path, PathBuf};

use ffmpeg_sidecar::{
    command::FfmpegCommand,
    event::{FfmpegEvent, LogLevel},
};
use tauri::AppHandle;

use crate::progress_event::emit_progress_update;

pub struct TranscodeConfig {
    pub input_path: PathBuf,
    pub input_format: String,
    pub output_path: PathBuf,
}

pub struct TranscodeResult {
    pub output_path: PathBuf,
}

pub fn transcode(
    app_handle: &AppHandle,
    event_id: &str,
    config: TranscodeConfig,
) -> Result<TranscodeResult, String> {
    let input_path = config.input_path;
    let input_format = config.input_format;
    let output_path = config.output_path;

    log::info!(
        "Transcode task start: input_path: {}, output_path: {}",
        input_path.display(),
        output_path.display()
    );

    FfmpegCommand::new()
        .args(["-f", input_format.as_str()])
        .input(input_path.to_str().unwrap())
        .args(["-c", "copy"])
        .args(["-y", output_path.to_str().unwrap()])
        .spawn()
        .unwrap()
        .iter()
        .unwrap()
        .for_each(|e| match e {
            FfmpegEvent::Log(LogLevel::Error, e) => println!("Error: {}", e),
            FfmpegEvent::Progress(p) => emit_progress_update(
                app_handle,
                event_id,
                format!("修复编码中：{}", p.time).as_str(),
                "",
            ),
            _ => {}
        });

    log::info!(
        "Transcode task end: output_path: {}",
        &output_path.display()
    );

    Ok(TranscodeResult { output_path })
}

pub async fn extract_audio(file: &Path) -> Result<(), String> {
    // ffmpeg -i fixed_\[30655190\]1742887114_0325084106_81.5.mp4 -ar 16000 test.wav
    log::info!("Extract audio task start: {}", file.display());
    let output_path = file.with_extension("wav");
    FfmpegCommand::new()
        .args(["-i", file.to_str().unwrap()])
        .args(["-ar", "16000"])
        .args([output_path.to_str().unwrap()])
        .args(["-y"])
        .spawn()
        .unwrap()
        .iter()
        .unwrap()
        .for_each(|e| {
            if let FfmpegEvent::Log(LogLevel::Error, e) = e {
                println!("Error: {}", e);
            }
        });

    log::info!("Extract audio task end: {}", output_path.display());
    Ok(())
}

pub async fn encode_video_subtitle(
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

    FfmpegCommand::new()
        .args(["-i", file.to_str().unwrap()])
        .args([
            "-vf",
            format!(
                "subtitles='{}':force_style='{}'",
                subtitle.to_str().unwrap(),
                srt_style
            )
            .as_str(),
        ])
        .args(["-c:v", "libx264"])
        .args(["-c:a", "copy"])
        .args([output_path.to_str().unwrap()])
        .args(["-y"])
        .spawn()
        .unwrap()
        .iter()
        .unwrap()
        .for_each(|e| {
            if let FfmpegEvent::Log(LogLevel::Error, e) = e {
                log::error!("Error: {}", e);
                command_error = Some(e.to_string());
            }
        });

    log::info!("Encode video subtitle task end: {}", output_path.display());

    if let Some(error) = command_error {
        Err(error)
    } else {
        Ok(output_filename)
    }
}
