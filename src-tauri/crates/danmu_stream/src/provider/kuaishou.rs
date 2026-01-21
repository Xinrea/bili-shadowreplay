mod messages;

use std::{
    io::Read,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use flate2::read::GzDecoder;
use futures_util::{SinkExt, StreamExt, TryStreamExt};
use log::{error, info, warn};
use rand::{distr::Alphanumeric, Rng};
use regex::Regex;
use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::Value;
use prost::Message;
use tokio::{
    sync::{mpsc, RwLock},
    time::sleep,
};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};

use crate::{
    provider::{DanmuMessageType, DanmuProvider},
    DanmuMessage, DanmuStreamError,
};

use messages::{
    CompressionType, CsHeartbeat, CsWebEnterRoom, PayloadType, ScWebFeedPush, SocketMessage,
};

type WsReadType = futures_util::stream::SplitStream<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
>;

type WsWriteType = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    WsMessage,
>;

const KUAISHOU_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
const HEARTBEAT_INTERVAL_SECS: u64 = 20;

#[derive(Clone)]
struct KuaishouRoomInit {
    token: String,
    live_stream_id: String,
    websocket_urls: Vec<String>,
}

pub struct KuaishouDanmu {
    client: reqwest::Client,
    room_id: String,
    cookie: String,
    kww: Option<String>,
    stop: Arc<RwLock<bool>>,
    write: Arc<RwLock<Option<WsWriteType>>>,
}

#[async_trait]
impl DanmuProvider for KuaishouDanmu {
    async fn new(cookie: &str, room_id: &str) -> Result<Self, DanmuStreamError> {
        let mut headers = HeaderMap::new();
        headers.insert(
            "User-Agent",
            HeaderValue::from_static(KUAISHOU_USER_AGENT),
        );
        if !cookie.trim().is_empty() {
            if let Ok(value) = HeaderValue::from_str(cookie) {
                headers.insert("Cookie", value);
            }
        }

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .build()?;

        Ok(Self {
            client,
            room_id: room_id.to_string(),
            cookie: cookie.to_string(),
            kww: extract_kww(cookie),
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
            "Kuaishou WebSocket connection started, room_id: {}",
            self.room_id
        );

        loop {
            if *self.stop.read().await {
                info!(
                    "Kuaishou WebSocket connection stopped, room_id: {}",
                    self.room_id
                );
                break;
            }

            match self.connect_and_handle(tx.clone()).await {
                Ok(_) => {
                    info!(
                        "Kuaishou WebSocket connection closed normally, room_id: {}",
                        self.room_id
                    );
                    retry_count = 0;
                }
                Err(e) => {
                    error!(
                        "Kuaishou WebSocket connection error, room_id: {}, error: {}",
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
                error!("Failed to close Kuaishou WebSocket connection: {}", e);
            }
        }
        Ok(())
    }
}

impl KuaishouDanmu {
    async fn connect_and_handle(
        &self,
        tx: mpsc::UnboundedSender<DanmuMessageType>,
    ) -> Result<(), DanmuStreamError> {
        let room_init = self.room_init().await?;
        let ws_url = room_init
            .websocket_urls
            .first()
            .ok_or(DanmuStreamError::WebsocketError {
                err: "No websocket URL available".to_string(),
            })?
            .to_string();

        let (conn, _) = connect_async(&ws_url).await.map_err(|e| {
            DanmuStreamError::WebsocketError {
                err: e.to_string(),
            }
        })?;

        let (write, read) = conn.split();
        *self.write.write().await = Some(write);

        self.send_enter_room(&room_init).await?;

        tokio::select! {
            v = Self::send_heartbeat_packets(Arc::clone(&self.write)) => v,
            v = Self::recv(read, tx, self.room_id.clone(), Arc::clone(&self.stop)) => v
        }?;

        Ok(())
    }

    async fn send_enter_room(&self, room_init: &KuaishouRoomInit) -> Result<(), DanmuStreamError> {
        let page_id = format!(
            "{}{}",
            rand::rng()
                .sample_iter(&Alphanumeric)
                .take(16)
                .map(char::from)
                .collect::<String>(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );

        let payload = CsWebEnterRoom {
            token: room_init.token.clone(),
            live_stream_id: room_init.live_stream_id.clone(),
            reconnect_count: 0,
            last_error_code: 0,
            exp_tag: String::new(),
            attach: String::new(),
            page_id,
        }
        .encode_to_vec();

        let msg = SocketMessage {
            payload_type: PayloadType::CsEnterRoom as i32,
            compression_type: CompressionType::None as i32,
            payload,
        };

        if let Some(write) = self.write.write().await.as_mut() {
            write
                .send(WsMessage::binary(msg.encode_to_vec()))
                .await
                .map_err(|e| DanmuStreamError::WebsocketError { err: e.to_string() })?;
        }

        Ok(())
    }

    async fn send_heartbeat_packets(
        write: Arc<RwLock<Option<WsWriteType>>>,
    ) -> Result<(), DanmuStreamError> {
        loop {
            let payload = CsHeartbeat {
                timestamp: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
            }
            .encode_to_vec();
            let msg = SocketMessage {
                payload_type: PayloadType::CsHeartbeat as i32,
                compression_type: CompressionType::None as i32,
                payload,
            };

            if let Some(write) = write.write().await.as_mut() {
                write
                .send(WsMessage::binary(msg.encode_to_vec()))
                    .await
                    .map_err(|e| DanmuStreamError::WebsocketError { err: e.to_string() })?;
            }
            sleep(Duration::from_secs(HEARTBEAT_INTERVAL_SECS)).await;
        }
    }

    async fn recv(
        mut read: WsReadType,
        tx: mpsc::UnboundedSender<DanmuMessageType>,
        room_id: String,
        stop: Arc<RwLock<bool>>,
    ) -> Result<(), DanmuStreamError> {
        while let Ok(Some(msg)) = read.try_next().await {
            if *stop.read().await {
                info!("Stopping Kuaishou danmu stream");
                break;
            }

            let data = msg.into_data();
            if data.is_empty() {
                continue;
            }

            let socket_msg = SocketMessage::decode(&*data).map_err(|e| {
                DanmuStreamError::MessageParseError {
                    err: e.to_string(),
                }
            })?;

            let payload = match CompressionType::try_from(socket_msg.compression_type).ok() {
                Some(CompressionType::None) | Some(CompressionType::Unknown) => socket_msg.payload,
                Some(CompressionType::Gzip) => gunzip(&socket_msg.payload)?,
                Some(CompressionType::Aes) => {
                    warn!("Kuaishou payload uses AES compression, skipping");
                    continue;
                }
                None => socket_msg.payload,
            };

            if PayloadType::try_from(socket_msg.payload_type).ok() == Some(PayloadType::ScFeedPush)
            {
                let feed = ScWebFeedPush::decode(&*payload).map_err(|e| {
                    DanmuStreamError::MessageParseError {
                        err: e.to_string(),
                    }
                })?;
                for comment in feed.comment_feeds {
                    let user = comment.user.unwrap_or_default();
                    let user_id = user.principal_id.parse::<u64>().unwrap_or(0);
                    let color = parse_color(&comment.color);
                    let ts = if comment.time > 0 {
                        comment.time as i64
                    } else {
                        SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as i64
                    };
                    let danmu = DanmuMessage {
                        room_id: room_id.clone(),
                        user_id,
                        user_name: user.user_name,
                        message: comment.content,
                        color,
                        timestamp: ts,
                    };
                    tx.send(DanmuMessageType::DanmuMessage(danmu))
                        .map_err(|e| DanmuStreamError::WebsocketError {
                            err: e.to_string(),
                        })?;
                }
            }
        }

        Ok(())
    }

    async fn room_init(&self) -> Result<KuaishouRoomInit, DanmuStreamError> {
        let referer = format!("https://live.kuaishou.com/u/{}", self.room_id);
        let resp = self
            .client
            .get("https://live.kuaishou.com/live_api/liveroom/livedetail")
            .query(&[("principalId", self.room_id.as_str())])
            .header("Referer", referer.clone())
            .send()
            .await?;

        let text = resp.text().await?;
        let data = parse_response_data(&text)?;

        let live_stream_id = data
            .get("liveStream")
            .and_then(|v| v.get("id"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();

        if live_stream_id.is_empty() {
            return Err(DanmuStreamError::MessageParseError {
                err: "Kuaishou liveStreamId missing (not live?)".to_string(),
            });
        }

        let mut token = data
            .get("websocketInfo")
            .and_then(|v| v.get("token"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();

        let mut websocket_urls = extract_websocket_urls(&data);

        if !token.is_empty() && !websocket_urls.is_empty() {
            return Ok(KuaishouRoomInit {
                token,
                live_stream_id,
                websocket_urls,
            });
        }

        if !self.cookie.is_empty() {
        let ws_info = self
                .client
                .get("https://live.kuaishou.com/live_api/liveroom/websocketinfo")
                .query(&[("caver", "2"), ("liveStreamId", live_stream_id.as_str())])
                .header("Referer", referer)
                .apply_header("Kww", self.kww.as_deref())
                .send()
                .await?
                .text()
                .await?;

            let ws_data = parse_response_data(&ws_info)?;
            token = ws_data
                .get("token")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            websocket_urls = extract_websocket_urls(&ws_data);
        }

        if token.is_empty() || websocket_urls.is_empty() {
            return Err(DanmuStreamError::MessageParseError {
                err: "Kuaishou websocket token or URL missing".to_string(),
            });
        }

        Ok(KuaishouRoomInit {
            token,
            live_stream_id,
            websocket_urls,
        })
    }
}

fn extract_kww(cookie: &str) -> Option<String> {
    if cookie.trim().is_empty() {
        return None;
    }
    let re = Regex::new(r"(?i)(?:kww|kwfv1)=([^;]+)").ok()?;
    re.captures(cookie)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_string())
}

fn parse_response_data(text: &str) -> Result<Value, DanmuStreamError> {
    let root: Value = serde_json::from_str(text).map_err(|e| DanmuStreamError::MessageParseError {
        err: e.to_string(),
    })?;

    let data = root.get("data").cloned().unwrap_or(root);

    if let Some(result) = data.get("result").and_then(|v| v.as_i64()) {
        if result != 1 && result != 671 && result != 677 {
            return Err(DanmuStreamError::MessageParseError {
                err: format!("Kuaishou API error: {result}"),
            });
        }
        if result == 671 || result == 677 {
            return Err(DanmuStreamError::MessageParseError {
                err: format!("Kuaishou room is not live: {result}"),
            });
        }
    }

    Ok(data)
}

fn extract_websocket_urls(data: &Value) -> Vec<String> {
    let mut urls = Vec::new();
    let ws_info = data.get("websocketInfo").unwrap_or(data);
    if let Some(list) = ws_info.get("webSocketAddresses").and_then(|v| v.as_array()) {
        for item in list {
            if let Some(url) = item.as_str() {
                urls.push(url.to_string());
            }
        }
    }
    if urls.is_empty() {
        if let Some(list) = ws_info.get("websocketUrls").and_then(|v| v.as_array()) {
            for item in list {
                if let Some(url) = item.as_str() {
                    urls.push(url.to_string());
                }
            }
        }
    }
    urls
}

fn gunzip(data: &[u8]) -> Result<Vec<u8>, DanmuStreamError> {
    let mut decoder = GzDecoder::new(data);
    let mut out = Vec::new();
    decoder
        .read_to_end(&mut out)
        .map_err(|e| DanmuStreamError::MessageParseError {
            err: e.to_string(),
        })?;
    Ok(out)
}

fn parse_color(color: &str) -> u32 {
    let trimmed = color.trim();
    if trimmed.is_empty() {
        return 0;
    }
    let hex = trimmed.trim_start_matches('#');
    u32::from_str_radix(hex, 16).unwrap_or(0)
}

trait HeaderApply {
    fn apply_header(self, name: &str, value: Option<&str>) -> Self;
}

impl HeaderApply for reqwest::RequestBuilder {
    fn apply_header(self, name: &str, value: Option<&str>) -> Self {
        if let Some(value) = value {
            if !value.is_empty() {
                return self.header(name, value);
            }
        }
        self
    }
}
