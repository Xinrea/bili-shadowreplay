use serde::Deserialize;
use serde_json::Value;

use super::dannmu_msg::BiliDanmuMessage;

use crate::{provider::DanmuMessageType, DanmuStreamError, LiveEvent};
use chrono::Utc;
use serde_json::json;

#[derive(Debug, Deserialize, Clone)]
pub struct WsStreamCtx {
    pub cmd: Option<String>,
    pub info: Option<Vec<Value>>,
    pub data: Option<WsStreamCtxData>,
    #[serde(flatten)]
    _v: Value,
    #[serde(skip)]
    pub raw: Value,
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
    #[serde(default)]
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
    #[serde(default)]
    pub face: String,
    #[serde(default)]
    pub uname: String,
}

impl WsStreamCtx {
    pub fn new(s: &str) -> Result<Self, DanmuStreamError> {
        let raw =
            serde_json::from_str::<Value>(s).map_err(|_| DanmuStreamError::MessageParseError {
                err: "Failed to parse message".to_string(),
            })?;
        let mut ctx = serde_json::from_value::<Self>(raw.clone()).map_err(|_| {
            DanmuStreamError::MessageParseError {
                err: "Failed to parse message".to_string(),
            }
        })?;
        ctx.raw = raw;
        Ok(ctx)
    }

    pub fn match_msg(&self) -> Result<DanmuMessageType, DanmuStreamError> {
        let cmd = self.handle_cmd();
        let event_type = match cmd {
            Some(c) if c.contains("DANMU_MSG") => "danmu",
            Some("SEND_GIFT") | Some("COMBO_SEND") => "gift",
            Some("SUPER_CHAT_MESSAGE") => "super_chat",
            Some("INTERACT_WORD") | Some("ENTRY_EFFECT") => "enter",
            Some("LIKE_INFO_V3_CLICK") | Some("LIKE_INFO_V3_UPDATE") => "like",
            Some("WATCHED_CHANGE") | Some("ONLINE") => "online",
            Some("ROOM_REAL_TIME_MESSAGE_UPDATE") => "room_update",
            Some(_) => "unknown",
            None => "unknown",
        };

        let data = match event_type {
            "danmu" => BiliDanmuMessage::new_from_ctx(self)
                .map(|message| {
                    json!({
                        "user_id": message.uid,
                        "user_name": message.username,
                        "content": message.msg,
                        "color": 0,
                        "fan": message.fan,
                        "fan_level": message.fan_level,
                    })
                })
                .unwrap_or_else(|_| self.raw.get("data").cloned().unwrap_or(Value::Null)),
            "gift" => super::send_gift::SendGift::new_from_ctx(self)
                .map(|gift| {
                    json!({
                        "user_id": gift.uid,
                        "user_name": gift.uname,
                        "gift_name": gift.gift_name,
                        "count": gift.num,
                        "action": gift.action,
                        "price": gift.price,
                        "medal_name": gift.medal_name,
                        "medal_level": gift.medal_level,
                    })
                })
                .unwrap_or_else(|_| self.raw.get("data").cloned().unwrap_or(Value::Null)),
            "super_chat" => super::super_chat::SuperChatMessage::new_from_ctx(self)
                .map(|sc| {
                    json!({
                        "user_id": sc.uid,
                        "user_name": sc.uname,
                        "content": sc.msg,
                        "price": sc.price,
                        "duration": sc.time,
                        "medal_name": sc.medal_name,
                        "medal_level": sc.medal_level,
                    })
                })
                .unwrap_or_else(|_| self.raw.get("data").cloned().unwrap_or(Value::Null)),
            "enter" => super::interact_word::InteractWord::new_from_ctx(self)
                .map(|enter| {
                    json!({
                        "user_id": enter.uid,
                        "user_name": enter.uname,
                        "medal_name": enter.fan,
                        "medal_level": enter.fan_level,
                    })
                })
                .unwrap_or_else(|_| self.raw.get("data").cloned().unwrap_or(Value::Null)),
            _ => self.raw.get("data").cloned().unwrap_or(Value::Null),
        };

        Ok(DanmuMessageType::Event(LiveEvent {
            ts: Utc::now().timestamp_millis(),
            platform: "bilibili".to_string(),
            room_id: String::new(),
            event_type: event_type.to_string(),
            data,
            raw: self.raw.clone(),
        }))
    }

    fn handle_cmd(&self) -> Option<&str> {
        // handle DANMU_MSG:4:0:2:2:2:0
        let cmd = if let Some(c) = self.cmd.as_deref() {
            if c.starts_with("DM_INTERACTION") {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_unknown_bilibili_events() {
        let ctx = WsStreamCtx::new(r#"{"cmd":"WATCHED_CHANGE","data":{"num":7}}"#).unwrap();
        let DanmuMessageType::Event(event) = ctx.match_msg().unwrap() else {
            panic!("expected normalized event");
        };

        assert_eq!(event.event_type, "online");
        assert_eq!(event.data["num"], 7);
        assert_eq!(event.raw["cmd"], "WATCHED_CHANGE");
    }

    #[test]
    fn normalizes_gifts_without_dropping_the_raw_payload() {
        let ctx = WsStreamCtx::new(
            r#"{"cmd":"SEND_GIFT","data":{"action":"赠送","giftName":"花束","num":2,"uname":"viewer","uid":42,"price":100}}"#,
        )
        .unwrap();
        let DanmuMessageType::Event(event) = ctx.match_msg().unwrap() else {
            panic!("expected normalized event");
        };

        assert_eq!(event.event_type, "gift");
        assert_eq!(event.data["gift_name"], "花束");
        assert_eq!(event.data["count"], 2);
        assert_eq!(event.raw["data"]["uid"], 42);
    }
}
