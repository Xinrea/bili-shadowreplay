use super::response::{RoomInfo as SigiRoomInfo, SigiStateResponse, StreamUrl as SigiStreamUrl};
use crate::account::Account;
use crate::errors::RecorderError;
use chrono::Utc;
use regex::Regex;
use reqwest::header::HeaderMap;
use reqwest::{Client, Proxy, Url};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::env;
use std::sync::atomic::{AtomicI64, Ordering};

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36";
const DEFAULT_COOKIE: &str = "1%7Cz7FKki38aKyy7i-BC9rEDwcrVvjcLcFEL6QIeqldoy4%7C1761302831%7C6c1461e9f1f980cbe0404c51905177d5d53bbd822e1bf66128887d942c9c3e2f";
const TIKTOK_COOLDOWN_SECS: i64 = 120;

static TIKTOK_COOLDOWN_UNTIL: AtomicI64 = AtomicI64::new(0);

fn tiktok_api_allowed() -> bool {
    let now = Utc::now().timestamp();
    now >= TIKTOK_COOLDOWN_UNTIL.load(Ordering::Relaxed)
}

fn set_tiktok_cooldown(reason: &str) {
    let until = Utc::now().timestamp() + TIKTOK_COOLDOWN_SECS;
    TIKTOK_COOLDOWN_UNTIL.store(until, Ordering::Relaxed);
    log::info!("[TikTok] API cooldown set ({}s): {}", TIKTOK_COOLDOWN_SECS, reason);
}

#[derive(Clone, Debug)]
pub struct RoomInfo {
    pub live_status: bool,
    pub room_title: String,
    pub room_cover_url: String,
    pub user_id: String,
    pub user_name: String,
    pub user_avatar: String,
}

#[derive(Clone, Debug)]
pub struct StreamInfo {
    pub hls_url: Option<String>,
    pub rtmp_url: Option<String>,
}

pub fn proxy_url_from_env() -> Option<String> {
    for key in [
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
        "ALL_PROXY",
        "all_proxy",
    ] {
        if let Ok(value) = env::var(key) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

pub fn build_proxy_client(proxy_url: &str) -> Result<Client, RecorderError> {
    let proxy = Proxy::all(proxy_url).map_err(|_| RecorderError::ApiError {
        error: "Invalid proxy URL".to_string(),
    })?;
    Client::builder()
        .proxy(proxy)
        .http1_only()
        .build()
        .map_err(|_| RecorderError::ApiError {
            error: "Failed to build proxy client".to_string(),
        })
}

fn extract_script_json(html_str: &str, script_id: &str) -> Option<String> {
    let pattern = format!(
        r#"(?s)<script[^>]*id=['"]{script_id}['"][^>]*>(.*?)</script>"#
    );
    let regex = Regex::new(&pattern).ok()?;
    regex
        .captures(html_str)
        .and_then(|cap| cap.get(1))
        .map(|value| value.as_str().to_string())
}

fn parse_first_json_value(raw: &str) -> Option<Value> {
    let start = raw
        .find('{')
        .or_else(|| raw.find('['))?;
    let candidate = &raw[start..];
    let mut deserializer = serde_json::Deserializer::from_str(candidate);
    Value::deserialize(&mut deserializer).ok()
}

fn extract_json_after_marker(html_str: &str, marker: &str) -> Option<Value> {
    for (index, _) in html_str.match_indices(marker) {
        let slice = &html_str[index + marker.len()..];
        if let Some(value) = parse_first_json_value(slice) {
            return Some(value);
        }
    }
    None
}

fn extract_state_value(html_str: &str) -> Option<Value> {
    let script_ids = ["SIGI_STATE", "__UNIVERSAL_DATA_FOR_REHYDRATION__", "__NEXT_DATA__"];
    for script_id in script_ids {
        if let Some(json_str) = extract_script_json(html_str, script_id) {
            if let Some(parsed) = parse_first_json_value(&json_str) {
                return Some(parsed);
            }
        }
    }

    for marker in script_ids {
        if let Some(parsed) = extract_json_after_marker(html_str, marker) {
            return Some(parsed);
        }
    }

    // Try regex for window assignments
    let patterns = [
        r#"(?s)window\['SIGI_STATE'\]\s*=\s*(.*?);\s*window"#,
        r#"(?s)window\['SIGI_STATE'\]\s*=\s*(.*?);\s*</script>"#,
        r#"(?s)window\.SIGI_STATE\s*=\s*(.*?);\s*window"#,
        r#"(?s)window\.__UNIVERSAL_DATA_FOR_REHYDRATION__\s*=\s*(.*?);\s*window"#,
        r#"(?s)window\.__UNIVERSAL_DATA_FOR_REHYDRATION__\s*=\s*(.*?);\s*</script>"#,
    ];

    for pattern in patterns {
        if let Ok(regex) = Regex::new(pattern) {
             if let Some(cap) = regex.captures(html_str) {
                 if let Some(json_str) = cap.get(1) {
                     if let Some(parsed) = parse_first_json_value(json_str.as_str()) {
                         return Some(parsed);
                     }
                 }
             }
        }
    }

    None
}

fn decode_json_string(raw: &str) -> Option<String> {
    serde_json::from_str::<String>(&format!("\"{raw}\""))
        .ok()
        .or_else(|| {
            let decoded = raw
                .replace("\\u002F", "/")
                .replace("\\/", "/")
                .replace("\\u0026", "&")
                .replace("\\u003D", "=");
            if decoded == raw {
                None
            } else {
                Some(decoded)
            }
        })
}

fn extract_m3u8_from_html(html_str: &str) -> Option<String> {
    let regex = Regex::new(r#"(https?:\\?/\\?/[^"'\\s]+\\.m3u8[^"'\\s]*)"#).ok()?;
    let raw = regex.captures(html_str)?.get(1)?.as_str();
    let decoded = decode_json_string(raw).unwrap_or_else(|| raw.to_string());
    if decoded.contains(".m3u8") {
        Some(decoded)
    } else {
        None
    }
}

fn find_m3u8_in_value(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => {
            let decoded = decode_json_string(value).unwrap_or_else(|| value.to_string());
            if decoded.contains(".m3u8") && decoded.starts_with("http") {
                Some(decoded)
            } else {
                None
            }
        }
        Value::Object(map) => {
            for child in map.values() {
                if let Some(url) = find_m3u8_in_value(child) {
                    return Some(url);
                }
            }
            None
        }
        Value::Array(values) => {
            for value in values {
                if let Some(url) = find_m3u8_in_value(value) {
                    return Some(url);
                }
            }
            None
        }
        _ => None,
    }
}

fn extract_username_from_url(url: &str) -> String {
    let url_no_query = url.split('?').next().unwrap_or(url);
    let segments = url_no_query.split('/').filter(|part| !part.is_empty());
    for segment in segments {
        if let Some(stripped) = segment.strip_prefix('@') {
            if !stripped.is_empty() {
                return stripped.to_string();
            }
        }
    }
    String::new()
}

fn get_string_field(map: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(value) = map.get(*key) {
            match value {
                Value::String(value) if !value.is_empty() => return Some(value.clone()),
                Value::Number(value) => return Some(value.to_string()),
                _ => {}
            }
        }
    }
    None
}

fn get_i64_field(map: &Map<String, Value>, keys: &[&str]) -> Option<i64> {
    for key in keys {
        if let Some(value) = map.get(*key) {
            match value {
                Value::Number(value) => return value.as_i64(),
                Value::String(value) => {
                    if let Ok(parsed) = value.parse::<i64>() {
                        return Some(parsed);
                    }
                }
                _ => {}
            }
        }
    }
    None
}

fn parse_stream_data_value(raw: &str) -> Option<Value> {
    serde_json::from_str::<Value>(raw)
        .ok()
        .or_else(|| decode_json_string(raw).and_then(|decoded| serde_json::from_str(&decoded).ok()))
}

fn append_codec(url: &str, codec: &str) -> String {
    if codec.is_empty() || url.contains("codec=") {
        return url.to_string();
    }
    let separator = if url.contains('?') { "&" } else { "?" };
    format!("{url}{separator}codec={codec}")
}

#[derive(Clone, Debug)]
struct StreamCandidate {
    hls_url: Option<String>,
    flv_url: Option<String>,
    bitrate: i64,
    width: i64,
    height: i64,
}

fn extract_stream_candidates(live_room_info: &Value) -> Vec<StreamCandidate> {
    let stream_data_raw = match live_room_info
        .get("liveRoom")
        .and_then(|value| value.get("streamData"))
        .and_then(|value| value.get("pull_data"))
        .and_then(|value| value.get("stream_data"))
        .and_then(|value| value.as_str())
    {
        Some(value) => value,
        None => return Vec::new(),
    };

    let stream_data_value = match parse_stream_data_value(stream_data_raw) {
        Some(value) => value,
        None => return Vec::new(),
    };
    let data = match stream_data_value.get("data").and_then(|value| value.as_object()) {
        Some(value) => value,
        None => return Vec::new(),
    };

    let mut candidates = Vec::new();

    for entry in data.values() {
        let main = entry.get("main").and_then(|value| value.as_object());
        let Some(main) = main else { continue };
        let sdk_params_raw = main
            .get("sdk_params")
            .and_then(|value| value.as_str())
            .unwrap_or("{}");
        let sdk_params = serde_json::from_str::<Value>(sdk_params_raw).unwrap_or(Value::Null);
        let bitrate = sdk_params
            .get("vbitrate")
            .and_then(|value| value.as_i64())
            .or_else(|| {
                sdk_params
                    .get("vbitrate")
                    .and_then(|value| value.as_str())
                    .and_then(|value| value.parse::<i64>().ok())
            })
            .unwrap_or(0);
        let resolution = sdk_params
            .get("resolution")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let (width, height) = resolution
            .split_once('x')
            .and_then(|(w, h)| Some((w.parse::<i64>().ok()?, h.parse::<i64>().ok()?)))
            .unwrap_or((0, 0));
        let vcodec = sdk_params
            .get("VCodec")
            .and_then(|value| value.as_str())
            .unwrap_or("");

        let hls_url = main
            .get("hls")
            .and_then(|value| value.as_str())
            .map(|url| append_codec(url, vcodec));
        let flv_url = main
            .get("flv")
            .and_then(|value| value.as_str())
            .map(|url| append_codec(url, vcodec));

        if hls_url.is_none() && flv_url.is_none() {
            continue;
        }

        candidates.push(StreamCandidate {
            hls_url,
            flv_url,
            bitrate,
            width,
            height,
        });
    }

    candidates.sort_by(|a, b| {
        b.bitrate
            .cmp(&a.bitrate)
            .then_with(|| b.width.cmp(&a.width))
            .then_with(|| b.height.cmp(&a.height))
    });

    candidates
}

fn extract_stream_from_live_room(live_room_info: &Value) -> Option<StreamInfo> {
    let candidates = extract_stream_candidates(live_room_info);
    let best = candidates.first()?;
    Some(StreamInfo {
        hls_url: best.hls_url.clone(),
        rtmp_url: best.flv_url.clone(),
    })
}

async fn check_url_accessible(client: &Client, headers: &HeaderMap, url: &str) -> bool {
    if url.contains(".m3u8") {
        return check_hls_stream_accessible(client, headers, url).await;
    }
    let mut request = client.get(url).headers(headers.clone());
    request = request.header("Range", "bytes=0-1");
    match request.send().await {
        Ok(response) => response.status().is_success(),
        Err(_) => false,
    }
}

fn extract_first_media_uri(m3u8_text: &str) -> Option<String> {
    for line in m3u8_text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        return Some(trimmed.to_string());
    }
    None
}

fn resolve_uri(base: &str, uri: &str) -> Option<String> {
    if uri.starts_with("http://") || uri.starts_with("https://") {
        return Some(uri.to_string());
    }
    let base_url = Url::parse(base).ok()?;
    base_url.join(uri).ok().map(|url| url.to_string())
}

async fn check_hls_stream_accessible(
    client: &Client,
    headers: &HeaderMap,
    m3u8_url: &str,
) -> bool {
    let response = match client.get(m3u8_url).headers(headers.clone()).send().await {
        Ok(resp) => resp,
        Err(_) => return false,
    };
    if !response.status().is_success() {
        return false;
    }
    let text = match response.text().await {
        Ok(text) => text,
        Err(_) => return false,
    };

    let first_uri = match extract_first_media_uri(&text) {
        Some(uri) => uri,
        None => return false,
    };

    let resolved = match resolve_uri(m3u8_url, &first_uri) {
        Some(url) => url,
        None => return false,
    };

    if resolved.contains(".m3u8") {
        let response = match client.get(&resolved).headers(headers.clone()).send().await {
            Ok(resp) => resp,
            Err(_) => return false,
        };
        if !response.status().is_success() {
            return false;
        }
        let text = match response.text().await {
            Ok(text) => text,
            Err(_) => return false,
        };
        let first_segment = match extract_first_media_uri(&text) {
            Some(uri) => uri,
            None => return false,
        };
        let segment_url = match resolve_uri(&resolved, &first_segment) {
            Some(url) => url,
            None => return false,
        };
        let response = match client
            .get(&segment_url)
            .headers(headers.clone())
            .header("Range", "bytes=0-1")
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(_) => return false,
        };
        return response.status().is_success();
    }

    let response = match client
        .get(&resolved)
        .headers(headers.clone())
        .header("Range", "bytes=0-1")
        .send()
        .await
    {
        Ok(resp) => resp,
        Err(_) => return false,
    };
    response.status().is_success()
}

async fn select_accessible_stream(
    client: &Client,
    headers: &HeaderMap,
    candidates: &[StreamCandidate],
) -> Option<StreamInfo> {
    for candidate in candidates {
        if let Some(hls_url) = candidate.hls_url.as_deref() {
            if check_url_accessible(client, headers, hls_url).await {
                return Some(StreamInfo {
                    hls_url: candidate.hls_url.clone(),
                    rtmp_url: candidate.flv_url.clone(),
                });
            }
        }

        if let Some(flv_url) = candidate.flv_url.as_deref() {
            if check_url_accessible(client, headers, flv_url).await {
                return Some(StreamInfo {
                    hls_url: None,
                    rtmp_url: Some(flv_url.to_string()),
                });
            }
        }
    }

    candidates.first().map(|candidate| StreamInfo {
        hls_url: candidate.hls_url.clone(),
        rtmp_url: candidate.flv_url.clone(),
    })
}

async fn verify_live_stream(
    client: &Client,
    headers: &HeaderMap,
    room_stream: Option<&SigiStreamUrl>,
    sigi_value: &Value,
    html_str: &str,
) -> bool {
    let mut candidates: Vec<String> = Vec::new();

    if let Some(stream) = room_stream {
        if let Some(url) = stream.hls_pull_url.as_ref() {
            candidates.push(url.clone());
        }
        if let Some(url) = stream.rtmp_pull_url.as_ref() {
            candidates.push(url.clone());
        }
    }

    if candidates.is_empty() {
        if let Some(stream) = find_stream_url(sigi_value) {
            if let Some(url) = stream.hls_pull_url.or(stream.rtmp_pull_url) {
                candidates.push(url);
            }
        }
    }

    if candidates.is_empty() {
        if let Some(live_room_info) = extract_live_room_user_info(sigi_value) {
            if let Some(stream_info) = extract_stream_from_live_room(&live_room_info) {
                if let Some(url) = stream_info.hls_url.or(stream_info.rtmp_url) {
                    candidates.push(url);
                }
            }
        }
    }

    if candidates.is_empty() {
        if let Some(url) = find_m3u8_in_value(sigi_value)
            .or_else(|| extract_m3u8_from_html(html_str))
        {
            candidates.push(url);
        }
    }

    let mut seen = HashSet::new();
    for url in candidates.into_iter().filter(|u| seen.insert(u.clone())) {
        if check_url_accessible(client, headers, &url).await {
            return true;
        }
    }

    false
}

fn extract_live_room_user_info(value: &Value) -> Option<Value> {
    match value {
        Value::Object(map) => {
            if let Some(live_room) = map.get("LiveRoom") {
                if let Some(info) = live_room.get("liveRoomUserInfo") {
                    return Some(info.clone());
                }
            }
            if let Some(info) = map.get("liveRoomUserInfo") {
                return Some(info.clone());
            }

            for child in map.values() {
                if let Some(info) = extract_live_room_user_info(child) {
                    return Some(info);
                }
            }

            None
        }
        Value::Array(values) => {
            for value in values {
                if let Some(info) = extract_live_room_user_info(value) {
                    return Some(info);
                }
            }
            None
        }
        Value::String(value) => {
            parse_first_json_value(value).and_then(|parsed| extract_live_room_user_info(&parsed))
        }
        _ => None,
    }
}

fn extract_first_url(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Object(map) => {
            if let Some(url) = map.get("url").and_then(|v| v.as_str()) {
                Some(url.to_string())
            } else if let Some(list) = map
                .get("url_list")
                .or_else(|| map.get("urlList"))
                .or_else(|| map.get("urls"))
                .and_then(|v| v.as_array())
            {
                list.first().and_then(|v| v.as_str()).map(|s| s.to_string())
            } else {
                None
            }
        }
        Value::Array(list) => list.first().and_then(|v| v.as_str()).map(|s| s.to_string()),
        _ => None
    }
}

fn normalize_image_url(url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.starts_with("//") {
        format!("https:{}", trimmed)
    } else {
        trimmed.to_string()
    }
}

fn find_cover_url(value: &Value) -> Option<String> {
    const COVER_KEYS: [&str; 10] = [
        "cover",
        "coverUrl",
        "cover_url",
        "coverImage",
        "coverImageUrl",
        "roomCover",
        "roomCoverUrl",
        "liveRoomCover",
        "shareCover",
        "shareImage",
    ];
    match value {
        Value::Object(map) => {
            for key in COVER_KEYS {
                if let Some(value) = map.get(key) {
                    if let Some(url) = extract_first_url(value) {
                        return Some(normalize_image_url(&url));
                    }
                }
            }
            for (key, value) in map {
                if key.to_ascii_lowercase().contains("cover") {
                    if let Some(url) = extract_first_url(value) {
                        return Some(normalize_image_url(&url));
                    }
                }
            }
            for child in map.values() {
                if let Some(url) = find_cover_url(child) {
                    return Some(url);
                }
            }
            None
        }
        Value::Array(values) => {
            for value in values {
                if let Some(url) = find_cover_url(value) {
                    return Some(url);
                }
            }
            None
        }
        _ => None,
    }
}

fn find_avatar_url(value: &Value) -> Option<String> {
    const AVATAR_KEYS: [&str; 9] = [
        "avatarThumb",
        "avatar_thumb",
        "avatar",
        "avatarUrl",
        "avatarMedium",
        "avatarLarge",
        "headUrl",
        "head_url",
        "profileImage",
    ];
    match value {
        Value::Object(map) => {
            for key in AVATAR_KEYS {
                if let Some(value) = map.get(key) {
                    if let Some(url) = extract_first_url(value) {
                        return Some(normalize_image_url(&url));
                    }
                }
            }
            for child in map.values() {
                if let Some(url) = find_avatar_url(child) {
                    return Some(url);
                }
            }
            None
        }
        Value::Array(values) => {
            for value in values {
                if let Some(url) = find_avatar_url(value) {
                    return Some(url);
                }
            }
            None
        }
        _ => None,
    }
}

fn extract_room_info_from_live_room(
    live_room_info: &Value,
    account: &Account,
    url: &str,
) -> Option<RoomInfo> {
    let live_room_map = live_room_info.as_object()?;
    let user_map = live_room_map.get("user").and_then(|value| value.as_object());
    let live_room_map = live_room_map.get("liveRoom").and_then(|value| value.as_object());

    let user_name = user_map
        .and_then(|map| get_string_field(map, &["nickname", "nickName", "name", "userName"]))
        .unwrap_or_default();
    let user_id = user_map
        .and_then(|map| get_string_field(map, &["uniqueId", "userId", "id", "uid"]))
        .unwrap_or_default();
    let status = user_map.and_then(|map| get_i64_field(map, &["status", "liveStatus"]));
    let live_room_status =
        live_room_map.and_then(|map| get_i64_field(map, &["status", "liveStatus"]));
    let title = live_room_map
        .and_then(|map| get_string_field(map, &["title", "roomTitle"]))
        .unwrap_or_default();
    
    let user_avatar = user_map
        .and_then(|map| map.get("avatarThumb"))
        .and_then(|v| extract_first_url(v))
        .map(|url| normalize_image_url(&url))
        .unwrap_or_default();
    let room_cover_url = live_room_map
        .and_then(|map| find_cover_url(&Value::Object(map.clone())))
        .unwrap_or_default();

    let status_flag = status.or(live_room_status);
    let live_status = if let Some(flag) = status_flag {
        flag == 2
    } else {
        extract_stream_from_live_room(live_room_info)
            .and_then(|stream| stream.hls_url.or(stream.rtmp_url))
            .is_some()
    };

    let extracted_name = extract_username_from_url(url);
    let final_user_name = if !user_name.is_empty() {
        user_name
    } else if !account.name.is_empty() {
        account.name.clone()
    } else if !extracted_name.is_empty() {
        extracted_name.clone()
    } else {
        "TikTok Live".to_string()
    };
    let final_user_id = if !user_id.is_empty() {
        user_id
    } else if !account.id.is_empty() {
        account.id.clone()
    } else {
        extracted_name
    };

    Some(RoomInfo {
        live_status,
        room_title: if title.is_empty() {
            format!("{}'s live", final_user_name)
        } else {
            title
        },
        room_cover_url: if room_cover_url.is_empty() {
            user_avatar.clone()
        } else {
            room_cover_url
        },
        user_id: final_user_id,
        user_name: final_user_name,
        user_avatar,
    })
}

fn looks_like_room_info(map: &Map<String, Value>) -> bool {
    if !(map.contains_key("status")
        || map.contains_key("liveStatus")
        || map.contains_key("live_status"))
    {
        return false;
    }
    let has_owner = map.contains_key("owner")
        || map.contains_key("ownerInfo")
        || map.contains_key("user")
        || map.contains_key("userInfo")
        || map.contains_key("host")
        || map.contains_key("author");
    let has_stream = map.contains_key("streamUrl")
        || map.contains_key("stream_url")
        || map.contains_key("streamUrlInfo")
        || map.contains_key("stream_url_info");
    let has_title = map.contains_key("title")
        || map.contains_key("roomId")
        || map.contains_key("room_id")
        || map.contains_key("liveRoomId")
        || map.contains_key("live_room_id");
    (has_owner || has_stream) && has_title
}

fn find_room_info(value: &Value) -> Option<SigiRoomInfo> {
    match value {
        Value::Object(map) => {
            if let Some(room_info_value) = map.get("roomInfo") {
                if let Ok(room_info) = serde_json::from_value::<SigiRoomInfo>(
                    room_info_value.clone(),
                ) {
                    return Some(room_info);
                }
            }

            if looks_like_room_info(map) {
                if let Ok(room_info) =
                    serde_json::from_value::<SigiRoomInfo>(Value::Object(map.clone()))
                {
                    return Some(room_info);
                }
            }

            for child in map.values() {
                if let Some(room_info) = find_room_info(child) {
                    return Some(room_info);
                }
            }

            None
        }
        Value::Array(values) => {
            for value in values {
                if let Some(room_info) = find_room_info(value) {
                    return Some(room_info);
                }
            }
            None
        }
        Value::String(value) => {
            parse_first_json_value(value).and_then(|parsed| find_room_info(&parsed))
        }
        _ => None,
    }
}

fn looks_like_stream_url(map: &Map<String, Value>) -> bool {
    map.contains_key("hlsPullUrl")
        || map.contains_key("hls_pull_url")
        || map.contains_key("hlsPlayUrl")
        || map.contains_key("rtmpPullUrl")
        || map.contains_key("rtmp_pull_url")
        || map.contains_key("rtmpPlayUrl")
        || map.contains_key("flvPullUrl")
}

fn find_stream_url(value: &Value) -> Option<SigiStreamUrl> {
    match value {
        Value::Object(map) => {
            if looks_like_stream_url(map) {
                if let Ok(stream_url) =
                    serde_json::from_value::<SigiStreamUrl>(Value::Object(map.clone()))
                {
                    return Some(stream_url);
                }
            }

            for child in map.values() {
                if let Some(stream_url) = find_stream_url(child) {
                    return Some(stream_url);
                }
            }

            None
        }
        Value::Array(values) => {
            for value in values {
                if let Some(stream_url) = find_stream_url(value) {
                    return Some(stream_url);
                }
            }
            None
        }
        Value::String(value) => {
            parse_first_json_value(value).and_then(|parsed| find_stream_url(&parsed))
        }
        _ => None,
    }
}

/// Get room information from TikTok page
/// Note: TikTok requires proxy to access in most regions
pub async fn get_room_info(
    client: &Client,
    account: &Account,
    url: &str,
) -> Result<RoomInfo, RecorderError> {
    if !tiktok_api_allowed() {
        return Err(RecorderError::ApiError {
            error: "TikTok API cooldown".to_string(),
        });
    }
    let proxy_url = proxy_url_from_env();
    let request_client = if let Some(proxy_url) = proxy_url.as_deref() {
        build_proxy_client(proxy_url)?
    } else {
        client.clone()
    };
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("User-Agent", USER_AGENT.parse().unwrap());
    headers.insert("referer", "https://www.tiktok.com/".parse().unwrap());
    headers.insert("accept-language", "en-US,en;q=0.9".parse().unwrap());

    let cookie = if account.cookies.is_empty() {
        DEFAULT_COOKIE
    } else {
        &account.cookies
    };
    headers.insert("cookie", cookie.parse().unwrap());

    // Retry up to 3 times
    for attempt in 0..3 {
        let response = request_client
            .get(url)
            .headers(headers.clone())
            .send()
            .await?;
        let status = response.status();
        let html_str = response.text().await?;
        if !status.is_success() {
            if status == reqwest::StatusCode::FORBIDDEN
                || status == reqwest::StatusCode::TOO_MANY_REQUESTS
            {
                set_tiktok_cooldown(&format!("response status {}", status));
            }
            return Err(RecorderError::ApiError {
                error: format!("TikTok response status: {}", status),
            });
        }

        // Check for region block
        if html_str.contains("We regret to inform you that we have discontinued operating TikTok") {
            return Err(RecorderError::ApiError {
                error: "TikTok is not available in this region. Please use a different proxy.".to_string(),
            });
        }

        // Check for unexpected EOF
        if html_str.contains("UNEXPECTED_EOF_WHILE_READING") {
            if attempt < 2 {
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                continue;
            } else {
                return Err(RecorderError::ApiError {
                    error: "Failed to load page after 3 attempts".to_string(),
                });
            }
        }

        let sigi_value = match extract_state_value(&html_str) {
            Some(value) => value,
            None => {
                if attempt < 2 {
                    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                    continue;
                }
                return Err(RecorderError::ApiError {
                    error: "Please check if your network can access TikTok normally. Failed to extract page state JSON.".to_string(),
                });
            }
        };

        let mut extracted_room_info = serde_json::from_value::<SigiStateResponse>(sigi_value.clone())
            .ok()
            .and_then(|state| state.room_store)
            .and_then(|store| store.room_info);

        if extracted_room_info.is_none() {
            extracted_room_info = find_room_info(&sigi_value);
        }

        if let Some(room_info) = extracted_room_info {
            let mut live_status = match room_info.status {
                Some(status) => status == 2,
                None => room_info
                    .stream_url
                    .as_ref()
                    .map(|stream| stream.hls_pull_url.is_some() || stream.rtmp_pull_url.is_some())
                    .unwrap_or(false),
            };
            if live_status
                && !verify_live_stream(
                    &request_client,
                    &headers,
                    room_info.stream_url.as_ref(),
                    &sigi_value,
                    &html_str,
                )
                .await
            {
                live_status = false;
            }

            let user_id = room_info
                .owner
                .as_ref()
                .and_then(|o| o.id.clone())
                .unwrap_or_default();

            let user_name = room_info
                .owner
                .as_ref()
                .and_then(|o| o.nickname.clone())
                .filter(|n| !n.is_empty())
                .or_else(|| {
                    room_info.owner.as_ref().and_then(|o| o.unique_id.clone())
                })
                .or_else(|| {
                    room_info.owner.as_ref().and_then(|o| o.id.clone())
                })
                .unwrap_or_default();

            let mut user_avatar = room_info
                .owner
                .as_ref()
                .and_then(|o| o.avatar_thumb.as_ref())
                .and_then(|v| extract_first_url(v))
                .map(|url| normalize_image_url(&url))
                .unwrap_or_default();
            if user_avatar.is_empty() {
                if let Some(found) = find_avatar_url(&sigi_value) {
                    user_avatar = found;
                }
            }
            let mut room_cover_url =
                find_cover_url(&sigi_value).unwrap_or_else(|| String::new());
            if room_cover_url.is_empty() {
                room_cover_url = user_avatar.clone();
            }

            return Ok(RoomInfo {
                live_status,
                room_title: room_info.title.unwrap_or_default(),
                room_cover_url,
                user_id,
                user_name,
                user_avatar,
            });
        }

        if let Some(live_room_info) = extract_live_room_user_info(&sigi_value) {
            if let Some(mut room_info) =
                extract_room_info_from_live_room(&live_room_info, account, url)
            {
                if room_info.live_status
                    && !verify_live_stream(
                        &request_client,
                        &headers,
                        None,
                        &sigi_value,
                        &html_str,
                    )
                    .await
                {
                    room_info.live_status = false;
                }
                return Ok(room_info);
            }
        }

        let fallback_stream = find_stream_url(&sigi_value)
            .and_then(|stream| stream.hls_pull_url.or(stream.rtmp_pull_url))
            .or_else(|| find_m3u8_in_value(&sigi_value))
            .or_else(|| extract_m3u8_from_html(&html_str));

        if let Some(fallback_stream) = fallback_stream {
            let user_avatar = find_avatar_url(&sigi_value).unwrap_or_default();
            let extracted_name = extract_username_from_url(url);
            let user_name = if !account.name.is_empty() {
                account.name.clone()
            } else if !extracted_name.is_empty() {
                extracted_name.clone()
            } else {
                "TikTok Live".to_string()
            };
            let user_id = if !account.id.is_empty() {
                account.id.clone()
            } else {
                extracted_name
            };

            let live_status =
                check_url_accessible(&request_client, &headers, &fallback_stream).await;

            return Ok(RoomInfo {
                live_status,
                room_title: format!("{}'s live", user_name),
                room_cover_url: user_avatar.clone(),
                user_id,
                user_name,
                user_avatar,
            });
        }

        return Err(RecorderError::ApiError {
            error: "Failed to extract room info from page data".to_string(),
        });
    }

    Err(RecorderError::ApiError {
        error: "Failed to fetch TikTok page after retries".to_string(),
    })
}

/// Get stream URL from TikTok page
pub async fn get_stream_url(
    client: &Client,
    account: &Account,
    url: &str,
) -> Result<StreamInfo, RecorderError> {
    if !tiktok_api_allowed() {
        return Err(RecorderError::ApiError {
            error: "TikTok API cooldown".to_string(),
        });
    }
    let proxy_url = proxy_url_from_env();
    let request_client = if let Some(proxy_url) = proxy_url.as_deref() {
        build_proxy_client(proxy_url)?
    } else {
        client.clone()
    };
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("User-Agent", USER_AGENT.parse().unwrap());
    headers.insert("referer", "https://www.tiktok.com/".parse().unwrap());
    headers.insert("accept-language", "en-US,en;q=0.9".parse().unwrap());

    let cookie = if account.cookies.is_empty() {
        DEFAULT_COOKIE
    } else {
        &account.cookies
    };
    headers.insert("cookie", cookie.parse().unwrap());

    // Retry up to 3 times
    for attempt in 0..3 {
        let response = request_client
            .get(url)
            .headers(headers.clone())
            .send()
            .await?;
        let status = response.status();
        let html_str = response.text().await?;
        if !status.is_success() {
            if status == reqwest::StatusCode::FORBIDDEN
                || status == reqwest::StatusCode::TOO_MANY_REQUESTS
            {
                set_tiktok_cooldown(&format!("response status {}", status));
            }
            return Err(RecorderError::ApiError {
                error: format!("TikTok response status: {}", status),
            });
        }

        // Check for unexpected EOF
        if html_str.contains("UNEXPECTED_EOF_WHILE_READING") {
            if attempt < 2 {
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                continue;
            } else {
                return Err(RecorderError::ApiError {
                    error: "Failed to load page after 3 attempts".to_string(),
                });
            }
        }

        let sigi_value = match extract_state_value(&html_str) {
            Some(value) => value,
            None => {
                if attempt < 2 {
                    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                    continue;
                }
                return Err(RecorderError::ApiError {
                    error: "Failed to extract page state JSON".to_string(),
                });
            }
        };

        if let Some(live_room_info) = extract_live_room_user_info(&sigi_value) {
            let candidates = extract_stream_candidates(&live_room_info);
            if let Some(stream_info) =
                select_accessible_stream(&request_client, &headers, &candidates).await
            {
                return Ok(stream_info);
            }
        }

        let mut stream_url = serde_json::from_value::<SigiStateResponse>(sigi_value.clone())
            .ok()
            .and_then(|state| state.room_store)
            .and_then(|store| store.room_info)
            .and_then(|room_info| room_info.stream_url);

        if stream_url.is_none() {
            stream_url = find_room_info(&sigi_value).and_then(|room_info| room_info.stream_url);
        }

        if stream_url.is_none() {
            stream_url = find_stream_url(&sigi_value);
        }

        if let Some(stream_url) = stream_url {
            let mut info = StreamInfo {
                hls_url: stream_url.hls_pull_url,
                rtmp_url: stream_url.rtmp_pull_url,
            };
            if let Some(hls_url) = info.hls_url.as_deref() {
                if !check_hls_stream_accessible(&request_client, &headers, hls_url).await {
                    info.hls_url = None;
                }
            }
            if info.hls_url.is_none() && info.rtmp_url.is_none() {
                return Err(RecorderError::ApiError {
                    error: "No available stream provided".to_string(),
                });
            }
            return Ok(info);
        }

        if let Some(m3u8_url) = find_m3u8_in_value(&sigi_value)
            .or_else(|| extract_m3u8_from_html(&html_str))
        {
            return Ok(StreamInfo {
                hls_url: Some(m3u8_url),
                rtmp_url: None,
            });
        }

        return Err(RecorderError::ApiError {
            error: "Failed to extract stream URL from page data".to_string(),
        });
    }

    Err(RecorderError::ApiError {
        error: "Failed to fetch TikTok page after retries".to_string(),
    })
}

/// Get user information from TikTok
pub async fn get_user_info(
    client: &Client,
    account: &Account,
) -> Result<crate::UserInfo, RecorderError> {
    let mut headers = HeaderMap::new();
    headers.insert("User-Agent", USER_AGENT.parse().unwrap());
    headers.insert(
        "Accept-Language",
        "zh-CN,zh;q=0.8,zh-TW;q=0.7,zh-HK;q=0.5,en-US;q=0.3,en;q=0.2"
            .parse()
            .unwrap(),
    );

    if !account.cookies.is_empty() {
        headers.insert("Cookie", account.cookies.parse().unwrap());
    }

    let proxy_url = proxy_url_from_env();
    let request_client = if let Some(proxy_url) = proxy_url.as_deref() {
        build_proxy_client(proxy_url)?
    } else {
        client.clone()
    };

    // Access TikTok homepage to get user info from state
    let response = request_client
        .get("https://www.tiktok.com/")
        .headers(headers.clone())
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(RecorderError::ApiError {
            error: format!("Failed to fetch TikTok, status: {}", response.status()),
        });
    }

    let html_str = response.text().await?;

    // Check for region block
    if html_str.contains("We regret to inform you that we have discontinued operating TikTok") {
        return Err(RecorderError::ApiError {
            error: "TikTok is not available in this region. Please use a different proxy.".to_string(),
        });
    }

    // 1. Try Passport API first (more reliable JSON API)
    if let Ok(info) = get_user_info_from_passport(&request_client, &headers).await {
        return Ok(info);
    }

    let state = extract_state_value(&html_str).ok_or(RecorderError::ApiError {
        error: "Failed to extract TikTok state - please check if your network can access TikTok normally (proxy might be needed).".to_string(),
    })?;

    // Try to find current user in SigiState
    if let Some(user_info) = find_current_user_info(&state) {
        return Ok(user_info);
    }

    // Fallback: extract from specific path if known
    // SIGI_STATE -> UserModule -> users -> [username]
    if let Some(user_module) = state.get("UserModule").and_then(|u| u.get("users")) {
        if let Some(obj) = user_module.as_object() {
            if let Some((_, user_val)) = obj.iter().next() {
                let user_id = get_string_field(user_val.as_object().unwrap_or(&Map::new()), &["id", "secUid"]).unwrap_or_default();
                let user_name = get_string_field(user_val.as_object().unwrap_or(&Map::new()), &["nickname", "uniqueId"]).unwrap_or_default();
                let user_avatar = find_avatar_url(user_val).unwrap_or_default();
                
                if !user_id.is_empty() && !user_name.is_empty() {
                    return Ok(crate::UserInfo {
                        user_id,
                        user_name,
                        user_avatar,
                    });
                }
            }
        }
    }

    Err(RecorderError::ApiError {
        error: "Could not find user info in TikTok page".to_string(),
    })
}

fn find_current_user_info(value: &Value) -> Option<crate::UserInfo> {
    // 1. In SIGI_STATE, the current user is often under "AppContext" or "UserModule"
    if let Some(user) = value.get("AppContext").and_then(|a| a.get("appContext")).and_then(|c| c.get("user")) {
        let user_id = get_string_field(user.as_object().unwrap_or(&Map::new()), &["uid", "id"]).unwrap_or_default();
        let user_name = get_string_field(user.as_object().unwrap_or(&Map::new()), &["nickname", "uniqueId"]).unwrap_or_default();
        let user_avatar = find_avatar_url(user).unwrap_or_default();
        
        if !user_id.is_empty() {
            return Some(crate::UserInfo {
                user_id: user_id.clone(),
                user_name: if user_name.is_empty() { user_id.clone() } else { user_name },
                user_avatar,
            });
        }
    }

    // 2. In __UNIVERSAL_DATA_FOR_REHYDRATION__, it's under webapp.user-info
    if let Some(obj) = value.as_object() {
        for val in obj.values() {
            if let Some(user) = val.get("__DEFAULT_SCOPE__")
                .and_then(|s| s.get("webapp.user-info"))
                .and_then(|u| u.get("data"))
                .and_then(|d| d.get("user")) {
                let user_id = get_string_field(user.as_object().unwrap_or(&Map::new()), &["uid", "id", "secUid"]).unwrap_or_default();
                let user_name = get_string_field(user.as_object().unwrap_or(&Map::new()), &["nickname", "uniqueId"]).unwrap_or_default();
                let user_avatar = find_avatar_url(user).unwrap_or_default();
                
                if !user_id.is_empty() {
                    return Some(crate::UserInfo {
                        user_id: user_id.clone(),
                        user_name: if user_name.is_empty() { user_id.clone() } else { user_name },
                        user_avatar,
                    });
                }
            }
        }
    }

    None
}

/// Get user info from TikTok passport API
async fn get_user_info_from_passport(
    client: &reqwest::Client,
    headers: &reqwest::header::HeaderMap,
) -> Result<crate::UserInfo, RecorderError> {
    let response = client
        .get("https://www.tiktok.com/passport/web/user/info/")
        .headers(headers.clone())
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(RecorderError::ApiError {
            error: format!("Failed to fetch TikTok passport info, status: {}", response.status()),
        });
    }

    let json: Value = response.json().await?;
    if let Some(data) = json.get("data").and_then(|d| d.as_object()) {
        let user_id = get_string_field(data, &["user_id", "uid", "id"]).unwrap_or_default();
        let user_name = get_string_field(data, &["nickname", "unique_id"]).unwrap_or_default();
        let user_avatar = find_avatar_url(&json).unwrap_or_default();

        if !user_id.is_empty() {
            let final_name = if user_name.is_empty() {
                user_id.clone()
            } else {
                user_name
            };
            return Ok(crate::UserInfo {
                user_id,
                user_name: final_name,
                user_avatar,
            });
        }
    }

    Err(RecorderError::ApiError {
        error: "User info not found in passport response".to_string(),
    })
}

/// Download file from URL to local path
pub async fn download_file(client: &Client, url: &str, path: &std::path::Path) -> Result<(), RecorderError> {
    if url.is_empty() {
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent).map_err(|e| RecorderError::IoError(e))?;
        }
    }

    let response = client.get(url).send().await?;
    let bytes = response.bytes().await?;
    let mut file = tokio::fs::File::create(&path).await?;
    let mut content = std::io::Cursor::new(bytes);
    tokio::io::copy(&mut content, &mut file).await?;
    Ok(())
}
