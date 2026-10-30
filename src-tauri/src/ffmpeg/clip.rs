use std::{num::NonZeroU32, path::Path};

use crate::progress::progress_reporter::ProgressReporterTrait;
use ffmpeg_utils::ffmpeg_command;

use super::{
    hwaccel,
    runner::{run_command, FfmpegJob, ProgressMode, ProgressTotal},
};

/// Re-encode or remux a video into `output_path`. A re-encode targets
/// `bitrate` (kbps) when given, otherwise the encoder's default quality.
pub async fn transcode<R: ProgressReporterTrait>(
    reporter: Option<&R>,
    file: &Path,
    output_path: &Path,
    copy_codecs: bool,
    bitrate: Option<NonZeroU32>,
) -> Result<(), String> {
    transcode_with_settings(reporter, file, output_path, copy_codecs, bitrate, None).await
}

pub async fn transcode_with_settings<R: ProgressReporterTrait>(
    reporter: Option<&R>,
    file: &Path,
    output_path: &Path,
    copy_codecs: bool,
    bitrate: Option<NonZeroU32>,
    settings: Option<&crate::project::RenderSettings>,
) -> Result<(), String> {
    if let Some(settings) = settings {
        settings.validate()?;
    }
    let copy_codecs = copy_codecs && settings.is_none();
    log::info!("Transcode: {} copy: {}", file.display(), copy_codecs);
    let mut command = ffmpeg_command();
    command.arg("-i").arg(file);

    if copy_codecs {
        command.args(["-c:v", "copy", "-c:a", "copy"]);
    } else {
        let video_encoder = hwaccel::material_encoder(settings).await?;
        hwaccel::apply_x264_encoder_args(
            &mut command,
            video_encoder,
            Some(hwaccel::H264_SCALE_PAD_FILTER),
        );
        command.args(["-c:a", "aac"]);
        hwaccel::apply_material_quality(&mut command, video_encoder, bitrate, settings);
        command.args(["-threads", "0"]);
    }

    command.args(["-y"]).arg(output_path);
    let stage = if copy_codecs {
        "转封装"
    } else {
        "重新编码"
    };
    run_command(
        command,
        ProgressMode::stage(stage, ProgressTotal::FirstInput),
        "Transcode",
        reporter,
    )
    .await
    .map(|_| ())
    .map_err(|error| error.to_string())
}

#[cfg(test)]
mod encoding_tests {
    use super::*;
    use crate::project::{RenderSettings, VideoCodec};

    #[tokio::test]
    async fn explicit_material_settings_encode_codec_instead_of_copying() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.mp4");
        let output = ffmpeg_command()
            .args([
                "-f",
                "lavfi",
                "-i",
                "testsrc2=s=320x180:r=30:d=0.3",
                "-f",
                "lavfi",
                "-i",
                "sine=duration=0.3",
                "-c:v",
                "libx264",
                "-c:a",
                "aac",
                "-y",
            ])
            .arg(&source)
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let mut input = source;
        for (codec, name) in [(VideoCodec::Hevc, "hevc"), (VideoCodec::H264, "h264")] {
            let settings = RenderSettings {
                codec,
                ..RenderSettings::default()
            };
            let destination = dir.path().join(format!("{name}.mp4"));
            transcode_with_settings(
                None::<&crate::progress::progress_reporter::ProgressReporter>,
                &input,
                &destination,
                true, // Explicit settings must override a copy request.
                None,
                Some(&settings),
            )
            .await
            .unwrap();
            let metadata = ffmpeg_utils::extract_video_metadata(&destination)
                .await
                .unwrap();
            assert_eq!(metadata.video_codec, name);
            input = destination;
        }
    }

    #[tokio::test]
    async fn material_join_uses_explicit_codec_and_priority() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.mp4");
        let output = ffmpeg_command()
            .args([
                "-f",
                "lavfi",
                "-i",
                "testsrc2=s=320x180:r=30:d=0.3",
                "-f",
                "lavfi",
                "-i",
                "sine=duration=0.3",
                "-c:v",
                "libx264",
                "-c:a",
                "aac",
                "-y",
            ])
            .arg(&source)
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let destination = dir.path().join("joined.mp4");
        let settings = RenderSettings {
            priority: crate::project::EncodingPriority::Quality,
            bitrate_kbps: 4000,
            ..RenderSettings::default()
        };
        super::super::general::concat_videos_with_settings(
            None::<&crate::progress::progress_reporter::ProgressReporter>,
            &[source.clone(), source],
            &destination,
            None,
            NonZeroU32::new(8000), // New settings override the legacy bitrate.
            Some(&settings),
        )
        .await
        .unwrap();
        let metadata = ffmpeg_utils::extract_video_metadata(&destination)
            .await
            .unwrap();
        assert_eq!(metadata.video_codec, "hevc");
        assert!(metadata.duration >= 0.5);
    }
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
        .progress(ProgressMode::stage("裁剪", ProgressTotal::Known(duration)))
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
        ProgressMode::stage("切片", ProgressTotal::Known(duration)),
        "切片",
        reporter,
    )
    .await
    .map(|_| ())
    .map_err(|error| error.to_string())
}
