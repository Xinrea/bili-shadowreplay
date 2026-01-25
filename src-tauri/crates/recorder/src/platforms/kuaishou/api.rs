use super::response::LiveStreamResponse;
use crate::errors::RecorderError;
use crate::{account::Account, utils::user_agent_generator};
use regex::Regex;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Map, Value};

fn generate_user_agent_header() -> reqwest::header::HeaderMap {
    let user_agent = user_agent_generator::UserAgentGenerator::new().generate(false);
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("user-agent", user_agent.parse().unwrap());
    headers
}

fn is_rate_limit_message(message: &str) -> bool {
    let trimmed = message.trim();
    !trimmed.is_empty()
        && (trimmed.contains("\u{64cd}\u{4f5c}\u{592a}\u{5feb}")
            || trimmed.contains("\u{8bbf}\u{95ee}\u{8fc7}\u{4e8e}\u{9891}\u{7e41}")
            || trimmed.contains("\u{8bbf}\u{95ee}\u{9891}\u{7e41}")
            || trimmed.contains("\u{8bf7}\u{7a0d}\u{540e}\u{518d}\u{8bd5}"))
}

pub fn is_rate_limited_error(error: &RecorderError) -> bool {
    match error {
        RecorderError::ApiError { error } => is_rate_limit_message(error),
        _ => false,
    }
}

fn decode_json_string(raw: &str) -> Option<String> {
    serde_json::from_str::<String>(&format!("\"{raw}\""))
        .ok()
        .or_else(|| {
            let decoded = raw
                .replace("\\u002F", "/")
                .replace("\\u0026", "&")
                .replace("\\u003D", "=");
            if decoded == raw {
                None
            } else {
                Some(decoded)
            }
        })
}

fn extract_hls_play_url(html_str: &str) -> Option<String> {
    let regex = Regex::new(r#""hlsPlayUrl"\s*:\s*"([^"]+)""#).ok()?;
    let raw = regex.captures(html_str)?.get(1)?.as_str();
    let decoded = decode_json_string(raw)?;
    if decoded.contains(".m3u8") {
        Some(decoded)
    } else {
        None
    }
}

fn extract_initial_state(html_str: &str) -> Option<String> {
    let patterns = [
        r#"(?s)window\.__INITIAL_STATE__\s*=\s*(\{.*?\});\s*\(function"#,
        r#"(?s)window\.__INITIAL_STATE__\s*=\s*(\{.*?\})\s*;\s*</script>"#,
        r#"(?s)window\['__INITIAL_STATE__'\]\s*=\s*(\{.*?\})\s*;\s*</script>"#,
        r#"(?s)window\.__INITIAL_STATE__\s*=\s*(\{.*?\})\s*;\s*window\.__"#,
        r#"(?s)__INITIAL_STATE__\s*=\s*(\{.*?\})\s*;\s*\(function"#,
        r#"(?s)__INITIAL_STATE__\s*=\s*(\{.*?\})\s*;\s*<"#,
    ];

    for (i, pattern) in patterns.iter().enumerate() {
        if let Ok(regex) = Regex::new(pattern) {
            if let Some(captures) = regex.captures(html_str) {
                if let Some(value) = captures.get(1) {
                    let json_str = value.as_str().trim();
                    log::debug!(
                        "[Kuaishou] Extracted __INITIAL_STATE__ using pattern #{}",
                        i
                    );
                    return Some(json_str.to_string());
                }
            }
        }
    }

    log::warn!(
        "[Kuaishou] Failed to extract __INITIAL_STATE__ with any pattern, HTML size: {} bytes",
        html_str.len()
    );
    // Log a snippet of the HTML for debugging (first 500 chars)
    let snippet_len = html_str.len().min(500);
    if snippet_len > 0 {
        log::debug!("[Kuaishou] HTML snippet: {}...", &html_str[..snippet_len]);
    }

    None
}

fn extract_metadata_from_html(html_str: &str) -> (Option<String>, Option<String>, Option<String>) {
    let mut title = None;
    let mut cover = None;
    let mut avatar = None;

    // Try finding title
    let title_patterns = [
        r#"<title>(.*?)</title>"#,
        r#""caption"\s*:\s*"([^"]+)""#,
        r#""title"\s*:\s*"([^"]+)""#,
    ];
    for p in title_patterns {
        if let Some(m) = Regex::new(p)
            .ok()
            .and_then(|re| re.captures(html_str))
            .and_then(|c| c.get(1))
        {
            let t = m.as_str().replace(" - 蹇墜鐩存挱", "").trim().to_string();
            if !t.is_empty() {
                title = Some(t);
                break;
            }
        }
    }

    // Try finding avatar
    let avatar_patterns = [
        r#""headUrl"\s*:\s*"([^"]+)""#,
        r#""avatar"\s*:\s*"([^"]+)""#,
        r#""avatarUrl"\s*:\s*"([^"]+)""#,
    ];
    for p in avatar_patterns {
        if let Some(m) = Regex::new(p)
            .ok()
            .and_then(|re| re.captures(html_str))
            .and_then(|c| c.get(1))
        {
            if let Some(decoded) = decode_json_string(m.as_str()) {
                avatar = Some(normalize_image_url(&decoded));
                break;
            }
        }
    }

    // Try finding cover
    let cover_patterns = [
        r#""poster"\s*:\s*"([^"]+)""#,
        r#""coverUrl"\s*:\s*"([^"]+)""#,
        r#""cover"\s*:\s*"([^"]+)""#,
        r#""snapshot"\s*:\s*"([^"]+)""#,
    ];
    for p in cover_patterns {
        if let Some(m) = Regex::new(p)
            .ok()
            .and_then(|re| re.captures(html_str))
            .and_then(|c| c.get(1))
        {
            if let Some(decoded) = decode_json_string(m.as_str()) {
                cover = Some(normalize_image_url(&decoded));
                break;
            }
        }
    }

    (title, cover, avatar)
}

fn find_live_stream_response(value: &Value) -> Option<LiveStreamResponse> {
    match value {
        Value::Object(map) => {
            if map.contains_key("liveStream") || map.contains_key("live_stream") {
                let mut cloned = map.clone();
                if !cloned.contains_key("liveStream") {
                    if let Some(v) = cloned.remove("live_stream") {
                        cloned.insert("liveStream".to_string(), v);
                    }
                }
                if let Ok(response) =
                    serde_json::from_value::<LiveStreamResponse>(Value::Object(cloned))
                {
                    // Check if this looks like a valid response with metadata
                    // Prioritize ones that have author info
                    if (response.live_stream.is_some() && response.author.is_some())
                        || response.error_type.is_some()
                    {
                        return Some(response);
                    }
                }
            }

            for child in map.values() {
                if let Some(response) = find_live_stream_response(child) {
                    return Some(response);
                }
            }

            None
        }
        Value::Array(values) => {
            for value in values {
                if let Some(response) = find_live_stream_response(value) {
                    return Some(response);
                }
            }
            None
        }
        _ => None,
    }
}

fn parse_live_stream_response(json_str: &str) -> Result<LiveStreamResponse, RecorderError> {
    let livestream_regex = Regex::new(r#"(?s)(\{"liveStream".*?),"gameInfo"#).map_err(|e| {
        RecorderError::ApiError {
            error: format!("Failed to create regex: {}", e),
        }
    })?;

    if let Some(cap) = livestream_regex
        .captures(json_str)
        .and_then(|cap| cap.get(1))
    {
        let full_json = format!("{}}}", cap.as_str());
        if let Ok(response) = serde_json::from_str::<LiveStreamResponse>(&full_json) {
            return Ok(response);
        }
    }

    let state: Value = serde_json::from_str(json_str).map_err(|e| RecorderError::ApiError {
        error: format!("Failed to parse JSON: {}", e),
    })?;

    find_live_stream_response(&state).ok_or(RecorderError::ApiError {
        error: "Failed to extract liveStream data".to_string(),
    })
}

fn normalize_cookie_header(cookies: &str) -> String {
    cookies
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("; ")
}

fn extract_user_id_from_url(url: &str) -> String {
    let url_no_fragment = url.split('#').next().unwrap_or(url);
    let url_no_query = url_no_fragment.split('?').next().unwrap_or(url_no_fragment);
    let trimmed = url_no_query.trim_end_matches('/');

    if let Some(pos) = trimmed.find("/u/") {
        return trimmed[(pos + 3)..].to_string();
    }
    if let Some(pos) = trimmed.find("/profile/") {
        return trimmed[(pos + 9)..].to_string();
    }

    if trimmed.contains("kuaishou.com") {
        if let Some(last) = trimmed.rsplit('/').next() {
            return last.to_string();
        }
    }

    String::new()
}

fn build_web_candidate_urls(url: &str) -> Vec<String> {
    let mut urls = Vec::new();
    if !url.is_empty() {
        urls.push(url.to_string());
    }

    if url.contains("live.kuaishou.com") {
        let www_url = url.replace("live.kuaishou.com", "www.kuaishou.com");
        if !urls.contains(&www_url) {
            urls.push(www_url);
        }
    } else if url.contains("www.kuaishou.com") {
        let live_url = url.replace("www.kuaishou.com", "live.kuaishou.com");
        if !urls.contains(&live_url) {
            urls.push(live_url);
        }
    }

    let eid = extract_user_id_from_url(url);
    if !eid.is_empty() {
        let candidates = [
            format!("https://live.kuaishou.com/u/{eid}"),
            format!("https://www.kuaishou.com/u/{eid}"),
            format!("https://www.kuaishou.com/live/u/{eid}"),
        ];
        for candidate in candidates {
            if !urls.contains(&candidate) {
                urls.push(candidate);
            }
        }
    }

    urls
}

async fn fetch_web_html(
    client: &Client,
    account: &Account,
    url: &str,
) -> Result<(String, reqwest::Url), RecorderError> {
    let mut headers = generate_user_agent_header();
    headers.insert(
        "Accept",
        "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"
            .parse()
            .unwrap(),
    );
    headers.insert(
        "Accept-Language",
        "zh-CN,zh;q=0.8,zh-TW;q=0.7,zh-HK;q=0.5,en-US;q=0.3,en;q=0.2"
            .parse()
            .unwrap(),
    );

    if !account.cookies.is_empty() {
        let cookie = normalize_cookie_header(&account.cookies);
        if !cookie.is_empty() {
            headers.insert("Cookie", cookie.parse().unwrap());
        }
    }

    let mut last_error: Option<RecorderError> = None;
    let mut last_small: Option<(String, reqwest::Url)> = None;

    for candidate in build_web_candidate_urls(url) {
        let referer = if candidate.contains("www.kuaishou.com") {
            "https://www.kuaishou.com/"
        } else {
            "https://live.kuaishou.com/"
        };
        let mut req_headers = headers.clone();
        req_headers.insert("Referer", referer.parse().unwrap());

        let response = client.get(&candidate).headers(req_headers).send().await?;
        let status = response.status();
        let final_url = response.url().clone();
        let html_str = response.text().await?;

        if !status.is_success() {
            log::warn!(
                "[Kuaishou] Web response status: {}, url: {}",
                status,
                final_url
            );
            let snippet_len = html_str.len().min(200);
            if snippet_len > 0 {
                log::debug!(
                    "[Kuaishou] Web error snippet: {}...",
                    &html_str[..snippet_len]
                );
            }
            last_error = Some(RecorderError::ApiError {
                error: format!("Kuaishou web status: {}", status),
            });
            continue;
        }

        if html_str.len() <= 256 {
            log::warn!(
                "[Kuaishou] Web response small ({} bytes), url: {}",
                html_str.len(),
                final_url
            );
            let snippet_len = html_str.len().min(200);
            if snippet_len > 0 {
                log::debug!(
                    "[Kuaishou] Web small snippet: {}...",
                    &html_str[..snippet_len]
                );
            }
            last_small = Some((html_str, final_url));
            continue;
        }

        return Ok((html_str, final_url));
    }

    if let Some((html, final_url)) = last_small {
        return Ok((html, final_url));
    }

    Err(last_error.unwrap_or_else(|| RecorderError::ApiError {
        error: "Failed to fetch Kuaishou web page".to_string(),
    }))
}

fn normalize_image_url(url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.starts_with("//") {
        format!("https:{}", trimmed)
    } else {
        trimmed.to_string()
    }
}

fn extract_image_url(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Object(map) => {
            if let Some(url) = map.get("url").and_then(|v| v.as_str()) {
                Some(url.to_string())
            } else if let Some(list) = map
                .get("urlList")
                .or_else(|| map.get("url_list"))
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

fn find_image_url(value: &Value, keys: &[&str]) -> Option<String> {
    match value {
        Value::Object(map) => {
            for key in keys {
                if let Some(value) = map.get(*key) {
                    if let Some(url) = extract_image_url(value) {
                        return Some(normalize_image_url(&url));
                    }
                }
            }

            for (key, value) in map {
                if key.to_ascii_lowercase().contains("cover")
                    || key.to_ascii_lowercase().contains("avatar")
                {
                    if let Some(url) = extract_image_url(value) {
                        return Some(normalize_image_url(&url));
                    }
                }
            }

            for child in map.values() {
                if let Some(url) = find_image_url(child, keys) {
                    return Some(url);
                }
            }

            None
        }
        Value::Array(values) => {
            for value in values {
                if let Some(url) = find_image_url(value, keys) {
                    return Some(url);
                }
            }
            None
        }
        _ => None,
    }
}

fn quality_rank(label: &str) -> i64 {
    let lower = label.to_ascii_lowercase();
    if lower.contains("4k") || lower.contains("2160") || lower.contains("uhd") {
        return 4000;
    }
    if lower.contains("2k") || lower.contains("1440") || lower.contains("qhd") {
        return 2000;
    }
    if lower.contains("1080") || lower.contains("fhd") {
        return 1080;
    }
    if lower.contains("720") || lower.contains("hd") {
        return 720;
    }
    if lower.contains("480") || lower.contains("sd") {
        return 480;
    }
    if lower.contains("360") || lower.contains("ld") {
        return 360;
    }
    if lower.contains("original") || lower.contains("source") {
        return 3000;
    }
    let mut digits = String::new();
    for ch in lower.chars() {
        if ch.is_ascii_digit() {
            digits.push(ch);
        } else if !digits.is_empty() {
            break;
        }
    }
    digits.parse::<i64>().unwrap_or(0)
}

/// QR code information for login
#[derive(Debug, Clone, serde::Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QrInfo {
    pub qr_login_token: String,
    pub qr_login_signature: String,
    pub image_data: String,
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
    pub url: String,
    pub quality: String,
    pub bitrate: Option<i64>,
}

/// QR code status for polling
#[derive(Debug, Clone, serde::Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QrStatus {
    pub code: u8,
    pub cookies: String,
}

/// Get room information from web page
pub async fn get_room_info(
    client: &Client,
    account: &Account,
    url: &str,
) -> Result<RoomInfo, RecorderError> {
    let (html_str, _final_url) = match fetch_web_html(client, account, url).await {
        Ok(result) => result,
        Err(e) => {
            return Err(e);
        }
    };
    if is_rate_limit_message(&html_str) {
        return Err(RecorderError::ApiError {
            error: "Rate limited by Kuaishou web page".to_string(),
        });
    }
    let has_fallback_stream = extract_hls_play_url(&html_str).is_some();
    let fallback_user_id = if account.id.is_empty() {
        extract_user_id_from_url(url)
    } else {
        account.id.clone()
    };
    let (html_title, html_cover, html_avatar) = extract_metadata_from_html(&html_str);

    let fallback_user_name = if account.name.is_empty() {
        html_title
            .clone()
            .unwrap_or_else(|| "Kuaishou Live".to_string())
    } else {
        account.name.clone()
    };

    let fallback_room_info = RoomInfo {
        live_status: has_fallback_stream,
        room_title: html_title
            .clone()
            .unwrap_or_else(|| format!("{}'s live", fallback_user_name)),
        room_cover_url: html_cover.clone().unwrap_or_default(),
        user_id: fallback_user_id,
        user_name: fallback_user_name,
        user_avatar: html_avatar.clone().unwrap_or_default(),
    };

    // Parse JSON from script tag
    let json_str = match extract_initial_state(&html_str) {
        Some(json_str) => json_str,
        None => {
            if has_fallback_stream {
                return Ok(fallback_room_info.clone());
            }
            return Err(RecorderError::ApiError {
                error: "Failed to extract JSON data from page".to_string(),
            });
        }
    };

    let live_data = match parse_live_stream_response(&json_str) {
        Ok(live_data) => live_data,
        Err(e) => {
            if has_fallback_stream {
                return Ok(fallback_room_info.clone());
            }
            return Err(e);
        }
    };

    // Check for errors
    if let Some(error) = live_data.error_type {
        return Err(RecorderError::ApiError {
            error: format!("{}: {}", error.title, error.content),
        });
    }

    let live_stream = live_data.live_stream.ok_or(RecorderError::ApiError {
        error: "No liveStream found in response".to_string(),
    })?;

    let author = live_data.author.unwrap_or_default();
    let author_name = if author.name.is_empty() {
        if account.name.is_empty() {
            "Kuaishou Live".to_string()
        } else {
            account.name.clone()
        }
    } else {
        author.name.clone()
    };
    let author_id = if author.id.is_empty() {
        account.id.clone()
    } else {
        author.id.clone()
    };
    let author_avatar = author
        .head_url
        .clone()
        .map(|url| normalize_image_url(&url))
        .filter(|url| !url.is_empty())
        .or_else(|| {
            if let Ok(value) = serde_json::from_str::<Value>(&json_str) {
                find_image_url(
                    &value,
                    &[
                        "headurl",
                        "headUrl",
                        "avatar",
                        "avatarUrl",
                        "portrait",
                        "profilePic",
                        "avatarThumb",
                    ],
                )
            } else {
                None
            }
        })
        .unwrap_or_default();

    let is_live = live_stream.play_urls.is_some()
        && live_stream
            .play_urls
            .as_ref()
            .and_then(|p| p.h264.as_ref())
            .and_then(|h| h.adaptation_set.as_ref())
            .map(|a| !a.representation.is_empty())
            .unwrap_or(false);

    let cover_url = live_stream
        .cover_url
        .clone()
        .map(|url| normalize_image_url(&url))
        .filter(|url| !url.is_empty());

    let room_cover_url = if let Some(url) = cover_url {
        url
    } else {
        // Try finding cover recursively, BUT explicitly exclude avatar-like keys first
        if let Ok(value) = serde_json::from_str::<Value>(&json_str) {
            find_image_url(&value, &["cover", "coverUrl", "poster", "image"])
                .or_else(|| {
                    // Try regex fallback for poster/cover patterns in HTML
                    let patterns = [
                        r#""poster"\s*:\s*"([^"]+)""#,
                        r#""coverUrl"\s*:\s*"([^"]+)""#,
                        r#""cover"\s*:\s*"([^"]+)""#,
                    ];
                    for pattern in patterns {
                        if let Ok(re) = Regex::new(pattern) {
                            if let Some(cap) = re.captures(&json_str) {
                                if let Some(m) = cap.get(1) {
                                    return decode_json_string(m.as_str());
                                }
                            }
                        }
                    }
                    None
                })
                .unwrap_or_else(|| author_avatar.clone())
        } else {
            author_avatar.clone()
        }
    };

    let final_title = live_stream
        .caption
        .clone()
        .or_else(|| live_data.config.and_then(|c| c.caption))
        .filter(|s| !s.is_empty())
        .or(html_title)
        .unwrap_or_else(|| format!("{}'s live", author_name));

    let final_cover = if room_cover_url.is_empty() {
        html_cover.unwrap_or_default()
    } else {
        room_cover_url
    };

    let final_avatar = if author_avatar.is_empty() {
        html_avatar.unwrap_or_default()
    } else {
        author_avatar
    };

    Ok(RoomInfo {
        live_status: is_live,
        room_title: final_title,
        room_cover_url: final_cover,
        user_id: author_id,
        user_name: author_name,
        user_avatar: final_avatar,
    })
}

/// Get stream URLs from Kuaishou web page
pub async fn get_stream_urls(
    client: &Client,
    account: &Account,
    url: &str,
) -> Result<Vec<StreamInfo>, RecorderError> {
    // Prefer web first to reduce mobile rate limits; only try mobile if no m3u8 is found.
    let (html_str, _final_url) = match fetch_web_html(client, account, url).await {
        Ok(result) => result,
        Err(e) => {
            if is_rate_limited_error(&e) {
                log::info!("[Kuaishou] Web page rate limited, skipping web page for now");
            }
            return Err(e);
        }
    };
    let fallback_hls = extract_hls_play_url(&html_str).map(|hls_url| StreamInfo {
        url: hls_url,
        quality: "hls".to_string(),
        bitrate: None,
    });
    let mut urls = Vec::new();

    let json_str = match extract_initial_state(&html_str) {
        Some(json_str) => json_str,
        None => {
            if let Some(fallback) = fallback_hls.clone() {
                urls.push(fallback);
            }
            if !urls.is_empty() {
                return Ok(urls);
            }
            return Err(RecorderError::ApiError {
                error: "Failed to extract JSON data from page".to_string(),
            });
        }
    };

    let live_data = match parse_live_stream_response(&json_str) {
        Ok(live_data) => live_data,
        Err(e) => {
            if let Some(fallback) = fallback_hls.clone() {
                urls.push(fallback);
            }
            if is_rate_limited_error(&e) {
                log::info!("[Kuaishou] Web page rate limited, skipping web page for now");
            }
            return Err(e);
        }
    };
    let play_urls = live_data
        .live_stream
        .as_ref()
        .unwrap()
        .play_urls
        .as_ref()
        .unwrap();

    let mut all_representations = Vec::new();

    if let Some(h264) = play_urls.h264.as_ref() {
        if let Some(set) = h264.adaptation_set.as_ref() {
            all_representations.extend(set.representation.clone());
        }
    }

    if let Some(h265) = play_urls.h265.as_ref() {
        if let Some(set) = h265.adaptation_set.as_ref() {
            all_representations.extend(set.representation.clone());
        }
    }

    if all_representations.is_empty() {
        if !urls.is_empty() {
            return Ok(urls);
        }
        return Err(RecorderError::ApiError {
            error: "No usable stream representations found".to_string(),
        });
    }

    all_representations.sort_by(|a, b| b.bitrate.unwrap_or(0).cmp(&a.bitrate.unwrap_or(0)));

    // Remove duplicates based on URL
    let mut seen_urls = std::collections::HashSet::new();

    urls.extend(all_representations.into_iter().filter_map(|rep| {
        if seen_urls.contains(&rep.url) {
            return None;
        }
        seen_urls.insert(rep.url.clone());
        Some(StreamInfo {
            url: rep.url,
            quality: rep.name.or(rep.quality_type).unwrap_or_default(),
            bitrate: rep.bitrate,
        })
    }));

    if !urls.iter().any(|stream| stream.url.contains(".m3u8")) {
        if let Some(fallback) = fallback_hls.clone() {
            urls.insert(0, fallback);
        }
    }

    urls.sort_by(|a, b| {
        let a_m3u8 = a.url.contains(".m3u8");
        let b_m3u8 = b.url.contains(".m3u8");
        b_m3u8
            .cmp(&a_m3u8)
            .then_with(|| b.bitrate.unwrap_or(0).cmp(&a.bitrate.unwrap_or(0)))
            .then_with(|| quality_rank(&b.quality).cmp(&quality_rank(&a.quality)))
    });

    if !urls.iter().any(|stream| stream.url.contains(".m3u8")) {
        if let Some(flv_url) = urls
            .iter()
            .find(|stream| stream.url.contains(".flv"))
            .map(|stream| stream.url.clone())
        {
            let guessed_hls = flv_url.replacen(".flv", ".m3u8", 1);
            if guessed_hls != flv_url {
                log::info!("[Kuaishou] No m3u8 found, guessing HLS from FLV URL");
                urls.insert(
                    0,
                    StreamInfo {
                        url: guessed_hls,
                        quality: "hls".to_string(),
                        bitrate: None,
                    },
                );
            }
        }
    }

    // Log available stream qualities for debugging
    if !urls.is_empty() {
        log::info!("[Kuaishou] Found {} stream(s):", urls.len());
        for (i, stream) in urls.iter().enumerate() {
            log::info!(
                "  [{}] Quality: {}, Bitrate: {}, Format: {}",
                i,
                if stream.quality.is_empty() {
                    "unknown"
                } else {
                    &stream.quality
                },
                stream
                    .bitrate
                    .map_or("unknown".to_string(), |b| format!("{} kbps", b)),
                if stream.url.contains(".m3u8") {
                    "HLS"
                } else if stream.url.contains(".flv") {
                    "FLV"
                } else {
                    "other"
                }
            );
        }
    } else {
        log::warn!("[Kuaishou] No streams found");
    }

    Ok(urls)
}

/// Get QR code for login
pub async fn get_qr(client: &Client) -> Result<QrInfo, RecorderError> {
    let mut headers = generate_user_agent_header();
    headers.insert("Content-Type", "application/json".parse().unwrap());
    headers.insert("Referer", "https://www.kuaishou.com/".parse().unwrap());

    let response = client
        .post("https://id.kuaishou.com/rest/c/infra/ks/qr/start")
        .headers(headers)
        .json(&json!({}))
        .send()
        .await?;

    let data: serde_json::Value = response.json().await?;

    Ok(QrInfo {
        qr_login_token: data["qrLoginToken"]
            .as_str()
            .ok_or(RecorderError::InvalidValue)?
            .to_string(),
        qr_login_signature: data["qrLoginSignature"]
            .as_str()
            .ok_or(RecorderError::InvalidValue)?
            .to_string(),
        image_data: data["imageData"]
            .as_str()
            .ok_or(RecorderError::InvalidValue)?
            .to_string(),
    })
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

fn user_info_from_map(map: &Map<String, Value>) -> Option<crate::UserInfo> {
    let user_id = get_string_field(map, &["user_id", "userId", "userIdStr", "uid"]);
    let user_name = get_string_field(map, &["user_name", "userName", "nickname", "nickName"]);
    if let (Some(user_id), Some(user_name)) = (user_id, user_name) {
        let user_avatar = get_string_field(
            map,
            &[
                "headurl",
                "headUrl",
                "avatar",
                "avatarUrl",
                "portrait",
                "profilePic",
            ],
        )
        .unwrap_or_default();
        return Some(crate::UserInfo {
            user_id,
            user_name,
            user_avatar,
        });
    }

    let user_id = get_string_field(map, &["id"]);
    let user_name = get_string_field(map, &["name"]);
    if let (Some(user_id), Some(user_name)) = (user_id, user_name) {
        let user_avatar = get_string_field(
            map,
            &[
                "headurl",
                "headUrl",
                "avatar",
                "avatarUrl",
                "portrait",
                "profilePic",
            ],
        )
        .unwrap_or_default();
        return Some(crate::UserInfo {
            user_id,
            user_name,
            user_avatar,
        });
    }

    None
}

fn find_user_info(value: &Value) -> Option<crate::UserInfo> {
    match value {
        Value::Object(map) => {
            if let Some(user_info) = user_info_from_map(map) {
                return Some(user_info);
            }
            for child in map.values() {
                if let Some(user_info) = find_user_info(child) {
                    return Some(user_info);
                }
            }
            None
        }
        Value::Array(values) => {
            for value in values {
                if let Some(user_info) = find_user_info(value) {
                    return Some(user_info);
                }
            }
            None
        }
        _ => None,
    }
}

/// Get user information from cookies
pub async fn get_user_info(
    client: &Client,
    account: &Account,
) -> Result<crate::UserInfo, RecorderError> {
    let mut headers = generate_user_agent_header();
    headers.insert(
        "Accept-Language",
        "zh-CN,zh;q=0.8,zh-TW;q=0.7,zh-HK;q=0.5,en-US;q=0.3,en;q=0.2"
            .parse()
            .unwrap(),
    );

    if !account.cookies.is_empty() {
        let cookie = normalize_cookie_header(&account.cookies);
        if !cookie.is_empty() {
            headers.insert("Cookie", cookie.parse().unwrap());
        }
    }

    // Access user's own profile page to get user info
    let response = client
        .get("https://live.kuaishou.com/")
        .headers(headers)
        .send()
        .await?;

    let html_str = response.text().await?;

    // Parse JSON from script tag
    let json_str = extract_initial_state(&html_str).ok_or(RecorderError::ApiError {
        error: "Failed to extract JSON data from page".to_string(),
    })?;

    if let Ok(state) = serde_json::from_str::<Value>(&json_str) {
        if let Some(user_info) = find_user_info(&state) {
            return Ok(user_info);
        }
    }

    #[derive(Deserialize)]
    struct KuaishouUser {
        #[serde(default, alias = "user_id", alias = "userId", alias = "id")]
        user_id: String,
        #[serde(default, alias = "user_name", alias = "userName", alias = "name")]
        user_name: String,
        #[serde(
            default,
            alias = "headurl",
            alias = "headUrl",
            alias = "avatar",
            alias = "avatarUrl"
        )]
        head_url: String,
    }

    // Extract user data with fallback regex if full JSON scan fails
    let user_regex = Regex::new(r#"(?s)"profile":\{"ownerCount".*?"user":(.*?),"currentWork"#)
        .map_err(|e| RecorderError::ApiError {
            error: format!("Failed to create user regex: {}", e),
        })?;

    let user_str = user_regex
        .captures(&json_str)
        .and_then(|cap| cap.get(1))
        .ok_or(RecorderError::ApiError {
            error: "Failed to extract user data - please check if cookies are valid".to_string(),
        })?
        .as_str();

    let user: KuaishouUser =
        serde_json::from_str(user_str).map_err(|e| RecorderError::ApiError {
            error: format!("Failed to parse user JSON: {}", e),
        })?;

    if user.user_id.is_empty() || user.user_name.is_empty() {
        return Err(RecorderError::ApiError {
            error: "Failed to extract user data - please check if cookies are valid".to_string(),
        });
    }

    Ok(crate::UserInfo {
        user_id: user.user_id,
        user_name: user.user_name,
        user_avatar: user.head_url,
    })
}

/// Poll QR code status and get cookies after successful login
pub async fn get_qr_status(
    client: &Client,
    qr_login_token: &str,
    qr_login_signature: &str,
) -> Result<QrStatus, RecorderError> {
    let mut headers = generate_user_agent_header();
    headers.insert("Content-Type", "application/json".parse().unwrap());
    headers.insert("Referer", "https://www.kuaishou.com/".parse().unwrap());

    let payload = json!({
        "qrLoginToken": qr_login_token,
        "qrLoginSignature": qr_login_signature,
    });

    // Step 1: Check scan status
    let scan_response = client
        .post("https://id.kuaishou.com/rest/c/infra/ks/qr/scanResult")
        .headers(headers.clone())
        .json(&payload)
        .send()
        .await?;

    let scan_data: serde_json::Value = scan_response.json().await?;

    // If not scanned yet, return pending status
    if scan_data["result"].as_u64().unwrap_or(1) != 1 {
        return Ok(QrStatus {
            code: 1,
            cookies: String::new(),
        });
    }

    // Step 2: Check accept status
    let accept_response = client
        .post("https://id.kuaishou.com/rest/c/infra/ks/qr/acceptResult")
        .headers(headers.clone())
        .json(&payload)
        .send()
        .await?;

    let accept_data: serde_json::Value = accept_response.json().await?;

    // If not accepted yet, return pending status
    if accept_data["result"].as_u64().unwrap_or(1) != 1 {
        return Ok(QrStatus {
            code: 2,
            cookies: String::new(),
        });
    }

    // Step 3: Get qrToken and perform callback
    let qr_token = accept_data["qrToken"]
        .as_str()
        .ok_or(RecorderError::InvalidValue)?;

    let callback_response = client
        .post("https://id.kuaishou.com/pass/kuaishou/login/qr/callback")
        .headers(headers.clone())
        .json(&json!({
            "qrToken": qr_token,
        }))
        .send()
        .await?;

    // Extract cookies from response headers
    let mut cookies_vec = Vec::new();
    if let Some(cookie_headers) = callback_response
        .headers()
        .get_all("set-cookie")
        .iter()
        .next()
    {
        if let Ok(cookie_str) = cookie_headers.to_str() {
            // Extract cookie name=value before the first semicolon
            if let Some(cookie_part) = cookie_str.split(';').next() {
                cookies_vec.push(cookie_part.to_string());
            }
        }
    }

    // Get all set-cookie headers
    for cookie_header in callback_response.headers().get_all("set-cookie") {
        if let Ok(cookie_str) = cookie_header.to_str() {
            if let Some(cookie_part) = cookie_str.split(';').next() {
                cookies_vec.push(cookie_part.to_string());
            }
        }
    }

    let cookies = cookies_vec.join("; ");

    if cookies.is_empty() {
        return Err(RecorderError::ApiError {
            error: "Failed to extract cookies from response".to_string(),
        });
    }

    Ok(QrStatus { code: 0, cookies })
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
