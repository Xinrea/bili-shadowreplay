use super::*;

use std::{
    collections::BTreeMap,
    f64::consts::TAU,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub(super) fn pcm_wav(path: &Path, rate: u32, channels: u16, frames: usize) {
    let bytes = (frames * usize::from(channels) * 2) as u32;
    let mut data = Vec::with_capacity(44 + bytes as usize);
    data.extend_from_slice(b"RIFF");
    data.extend_from_slice(&(36 + bytes).to_le_bytes());
    data.extend_from_slice(b"WAVEfmt ");
    data.extend_from_slice(&16u32.to_le_bytes());
    data.extend_from_slice(&1u16.to_le_bytes());
    data.extend_from_slice(&channels.to_le_bytes());
    data.extend_from_slice(&rate.to_le_bytes());
    data.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
    data.extend_from_slice(&(channels * 2).to_le_bytes());
    data.extend_from_slice(&16u16.to_le_bytes());
    data.extend_from_slice(b"data");
    data.extend_from_slice(&bytes.to_le_bytes());
    for index in 0..frames {
        for channel in 0..channels {
            let time = index as f64 / f64::from(rate);
            let frequency = 300.0 + f64::from(channel) * 211.0;
            let sample = (12_000.0 * (TAU * (frequency * time + 800.0 * time * time)).sin()) as i16;
            data.extend_from_slice(&sample.to_le_bytes());
        }
    }
    fs::write(path, data).unwrap();
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Ogg {
    pub(super) packets: Vec<Vec<u8>>,
    pub(super) pages: Vec<(u8, u64)>,
    pub(super) vendor: String,
    pub(super) tags: BTreeMap<String, String>,
}

pub(super) fn ogg(path: &Path) -> Ogg {
    let data = fs::read(path).unwrap();
    let mut cursor = 0;
    let mut pages = Vec::new();
    let mut packets = Vec::new();
    let mut pending = Vec::new();
    while cursor < data.len() {
        assert_eq!(&data[cursor..cursor + 4], b"OggS");
        assert_eq!(data[cursor + 4], 0);
        let flags = data[cursor + 5];
        let granule = u64::from_le_bytes(data[cursor + 6..cursor + 14].try_into().unwrap());
        let segments = usize::from(data[cursor + 26]);
        let laces = &data[cursor + 27..cursor + 27 + segments];
        let mut body = cursor + 27 + segments;
        for size in laces {
            let end = body + usize::from(*size);
            pending.extend_from_slice(&data[body..end]);
            body = end;
            if *size < 255 {
                packets.push(std::mem::take(&mut pending));
            }
        }
        pages.push((flags, granule));
        cursor = body;
    }
    assert!(pending.is_empty());
    assert!(packets.len() >= 2);
    assert_eq!(&packets[0][..8], b"OpusHead");
    assert_eq!(&packets[1][..8], b"OpusTags");
    let comments = &packets[1];
    let mut cursor = 8;
    fn number(data: &[u8], cursor: &mut usize) -> usize {
        let value = u32::from_le_bytes(data[*cursor..*cursor + 4].try_into().unwrap()) as usize;
        *cursor += 4;
        value
    }
    fn string(data: &[u8], cursor: &mut usize) -> String {
        let size = number(data, cursor);
        let value = String::from_utf8(data[*cursor..*cursor + size].to_vec()).unwrap();
        *cursor += size;
        value
    }
    let vendor = string(comments, &mut cursor);
    let count = number(comments, &mut cursor);
    let mut tags = BTreeMap::new();
    for _ in 0..count {
        let comment = string(comments, &mut cursor);
        let (key, value) = comment.split_once('=').unwrap();
        assert!(tags
            .insert(key.to_ascii_lowercase(), value.into())
            .is_none());
    }
    Ogg {
        packets,
        pages,
        vendor,
        tags,
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct EncodedPacket {
    pub(super) data: Vec<u8>,
    pub(super) pts: Option<i64>,
    pub(super) dts: Option<i64>,
    pub(super) duration: i64,
    pub(super) skip: Vec<Vec<u8>>,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Timeline {
    pub(super) start: i64,
    pub(super) duration: i64,
    pub(super) time_base: (i32, i32),
    pub(super) packets: Vec<EncodedPacket>,
}

fn timeline(path: &Path) -> Timeline {
    timeline_channels(path, 1)
}

pub(super) fn timeline_channels(path: &Path, channels: u16) -> Timeline {
    let mut input = open_input(path, Arc::new(AtomicBool::new(false))).unwrap();
    let stream = input.streams().best(media::Type::Audio).unwrap();
    assert_eq!(stream.parameters().id(), codec::Id::OPUS);
    let decoder = codec::context::Context::from_parameters(stream.parameters())
        .unwrap()
        .decoder()
        .audio()
        .unwrap();
    assert_eq!(decoder.rate(), 48_000);
    assert_eq!(decoder.channels(), channels);
    let index = stream.index();
    let mut result = Timeline {
        start: stream.start_time(),
        duration: stream.duration(),
        time_base: (
            stream.time_base().numerator(),
            stream.time_base().denominator(),
        ),
        packets: Vec::new(),
    };
    loop {
        let mut packet = Packet::empty();
        match packet.read(&mut input) {
            Ok(()) => {
                assert_eq!(packet.stream(), index);
                result.packets.push(EncodedPacket {
                    data: packet.data().unwrap().to_vec(),
                    pts: packet.pts(),
                    dts: packet.dts(),
                    duration: packet.duration(),
                    skip: packet
                        .side_data()
                        .filter(|data| data.kind() == codec::packet::side_data::Type::SkipSamples)
                        .map(|data| data.data().to_vec())
                        .collect(),
                });
            }
            Err(ffmpeg::Error::Eof) => break,
            Err(error) => panic!("Opus demux failed: {error}"),
        }
    }
    result
}

#[derive(Default)]
struct Count(u64);
impl AudioSink for Count {
    fn write(&mut self, samples: &[i16]) -> Result<(), String> {
        self.0 += samples.len() as u64;
        Ok(())
    }
}

pub(super) fn assert_clean(directory: &Path) {
    assert!(fs::read_dir(directory).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".opus-")));
}

#[tokio::test]
async fn native_opus_short_tails_and_empty_headers_work_without_a_cli() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("音频 样本.wav");
    let output = directory.path().join("音频 样本.opus");
    for frames in [0, 1, 16, 103, 216, 217, 319, 320, 321, 641, 16_001] {
        pcm_wav(&source, 16_000, 1, frames);
        encode_opus_sample(&source, &output).await.unwrap();
        let stream = ogg(&output);
        let head = &stream.packets[0];
        assert_eq!(head.len(), 19);
        assert_eq!(head[8], 1);
        assert_eq!(head[9], 1);
        assert_eq!(u16::from_le_bytes(head[10..12].try_into().unwrap()), 312);
        assert_eq!(u32::from_le_bytes(head[12..16].try_into().unwrap()), 16_000);
        assert_eq!(head[18], 0);
        assert!(stream.tags["encoder"].ends_with(" libopus"));
        if frames == 0 {
            assert_eq!(stream.packets.len(), 2);
            assert_eq!(stream.pages, [(2, 0), (0, 0)]);
        } else {
            assert_eq!(stream.pages.last().unwrap(), &(4, frames as u64 * 3 + 312));
            let encoded = timeline(&output);
            assert_eq!(encoded.time_base, (1, 48_000));
            assert_eq!(encoded.start, 0);
            assert_eq!(encoded.duration, frames as i64 * 3 + 312);
            assert_eq!(encoded.packets[0].pts, Some(-312));
            let decoded = decode_audio(
                &output,
                AudioDecodeOptions {
                    sample_rate: 48_000,
                },
                Count::default(),
            )
            .await
            .unwrap();
            assert_eq!(decoded.samples, frames as u64 * 3, "frames={frames}");
            assert_eq!(decoded.sink.0, decoded.samples);
        }
        assert_clean(directory.path());
    }
}

#[tokio::test]
async fn recorded_fmp4_encodes_real_aac_without_a_cli() {
    let directory = tempfile::tempdir().unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/video");
    let source = directory.path().join("recorded.mp4");
    let mut data = fs::read(fixtures.join("init.m4s")).unwrap();
    data.extend(fs::read(fixtures.join("segment.m4s")).unwrap());
    fs::write(&source, data).unwrap();
    let output = directory.path().join("recorded.opus");
    encode_opus_sample(&source, &output).await.unwrap();
    let encoded = timeline(&output);
    // Independently measured with the former CLI profile. The live fragment's
    // first audio PTS precedes the container origin by 33 samples at 48 kHz.
    assert_eq!(encoded.packets.len(), 51);
    assert_eq!(encoded.start, -33);
    assert_eq!(encoded.duration, 48_408);
    assert_eq!(encoded.packets[0].pts, Some(-345));
    assert_eq!(encoded.packets.last().unwrap().duration, 441);
    assert!(encoded.packets.iter().any(|packet| packet.data.len() > 10));
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
    assert_eq!(decoded.samples, 48_129);
    assert_clean(directory.path());
}

#[tokio::test]
async fn native_opus_rejects_invalid_video_and_same_file_without_touching_output() {
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("kept.opus");
    fs::write(&output, b"previous output").unwrap();
    let video = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/video/test.mp4");
    let invalid = directory.path().join("invalid.wav");
    fs::write(&invalid, b"not media").unwrap();
    for source in [
        &invalid,
        &directory.path().join("missing.wav"),
        &video,
        &output,
    ] {
        assert!(encode_opus_sample(source, &output).await.is_err());
        assert_eq!(fs::read(&output).unwrap(), b"previous output");
        assert_clean(directory.path());
    }
    #[cfg(unix)]
    {
        let alias = directory.path().join("alias.opus");
        std::os::unix::fs::symlink(&output, &alias).unwrap();
        assert!(encode_opus_sample(&alias, &output).await.is_err());
        assert_eq!(fs::read(&output).unwrap(), b"previous output");
        assert_clean(directory.path());
    }
}

#[tokio::test]
async fn cancelling_native_opus_closes_http_and_removes_partial_output_without_a_cli() {
    assert_cancelled_opus(|input, output| async move { encode_opus_sample(&input, &output).await })
        .await;
}

pub(super) async fn assert_cancelled_opus<F, Fut>(encode: F)
where
    F: FnOnce(PathBuf, PathBuf) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<(), String>> + Send + 'static,
{
    assert_cancelled_media(encode, "opus", ".opus-").await;
}

pub(crate) async fn assert_cancelled_media<F, Fut>(encode: F, extension: &str, prefix: &str)
where
    F: FnOnce(PathBuf, PathBuf) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<(), String>> + Send + 'static,
{
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    struct AbortOnDrop(tokio::task::AbortHandle);
    impl Drop for AbortOnDrop {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.wav");
    pcm_wav(&source, 16_000, 1, 160_000);
    let mut data = fs::read(&source).unwrap();
    data[4..8].copy_from_slice(&999_999_992u32.to_le_bytes());
    data[40..44].copy_from_slice(&999_999_956u32.to_le_bytes());
    let output = directory.path().join(format!("kept.{extension}"));
    fs::write(&output, b"previous output").unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let mut server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut bytes = [0; 1024];
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let read = socket.read(&mut bytes).await.unwrap();
            assert!(read > 0 && request.len() < 16 * 1024);
            request.extend_from_slice(&bytes[..read]);
        }
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: audio/wav\r\nContent-Length: 1000000000\r\nConnection: close\r\n\r\n").await.unwrap();
        socket.write_all(&data).await.unwrap();
        let _ = ready_tx.send(());
        socket.read(&mut bytes).await
    });
    let _server_guard = AbortOnDrop(server.abort_handle());
    let url = PathBuf::from(format!("http://{address}/stalled.wav"));
    let target = output.clone();
    let task = tokio::spawn(async move { encode(url, target).await });
    let _task_guard = AbortOnDrop(task.abort_handle());
    tokio::time::timeout(Duration::from_secs(5), ready_rx)
        .await
        .unwrap()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if fs::read_dir(directory.path()).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(prefix)
        }) {
            break;
        }
        assert!(!task.is_finished());
        assert!(
            Instant::now() < deadline,
            "encoder did not begin its temporary output"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let closed = tokio::time::timeout(Duration::from_secs(5), &mut server)
        .await
        .unwrap()
        .unwrap();
    // Closing a socket with unread buffered input may send RST instead of FIN.
    assert!(
        matches!(closed, Ok(0))
            || matches!(closed, Err(error) if matches!(error.kind(), std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted))
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let partial = fs::read_dir(directory.path()).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(prefix)
        });
        if !partial {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "cancelled worker retained its temporary Opus"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(fs::read(&output).unwrap(), b"previous output");
    assert!(fs::read_dir(directory.path()).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(prefix)));
}

pub(super) fn has_cli() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

pub(super) fn cli(args: &[&str], output: &Path) {
    let result = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-y"])
        .args(args)
        .arg(output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

async fn assert_cli(source: &Path, directory: &Path, case: &str) {
    let native = directory.join("native.opus");
    let oracle = directory.join("oracle.opus");
    encode_opus_sample(source, &native)
        .await
        .unwrap_or_else(|error| panic!("{case}: {error}"));
    cli(
        &[
            "-i",
            source.to_str().unwrap(),
            "-c:a",
            "libopus",
            "-ar",
            "16000",
            "-ac",
            "1",
            "-vn",
            "-b:a",
            "64k",
            "-vbr",
            "on",
            "-compression_level",
            "10",
        ],
        &oracle,
    );
    assert_equivalent(&native, &oracle, case, 1);
    assert_clean(directory);
}

pub(super) fn assert_equivalent(native: &Path, oracle: &Path, case: &str, channels: u16) {
    let native_ogg = ogg(native);
    let oracle_ogg = ogg(oracle);
    assert_eq!(
        native_ogg.packets[0], oracle_ogg.packets[0],
        "{case}: OpusHead"
    );
    assert_eq!(native_ogg.vendor, oracle_ogg.vendor, "{case}: vendor");
    assert_eq!(native_ogg.tags, oracle_ogg.tags, "{case}: tags");
    assert_eq!(
        native_ogg.pages, oracle_ogg.pages,
        "{case}: Ogg page granules"
    );
    assert_eq!(
        native_ogg.packets.len(),
        oracle_ogg.packets.len(),
        "{case}: packet count"
    );
    for (index, (actual, expected)) in native_ogg
        .packets
        .iter()
        .zip(&oracle_ogg.packets)
        .enumerate()
        .skip(2)
    {
        assert!(
            actual == expected,
            "{case}: coded packet {index} differs ({} vs {} bytes)",
            actual.len(),
            expected.len()
        );
    }
    if native_ogg.packets.len() > 2 {
        let actual = timeline_channels(native, channels);
        let expected = timeline_channels(oracle, channels);
        assert_eq!(
            (actual.start, actual.duration, actual.time_base),
            (expected.start, expected.duration, expected.time_base),
            "{case}: stream timeline"
        );
        assert_eq!(
            actual.packets.len(),
            expected.packets.len(),
            "{case}: demux packet count"
        );
        for (index, (actual, expected)) in actual.packets.iter().zip(&expected.packets).enumerate()
        {
            assert!(
                actual == expected,
                "{case}: packet {index} payload/timestamp/duration/skip differs: {:?} vs {:?}",
                (actual.pts, actual.dts, actual.duration, &actual.skip),
                (
                    expected.pts,
                    expected.dts,
                    expected.duration,
                    &expected.skip
                )
            );
        }
    }
}

#[tokio::test]
async fn opus_matches_cli_pcm_resampling_stereo_float_and_short_tails() {
    if !has_cli() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.wav");
    for (rate, channels, frames) in [
        (16_000, 1, 0),
        (16_000, 1, 1),
        (16_000, 1, 217),
        (16_000, 1, 16_001),
        (44_100, 2, 44_113),
        (48_000, 2, 48_017),
    ] {
        pcm_wav(&source, rate, channels, frames);
        assert_cli(
            &source,
            directory.path(),
            &format!("PCM {rate}/{channels}/{frames}"),
        )
        .await;
    }
    let float = directory.path().join("float.wav");
    cli(
        &["-i", source.to_str().unwrap(), "-c:a", "pcm_f32le"],
        &float,
    );
    assert_cli(&float, directory.path(), "float PCM").await;
}

#[tokio::test]
async fn opus_matches_cli_aac_mp3_flv_and_recorded_fmp4() {
    if !has_cli() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.wav");
    for rate in [44_100, 48_000] {
        pcm_wav(&source, rate, 2, rate as usize + 17);
        for (extension, codec) in [
            ("m4a", "aac"),
            ("mp3", "libmp3lame"),
            ("flv", "aac"),
            ("aac", "aac"),
        ] {
            let encoded = directory.path().join(format!("{rate}.{extension}"));
            cli(&["-i", source.to_str().unwrap(), "-c:a", codec], &encoded);
            assert_cli(&encoded, directory.path(), &format!("{rate}.{extension}")).await;
        }
    }
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/video");
    let mut data = fs::read(fixtures.join("init.m4s")).unwrap();
    data.extend(fs::read(fixtures.join("segment.m4s")).unwrap());
    let recorded = directory.path().join("recorded.mp4");
    fs::write(&recorded, data).unwrap();
    assert_cli(&recorded, directory.path(), "recorded fMP4").await;
}

#[tokio::test]
async fn opus_matches_cli_delayed_audio_origin_and_timestamp_gaps() {
    if !has_cli() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.wav");
    pcm_wav(&source, 48_000, 1, 144_000);
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
            source.to_str().unwrap(),
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
    assert_cli(&delayed, directory.path(), "delayed audio").await;
    let gap = directory.path().join("gap.m4a");
    cli(
        &[
            "-i",
            source.to_str().unwrap(),
            "-af",
            "asetpts=PTS+gte(T\\,1)/TB",
            "-c:a",
            "aac",
        ],
        &gap,
    );
    assert_cli(&gap, directory.path(), "timestamp gap").await;
}

#[tokio::test]
async fn opus_matches_cli_metadata_chapters_and_automatic_multitrack_selection() {
    if !has_cli() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("first.wav");
    let second = directory.path().join("second.wav");
    pcm_wav(&first, 44_100, 2, 44_113);
    pcm_wav(&second, 48_000, 1, 48_017);
    let metadata = directory.path().join("chapters.txt");
    fs::write(&metadata, ";FFMETADATA1\ntitle=Global title\nartist=Global artist\ncomment=Global comment\nGLOBAL_CUSTOM=Global custom\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=0\nEND=500\ntitle=First chapter\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=500\nEND=1000\ntitle=Second chapter\n").unwrap();
    let source = directory.path().join("metadata.mka");
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
            "-metadata:s:a:1",
            "title=Selected title",
            "-metadata:s:a:1",
            "artist=Selected artist",
            "-metadata:s:a:1",
            "language=jpn",
            "-metadata:s:a:1",
            "STREAM_CUSTOM=Stream custom",
        ],
        &source,
    );
    assert_cli(
        &source,
        directory.path(),
        "metadata, chapters, default mono track",
    )
    .await;
}

#[tokio::test]
async fn opus_matches_cli_when_adts_changes_sample_rate_with_a_partial_encoder_frame() {
    if !has_cli() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.wav");
    pcm_wav(&source, 48_000, 1, 48_000);
    let mut joined = Vec::new();
    for rate in [48_000, 44_100] {
        let part = directory.path().join(format!("part{rate}.aac"));
        cli(
            &[
                "-i",
                source.to_str().unwrap(),
                "-t",
                "0.5",
                "-ar",
                &rate.to_string(),
                "-c:a",
                "aac",
                "-f",
                "adts",
            ],
            &part,
        );
        joined.extend(fs::read(part).unwrap());
    }
    let changing = directory.path().join("changing-rate.aac");
    fs::write(&changing, joined).unwrap();
    assert_cli(&changing, directory.path(), "ADTS 48000 to 44100").await;
}
