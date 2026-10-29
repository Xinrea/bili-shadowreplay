use super::{opus_tests::*, *};

use std::{fs, path::Path, process::Command};

#[derive(Default)]
struct Count(u64);

impl AudioSink for Count {
    fn write(&mut self, samples: &[i16]) -> Result<(), String> {
        self.0 += samples.len() as u64;
        Ok(())
    }
}

#[tokio::test]
async fn upload_preserves_stereo_short_tails_and_empty_headers_without_a_cli() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("立体声 录播.wav");
    let output = directory.path().join("upload.opus");
    for (rate, channels, frames) in [
        (48_000, 2, 0),
        (48_000, 2, 1),
        (48_000, 2, 647),
        (48_000, 2, 648),
        (48_000, 2, 649),
        (48_000, 2, 959),
        (48_000, 2, 960),
        (48_000, 2, 961),
        (48_000, 2, 48_017),
        (16_000, 1, 1),
        (16_000, 1, 321),
    ] {
        pcm_wav(&input, rate, channels, frames);
        encode_opus_upload(&input, &output).await.unwrap();
        let encoded = ogg(&output);
        let head = &encoded.packets[0];
        assert_eq!(head.len(), 19);
        assert_eq!(u16::from(head[9]), channels);
        assert_eq!(u32::from_le_bytes(head[12..16].try_into().unwrap()), rate);
        assert_eq!(u16::from_le_bytes(head[10..12].try_into().unwrap()), 312);
        assert_eq!(head[18], 0);
        if frames == 0 {
            assert_eq!(encoded.packets.len(), 2);
            assert_eq!(encoded.pages, [(2, 0), (0, 0)]);
        } else {
            let samples = frames as u64 * 48_000 / u64::from(rate);
            assert_eq!(encoded.pages.last().unwrap(), &(4, samples + 312));
            let packets = timeline_channels(&output, channels);
            assert_eq!(packets.start, 0);
            assert_eq!(packets.duration, samples as i64 + 312);
            assert_eq!(packets.packets[0].pts, Some(-312));
            let decoded = decode_audio(
                &output,
                AudioDecodeOptions {
                    sample_rate: 48_000,
                },
                Count::default(),
            )
            .await
            .unwrap();
            assert!(decoded.has_audio);
            assert_eq!(decoded.samples, samples, "{rate}/{channels}/{frames}");
            assert_eq!(decoded.sink.0, decoded.samples);
        }
        assert_clean(directory.path());
    }
}

#[tokio::test]
async fn invalid_uploads_preserve_existing_output_without_a_cli() {
    let directory = tempfile::tempdir().unwrap();
    let invalid = directory.path().join("invalid.wav");
    fs::write(&invalid, b"not media").unwrap();
    let unsupported = directory.path().join("unsupported.wav");
    // FFmpeg's default three-channel layout is 2.1, which libopus rejects.
    pcm_wav(&unsupported, 48_000, 3, 1_001);
    let video = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/video/test.mp4");
    let output = directory.path().join("kept.opus");
    fs::write(&output, b"previous upload").unwrap();
    for source in [
        &invalid,
        &directory.path().join("missing.wav"),
        &unsupported,
        &video,
        &output,
    ] {
        assert!(encode_opus_upload(source, &output).await.is_err());
        assert_eq!(fs::read(&output).unwrap(), b"previous upload");
        assert_clean(directory.path());
    }
}

#[tokio::test]
async fn cancelled_upload_closes_http_and_removes_partial_output_without_a_cli() {
    assert_cancelled_opus(|input, output| async move { encode_opus_upload(&input, &output).await })
        .await;
}

async fn assert_cli_upload(input: &Path, directory: &Path, case: &str, channels: u16) {
    let output = directory.join("upload.opus");
    let oracle = directory.join("oracle.opus");
    encode_opus_upload(input, &output)
        .await
        .unwrap_or_else(|error| panic!("{case}: {error}"));
    cli(
        &[
            "-i",
            input.to_str().unwrap(),
            "-vn",
            "-acodec",
            "libopus",
            "-b:a",
            "128k",
        ],
        &oracle,
    );
    assert_equivalent(&output, &oracle, case, channels);
    assert_clean(directory);
}

#[tokio::test]
async fn upload_matches_cli_rate_negotiation_mono_stereo_surround_and_float() {
    if !has_cli() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("source.wav");
    for (rate, channels) in [
        (48_000, 1),
        (48_000, 2),
        (44_100, 2),
        (32_000, 1),
        (20_000, 2),
        (11_025, 1),
        (48_000, 6),
    ] {
        pcm_wav(&input, rate, channels, rate as usize / 4 + 17);
        assert_cli_upload(
            &input,
            directory.path(),
            &format!("PCM {rate}/{channels}"),
            channels,
        )
        .await;
    }
    let float = directory.path().join("float.wav");
    pcm_wav(&input, 44_100, 2, 11_039);
    cli(
        &["-i", input.to_str().unwrap(), "-c:a", "pcm_f32le"],
        &float,
    );
    assert_cli_upload(&float, directory.path(), "float stereo", 2).await;
    for (rate, channels) in [(32_000, 1), (48_000, 6)] {
        pcm_wav(&input, rate, channels, 0);
        assert_cli_upload(
            &input,
            directory.path(),
            &format!("empty PCM {rate}/{channels}"),
            channels,
        )
        .await;
    }
}

#[tokio::test]
async fn upload_rejects_the_same_unsupported_layouts_as_cli() {
    if !has_cli() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("source.wav");
    let output = directory.path().join("upload.opus");
    for (layout, channels) in [("2.1", 3), ("5.1(side)", 6)] {
        for frames in [0, 1_001] {
            let source = directory.path().join("raw.wav");
            pcm_wav(&source, 48_000, channels, frames);
            cli(
                &[
                    "-i",
                    source.to_str().unwrap(),
                    "-channel_layout",
                    layout,
                    "-c:a",
                    "pcm_s16le",
                ],
                &input,
            );
            let oracle = directory.path().join("oracle.opus");
            let result = Command::new("ffmpeg")
                .args(["-v", "error", "-nostdin", "-y", "-i"])
                .arg(&input)
                .args(["-vn", "-acodec", "libopus", "-b:a", "128k"])
                .arg(&oracle)
                .output()
                .unwrap();
            assert!(!result.status.success(), "CLI accepted {layout}/{frames}");
            fs::write(&output, b"previous upload").unwrap();
            assert!(
                encode_opus_upload(&input, &output).await.is_err(),
                "native accepted {layout}/{frames}"
            );
            assert_eq!(fs::read(&output).unwrap(), b"previous upload");
            assert_clean(directory.path());
        }
    }
}

#[tokio::test]
async fn upload_matches_cli_aac_mp3_flv_fmp4_and_automatic_track_selection() {
    if !has_cli() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("source.wav");
    pcm_wav(&input, 44_100, 2, 44_113);
    for (extension, codec) in [("m4a", "aac"), ("mp3", "libmp3lame"), ("flv", "aac")] {
        let compressed = directory.path().join(format!("source.{extension}"));
        cli(&["-i", input.to_str().unwrap(), "-c:a", codec], &compressed);
        assert_cli_upload(&compressed, directory.path(), extension, 2).await;
    }
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/video");
    let mut data = fs::read(fixtures.join("init.m4s")).unwrap();
    data.extend(fs::read(fixtures.join("segment.m4s")).unwrap());
    let recorded = directory.path().join("recorded.mp4");
    fs::write(&recorded, data).unwrap();
    assert_cli_upload(&recorded, directory.path(), "recorded fMP4", 2).await;

    let mono = directory.path().join("mono.wav");
    pcm_wav(&mono, 32_000, 1, 32_017);
    let tracks = directory.path().join("tracks.mka");
    cli(
        &[
            "-i",
            input.to_str().unwrap(),
            "-i",
            mono.to_str().unwrap(),
            "-map",
            "0:a",
            "-map",
            "1:a",
            "-c:a",
            "flac",
            "-disposition:a:0",
            "0",
            "-disposition:a:1",
            "default",
            "-metadata",
            "title=Archive upload",
            "-metadata:s:a:1",
            "language=jpn",
            "-metadata:s:a:1",
            "title=Selected mono",
        ],
        &tracks,
    );
    assert_cli_upload(
        &tracks,
        directory.path(),
        "default mono track overrides stereo",
        1,
    )
    .await;
}

#[tokio::test]
async fn upload_matches_cli_delayed_audio_and_timestamp_gaps() {
    if !has_cli() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("source.wav");
    pcm_wav(&input, 48_000, 2, 96_017);
    let delayed = directory.path().join("delayed.mp4");
    cli(
        &[
            "-f",
            "lavfi",
            "-i",
            "color=size=16x16:rate=10:duration=3",
            "-itsoffset",
            "1",
            "-i",
            input.to_str().unwrap(),
            "-map",
            "0:v",
            "-map",
            "1:a",
            "-c:v",
            "mpeg4",
            "-c:a",
            "aac",
        ],
        &delayed,
    );
    assert_cli_upload(&delayed, directory.path(), "delayed stereo", 2).await;
    let gap = directory.path().join("gap.m4a");
    cli(
        &[
            "-i",
            input.to_str().unwrap(),
            "-af",
            "asetpts=PTS+gte(T\\,1)/TB",
            "-c:a",
            "aac",
        ],
        &gap,
    );
    assert_cli_upload(&gap, directory.path(), "stereo timestamp gap", 2).await;
}

#[tokio::test]
async fn upload_matches_cli_when_adts_rate_or_layout_changes_midstream() {
    if !has_cli() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("source.wav");
    pcm_wav(&input, 48_000, 2, 48_000);
    for ((first_rate, first_channels), (second_rate, second_channels)) in [
        ((32_000, 1), (48_000, 2)),
        ((48_000, 2), (32_000, 1)),
        ((48_000, 1), (48_000, 2)),
        ((48_000, 2), (48_000, 1)),
    ] {
        let mut data = Vec::new();
        for (rate, channels) in [(first_rate, first_channels), (second_rate, second_channels)] {
            let part = directory.path().join("part.aac");
            cli(
                &[
                    "-i",
                    input.to_str().unwrap(),
                    "-t",
                    "0.35",
                    "-ar",
                    &rate.to_string(),
                    "-ac",
                    &channels.to_string(),
                    "-c:a",
                    "aac",
                    "-f",
                    "adts",
                ],
                &part,
            );
            data.extend(fs::read(&part).unwrap());
        }
        let changing = directory.path().join("changing.aac");
        fs::write(&changing, data).unwrap();
        assert_cli_upload(
            &changing,
            directory.path(),
            &format!("ADTS {first_rate}/{first_channels} to {second_rate}/{second_channels}"),
            first_channels,
        )
        .await;
    }
}
