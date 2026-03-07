pub mod bilibili;
pub mod douyin;
pub mod huya;
pub mod kuaishou;
pub mod tiktok;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::str::FromStr;

    #[test]
    fn test_platform_type_as_str() {
        assert_eq!(PlatformType::BiliBili.as_str(), "bilibili");
        assert_eq!(PlatformType::Douyin.as_str(), "douyin");
        assert_eq!(PlatformType::Huya.as_str(), "huya");
        assert_eq!(PlatformType::Youtube.as_str(), "youtube");
        assert_eq!(PlatformType::Kuaishou.as_str(), "kuaishou");
        assert_eq!(PlatformType::Xiaohongshu.as_str(), "xiaohongshu");
        assert_eq!(PlatformType::TikTok.as_str(), "tiktok");
        assert_eq!(PlatformType::Weibo.as_str(), "weibo");
    }

    #[test]
    fn test_platform_type_from_str() {
        assert_eq!(
            PlatformType::from_str("bilibili"),
            Ok(PlatformType::BiliBili)
        );
        assert_eq!(PlatformType::from_str("douyin"), Ok(PlatformType::Douyin));
        assert_eq!(PlatformType::from_str("huya"), Ok(PlatformType::Huya));
        assert_eq!(PlatformType::from_str("youtube"), Ok(PlatformType::Youtube));
        assert_eq!(
            PlatformType::from_str("kuaishou"),
            Ok(PlatformType::Kuaishou)
        );
        assert_eq!(
            PlatformType::from_str("xiaohongshu"),
            Ok(PlatformType::Xiaohongshu)
        );
        assert_eq!(PlatformType::from_str("tiktok"), Ok(PlatformType::TikTok));
        assert_eq!(PlatformType::from_str("weibo"), Ok(PlatformType::Weibo));
    }

    #[test]
    fn test_platform_type_from_str_invalid() {
        assert!(PlatformType::from_str("invalid").is_err());
        assert!(PlatformType::from_str("").is_err());
        assert!(PlatformType::from_str("BiliBili").is_err()); // case sensitive
    }

    #[test]
    fn test_platform_type_roundtrip() {
        let platforms = [
            PlatformType::BiliBili,
            PlatformType::Douyin,
            PlatformType::Huya,
            PlatformType::Youtube,
            PlatformType::Kuaishou,
            PlatformType::Xiaohongshu,
            PlatformType::TikTok,
            PlatformType::Weibo,
        ];
        for p in platforms {
            assert_eq!(PlatformType::from_str(p.as_str()), Ok(p));
        }
    }

    #[test]
    fn test_platform_type_hash() {
        let mut set = HashSet::new();
        set.insert(PlatformType::BiliBili);
        set.insert(PlatformType::Douyin);
        set.insert(PlatformType::BiliBili); // duplicate
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn test_platform_type_equality() {
        assert_eq!(PlatformType::BiliBili, PlatformType::BiliBili);
        assert_ne!(PlatformType::BiliBili, PlatformType::Douyin);
    }

    #[test]
    fn test_platform_type_clone() {
        let p = PlatformType::Huya;
        let p2 = p;
        assert_eq!(p, p2);
    }
}
