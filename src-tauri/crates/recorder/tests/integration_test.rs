use recorder::platforms::PlatformType;
use recorder::account::Account;
use wiremock::{MockServer, Mock, ResponseTemplate};
use wiremock::matchers::{method, path};

fn load_fixture(name: &str) -> String {
    let fixture_path = format!("{}/tests/fixtures/{}", env!("CARGO_MANIFEST_DIR"), name);
    std::fs::read_to_string(&fixture_path)
        .unwrap_or_else(|e| panic!("Failed to load fixture {name}: {e}"))
}

fn load_json(name: &str) -> serde_json::Value {
    serde_json::from_str(&load_fixture(name)).expect("fixture must be valid JSON")
}

#[tokio::test]
async fn test_bilibili_room_info_fixture_shape() {
    let mock_server = MockServer::start().await;
    let room_info_json = load_fixture("bilibili_room_info.json");

    Mock::given(method("GET"))
        .and(path("/room/v1/Room/get_info"))
        .respond_with(ResponseTemplate::new(200).set_body_string(room_info_json))
        .mount(&mock_server)
        .await;

    let response = load_json("bilibili_room_info.json");
    assert_eq!(response["code"], 0);
    assert!(response["data"]["room_id"].as_i64().unwrap() > 0);
    assert!(response["data"]["uid"].as_i64().unwrap() > 0);
    assert!(!response["data"]["title"].as_str().unwrap().is_empty());
    assert!(response["data"]["live_status"].as_u64().is_some());
    // Fields consumed by platforms::bilibili::api::get_room_info
    assert!(response["data"]["user_cover"].as_str().is_some());
    assert!(response["data"]["keyframe"].as_str().is_some());
    assert!(response["data"]["live_time"].as_str().is_some());
}

#[tokio::test]
async fn test_bilibili_user_info_fixture_shape() {
    let response = load_json("bilibili_user_info.json");
    assert_eq!(response["code"], 0);
    assert!(response["data"]["mid"].as_i64().unwrap() > 0);
    assert!(!response["data"]["name"].as_str().unwrap().is_empty());
    assert!(response["data"]["face"]
        .as_str()
        .unwrap()
        .starts_with("http"));
}

#[tokio::test]
async fn test_bilibili_play_url_fixture_shape() {
    let mock_server = MockServer::start().await;
    let play_url_json = load_fixture("bilibili_play_url.json");

    Mock::given(method("GET"))
        .and(path("/xlive/web-room/v2/index/getRoomPlayInfo"))
        .respond_with(ResponseTemplate::new(200).set_body_string(play_url_json))
        .mount(&mock_server)
        .await;

    let response = load_json("bilibili_play_url.json");
    assert_eq!(response["code"], 0);
    let streams = response["data"]["playurl_info"]["playurl"]["stream"]
        .as_array()
        .expect("stream array");
    assert!(!streams.is_empty());
    assert_eq!(streams[0]["protocol_name"], "http_hls");
    let formats = streams[0]["format"].as_array().unwrap();
    assert!(!formats.is_empty());
    let codecs = formats[0]["codec"].as_array().unwrap();
    assert!(!codecs.is_empty());
    assert!(!codecs[0]["base_url"].as_str().unwrap().is_empty());
    assert!(!codecs[0]["url_info"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn test_douyin_room_info_fixture_shape() {
    let response = load_json("douyin_room_info.json");
    assert_eq!(response["status_code"], 0);
    let room = &response["data"]["room"];
    assert!(!room["id_str"].as_str().unwrap_or("").is_empty() || room["id"].as_i64().is_some());
    assert!(room["status"].as_u64().is_some());
    assert!(room["title"].as_str().is_some());
}

#[tokio::test]
async fn test_douyin_offline_fixture_has_non_live_status() {
    let response = load_json("douyin_room_offline.json");
    assert_eq!(response["status_code"], 0);
    // Live == 2 in Douyin H5; offline fixtures must not claim live.
    assert_ne!(response["data"]["room"]["status"], 2);
}

#[tokio::test]
async fn test_huya_page_fixture_contains_global_init() {
    let html = load_fixture("huya_room_page.html");
    assert!(html.contains("HNF_GLOBAL_INIT"));
    assert!(html.contains("roomInfo"));
    assert!(html.contains("roomProfile"));
}

#[tokio::test]
async fn test_kuaishou_initial_state_fixture_shape() {
    let response = load_json("kuaishou_initial_state.json");
    assert!(response.get("liveroom").is_some());
}

#[test]
fn test_platform_type_string_conversion() {
    assert_eq!(PlatformType::BiliBili.as_str(), "bilibili");
    assert_eq!(PlatformType::Douyin.as_str(), "douyin");
    assert_eq!(PlatformType::Kuaishou.as_str(), "kuaishou");

    assert_eq!(
        "bilibili".parse::<PlatformType>().unwrap(),
        PlatformType::BiliBili
    );
    assert_eq!(
        "douyin".parse::<PlatformType>().unwrap(),
        PlatformType::Douyin
    );
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
