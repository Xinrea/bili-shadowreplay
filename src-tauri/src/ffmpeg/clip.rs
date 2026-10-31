use std::{num::NonZeroU32, path::Path};

use crate::progress::progress_reporter::ProgressReporterTrait;
use ffmpeg_utils::{
    AudioEncodeOptions, InputSeek, MediaInput, MediaOutput, MediaPlan, NegativeTs, StreamMode,
    StreamRoute, StreamSelector, SubtitleEncodeOptions,
};

use super::{
    hwaccel,
    runner::{run_native_media, ProgressMode, ProgressTotal},
};

pub(super) fn single_input_plan(
    input: &Path,
    output: &Path,
    video: StreamMode,
    audio: StreamMode,
) -> MediaPlan {
    MediaPlan {
        inputs: vec![MediaInput::file(input)],
        outputs: vec![MediaOutput::file(output)],
        routes: vec![
            StreamRoute {
                source: ffmpeg_utils::StreamSource::Input {
                    input: 0,
                    selector: StreamSelector::AutoVideo,
                },
                output: 0,
                required: false,
                mode: video,
            },
            StreamRoute {
                source: ffmpeg_utils::StreamSource::Input {
                    input: 0,
                    selector: StreamSelector::AutoAudio,
                },
                output: 0,
                required: false,
                mode: audio,
            },
            StreamRoute {
                source: ffmpeg_utils::StreamSource::Input {
                    input: 0,
                    selector: StreamSelector::AutoSubtitle,
                },
                output: 0,
                required: false,
                mode: StreamMode::Subtitle(SubtitleEncodeOptions::default()),
            },
        ],
        ..Default::default()
    }
}

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
    let plan = if copy_codecs {
        single_input_plan(file, output_path, StreamMode::Copy, StreamMode::Copy)
    } else {
        let video_encoder = hwaccel::material_encoder(settings).await;
        let mut video = hwaccel::native_video_options(
            video_encoder,
            Some(hwaccel::H264_SCALE_PAD_FILTER),
            bitrate,
            settings,
        )?;
        video.threads = Some(0);
        single_input_plan(
            file,
            output_path,
            StreamMode::Video(video),
            StreamMode::Audio(AudioEncodeOptions::default()),
        )
    };
    let stage = if copy_codecs {
        "转封装"
    } else {
        "重新编码"
    };
    run_native_media(
        plan,
        ProgressMode::stage(stage, ProgressTotal::FirstInput),
        "Transcode",
        reporter,
    )
    .await
    .map(|_| ())
}

#[cfg(test)]
mod encoding_tests {
    use super::*;
    use crate::project::{RenderSettings, VideoCodec};
    use ffmpeg_utils::ffmpeg_command;

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
    let mut plan = single_input_plan(file, output_path, StreamMode::Copy, StreamMode::Copy);
    plan.inputs[0].seek = Some(InputSeek {
        seconds: start_time,
        accurate: true,
    });
    plan.outputs[0].duration = Some(duration);
    // Preserve automatic subtitles when the target muxer accepts their type.
    for route in &mut plan.routes {
        if matches!(
            &route.source,
            ffmpeg_utils::StreamSource::Input {
                selector: StreamSelector::AutoSubtitle,
                ..
            }
        ) {
            route.mode = StreamMode::Copy;
        }
    }
    run_native_media(
        plan,
        ProgressMode::stage("裁剪", ProgressTotal::Known(duration)),
        "Trim video",
        reporter,
    )
    .await
    .map(|_| ())
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

    let video_encoder = hwaccel::get_x264_encoder().await;
    let video = hwaccel::native_video_options(video_encoder, None, None, None)?;
    let mut plan = single_input_plan(
        input_path,
        output_path,
        StreamMode::Video(video),
        StreamMode::Audio(AudioEncodeOptions::default()),
    );
    plan.outputs[0].seek = Some(start_time);
    plan.outputs[0].duration = Some(duration);
    plan.outputs[0].negative_ts = NegativeTs::MakeZero;
    run_native_media(
        plan,
        ProgressMode::stage("切片", ProgressTotal::Known(duration)),
        "切片",
        reporter,
    )
    .await
    .map(|_| ())
}

#[cfg(test)]
mod native_clip_tests {
    use super::*;

    #[tokio::test]
    async fn native_remux_and_trim_read_real_recordings_without_cli() {
        let directory = tempfile::tempdir().unwrap();
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/video");
        let source = directory.path().join("录播 素材.mp4");
        let mut data = std::fs::read(fixtures.join("init.m4s")).unwrap();
        data.extend(std::fs::read(fixtures.join("segment.m4s")).unwrap());
        std::fs::write(&source, data).unwrap();
        let output = directory.path().join("copy.mp4");
        transcode(
            None::<&crate::progress::progress_reporter::ProgressReporter>,
            &source,
            &output,
            true,
            None,
        )
        .await
        .unwrap();
        let metadata = ffmpeg_utils::extract_video_metadata(&output).await.unwrap();
        assert_eq!(metadata.video_codec, "h264");
        assert_eq!(metadata.audio_codec, "aac");
        let trimmed = directory.path().join("trimmed.mp4");
        trim_video(
            None::<&crate::progress::progress_reporter::ProgressReporter>,
            &source,
            &trimmed,
            0.111,
            0.777,
        )
        .await
        .unwrap();
        let metadata = ffmpeg_utils::extract_video_metadata(&trimmed)
            .await
            .unwrap();
        assert_eq!(metadata.video_codec, "h264");
        assert_eq!(metadata.audio_codec, "aac");
        assert!(metadata.duration > 0.0);
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 3);
    }

    #[tokio::test]
    async fn native_clip_failures_leave_existing_output_intact() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("missing.mp4");
        let output = directory.path().join("kept.mp4");
        std::fs::write(&output, b"previous video").unwrap();
        assert!(transcode(
            None::<&crate::progress::progress_reporter::ProgressReporter>,
            &source,
            &output,
            true,
            None
        )
        .await
        .is_err());
        assert_eq!(std::fs::read(&output).unwrap(), b"previous video");
        assert!(trim_video(
            None::<&crate::progress::progress_reporter::ProgressReporter>,
            &source,
            &output,
            0.0,
            0.5
        )
        .await
        .is_err());
        assert_eq!(std::fs::read(&output).unwrap(), b"previous video");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
