use super::{
    file_test_helpers::*,
    opus_tests::{assert_cancelled_media, cli, has_cli, pcm_wav},
    *,
};

use std::{fs, path::Path};

fn info_tag(path: &Path) -> Vec<u8> {
    let data = fs::read(path).unwrap();
    assert_eq!(&data[..3], b"ID3");
    let start = 10
        + data[6..10]
            .iter()
            .fold(0usize, |size, byte| (size << 7) | usize::from(*byte));
    let offset = data[start..]
        .windows(4)
        .position(|bytes| bytes == b"Xing" || bytes == b"Info")
        .unwrap()
        + start;
    let flags = u32::from_be_bytes(data[offset + 4..offset + 8].try_into().unwrap());
    let mut end = offset + 8;
    for (flag, length) in [(1, 4), (2, 4), (4, 100), (8, 4)] {
        if flags & flag != 0 {
            end += length;
        }
    }
    assert!(data[end..end + 9].starts_with(b"Lavc"));
    data[offset..end + 36].to_vec()
}

fn assert_clean(directory: &Path) {
    assert!(fs::read_dir(directory).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with('.')));
}

#[tokio::test]
async fn native_mp3_keeps_stereo_priming_and_short_tail_contract_without_a_cli() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("导出 样本.wav");
    let output = directory.path().join("audio.mp3");
    // Independent CLI baselines include MP3's minimum decoded final span.
    for (frames, decoded_frames) in [
        (1, 47),
        (16, 47),
        (1151, 1151),
        (1152, 1152),
        (1153, 1199),
        (48_017, 48_017),
    ] {
        pcm_wav(&source, 48_000, 2, frames);
        encode_mp3(&source, &output).await.unwrap();
        let encoded = snapshot(&output);
        assert_eq!(
            (encoded.codec, encoded.rate, encoded.channels),
            (codec::Id::MP3, 48_000, 2)
        );
        let first_skip = &encoded.packets[0].skip[0];
        assert_eq!(
            u32::from_le_bytes(first_skip[..4].try_into().unwrap()),
            1105
        );
        assert_eq!(decoded_mono(&output, 48_000).await.len(), decoded_frames);
        assert!(!info_tag(&output).is_empty());
        assert_clean(directory.path());
    }
    pcm_wav(&source, 48_000, 2, 0);
    encode_mp3(&source, &output).await.unwrap();
    assert!(!info_tag(&output).is_empty());
    assert_clean(directory.path());
}

#[tokio::test]
async fn native_mp3_failures_keep_previous_output_without_a_cli() {
    let directory = tempfile::tempdir().unwrap();
    let invalid = directory.path().join("invalid.wav");
    fs::write(&invalid, b"not media").unwrap();
    let output = directory.path().join("kept.mp3");
    fs::write(&output, b"previous export").unwrap();
    let video = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/video/test.mp4");
    for source in [
        &invalid,
        &directory.path().join("missing.wav"),
        &video,
        &output,
    ] {
        assert!(encode_mp3(source, &output).await.is_err());
        assert_eq!(fs::read(&output).unwrap(), b"previous export");
        assert_clean(directory.path());
    }
}

#[tokio::test]
async fn cancelled_mp3_export_cleans_the_worker_output_without_a_cli() {
    assert_cancelled_media(
        |input, output| async move { encode_mp3(&input, &output).await },
        "mp3",
        ".audio-encode-",
    )
    .await;
}

async fn assert_cli(input: &Path, directory: &Path, case: &str) {
    let output = directory.join("native.mp3");
    let oracle = directory.join("oracle.mp3");
    encode_mp3(input, &output)
        .await
        .unwrap_or_else(|error| panic!("{case}: {error}"));
    cli(
        &[
            "-i",
            input.to_str().unwrap(),
            "-vn",
            "-acodec",
            "libmp3lame",
            "-q:a",
            "2",
        ],
        &oracle,
    );
    assert_eq!(
        info_tag(&output),
        info_tag(&oracle),
        "{case}: Xing/Info and encoder delay/padding"
    );
    let actual = snapshot(&output);
    let expected = snapshot(&oracle);
    assert_equivalent(&actual, &expected, case);
    assert_samples(
        &decoded_mono(&output, actual.rate).await,
        &decoded_mono(&oracle, expected.rate).await,
        case,
    );
    assert_clean(directory);
}

#[tokio::test]
async fn mp3_matches_cli_rate_channel_negotiation_and_short_tails() {
    if !has_cli() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("source.wav");
    for (rate, channels) in [
        (48_000, 1),
        (48_000, 2),
        (44_100, 2),
        (32_000, 2),
        (20_000, 1),
        (11_025, 1),
        (48_000, 6),
    ] {
        pcm_wav(&input, rate, channels, rate as usize / 3 + 17);
        assert_cli(&input, directory.path(), &format!("PCM {rate}/{channels}")).await;
    }
    for frames in [1, 16, 1151, 1152, 1153] {
        pcm_wav(&input, 48_000, 2, frames);
        assert_cli(&input, directory.path(), &format!("short {frames}")).await;
    }
}

#[tokio::test]
async fn mp3_matches_cli_compressed_sources_and_automatic_default_track() {
    if !has_cli() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("source.wav");
    pcm_wav(&input, 44_100, 2, 44_113);
    for (extension, codec) in [("m4a", "aac"), ("mp3", "libmp3lame"), ("flv", "aac")] {
        let source = directory.path().join(format!("source.{extension}"));
        cli(&["-i", input.to_str().unwrap(), "-c:a", codec], &source);
        assert_cli(&source, directory.path(), extension).await;
    }
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
            "title=MP3 export",
            "-metadata",
            "artist=Recorder",
            "-metadata:s:a:1",
            "language=jpn",
            "-metadata:s:a:1",
            "title=Selected mono",
        ],
        &tracks,
    );
    assert_cli(&tracks, directory.path(), "default mono track").await;
}

#[tokio::test]
async fn mp3_matches_cli_delayed_audio_timestamp_gaps_and_recorded_fmp4() {
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
    assert_cli(&delayed, directory.path(), "delayed AAC").await;
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
    assert_cli(&gap, directory.path(), "timestamp gap").await;
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/video");
    let mut data = fs::read(fixtures.join("init.m4s")).unwrap();
    data.extend(fs::read(fixtures.join("segment.m4s")).unwrap());
    let recorded = directory.path().join("recorded.mp4");
    fs::write(&recorded, data).unwrap();
    assert_cli(&recorded, directory.path(), "recorded fMP4").await;
}

#[tokio::test]
async fn mp3_matches_cli_planar_reblocking_across_aac_format_changes() {
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
        assert_cli(
            &changing,
            directory.path(),
            &format!("changing AAC {formats:?}"),
        )
        .await;
    }
}
