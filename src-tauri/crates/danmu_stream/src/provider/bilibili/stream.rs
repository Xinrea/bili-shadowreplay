use serde::Deserialize;
use serde_json::Value;

use crate::{
    provider::{bilibili::dannmu_msg::BiliDanmuMessage, DanmuMessageType},
    DanmuStreamError, DanmuMessage,
};

#[derive(Debug, Deserialize, Clone)]
pub struct WsStreamCtx {
    pub cmd: Option<String>,
    pub info: Option<Vec<Value>>,
    pub data: Option<WsStreamCtxData>,
    #[serde(flatten)]
    _v: Value,
}

#[derive(Debug, Deserialize, Clone)]
#[allow(dead_code)]
pub struct WsStreamCtxData {
    pub message: Option<String>,
    pub price: Option<u32>,
    pub start_time: Option<u64>,
    pub time: Option<u32>,
    pub uid: Option<Value>,
    pub user_info: Option<WsStreamCtxDataUser>,
    pub medal_info: Option<WsStreamCtxDataMedalInfo>,
    pub uname: Option<String>,
    pub fans_medal: Option<WsStreamCtxDataMedalInfo>,
    pub action: Option<String>,
    #[serde(rename = "giftName")]
    pub gift_name: Option<String>,
    pub num: Option<u64>,
    pub combo_num: Option<u64>,
    pub gift_num: Option<u64>,
    pub combo_send: Box<Option<WsStreamCtxData>>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct WsStreamCtxDataMedalInfo {
    pub medal_name: Option<String>,
    pub medal_level: Option<u32>,
}

#[derive(Debug, Deserialize, Clone)]
#[allow(dead_code)]
pub struct WsStreamCtxDataUser {
    pub face: String,
    pub uname: String,
}

impl WsStreamCtx {
    pub fn new(s: &str) -> Result<Self, DanmuStreamError> {
        serde_json::from_str(s).map_err(|_| DanmuStreamError::MessageParseError {
            err: "Failed to parse message".to_string(),
        })
    }

    pub fn match_msg(&self) -> Result<DanmuMessageType, DanmuStreamError> {
        let cmd = self.handle_cmd();

        let danmu_msg = match cmd {
            Some(c) if c.contains("DANMU_MSG") => Some(BiliDanmuMessage::new_from_ctx(self)?),
            _ => None,
        };

        if let Some(danmu_msg) = danmu_msg {
            Ok(DanmuMessageType::DanmuMessage(DanmuMessage {
                room_id: 0,
                user_id: danmu_msg.uid,
                user_name: danmu_msg.username,
                message: danmu_msg.msg,
                color: 0,
                timestamp: danmu_msg.timestamp,
            }))
        } else {
            Err(DanmuStreamError::MessageParseError {
                err: "Unknown message".to_string(),
            })
        }
    }

    fn handle_cmd(&self) -> Option<&str> {
        // handle DANMU_MSG:4:0:2:2:2:0
        let cmd = if let Some(c) = self.cmd.as_deref() {
            if c.starts_with("DANMU_MSG") {
                Some("DANMU_MSG")
            } else {
                Some(c)
            }
        } else {
            None
        };

        cmd
    }
}
