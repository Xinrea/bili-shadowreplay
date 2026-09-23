//! YouTube live-chat provider.
//!
//! YouTube does not expose live chat through a public websocket.  The web
//! client bootstraps an Innertube continuation in the watch page and polls
//! `live_chat/get_live_chat`; this provider follows that same protocol without
//! requiring an API key or a signed-in account.

use std::collections::{HashSet, VecDeque};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::{json, Map, Value};
use tokio::sync::mpsc;
use tokio::time::sleep;

use crate::provider::{DanmuMessageType, DanmuProvider};
use crate::{DanmuStreamError, LiveEvent};

const YOUTUBE_WATCH_URL: &str = "https://www.youtube.com/watch?v=";
const YOUTUBE_CHAT_URL: &str = "https://www.youtube.com/youtubei/v1/live_chat/get_live_chat";
const YOUTUBE_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const DEFAULT_CLIENT_VERSION: &str = "2.20250101.00.00";
const DEFAULT_POLL_MS: u64 = 1_000;
const MAX_POLL_MS: u64 = 60_000;
const MAX_SEEN_IDS: usize = 2_000;
const REBOOTSTRAP_AFTER_ERRORS: u32 = 3;
const MAX_RETRY_DELAY_SECS: u64 = 30;

const CONTINUATION_KEYS: [&str; 4] = [
    "timedContinuationData",
    "invalidationContinuationData",
    "reloadContinuationData",
    "liveChatReplayContinuationData",
];

const RENDERER_KEYS: [&str; 8] = [
    "liveChatTextMessageRenderer",
    "liveChatPaidMessageRenderer",
    "liveChatPaidStickerRenderer",
    "liveChatMembershipItemRenderer",
    "liveChatSponsorshipsGiftPurchaseAnnouncementRenderer",
    "liveChatSponsorshipsWelcomeMessageRenderer",
    "liveChatViewerEngagementMessageRenderer",
    "liveChatPlaceholderItemRenderer",
];

#[derive(Debug, Clone)]
struct Bootstrap {
    api_key: String,
    client_version: String,
    visitor_data: Option<String>,
    continuation: String,
}

#[derive(Debug, Clone)]
struct NextContinuation {
    token: String,
    timeout_ms: u64,
}

/// A YouTube chat provider is created for the current live video.  The
/// recorder resolves a channel/handle to that video before constructing it.
pub struct YoutubeDanmu {
    client: reqwest::Client,
    room_id: String,
    stop: Arc<AtomicBool>,
}

#[async_trait]
impl DanmuProvider for YoutubeDanmu {
    async fn new(cookie: &str, room_id: &str) -> Result<Self, DanmuStreamError> {
        let room_id = room_id.trim();
        if room_id.is_empty() {
            return Err(DanmuStreamError::InvalidIdentifier {
                err: "YouTube video id is empty".to_string(),
            });
        }

        let mut headers = HeaderMap::new();
        headers.insert("User-Agent", HeaderValue::from_static(YOUTUBE_USER_AGENT));
        headers.insert(
            "Accept-Language",
            HeaderValue::from_static("en-US,en;q=0.8"),
        );
        if !cookie.trim().is_empty() {
            let value =
                HeaderValue::from_str(cookie).map_err(|_| DanmuStreamError::InvalidIdentifier {
                    err: "invalid YouTube cookie header".to_string(),
                })?;
            headers.insert("Cookie", value);
        }

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(20))
            .build()?;

        Ok(Self {
            client,
            room_id: room_id.to_string(),
            stop: Arc::new(AtomicBool::new(false)),
        })
    }

    async fn start(
        &self,
        tx: mpsc::UnboundedSender<DanmuMessageType>,
    ) -> Result<(), DanmuStreamError> {
        let mut bootstrap = None;
        let mut continuation = String::new();
        let mut should_bootstrap = true;
        let mut failures = 0u32;
        let mut seen = DedupCache::default();

        loop {
            if self.stop.load(Ordering::Acquire) {
                return Ok(());
            }

            if should_bootstrap {
                if failures > 0 && sleep_or_stop(&self.stop, retry_delay(failures)).await {
                    return Ok(());
                }
                match fetch_bootstrap(&self.client, &self.room_id).await {
                    Ok(next_bootstrap) => {
                        continuation = next_bootstrap.continuation.clone();
                        bootstrap = Some(next_bootstrap);
                        failures = 0;
                        should_bootstrap = false;
                    }
                    Err(error) => {
                        failures = failures.saturating_add(1);
                        log::warn!(
                            "[YouTube][{}] Failed to refresh live-chat continuation: {}",
                            self.room_id,
                            error
                        );
                        continue;
                    }
                }
            }

            let current_bootstrap = bootstrap
                .as_ref()
                .expect("bootstrap is set before polling live chat");
            let response = match fetch_chat(&self.client, current_bootstrap, &continuation).await {
                Ok(response) => {
                    failures = 0;
                    response
                }
                Err(error) => {
                    failures = failures.saturating_add(1);
                    log::warn!(
                        "[YouTube][{}] Live-chat poll failed (attempt {}): {}",
                        self.room_id,
                        failures,
                        error
                    );
                    if should_rebootstrap(failures) {
                        should_bootstrap = true;
                    } else if sleep_or_stop(&self.stop, retry_delay(failures)).await {
                        return Ok(());
                    }
                    continue;
                }
            };

            let mut renderers = Vec::new();
            collect_renderers(&response, &mut renderers);
            for (kind, renderer) in renderers {
                let id = renderer
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty());
                if let Some(id) = id {
                    if !seen.insert(id) {
                        continue;
                    }
                }

                if let Some(event) = normalize_renderer(kind, renderer, &self.room_id) {
                    tx.send(DanmuMessageType::Event(event)).map_err(|_| {
                        DanmuStreamError::WebsocketError {
                            err: "YouTube danmu receiver closed".to_string(),
                        }
                    })?;
                }
            }

            let Some(next) = find_continuation(&response) else {
                // Continuations can expire or disappear during a live. Refresh
                // the watch-page bootstrap instead of silently ending chat.
                log::warn!(
                    "[YouTube][{}] Live-chat response had no continuation; refreshing bootstrap",
                    self.room_id
                );
                failures = REBOOTSTRAP_AFTER_ERRORS;
                should_bootstrap = true;
                continue;
            };
            continuation = next.token;

            if sleep_or_stop(
                &self.stop,
                Duration::from_millis(next.timeout_ms.clamp(250, MAX_POLL_MS)),
            )
            .await
            {
                return Ok(());
            }
        }
    }

    async fn stop(&self) -> Result<(), DanmuStreamError> {
        self.stop.store(true, Ordering::Release);
        Ok(())
    }
}

async fn wait_for_stop(stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::Acquire) {
        sleep(Duration::from_millis(100)).await;
    }
}

async fn sleep_or_stop(stop: &Arc<AtomicBool>, delay: Duration) -> bool {
    tokio::select! {
        _ = sleep(delay) => false,
        _ = wait_for_stop(Arc::clone(stop)) => true,
    }
}

fn retry_delay(failures: u32) -> Duration {
    let exponent = failures.saturating_sub(1).min(5);
    Duration::from_secs((1u64 << exponent).min(MAX_RETRY_DELAY_SECS))
}

fn should_rebootstrap(failures: u32) -> bool {
    failures >= REBOOTSTRAP_AFTER_ERRORS
}

async fn fetch_bootstrap(
    client: &reqwest::Client,
    video_id: &str,
) -> Result<Bootstrap, DanmuStreamError> {
    let response = client
        .get(format!("{YOUTUBE_WATCH_URL}{video_id}"))
        .send()
        .await?
        .error_for_status()?;
    let html = response.text().await?;

    let config = extract_json_values_after_marker(&html, "ytcfg.set")
        .into_iter()
        .find(|value| value.get("INNERTUBE_API_KEY").is_some())
        .ok_or_else(|| parse_error("YouTube page does not contain ytcfg"))?;
    let api_key = config
        .get("INNERTUBE_API_KEY")
        .and_then(Value::as_str)
        .filter(|key| !key.is_empty())
        .ok_or_else(|| parse_error("YouTube page does not contain an Innertube API key"))?
        .to_string();
    let client_version = config
        .get("INNERTUBE_CLIENT_VERSION")
        .and_then(Value::as_str)
        .filter(|version| !version.is_empty())
        .unwrap_or(DEFAULT_CLIENT_VERSION)
        .to_string();
    let visitor_data = config
        .get("VISITOR_DATA")
        .and_then(Value::as_str)
        .filter(|visitor| !visitor.is_empty())
        .map(str::to_string);

    let initial_data = extract_json_values_after_marker(&html, "ytInitialData")
        .into_iter()
        .find_map(|data| find_live_chat_continuation(&data))
        .ok_or_else(|| parse_error("YouTube page does not contain live chat data"))?;
    let continuation = initial_data.token;

    Ok(Bootstrap {
        api_key,
        client_version,
        visitor_data,
        continuation,
    })
}

async fn fetch_chat(
    client: &reqwest::Client,
    bootstrap: &Bootstrap,
    continuation: &str,
) -> Result<Value, DanmuStreamError> {
    let mut client_context = json!({
        "clientName": "WEB",
        "clientVersion": bootstrap.client_version,
        "hl": "en",
        "gl": "US"
    });
    if let Some(visitor_data) = &bootstrap.visitor_data {
        client_context["visitorData"] = Value::String(visitor_data.clone());
    }
    let context = json!({"client": client_context});
    let response = client
        .post(format!("{YOUTUBE_CHAT_URL}?key={}", bootstrap.api_key))
        .header("Content-Type", "application/json")
        .header("Origin", "https://www.youtube.com")
        .json(&json!({
            "context": context,
            "continuation": continuation,
        }))
        .send()
        .await
        .map_err(|_| parse_error("YouTube live chat request failed"))?;
    if !response.status().is_success() {
        return Err(parse_error(&format!(
            "YouTube live chat request returned HTTP {}",
            response.status()
        )));
    }
    response
        .json()
        .await
        .map_err(|_| parse_error("YouTube live chat response was not valid JSON"))
}

fn parse_error(message: &str) -> DanmuStreamError {
    DanmuStreamError::MessageParseError {
        err: message.to_string(),
    }
}

/// Extract the first balanced JSON value following a JavaScript marker.
/// YouTube changes whether these assignments are prefixed with `var`,
/// `window.` or whitespace, so looking for the first object/array is more
/// resilient than matching one exact assignment string.
#[cfg(test)]
fn extract_json_after_marker(html: &str, marker: &str) -> Option<Value> {
    extract_json_values_after_marker(html, marker)
        .into_iter()
        .next()
}

fn extract_json_values_after_marker(html: &str, marker: &str) -> Vec<Value> {
    let mut values = Vec::new();
    for (offset, _) in html.match_indices(marker) {
        let rest = &html[offset + marker.len()..];
        let Some(start) = rest.find(['{', '[']) else {
            continue;
        };
        let Some(json) = balanced_json(&rest[start..]) else {
            continue;
        };
        if let Ok(value) = serde_json::from_str(json) {
            values.push(value);
        }
    }
    values
}

/// Return a leading balanced JSON object or array, respecting escaped quotes.
fn balanced_json(value: &str) -> Option<&str> {
    let first = value.chars().next()?;
    if first != '{' && first != '[' {
        return None;
    }
    let mut stack = vec![first];
    let mut in_string = false;
    let mut escaped = false;

    for (index, ch) in value.char_indices().skip(1) {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' | '[' => stack.push(ch),
            '}' | ']' => {
                let expected = if ch == '}' { '{' } else { '[' };
                if stack.pop()? != expected {
                    return None;
                }
                if stack.is_empty() {
                    return Some(&value[..index + ch.len_utf8()]);
                }
            }
            _ => {}
        }
    }
    None
}

fn find_live_chat_continuation(value: &Value) -> Option<NextContinuation> {
    match value {
        Value::Object(object) => {
            for key in ["liveChatRenderer", "liveChatReplayRenderer"] {
                if let Some(chat) = object.get(key) {
                    if let Some(found) = find_continuation(chat) {
                        return Some(found);
                    }
                }
            }
            for child in object.values() {
                if let Some(found) = find_live_chat_continuation(child) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(values) => values.iter().find_map(find_live_chat_continuation),
        _ => None,
    }
}

fn find_continuation(value: &Value) -> Option<NextContinuation> {
    match value {
        Value::Object(object) => {
            for key in CONTINUATION_KEYS {
                if let Some(data) = object.get(key).and_then(Value::as_object) {
                    if let Some(token) = data.get("continuation").and_then(Value::as_str) {
                        return Some(NextContinuation {
                            token: token.to_string(),
                            timeout_ms: data
                                .get("timeoutMs")
                                .and_then(Value::as_u64)
                                .unwrap_or(DEFAULT_POLL_MS),
                        });
                    }
                }
            }
            for child in object.values() {
                if let Some(found) = find_continuation(child) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(values) => values.iter().find_map(find_continuation),
        _ => None,
    }
}

fn collect_renderers<'a>(value: &'a Value, output: &mut Vec<(&'static str, &'a Value)>) {
    match value {
        Value::Object(object) => {
            for key in RENDERER_KEYS {
                if let Some(renderer) = object.get(key) {
                    output.push((key, renderer));
                    return;
                }
            }
            for child in object.values() {
                collect_renderers(child, output);
            }
        }
        Value::Array(values) => {
            for child in values {
                collect_renderers(child, output);
            }
        }
        _ => {}
    }
}

fn normalize_renderer(kind: &str, renderer: &Value, room_id: &str) -> Option<LiveEvent> {
    let timestamp = renderer
        .get("timestampUsec")
        .and_then(|value| match value {
            Value::String(value) => value.parse::<i64>().ok(),
            Value::Number(value) => value.as_i64(),
            _ => None,
        })
        .map(|timestamp| timestamp / 1_000)
        .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());

    let user_name = renderer
        .get("authorName")
        .and_then(text_value)
        .filter(|name| !name.is_empty());
    let user_id = renderer
        .get("authorExternalChannelId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty());

    let (event_type, content, purchase_amount) = match kind {
        "liveChatTextMessageRenderer" => {
            ("danmu", renderer.get("message").and_then(text_value), None)
        }
        "liveChatPaidMessageRenderer" => (
            "super_chat",
            renderer
                .get("message")
                .and_then(text_value)
                .or_else(|| renderer.get("purchaseAmountText").and_then(text_value)),
            renderer.get("purchaseAmountText").and_then(text_value),
        ),
        "liveChatPaidStickerRenderer" => (
            "super_chat",
            renderer
                .get("sticker")
                .and_then(text_value)
                .or_else(|| renderer.get("purchaseAmountText").and_then(text_value)),
            renderer.get("purchaseAmountText").and_then(text_value),
        ),
        "liveChatMembershipItemRenderer"
        | "liveChatSponsorshipsGiftPurchaseAnnouncementRenderer"
        | "liveChatSponsorshipsWelcomeMessageRenderer"
        | "liveChatViewerEngagementMessageRenderer" => (
            "danmu",
            renderer
                .get("headerPrimaryText")
                .and_then(text_value)
                .or_else(|| renderer.get("headerSubtext").and_then(text_value))
                .or_else(|| renderer.get("message").and_then(text_value)),
            None,
        ),
        _ => return None,
    };

    let content = content?.trim().to_string();
    if content.is_empty() {
        return None;
    }

    let mut data = Map::new();
    if let Some(user_id) = user_id {
        data.insert("user_id".to_string(), Value::String(user_id.to_string()));
    }
    if let Some(user_name) = user_name {
        data.insert("user_name".to_string(), Value::String(user_name));
    }
    data.insert("content".to_string(), Value::String(content));
    if let Some(purchase_amount) = purchase_amount {
        data.insert(
            "purchase_amount".to_string(),
            Value::String(purchase_amount.clone()),
        );
        if let Some(price) = parse_cny_price(&purchase_amount) {
            data.insert("price".to_string(), Value::from(price));
        }
    }

    Some(LiveEvent {
        ts: timestamp,
        platform: "youtube".to_string(),
        room_id: room_id.to_string(),
        event_type: event_type.to_string(),
        data: Value::Object(data),
        raw: renderer.clone(),
    })
}

fn text_value(value: &Value) -> Option<String> {
    if let Some(text) = value.get("simpleText").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    if let Some(text) = value.get("text").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    if let Some(runs) = value.get("runs").and_then(Value::as_array) {
        let mut result = String::new();
        for run in runs {
            if let Some(text) = run.get("text").and_then(Value::as_str) {
                result.push_str(text);
                continue;
            }
            if let Some(emoji) = run.get("emoji") {
                if let Some(label) = emoji
                    .get("image")
                    .and_then(|image| image.get("accessibility"))
                    .and_then(|accessibility| accessibility.get("accessibilityData"))
                    .and_then(|data| data.get("label"))
                    .and_then(Value::as_str)
                    .or_else(|| {
                        emoji
                            .get("shortcuts")
                            .and_then(Value::as_array)
                            .and_then(|shortcuts| shortcuts.first())
                            .and_then(Value::as_str)
                    })
                    .or_else(|| emoji.get("emojiId").and_then(Value::as_str))
                {
                    result.push_str(label);
                }
            }
        }
        return (!result.is_empty()).then_some(result);
    }
    value
        .get("accessibility")
        .and_then(|accessibility| accessibility.get("accessibilityData"))
        .and_then(|data| data.get("label"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn parse_cny_price(amount: &str) -> Option<u32> {
    let trimmed = amount.trim();
    // The yen sign is shared by CNY and JPY, so only use explicit currency
    // codes for the existing CNY-only preview price field.
    let currency = trimmed.to_ascii_uppercase();
    let is_cny = currency.contains("CNY") || currency.contains("RMB");
    if !is_cny {
        return None;
    }

    let number: String = trimmed
        .chars()
        .filter(|ch| ch.is_ascii_digit() || *ch == '.')
        .collect();
    let value = number.parse::<f64>().ok()?;
    (value.is_finite() && value >= 0.0 && value.fract() == 0.0 && value <= u32::MAX as f64)
        .then_some(value as u32)
}

#[derive(Default)]
struct DedupCache {
    ids: HashSet<String>,
    order: VecDeque<String>,
}

impl DedupCache {
    fn insert(&mut self, id: &str) -> bool {
        if !self.ids.insert(id.to_string()) {
            return false;
        }
        self.order.push_back(id.to_string());
        while self.order.len() > MAX_SEEN_IDS {
            if let Some(old) = self.order.pop_front() {
                self.ids.remove(&old);
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_balanced_json_with_braces_inside_strings() {
        let html = r#"<script>var ytInitialData = {"text":"{not json}","items":[1,2]};</script>"#;
        let value = extract_json_after_marker(html, "ytInitialData").unwrap();
        assert_eq!(value["items"][1], 2);
        assert_eq!(value["text"], "{not json}");
    }

    #[test]
    fn finds_all_continuation_shapes() {
        for key in CONTINUATION_KEYS {
            let value = json!({key: {"continuation": "token", "timeoutMs": 1234}});
            let continuation = find_continuation(&value).unwrap();
            assert_eq!(continuation.token, "token");
            assert_eq!(continuation.timeout_ms, 1234);
        }
    }

    #[test]
    fn extracts_text_and_emoji_runs() {
        let value = json!({
            "runs": [
                {"text": "hello "},
                {"emoji": {"emojiId": "smile", "shortcuts": [":smile:"]}},
                {"text": "!"}
            ]
        });
        assert_eq!(text_value(&value).as_deref(), Some("hello :smile:!"));
    }

    #[test]
    fn normalizes_text_message() {
        let renderer = json!({
            "id": "message-1",
            "timestampUsec": "1700000000123456",
            "authorExternalChannelId": "UC1",
            "authorName": {"simpleText": "alice"},
            "message": {"runs": [{"text": "hello"}]}
        });
        let event = normalize_renderer("liveChatTextMessageRenderer", &renderer, "video").unwrap();
        assert_eq!(event.ts, 1_700_000_000_123);
        assert_eq!(event.event_type, "danmu");
        assert_eq!(event.data["content"], "hello");
        assert_eq!(event.data["user_name"], "alice");
        assert_eq!(event.raw["id"], "message-1");
    }

    #[test]
    fn normalizes_cny_paid_message_but_does_not_mislabel_other_currencies() {
        let mut renderer = json!({
            "id": "paid-1",
            "authorName": {"simpleText": "alice"},
            "purchaseAmountText": {"simpleText": "CNY 10"},
            "message": {"simpleText": "support"}
        });
        let event = normalize_renderer("liveChatPaidMessageRenderer", &renderer, "video").unwrap();
        assert_eq!(event.event_type, "super_chat");
        assert_eq!(event.data["purchase_amount"], "CNY 10");
        assert_eq!(event.data["price"], 10);

        for amount in ["¥10.00", "$5.00", "CNY 10.50"] {
            renderer["purchaseAmountText"] = json!({"simpleText": amount});
            let event =
                normalize_renderer("liveChatPaidMessageRenderer", &renderer, "video").unwrap();
            assert!(event.data.get("price").is_none(), "misclassified {amount}");
        }
    }

    #[test]
    fn malformed_renderer_is_ignored() {
        assert!(normalize_renderer(
            "liveChatTextMessageRenderer",
            &json!({"message": {"simpleText": ""}}),
            "video"
        )
        .is_none());
        assert!(normalize_renderer("unknown", &json!({}), "video").is_none());
    }

    #[test]
    fn chat_poll_rebootstraps_after_repeated_errors() {
        assert!(!should_rebootstrap(1));
        assert!(!should_rebootstrap(2));
        assert!(should_rebootstrap(3));
        assert!(should_rebootstrap(4));
    }

    #[test]
    fn chat_retry_backoff_is_bounded() {
        assert_eq!(retry_delay(1), Duration::from_secs(1));
        assert_eq!(retry_delay(2), Duration::from_secs(2));
        assert_eq!(retry_delay(10), Duration::from_secs(MAX_RETRY_DELAY_SECS));
    }

    #[test]
    fn dedup_cache_rejects_repeated_ids() {
        let mut cache = DedupCache::default();
        assert!(cache.insert("one"));
        assert!(!cache.insert("one"));
        assert!(cache.insert("two"));
    }
}
