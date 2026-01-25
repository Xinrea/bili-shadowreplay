use super::response::{RoomInfo as SigiRoomInfo, SigiStateResponse, StreamUrl as SigiStreamUrl};
use crate::account::Account;
use crate::errors::RecorderError;
use crate::utils::user_agent_generator;
use regex::Regex;
use reqwest::header::HeaderMap;
use reqwest::{Client, Url};
use serde::Deserialize;
use serde_json::{Map, Value};

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
    pub level: String,
    pub hls_url: Option<String>,
    pub rtmp_url: Option<String>,
}

fn generate_user_agent_header() -> reqwest::header::HeaderMap {
    let user_agent = user_agent_generator::UserAgentGenerator::new().generate(false);
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("user-agent", user_agent.parse().unwrap());
    headers
}

fn extract_script_json(html_str: &str, script_id: &str) -> Option<String> {
    let pattern = format!(r#"(?s)<script[^>]*id=['"]{script_id}['"][^>]*>(.*?)</script>"#);
    let regex = Regex::new(&pattern).ok()?;
    regex
        .captures(html_str)
        .and_then(|cap| cap.get(1))
        .map(|value| value.as_str().to_string())
}

fn parse_first_json_value(raw: &str) -> Option<Value> {
    let start = raw.find('{').or_else(|| raw.find('['))?;
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

/// Extract the state value from the HTML string
/// script_id is the ID of the script tag to extract the state value from
/// html_str is the HTML string to extract the state value from
/// Returns the state value if found, otherwise None
/// script_id can be "SIGI_STATE", "\_\_UNIVERSAL_DATA_FOR_REHYDRATION__", "\_\_NEXT_DATA__"
fn extract_state_value(script_id: &str, html_str: &str) -> Option<Value> {
    if let Some(json_str) = extract_script_json(html_str, script_id) {
        if let Some(parsed) = parse_first_json_value(&json_str) {
            return Some(parsed);
        }
    }

    if let Some(parsed) = extract_json_after_marker(html_str, script_id) {
        return Some(parsed);
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
    level: String,
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
    let data = match stream_data_value
        .get("data")
        .and_then(|value| value.as_object())
    {
        Some(value) => value,
        None => return Vec::new(),
    };

    let mut candidates = Vec::new();

    for (level, entry) in data.iter() {
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
            level: level.clone(),
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

async fn check_hls_stream_accessible(client: &Client, headers: &HeaderMap, m3u8_url: &str) -> bool {
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
    // prefer higher level
    // levels: origin, uhd, uhd60, ld, hd60, ao, hd, sd
    let levels = vec![
        "origin", "uhd", "uhd60", "ld", "hd60", "ao", "hd", "sd", "original",
    ];

    for level in levels {
        for candidate in candidates {
            if candidate.level == level {
                if let Some(hls_url) = candidate.hls_url.as_deref() {
                    if check_url_accessible(client, headers, hls_url).await {
                        return Some(StreamInfo {
                            level: candidate.level.clone(),
                            hls_url: candidate.hls_url.clone(),
                            rtmp_url: candidate.flv_url.clone(),
                        });
                    }
                }
                if let Some(flv_url) = candidate.flv_url.as_deref() {
                    if check_url_accessible(client, headers, flv_url).await {
                        return Some(StreamInfo {
                            level: candidate.level.clone(),
                            hls_url: None,
                            rtmp_url: Some(flv_url.to_string()),
                        });
                    }
                }
            }
        }
    }
    None
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
        _ => None,
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
                if let Ok(room_info) =
                    serde_json::from_value::<SigiRoomInfo>(room_info_value.clone())
                {
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
pub async fn get_room_info(
    client: &Client,
    account: &Account,
    room_id: &str,
) -> Result<RoomInfo, RecorderError> {
    let mut headers = generate_user_agent_header();
    headers.insert("Referer", "https://www.tiktok.com/".parse().unwrap());
    headers.insert("accept-language", "en-US,en;q=0.9".parse().unwrap());
    if !account.cookies.is_empty() {
        headers.insert("Cookie", account.cookies.parse().unwrap());
    }

    // TikTok URLs are typically like: https://www.tiktok.com/@username/live
    let url = format!("https://www.tiktok.com/{room_id}/live");

    // Retry up to 3 times
    for attempt in 0..3 {
        let response = client.get(&url).headers(headers.clone()).send().await?;
        let status = response.status();
        let html_str = response.text().await?;
        if !status.is_success() {
            return Err(RecorderError::ApiError {
                error: format!("TikTok response status: {}", status),
            });
        }

        // Check for region block
        if html_str.contains("We regret to inform you that we have discontinued operating TikTok") {
            return Err(RecorderError::ApiError {
                error: "TikTok is not available in this region. Please use a different proxy."
                    .to_string(),
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

        let sigi_value = match extract_state_value("SIGI_STATE", &html_str) {
            Some(value) => value,
            None => {
                if attempt < 2 {
                    log::debug!("Attempt {} failed to extract page state JSON", attempt);
                    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                    continue;
                }
                return Err(RecorderError::ApiError {
                    error: "Please check if your network can access TikTok normally. Failed to extract page state JSON.\n\nHTML: ".to_string() + &html_str,
                });
            }
        };

        let extracted_room_info = serde_json::from_value::<SigiStateResponse>(sigi_value.clone())
            .ok()
            .and_then(|state| state.live_room);

        if extracted_room_info.is_none() {
            log::warn!("Failed to extract room info from page data");
            return Ok(RoomInfo {
                live_status: false,
                room_title: "TikTok Live Not Started".to_string(),
                room_cover_url: "".to_string(),
                user_id: "".to_string(),
                user_name: "TikTok Live Not Started".to_string(),
                user_avatar: "".to_string(),
            });
        }

        log::debug!("Room info: {:?}", extracted_room_info);

        if let Some(room_info) = extracted_room_info {
            let user_id = room_info.live_room_user_info.user.id.clone();
            let user_name = room_info.live_room_user_info.user.nickname.clone();
            let mut user_avatar = room_info.live_room_user_info.user.avatar_thumb.clone();
            if user_avatar.is_empty() {
                if let Some(found) = find_avatar_url(&sigi_value) {
                    user_avatar = found;
                }
            }
            let mut room_cover_url = find_cover_url(&sigi_value).unwrap_or_default();
            if room_cover_url.is_empty() {
                room_cover_url = user_avatar.clone();
            }

            return Ok(RoomInfo {
                live_status: true,
                room_title: room_info.live_room_user_info.live_room.title.clone(),
                room_cover_url,
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
    room_id: &str,
) -> Result<StreamInfo, RecorderError> {
    let mut headers = generate_user_agent_header();
    headers.insert("Referer", "https://www.tiktok.com/".parse().unwrap());
    headers.insert("accept-language", "en-US,en;q=0.9".parse().unwrap());
    if !account.cookies.is_empty() {
        headers.insert("Cookie", account.cookies.parse().unwrap());
    }

    // TikTok URLs are typically like: https://www.tiktok.com/@username/live
    let url = format!("https://www.tiktok.com/{room_id}/live");

    // Retry up to 3 times
    for attempt in 0..3 {
        let response = client.get(&url).headers(headers.clone()).send().await?;
        let status = response.status();
        let html_str = response.text().await?;
        if !status.is_success() {
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

        let sigi_value = match extract_state_value("SIGI_STATE", &html_str) {
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
            if let Some(stream_info) = select_accessible_stream(client, &headers, &candidates).await
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
                level: "".to_string(),
                hls_url: stream_url.hls_pull_url,
                rtmp_url: stream_url.rtmp_pull_url,
            };
            if let Some(hls_url) = info.hls_url.as_deref() {
                if !check_hls_stream_accessible(client, &headers, hls_url).await {
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

        if let Some(m3u8_url) =
            find_m3u8_in_value(&sigi_value).or_else(|| extract_m3u8_from_html(&html_str))
        {
            return Ok(StreamInfo {
                level: "".to_string(),
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
    let mut headers = generate_user_agent_header();
    headers.insert("Referer", "https://www.tiktok.com/".parse().unwrap());
    headers.insert("accept-language", "en-US,en;q=0.9".parse().unwrap());
    if !account.cookies.is_empty() {
        headers.insert("Cookie", account.cookies.parse().unwrap());
    }

    // Access TikTok homepage to get user info from state
    let response = client
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
            error: "TikTok is not available in this region. Please use a different proxy."
                .to_string(),
        });
    }

    let state = extract_state_value("__UNIVERSAL_DATA_FOR_REHYDRATION__", &html_str).ok_or(RecorderError::ApiError {
        error: "Failed to extract TikTok state - please check if your network can access TikTok normally (proxy might be needed).".to_string(),
    })?;

    log::debug!("TikTok user state: {:?}", state);

    // Try to find current user in SigiState
    if let Some(user_info) = find_current_user_info(&state) {
        return Ok(user_info);
    }

    // Fallback: extract from specific path if known
    // SIGI_STATE -> UserModule -> users -> [username]
    if let Some(user_module) = state.get("UserModule").and_then(|u| u.get("users")) {
        if let Some(obj) = user_module.as_object() {
            if let Some((_, user_val)) = obj.iter().next() {
                let user_id = get_string_field(
                    user_val.as_object().unwrap_or(&Map::new()),
                    &["id", "secUid"],
                )
                .unwrap_or_default();
                let user_name = get_string_field(
                    user_val.as_object().unwrap_or(&Map::new()),
                    &["nickname", "uniqueId"],
                )
                .unwrap_or_default();
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
    log::debug!("Find current user info in SIGI_STATE");
    if let Some(user) = value
        .get("AppContext")
        .and_then(|a| a.get("appContext"))
        .and_then(|c| c.get("user"))
    {
        let user_id = get_string_field(user.as_object().unwrap_or(&Map::new()), &["uid", "id"])
            .unwrap_or_default();
        let user_name = get_string_field(
            user.as_object().unwrap_or(&Map::new()),
            &["nickname", "uniqueId"],
        )
        .unwrap_or_default();
        let user_avatar = find_avatar_url(user).unwrap_or_default();

        if !user_id.is_empty() {
            return Some(crate::UserInfo {
                user_id: user_id.clone(),
                user_name: if user_name.is_empty() {
                    user_id.clone()
                } else {
                    user_name
                },
                user_avatar,
            });
        }
    }

    // 2. In __UNIVERSAL_DATA_FOR_REHYDRATION__, it's under webapp.user-info
    // {
    //     "__DEFAULT_SCOPE__": {
    //         "webapp.app-context": {
    //             "language": "zh-Hans",
    //             "region": "US",
    //             "appId": 1233,
    //             "appType": "m",
    //             "user": {
    //                 "ftcUser": false,
    //                 "secUid": "MS4wLjABAAAAMw4UwoPSxm7vPq3JnxBtUaYYGHb9_Wxad5s5H_kPHk1vlxbbfezo3xCH4R5pK7Ha",
    //                 "uid": "7598891269284693006",
    //                 "nickName": "hanhuang14",
    //                 "signature": "",
    //                 "uniqueId": "hanhuang14",
    //                 "createTime": "0",
    //                 "hasLivePermission": true,
    //                 "roomId": "",
    //                 "region": "US",
    //                 "avatarUri": [
    //                     "https:\u002F\u002Fp19-common-sign.tiktokcdn-us.com\u002Ftos-useast5-avt-0068-tx\u002Fac9296dcd7310f92b506799d947787e8~tplv-tiktokx-cropcenter:720:720.jpeg?dr=9640&refresh_token=e699f96d&x-expires=1769443200&x-signature=RdAZWdoaiJjJ9nGM48hZLg67ld0%3D&t=4d5b0474&ps=13740610&shp=a5d48078&shcp=81f88b70&idc=useast5",
    //                     "https:\u002F\u002Fp16-common-sign.tiktokcdn-us.com\u002Ftos-useast5-avt-0068-tx\u002Fac9296dcd7310f92b506799d947787e8~tplv-tiktokx-cropcenter:720:720.jpeg?dr=9640&refresh_token=2fcf6eff&x-expires=1769443200&x-signature=AqDvsppKcnf02CEtE%2Fsek7%2BiIcA%3D&t=4d5b0474&ps=13740610&shp=a5d48078&shcp=81f88b70&idc=useast5"
    //                 ],
    log::debug!("Find current user info in __UNIVERSAL_DATA_FOR_REHYDRATION__");
    if let Some(obj) = value.as_object() {
        for (key, val) in obj.iter() {
            log::debug!(
                "Find current user info in __UNIVERSAL_DATA_FOR_REHYDRATION__: {:?}",
                key
            );
            // NOTE:
            // In practice `user` is usually NOT directly under `__DEFAULT_SCOPE__`,
            // but under something like `webapp.app-context.user` / `webapp.user-info.user`.
            let mut candidates: Vec<&Value> = Vec::new();
            candidates.push(val);
            if let Some(inner_obj) = val.as_object() {
                for (_inner_key, inner_val) in inner_obj.iter() {
                    candidates.push(inner_val);
                }
            }

            for candidate in candidates {
                if let Some(user) = candidate.get("user") {
                    log::debug!(
                        "Find current user info in __UNIVERSAL_DATA_FOR_REHYDRDRATION__: {:?}",
                        user
                    );

                    let user_id =
                        get_string_field(user.as_object().unwrap_or(&Map::new()), &["secUid"])
                            .unwrap_or_default();
                    let user_name =
                        get_string_field(user.as_object().unwrap_or(&Map::new()), &["nickName"])
                            .unwrap_or_default();
                    let user_avatar = find_avatar_url(user).unwrap_or_default();

                    if !user_id.is_empty() {
                        return Some(crate::UserInfo {
                            user_id: user_id.clone(),
                            user_name: if user_name.is_empty() {
                                user_id.clone()
                            } else {
                                user_name
                            },
                            user_avatar,
                        });
                    }
                }
            }
        }
    }

    None
}

/// Download file from URL to local path
pub async fn download_file(
    client: &Client,
    url: &str,
    path: &std::path::Path,
) -> Result<(), RecorderError> {
    if url.is_empty() {
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent).map_err(RecorderError::IoError)?;
        }
    }

    let response = client.get(url).send().await?;
    let bytes = response.bytes().await?;
    let mut file = tokio::fs::File::create(&path).await?;
    let mut content = std::io::Cursor::new(bytes);
    tokio::io::copy(&mut content, &mut file).await?;
    Ok(())
}
