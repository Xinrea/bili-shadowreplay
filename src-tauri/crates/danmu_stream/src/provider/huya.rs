//! Huya danmaku provider.
//!
//! Connects to Huya's chat websocket (`wss://cdnws.api.huya.com`) with the
//! TAF/JCE protocol used by the mobile web client: register the connection to
//! the room's live channels (`WSUserInfo`), keep it alive with a wup
//! heartbeat (`onlineui.OnUserHeartBeat`) and decode `MessageNotice` pushes
//! (uri 1400) into danmu messages. The channel ids are scraped from the
//! room's `m.huya.com` page, the same source the web client uses.

mod jce;
mod messages;

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use base64::engine::general_purpose;
use base64::Engine as _;
use futures_util::{SinkExt, StreamExt, TryStreamExt};
use log::{debug, error, info, warn};
use rand::RngExt;
use tokio::sync::{mpsc, RwLock};
use tokio::time::sleep;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message as WsMessage;

use crate::provider::{DanmuMessageType, DanmuProvider};
use crate::{DanmuMessage, DanmuStreamError};
use serde_json::Value;

use messages::{
    decode_message_notice, decode_push_message, decode_register_rsp, decode_websocket_command,
    encode_heartbeat_wup, encode_register, encode_websocket_command, MessageNotice, PushMessage,
    WebSocketCommand, URI_MESSAGE_NOTICE, WS_CMD_REGISTER_REQ, WS_CMD_REGISTER_RSP,
    WS_CMD_S2C_HEARTBEAT_ACK, WS_CMD_S2C_MSG_PUSH_REQ, WS_CMD_WUP_REQ, WS_CMD_WUP_RSP,
};

const HUYA_WS_URL: &str = "wss://cdnws.api.huya.com";
/// Huya pushes each chat message once per subscribed group ("live" and
/// "chat"), so identical messages from one user arrive back to back; the
/// web-side clients drop repeats inside a short window (`List` in lib.js).
const DEDUP_WINDOW_MS: i64 = 1000;
const DEDUP_MAX_ENTRIES: usize = 100;
const HUYA_ORIGIN: &str = "https://m.huya.com";
const HUYA_USER_AGENT: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148 Safari/604.1";
const HEARTBEAT_INTERVAL_SECS: u64 = 60;

type WsReadType = futures_util::stream::SplitStream<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
>;

type WsWriteType = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    WsMessage,
>;

/// Channel ids the danmaku subscription needs, scraped from the mobile room
/// page (present only while the room is live).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RoomChannelIds {
    /// `lChannelId` — the live channel id (typically the presenter uid).
    topsid: i64,
    /// `lSubChannelId` — the live sub channel id.
    subsid: i64,
    /// `lUid` — room profile uid, used as the group to join.
    yyuid: i64,
}

pub struct HuyaDanmu {
    client: reqwest::Client,
    room_id: String,
    stop: Arc<RwLock<bool>>,
    write: Arc<RwLock<Option<WsWriteType>>>,
}

#[async_trait]
impl DanmuProvider for HuyaDanmu {
    async fn new(cookie: &str, room_id: &str) -> Result<Self, DanmuStreamError> {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "User-Agent",
            reqwest::header::HeaderValue::from_static(HUYA_USER_AGENT),
        );
        if !cookie.trim().is_empty() {
            if let Ok(value) = cookie.parse::<reqwest::header::HeaderValue>() {
                headers.insert("Cookie", value);
            }
        }

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .build()?;

        Ok(Self {
            client,
            room_id: room_id.to_string(),
            stop: Arc::new(RwLock::new(false)),
            write: Arc::new(RwLock::new(None)),
        })
    }

    async fn start(
        &self,
        tx: mpsc::UnboundedSender<DanmuMessageType>,
    ) -> Result<(), DanmuStreamError> {
        let mut retry_count = 0;
        const RETRY_DELAY: Duration = Duration::from_secs(5);
        info!(
            "Huya WebSocket connection started, room_id: {}",
            self.room_id
        );

        loop {
            if *self.stop.read().await {
                info!(
                    "Huya WebSocket connection stopped, room_id: {}",
                    self.room_id
                );
                break;
            }

            match self.connect_and_handle(tx.clone()).await {
                Ok(_) => {
                    info!(
                        "Huya WebSocket connection closed normally, room_id: {}",
                        self.room_id
                    );
                    retry_count = 0;
                }
                Err(e) => {
                    error!(
                        "Huya WebSocket connection error, room_id: {}, error: {}",
                        self.room_id, e
                    );
                    retry_count += 1;
                }
            }

            info!(
                "Retrying connection in {} seconds... (Attempt {}), room_id: {}",
                RETRY_DELAY.as_secs(),
                retry_count,
                self.room_id
            );
            sleep(RETRY_DELAY).await;
        }

        Ok(())
    }

    async fn stop(&self) -> Result<(), DanmuStreamError> {
        *self.stop.write().await = true;
        if let Some(mut write) = self.write.write().await.take() {
            if let Err(e) = write.close().await {
                error!("Failed to close Huya WebSocket connection: {}", e);
            }
        }
        Ok(())
    }
}

impl HuyaDanmu {
    async fn connect_and_handle(
        &self,
        tx: mpsc::UnboundedSender<DanmuMessageType>,
    ) -> Result<(), DanmuStreamError> {
        let ids = self.fetch_channel_ids().await?;
        info!(
            "Huya danmaku channel ids for room {}: topsid={}, subsid={}, yyuid={}",
            self.room_id, ids.topsid, ids.subsid, ids.yyuid
        );

        let mut request = HUYA_WS_URL
            .into_client_request()
            .map_err(|e| DanmuStreamError::WebsocketError { err: e.to_string() })?;
        let request_headers = request.headers_mut();
        request_headers.insert("User-Agent", HeaderValue::from_static(HUYA_USER_AGENT));
        request_headers.insert("Origin", HeaderValue::from_static(HUYA_ORIGIN));

        let (conn, _) = connect_async(request)
            .await
            .map_err(|e| DanmuStreamError::WebsocketError { err: e.to_string() })?;
        let (write, read) = conn.split();
        *self.write.write().await = Some(write);

        // Register the connection to the room's channels, then keep it alive
        // with an immediate heartbeat and one every interval.
        self.send_frame(&encode_websocket_command(
            WS_CMD_REGISTER_REQ,
            &encode_register(ids.yyuid, &random_guid(), ids.topsid, ids.subsid),
        ))
        .await?;
        let heartbeat_packet = encode_heartbeat_wup(ids.topsid, ids.subsid, ids.yyuid, 1);
        let heartbeat_frame = encode_websocket_command(WS_CMD_WUP_REQ, &heartbeat_packet);
        self.send_frame(&heartbeat_frame).await?;

        let heartbeat_writer = Arc::clone(&self.write);
        tokio::select! {
            v = Self::send_heartbeat_packets(heartbeat_writer, ids, 1) => v,
            v = Self::recv(read, tx, self.room_id.clone(), Arc::clone(&self.stop)) => v
        }
    }

    async fn send_frame(&self, frame: &[u8]) -> Result<(), DanmuStreamError> {
        let mut write = self.write.write().await;
        let Some(write) = write.as_mut() else {
            return Err(DanmuStreamError::WebsocketError {
                err: "connection is closed".to_string(),
            });
        };
        write
            .send(WsMessage::binary(frame.to_vec()))
            .await
            .map_err(|e| DanmuStreamError::WebsocketError { err: e.to_string() })
    }

    async fn send_heartbeat_packets(
        write: Arc<RwLock<Option<WsWriteType>>>,
        ids: RoomChannelIds,
        mut request_id: u32,
    ) -> Result<(), DanmuStreamError> {
        loop {
            sleep(Duration::from_secs(HEARTBEAT_INTERVAL_SECS)).await;
            // The wup request id identifies each heartbeat; the web client
            // increments it per request.
            request_id = request_id.wrapping_add(1);
            let frame = encode_websocket_command(
                WS_CMD_WUP_REQ,
                &encode_heartbeat_wup(ids.topsid, ids.subsid, ids.yyuid, request_id),
            );
            let mut write = write.write().await;
            if let Some(write) = write.as_mut() {
                write
                    .send(WsMessage::binary(frame))
                    .await
                    .map_err(|e| DanmuStreamError::WebsocketError { err: e.to_string() })?;
            }
        }
    }

    async fn recv(
        mut read: WsReadType,
        tx: mpsc::UnboundedSender<DanmuMessageType>,
        room_id: String,
        stop: Arc<RwLock<bool>>,
    ) -> Result<(), DanmuStreamError> {
        let mut seen_danmu: Vec<(String, i64)> = Vec::new();
        while let Ok(Some(msg)) = read.try_next().await {
            if *stop.read().await {
                info!("Stopping Huya danmu stream");
                break;
            }

            let data = msg.into_data();
            if data.is_empty() {
                continue;
            }

            let command: WebSocketCommand = match decode_websocket_command(&data) {
                Ok(command) => command,
                Err(e) => {
                    warn!("Failed to decode Huya WebSocketCommand: {}", e);
                    continue;
                }
            };

            match command.cmd_type {
                WS_CMD_REGISTER_RSP => match decode_register_rsp(&command.data) {
                    Ok(Some(0)) => debug!("Huya danmaku registered, room_id: {}", room_id),
                    Ok(Some(code)) => {
                        // The server rejected the subscription (stale or
                        // invalid channel ids); fail the connection so the
                        // retry loop re-fetches ids and registers again
                        // instead of idling without danmaku.
                        warn!(
                            "Huya danmaku register rejected ({}), reconnecting, room_id: {}",
                            code, room_id
                        );
                        return Err(DanmuStreamError::WebsocketError {
                            err: format!("register rejected with code {code}"),
                        });
                    }
                    Ok(None) => warn!("Huya register response without code, room_id: {}", room_id),
                    Err(e) => warn!("Failed to decode Huya register response: {}", e),
                },
                WS_CMD_S2C_HEARTBEAT_ACK | WS_CMD_WUP_RSP => {}
                WS_CMD_S2C_MSG_PUSH_REQ => match decode_push_message(&command.data) {
                    Ok(Some(PushMessage { uri, msg })) => {
                        if uri == URI_MESSAGE_NOTICE {
                            if let Err(e) = Self::emit_danmu(&msg, &room_id, &tx, &mut seen_danmu) {
                                warn!("Failed to emit Huya danmu: {}", e);
                            }
                        } else {
                            debug!("Ignoring Huya push uri {}, room_id: {}", uri, room_id);
                        }
                    }
                    Ok(None) => warn!("Huya push message without uri, room_id: {}", room_id),
                    Err(e) => warn!("Failed to decode Huya push message: {}", e),
                },
                other => debug!("Ignoring Huya command type {}", other),
            }
        }

        Ok(())
    }

    fn emit_danmu(
        data: &[u8],
        room_id: &str,
        tx: &mpsc::UnboundedSender<DanmuMessageType>,
        seen_danmu: &mut Vec<(String, i64)>,
    ) -> Result<(), DanmuStreamError> {
        let Some(MessageNotice {
            user_id,
            nick_name,
            content,
            font_color,
        }) = decode_message_notice(data)?
        else {
            return Ok(());
        };
        if content.is_empty() {
            return Ok(());
        }

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let danmu = DanmuMessage {
            room_id: room_id.to_string(),
            user_id: u64::try_from(user_id).unwrap_or(0),
            user_name: nick_name,
            message: content,
            color: if font_color > 0 {
                font_color as u32
            } else {
                0xFFFFFF
            },
            timestamp,
        };
        if !is_new_danmu(
            seen_danmu,
            &format!("{}:{}", danmu.user_id, danmu.message),
            danmu.timestamp,
        ) {
            debug!("Dropping repeated Huya danmu, room_id: {}", room_id);
            return Ok(());
        }
        tx.send(DanmuMessageType::DanmuMessage(danmu))
            .map_err(|e| DanmuStreamError::WebsocketError { err: e.to_string() })
    }

    async fn fetch_channel_ids(&self) -> Result<RoomChannelIds, DanmuStreamError> {
        let url = format!("https://m.huya.com/{}", self.room_id);
        let response = self.client.get(&url).send().await?;
        let page = response.text().await?;
        parse_channel_ids(&page).ok_or_else(|| DanmuStreamError::MessageParseError {
            err: format!(
                "no live channel ids on m.huya.com/{} (room offline?)",
                self.room_id
            ),
        })
    }
}

/// Scrapes the danmaku channel ids from a `m.huya.com/{room}` page.
///
/// Reads the same `window.HNF_GLOBAL_INIT` blob the stream extractor uses
/// instead of scanning the whole page: recommendation blocks elsewhere in the
/// HTML carry other rooms' ids. Any zero id means the room has no live
/// channel to subscribe to (offline page).
fn parse_channel_ids(page: &str) -> Option<RoomChannelIds> {
    let init = extract_global_init(page)?;

    let (topsid, subsid) = stream_channel_ids(&init)
        .or_else(|| live_info_channel_ids(&init))
        .or_else(|| stream_name_channel_ids(&init))?;
    let yyuid = i64_at(&init, &["roomInfo", "tProfileInfo", "lUid"])
        .or_else(|| i64_at(&init, &["roomInfo", "tLiveInfo", "lUid"]))
        .or_else(|| i64_at(&init, &["roomProfile", "lUid"]))?;

    (topsid != 0 && subsid != 0 && yyuid != 0).then_some(RoomChannelIds {
        topsid,
        subsid,
        yyuid,
    })
}

const GLOBAL_INIT_VAR: &str = "window.HNF_GLOBAL_INIT";

/// Extracts and parses the `window.HNF_GLOBAL_INIT = {...}` assignment.
fn extract_global_init(page: &str) -> Option<Value> {
    let start = page.find(GLOBAL_INIT_VAR)? + GLOBAL_INIT_VAR.len();
    let rest = page[start..].trim_start().strip_prefix('=')?.trim_start();
    serde_json::from_str(balanced_json(rest)?).ok()
}

/// Returns the leading balanced `{..}` object, string and escape aware.
fn balanced_json(s: &str) -> Option<&str> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escape_next = false;
    for (i, ch) in s.char_indices() {
        if escape_next {
            escape_next = false;
        } else if ch == '\\' && in_string {
            escape_next = true;
        } else if ch == '"' {
            in_string = !in_string;
        } else if ch == '{' && !in_string {
            depth += 1;
        } else if ch == '}' && !in_string {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                return Some(&s[..i + ch.len_utf8()]);
            }
        }
    }
    None
}

fn i64_at(value: &Value, path: &[&str]) -> Option<i64> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_i64()
}

fn str_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_str()
}

/// First live stream entry carrying non-zero channel ids:
/// `roomInfo.tLiveInfo.tLiveStreamInfo.vStreamInfo.value[]`.
fn stream_channel_ids(init: &Value) -> Option<(i64, i64)> {
    let streams = init
        .get("roomInfo")?
        .get("tLiveInfo")?
        .get("tLiveStreamInfo")?
        .get("vStreamInfo")?
        .get("value")?
        .as_array()?;
    streams.iter().find_map(|stream| {
        let topsid = stream.get("lChannelId")?.as_i64()?;
        let subsid = stream.get("lSubChannelId")?.as_i64()?;
        (topsid != 0 && subsid != 0).then_some((topsid, subsid))
    })
}

/// Live-info level channel ids, present even when the per-stream list is
/// empty (the extractor's liveLineUrl fallback rooms).
fn live_info_channel_ids(init: &Value) -> Option<(i64, i64)> {
    let topsid = i64_at(init, &["roomInfo", "tLiveInfo", "lChannel"])?;
    let subsid = i64_at(init, &["roomInfo", "tLiveInfo", "lLiveChannel"])?;
    (topsid != 0 && subsid != 0).then_some((topsid, subsid))
}

/// Decodes `roomProfile.liveLineUrl` (a base64 HLS URL) and takes the stream
/// name prefix `{lChannelId}-{lSubChannelId}-...` as a last resort.
fn stream_name_channel_ids(init: &Value) -> Option<(i64, i64)> {
    let encoded = str_at(init, &["roomProfile", "liveLineUrl"])?;
    let decoded = general_purpose::STANDARD.decode(encoded).ok()?;
    let url = String::from_utf8(decoded).ok()?;
    let filename = url.split('?').next()?.rsplit('/').next()?;
    let filename = filename.trim_end_matches(".m3u8").trim_end_matches(".flv");
    let mut segments = filename.split('-');
    let topsid = segments.next()?.parse().ok()?;
    let subsid = segments.next()?.parse().ok()?;
    (topsid != 0 && subsid != 0).then_some((topsid, subsid))
}

fn random_guid() -> String {
    hex::encode(rand::rng().random::<[u8; 16]>())
}

/// Keeps the most recent danmu keys and reports whether `(key, ts)` is not a
/// repeat inside [`DEDUP_WINDOW_MS`].
fn is_new_danmu(seen: &mut Vec<(String, i64)>, key: &str, ts: i64) -> bool {
    if seen
        .iter()
        .any(|(seen_key, seen_ts)| seen_key == key && ts - seen_ts < DEDUP_WINDOW_MS)
    {
        return false;
    }
    seen.retain(|(seen_key, _)| seen_key != key);
    seen.push((key.to_string(), ts));
    if seen.len() > DEDUP_MAX_ENTRIES {
        seen.remove(0);
    }
    true
}

#[cfg(test)]
mod tests {
    use base64::engine::general_purpose::STANDARD as BASE64;
    use base64::Engine as _;

    use super::jce::{JceReader, JceWriter};
    use super::messages::{
        decode_message_notice, decode_push_message, decode_register_rsp, decode_websocket_command,
        encode_heartbeat_wup, encode_register, encode_websocket_command, WS_CMD_REGISTER_REQ,
    };
    use super::parse_channel_ids;

    #[test]
    fn websocket_command_round_trip() {
        let payload = [1u8, 2, 3, 4];
        let frame = encode_websocket_command(WS_CMD_REGISTER_REQ, &payload);
        let command = decode_websocket_command(&frame).unwrap();
        assert_eq!(command.cmd_type, WS_CMD_REGISTER_REQ);
        assert_eq!(command.data, payload);
    }

    #[test]
    fn register_rsp_res_code() {
        let mut writer = JceWriter::new();
        writer.write_i64(0, 0);
        writer.write_i64(1, 7);
        writer.write_str(2, "ok");
        let rsp = decode_register_rsp(&writer.into_bytes()).unwrap();
        assert_eq!(rsp, Some(0));
    }

    /// The heartbeat wup frame must carry servant/func and the request struct
    /// through the nested sBuffer map, exactly like the web client's wup.
    #[test]
    fn heartbeat_wup_decodes() {
        let frame = encode_heartbeat_wup(431653844, 431653844, 431653844, 2);

        // Strip the 4-byte packet length prefix, then decode the fields.
        let len = u32::from_be_bytes(frame[..4].try_into().unwrap()) as usize;
        assert_eq!(len, frame.len());
        let packet = &frame[4..];

        let mut reader = JceReader::new(packet);
        assert_eq!(reader.read_i64(1).unwrap(), Some(3)); // iVersion
        assert_eq!(reader.read_i64(4).unwrap(), Some(2)); // iRequestId
        assert_eq!(reader.read_string(5).unwrap().as_deref(), Some("onlineui"));
        assert_eq!(
            reader.read_string(6).unwrap().as_deref(),
            Some("OnUserHeartBeat")
        );
        let buffer = reader.read_bytes(7).unwrap().unwrap(); // sBuffer

        // sBuffer (v3): map { "tReq": UserHeartBeatReq }.
        let mut map_reader = JceReader::new(&buffer);
        assert_eq!(map_reader.enter_map(0).unwrap(), Some(1));
        let key = map_reader.read_string(0).unwrap().unwrap();
        assert_eq!(key, "tReq");
        let req_bytes = map_reader.read_bytes(1).unwrap().unwrap();

        // UserHeartBeatReq: tId (tag 0, UserId), lTid (1), lSid (2), lPid (4),
        // eLineType (6).
        let mut req_reader = JceReader::new(&req_bytes);
        assert!(req_reader.enter_struct(0).unwrap()); // tId
        let ua = req_reader.read_string(3).unwrap().unwrap();
        assert_eq!(ua, "webh5&1.0.0&websocket");
        req_reader.leave_struct().unwrap();
        assert_eq!(req_reader.read_i64(1).unwrap(), Some(431653844));
        assert_eq!(req_reader.read_i64(2).unwrap(), Some(431653844));
        assert_eq!(req_reader.read_i64(4).unwrap(), Some(431653844));
        assert_eq!(req_reader.read_i64(6).unwrap(), Some(1));
    }

    #[test]
    fn message_notice_decodes() {
        // Build the push chain: WebSocketCommand(MsgPushReq) > WSPushMessage
        // (uri 1400) > MessageNotice with sender + bullet color.
        let notice = {
            let mut writer = JceWriter::new();
            writer.write_struct(0, |sender| {
                sender.write_i64(0, 12345678);
                sender.write_str(2, " viewer ");
            });
            writer.write_str(3, "666");
            writer.write_struct(6, |format| {
                format.write_i64(0, 0xFF0000i64);
            });
            writer.into_bytes()
        };
        let push = {
            let mut writer = JceWriter::new();
            writer.write_i64(0, 0); // ePushType
            writer.write_i64(1, 1400); // iUri
            writer.write_bytes(2, &notice); // sMsg
            writer.into_bytes()
        };
        let frame = encode_websocket_command(7, &push);

        let command = decode_websocket_command(&frame).unwrap();
        assert_eq!(command.cmd_type, 7);
        let push = decode_push_message(&command.data).unwrap().unwrap();
        assert_eq!(push.uri, 1400);

        let notice = decode_message_notice(&push.msg).unwrap().unwrap();
        assert_eq!(notice.user_id, 12345678);
        assert_eq!(notice.nick_name, " viewer ");
        assert_eq!(notice.content, "666");
        assert_eq!(notice.font_color, 0xFF0000);
    }

    #[test]
    fn message_notice_without_optional_fields() {
        // Content only: both nested structs missing.
        let mut writer = JceWriter::new();
        writer.write_str(3, "hello");
        let notice = decode_message_notice(&writer.into_bytes())
            .unwrap()
            .unwrap();
        assert_eq!(notice.user_id, 0);
        assert_eq!(notice.nick_name, "");
        assert_eq!(notice.content, "hello");
        assert_eq!(notice.font_color, -1);
    }

    #[test]
    fn register_encodes_anonymous_flag() {
        let anonymous = encode_register(0, "a".repeat(32).as_str(), 1, 2);
        let mut reader = JceReader::new(&anonymous);
        assert_eq!(reader.read_i64(0).unwrap(), Some(0));
        assert_eq!(reader.read_bool(1).unwrap(), Some(true));
        assert_eq!(reader.read_i64(6).unwrap(), Some(0));
        assert_eq!(reader.read_i64(7).unwrap(), Some(3));

        let logged_in = encode_register(42, "a".repeat(32).as_str(), 1, 2);
        let mut reader = JceReader::new(&logged_in);
        assert_eq!(reader.read_i64(0).unwrap(), Some(42));
        assert_eq!(reader.read_bool(1).unwrap(), Some(false));
    }

    #[test]
    fn dedups_repeats_inside_the_window() {
        let mut seen = Vec::new();
        assert!(super::is_new_danmu(&mut seen, "42:hi", 1000));
        assert!(!super::is_new_danmu(&mut seen, "42:hi", 1500));
        assert!(super::is_new_danmu(&mut seen, "42:hi", 2100));
        assert!(super::is_new_danmu(&mut seen, "43:hi", 2100));
        assert_eq!(seen.len(), 2);
    }

    #[test]
    fn dedup_is_bounded() {
        let mut seen = Vec::new();
        for i in 0..(super::DEDUP_MAX_ENTRIES * 2) as i64 {
            super::is_new_danmu(&mut seen, &format!("u:{i}"), i);
        }
        assert_eq!(seen.len(), super::DEDUP_MAX_ENTRIES);
    }

    /// Realistic live-page shape: the ids live inside
    /// `window.HNF_GLOBAL_INIT`, and recommendation blocks elsewhere in the
    /// HTML carry other rooms' ids that must be ignored.
    #[test]
    fn parses_channel_ids_from_room_page() {
        let page = r#"
            <script>var recommend = {"lChannelId":999999,"lSubChannelId":999999,"lUid":42};</script>
            <script>
            window.HNF_GLOBAL_INIT = {"roomProfile":{"lUid":431653844},
            "roomInfo":{"eLiveStatus":2,"tProfileInfo":{"lUid":431653844},
            "tLiveInfo":{"lChannel":431653844,"lLiveChannel":431653844,
            "tLiveStreamInfo":{"vStreamInfo":{"value":[
            {"sCdnType":"AL","lChannelId":431653844,"lSubChannelId":431653844,
             "lPresenterUid":431653844,"sStreamName":"431653844-431653844-1"}]}}}}};
            </script>
        "#;
        let ids = parse_channel_ids(page).unwrap();
        assert_eq!(
            ids,
            super::RoomChannelIds {
                topsid: 431653844,
                subsid: 431653844,
                yyuid: 431653844,
            }
        );
    }

    #[test]
    fn channel_ids_missing_when_offline() {
        // The checked-in offline fixture shape: room profile uid exists, but
        // the live info carries no channel (all zeros) and no stream list.
        let page = r#"
            window.HNF_GLOBAL_INIT = {"roomProfile":{"lUid":431653844},
            "roomInfo":{"eLiveStatus":3,"tProfileInfo":{"lUid":431653844},
            "tLiveInfo":{"lChannel":0,"lLiveChannel":0,
            "tLiveStreamInfo":{"vStreamInfo":{"value":[]}}}}};
        "#;
        assert!(parse_channel_ids(page).is_none());
    }

    #[test]
    fn zero_ids_are_treated_as_offline() {
        // A stream list entry with a zero channel id must not be registered.
        let page = r#"
            window.HNF_GLOBAL_INIT = {"roomProfile":{"lUid":431653844},
            "roomInfo":{"tProfileInfo":{"lUid":431653844},"tLiveInfo":{
            "tLiveStreamInfo":{"vStreamInfo":{"value":[
            {"lChannelId":0,"lSubChannelId":431653844}]}}}}};
        "#;
        assert!(parse_channel_ids(page).is_none());

        // Same for a zero room uid on every uid path.
        let page = r#"
            window.HNF_GLOBAL_INIT = {"roomProfile":{"lUid":0},
            "roomInfo":{"tProfileInfo":{"lUid":0},"tLiveInfo":{"lUid":0,
            "lChannel":1,"lLiveChannel":2}}};
        "#;
        assert!(parse_channel_ids(page).is_none());
    }

    /// Rooms the extractor records through the live-info channel ids (empty
    /// per-stream list) must still resolve their ids.
    #[test]
    fn parses_ids_from_live_info_fallback() {
        let page = r#"
            window.HNF_GLOBAL_INIT = {"roomProfile":{"lUid":123},
            "roomInfo":{"eLiveStatus":2,"tProfileInfo":{"lUid":123},
            "tLiveInfo":{"lChannel":123,"lLiveChannel":123,
            "tLiveStreamInfo":{"vStreamInfo":{"value":[]}}}}};
        "#;
        let ids = parse_channel_ids(page).unwrap();
        assert_eq!(
            ids,
            super::RoomChannelIds {
                topsid: 123,
                subsid: 123,
                yyuid: 123,
            }
        );
    }

    /// Rooms the extractor records through the `roomProfile.liveLineUrl`
    /// fallback have neither stream entries nor live-info channel ids; the
    /// ids come from the stream name prefix
    /// `{lChannelId}-{lSubChannelId}-...`.
    #[test]
    fn parses_ids_from_live_line_url_fallback() {
        let hls_url = "//hs.hls.huya.com/huyalive/123-456-789-1011-10057-A-0-1.m3u8?ratio=2000";
        let live_line_url = BASE64.encode(hls_url);
        let page = format!(
            r#"
            window.HNF_GLOBAL_INIT = {{"roomProfile":{{"lUid":123,"liveLineUrl":"{live_line_url}"}},
            "roomInfo":{{"eLiveStatus":2,"tProfileInfo":{{"lUid":123}},
            "tLiveInfo":{{"tLiveStreamInfo":{{"vStreamInfo":{{"value":[]}}}}}}}}}};
        "#
        );
        let ids = parse_channel_ids(&page).unwrap();
        assert_eq!(
            ids,
            super::RoomChannelIds {
                topsid: 123,
                subsid: 456,
                yyuid: 123,
            }
        );
    }

    #[test]
    fn heartbeat_request_id_is_per_request() {
        let first = encode_heartbeat_wup(431653844, 431653844, 431653844, 1);
        let second = encode_heartbeat_wup(431653844, 431653844, 431653844, 2);
        assert_ne!(first, second);
        for (frame, expected) in [(first, 1), (second, 2)] {
            let packet = &frame[4..];
            let mut reader = JceReader::new(packet);
            assert_eq!(reader.read_i64(4).unwrap(), Some(expected)); // iRequestId
        }
    }
}
