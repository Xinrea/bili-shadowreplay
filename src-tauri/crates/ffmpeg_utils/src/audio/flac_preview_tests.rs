use super::{
    opus_tests::{assert_cancelled_media, cli, has_cli, pcm_wav},
    *,
};

use std::{collections::BTreeMap, fs, path::Path};

// The former CLI graph is deliberately kept here as the independent oracle.
fn filters(timing: AudioTiming, range: AudioRange) -> String {
    let rate = timing.sample_rate;
    let origin = timing.start;
    let first = if timing.aac {
        format!("round((STARTPTS-({origin}))/1024)*1024")
    } else {
        format!("round(STARTPTS)-({origin})")
    };
    let from = (range.start * f64::from(rate)).round() as i64;
    let to = from + (range.duration * f64::from(rate)).round() as i64;
    format!("asettb=1/{rate},asetpts='{first}+N',atrim=start_pts={from}:end_pts={to},asetpts=PTS-STARTPTS,aresample=48000,asetnsamples=n=4096:p=0")
}

async fn timing(path: &Path) -> AudioTiming {
    let audio = crate::probe_media(path).await.unwrap().audio.unwrap();
    AudioTiming {
        sample_rate: audio.sample_rate,
        start: (audio.start_time.unwrap_or(0.0) * f64::from(audio.sample_rate)).round() as i64,
        aac: audio.codec == "aac",
    }
}

fn cli_preview(
    input: &Path,
    output: &Path,
    timing: AudioTiming,
    range: AudioRange,
    pre_roll: f64,
    frame_size: Option<usize>,
) {
    let mut args = vec!["-noaccurate_seek".into(), "-copyts".into()];
    let seek = range.start - pre_roll;
    if seek > 0.0 {
        args.extend(["-ss".into(), format!("{seek:.6}")]);
    }
    args.extend([
        "-i".into(),
        input.to_str().unwrap().into(),
        "-map".into(),
        "0:a:0".into(),
        "-af".into(),
        filters(timing, range),
        "-ac".into(),
        "2".into(),
        "-c:a".into(),
        "flac".into(),
        "-sample_fmt".into(),
        "s16".into(),
        "-f".into(),
        "flac".into(),
    ]);
    if let Some(frame_size) = frame_size {
        args.extend(["-frame_size".into(), frame_size.to_string()]);
    }
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    cli(&args, output);
}

#[derive(Debug, PartialEq, Eq)]
struct CodedFrame {
    data: Vec<u8>,
    pts: Option<i64>,
    duration: i64,
}

#[derive(Debug, PartialEq, Eq)]
struct Flac {
    info: Vec<u8>,
    metadata: BTreeMap<String, String>,
    start: i64,
    packets: Vec<CodedFrame>,
    blocks: Vec<usize>,
    samples: Vec<i16>,
}

fn snapshot(path: &Path) -> Flac {
    let data = fs::read(path).unwrap();
    assert_eq!(&data[..4], b"fLaC");
    assert_eq!(data[4] & 127, 0);
    assert_eq!(&data[5..8], &[0, 0, 34]);
    let info = data[8..42].to_vec();
    let format = u64::from_be_bytes(info[10..18].try_into().unwrap());
    assert_eq!(format >> 44, 48_000);
    assert_eq!(((format >> 41) & 7) + 1, 2);
    assert_eq!(((format >> 36) & 31) + 1, 16);
    let cancelled = Arc::new(AtomicBool::new(false));
    let mut input = open_input(path, cancelled.clone()).unwrap();
    let stream = input.streams().best(media::Type::Audio).unwrap();
    assert_eq!(stream.parameters().id(), codec::Id::FLAC);
    assert_eq!(stream.time_base(), ffmpeg::Rational(1, 48_000));
    let mut result = Flac {
        info,
        metadata: input
            .metadata()
            .iter()
            .map(|(key, value)| (key.into(), value.into()))
            .collect(),
        start: stream.start_time(),
        packets: Vec::new(),
        blocks: Vec::new(),
        samples: Vec::new(),
    };
    let index = stream.index();
    let mut decoder = codec::context::Context::from_parameters(stream.parameters())
        .unwrap()
        .decoder()
        .audio()
        .unwrap();
    decode_frames(
        &mut input,
        index,
        &mut decoder,
        &cancelled,
        |packet| {
            let data = packet.data().unwrap();
            assert!(data.len() >= 4);
            assert_eq!(data[0], 0xff);
            assert_eq!(data[1] & 0xfe, 0xf8);
            assert_eq!(data[1] & 1, 0, "variable FLAC blocks break WebKit playback");
            result.packets.push(CodedFrame {
                data: data.to_vec(),
                pts: packet.pts(),
                duration: packet.duration(),
            });
            Ok(())
        },
        |frame| {
            assert_eq!(frame.rate(), 48_000);
            assert_eq!(frame.channels(), 2);
            assert_eq!(
                frame.format(),
                format::Sample::I16(format::sample::Type::Packed)
            );
            result.blocks.push(frame.samples());
            result.samples.extend_from_slice(frame.plane::<i16>(0));
            Ok(ControlFlow::Continue(()))
        },
    )
    .unwrap();
    assert_eq!(result.samples.len() % 2, 0);
    assert_eq!(format & ((1 << 36) - 1), result.samples.len() as u64 / 2);
    let block_size = u16::from_be_bytes(result.info[..2].try_into().unwrap());
    assert_eq!(
        block_size,
        u16::from_be_bytes(result.info[2..4].try_into().unwrap())
    );
    assert!(block_size >= 16);
    assert!(result
        .blocks
        .iter()
        .all(|block| *block <= usize::from(block_size)));
    if let Some((last, full)) = result.blocks.split_last() {
        assert!(*last > 0);
        assert!(full.iter().all(|block| *block == usize::from(block_size)));
    }
    result
}

fn assert_native_blocks(flac: &Flac) {
    assert!(flac.blocks.iter().all(|block| *block <= 4096));
    if let Some((_, full)) = flac.blocks.split_last() {
        assert!(full.iter().all(|block| *block == 4096));
    }
    let expected = if flac.samples.is_empty() {
        4608
    } else {
        (flac.samples.len() / 2).min(4096)
    };
    assert_eq!(
        u16::from_be_bytes(flac.info[..2].try_into().unwrap()) as usize,
        expected
    );
}

fn assert_samples(actual: &[i16], expected: &[i16], case: &str) {
    assert_eq!(actual.len(), expected.len(), "{case}: sample count");
    let mismatch = actual
        .iter()
        .zip(expected)
        .position(|(left, right)| left != right);
    assert!(
        mismatch.is_none(),
        "{case}: first sample mismatch {mismatch:?}"
    );
}

fn assert_clean(directory: &Path) {
    assert!(fs::read_dir(directory).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".preview-flac-")));
}

async fn native(input: &Path, output: &Path, start: f64, duration: f64, pre_roll: f64) -> Flac {
    encode_flac_preview(
        input,
        output,
        timing(input).await,
        AudioRange { start, duration },
        pre_roll,
    )
    .await
    .unwrap();
    let result = snapshot(output);
    assert_native_blocks(&result);
    result
}

#[tokio::test]
async fn native_pcm_preview_chunks_join_the_whole_source_with_fixed_blocks_without_a_cli() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("无缝 预览.wav");
    let output = directory.path().join("preview.flac");
    pcm_wav(&input, 48_000, 2, 100_001);
    let wav = fs::read(&input).unwrap();
    let expected: Vec<_> = wav[44..]
        .chunks_exact(2)
        .map(|sample| i16::from_le_bytes(sample.try_into().unwrap()))
        .collect();
    let first = native(&input, &output, 0.0, 1.0, 1.0).await;
    assert_samples(&first.samples, &expected[..96_000], "first chunk");
    assert_eq!(&first.info[..4], &[16, 0, 16, 0]);
    assert_eq!(first.blocks, [vec![4096; 11], vec![2944]].concat());
    let second = native(&input, &output, 1.0, 1.0, 1.0).await;
    let last = native(&input, &output, 2.0, 1.0, 1.0).await;
    assert_eq!(last.blocks, [4001]);
    assert_samples(
        &[first.samples, second.samples, last.samples].concat(),
        &expected,
        "adjacent chunks",
    );
    let eof = native(&input, &output, 4.0, 1.0, 1.0).await;
    assert!(eof.samples.is_empty());
    assert!(eof.packets.is_empty());
    assert_eq!(&eof.info[..4], &[18, 0, 18, 0]);
    assert_clean(directory.path());
}

#[tokio::test]
async fn native_recorded_aac_preview_uses_the_frame_grid_without_a_cli() {
    let directory = tempfile::tempdir().unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/video");
    let input = directory.path().join("recorded.mp4");
    let mut data = fs::read(fixtures.join("init.m4s")).unwrap();
    data.extend(fs::read(fixtures.join("segment.m4s")).unwrap());
    fs::write(&input, data).unwrap();
    let output = directory.path().join("recorded.flac");
    let timing = timing(&input).await;
    assert_eq!(
        timing,
        AudioTiming {
            sample_rate: 48_000,
            start: 9_888_240,
            aac: true
        }
    );
    // These counts were measured independently with the former CLI graph.
    let result = native(&input, &output, 0.0, 0.777, 1.0).await;
    assert_eq!(result.samples.len(), 37_296 * 2);
    assert_eq!(result.blocks, [vec![4096; 9], vec![432]].concat());
    assert!(result.samples.iter().any(|sample| *sample != 0));
    assert_clean(directory.path());
}

#[tokio::test]
async fn native_flac_empty_short_and_invalid_outputs_remain_atomic_without_a_cli() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("input.wav");
    let output = directory.path().join("kept.flac");
    for frames in [0, 16, 4095, 4096, 4097] {
        pcm_wav(&input, 48_000, 2, frames);
        let result = native(&input, &output, 0.0, 1.0, 1.0).await;
        assert_eq!(result.samples.len(), frames * 2);
    }
    for frames in [1, 15] {
        pcm_wav(&input, 48_000, 2, frames);
        fs::write(&output, b"previous preview").unwrap();
        assert!(encode_flac_preview(
            &input,
            &output,
            timing(&input).await,
            AudioRange {
                start: 0.0,
                duration: 1.0
            },
            1.0
        )
        .await
        .is_err());
        assert_eq!(fs::read(&output).unwrap(), b"previous preview");
        assert_clean(directory.path());
    }
    pcm_wav(&input, 48_000, 2, 4800);
    let timing = timing(&input).await;
    let video = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/video/test.mp4");
    let invalid = directory.path().join("invalid.wav");
    fs::write(&invalid, b"not media").unwrap();
    for source in [
        &directory.path().join("missing.wav"),
        &invalid,
        &video,
        &output,
    ] {
        assert!(encode_flac_preview(
            source,
            &output,
            timing,
            AudioRange {
                start: 0.0,
                duration: 1.0
            },
            1.0
        )
        .await
        .is_err());
        assert_eq!(fs::read(&output).unwrap(), b"previous preview");
        assert_clean(directory.path());
    }
    for (start, duration, pre_roll) in [
        (-1.0, 1.0, 1.0),
        (0.0, -1.0, 1.0),
        (f64::NAN, 1.0, 1.0),
        (0.0, f64::INFINITY, 1.0),
        (0.0, 1.0, -1.0),
    ] {
        assert!(encode_flac_preview(
            &input,
            &output,
            timing,
            AudioRange { start, duration },
            pre_roll
        )
        .await
        .is_err());
        assert_eq!(fs::read(&output).unwrap(), b"previous preview");
    }
    assert_clean(directory.path());
}

#[tokio::test]
async fn cancelled_flac_preview_closes_http_and_removes_partial_output_without_a_cli() {
    assert_cancelled_media(
        |input, output| async move {
            encode_flac_preview(
                &input,
                &output,
                AudioTiming {
                    sample_rate: 16_000,
                    start: 0,
                    aac: false,
                },
                AudioRange {
                    start: 0.0,
                    duration: 60.0,
                },
                1.0,
            )
            .await
        },
        "flac",
        ".preview-flac-",
    )
    .await;
}

async fn assert_cli(input: &Path, directory: &Path, case: &str, ranges: &[(f64, f64)]) {
    let timing = timing(input).await;
    let output = directory.join("native.flac");
    let oracle = directory.join("oracle.flac");
    for &(start, duration) in ranges {
        let range = AudioRange { start, duration };
        encode_flac_preview(input, &output, timing, range, 1.0)
            .await
            .unwrap_or_else(|error| panic!("{case} {start}/{duration}: {error}"));
        cli_preview(input, &oracle, timing, range, 1.0, None);
        let actual = snapshot(&output);
        assert_native_blocks(&actual);
        let mut expected = snapshot(&oracle);
        assert_eq!(
            actual.metadata, expected.metadata,
            "{case} {start}/{duration}: metadata"
        );
        if ffmpeg::codec::version() >> 16 == 60
            && expected.start == ffi::AV_NOPTS_VALUE
            && actual.start != expected.start
        {
            // With one final block below its default 4608, the 6.x demuxer
            // sometimes leaves stream.start_time unknown. Its actual frame
            // number and packet PTS still establish the zero sample origin.
            assert_eq!(
                expected.packets.len(),
                1,
                "{case}: unknown legacy start must be the single short-block case"
            );
            assert_eq!(expected.packets[0].pts, Some(0));
            assert_eq!(expected.packets[0].data[4], 0);
            assert_eq!(actual.start, 0);
            assert_eq!(actual.packets[0].pts, Some(0));
        } else {
            assert_eq!(
                actual.start, expected.start,
                "{case} {start}/{duration}: first timestamp"
            );
        }
        assert_samples(&actual.samples, &expected.samples, case);
        // FFmpeg 6's CLI preopens FLAC with its 4608 default; FFmpeg 9
        // chooses the first filtered frame. The native preview consistently
        // follows 9's fixed 4096 contract. The original CLI's PCM, complete
        // sample count, digest, timing and tags remain strict on both SDKs.
        assert_eq!(
            &actual.info[10..],
            &expected.info[10..],
            "{case}: format/count/MD5"
        );
        if ffmpeg::codec::version() >> 16 == 60 {
            assert_eq!(
                &expected.info[..4],
                &[18, 0, 18, 0],
                "CLI 6 default block size"
            );
            let first_filtered =
                (!expected.samples.is_empty()).then(|| (expected.samples.len() / 2).min(4096));
            cli_preview(input, &oracle, timing, range, 1.0, first_filtered);
            expected = snapshot(&oracle);
            assert_samples(&actual.samples, &expected.samples, case);
            assert_eq!(
                actual.metadata, expected.metadata,
                "{case}: normalized CLI tags"
            );
            assert_eq!(
                actual.start, expected.start,
                "{case}: normalized CLI timeline"
            );
        }
        assert_eq!(
            actual.info, expected.info,
            "{case} {start}/{duration}: STREAMINFO"
        );
        assert_eq!(
            actual.blocks, expected.blocks,
            "{case} {start}/{duration}: fixed blocks"
        );
        assert_eq!(
            actual.packets.len(),
            expected.packets.len(),
            "{case}: packet count"
        );
        for (index, (actual, expected)) in actual.packets.iter().zip(&expected.packets).enumerate()
        {
            assert!(
                actual == expected,
                "{case} {start}/{duration}: coded FLAC frame {index} differs"
            );
        }
    }
    assert_clean(directory);
}

#[tokio::test]
async fn flac_preview_matches_cli_pcm_resampling_seek_and_empty_tails() {
    if !has_cli() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("source.wav");
    for (rate, channels) in [(48_000, 2), (44_100, 1), (16_000, 2)] {
        pcm_wav(&input, rate, channels, rate as usize * 3 + 17);
        assert_cli(
            &input,
            directory.path(),
            &format!("PCM {rate}/{channels}"),
            &[
                (0.0, 0.777),
                (0.111, 0.923),
                (1.25, 1.123),
                (2.95, 0.777),
                (5.0, 0.5),
                (0.0, 0.0),
            ],
        )
        .await;
    }
    for frames in [0, 16, 4095, 4096, 4097] {
        pcm_wav(&input, 48_000, 2, frames);
        assert_cli(
            &input,
            directory.path(),
            &format!("short {frames}"),
            &[(0.0, 1.0)],
        )
        .await;
    }
}

#[tokio::test]
async fn flac_preview_uses_ffmpeg9s_short_first_block_contract_on_both_sdks() {
    if !has_cli() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("short.wav");
    let oracle = directory.path().join("oracle.flac");
    let output = directory.path().join("native.flac");
    let timing = AudioTiming {
        sample_rate: 48_000,
        start: 0,
        aac: false,
    };
    let range = AudioRange {
        start: 0.0,
        duration: 1.0,
    };
    for frames in [1, 15] {
        pcm_wav(&input, 48_000, 2, frames);
        let result = std::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-nostdin",
                "-noaccurate_seek",
                "-copyts",
                "-i",
            ])
            .arg(&input)
            .args([
                "-map",
                "0:a:0",
                "-af",
                &filters(timing, range),
                "-ac",
                "2",
                "-c:a",
                "flac",
                "-sample_fmt",
                "s16",
                "-f",
                "flac",
                "-y",
            ])
            .arg(&oracle)
            .output()
            .unwrap();
        if ffmpeg::codec::version() >> 16 == 60 {
            assert!(result.status.success());
            let legacy = snapshot(&oracle);
            assert_eq!(legacy.samples.len(), frames * 2);
            assert_eq!(&legacy.info[..4], &[18, 0, 18, 0]);
        } else {
            assert!(!result.status.success());
        }
        fs::write(&output, b"previous preview").unwrap();
        assert!(encode_flac_preview(&input, &output, timing, range, 1.0)
            .await
            .is_err());
        assert_eq!(fs::read(&output).unwrap(), b"previous preview");
        assert_clean(directory.path());
    }
}

#[tokio::test]
async fn flac_preview_matches_cli_aac_grid_mp3_flv_and_recorded_fmp4() {
    if !has_cli() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("source.wav");
    for rate in [48_000, 44_100] {
        pcm_wav(&input, rate, 2, rate as usize * 3 + 17);
        for (extension, codec) in [("m4a", "aac"), ("mp3", "libmp3lame"), ("flv", "aac")] {
            let encoded = directory.path().join(format!("{rate}.{extension}"));
            cli(&["-i", input.to_str().unwrap(), "-c:a", codec], &encoded);
            assert_cli(
                &encoded,
                directory.path(),
                &format!("{rate}.{extension}"),
                &[
                    (0.0, 0.777),
                    (0.111, 0.923),
                    (1.25, 1.123),
                    (2.95, 0.777),
                    (5.0, 0.5),
                ],
            )
            .await;
        }
    }
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/video");
    let mut data = fs::read(fixtures.join("init.m4s")).unwrap();
    data.extend(fs::read(fixtures.join("segment.m4s")).unwrap());
    let recorded = directory.path().join("recorded.mp4");
    fs::write(&recorded, data).unwrap();
    assert_cli(
        &recorded,
        directory.path(),
        "recorded fMP4",
        &[(0.0, 0.777), (0.111, 0.777), (0.99, 0.777), (5.0, 0.5)],
    )
    .await;
}

#[tokio::test]
async fn flac_preview_matches_cli_first_audio_track_metadata_and_container_origin() {
    if !has_cli() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("first.wav");
    let second = directory.path().join("second.wav");
    pcm_wav(&first, 48_000, 1, 144_017);
    pcm_wav(&second, 44_100, 2, 132_317);
    let metadata = directory.path().join("chapters.txt");
    fs::write(&metadata, ";FFMETADATA1\ntitle=Global preview\nartist=Global artist\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=500\nEND=1500\ntitle=Preview chapter\n").unwrap();
    let source = directory.path().join("tracks.mka");
    cli(
        &[
            "-i",
            first.to_str().unwrap(),
            "-i",
            second.to_str().unwrap(),
            "-i",
            metadata.to_str().unwrap(),
            "-map",
            "0:a",
            "-map",
            "1:a",
            "-map_metadata",
            "2",
            "-map_chapters",
            "2",
            "-c:a",
            "flac",
            "-disposition:a:0",
            "0",
            "-disposition:a:1",
            "default",
            "-metadata:s:a:0",
            "language=jpn",
            "-metadata:s:a:0",
            "title=First mono",
            "-metadata:s:a:1",
            "title=Default stereo",
        ],
        &source,
    );
    assert_cli(
        &source,
        directory.path(),
        "explicit first mono stream",
        &[(0.0, 0.777), (1.25, 1.123)],
    )
    .await;
    let delayed = directory.path().join("delayed.mp4");
    cli(
        &[
            "-f",
            "lavfi",
            "-i",
            "color=size=16x16:rate=10:duration=4",
            "-itsoffset",
            "1",
            "-i",
            first.to_str().unwrap(),
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
    assert_cli(
        &delayed,
        directory.path(),
        "delayed AAC",
        &[(0.0, 0.777), (1.25, 1.123), (2.95, 0.777)],
    )
    .await;
    let shifted = directory.path().join("shifted.m4a");
    cli(
        &[
            "-i",
            first.to_str().unwrap(),
            "-af",
            "asetpts=PTS+5/TB",
            "-c:a",
            "aac",
        ],
        &shifted,
    );
    assert_cli(
        &shifted,
        directory.path(),
        "nonzero container origin",
        &[(0.0, 0.777), (1.25, 1.123)],
    )
    .await;
}

#[tokio::test]
async fn flac_preview_matches_cli_when_aac_rate_and_layout_reinitialize_the_graph() {
    if !has_cli() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("source.wav");
    pcm_wav(&input, 48_000, 2, 48_000);
    for formats in [[(48_000, 1), (44_100, 2)], [(44_100, 2), (48_000, 1)]] {
        let mut data = Vec::new();
        for (rate, channels) in formats {
            let part = directory.path().join("part.aac");
            cli(
                &[
                    "-i",
                    input.to_str().unwrap(),
                    "-t",
                    "0.5",
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
        assert_cli(
            &changing,
            directory.path(),
            &format!("changing AAC {formats:?}"),
            &[(0.0, 0.777), (0.111, 0.923)],
        )
        .await;
    }
}
