use super::stream::WsStreamCtx;

use crate::DanmuStreamError;

#[derive(Debug)]
#[allow(dead_code)]
pub struct InteractWord {
    pub uid: u64,
    pub uname: String,
    pub fan: Option<String>,
    pub fan_level: Option<u32>,
}

#[allow(dead_code)]
impl InteractWord {
    pub fn new_from_ctx(ctx: &WsStreamCtx) -> Result<Self, DanmuStreamError> {
        let data = ctx
            .data
            .as_ref()
            .ok_or_else(|| DanmuStreamError::MessageParseError {
                err: "data is None".to_string(),
            })?;

        let uname = data
            .uname
            .as_ref()
            .ok_or_else(|| DanmuStreamError::MessageParseError {
                err: "uname is None".to_string(),
            })?
            .to_string();

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

        let fan = data
            .fans_medal
            .as_ref()
            .and_then(|x| x.medal_name.to_owned());

        let fan = if fan == Some("".to_string()) {
            None
        } else {
            fan
        };

        let fan_level = data.fans_medal.as_ref().and_then(|x| x.medal_level);

        let fan_level = if fan_level == Some(0) {
            None
        } else {
            fan_level
        };

        Ok(Self {
            uid,
            uname,
            fan,
            fan_level,
        })
    }
}
