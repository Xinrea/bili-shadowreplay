use super::*;
use crate::progress::progress_reporter::ProgressReporter;
use crate::project::{EncodingPriority, RenderResolution, RenderSettings, VideoCodec};

async fn cli(args: &[&str], output: &Path) {
    let result = ffmpeg_command()
        .args(["-v", "error", "-nostdin", "-y"])
        .args(args)
        .arg(output)
        .output()
        .await
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

async fn fragment(path: &Path, size: &str, color: &str, duration: f64, start: f64, filter: &str) {
    cli(
        &[
            "-f",
            "lavfi",
            "-i",
            &format!("color=c={color}:s={size}:r=20:d={duration}"),
            "-f",
            "lavfi",
            "-i",
            &format!("sine=sample_rate=48000:duration={duration}"),
            "-vf",
            filter,
            "-c:v",
            "libx264",
            "-threads:v",
            "1",
            "-bf",
            "2",
            "-c:a",
            "aac",
            "-output_ts_offset",
            &start.to_string(),
            "-f",
            "mpegts",
        ],
        path,
    )
    .await;
}

async fn colors(path: &Path) -> (usize, usize) {
    let result = ffmpeg_command()
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args([
            "-vf",
            "crop=2:2:(iw-2)/2:(ih-2)/2,format=rgb24",
            "-fps_mode",
            "passthrough",
            "-enc_time_base",
            "1:90000",
            "-f",
            "rawvideo",
            "-",
        ])
        .output()
        .await
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        result.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let (frames, remainder) = result.stdout.as_chunks::<12>();
    assert!(remainder.is_empty());
    let mut red = 0;
    let mut blue = 0;
    for frame in frames {
        if frame[0] > 200 && frame[2] < 30 {
            red += 1;
        } else if frame[2] > 200 && frame[0] < 30 {
            blue += 1;
        } else {
            panic!("unexpected pixel {:?}", &frame[..3]);
        }
    }
    (red, blue)
}

fn settings() -> RenderSettings {
    RenderSettings {
        codec: VideoCodec::H264,
        priority: EncodingPriority::Quality,
        bitrate_kbps: 1000,
        resolution: RenderResolution::Original,
    }
}

async fn pcm(path: &Path) -> Vec<i16> {
    let result = ffmpeg_command()
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args(["-vn", "-ac", "1", "-ar", "48000", "-f", "s16le", "-"])
        .output()
        .await
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        result.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    result
        .stdout
        .as_chunks::<2>()
        .0
        .iter()
        .map(|sample| i16::from_le_bytes(*sample))
        .collect()
}

#[tokio::test]
async fn whole_clip_normalization_does_not_insert_aac_preroll_at_joins() {
    let directory = tempfile::tempdir().unwrap();
    let mut sources = Vec::new();
    for index in 0..3 {
        let source = directory.path().join(format!("{index}.mov"));
        let size = if index == 1 { "90x160" } else { "192x144" };
        cli(
            &[
                "-f",
                "lavfi",
                "-i",
                &format!("color=c=blue:s={size}:r=20:d=1"),
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000:duration=1",
                "-c:v",
                "libx264",
                "-threads:v",
                "1",
                "-c:a",
                "pcm_f32le",
            ],
            &source,
        )
        .await;
        sources.push(source);
    }
    let output = directory.path().join("joined.mp4");
    concat_recordings_with_settings(
        None::<&ProgressReporter>,
        &sources,
        &output,
        Some(&settings()),
        false,
    )
    .await
    .unwrap();
    let oracle = directory.path().join("continuous.m4a");
    cli(
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=3",
            "-ac",
            "2",
            "-c:a",
            "aac",
        ],
        &oracle,
    )
    .await;
    let actual = pcm(&output).await;
    let expected = pcm(&oracle).await;
    assert_eq!(expected.len(), 144_000);
    assert_eq!(
        actual.len(),
        expected.len(),
        "AAC pre-roll must not appear at joins"
    );
    let rms = (actual
        .iter()
        .zip(&expected)
        .map(|(a, b)| (f64::from(*a) - f64::from(*b)).powi(2))
        .sum::<f64>()
        / actual.len() as f64)
        .sqrt();
    assert!(
        rms < 100.0,
        "audio differs from a continuously encoded tone: RMS {rms}"
    );
}

#[tokio::test]
async fn whole_clip_normalizes_each_changing_recording_before_joining() {
    let directory = tempfile::tempdir().unwrap();
    let mut sources = Vec::new();
    for index in 0..7 {
        let source = directory.path().join(format!("{index}.ts"));
        let mp4 = directory.path().join(format!("{index}.mp4"));
        let color = if index % 2 == 0 { "red" } else { "blue" };
        if index == 0 {
            fragment(&source, "192x144", color, 4.0, 30.0, "setsar=1").await;
        } else {
            let body = directory.path().join(format!("{index}-body.ts"));
            let start = 30.0 + index as f64 * 100.0;
            let split = if index == 6 { 1.9 } else { 0.1 };
            let head_filter = if index == 4 {
                "setsar=1,setparams=colorspace=bt470bg:range=limited"
            } else {
                "setsar=1"
            };
            fragment(&source, "90x160", color, split, start, head_filter).await;
            let (size, filter) = match index {
                1 => ("90x160", "setsar=2"),
                3 => ("90x160", "setsar=1,format=yuv444p"),
                4 => (
                    "90x160",
                    "setsar=1,setparams=colorspace=bt709:range=limited",
                ),
                _ => ("120x120", "setsar=1"),
            };
            fragment(&body, size, color, 2.0 - split, start + split, filter).await;
            let bytes = [
                std::fs::read(&source).unwrap(),
                std::fs::read(body).unwrap(),
            ]
            .concat();
            std::fs::write(&source, bytes).unwrap();
        }
        super::super::transcode(None::<&ProgressReporter>, &source, &mp4, true, None)
            .await
            .unwrap();
        if index == 5 {
            let rotated = directory.path().join("rotated.mp4");
            cli(
                &[
                    "-display_rotation",
                    "90",
                    "-i",
                    mp4.to_str().unwrap(),
                    "-c",
                    "copy",
                ],
                &rotated,
            )
            .await;
            let original = ffmpeg_utils::probe_media(&mp4).await.unwrap();
            let rotated_probe = ffmpeg_utils::probe_media(&rotated).await.unwrap();
            assert_eq!(
                rotated_probe.display_dimensions,
                (original.display_dimensions.1, original.display_dimensions.0)
            );
            sources.push(rotated);
        } else {
            sources.push(mp4);
        }
    }
    for (count, policy) in [(2, Some(settings())), (7, Some(settings())), (7, None)] {
        let output = directory
            .path()
            .join(format!("joined-{count}-{}.mp4", policy.is_some()));
        concat_recordings_with_settings(
            None::<&ProgressReporter>,
            &sources[..count],
            &output,
            policy.as_ref(),
            false,
        )
        .await
        .unwrap();
        assert_eq!(
            colors(&output).await,
            if count == 2 { (80, 40) } else { (200, 120) }
        );
        let metadata = ffmpeg_utils::extract_video_metadata(&output).await.unwrap();
        assert_eq!((metadata.width, metadata.height), (192, 144));
        let expected = if count == 2 { 6.0 } else { 16.0 };
        assert!((metadata.duration - expected).abs() < 0.8, "{metadata:?}");
    }
}

#[tokio::test]
async fn whole_clip_normalization_keeps_mixed_frame_rates_and_supplies_silence() {
    let directory = tempfile::tempdir().unwrap();
    let mut sources = Vec::new();
    for (index, fps) in [20, 25, 30].into_iter().enumerate() {
        let source = directory.path().join(format!("{index}.mp4"));
        let color = if index == 1 { "blue" } else { "red" };
        let size = if index == 1 { "90x160" } else { "192x144" };
        let video = format!("color=c={color}:s={size}:r={fps}:d=1");
        let mut args = vec!["-f", "lavfi", "-i", &video];
        if index != 1 {
            args.extend(["-f", "lavfi", "-i", "sine=duration=1:sample_rate=44100"]);
        }
        args.extend(["-c:v", "libx264", "-threads:v", "1", "-c:a", "aac"]);
        cli(&args, &source).await;
        sources.push(source);
    }
    let output = directory.path().join("joined.mp4");
    concat_recordings_with_settings(
        None::<&ProgressReporter>,
        &sources,
        &output,
        Some(&settings()),
        false,
    )
    .await
    .unwrap();
    assert_eq!(colors(&output).await, (50, 25));
    let metadata = ffmpeg_utils::extract_video_metadata(&output).await.unwrap();
    assert!((metadata.duration - 3.0).abs() < 0.3, "{metadata:?}");
    assert_eq!(metadata.audio_codec, "aac");
}
