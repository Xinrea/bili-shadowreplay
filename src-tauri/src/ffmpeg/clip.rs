use std::path::Path;

use crate::progress::progress_reporter::ProgressReporterTrait;
use ffmpeg_utils::ffmpeg_command;

use super::{
    hwaccel,
    runner::{run_command, FfmpegJob, ProgressMode},
};

/// Re-encode or remux a video into `output_path`.
pub async fn transcode<R: ProgressReporterTrait>(
    reporter: Option<&R>,
    file: &Path,
    output_path: &Path,
    copy_codecs: bool,
) -> Result<(), String> {
    log::info!("Transcode: {} copy: {}", file.display(), copy_codecs);
    let mut command = ffmpeg_command();
    command.arg("-i").arg(file);

    if copy_codecs {
        command.args(["-c:v", "copy", "-c:a", "copy"]);
    } else {
        let video_encoder = hwaccel::get_x264_encoder().await;
        hwaccel::apply_x264_encoder_args(
            &mut command,
            video_encoder,
            Some(hwaccel::H264_SCALE_PAD_FILTER),
        );
        command.args(["-c:a", "aac"]);
        hwaccel::apply_x264_quality_args(&mut command, video_encoder);
        command.args(["-threads", "0"]);
    }

    command.args(["-y"]).arg(output_path);
    run_command(
        command,
        ProgressMode::prefixed("压制中："),
        "Transcode",
        reporter,
    )
    .await
    .map(|_| ())
    .map_err(|error| error.to_string())
}

/// Trim a video without re-encoding its streams.
pub async fn trim_video<R: ProgressReporterTrait>(
    reporter: Option<&R>,
    file: &Path,
    output_path: &Path,
    start_time: f64,
    duration: f64,
) -> Result<(), String> {
    log::info!("Trim video task start: {}", file.display());
    let job = FfmpegJob::new()
        .args([
            "-ss".to_string(),
            start_time.to_string(),
            "-i".to_string(),
            file.to_string_lossy().into_owned(),
            "-t".to_string(),
            duration.to_string(),
            "-c".to_string(),
            "copy".to_string(),
            "-y".to_string(),
        ])
        .output(output_path.to_path_buf())
        .progress(ProgressMode::prefixed("切片中："))
        .context("Trim video");
    job.run(reporter)
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// Cut a video range and encode it with the selected hardware/software encoder.
pub async fn clip_from_video_file<R: ProgressReporterTrait>(
    reporter: Option<&R>,
    input_path: &Path,
    output_path: &Path,
    start_time: f64,
    duration: f64,
) -> Result<(), String> {
    let output_folder = output_path
        .parent()
        .ok_or_else(|| format!("Output path has no parent: {}", output_path.display()))?;
    std::fs::create_dir_all(output_folder)
        .map_err(|error| format!("Failed to create output directory: {error}"))?;

    let mut command = ffmpeg_command();
    let video_encoder = hwaccel::get_x264_encoder().await;
    command.arg("-i").arg(input_path).args([
        "-ss",
        &start_time.to_string(),
        "-t",
        &duration.to_string(),
    ]);
    hwaccel::apply_x264_encoder_args(&mut command, video_encoder, None);
    command.args(["-c:a", "aac"]);
    hwaccel::apply_x264_quality_args(&mut command, video_encoder);
    command
        .args(["-avoid_negative_ts", "make_zero", "-y"])
        .arg(output_path);

    run_command(
        command,
        ProgressMode::prefixed("切片进度: "),
        "切片",
        reporter,
    )
    .await
    .map(|_| ())
    .map_err(|error| error.to_string())
}
