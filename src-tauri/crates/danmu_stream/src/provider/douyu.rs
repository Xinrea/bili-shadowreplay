//! Douyu (斗鱼) danmaku provider.
//!
//! Douyu's danmaku websocket uses STT (Serialized Text Transport), a small
//! key/value protocol wrapped in a length-prefixed binary packet.  The packet
//! decoder intentionally lives in this module instead of relying on websocket
//! message boundaries: a websocket message may contain half a Douyu packet or
//! several packets.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use log::{debug, error, info, warn};
use serde_json::{json, Map, Value};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, watch, Mutex};
use tokio::time::{sleep, timeout};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{header, HeaderValue};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

use crate::provider::DanmuProvider;
use crate::{DanmuMessage, DanmuMessageType, DanmuStreamError, LiveEvent};

const DOUYU_WS_HOST: &str = "danmuproxy.douyu.com";
const DOUYU_WS_PORTS: [u16; 2] = [8502, 8506];
const DOUYU_ORIGIN: &str = "https://www.douyu.com";
const DOUYU_REFERER: &str = "https://www.douyu.com/";
const DOUYU_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(45);
const RECONNECT_DELAY: Duration = Duration::from_secs(5);
const WEBSOCKET_IO_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_GROUP_ID: &str = "-9999";

/// The packet length in the first two fields includes the second length field,
/// magic field, payload, and terminating NUL, but not the first length field.
const PACKET_LENGTH_OVERHEAD: usize = 9;
const PACKET_HEADER_LEN: usize = 12;
const PACKET_MIN_LENGTH: usize = PACKET_LENGTH_OVERHEAD;
/// Do not allow a corrupt length field to make the accumulator grow without
/// bound while waiting for a packet that can never arrive.
const MAX_PACKET_LENGTH: usize = 16 * 1024 * 1024;
const CLIENT_MAGIC: [u8; 4] = [0xb1, 0x02, 0x00, 0x00];
const SERVER_MAGIC: [u8; 4] = [0xb2, 0x02, 0x00, 0x00];

const CONTROL_MESSAGE_TYPES: &[&str] = &[
    "loginres",
    "joingroup",
    "keeplive",
    "keepalive",
    "pingreq",
    "mrkl",
];

type SttMap = BTreeMap<String, String>;
type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;
type WsRead = futures_util::stream::SplitStream<WsStream>;
type WsWrite = futures_util::stream::SplitSink<WsStream, WsMessage>;

/// Errors raised by the strict STT packet codec.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FrameError {
    Invalid(String),
    TooLarge(usize),
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => write!(f, "invalid Douyu STT packet: {message}"),
            Self::TooLarge(length) => write!(f, "Douyu STT packet is too large: {length} bytes"),
        }
    }
}

impl std::error::Error for FrameError {}

/// Decode one packet from the beginning of `data`.
///
/// `Ok(None)` means that the bytes are a valid prefix but more bytes are
/// needed.  Once a complete packet is available, the returned `usize` is the
/// exact number of bytes consumed, allowing callers to process coalesced
/// packets without guessing from a trailing NUL.
fn decode_packet(data: &[u8]) -> Result<Option<(Vec<u8>, usize)>, FrameError> {
    // The two length fields are themselves part of the framing and can arrive
    // separately from the rest of the header.
    if data.len() < 8 {
        return Ok(None);
    }

    let length = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
    if length < PACKET_MIN_LENGTH {
        return Err(FrameError::Invalid(format!(
            "length field {length} is smaller than {PACKET_MIN_LENGTH}"
        )));
    }
    if length > MAX_PACKET_LENGTH {
        return Err(FrameError::TooLarge(length));
    }

    let second_length = u32::from_le_bytes([data[4], data[5], data[6], data[7]]) as usize;
    if second_length != length {
        return Err(FrameError::Invalid(format!(
            "length fields differ ({length} != {second_length})"
        )));
    }

    if data.len() < PACKET_HEADER_LEN {
        return Ok(None);
    }

    let magic = &data[8..12];
    if magic != CLIENT_MAGIC.as_slice() && magic != SERVER_MAGIC.as_slice() {
        return Err(FrameError::Invalid(format!(
            "unexpected magic {magic:02x?}"
        )));
    }

    // `length` excludes the first four-byte length field.
    let total_size = length.checked_add(4).ok_or(FrameError::TooLarge(length))?;
    if data.len() < total_size {
        return Ok(None);
    }

    let payload_end = total_size - 1;
    if data[payload_end] != 0 {
        return Err(FrameError::Invalid(
            "packet is missing its NUL terminator".to_string(),
        ));
    }

    Ok(Some((
        data[PACKET_HEADER_LEN..payload_end].to_vec(),
        total_size,
    )))
}

/// Encode an STT payload using the client-to-server packet magic.
fn encode_packet(payload: &str) -> Result<Vec<u8>, FrameError> {
    let length = payload
        .len()
        .checked_add(PACKET_LENGTH_OVERHEAD)
        .ok_or(FrameError::TooLarge(payload.len()))?;
    if length > MAX_PACKET_LENGTH || length > u32::MAX as usize {
        return Err(FrameError::TooLarge(length));
    }

    let length = length as u32;
    let mut packet = Vec::with_capacity(length as usize + 4);
    packet.extend_from_slice(&length.to_le_bytes());
    packet.extend_from_slice(&length.to_le_bytes());
    packet.extend_from_slice(&CLIENT_MAGIC);
    packet.extend_from_slice(payload.as_bytes());
    packet.push(0);
    Ok(packet)
}

/// Accumulates arbitrary websocket chunks and yields complete STT payloads.
#[derive(Debug, Default)]
struct PacketDecoder {
    buffer: Vec<u8>,
}

impl PacketDecoder {
    fn feed(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, FrameError> {
        self.buffer.extend_from_slice(bytes);

        let mut packets = Vec::new();
        loop {
            let Some((payload, consumed)) = decode_packet(&self.buffer)? else {
                break;
            };
            packets.push(payload);
            self.buffer.drain(..consumed);
        }
        Ok(packets)
    }

    #[cfg(test)]
    fn buffered_len(&self) -> usize {
        self.buffer.len()
    }
}

/// Escape the two STT control characters in a key or value.
fn stt_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '@' => escaped.push_str("@A"),
            '/' => escaped.push_str("@S"),
            _ => escaped.push(character),
        }
    }
    escaped
}

/// Undo STT escaping. Unknown/incomplete escape sequences are retained rather
/// than discarded, which keeps malformed provider fields observable in `raw`.
fn stt_unescape(value: &str) -> String {
    let mut unescaped = String::with_capacity(value.len());
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        if character == '@' {
            match characters.next() {
                Some('A') => unescaped.push('@'),
                Some('S') => unescaped.push('/'),
                Some(other) => {
                    unescaped.push('@');
                    unescaped.push(other);
                }
                None => unescaped.push('@'),
            }
        } else {
            unescaped.push(character);
        }
    }
    unescaped
}

/// Encode fields in a deterministic order supplied by the caller.
fn stt_encode(fields: &[(&str, &str)]) -> String {
    let mut encoded = String::new();
    for (key, value) in fields {
        encoded.push_str(&stt_escape(key));
        encoded.push_str("@=");
        encoded.push_str(&stt_escape(value));
        encoded.push('/');
    }
    encoded
}

/// Encode a decoded field map. This is useful for codec round-trip tests and
/// for callers that need deterministic output independent of hash iteration.
#[cfg(test)]
fn stt_encode_map(map: &SttMap) -> String {
    let mut encoded = String::new();
    for (key, value) in map {
        encoded.push_str(&stt_escape(key));
        encoded.push_str("@=");
        encoded.push_str(&stt_escape(value));
        encoded.push('/');
    }
    encoded
}

/// Decode an STT text payload into all its key/value fields.
fn stt_decode(payload: &str) -> SttMap {
    let mut fields = SttMap::new();
    for (index, field) in payload.split('/').enumerate() {
        if field.is_empty() {
            continue;
        }

        let Some((key, value)) = field.split_once("@=") else {
            // STT normally never emits a field without `@=`, but retaining it
            // under a stable synthetic key is preferable to silently losing
            // an abnormal provider field in the raw event.
            fields.insert(format!("__stt_invalid_field_{index}"), stt_unescape(field));
            continue;
        };
        fields.insert(stt_unescape(key), stt_unescape(value));
    }
    fields
}

fn stt_decode_bytes(payload: &[u8]) -> SttMap {
    stt_decode(&String::from_utf8_lossy(payload))
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn field<'a>(map: &'a SttMap, names: &[&str]) -> Option<&'a str> {
    names
        .iter()
        .find_map(|name| map.get(*name).map(String::as_str))
}

fn parse_u64(raw: Option<&str>) -> Option<u64> {
    let raw = raw?.trim();
    if let Some(hex) = raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16).ok()
    } else {
        raw.parse().ok()
    }
}

fn parse_color(raw: Option<&str>) -> u32 {
    let Some(raw) = raw.map(str::trim).filter(|raw| !raw.is_empty()) else {
        return 0;
    };
    let raw = raw.strip_prefix('#').unwrap_or(raw);
    if let Some(hex) = raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X")) {
        return u32::from_str_radix(hex, 16).unwrap_or(0);
    }
    let Ok(color) = raw.parse::<u32>() else {
        return u32::from_str_radix(raw, 16).unwrap_or(0);
    };
    match color {
        0 => 0x00ff_ffff,
        1 => 0x00ff_0000,
        2 => 0x001e_90ff,
        3 => 0x0000_ff00,
        4 => 0x00ff_7f00,
        5 => 0x00ff_00ff,
        6 => 0x0000_ffff,
        _ if color > 0xffff => color,
        _ => 0x00ff_ffff,
    }
}

fn timestamp_from_map(map: &SttMap) -> i64 {
    for name in ["cst", "timestamp", "ts"] {
        if let Some(timestamp) = map.get(name).and_then(|value| value.parse::<i64>().ok()) {
            if timestamp > 0 {
                // Douyu's chat `cst` field is normally Unix seconds, while
                // some newer payloads expose millisecond timestamps.  Keep
                // the crate-wide millisecond representation in both cases.
                return if timestamp < 10_000_000_000 {
                    timestamp.saturating_mul(1_000)
                } else {
                    timestamp
                };
            }
        }
    }
    now_millis()
}

fn json_id(raw: Option<&str>) -> Value {
    match raw {
        Some(value) => match parse_u64(Some(value)) {
            Some(number) => json!(number),
            None => Value::String(value.to_string()),
        },
        None => Value::Null,
    }
}

fn map_to_json(map: &SttMap) -> Value {
    let object: Map<String, Value> = map
        .iter()
        .map(|(key, value)| (key.clone(), Value::String(value.clone())))
        .collect();
    Value::Object(object)
}

/// Convert a Douyu `chatmsg` map to the crate's existing danmaku type.
///
/// Missing fields receive the same harmless defaults as the surrounding
/// providers instead of causing the complete message to be dropped.  The
/// original protocol has no raw field on `DanmuMessage`; gift and unknown
/// messages below therefore use `LiveEvent::raw` for forward compatibility.
fn map_chat_message(map: &SttMap, room_id: &str) -> DanmuMessage {
    DanmuMessage {
        room_id: room_id.to_string(),
        user_id: parse_u64(field(map, &["uid", "user_id"])).unwrap_or(0),
        user_name: field(map, &["nn", "user_name"])
            .unwrap_or_default()
            .to_string(),
        message: field(map, &["txt", "content"])
            .unwrap_or_default()
            .to_string(),
        color: parse_color(field(map, &["col", "color"])),
        timestamp: timestamp_from_map(map),
    }
}

fn map_gift_event(map: &SttMap, room_id: &str, message_type: &str) -> LiveEvent {
    let gift_name = field(map, &["gfn", "gfname", "gift_name"])
        .unwrap_or("Gift")
        .to_string();
    let gift_id = field(map, &["gfid", "gift_id"])
        .unwrap_or_default()
        .to_string();
    let count = parse_u64(field(map, &["gfcnt", "count"])).unwrap_or(1);
    let hits = parse_u64(field(map, &["hits", "combo_count"])).unwrap_or(0);
    let raw = map_to_json(map);

    let mut event = LiveEvent::new(
        "douyu",
        room_id,
        "gift",
        json!({
            "user_id": json_id(field(map, &["uid", "user_id"])),
            "user_name": field(map, &["nn", "user_name"]).unwrap_or_default(),
            "gift_id": gift_id,
            "gift_name": gift_name,
            "count": count,
            "hits": hits,
            "message_type": message_type,
        }),
    );
    event.ts = timestamp_from_map(map);
    event.raw = raw;
    event
}

fn map_unknown_event(map: &SttMap, room_id: &str, message_type: &str) -> LiveEvent {
    let raw = map_to_json(map);
    let event_type = if message_type == "uenter" {
        "enter"
    } else {
        "unknown"
    };
    let mut event = LiveEvent::new("douyu", room_id, event_type, raw.clone());
    event.ts = timestamp_from_map(map);
    event.raw = raw;
    event
}

/// Map one decoded STT payload to an output event. Protocol acknowledgements
/// and heartbeats are intentionally not emitted as user-facing events.
fn map_message(map: &SttMap, room_id: &str) -> Option<DanmuMessageType> {
    if map.is_empty() {
        return None;
    }

    let message_type = map.get("type").map(String::as_str).unwrap_or("");
    match message_type {
        "chatmsg" => Some(DanmuMessageType::DanmuMessage(map_chat_message(
            map, room_id,
        ))),
        "dgb" | "gift" => Some(DanmuMessageType::Event(map_gift_event(
            map,
            room_id,
            message_type,
        ))),
        type_name if CONTROL_MESSAGE_TYPES.contains(&type_name) => None,
        _ => Some(DanmuMessageType::Event(map_unknown_event(
            map,
            room_id,
            message_type,
        ))),
    }
}

pub struct DouyuDanmu {
    room_id: String,
    cookie: String,
    stop: watch::Sender<bool>,
    write: Arc<Mutex<Option<WsWrite>>>,
}

#[async_trait]
impl DanmuProvider for DouyuDanmu {
    async fn new(cookie: &str, room_id: &str) -> Result<Self, DanmuStreamError> {
        let (stop, _stop_rx) = watch::channel(false);
        Ok(Self {
            room_id: room_id.to_string(),
            cookie: cookie.to_string(),
            stop,
            write: Arc::new(Mutex::new(None)),
        })
    }

    async fn start(
        &self,
        tx: mpsc::UnboundedSender<DanmuMessageType>,
    ) -> Result<(), DanmuStreamError> {
        info!(
            "Douyu WebSocket connection started, room_id: {}",
            self.room_id
        );

        while !self.is_stopped() {
            let mut last_error = None;

            // Try both endpoints for every reconnect cycle.  A failed 8502
            // handshake must not prevent trying 8506 immediately.
            for port in DOUYU_WS_PORTS {
                if self.is_stopped() {
                    break;
                }

                match self.connect_and_handle(port, tx.clone()).await {
                    Ok(()) => {
                        // `Ok(())` is used for an intentional stop; a remote
                        // close/error returns Err and enters the retry path.
                        break;
                    }
                    Err(error) => {
                        warn!(
                            "Douyu websocket port {} failed for room {}: {}",
                            port, self.room_id, error
                        );
                        last_error = Some(error);
                    }
                }
            }

            if self.is_stopped() {
                break;
            }
            if let Some(error) = last_error {
                error!(
                    "Douyu websocket disconnected, room_id: {}, retrying: {}",
                    self.room_id, error
                );
            }
            if self.wait_or_stop(RECONNECT_DELAY).await {
                break;
            }
        }

        info!(
            "Douyu WebSocket connection stopped, room_id: {}",
            self.room_id
        );
        Ok(())
    }

    async fn stop(&self) -> Result<(), DanmuStreamError> {
        self.stop.send_replace(true);

        // Closing the write half complements the watch cancellation.  It
        // wakes a pending websocket read on implementations that do not
        // immediately observe the cancellation signal.
        let Ok(mut writer) = timeout(WEBSOCKET_IO_TIMEOUT, self.write.lock()).await else {
            warn!("timed out waiting for the Douyu websocket writer to stop");
            return Ok(());
        };
        if let Some(mut write) = writer.take() {
            match timeout(WEBSOCKET_IO_TIMEOUT, write.close()).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => debug!("failed to close Douyu websocket: {}", error),
                Err(_) => warn!("timed out closing the Douyu websocket"),
            }
        }
        Ok(())
    }
}

impl DouyuDanmu {
    fn is_stopped(&self) -> bool {
        *self.stop.borrow()
    }

    async fn wait_or_stop(&self, duration: Duration) -> bool {
        let mut stop_rx = self.stop.subscribe();
        tokio::select! {
            _ = sleep(duration) => false,
            changed = stop_rx.changed() => changed.is_err() || *stop_rx.borrow(),
        }
    }

    async fn connect_and_handle(
        &self,
        port: u16,
        tx: mpsc::UnboundedSender<DanmuMessageType>,
    ) -> Result<(), DanmuStreamError> {
        if self.is_stopped() {
            return Ok(());
        }

        let url = format!("wss://{DOUYU_WS_HOST}:{port}/");
        let mut request =
            url.into_client_request()
                .map_err(|error| DanmuStreamError::WebsocketError {
                    err: format!("failed to build Douyu websocket request: {error}"),
                })?;
        {
            let headers = request.headers_mut();
            headers.insert(header::ORIGIN, HeaderValue::from_static(DOUYU_ORIGIN));
            headers.insert(header::REFERER, HeaderValue::from_static(DOUYU_REFERER));
            headers.insert(
                header::USER_AGENT,
                HeaderValue::from_static(DOUYU_USER_AGENT),
            );
            if !self.cookie.trim().is_empty() {
                let cookie = HeaderValue::from_str(&self.cookie).map_err(|error| {
                    DanmuStreamError::WebsocketError {
                        err: format!("invalid Douyu cookie header: {error}"),
                    }
                })?;
                headers.insert(header::COOKIE, cookie);
            }
        }

        let mut stop_rx = self.stop.subscribe();
        let (stream, response) = tokio::select! {
            result = connect_async(request) => result.map_err(|error| DanmuStreamError::WebsocketError {
                err: format!("failed to connect to Douyu websocket on port {port}: {error}"),
            })?,
            changed = stop_rx.changed() => {
                if changed.is_err() || *stop_rx.borrow() {
                    return Ok(());
                }
                return Err(DanmuStreamError::WebsocketError {
                    err: "Douyu stop signal changed unexpectedly".to_string(),
                });
            }
        };
        info!(
            "Douyu websocket connected on port {}, status {}, room_id: {}",
            port,
            response.status(),
            self.room_id
        );

        let (write, read) = stream.split();
        *self.write.lock().await = Some(write);

        let result = match self.send_handshake().await {
            Ok(()) => self.run_connection(read, tx).await,
            Err(error) => Err(error),
        };
        self.clear_writer().await;
        result
    }

    async fn send_handshake(&self) -> Result<(), DanmuStreamError> {
        let login = stt_encode(&[("type", "loginreq"), ("roomid", &self.room_id)]);
        let join_group = stt_encode(&[
            ("type", "joingroup"),
            ("rid", &self.room_id),
            ("gid", DEFAULT_GROUP_ID),
        ]);

        debug!("Douyu login payload: {login}");
        debug!("Douyu join-group payload: {join_group}");
        self.send_payload(&login).await?;
        self.send_payload(&join_group).await
    }

    async fn send_payload(&self, payload: &str) -> Result<(), DanmuStreamError> {
        let packet =
            encode_packet(payload).map_err(|error| DanmuStreamError::MessageParseError {
                err: error.to_string(),
            })?;
        self.send_ws_message(WsMessage::binary(packet)).await
    }

    async fn send_ws_message(&self, message: WsMessage) -> Result<(), DanmuStreamError> {
        let mut write = self.write.lock().await;
        let Some(write) = write.as_mut() else {
            return Err(DanmuStreamError::WebsocketError {
                err: "Douyu websocket writer is not available".to_string(),
            });
        };
        timeout(WEBSOCKET_IO_TIMEOUT, write.send(message))
            .await
            .map_err(|_| DanmuStreamError::WebsocketError {
                err: "timed out sending Douyu websocket message".to_string(),
            })?
            .map_err(|error| DanmuStreamError::WebsocketError {
                err: format!("failed to send Douyu websocket message: {error}"),
            })
    }

    async fn clear_writer(&self) {
        let _ = self.write.lock().await.take();
    }

    async fn run_connection(
        &self,
        read: WsRead,
        tx: mpsc::UnboundedSender<DanmuMessageType>,
    ) -> Result<(), DanmuStreamError> {
        let mut receive = Box::pin(self.receive_loop(read, tx));
        let mut heartbeat = Box::pin(self.heartbeat_loop());

        tokio::select! {
            result = &mut receive => result,
            result = &mut heartbeat => result,
        }
    }

    async fn heartbeat_loop(&self) -> Result<(), DanmuStreamError> {
        let mut stop_rx = self.stop.subscribe();
        loop {
            tokio::select! {
                _ = sleep(HEARTBEAT_INTERVAL) => {}
                changed = stop_rx.changed() => {
                    if changed.is_err() || *stop_rx.borrow() {
                        return Ok(());
                    }
                }
            }

            if self.is_stopped() {
                return Ok(());
            }
            self.send_payload("type@=mrkl/").await?;
        }
    }

    async fn receive_loop(
        &self,
        mut read: WsRead,
        tx: mpsc::UnboundedSender<DanmuMessageType>,
    ) -> Result<(), DanmuStreamError> {
        let mut packets = PacketDecoder::default();
        let mut stop_rx = self.stop.subscribe();

        loop {
            if *stop_rx.borrow() {
                return Ok(());
            }

            let message = tokio::select! {
                changed = stop_rx.changed() => {
                    if changed.is_err() || *stop_rx.borrow() {
                        return Ok(());
                    }
                    continue;
                }
                message = read.next() => {
                    match message {
                        Some(Ok(message)) => message,
                        Some(Err(error)) => {
                            return Err(DanmuStreamError::WebsocketError {
                                err: format!("failed to read Douyu websocket message: {error}"),
                            });
                        }
                        None => {
                            return Err(DanmuStreamError::WebsocketError {
                                err: "Douyu websocket closed by peer".to_string(),
                            });
                        }
                    }
                }
            };

            match message {
                WsMessage::Binary(data) => {
                    let payloads = packets.feed(&data).map_err(|error| {
                        DanmuStreamError::MessageParseError {
                            err: error.to_string(),
                        }
                    })?;
                    for payload in payloads {
                        let decoded = stt_decode_bytes(&payload);
                        if decoded
                            .get("type")
                            .is_some_and(|message_type| message_type == "pingreq")
                        {
                            self.send_payload("type@=mrkl/").await?;
                            continue;
                        }
                        if let Some(event) = map_message(&decoded, &self.room_id) {
                            tx.send(event)
                                .map_err(|error| DanmuStreamError::WebsocketError {
                                    err: format!("failed to forward Douyu event: {error}"),
                                })?;
                        }
                    }
                }
                WsMessage::Ping(data) => {
                    self.send_ws_message(WsMessage::Pong(data)).await?;
                }
                WsMessage::Pong(_) => {}
                WsMessage::Close(frame) => {
                    return Err(DanmuStreamError::WebsocketError {
                        err: format!("Douyu websocket closed: {frame:?}"),
                    });
                }
                WsMessage::Text(text) => {
                    // Douyu STT is binary; retain the connection but make an
                    // unexpected text frame visible for diagnostics.
                    warn!("ignoring unexpected Douyu text frame: {text}");
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stt_escape_and_unescape_round_trip_special_fields() {
        let value = "slash / at @ unicode 弹幕 @/";
        assert_eq!(stt_escape(value), "slash @S at @A unicode 弹幕 @A@S");
        assert_eq!(stt_unescape(&stt_escape(value)), value);
        assert_eq!(stt_unescape("broken@Xtail@"), "broken@Xtail@");
    }

    #[test]
    fn stt_encode_decode_preserves_fields_and_unknown_keys() {
        let encoded = stt_encode(&[
            ("type", "chatmsg"),
            ("txt", "hello / @ world"),
            ("new_field", "unknown"),
        ]);
        let decoded = stt_decode(&encoded);
        assert_eq!(decoded.get("type"), Some(&"chatmsg".to_string()));
        assert_eq!(decoded.get("txt"), Some(&"hello / @ world".to_string()));
        assert_eq!(decoded.get("new_field"), Some(&"unknown".to_string()));
        assert_eq!(stt_decode(&stt_encode_map(&decoded)), decoded);
    }

    #[test]
    fn packet_encode_decode_has_strict_lengths_and_payload() {
        let payload = "type@=chatmsg/txt@=hello/";
        let packet = encode_packet(payload).expect("packet should encode");
        let length = u32::from_le_bytes(packet[0..4].try_into().unwrap()) as usize;
        assert_eq!(length, payload.len() + PACKET_LENGTH_OVERHEAD);
        assert_eq!(packet.len(), length + 4);
        assert_eq!(&packet[8..12], &CLIENT_MAGIC);
        assert_eq!(packet[packet.len() - 1], 0);

        let (decoded, consumed) = decode_packet(&packet).unwrap().unwrap();
        assert_eq!(decoded, payload.as_bytes());
        assert_eq!(consumed, packet.len());

        let mut bad_lengths = packet.clone();
        bad_lengths[4..8].copy_from_slice(&(length as u32 + 1).to_le_bytes());
        assert!(matches!(
            decode_packet(&bad_lengths),
            Err(FrameError::Invalid(_))
        ));

        let mut bad_magic = packet.clone();
        bad_magic[8] = 0;
        assert!(matches!(
            decode_packet(&bad_magic),
            Err(FrameError::Invalid(_))
        ));

        let mut bad_terminator = packet.clone();
        *bad_terminator.last_mut().unwrap() = b'x';
        assert!(matches!(
            decode_packet(&bad_terminator),
            Err(FrameError::Invalid(_))
        ));
    }

    #[test]
    fn packet_decoder_handles_every_half_packet_boundary_and_coalescing() {
        let first = encode_packet("type@=first/").unwrap();
        let second = encode_packet("type@=second/").unwrap();

        for split in 0..=first.len() {
            let mut decoder = PacketDecoder::default();
            let first_chunk = decoder.feed(&first[..split]).unwrap();
            if split < first.len() {
                assert!(first_chunk.is_empty());
            } else {
                assert_eq!(first_chunk, vec![b"type@=first/".to_vec()]);
            }

            let second_chunk = decoder.feed(&first[split..]).unwrap();
            if split < first.len() {
                assert_eq!(second_chunk, vec![b"type@=first/".to_vec()]);
            } else {
                assert!(second_chunk.is_empty());
            }
            assert_eq!(decoder.buffered_len(), 0);
        }

        let mut combined = first;
        combined.extend_from_slice(&second);
        let mut decoder = PacketDecoder::default();
        assert_eq!(
            decoder.feed(&combined).unwrap(),
            vec![b"type@=first/".to_vec(), b"type@=second/".to_vec()]
        );
        assert_eq!(decoder.buffered_len(), 0);
    }

    #[test]
    fn packet_decoder_waits_for_payload_length() {
        let packet = encode_packet("type@=test/").unwrap();
        for end in 0..packet.len() {
            assert!(decode_packet(&packet[..end]).unwrap().is_none());
        }
    }

    #[test]
    fn chat_mapping_uses_identity_content_color_and_server_timestamp() {
        let map = stt_decode(
            "type@=chatmsg/rid@=123/uid@=42/nn=bad/nn@=viewer/txt@=hello@Sworld/col@=6/cst@=1700000000123/extra@=kept/",
        );
        let DanmuMessageType::DanmuMessage(message) = map_message(&map, "123").unwrap() else {
            panic!("chatmsg should map to DanmuMessage");
        };
        assert_eq!(message.room_id, "123");
        assert_eq!(message.user_id, 42);
        assert_eq!(message.user_name, "viewer");
        assert_eq!(message.message, "hello/world");
        assert_eq!(message.color, 0x0000_ffff);
        assert_eq!(message.timestamp, 1_700_000_000_123);

        let seconds_map = stt_decode("type@=chatmsg/cst@=1700000000/");
        let DanmuMessageType::DanmuMessage(seconds_message) =
            map_message(&seconds_map, "123").unwrap()
        else {
            panic!("chatmsg should map to DanmuMessage");
        };
        assert_eq!(seconds_message.timestamp, 1_700_000_000_000);
    }

    #[test]
    fn douyu_color_codes_map_to_rgb_values() {
        assert_eq!(parse_color(Some("0")), 0x00ff_ffff);
        assert_eq!(parse_color(Some("1")), 0x00ff_0000);
        assert_eq!(parse_color(Some("6")), 0x0000_ffff);
        assert_eq!(parse_color(Some("0x123456")), 0x0012_3456);
    }

    #[test]
    fn gift_mapping_normalizes_event_and_keeps_full_raw_map() {
        let map = stt_decode(
            "type@=dgb/rid@=123/uid@=42/nn=bad/nn@=viewer/gfid@=7/gfn@=Rose/gfcnt@=3/hits@=2/new_field@=value/",
        );
        let DanmuMessageType::Event(event) = map_message(&map, "123").unwrap() else {
            panic!("dgb should map to LiveEvent");
        };
        assert_eq!(event.event_type, "gift");
        assert_eq!(event.data["user_id"], json!(42));
        assert_eq!(event.data["gift_name"], json!("Rose"));
        assert_eq!(event.data["count"], json!(3));
        assert_eq!(event.raw["new_field"], json!("value"));
        assert_eq!(event.raw["nn"], json!("viewer"));
    }

    #[test]
    fn unknown_mapping_is_not_dropped_and_retains_raw_stt_fields() {
        let map = stt_decode("type@=newmsg/foo@=bar/value@=with@Sslash/");
        let DanmuMessageType::Event(event) = map_message(&map, "room").unwrap() else {
            panic!("unknown message should map to LiveEvent");
        };
        assert_eq!(event.event_type, "unknown");
        assert_eq!(event.raw["type"], json!("newmsg"));
        assert_eq!(event.raw["value"], json!("with/slash"));
        assert_eq!(event.data, event.raw);
    }
}
