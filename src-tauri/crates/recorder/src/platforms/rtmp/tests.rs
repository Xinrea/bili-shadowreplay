use super::*;
use crate::events::RecorderEvent;
use rml_rtmp::{
    handshake::{Handshake, HandshakeProcessResult, PeerType},
    sessions::{
        ClientSession, ClientSessionConfig, ClientSessionEvent, ClientSessionResult,
        PublishRequestType,
    },
    time::RtmpTimestamp,
};
use std::{collections::VecDeque, net::SocketAddr, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::broadcast,
    time::timeout,
};

const DEADLINE: Duration = Duration::from_secs(10);

async fn recorder(
    directory: &std::path::Path,
) -> (RtmpRecorder, SocketAddr, broadcast::Receiver<RecorderEvent>) {
    recorder_with_storage(directory.to_owned()).await
}

async fn recorder_with_storage(
    storage: impl Into<recording_storage::RecordingStore>,
) -> (RtmpRecorder, SocketAddr, broadcast::Receiver<RecorderEvent>) {
    let (tx, rx) = broadcast::channel(64);
    let recorder = RtmpRecorder::new(
        "obs",
        RtmpConfig {
            listen_addr: "127.0.0.1".parse().unwrap(),
            ..RtmpConfig::default()
        },
        storage,
        tx,
        true,
    )
    .await
    .unwrap();
    let address = recorder
        .extra
        .listener
        .lock()
        .await
        .as_ref()
        .unwrap()
        .local_addr()
        .unwrap();
    recorder.run().await;
    (recorder, address, rx)
}

#[tokio::test]
async fn push_media_and_cover_use_injected_backend_without_a_workspace() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = directory.path().join("must-not-be-created");
    let store = recording_storage::RecordingStore::with_backend(
        Arc::new(recording_storage::LocalBackend::new(
            directory.path().join("objects"),
        )),
        &workspace,
    );
    let (recorder, address, mut events) = recorder_with_storage(store.clone()).await;
    let mut publisher = Publisher::connect(address, "live", &recorder.extra.config.stream_key)
        .await
        .unwrap();
    publisher.fixture().await;
    drop(publisher);
    let (live_id, duration, size) = ended(&mut events).await;
    let storage = store
        .recording("rtmp", &recorder.room_id, &live_id)
        .unwrap();
    let bytes = storage.read("playlist.m3u8").await.unwrap();
    let (_, playlist) = m3u8_rs::parse_media_playlist(&bytes).unwrap();
    assert!(playlist.end_list);
    assert!(playlist.segments.len() >= 2);
    assert!((duration - 6.0).abs() < 0.2, "{duration}");
    let mut actual_size = 0;
    for segment in playlist.segments {
        actual_size += storage.size(&segment.uri).await.unwrap();
    }
    assert_eq!(size, actual_size);
    assert!(storage
        .read("cover.jpg")
        .await
        .unwrap()
        .starts_with(&[0xff, 0xd8]));
    assert!(!workspace.exists());
    timeout(DEADLINE, recorder.stop()).await.unwrap();
}

struct Publisher {
    socket: TcpStream,
    session: ClientSession,
}

impl Publisher {
    async fn connect(address: SocketAddr, app: &str, key: &str) -> Result<Self, String> {
        timeout(DEADLINE, Self::connect_inner(address, app, Some(key)))
            .await
            .map_err(|e| e.to_string())?
    }

    async fn connect_without_publishing(address: SocketAddr) -> Result<Self, String> {
        timeout(DEADLINE, Self::connect_inner(address, "live", None))
            .await
            .map_err(|e| e.to_string())?
    }

    async fn connect_inner(
        address: SocketAddr,
        app: &str,
        key: Option<&str>,
    ) -> Result<Self, String> {
        let mut socket = TcpStream::connect(address)
            .await
            .map_err(|e| e.to_string())?;
        let mut handshake = Handshake::new(PeerType::Client);
        socket
            .write_all(&handshake.generate_outbound_p0_and_p1().unwrap())
            .await
            .unwrap();
        let mut buffer = [0; 4096];
        let remaining = loop {
            let length = socket.read(&mut buffer).await.map_err(|e| e.to_string())?;
            if length == 0 {
                return Err("disconnected".into());
            }
            match handshake.process_bytes(&buffer[..length]).unwrap() {
                HandshakeProcessResult::InProgress { response_bytes } => {
                    socket.write_all(&response_bytes).await.unwrap()
                }
                HandshakeProcessResult::Completed {
                    response_bytes,
                    remaining_bytes,
                } => {
                    socket.write_all(&response_bytes).await.unwrap();
                    break remaining_bytes;
                }
            }
        };
        let (mut session, initial) = ClientSession::new(ClientSessionConfig::new()).unwrap();
        let mut pending = VecDeque::from(initial);
        pending.push_back(session.request_connection(app.into()).unwrap());
        pending.extend(
            session
                .handle_input(&remaining)
                .map_err(|e| e.to_string())?,
        );
        loop {
            while let Some(result) = pending.pop_front() {
                match result {
                    ClientSessionResult::OutboundResponse(packet) => socket
                        .write_all(&packet.bytes)
                        .await
                        .map_err(|e| e.to_string())?,
                    ClientSessionResult::RaisedEvent(
                        ClientSessionEvent::ConnectionRequestAccepted,
                    ) => {
                        let Some(key) = key else {
                            return Ok(Self { socket, session });
                        };
                        pending.push_back(
                            session
                                .request_publishing(key.into(), PublishRequestType::Live)
                                .unwrap(),
                        );
                    }
                    ClientSessionResult::RaisedEvent(
                        ClientSessionEvent::PublishRequestAccepted,
                    ) => return Ok(Self { socket, session }),
                    ClientSessionResult::RaisedEvent(
                        ClientSessionEvent::ConnectionRequestRejected { description },
                    ) => return Err(description),
                    ClientSessionResult::RaisedEvent(
                        ClientSessionEvent::UnhandleableAmf0Command { command_name, .. },
                    ) if command_name == "_error" => return Err("rejected".into()),
                    _ => {}
                }
            }
            let length = socket.read(&mut buffer).await.map_err(|e| e.to_string())?;
            if length == 0 {
                return Err("disconnected".into());
            }
            pending.extend(
                session
                    .handle_input(&buffer[..length])
                    .map_err(|e| e.to_string())?,
            );
        }
    }

    async fn ping(&mut self) {
        let (packet, timestamp) = self.session.send_ping_request().unwrap();
        self.socket.write_all(&packet.bytes).await.unwrap();
        timeout(DEADLINE, async {
            let mut buffer = [0; 4096];
            loop {
                let length = self.socket.read(&mut buffer).await.unwrap();
                assert!(
                    length > 0,
                    "RTMP connection closed before the ping response"
                );
                for result in self.session.handle_input(&buffer[..length]).unwrap() {
                    match result {
                        ClientSessionResult::RaisedEvent(
                            ClientSessionEvent::PingResponseReceived {
                                timestamp: received,
                            },
                        ) => {
                            assert_eq!(received, timestamp);
                            return;
                        }
                        ClientSessionResult::OutboundResponse(packet) => {
                            self.socket.write_all(&packet.bytes).await.unwrap();
                        }
                        _ => {}
                    }
                }
            }
        })
        .await
        .unwrap();
    }

    async fn fixture(&mut self) {
        let flv = include_bytes!("../../../../ffmpeg_utils/src/hls/fixtures/live.flv");
        let mut offset = 13;
        while offset + 11 <= flv.len() {
            let size =
                u32::from_be_bytes([0, flv[offset + 1], flv[offset + 2], flv[offset + 3]]) as usize;
            let timestamp = u32::from_be_bytes([
                flv[offset + 7],
                flv[offset + 4],
                flv[offset + 5],
                flv[offset + 6],
            ]);
            let data = flv[offset + 11..offset + 11 + size].to_vec().into();
            let result = match flv[offset] {
                8 => self
                    .session
                    .publish_audio_data(data, RtmpTimestamp::new(timestamp), false),
                9 => self
                    .session
                    .publish_video_data(data, RtmpTimestamp::new(timestamp), false),
                _ => {
                    offset += 15 + size;
                    continue;
                }
            }
            .unwrap();
            if let ClientSessionResult::OutboundResponse(packet) = result {
                self.socket.write_all(&packet.bytes).await.unwrap();
            }
            offset += 15 + size;
        }
    }
}

async fn ended(rx: &mut broadcast::Receiver<RecorderEvent>) -> (String, f64, u64) {
    timeout(DEADLINE, async {
        let mut live_id = String::new();
        let mut duration = 0.0;
        let mut size = 0;
        let mut record_ended = false;
        loop {
            match rx.recv().await.unwrap() {
                RecorderEvent::RecordStart { recorder } => {
                    assert!(live_id.is_empty());
                    live_id = recorder.live_id;
                }
                RecorderEvent::RecordUpdate {
                    live_id: id,
                    duration_secs,
                    cached_size_bytes,
                } => {
                    assert_eq!(id, live_id);
                    assert!(!record_ended);
                    duration += duration_secs;
                    size += cached_size_bytes;
                }
                RecorderEvent::RecordEnd { recorder } => {
                    assert_eq!(recorder.live_id, live_id);
                    assert!(!recorder.recording);
                    record_ended = true;
                }
                RecorderEvent::LiveEnd { recorder, .. } => {
                    assert!(record_ended);
                    assert_eq!(recorder.platform_live_id, live_id);
                    return (live_id, duration, size);
                }
                _ => {}
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn push_reconnect_and_stop_preserve_replay_and_release_port() {
    let directory = tempfile::tempdir().unwrap();
    let (recorder, address, mut events) = recorder(directory.path()).await;
    assert_eq!(recorder.config().port, address.port());
    assert_ne!(recorder.config().port, 0);
    assert!(!recorder.info().await.room_info.status);
    assert!(!recorder.info().await.recording);
    assert!(events.try_recv().is_err()); // Waiting must not create empty archives.
    let mut previous = String::new();
    for _ in 0..2 {
        let mut publisher = Publisher::connect(address, "live", &recorder.extra.config.stream_key)
            .await
            .unwrap();
        assert!(recorder.info().await.room_info.status);
        assert!(recorder.info().await.recording);
        publisher.fixture().await;
        drop(publisher);
        let (id, duration, size) = ended(&mut events).await;
        assert_ne!(id, previous);
        let work_dir = recorder.work_dir(&id).await.full_path();
        let bytes = std::fs::read(work_dir.join("playlist.m3u8")).unwrap();
        let (_, playlist) = m3u8_rs::parse_media_playlist(&bytes).unwrap();
        assert!(playlist.end_list);
        assert!(playlist.segments.len() >= 2);
        assert!((duration - 6.0).abs() < 0.2, "{duration}");
        let actual_size: u64 = playlist
            .segments
            .iter()
            .map(|s| std::fs::metadata(work_dir.join(&s.uri)).unwrap().len())
            .sum();
        assert_eq!(size, actual_size);
        assert!(size > 0);
        let probe = ffmpeg_utils::probe_media(&work_dir.join("playlist.m3u8"))
            .await
            .unwrap();
        assert_eq!(probe.metadata.video_codec, "h264");
        assert_eq!(probe.metadata.audio_codec, "aac");
        assert_eq!((probe.metadata.width, probe.metadata.height), (64, 48));
        assert!(work_dir.join("cover.jpg").exists());
        assert!(!recorder.info().await.recording);
        assert!(!recorder.info().await.room_info.status);
        assert!(recorder.info().await.live_id.is_empty());
        previous = id;
    }
    let conflict = RtmpRecorder::new(
        "other",
        RtmpConfig {
            port: address.port(),
            ..recorder.extra.config.clone()
        },
        directory.path().to_owned(),
        broadcast::channel(1).0,
        true,
    )
    .await;
    assert!(conflict.is_err());
    timeout(DEADLINE, recorder.stop()).await.unwrap();
    let _rebound = TcpListener::bind(address).await.unwrap();
}

#[tokio::test]
async fn rejects_wrong_key_and_second_publisher_and_can_disable_and_restart() {
    let directory = tempfile::tempdir().unwrap();
    let (recorder, address, mut events) = recorder(directory.path()).await;
    assert!(
        Publisher::connect(address, "wrong-app", &recorder.extra.config.stream_key)
            .await
            .is_err()
    );
    assert!(Publisher::connect(address, "live", "wrong-key")
        .await
        .is_err());
    assert!(events.try_recv().is_err());
    let mut publisher = Publisher::connect(address, "live", &recorder.extra.config.stream_key)
        .await
        .unwrap();
    assert!(
        Publisher::connect(address, "live", &recorder.extra.config.stream_key)
            .await
            .is_err()
    );
    publisher.fixture().await;
    recorder.disable().await;
    ended(&mut events).await;
    assert!(!recorder.info().await.enabled);
    assert!(
        Publisher::connect(address, "live", &recorder.extra.config.stream_key)
            .await
            .is_err()
    );
    recorder.enable().await;
    let mut publisher = Publisher::connect(address, "live", &recorder.extra.config.stream_key)
        .await
        .unwrap();
    publisher.fixture().await;
    timeout(DEADLINE, recorder.stop()).await.unwrap();
    ended(&mut events).await;
    let _rebound = TcpListener::bind(address).await.unwrap();
}

#[tokio::test]
async fn stopping_interrupts_an_incomplete_handshake() {
    let directory = tempfile::tempdir().unwrap();
    let (recorder, address, _) = recorder(directory.path()).await;
    let mut idle = TcpStream::connect(address).await.unwrap();
    idle.write_all(&[3]).await.unwrap();
    tokio::task::yield_now().await;
    timeout(Duration::from_secs(2), recorder.stop())
        .await
        .unwrap();
    let closed = timeout(Duration::from_secs(1), idle.read(&mut [0u8; 1]))
        .await
        .unwrap();
    assert!(matches!(closed, Ok(0) | Err(_)));
    drop(idle);
    let _rebound = TcpListener::bind(address).await.unwrap();
}

async fn advance_connection_clock(duration: Duration) {
    // Advance deadlines without spending 15 seconds in each regression test;
    // keep real TCP and native recording I/O on a running clock.
    tokio::time::pause();
    tokio::time::advance(duration).await;
    tokio::time::resume();
}

#[tokio::test]
async fn unauthenticated_keepalives_expire_and_release_all_connection_slots() {
    let directory = tempfile::tempdir().unwrap();
    let (recorder, address, mut events) = recorder(directory.path()).await;
    let mut idle = Vec::new();
    for _ in 0..8 {
        idle.push(
            Publisher::connect_without_publishing(address)
                .await
                .unwrap(),
        );
    }
    let step = session::AUTH_TIMEOUT / 4;
    for _ in 0..3 {
        advance_connection_clock(step).await;
        for client in &mut idle {
            client.ping().await;
        }
    }
    advance_connection_clock(step + Duration::from_millis(100)).await;
    for client in &mut idle {
        let closed = timeout(Duration::from_secs(1), client.socket.read(&mut [0; 1]))
            .await
            .expect("unauthenticated keepalives extended the authentication deadline");
        assert!(matches!(closed, Ok(0) | Err(_)));
    }
    assert!(events.try_recv().is_err());
    assert!(!recorder.info().await.recording);
    let mut publisher = Publisher::connect(address, "live", &recorder.config().stream_key)
        .await
        .unwrap();
    publisher.fixture().await;
    drop(publisher);
    let (_, duration, size) = ended(&mut events).await;
    assert!(duration > 0.0 && size > 0);
    recorder.stop().await;
}

#[tokio::test]
async fn authenticated_publisher_stays_connected_past_the_authentication_deadline() {
    let directory = tempfile::tempdir().unwrap();
    let (recorder, address, mut events) = recorder(directory.path()).await;
    let mut publisher = Publisher::connect(address, "live", &recorder.config().stream_key)
        .await
        .unwrap();
    publisher.fixture().await;
    for _ in 0..5 {
        advance_connection_clock(session::AUTH_TIMEOUT / 4).await;
        publisher.ping().await;
    }
    assert!(recorder.info().await.recording);
    drop(publisher);
    let (_, duration, size) = ended(&mut events).await;
    assert!((duration - 6.0).abs() < 0.2 && size > 0);
    recorder.stop().await;
}

#[tokio::test]
async fn media_timeout_ends_a_publisher_with_keepalives_and_allows_reconnection() {
    let directory = tempfile::tempdir().unwrap();
    let (recorder, address, mut events) = recorder(directory.path()).await;
    let mut publisher = Publisher::connect(address, "live", &recorder.config().stream_key)
        .await
        .unwrap();
    publisher.fixture().await;

    // The native reader uses wall-clock time. Keep RTMP control traffic flowing
    // while withholding media, so only the recording worker's timeout can end it.
    let mut pings = tokio::time::interval(Duration::from_secs(1));
    let mut buffer = [0; 4096];
    let mut responses = 0;
    timeout(Duration::from_secs(40), async {
        'connected: loop {
            tokio::select! {
                _ = pings.tick() => {
                    let (packet, _) = publisher.session.send_ping_request().unwrap();
                    if publisher.socket.write_all(&packet.bytes).await.is_err() {
                        break;
                    }
                }
                result = publisher.socket.read(&mut buffer) => {
                    let length = match result {
                        Ok(0) | Err(_) => break,
                        Ok(length) => length,
                    };
                    for result in publisher.session.handle_input(&buffer[..length]).unwrap() {
                        match result {
                            ClientSessionResult::RaisedEvent(
                                ClientSessionEvent::PingResponseReceived { .. },
                            ) => responses += 1,
                            ClientSessionResult::OutboundResponse(packet) => {
                                if publisher.socket.write_all(&packet.bytes).await.is_err() {
                                    break 'connected;
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    })
    .await
    .expect("RTMP keepalives kept the publisher connected after its recording worker exited");
    assert!(responses > 1);

    let (first_id, duration, size) = ended(&mut events).await;
    assert!((duration - 6.0).abs() < 0.2 && size > 0);
    let work_dir = recorder.work_dir(&first_id).await.full_path();
    let bytes = std::fs::read(work_dir.join("playlist.m3u8")).unwrap();
    let (_, playlist) = m3u8_rs::parse_media_playlist(&bytes).unwrap();
    assert!(playlist.end_list);
    let actual_size: u64 = playlist
        .segments
        .iter()
        .map(|segment| {
            std::fs::metadata(work_dir.join(&segment.uri))
                .unwrap()
                .len()
        })
        .sum();
    assert_eq!(size, actual_size);
    let idle = recorder.info().await;
    assert!(!idle.recording && !idle.room_info.status);
    assert!(idle.live_id.is_empty());
    assert!(events.try_recv().is_err());

    let mut replacement = Publisher::connect(address, "live", &recorder.config().stream_key)
        .await
        .unwrap();
    assert!(recorder.info().await.recording);
    replacement.fixture().await;
    drop(replacement);
    let (next_id, duration, size) = ended(&mut events).await;
    assert_ne!(first_id, next_id);
    assert!((duration - 6.0).abs() < 0.2 && size > 0);
    recorder.stop().await;
}

#[tokio::test]
async fn simultaneous_publishers_have_distinct_archive_ids_and_isolated_streams() {
    let first_dir = tempfile::tempdir().unwrap();
    let second_dir = tempfile::tempdir().unwrap();
    let (first, first_addr, mut first_events) = recorder(first_dir.path()).await;
    let (second, second_addr, mut second_events) = recorder(second_dir.path()).await;
    assert_ne!(first_addr.port(), second_addr.port());
    assert_ne!(first.config().stream_key, second.config().stream_key);
    let (first_publisher, second_publisher) = tokio::join!(
        Publisher::connect(first_addr, "live", &first.extra.config.stream_key),
        Publisher::connect(second_addr, "live", &second.extra.config.stream_key),
    );
    let mut first_publisher = first_publisher.unwrap();
    let mut second_publisher = second_publisher.unwrap();
    assert_ne!(first.info().await.live_id, second.info().await.live_id);
    first_publisher.fixture().await;
    second_publisher.fixture().await;
    drop(first_publisher);
    drop(second_publisher);
    let (first_archive, second_archive) =
        tokio::join!(ended(&mut first_events), ended(&mut second_events));
    assert_ne!(first_archive.0, second_archive.0);
    assert!(first_archive.2 > 0 && second_archive.2 > 0);
    first.stop().await;
    second.stop().await;
}

#[tokio::test]
#[ignore = "requires the optional FFmpeg CLI publisher"]
async fn ffmpeg_publisher_records_through_the_obs_compatible_protocol() {
    let directory = tempfile::tempdir().unwrap();
    let (recorder, address, mut events) = recorder(directory.path()).await;
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../ffmpeg_utils/src/hls/fixtures/live.flv");
    let output = timeout(
        DEADLINE,
        ffmpeg_utils::ffmpeg_command()
            .args(["-hide_banner", "-loglevel", "error", "-i"])
            .arg(fixture)
            .args(["-c", "copy", "-f", "flv"])
            .arg(format!(
                "rtmp://{address}/live/{}",
                recorder.extra.config.stream_key
            ))
            .output(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let (id, duration, size) = ended(&mut events).await;
    assert!((duration - 6.0).abs() < 0.2);
    assert!(size > 0);
    let path = recorder
        .work_dir(&id)
        .await
        .full_path()
        .join("playlist.m3u8");
    assert!(std::fs::read_to_string(path)
        .unwrap()
        .contains("#EXT-X-ENDLIST"));
    recorder.stop().await;
}

#[test]
fn new_rooms_require_only_a_name_and_generate_their_own_credentials() {
    let first = RtmpConfig::for_new_room(r#"{"name":"  OBS  "}"#).unwrap();
    let second = RtmpConfig::for_new_room(r#"{"name":"OBS"}"#).unwrap();
    assert_eq!(first.name, "OBS");
    assert_eq!(first.port, 0);
    assert_ne!(first.stream_key, second.stream_key);
    assert!(uuid::Uuid::parse_str(&first.stream_key).is_ok());
    for invalid in [
        r#"{}"#,
        r#"{"name":" "}"#,
        r#"{"name":"OBS","port":1935}"#,
        r#"{"name":"OBS","stream_key":"chosen-key"}"#,
    ] {
        assert!(RtmpConfig::for_new_room(invalid).is_err(), "{invalid}");
    }
}

#[tokio::test]
async fn saved_configuration_reuses_the_allocated_port_and_key_after_restart() {
    let directory = tempfile::tempdir().unwrap();
    let (original, address, _) = recorder(directory.path()).await;
    let stored = serde_json::to_string(original.config()).unwrap();
    let key = original.config().stream_key.clone();
    original.stop().await;
    let (tx, _rx) = broadcast::channel(64);
    let restored = RtmpRecorder::new(
        "obs",
        RtmpConfig::parse(&stored).unwrap(),
        directory.path().to_owned(),
        tx,
        true,
    )
    .await
    .unwrap();
    assert_eq!(restored.config().port, address.port());
    assert_eq!(restored.config().stream_key, key);
    restored.run().await;
    let publisher = Publisher::connect(address, "live", &key).await.unwrap();
    assert!(restored.info().await.room_info.status);
    assert!(restored.info().await.recording);
    drop(publisher);
    timeout(DEADLINE, restored.stop()).await.unwrap();
}

#[test]
fn config_roundtrip_keeps_key_and_rejects_invalid_addresses_and_paths() {
    let config = RtmpConfig::parse(r#"{"name":"OBS","port":1936}"#).unwrap();
    let stored = serde_json::to_string(&config).unwrap();
    let loaded = RtmpConfig::parse(&stored).unwrap();
    assert_eq!(loaded.stream_key, config.stream_key);
    assert_eq!(loaded.port, 1936);
    for invalid in [
        r#"{"port":65536}"#,
        r#"{"stream_key":"../x"}"#,
        r#"{"name":" "}"#,
        r#"{"listen_addr":"hostname"}"#,
    ] {
        assert!(RtmpConfig::parse(invalid).is_err(), "{invalid}");
    }
    assert!(normalize_room_id("../outside").is_err());
    assert_eq!(normalize_room_id("obs").unwrap(), "rtmp-obs");
    assert_eq!(normalize_room_id("rtmp-obs").unwrap(), "rtmp-obs");
    assert_ne!(
        normalize_room_id("").unwrap(),
        normalize_room_id("").unwrap()
    );
}
