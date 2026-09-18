/// Tests for pure utility functions and helpers across the recorder crate
use recorder::sanitize_filename;
use recorder::platforms::PlatformType;

#[test]
fn test_sanitize_filename_removes_invalid_chars() {
    assert_eq!(
        sanitize_filename("test<>:\"/\\|?*.mp4"),
        "test_________.mp4"
    );
}

#[test]
fn test_sanitize_filename_preserves_valid_chars() {
    assert_eq!(
        sanitize_filename("valid_filename-123.mp4"),
        "valid_filename-123.mp4"
    );
}

#[test]
fn test_sanitize_filename_handles_chinese() {
    let result = sanitize_filename("测试直播间.mp4");
    assert!(result.contains("测试直播间"));
}

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
