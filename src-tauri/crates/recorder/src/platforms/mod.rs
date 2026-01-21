pub mod bilibili;
pub mod douyin;
pub mod huya;
pub mod kuaishou;
pub mod xiaohongshu;
pub mod tiktok;
pub mod weibo;
use std::hash::{Hash, Hasher};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformType {
    BiliBili,
    Douyin,
    Huya,
    Youtube,
    Kuaishou,
    Xiaohongshu,
    TikTok,
    Weibo,
}

impl PlatformType {
    pub fn as_str(&self) -> &'static str {
        match self {
            PlatformType::BiliBili => "bilibili",
            PlatformType::Douyin => "douyin",
            PlatformType::Huya => "huya",
            PlatformType::Youtube => "youtube",
            PlatformType::Kuaishou => "kuaishou",
            PlatformType::Xiaohongshu => "xiaohongshu",
            PlatformType::TikTok => "tiktok",
            PlatformType::Weibo => "weibo",
        }
    }
}

impl std::str::FromStr for PlatformType {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "bilibili" => Ok(PlatformType::BiliBili),
            "douyin" => Ok(PlatformType::Douyin),
            "huya" => Ok(PlatformType::Huya),
            "youtube" => Ok(PlatformType::Youtube),
            "kuaishou" => Ok(PlatformType::Kuaishou),
            "xiaohongshu" => Ok(PlatformType::Xiaohongshu),
            "tiktok" => Ok(PlatformType::TikTok),
            "weibo" => Ok(PlatformType::Weibo),
            _ => Err(format!("Invalid platform type: {s}")),
        }
    }
}

impl Hash for PlatformType {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
    }
}
