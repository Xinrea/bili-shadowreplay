/// Tests for pure utility functions and helpers across the recorder crate
use recorder::platforms::PlatformType;

#[test]
fn test_platform_type_all_variants() {
    // Ensure ALL is in sync with enum variants
    assert_eq!(PlatformType::ALL.len(), 8);

    // Verify all platforms can round-trip through string conversion
    for platform in PlatformType::ALL {
        let string_repr = platform.as_str();
        let parsed = string_repr.parse::<PlatformType>().unwrap();
        assert_eq!(parsed, platform);
    }
}

#[test]
fn test_platform_type_hash() {
    use std::collections::HashSet;

    let mut set = HashSet::new();
    set.insert(PlatformType::BiliBili);
    set.insert(PlatformType::Douyin);
    set.insert(PlatformType::BiliBili); // Duplicate

    assert_eq!(set.len(), 2);
    assert!(set.contains(&PlatformType::BiliBili));
    assert!(set.contains(&PlatformType::Douyin));
}

#[test]
fn test_platform_type_equality() {
    assert_eq!(PlatformType::BiliBili, PlatformType::BiliBili);
    assert_ne!(PlatformType::BiliBili, PlatformType::Douyin);
}

#[test]
fn test_platform_type_invalid_parse() {
    assert!("invalid_platform".parse::<PlatformType>().is_err());
    assert!("BILIBILI".parse::<PlatformType>().is_err()); // Case sensitive
    assert!("".parse::<PlatformType>().is_err());
}

#[test]
fn test_platform_type_as_str() {
    assert_eq!(PlatformType::BiliBili.as_str(), "bilibili");
    assert_eq!(PlatformType::Douyin.as_str(), "douyin");
    assert_eq!(PlatformType::Huya.as_str(), "huya");
    assert_eq!(PlatformType::Kuaishou.as_str(), "kuaishou");
    assert_eq!(PlatformType::TikTok.as_str(), "tiktok");
}

#[test]
fn test_all_platforms_covered() {
    let all_names: Vec<&str> = PlatformType::ALL.iter().map(|p| p.as_str()).collect();
    assert!(all_names.contains(&"bilibili"));
    assert!(all_names.contains(&"douyin"));
    assert!(all_names.contains(&"huya"));
    assert!(all_names.contains(&"kuaishou"));
    assert!(all_names.contains(&"tiktok"));
}
