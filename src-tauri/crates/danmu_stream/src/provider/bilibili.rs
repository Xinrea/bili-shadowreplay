mod dannmu_msg;
mod interact_word;
mod pack;
mod send_gift;
mod stream;
mod super_chat;

use std::{sync::Arc, time::SystemTime};

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt, TryStreamExt};
use log::{error, info};
use pct_str::{PctString, URIReserved};
use regex::Regex;
use serde::{Deserialize, Serialize};
use tokio::{
    sync::{mpsc, RwLock},
    time::{sleep, Duration},
};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::{
    http_client::ApiClient,
    provider::{DanmuMessageType, DanmuProvider},
    DanmuStreamError,
};

type WsReadType = futures_util::stream::SplitStream<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
>;

type WsWriteType = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    Message,
>;

pub struct BiliDanmu {
    client: ApiClient,
    room_id: u64,
    user_id: u64,
    stop: Arc<RwLock<bool>>,
    write: Arc<RwLock<Option<WsWriteType>>>,
}

#[async_trait]
impl DanmuProvider for BiliDanmu {
    async fn new(cookie: &str, room_id: u64) -> Result<Self, DanmuStreamError> {
        // find DedeUserID=<user_id> in cookie str
        let user_id = BiliDanmu::parse_user_id(cookie)?;
        // add buvid3 to cookie
        let cookie = format!("{};buvid3={}", cookie, uuid::Uuid::new_v4());
        let client = ApiClient::new(&cookie);

        Ok(Self {
            client,
            user_id,
            room_id,
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
            "Bilibili WebSocket connection started, room_id: {}",
            self.room_id
        );

        loop {
            if *self.stop.read().await {
                info!(
                    "Bilibili WebSocket connection stopped, room_id: {}",
                    self.room_id
                );
                break;
            }

            match self.connect_and_handle(tx.clone()).await {
                Ok(_) => {
                    info!(
                        "Bilibili WebSocket connection closed normally, room_id: {}",
                        self.room_id
                    );
                    retry_count = 0;
                }
                Err(e) => {
                    error!(
                        "Bilibili WebSocket connection error, room_id: {}, error: {}",
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
            tokio::time::sleep(RETRY_DELAY).await;
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

impl BiliDanmu {
    async fn connect_and_handle(
        &self,
        tx: mpsc::UnboundedSender<DanmuMessageType>,
    ) -> Result<(), DanmuStreamError> {
        let wbi_key = self.get_wbi_key().await?;
        let real_room = self.get_real_room(&wbi_key, self.room_id).await?;
        let danmu_info = self.get_danmu_info(&wbi_key, real_room).await?;
        let ws_hosts = danmu_info.data.host_list.clone();
        let mut conn = None;
        log::debug!("ws_hosts: {:?}", ws_hosts);
        // try to connect to ws_hsots, once success, send the token to the tx
        for i in ws_hosts {
            let host = format!("wss://{}/sub", i.host);
            match connect_async(&host).await {
                Ok((c, _)) => {
                    conn = Some(c);
                    break;
                }
                Err(e) => {
                    eprintln!(
                        "Connect ws host: {} has error, trying next host ...\n{:?}\n{:?}",
                        host, i, e
                    );
                }
            }
        }

        let conn = conn.ok_or(DanmuStreamError::WebsocketError {
            err: "Failed to connect to ws host".into(),
        })?;

        let (write, read) = conn.split();
        *self.write.write().await = Some(write);

        let json = serde_json::to_string(&WsSend {
            roomid: real_room,
            key: danmu_info.data.token,
            uid: self.user_id,
            protover: 3,
            platform: "web".to_string(),
            t: 2,
        })
        .map_err(|e| DanmuStreamError::WebsocketError { err: e.to_string() })?;

        let json = pack::encode(&json, 7);
        if let Some(write) = self.write.write().await.as_mut() {
            write
                .send(Message::binary(json))
                .await
                .map_err(|e| DanmuStreamError::WebsocketError { err: e.to_string() })?;
        }

        tokio::select! {
            v = BiliDanmu::send_heartbeat_packets(Arc::clone(&self.write)) => v,
            v = BiliDanmu::recv(read, tx, Arc::clone(&self.stop)) => v
        }?;

        Ok(())
    }

    async fn send_heartbeat_packets(
        write: Arc<RwLock<Option<WsWriteType>>>,
    ) -> Result<(), DanmuStreamError> {
        loop {
            if let Some(write) = write.write().await.as_mut() {
                write
                    .send(Message::binary(pack::encode("", 2)))
                    .await
                    .map_err(|e| DanmuStreamError::WebsocketError { err: e.to_string() })?;
            }
            sleep(Duration::from_secs(30)).await;
        }
    }

    async fn recv(
        mut read: WsReadType,
        tx: mpsc::UnboundedSender<DanmuMessageType>,
        stop: Arc<RwLock<bool>>,
    ) -> Result<(), DanmuStreamError> {
        while let Ok(Some(msg)) = read.try_next().await {
            if *stop.read().await {
                log::info!("Stopping bilibili danmu stream");
                break;
            }
            let data = msg.into_data();

            if !data.is_empty() {
                let s = pack::build_pack(&data);

                if let Ok(msgs) = s {
                    for i in msgs {
                        let ws = stream::WsStreamCtx::new(&i);
                        if let Ok(ws) = ws {
                            match ws.match_msg() {
                                Ok(v) => {
                                    log::debug!("Received message: {:?}", v);
                                    tx.send(v).map_err(|e| DanmuStreamError::WebsocketError {
                                        err: e.to_string(),
                                    })?;
                                }
                                Err(e) => {
                                    log::trace!(
                                        "This message parsing is not yet supported:\nMessage: {i}\nErr: {e:#?}"
                                    );
                                }
                            }
                        } else {
                            log::error!("{}", ws.unwrap_err());
                        }
                    }
                }
            }
        }

        Ok(())
    }

    async fn get_danmu_info(
        &self,
        wbi_key: &str,
        room_id: u64,
    ) -> Result<DanmuInfo, DanmuStreamError> {
        let params = self
            .get_sign(
                wbi_key,
                serde_json::json!({
                    "id": room_id,
                    "type": 0,
                }),
            )
            .await?;
        let resp = self
            .client
            .get(
                &format!(
                    "https://api.live.bilibili.com/xlive/web-room/v1/index/getDanmuInfo?{}",
                    params
                ),
                None,
            )
            .await?
            .json::<DanmuInfo>()
            .await?;

        Ok(resp)
    }

    async fn get_real_room(&self, wbi_key: &str, room_id: u64) -> Result<u64, DanmuStreamError> {
        let params = self
            .get_sign(
                wbi_key,
                serde_json::json!({
                    "id": room_id,
                    "from": "room",
                }),
            )
            .await?;
        let resp = self
            .client
            .get(
                &format!(
                    "https://api.live.bilibili.com/room/v1/Room/room_init?{}",
                    params
                ),
                None,
            )
            .await?
            .json::<RoomInit>()
            .await?
            .data
            .room_id;

        Ok(resp)
    }

    fn parse_user_id(cookie: &str) -> Result<u64, DanmuStreamError> {
        let mut user_id = None;

        // find DedeUserID=<user_id> in cookie str
        let re = Regex::new(r"DedeUserID=(\d+)").unwrap();
        if let Some(captures) = re.captures(cookie) {
            if let Some(user) = captures.get(1) {
                user_id = Some(user.as_str().parse::<u64>().unwrap());
            }
        }

        if let Some(user_id) = user_id {
            Ok(user_id)
        } else {
            Err(DanmuStreamError::InvalidIdentifier {
                err: format!("Failed to find user_id in cookie: {cookie}"),
            })
        }
    }

    async fn get_wbi_key(&self) -> Result<String, DanmuStreamError> {
        let nav_info: serde_json::Value = self
            .client
            .get("https://api.bilibili.com/x/web-interface/nav", None)
            .await?
            .json()
            .await?;
        let re = Regex::new(r"wbi/(.*).png").unwrap();
        let img = re
            .captures(nav_info["data"]["wbi_img"]["img_url"].as_str().unwrap())
            .unwrap()
            .get(1)
            .unwrap()
            .as_str();
        let sub = re
            .captures(nav_info["data"]["wbi_img"]["sub_url"].as_str().unwrap())
            .unwrap()
            .get(1)
            .unwrap()
            .as_str();
        let raw_string = format!("{}{}", img, sub);
        Ok(raw_string)
    }

    pub async fn get_sign(
        &self,
        wbi_key: &str,
        mut parameters: serde_json::Value,
    ) -> Result<String, DanmuStreamError> {
        let table = vec![
            46, 47, 18, 2, 53, 8, 23, 32, 15, 50, 10, 31, 58, 3, 45, 35, 27, 43, 5, 49, 33, 9, 42,
            19, 29, 28, 14, 39, 12, 38, 41, 13, 37, 48, 7, 16, 24, 55, 40, 61, 26, 17, 0, 1, 60,
            51, 30, 4, 22, 25, 54, 21, 56, 59, 6, 63, 57, 62, 11, 36, 20, 34, 44, 52,
        ];
        let raw_string = wbi_key;
        let mut encoded = Vec::new();
        table.into_iter().for_each(|x| {
            if x < raw_string.len() {
                encoded.push(raw_string.as_bytes()[x]);
            }
        });
        // only keep 32 bytes of encoded
        encoded = encoded[0..32].to_vec();
        let encoded = String::from_utf8(encoded).unwrap();
        // Timestamp in seconds
        let wts = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        parameters
            .as_object_mut()
            .unwrap()
            .insert("wts".to_owned(), serde_json::Value::String(wts.to_string()));
        // Get all keys from parameters into vec
        let mut keys = parameters
            .as_object()
            .unwrap()
            .keys()
            .map(|x| x.to_owned())
            .collect::<Vec<String>>();
        // sort keys
        keys.sort();
        let mut params = String::new();
        keys.iter().for_each(|x| {
            params.push_str(x);
            params.push('=');
            // Convert value to string based on its type
            let value = match parameters.get(x).unwrap() {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::Bool(b) => b.to_string(),
                _ => "".to_string(),
            };
            // Value filters !'()* characters
            let value = value.replace(['!', '\'', '(', ')', '*'], "");
            let value = PctString::encode(value.chars(), URIReserved);
            params.push_str(value.as_str());
            // add & if not last
            if x != keys.last().unwrap() {
                params.push('&');
            }
        });
        // md5 params+encoded
        let w_rid = md5::compute(params.to_string() + encoded.as_str());
        let params = params + format!("&w_rid={:x}", w_rid).as_str();
        Ok(params)
    }
}

#[derive(Serialize)]
struct WsSend {
    uid: u64,
    roomid: u64,
    key: String,
    protover: u32,
    platform: String,
    #[serde(rename = "type")]
    t: u32,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DanmuInfo {
    pub data: DanmuInfoData,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DanmuInfoData {
    pub token: String,
    pub host_list: Vec<WsHost>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct WsHost {
    pub host: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RoomInit {
    data: RoomInitData,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RoomInitData {
    room_id: u64,
}
