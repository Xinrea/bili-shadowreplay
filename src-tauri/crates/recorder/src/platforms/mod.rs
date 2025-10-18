pub mod bilibili;
pub mod douyin;
use std::hash::{Hash, Hasher};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformType {
    BiliBili,
    Douyin,
    Huya,
    Youtube,
}

impl PlatformType {
    pub fn as_str(&self) -> &'static str {
        match self {
            PlatformType::BiliBili => "bilibili",
            PlatformType::Douyin => "douyin",
            PlatformType::Huya => "huya",
            PlatformType::Youtube => "youtube",
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
            _ => Err(format!("Invalid platform type: {s}")),
        }
    }
}

impl Hash for PlatformType {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
    }
}
