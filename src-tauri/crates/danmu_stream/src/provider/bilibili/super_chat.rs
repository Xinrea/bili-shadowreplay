use serde::Deserialize;

use super::stream::WsStreamCtx;

use crate::DanmuStreamError;

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct SuperChatMessage {
    pub uname: String,
    pub uid: u64,
    pub face: String,
    pub price: u32,
    pub start_time: u64,
    pub time: u32,
    pub msg: String,
    pub medal_name: Option<String>,
    pub medal_level: Option<u32>,
}

#[allow(dead_code)]
impl SuperChatMessage {
    pub fn new_from_ctx(ctx: &WsStreamCtx) -> Result<Self, DanmuStreamError> {
        let data = ctx
            .data
            .as_ref()
            .ok_or_else(|| DanmuStreamError::MessageParseError {
                err: "data is None".to_string(),
            })?;

        let user_info =
            data.user_info
                .as_ref()
                .ok_or_else(|| DanmuStreamError::MessageParseError {
                    err: "user_info is None".to_string(),
                })?;

        let uname = user_info.uname.to_owned();

        let uid = data.uid.as_ref().and_then(|x| x.as_u64()).ok_or_else(|| {
            DanmuStreamError::MessageParseError {
                err: "uid is None".to_string(),
            }
        })?;

        let face = user_info.face.to_owned();

        let price = data
            .price
            .ok_or_else(|| DanmuStreamError::MessageParseError {
                err: "price is None".to_string(),
            })?;

        let start_time = data
            .start_time
            .ok_or_else(|| DanmuStreamError::MessageParseError {
                err: "start_time is None".to_string(),
            })?;

        let time = data
            .time
            .ok_or_else(|| DanmuStreamError::MessageParseError {
                err: "time is None".to_string(),
            })?;

        let msg = data
            .message
            .as_ref()
            .ok_or_else(|| DanmuStreamError::MessageParseError {
                err: "message is None".to_string(),
            })?
            .to_owned();

        let medal = data
            .medal_info
            .as_ref()
            .map(|x| (x.medal_name.to_owned(), x.medal_level.to_owned()));

        let medal_name = medal.as_ref().and_then(|(name, _)| name.to_owned());

        let medal_level = medal.and_then(|(_, level)| level);

        Ok(Self {
            uname,
            uid,
            face,
            price,
            start_time,
            time,
            msg,
            medal_name,
            medal_level,
        })
    }
}
