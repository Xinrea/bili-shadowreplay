use std::fmt;

use crate::recorder::stream::{Stream, StreamType};
#[derive(Clone, Debug)]
pub struct BiliStream {
    pub format: StreamType,
    pub host: String,
    pub path: String,
    pub extra: String,
    pub expire: i64,
}

impl fmt::Display for BiliStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "type: {:?}, host: {}, path: {}, extra: {}, expire: {}",
            self.format, self.host, self.path, self.extra, self.expire
        )
    }
}

impl Stream for BiliStream {
    fn index(&self) -> String {
        format!("{}{}{}?{}", self.host, self.path, "index.m3u8", self.extra)
    }

    fn ts_url(&self, seg_name: &str) -> String {
        format!("{}{}{}?{}", self.host, self.path, seg_name, self.extra)
    }

    fn is_expired(&self) -> bool {
        self.expire < chrono::Utc::now().timestamp()
    }
}

impl BiliStream {
    pub fn new(format: StreamType, base_url: &str, host: &str, extra: &str) -> BiliStream {
        BiliStream {
            format,
            host: host.into(),
            path: BiliStream::get_path(base_url),
            extra: extra.into(),
            expire: BiliStream::get_expire(extra).unwrap(),
        }
    }

    pub fn get_path(base_url: &str) -> String {
        match base_url.rfind('/') {
            Some(pos) => base_url[..pos + 1].to_string(),
            None => base_url.to_string(),
        }
    }

    fn get_expire(extra: &str) -> Option<i64> {
        extra.split('&').find_map(|param| {
            if param.starts_with("expires=") {
                param.split('=').nth(1)?.parse().ok()
            } else {
                None
            }
        })
    }
}
