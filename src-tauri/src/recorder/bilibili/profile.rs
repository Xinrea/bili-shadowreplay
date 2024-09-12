use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Profile {
    pub videos: Vec<Video>,
    pub cover: String,
    pub cover43: Option<String>,
    pub title: String,
    // 1 自制，2 转载
    pub copyright: u8,
    pub tid: u64,
    pub tag: String,
    pub desc_format_id: u64,
    pub desc: String,
    pub recreate: i8,
    pub dynamic: String,
    pub interactive: u8,
    pub act_reserve_create: u8,
    pub no_disturbance: u8,
    pub no_reprint: u8,
    pub subtitle: Subtitle,
    pub dolby: u8,
    pub lossless_music: u8,
    pub up_selection_reply: bool,
    pub up_close_reply: bool,
    pub up_close_danmu: bool,
    pub web_os: u8,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Subtitle {
    open: u8,
    lan: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Video {
    pub title: String,
    pub filename: String,
    pub desc: String,
    pub cid: u64,
}

impl Profile {
    pub fn new(title: &str, desc: &str, tid: u64) -> Self {
        Profile {
            videos: Vec::new(),
            cover: "".to_string(),
            cover43: None,
            title: title.to_string(),
            copyright: 1,
            tid,
            tag: "测试".to_string(),
            desc_format_id: 9999,
            desc: desc.to_string(),
            recreate: -1,
            dynamic: "测试".to_string(),
            interactive: 0,
            act_reserve_create: 0,
            no_disturbance: 0,
            no_reprint: 0,
            subtitle: Subtitle {
                open: 0,
                lan: "".to_string(),
            },
            dolby: 0,
            lossless_music: 0,
            up_selection_reply: false,
            up_close_reply: false,
            up_close_danmu: false,
            web_os: 3,
        }
    }
}
