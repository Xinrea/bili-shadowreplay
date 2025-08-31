mod messages;

use std::io::Read;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use deno_core::v8;
use deno_core::JsRuntime;
use deno_core::RuntimeOptions;
use flate2::read::GzDecoder;
use futures_util::{SinkExt, StreamExt, TryStreamExt};
use log::debug;
use log::{error, info};
use messages::*;
use prost::bytes::Bytes;
use prost::Message;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::sync::RwLock;
use tokio_tungstenite::{
    connect_async, tungstenite::Message as WsMessage, MaybeTlsStream, WebSocketStream,
};

use crate::{provider::DanmuProvider, DanmuMessage, DanmuMessageType, DanmuStreamError};

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);

type WsReadType = futures_util::stream::SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>;
type WsWriteType =
    futures_util::stream::SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, WsMessage>;

pub struct DouyinDanmu {
    room_id: u64,
    cookie: String,
    stop: Arc<RwLock<bool>>,
    write: Arc<RwLock<Option<WsWriteType>>>,
}

impl DouyinDanmu {
    async fn connect_and_handle(
        &self,
        tx: mpsc::UnboundedSender<DanmuMessageType>,
    ) -> Result<(), DanmuStreamError> {
        let url = self.get_wss_url().await?;

        let request = tokio_tungstenite::tungstenite::http::Request::builder()
            .uri(url)
            .header(
                tokio_tungstenite::tungstenite::http::header::COOKIE,
                self.cookie.as_str(),
            )
            .header(
                tokio_tungstenite::tungstenite::http::header::REFERER,
                "https://live.douyin.com/",
            )
            .header(
                tokio_tungstenite::tungstenite::http::header::USER_AGENT,
                USER_AGENT,
            )
            .header(
                tokio_tungstenite::tungstenite::http::header::HOST,
                "webcast5-ws-web-hl.douyin.com",
            )
            .header(
                tokio_tungstenite::tungstenite::http::header::UPGRADE,
                "websocket",
            )
            .header(
                tokio_tungstenite::tungstenite::http::header::CONNECTION,
                "Upgrade",
            )
            .header(
                tokio_tungstenite::tungstenite::http::header::SEC_WEBSOCKET_VERSION,
                "13",
            )
            .header(
                tokio_tungstenite::tungstenite::http::header::SEC_WEBSOCKET_EXTENSIONS,
                "permessage-deflate; client_max_window_bits",
            )
            .header(
                tokio_tungstenite::tungstenite::http::header::SEC_WEBSOCKET_KEY,
                "V1Yza5x1zcfkembl6u/0Pg==",
            )
            .body(())
            .unwrap();

        let (ws_stream, response) =
            connect_async(request)
                .await
                .map_err(|e| DanmuStreamError::WebsocketError {
                    err: format!("Failed to connect to douyin websocket: {}", e),
                })?;

        // Log the response status for debugging
        info!("WebSocket connection response: {:?}", response.status());

        let (write, read) = ws_stream.split();
        *self.write.write().await = Some(write);
        self.handle_connection(read, tx).await
    }

    async fn get_wss_url(&self) -> Result<String, DanmuStreamError> {
        // Create a new V8 runtime
        let mut runtime = JsRuntime::new(RuntimeOptions::default());

        // Add global CryptoJS object
        let crypto_js = include_str!("douyin/crypto-js.min.js");
        runtime
            .execute_script(
                "<crypto-js.min.js>",
                deno_core::FastString::from_static(crypto_js),
            )
            .map_err(|e| DanmuStreamError::WebsocketError {
                err: format!("Failed to execute crypto-js: {}", e),
            })?;

        // Load and execute the sign.js file
        let js_code = include_str!("douyin/webmssdk.js");
        runtime
            .execute_script("<sign.js>", deno_core::FastString::from_static(js_code))
            .map_err(|e| DanmuStreamError::WebsocketError {
                err: format!("Failed to execute JavaScript: {}", e),
            })?;

        // Call the get_wss_url function
        let sign_call = format!("get_wss_url(\"{}\")", self.room_id);
        let result = runtime
            .execute_script("<sign_call>", deno_core::FastString::from(sign_call))
            .map_err(|e| DanmuStreamError::WebsocketError {
                err: format!("Failed to execute JavaScript: {}", e),
            })?;

        // Get the result from the V8 runtime
        let scope = &mut runtime.handle_scope();
        let local = v8::Local::new(scope, result);
        let url = local.to_string(scope).unwrap().to_rust_string_lossy(scope);

        debug!("Douyin wss url: {}", url);

        Ok(url)
    }

    async fn handle_connection(
        &self,
        mut read: WsReadType,
        tx: mpsc::UnboundedSender<DanmuMessageType>,
    ) -> Result<(), DanmuStreamError> {
        // Start heartbeat task with error handling
        let (tx_write, mut _rx_write) = mpsc::channel(32);
        let tx_write_clone = tx_write.clone();
        let stop = Arc::clone(&self.stop);
        let heartbeat_handle = tokio::spawn(async move {
            let mut last_heartbeat = SystemTime::now();
            let mut consecutive_failures = 0;
            const MAX_FAILURES: u32 = 3;

            loop {
                if *stop.read().await {
                    log::info!("Stopping douyin danmu stream");
                    break;
                }

                tokio::time::sleep(HEARTBEAT_INTERVAL).await;

                match Self::send_heartbeat(&tx_write_clone).await {
                    Ok(_) => {
                        last_heartbeat = SystemTime::now();
                        consecutive_failures = 0;
                    }
                    Err(e) => {
                        error!("Failed to send heartbeat: {}", e);
                        consecutive_failures += 1;

                        if consecutive_failures >= MAX_FAILURES {
                            error!("Too many consecutive heartbeat failures, closing connection");
                            break;
                        }

                        // Check if we've exceeded the maximum time without a successful heartbeat
                        if let Ok(duration) = last_heartbeat.elapsed() {
                            if duration > HEARTBEAT_INTERVAL * 2 {
                                error!("No successful heartbeat for too long, closing connection");
                                break;
                            }
                        }
                    }
                }
            }
        });

        // Main message handling loop
        let room_id = self.room_id;
        let stop = Arc::clone(&self.stop);
        let write = Arc::clone(&self.write);
        let message_handle = tokio::spawn(async move {
            while let Some(msg) =
                read.try_next()
                    .await
                    .map_err(|e| DanmuStreamError::WebsocketError {
                        err: format!("Failed to read message: {}", e),
                    })?
            {
                if *stop.read().await {
                    log::info!("Stopping douyin danmu stream");
                    break;
                }

                match msg {
                    WsMessage::Binary(data) => {
                        if let Ok(Some(ack)) = handle_binary_message(&data, &tx, room_id).await {
                            if let Some(write) = write.write().await.as_mut() {
                                if let Err(e) =
                                    write.send(WsMessage::binary(ack.encode_to_vec())).await
                                {
                                    error!("Failed to send ack: {}", e);
                                }
                            }
                        }
                    }
                    WsMessage::Close(_) => {
                        info!("WebSocket connection closed");
                        break;
                    }
                    WsMessage::Ping(data) => {
                        // Respond to ping with pong
                        if let Err(e) = tx_write.send(WsMessage::Pong(data)).await {
                            error!("Failed to send pong: {}", e);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            Ok::<(), DanmuStreamError>(())
        });

        // Wait for either the heartbeat or message handling to complete
        tokio::select! {
            result = heartbeat_handle => {
                if let Err(e) = result {
                    error!("Heartbeat task failed: {}", e);
                }
            }
            result = message_handle => {
                if let Err(e) = result {
                    error!("Message handling task failed: {}", e);
                }
            }
        }

        Ok(())
    }

    async fn send_heartbeat(tx: &mpsc::Sender<WsMessage>) -> Result<(), DanmuStreamError> {
        // heartbeat message: 3A 02 68 62
        tx.send(WsMessage::binary(vec![0x3A, 0x02, 0x68, 0x62]))
            .await
            .map_err(|e| DanmuStreamError::WebsocketError {
                err: format!("Failed to send heartbeat message: {}", e),
            })?;
        Ok(())
    }
}

async fn handle_binary_message(
    data: &[u8],
    tx: &mpsc::UnboundedSender<DanmuMessageType>,
    room_id: u64,
) -> Result<Option<PushFrame>, DanmuStreamError> {
    // First decode the PushFrame
    let push_frame = PushFrame::decode(Bytes::from(data.to_vec())).map_err(|e| {
        DanmuStreamError::WebsocketError {
            err: format!("Failed to decode PushFrame: {}", e),
        }
    })?;

    // Decompress the payload
    let mut decoder = GzDecoder::new(push_frame.payload.as_slice());
    let mut decompressed = Vec::new();
    decoder
        .read_to_end(&mut decompressed)
        .map_err(|e| DanmuStreamError::WebsocketError {
            err: format!("Failed to decompress payload: {}", e),
        })?;

    // Decode the Response from decompressed payload
    let response = Response::decode(Bytes::from(decompressed)).map_err(|e| {
        DanmuStreamError::WebsocketError {
            err: format!("Failed to decode Response: {}", e),
        }
    })?;

    // if payload_package.needAck:
    // obj = PushFrame()
    // obj.payloadType = 'ack'
    // obj.logId = log_id
    // obj.payloadType = payload_package.internalExt
    // ack = obj.SerializeToString()
    let mut ack = None;
    if response.need_ack {
        let ack_msg = PushFrame {
            payload_type: "ack".to_string(),
            log_id: push_frame.log_id,
            payload_encoding: "".to_string(),
            payload: vec![],
            seq_id: 0,
            service: 0,
            method: 0,
            headers_list: vec![],
        };

        debug!("Need to respond ack: {:?}", ack_msg);

        ack = Some(ack_msg);
    }

    for message in response.messages_list {
        match message.method.as_str() {
            "WebcastChatMessage" => {
                let chat_msg =
                    DouyinChatMessage::decode(message.payload.as_slice()).map_err(|e| {
                        DanmuStreamError::WebsocketError {
                            err: format!("Failed to decode chat message: {}", e),
                        }
                    })?;
                if let Some(user) = chat_msg.user {
                    let danmu_msg = DanmuMessage {
                        room_id,
                        user_id: user.id,
                        user_name: user.nick_name,
                        message: chat_msg.content,
                        color: 0xffffff,
                        timestamp: chat_msg.event_time as i64 * 1000,
                    };
                    debug!("Received danmu message: {:?}", danmu_msg);
                    tx.send(DanmuMessageType::DanmuMessage(danmu_msg))
                        .map_err(|e| DanmuStreamError::WebsocketError {
                            err: format!("Failed to send message to channel: {}", e),
                        })?;
                }
            }
            "WebcastGiftMessage" => {
                let gift_msg = GiftMessage::decode(message.payload.as_slice()).map_err(|e| {
                    DanmuStreamError::WebsocketError {
                        err: format!("Failed to decode gift message: {}", e),
                    }
                })?;
                if let Some(user) = gift_msg.user {
                    if let Some(gift) = gift_msg.gift {
                        log::debug!("Received gift: {} from user: {}", gift.name, user.nick_name);
                    }
                }
            }
            "WebcastLikeMessage" => {
                let like_msg = LikeMessage::decode(message.payload.as_slice()).map_err(|e| {
                    DanmuStreamError::WebsocketError {
                        err: format!("Failed to decode like message: {}", e),
                    }
                })?;
                if let Some(user) = like_msg.user {
                    log::debug!(
                        "Received {} likes from user: {}",
                        like_msg.count,
                        user.nick_name
                    );
                }
            }
            "WebcastMemberMessage" => {
                let member_msg =
                    MemberMessage::decode(message.payload.as_slice()).map_err(|e| {
                        DanmuStreamError::WebsocketError {
                            err: format!("Failed to decode member message: {}", e),
                        }
                    })?;
                if let Some(user) = member_msg.user {
                    log::debug!(
                        "Member joined: {} (Action: {})",
                        user.nick_name,
                        member_msg.action_description
                    );
                }
            }
            _ => {
                debug!("Unknown message: {:?}", message);
            }
        }
    }

    Ok(ack)
}

#[async_trait]
impl DanmuProvider for DouyinDanmu {
    async fn new(identifier: &str, room_id: u64) -> Result<Self, DanmuStreamError> {
        Ok(Self {
            room_id,
            cookie: identifier.to_string(),
            stop: Arc::new(RwLock::new(false)),
            write: Arc::new(RwLock::new(None)),
        })
    }

    async fn start(
        &self,
        tx: mpsc::UnboundedSender<DanmuMessageType>,
    ) -> Result<(), DanmuStreamError> {
        let mut retry_count = 0;
        const MAX_RETRIES: u32 = 5;
        const RETRY_DELAY: Duration = Duration::from_secs(5);
        info!(
            "Douyin WebSocket connection started, room_id: {}",
            self.room_id
        );

        loop {
            if *self.stop.read().await {
                break;
            }

            match self.connect_and_handle(tx.clone()).await {
                Ok(_) => {
                    info!("Douyin WebSocket connection closed normally");
                    break;
                }
                Err(e) => {
                    error!("Douyin WebSocket connection error: {}", e);
                    retry_count += 1;

                    if retry_count >= MAX_RETRIES {
                        return Err(DanmuStreamError::WebsocketError {
                            err: format!("Failed to connect after {} retries", MAX_RETRIES),
                        });
                    }

                    info!(
                        "Retrying connection in {} seconds... (Attempt {}/{})",
                        RETRY_DELAY.as_secs(),
                        retry_count,
                        MAX_RETRIES
                    );
                    tokio::time::sleep(RETRY_DELAY).await;
                }
            }
        }

        Ok(())
    }

    async fn stop(&self) -> Result<(), DanmuStreamError> {
        *self.stop.write().await = true;
        if let Some(mut write) = self.write.write().await.take() {
            if let Err(e) = write.close().await {
                error!("Failed to close WebSocket connection: {}", e);
            }
        }
        Ok(())
    }
}
