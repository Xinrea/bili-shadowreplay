use recorder::account::Account;
use recorder::core::{Codec, Format};
use recorder::platforms::bilibili::api::{
    get_room_info_with_base, get_stream_info_with_base, parse_user_info_response, Protocol, Qn,
};
use recorder::platforms::PlatformType;
use reqwest::Client;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn load_fixture(name: &str) -> String {
    let fixture_path = format!("{}/tests/fixtures/{}", env!("CARGO_MANIFEST_DIR"), name);
    std::fs::read_to_string(&fixture_path)
        .unwrap_or_else(|e| panic!("Failed to load fixture {name}: {e}"))
}

fn load_json(name: &str) -> serde_json::Value {
    serde_json::from_str(&load_fixture(name)).expect("fixture must be valid JSON")
}

fn test_account() -> Account {
    Account {
        platform: "bilibili".to_string(),
        id: "1".to_string(),
        name: "test".to_string(),
        avatar: String::new(),
        csrf: "csrf".to_string(),
        // HeaderValue::from_str accepts a bare cookie header value.
        cookies: "SESSDATA=test".to_string(),
    }
}

#[tokio::test]
async fn test_bilibili_room_info_via_adapter() {
    let mock_server = MockServer::start().await;
    let room_info_json = load_fixture("bilibili_room_info.json");

    Mock::given(method("GET"))
        .and(path("/room/v1/Room/get_info"))
        .respond_with(ResponseTemplate::new(200).set_body_string(room_info_json))
        .expect(1)
        .mount(&mock_server)
        .await;

    let room = get_room_info_with_base(
        &Client::new(),
        &test_account(),
        "545068",
        &mock_server.uri(),
    )
    .await
    .expect("adapter should parse fixture room info");

    assert_eq!(room.room_id, "545068");
    assert_eq!(room.user_id, "8739477");
    assert!(!room.room_title.is_empty());
    assert!(!room.room_cover_url.is_empty());
    assert!(!room.room_keyframe_url.is_empty());
}

#[tokio::test]
async fn test_bilibili_user_info_via_adapter_parse() {
    // get_user_info also calls the WBI nav/sign endpoint, so full HTTP mocking is
    // awkward; exercise the same typed parse path the adapter uses after fetch.
    let response = load_json("bilibili_user_info.json");
    let user = parse_user_info_response(&response, "8739477")
        .expect("adapter should parse fixture user info");

    assert_eq!(user.user_id, "8739477");
    assert_eq!(user.user_name, "老实憨厚的笑笑");
    assert!(user.user_avatar_url.starts_with("http"));
    assert!(!user.user_sign.is_empty());
}

#[tokio::test]
async fn test_bilibili_play_url_via_adapter() {
    let mock_server = MockServer::start().await;
    let play_url_json = load_fixture("bilibili_play_url.json");

    Mock::given(method("GET"))
        .and(path("/xlive/web-room/v2/index/getRoomPlayInfo"))
        .respond_with(ResponseTemplate::new(200).set_body_string(play_url_json))
        .expect(1)
        .mount(&mock_server)
        .await;

    let stream = get_stream_info_with_base(
        &Client::new(),
        &test_account(),
        "545068",
        Protocol::HttpHls,
        Format::FMP4,
        &[Codec::Avc],
        Qn::Q10000,
        &mock_server.uri(),
    )
    .await
    .expect("adapter should parse fixture play URL");

    assert!(!stream.base_url.is_empty());
    assert!(!stream.url_info.is_empty());
    assert!(!stream.url_info[0].host.is_empty());
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
    assert_eq!(PlatformType::Twitch.as_str(), "twitch");

    assert_eq!(
        "bilibili".parse::<PlatformType>().unwrap(),
        PlatformType::BiliBili
    );
    assert_eq!(
        "douyin".parse::<PlatformType>().unwrap(),
        PlatformType::Douyin
    );
    assert_eq!(
        "twitch".parse::<PlatformType>().unwrap(),
        PlatformType::Twitch
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
