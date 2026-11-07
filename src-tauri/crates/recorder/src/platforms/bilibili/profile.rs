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

/// A new self-made submission with Bilibili's usual settings, matching the
/// upload form's defaults (`default_profile` in the frontend).
impl Default for Profile {
    fn default() -> Self {
        Self {
            videos: Vec::new(),
            cover: String::new(),
            cover43: None,
            title: String::new(),
            copyright: 1,
            tid: 27,
            tag: String::new(),
            desc_format_id: 9999,
            desc: String::new(),
            recreate: -1,
            dynamic: String::new(),
            interactive: 0,
            act_reserve_create: 0,
            no_disturbance: 0,
            no_reprint: 0,
            subtitle: Subtitle {
                open: 0,
                lan: String::new(),
            },
            dolby: 0,
            lossless_music: 0,
            up_selection_reply: false,
            up_close_reply: false,
            up_close_danmu: false,
            web_os: 0,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Video {
    pub title: String,
    pub filename: String,
    pub desc: String,
    pub cid: u64,
}
