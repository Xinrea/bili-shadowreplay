use std::{
    collections::VecDeque,
    io,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use rml_rtmp::{
    handshake::{Handshake, HandshakeProcessResult, PeerType},
    sessions::{
        ServerSession, ServerSessionConfig, ServerSessionEvent as Event,
        ServerSessionResult as Outcome,
    },
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{watch, OwnedMutexGuard},
    task::JoinHandle,
    time::{timeout, timeout_at, Instant},
};

use super::RtmpRecorder;
use crate::{
    core::flv_recorder::FlvRecorder, errors::RecorderError, events::RecorderEvent,
    platforms::PlatformType, traits::RecorderTrait,
};

const IO_TIMEOUT: Duration = Duration::from_secs(15);
pub(super) const AUTH_TIMEOUT: Duration = Duration::from_secs(15);

fn protocol_error(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}

pub(super) async fn receive(
    recorder: RtmpRecorder,
    mut socket: TcpStream,
    mut control: watch::Receiver<u64>,
) {
    if !recorder.should_record().await {
        return;
    }
    let mut connection = Connection {
        recorder,
        publisher: None,
        recording: None,
        live_started: false,
        record_started: false,
    };
    let cancelled = tokio::select! {
        biased;
        _ = control.changed() => true,
        result = connection.serve(&mut socket) => {
            if let Err(error) = result {
                log::debug!("RTMP connection ended: {error}");
            }
            false
        }
    };
    drop(socket);
    connection.finish(cancelled).await;
}

struct Recording {
    writer: Option<TcpStream>,
    worker: Option<JoinHandle<Result<(), RecorderError>>>,
    enabled: Arc<AtomicBool>,
}

impl Recording {
    async fn wait_for_worker(&mut self) {
        if let Some(worker) = self.worker.as_mut() {
            // Borrow the handle while waiting so cancellation leaves it available
            // to finish(). Once joined, never poll the completed handle again.
            let result = worker.await;
            self.worker = None;
            match result {
                Ok(Ok(())) => {}
                Ok(Err(error)) => log::warn!("RTMP recording failed: {error}"),
                Err(error) => log::error!("RTMP recording task failed: {error}"),
            }
        }
    }
}

struct Connection {
    recorder: RtmpRecorder,
    publisher: Option<OwnedMutexGuard<()>>,
    recording: Option<Recording>,
    live_started: bool,
    record_started: bool,
}

impl Connection {
    fn io_deadline(&self, auth_deadline: Instant) -> Instant {
        if self.publisher.is_some() {
            Instant::now() + IO_TIMEOUT
        } else {
            auth_deadline
        }
    }

    async fn serve(&mut self, socket: &mut TcpStream) -> io::Result<()> {
        // Handshake, control packets and partial commands share one deadline
        // until the publish request authenticates and reserves the room.
        let auth_deadline = Instant::now() + AUTH_TIMEOUT;
        let remaining = timeout_at(auth_deadline, handshake(socket)).await??;
        let (mut session, initial) =
            ServerSession::new(ServerSessionConfig::new()).map_err(protocol_error)?;
        let mut pending = VecDeque::from(initial);
        pending.extend(session.handle_input(&remaining).map_err(protocol_error)?);
        let mut buffer = [0u8; 32 * 1024];
        let mut rejected = false;
        loop {
            // Ready socket I/O can win over an expired Tokio timeout; check
            // explicitly so a continuous flood cannot extend authentication.
            if self.publisher.is_none() && Instant::now() >= auth_deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "RTMP authentication timed out",
                ));
            }
            while let Some(outcome) = pending.pop_front() {
                let responses = match outcome {
                    Outcome::OutboundResponse(packet) => {
                        timeout_at(
                            self.io_deadline(auth_deadline),
                            socket.write_all(&packet.bytes),
                        )
                        .await??;
                        continue;
                    }
                    Outcome::RaisedEvent(Event::ConnectionRequested {
                        request_id,
                        app_name,
                    }) => {
                        if app_name == "live" {
                            session.accept_request(request_id)
                        } else {
                            rejected = true;
                            session.reject_request(
                                request_id,
                                "NetConnection.Connect.Rejected",
                                "Use the live application",
                            )
                        }
                    }
                    Outcome::RaisedEvent(Event::PublishStreamRequested {
                        request_id,
                        app_name,
                        stream_key,
                        ..
                    }) => {
                        let allowed = app_name == "live"
                            && stream_key == self.recorder.extra.config.stream_key
                            && self.recorder.should_record().await
                            && self.publisher.is_none();
                        let permit = allowed
                            .then(|| self.recorder.extra.publisher.clone().try_lock_owned().ok())
                            .flatten();
                        if let Some(permit) = permit {
                            self.publisher = Some(permit);
                            self.start().await?;
                            session.accept_request(request_id)
                        } else {
                            rejected = true;
                            session.reject_request(
                                request_id,
                                "NetStream.Publish.BadName",
                                "Invalid key, disabled room or publisher already connected",
                            )
                        }
                    }
                    Outcome::RaisedEvent(Event::AudioDataReceived {
                        data, timestamp, ..
                    }) => {
                        self.write_tag(8, timestamp.value, &data).await?;
                        continue;
                    }
                    Outcome::RaisedEvent(Event::VideoDataReceived {
                        data, timestamp, ..
                    }) => {
                        self.write_tag(9, timestamp.value, &data).await?;
                        continue;
                    }
                    Outcome::RaisedEvent(Event::PublishStreamFinished { .. }) => return Ok(()),
                    Outcome::RaisedEvent(Event::PlayStreamRequested { request_id, .. }) => {
                        rejected = true;
                        session.reject_request(
                            request_id,
                            "NetStream.Play.Failed",
                            "Use the application's replay player",
                        )
                    }
                    _ => continue,
                };
                pending.extend(responses.map_err(protocol_error)?);
            }
            if rejected {
                return Ok(());
            }
            let input = timeout_at(self.io_deadline(auth_deadline), socket.read(&mut buffer));
            let length = match self.recording.as_mut() {
                Some(recording) => tokio::select! {
                    // Control traffic must not retain the publisher permit after
                    // the native recorder has timed out or failed.
                    biased;
                    _ = recording.wait_for_worker() => return Ok(()),
                    result = input => result??,
                },
                None => input.await??,
            };
            if length == 0 {
                return Ok(());
            }
            pending.extend(
                session
                    .handle_input(&buffer[..length])
                    .map_err(protocol_error)?,
            );
        }
    }

    async fn start(&mut self) -> io::Result<()> {
        let recorder = &self.recorder;
        let live_id = crate::new_recording_id();
        let storage = recorder.recording_storage(&live_id).await?;
        storage.prepare().await?;
        let bridge = TcpListener::bind("127.0.0.1:0").await?;
        let source = format!("tcp://{}", bridge.local_addr()?);
        *recorder.live_id.write().await = live_id.clone();
        *recorder.platform_live_id.write().await = live_id.clone();
        recorder.room_info.write().await.status = true;
        recorder
            .event_channel
            .send(RecorderEvent::LiveStart {
                recorder: recorder.info().await,
            })
            .await
            .map_err(protocol_error)?;
        self.live_started = true;
        recorder.is_recording.store(true, Ordering::Release);
        let mut info = recorder.info().await;
        info.storage_id = storage.identity().map(str::to_owned);
        recorder
            .event_channel
            .send(RecorderEvent::RecordStart { recorder: info })
            .await
            .map_err(protocol_error)?;
        self.record_started = true;
        let enabled = Arc::new(AtomicBool::new(true));
        let flv = FlvRecorder::new(
            source,
            None,
            Vec::new(),
            storage.clone(),
            enabled.clone(),
            recorder.event_channel.clone(),
            live_id,
        )
        .with_read_timeout(IO_TIMEOUT);
        self.recording = Some(Recording {
            writer: None,
            enabled,
            worker: Some(tokio::spawn(async move {
                let result = flv.start().await;
                // Push sources have no remote cover. Decode a completed backend
                // object and store the resulting JPEG through the same backend.
                // All cover work is best-effort, including storage lookups:
                // a thumbnail error must never replace the recording result.
                let cover_result: Result<(), RecorderError> = async {
                    if storage.exists("0.ts").await? {
                        let jpeg = ffmpeg_utils::extract_jpeg_from_bytes(
                            storage.read("0.ts").await?,
                            0.0,
                            Some(2),
                        )
                        .await
                        .map_err(RecorderError::FfmpegError)?;
                        storage.write("cover.jpg", &jpeg).await?;
                    }
                    Ok(())
                }
                .await;
                if let Err(error) = cover_result {
                    log::debug!("RTMP cover unavailable: {error}");
                }
                result
            })),
        });
        let (mut writer, _) = timeout(IO_TIMEOUT, bridge.accept()).await??;
        // FLV header: audio and video enabled, followed by PreviousTagSize0.
        writer
            .write_all(b"FLV\x01\x05\x00\x00\x00\x09\x00\x00\x00\x00")
            .await?;
        self.recording.as_mut().unwrap().writer = Some(writer);
        Ok(())
    }

    async fn write_tag(&mut self, kind: u8, timestamp: u32, data: &[u8]) -> io::Result<()> {
        let Some(writer) = self.recording.as_mut().and_then(|r| r.writer.as_mut()) else {
            return Ok(());
        };
        if data.len() > 0xFF_FFFF {
            return Err(protocol_error("RTMP media tag is too large"));
        }
        let size = data.len() as u32;
        let mut header = [0u8; 11];
        header[0] = kind;
        header[1..4].copy_from_slice(&size.to_be_bytes()[1..]);
        header[4..7].copy_from_slice(&timestamp.to_be_bytes()[1..]);
        header[7] = (timestamp >> 24) as u8;
        timeout(IO_TIMEOUT, async {
            writer.write_all(&header).await?;
            writer.write_all(data).await?;
            writer.write_all(&(size + 11).to_be_bytes()).await
        })
        .await?
    }

    async fn finish(&mut self, cancelled: bool) {
        if self.publisher.is_none() {
            return;
        }
        if let Some(mut recording) = self.recording.take() {
            if cancelled {
                recording.enabled.store(false, Ordering::Release);
            }
            drop(recording.writer.take());
            // Await the native worker, including the final segment's size delta,
            // before emitting RecordEnd/LiveEnd or accepting another publisher.
            recording.wait_for_worker().await;
        }
        let recorder = &self.recorder;
        recorder.is_recording.store(false, Ordering::Release);
        recorder.room_info.write().await.status = false;
        let ended = recorder.info().await;
        recorder.live_id.write().await.clear();
        recorder.platform_live_id.write().await.clear();
        if self.record_started {
            let _ = recorder
                .event_channel
                .send(RecorderEvent::RecordEnd {
                    recorder: ended.clone(),
                })
                .await;
        }
        if self.live_started {
            let _ = recorder
                .event_channel
                .send(RecorderEvent::LiveEnd {
                    platform: PlatformType::Rtmp,
                    room_id: recorder.room_id.clone(),
                    recorder: ended,
                })
                .await;
        }
    }
}

async fn handshake(socket: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut handshake = Handshake::new(PeerType::Server);
    let mut buffer = [0u8; 4096];
    loop {
        let length = socket.read(&mut buffer).await?;
        if length == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        match handshake
            .process_bytes(&buffer[..length])
            .map_err(protocol_error)?
        {
            HandshakeProcessResult::InProgress { response_bytes } => {
                socket.write_all(&response_bytes).await?
            }
            HandshakeProcessResult::Completed {
                response_bytes,
                remaining_bytes,
            } => {
                socket.write_all(&response_bytes).await?;
                return Ok(remaining_bytes);
            }
        }
    }
}
