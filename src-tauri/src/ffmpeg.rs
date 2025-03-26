use std::path::PathBuf;

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
    work_dir: PathBuf,
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
        .input(work_dir.join(input_path).to_str().unwrap())
        .args(["-c", "copy"])
        .args(["-y", work_dir.join(&output_path).to_str().unwrap()])
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

    Ok(TranscodeResult {
        output_path: work_dir.join(output_path),
    })
}
