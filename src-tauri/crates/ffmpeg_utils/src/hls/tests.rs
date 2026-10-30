use super::*;

use std::{fs, path::PathBuf, process::Command, time::Instant};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/hls/fixtures/live.flv")
}

fn playlist(directory: &Path) -> (String, Vec<PathBuf>, Vec<f64>) {
    let text = fs::read_to_string(directory.join("playlist.m3u8")).unwrap();
    let segments = text
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| directory.join(line))
        .collect();
    let durations = text
        .lines()
        .filter_map(|line| line.strip_prefix("#EXTINF:"))
        .map(|line| line.split(',').next().unwrap().parse().unwrap())
        .collect();
    (text, segments, durations)
}

#[derive(Debug, PartialEq, Eq)]
struct MediaPacket {
    codec: codec::Id,
    pts: Option<i64>,
    dts: Option<i64>,
    duration: i64,
    bytes: Vec<u8>,
}

fn packets(path: &Path) -> Vec<MediaPacket> {
    let mut input = crate::metadata::open_input(path, Arc::new(AtomicBool::new(false))).unwrap();
    let mut packets = Vec::new();
    loop {
        let mut packet = Packet::empty();
        match packet.read(&mut input) {
            Ok(()) => {
                let stream = input.stream(packet.stream()).unwrap();
                assert!(matches!(
                    stream.parameters().medium(),
                    media::Type::Video | media::Type::Audio
                ));
                packet.rescale_ts(stream.time_base(), (1, 90_000));
                packets.push(MediaPacket {
                    codec: stream.parameters().id(),
                    pts: packet.pts(),
                    dts: packet.dts(),
                    duration: packet.duration(),
                    bytes: packet.data().unwrap().to_vec(),
                });
            }
            Err(ffmpeg::Error::Eof) => break,
            Err(error) => panic!("demux failed: {error}"),
        }
    }
    packets
}

async fn record(directory: &Path) {
    record_to_hls(
        fixture().to_str().unwrap(),
        &directory.join("playlist.m3u8"),
        &directory.join("%d.ts"),
        HlsRecordingOptions::default(),
        Arc::new(AtomicBool::new(true)),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn finite_live_input_records_video_audio_and_appends_without_a_cli() {
    let directory = tempfile::tempdir().unwrap();
    record(directory.path()).await;
    let (text, segments, durations) = playlist(directory.path());
    assert!(text.contains("#EXT-X-PLAYLIST-TYPE:EVENT"));
    assert!(!text.contains("#EXT-X-ENDLIST"));
    assert_eq!(segments.len(), durations.len());
    assert!(segments.len() >= 2);
    assert!((durations.iter().sum::<f64>() - 6.0).abs() < 0.2);
    let recorded: Vec<_> = segments.iter().flat_map(|path| packets(path)).collect();
    assert_eq!(
        recorded
            .iter()
            .filter(|p| p.codec == codec::Id::H264)
            .count(),
        150
    );
    assert!(
        recorded
            .iter()
            .filter(|p| p.codec == codec::Id::AAC)
            .count()
            >= 281
    );
    let original: Vec<_> = segments
        .iter()
        .map(|path| fs::read(path).unwrap())
        .collect();
    record(directory.path()).await;
    let (text, appended, _) = playlist(directory.path());
    assert!(text.contains("#EXT-X-DISCONTINUITY"));
    assert!(appended.len() > segments.len());
    for (path, bytes) in segments.iter().zip(original) {
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
}

#[tokio::test]
async fn native_hls_matches_cli_segments_packets_and_timing_when_available() {
    if !Command::new(crate::ffmpeg_path())
        .arg("-version")
        .output()
        .is_ok_and(|result| result.status.success())
    {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let native = directory.path().join("native");
    let oracle = directory.path().join("oracle");
    fs::create_dir(&native).unwrap();
    fs::create_dir(&oracle).unwrap();
    record(&native).await;
    let output = Command::new(crate::ffmpeg_path())
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i"])
        .arg(fixture())
        .args([
            "-c",
            "copy",
            "-sn",
            "-f",
            "hls",
            "-hls_time",
            "4",
            "-hls_list_size",
            "0",
            "-hls_playlist_type",
            "event",
            "-hls_flags",
            "append_list+omit_endlist",
            "-hls_segment_filename",
        ])
        .arg(oracle.join("%d.ts"))
        .arg(oracle.join("playlist.m3u8"))
        .arg("-y")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let (actual, segments, durations) = playlist(&native);
    let (expected, other_segments, other_durations) = playlist(&oracle);
    assert_eq!(actual, expected);
    assert_eq!(durations, other_durations);
    assert_eq!(segments.len(), other_segments.len());
    for (index, (a, b)) in segments.iter().zip(other_segments).enumerate() {
        assert_eq!(
            packets(a),
            packets(&b),
            "segment {index} packet payload/timing"
        );
    }
}

struct AbortOnDrop(tokio::task::AbortHandle);
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn stalled_source() -> (String, tokio::task::JoinHandle<std::io::Result<usize>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let data = fs::read(fixture()).unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut bytes = [0; 2048];
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let read = socket.read(&mut bytes).await?;
            assert!(read > 0 && request.len() < 16 * 1024);
            request.extend_from_slice(&bytes[..read]);
        }
        let headers = String::from_utf8(request).unwrap().to_ascii_lowercase();
        assert!(headers.contains("user-agent: native-record-test"));
        assert!(headers.contains("x-record-test: included"));
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: video/x-flv\r\nContent-Length: 1000000000\r\nConnection: close\r\n\r\n").await?;
        socket.write_all(&data).await?;
        socket.read(&mut bytes).await
    });
    (format!("http://{address}/live.flv"), server)
}

fn options(timeout: Option<Duration>) -> HlsRecordingOptions {
    HlsRecordingOptions {
        user_agent: Some("native-record-test".into()),
        http_headers: vec![("X-Record-Test".into(), "included".into())],
        read_timeout: timeout,
    }
}

async fn begun(directory: &Path, task: &tokio::task::JoinHandle<Result<(), String>>) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if fs::read_to_string(directory.join("playlist.m3u8"))
            .is_ok_and(|text| text.contains("#EXTINF:"))
        {
            break;
        }
        assert!(
            !task.is_finished(),
            "recording ended before its first segment"
        );
        assert!(
            Instant::now() < deadline,
            "recording did not publish a segment"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn closed(server: tokio::task::JoinHandle<std::io::Result<usize>>) {
    let result = tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(result, Ok(0))
            || matches!(result, Err(error) if matches!(error.kind(),
        std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted))
    );
}

#[tokio::test]
async fn disabling_live_recording_interrupts_input_and_finishes_the_tail_without_a_cli() {
    let directory = tempfile::tempdir().unwrap();
    let (source, server) = stalled_source().await;
    let _server_guard = AbortOnDrop(server.abort_handle());
    let enabled = Arc::new(AtomicBool::new(true));
    let switch = enabled.clone();
    let path = directory.path().to_owned();
    let task = tokio::spawn(async move {
        record_to_hls(
            &source,
            &path.join("playlist.m3u8"),
            &path.join("%d.ts"),
            options(None),
            switch,
        )
        .await
    });
    let _task_guard = AbortOnDrop(task.abort_handle());
    begun(directory.path(), &task).await;
    let before = playlist(directory.path()).1.len();
    enabled.store(false, Ordering::Release);
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    closed(server).await;
    let (_, after, _) = playlist(directory.path());
    assert!(
        after.len() > before,
        "graceful stop must finish the pending segment"
    );
    assert!(after
        .iter()
        .all(|path| fs::metadata(path).unwrap().len() > 0));
}

#[tokio::test]
async fn cancelled_recording_closes_network_io_and_stops_writing_without_a_cli() {
    let directory = tempfile::tempdir().unwrap();
    let (source, server) = stalled_source().await;
    let _server_guard = AbortOnDrop(server.abort_handle());
    let path = directory.path().to_owned();
    let task = tokio::spawn(async move {
        record_to_hls(
            &source,
            &path.join("playlist.m3u8"),
            &path.join("%d.ts"),
            options(None),
            Arc::new(AtomicBool::new(true)),
        )
        .await
    });
    let _task_guard = AbortOnDrop(task.abort_handle());
    begun(directory.path(), &task).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    closed(server).await;
    let snapshot = || {
        fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| {
                let path = entry.unwrap().path();
                (path.clone(), fs::read(path).unwrap())
            })
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    let before = snapshot();
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(snapshot(), before);
}

#[tokio::test]
async fn live_idle_timeout_preserves_the_complete_tail_before_error_and_append() {
    let directory = tempfile::tempdir().unwrap();
    record_to_hls(
        "http://127.0.0.1:1/not-opened",
        &directory.path().join("playlist.m3u8"),
        &directory.path().join("%d.ts"),
        HlsRecordingOptions::default(),
        Arc::new(AtomicBool::new(false)),
    )
    .await
    .unwrap();
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
    let reference = tempfile::tempdir().unwrap();
    record(reference.path()).await;
    let (source, server) = stalled_source().await;
    let _server_guard = AbortOnDrop(server.abort_handle());
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        record_to_hls(
            &source,
            &directory.path().join("playlist.m3u8"),
            &directory.path().join("%d.ts"),
            options(Some(Duration::from_millis(150))),
            Arc::new(AtomicBool::new(true)),
        ),
    )
    .await
    .unwrap();
    let error = result.unwrap_err();
    assert!(error.starts_with("读取录制数据失败:"), "{error}");
    closed(server).await;

    let (text, segments, durations) = playlist(directory.path());
    let (expected, reference_segments, reference_durations) = playlist(reference.path());
    assert_eq!(text, expected, "read errors must publish the pending tail");
    assert_eq!(durations, reference_durations);
    assert_eq!(segments.len(), reference_segments.len());
    for (actual, expected) in segments.iter().zip(&reference_segments) {
        assert_eq!(packets(actual), packets(expected));
    }
    let original: Vec<_> = segments
        .iter()
        .map(|path| fs::read(path).unwrap())
        .collect();
    record(directory.path()).await;
    let (_, appended, _) = playlist(directory.path());
    assert_eq!(appended.len(), segments.len() + reference_segments.len());
    assert_eq!(&appended[..segments.len()], segments.as_slice());
    assert!(appended[segments.len()..]
        .iter()
        .all(|path| !segments.contains(path)));
    for (path, bytes) in segments.iter().zip(original) {
        assert_eq!(fs::read(path).unwrap(), bytes, "append overwrote a segment");
    }
}
