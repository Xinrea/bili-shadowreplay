use super::{
    file_test_helpers::*,
    opus_tests::{assert_cancelled_media, cli, has_cli, pcm_wav},
    *,
};

use std::{fs, path::Path};

fn assert_faststart(path: &Path) {
    let data = fs::read(path).unwrap();
    let mut cursor = 0;
    let mut moov = None;
    let mut mdat = None;
    while cursor + 8 <= data.len() {
        let size = u32::from_be_bytes(data[cursor..cursor + 4].try_into().unwrap()) as usize;
        let (size, header) = if size == 1 {
            (
                u64::from_be_bytes(data[cursor + 8..cursor + 16].try_into().unwrap()) as usize,
                16,
            )
        } else if size == 0 {
            (data.len() - cursor, 8)
        } else {
            (size, 8)
        };
        assert!(size >= header && cursor + size <= data.len());
        match &data[cursor + 4..cursor + 8] {
            b"moov" => moov = Some(cursor),
            b"mdat" => mdat = Some(cursor),
            _ => (),
        }
        cursor += size;
    }
    assert!(
        moov.unwrap() < mdat.unwrap(),
        "M4A index must be available before its audio data"
    );
}

fn assert_clean(directory: &Path) {
    assert!(fs::read_dir(directory).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with('.')));
}

#[tokio::test]
async fn native_import_keeps_pcm_and_converts_float_without_a_cli() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("音乐 样本.wav");
    let output = directory.path().join("kept.wav");
    pcm_wav(&source, 44_100, 2, 44_113);
    import_audio(&source, &output, ImportAudioMode::CopyWav)
        .await
        .unwrap();
    let kept = snapshot(&output);
    assert_eq!(
        (kept.codec, kept.rate, kept.channels),
        (codec::Id::PCM_S16LE, 44_100, 2)
    );
    let data = fs::read(&source).unwrap();
    assert_eq!(
        kept.packets
            .iter()
            .flat_map(|packet| packet.data.iter().copied())
            .collect::<Vec<_>>(),
        data[44..]
    );

    let floats = [-1.0f32, -0.5, 0.0, 0.5, 1.0];
    let expected = [-32768i16, -16384, 0, 16384, 32767];
    let samples = 10_000u32;
    let bytes = samples * 4;
    let mut data = b"RIFF".to_vec();
    data.extend_from_slice(&(36 + bytes).to_le_bytes());
    data.extend_from_slice(b"WAVEfmt ");
    data.extend_from_slice(&16u32.to_le_bytes());
    data.extend_from_slice(&3u16.to_le_bytes());
    data.extend_from_slice(&2u16.to_le_bytes());
    data.extend_from_slice(&44_100u32.to_le_bytes());
    data.extend_from_slice(&(44_100u32 * 8).to_le_bytes());
    data.extend_from_slice(&8u16.to_le_bytes());
    data.extend_from_slice(&32u16.to_le_bytes());
    data.extend_from_slice(b"data");
    data.extend_from_slice(&bytes.to_le_bytes());
    for index in 0..samples {
        data.extend_from_slice(&floats[index as usize % floats.len()].to_le_bytes());
    }
    fs::write(&source, data).unwrap();
    import_audio(&source, &output, ImportAudioMode::Pcm16Wav)
        .await
        .unwrap();
    let converted = snapshot(&output);
    assert_eq!(
        (converted.codec, converted.rate, converted.channels),
        (codec::Id::PCM_S16LE, 44_100, 2)
    );
    let bytes: Vec<_> = converted
        .packets
        .iter()
        .flat_map(|packet| packet.data.iter().copied())
        .collect();
    let samples: Vec<_> = bytes
        .chunks_exact(2)
        .map(|pair| i16::from_le_bytes(pair.try_into().unwrap()))
        .collect();
    assert!(samples
        .iter()
        .enumerate()
        .all(|(index, sample)| *sample == expected[index % expected.len()]));
    let aac = directory.path().join("encoded.m4a");
    import_audio(&source, &aac, ImportAudioMode::Aac256M4a)
        .await
        .unwrap();
    let encoded = snapshot(&aac);
    assert_eq!(
        (encoded.codec, encoded.rate, encoded.channels),
        (codec::Id::AAC, 44_100, 2)
    );
    assert!(!encoded.packets.is_empty());
    assert!(encoded.duration > 0);
    assert_faststart(&aac);
    assert_clean(directory.path());
}

#[tokio::test]
async fn native_aac_import_is_faststart_and_preserves_recorded_payloads_without_a_cli() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("recorded.mp4");
    let output = directory.path().join("recorded.m4a");
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/video");
    let mut data = fs::read(fixtures.join("init.m4s")).unwrap();
    data.extend(fs::read(fixtures.join("segment.m4s")).unwrap());
    fs::write(&input, data).unwrap();
    import_audio(&input, &output, ImportAudioMode::CopyAacM4a)
        .await
        .unwrap();
    let copied = snapshot(&output);
    assert_eq!(
        (copied.codec, copied.rate, copied.channels),
        (codec::Id::AAC, 48_000, 2)
    );
    assert!(!copied.packets.is_empty());
    let mut source = open_input(&input, Arc::new(AtomicBool::new(false))).unwrap();
    let audio = source
        .streams()
        .find(|stream| stream.parameters().medium() == media::Type::Audio)
        .unwrap()
        .index();
    let mut payloads = Vec::new();
    loop {
        let mut packet = Packet::empty();
        match packet.read(&mut source) {
            Ok(()) if packet.stream() == audio => payloads.push(packet.data().unwrap().to_vec()),
            Ok(()) => (),
            Err(ffmpeg::Error::Eof) => break,
            Err(error) => panic!("recorded AAC demux failed: {error}"),
        }
    }
    assert_eq!(copied.packets.len(), payloads.len());
    assert!(copied
        .packets
        .iter()
        .zip(&payloads)
        .all(|(packet, input)| packet.data == *input));
    assert_faststart(&output);
    assert_clean(directory.path());
}

#[tokio::test]
async fn failed_import_keeps_previous_output_without_a_cli() {
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("kept.m4a");
    fs::write(&output, b"previous sound").unwrap();
    let invalid = directory.path().join("invalid.wav");
    fs::write(&invalid, b"not media").unwrap();
    let video = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/video/test.mp4");
    for mode in [
        ImportAudioMode::CopyWav,
        ImportAudioMode::CopyFlac,
        ImportAudioMode::CopyAacM4a,
        ImportAudioMode::Pcm16Wav,
        ImportAudioMode::Aac256M4a,
    ] {
        for input in [
            &invalid,
            &directory.path().join("missing.wav"),
            &video,
            &output,
        ] {
            assert!(import_audio(input, &output, mode).await.is_err());
            assert_eq!(fs::read(&output).unwrap(), b"previous sound");
            assert_clean(directory.path());
        }
    }
}

#[tokio::test]
async fn cancelled_copy_and_aac_imports_clean_the_worker_output_without_a_cli() {
    assert_cancelled_media(|input, output| async move { import_audio(&input, &output, ImportAudioMode::CopyWav).await }, "wav", ".audio-remux-").await;
    assert_cancelled_media(|input, output| async move { import_audio(&input, &output, ImportAudioMode::Aac256M4a).await }, "m4a", ".audio-encode-").await;
}

async fn assert_cli(input: &Path, directory: &Path, mode: ImportAudioMode, case: &str) {
    let output = directory.join(format!("native.{}", mode.extension()));
    let oracle = directory.join(format!("oracle.{}", mode.extension()));
    import_audio(input, &output, mode)
        .await
        .unwrap_or_else(|error| panic!("{case}: {error}"));
    let mut args = vec![
        "-i",
        input.to_str().unwrap(),
        "-map",
        "0:a:0",
        "-vn",
        "-sn",
        "-dn",
        "-c:a",
    ];
    match mode {
        ImportAudioMode::CopyWav | ImportAudioMode::CopyFlac | ImportAudioMode::CopyAacM4a => {
            args.push("copy")
        }
        ImportAudioMode::Pcm16Wav => args.push("pcm_s16le"),
        ImportAudioMode::Aac256M4a => args.extend(["aac", "-b:a", "256k"]),
    }
    if mode.extension() == "m4a" {
        args.extend(["-movflags", "+faststart"]);
    }
    cli(&args, &oracle);
    let actual = snapshot(&output);
    let expected = snapshot(&oracle);
    assert_equivalent(&actual, &expected, case);
    assert_samples(
        &decoded_mono(&output, actual.rate).await,
        &decoded_mono(&oracle, expected.rate).await,
        case,
    );
    if mode.extension() == "m4a" {
        assert_faststart(&output);
    }
    assert_clean(directory);
}

#[tokio::test]
async fn import_matches_cli_copy_pcm_and_aac_encoding_modes() {
    if !has_cli() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.wav");
    pcm_wav(&source, 48_000, 2, 48_017);
    assert_cli(
        &source,
        directory.path(),
        ImportAudioMode::CopyWav,
        "PCM copy",
    )
    .await;
    for (extension, codec, mode) in [
        ("flac", "flac", ImportAudioMode::CopyFlac),
        ("m4a", "aac", ImportAudioMode::CopyAacM4a),
        ("aac", "aac", ImportAudioMode::CopyAacM4a),
        ("wav", "pcm_s24le", ImportAudioMode::Pcm16Wav),
        ("mp3", "libmp3lame", ImportAudioMode::Aac256M4a),
    ] {
        let input = directory.path().join(format!("encoded.{extension}"));
        cli(&["-i", source.to_str().unwrap(), "-c:a", codec], &input);
        assert_cli(
            &input,
            directory.path(),
            mode,
            &format!("{extension} {codec}"),
        )
        .await;
    }
    pcm_wav(&source, 44_100, 1, 44_113);
    assert_cli(
        &source,
        directory.path(),
        ImportAudioMode::Aac256M4a,
        "44.1k mono AAC256",
    )
    .await;
}

#[tokio::test]
async fn import_matches_cli_first_track_tags_faststart_and_delayed_timestamps() {
    if !has_cli() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("first.wav");
    let second = directory.path().join("second.wav");
    pcm_wav(&first, 48_000, 1, 96_017);
    pcm_wav(&second, 44_100, 2, 88_217);
    let chapters = directory.path().join("chapters.txt");
    fs::write(&chapters, ";FFMETADATA1\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=0\nEND=1000\ntitle=First chapter\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=1000\nEND=2000\ntitle=Second chapter\n").unwrap();
    let tracks = directory.path().join("tracks.mka");
    cli(
        &[
            "-i",
            first.to_str().unwrap(),
            "-i",
            second.to_str().unwrap(),
            "-i",
            chapters.to_str().unwrap(),
            "-map_chapters",
            "2",
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
            "title=Library import",
            "-metadata",
            "artist=Recorder",
            "-metadata:s:a:0",
            "language=jpn",
            "-metadata:s:a:0",
            "title=First mono",
            "-metadata:s:a:1",
            "title=Default stereo",
        ],
        &tracks,
    );
    assert_cli(
        &tracks,
        directory.path(),
        ImportAudioMode::Aac256M4a,
        "explicit first mono track",
    )
    .await;
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
        ImportAudioMode::CopyAacM4a,
        "delayed AAC copy",
    )
    .await;
    assert_cli(
        &delayed,
        directory.path(),
        ImportAudioMode::Aac256M4a,
        "delayed AAC encode",
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
        ImportAudioMode::CopyAacM4a,
        "nonzero origin copy",
    )
    .await;
    let flv = directory.path().join("source.flv");
    cli(&["-i", second.to_str().unwrap(), "-c:a", "aac"], &flv);
    assert_cli(
        &flv,
        directory.path(),
        ImportAudioMode::CopyAacM4a,
        "44.1k FLV timestamp rounding",
    )
    .await;
}
