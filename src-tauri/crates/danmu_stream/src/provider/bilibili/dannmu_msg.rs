use serde::Deserialize;

use crate::{provider::bilibili::stream::WsStreamCtx, DanmmuStreamError};

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct BiliDanmuMessage {
    pub uid: u64,
    pub username: String,
    pub msg: String,
    pub fan: Option<String>,
    pub fan_level: Option<u64>,
    pub timestamp: i64,
}

impl BiliDanmuMessage {
    pub fn new_from_ctx(ctx: &WsStreamCtx) -> Result<Self, DanmmuStreamError> {
        let info = ctx
            .info
            .as_ref()
            .ok_or_else(|| DanmmuStreamError::MessageParseError {
                err: "info is None".to_string(),
            })?;

        let array_2 = info
            .get(2)
            .and_then(|x| x.as_array())
            .ok_or_else(|| DanmmuStreamError::MessageParseError {
                err: "array_2 is None".to_string(),
            })?
            .to_owned();

        let uid = array_2.first().and_then(|x| x.as_u64()).ok_or_else(|| {
            DanmmuStreamError::MessageParseError {
                err: "uid is None".to_string(),
            }
        })?;

        let username = array_2
            .get(1)
            .and_then(|x| x.as_str())
            .ok_or_else(|| DanmmuStreamError::MessageParseError {
                err: "username is None".to_string(),
            })?
            .to_string();

        let msg = info
            .get(1)
            .and_then(|x| x.as_str())
            .ok_or_else(|| DanmmuStreamError::MessageParseError {
                err: "msg is None".to_string(),
            })?
            .to_string();

        let array_3 = info
            .get(3)
            .and_then(|x| x.as_array())
            .ok_or_else(|| DanmmuStreamError::MessageParseError {
                err: "array_3 is None".to_string(),
            })?
            .to_owned();

        let fan = array_3
            .get(1)
            .and_then(|x| x.as_str())
            .map(|x| x.to_owned());

        let fan_level = array_3.first().and_then(|x| x.as_u64());

        let timestamp = info
            .first()
            .and_then(|x| x.as_array())
            .and_then(|x| x.get(4))
            .and_then(|x| x.as_i64())
            .ok_or_else(|| DanmmuStreamError::MessageParseError {
                err: "timestamp is None".to_string(),
            })?;

        Ok(Self {
            uid,
            username,
            msg,
            fan,
            fan_level,
            timestamp,
        })
    }
}
