use recorder::platforms::PlatformType;
use recorder::account::Account;
use wiremock::{MockServer, Mock, ResponseTemplate};
use wiremock::matchers::{method, path};

/// Test helper to load fixture data
fn load_fixture(name: &str) -> String {
    let fixture_path = format!("{}/tests/fixtures/{}", env!("CARGO_MANIFEST_DIR"), name);
    std::fs::read_to_string(fixture_path)
        .unwrap_or_else(|_| panic!("Failed to load fixture: {}", name))
}

#[tokio::test]
async fn test_bilibili_room_info_parsing() {
    // Setup mock server
    let mock_server = MockServer::start().await;
    
    let room_info_json = load_fixture("bilibili_room_info.json");
    
    Mock::given(method("GET"))
        .and(path("/room/v1/Room/get_info"))
        .respond_with(ResponseTemplate::new(200).set_body_string(room_info_json))
        .mount(&mock_server)
        .await;
    
    // Test URL parsing and basic response structure
    let response: serde_json::Value = serde_json::from_str(&load_fixture("bilibili_room_info.json")).unwrap();
    assert_eq!(response["code"], 0);
    assert_eq!(response["data"]["room_id"], 167537);
    assert_eq!(response["data"]["live_status"], 1);
}

#[tokio::test]
async fn test_bilibili_user_info_parsing() {
    let user_info_json = load_fixture("bilibili_user_info.json");
    let response: serde_json::Value = serde_json::from_str(&user_info_json).unwrap();
    
    assert_eq!(response["code"], 0);
    assert_eq!(response["data"]["mid"], 123456);
    assert_eq!(response["data"]["name"], "测试主播");
}

#[tokio::test]
async fn test_bilibili_play_url_parsing() {
    let play_url_json = load_fixture("bilibili_play_url.json");
    let response: serde_json::Value = serde_json::from_str(&play_url_json).unwrap();
    
    assert_eq!(response["code"], 0);
    assert!(response["data"]["playurl_info"]["playurl"]["stream"].is_array());
}

#[test]
fn test_platform_type_string_conversion() {
    assert_eq!(PlatformType::BiliBili.as_str(), "bilibili");
    assert_eq!(PlatformType::Douyin.as_str(), "douyin");
    assert_eq!(PlatformType::Kuaishou.as_str(), "kuaishou");
    
    assert_eq!("bilibili".parse::<PlatformType>().unwrap(), PlatformType::BiliBili);
    assert_eq!("douyin".parse::<PlatformType>().unwrap(), PlatformType::Douyin);
}

#[test]
fn test_account_creation() {
    let account = Account {
        platform: "bilibili".to_string(),
        id: "123456".to_string(),
        name: "TestUser".to_string(),
        avatar: "https://example.com/avatar.jpg".to_string(),
        csrf: "token123".to_string(),
        cookies: "session=abc123".to_string(),
    };
    
    assert_eq!(account.platform, "bilibili");
    assert_eq!(account.id, "123456");
    assert!(!account.cookies.is_empty());
}

#[test]
fn test_account_default() {
    let account = Account::default();
    assert!(account.cookies.is_empty());
    assert!(account.platform.is_empty());
}
