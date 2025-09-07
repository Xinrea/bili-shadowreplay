use serde::Deserialize;

use super::stream::WsStreamCtx;

use crate::DanmuStreamError;

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct SendGift {
    pub action: String,
    pub gift_name: String,
    pub num: u64,
    pub uname: String,
    pub uid: u64,
    pub medal_name: Option<String>,
    pub medal_level: Option<u32>,
    pub price: u32,
}

#[allow(dead_code)]
impl SendGift {
    pub fn new_from_ctx(ctx: &WsStreamCtx) -> Result<Self, DanmuStreamError> {
        let data = ctx
            .data
            .as_ref()
            .ok_or_else(|| DanmuStreamError::MessageParseError {
                err: "data is None".to_string(),
            })?;

        let action = data
            .action
            .as_ref()
            .ok_or_else(|| DanmuStreamError::MessageParseError {
                err: "action is None".to_string(),
            })?
            .to_owned();

        let combo_send = data.combo_send.clone();

        let gift_name = if let Some(gift) = data.gift_name.as_ref() {
            gift.to_owned()
        } else if let Some(gift) = combo_send.clone().and_then(|x| x.gift_name) {
            gift
        } else {
            return Err(DanmuStreamError::MessageParseError {
                err: "gift_name is None".to_string(),
            });
        };

        let num = if let Some(num) = combo_send.clone().and_then(|x| x.combo_num) {
            num
        } else if let Some(num) = data.num {
            num
        } else if let Some(num) = combo_send.and_then(|x| x.gift_num) {
            num
        } else {
            return Err(DanmuStreamError::MessageParseError {
                err: "num is None".to_string(),
            });
        };

        let uname = data
            .uname
            .as_ref()
            .ok_or_else(|| DanmuStreamError::MessageParseError {
                err: "uname is None".to_string(),
            })?
            .to_owned();

        let uid = data
            .uid
            .as_ref()
            .ok_or_else(|| DanmuStreamError::MessageParseError {
                err: "uid is None".to_string(),
            })?
            .as_u64()
            .ok_or_else(|| DanmuStreamError::MessageParseError {
                err: "uid is None".to_string(),
            })?;

        let medal_name = data
            .medal_info
            .as_ref()
            .and_then(|x| x.medal_name.to_owned());

        let medal_level = data.medal_info.as_ref().and_then(|x| x.medal_level);

        let medal_name = if medal_name == Some("".to_string()) {
            None
        } else {
            medal_name
        };

        let medal_level = if medal_level == Some(0) {
            None
        } else {
            medal_level
        };

        let price = data
            .price
            .ok_or_else(|| DanmuStreamError::MessageParseError {
                err: "price is None".to_string(),
            })?;

        Ok(Self {
            action,
            gift_name,
            num,
            uname,
            uid,
            medal_name,
            medal_level,
            price,
        })
    }
}
