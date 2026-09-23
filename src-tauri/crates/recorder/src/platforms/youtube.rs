//! YouTube live recorder.
//!
//! A YouTube recorder is configured with a channel id, handle, or video URL.
//! Every status poll resolves that stable identifier to the currently live
//! video and obtains an HLS manifest from YouTube's player response.

use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use danmu_stream::provider::ProviderType;
use reqwest::header::HeaderValue;
use serde_json::{json, Value};
use tokio::sync::{broadcast, RwLock};
use url::Url;

use crate::account::Account;
use crate::errors::RecorderError;
use crate::platforms::common::{DanmuConfig, DanmuSpawn, PlatformApi, RoomPoll, StreamPull};
use crate::platforms::PlatformType;
use crate::traits::RecorderTrait;
use crate::{Recorder, UserInfo};

const YOUTUBE_WATCH_URL: &str = "https://www.youtube.com/watch?v=";
const YOUTUBE_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const YOUTUBE_ANDROID_CLIENT_VERSION: &str = "20.10.38";
const YOUTUBE_ANDROID_USER_AGENT: &str =
    "com.google.android.youtube/20.10.38 (Linux; U; Android 15) gzip";

pub type YoutubeRecorder = Recorder<YoutubeExtra>;

#[derive(Clone)]
pub struct YoutubeExtra {
    live_stream: Arc<RwLock<Option<YoutubeStreamInfo>>>,
    /// The result of the latest room poll.  It must survive `clear_stream()`
    /// because the shared loop resets the old stream between poll and pull.
    pending: Arc<RwLock<Option<YoutubeStreamInfo>>>,
}

#[derive(Debug, Clone)]
struct YoutubeStreamInfo {
    video_id: String,
    hls_url: Option<String>,
}

#[derive(Debug, Clone)]
struct YoutubePage {
    video_id: String,
    title: String,
    cover: String,
    channel_id: String,
    channel_name: String,
    is_live: bool,
    hls_url: Option<String>,
}

impl YoutubeRecorder {
    pub fn new(
        room_id: &str,
        account: &Account,
        cache_dir: PathBuf,
        event_channel: broadcast::Sender<crate::events::RecorderEvent>,
        update_interval: Arc<AtomicU64>,
        enabled: bool,
    ) -> Result<Self, RecorderError> {
        if room_id.trim().is_empty() {
            return Err(RecorderError::InvalidValue);
        }

        Ok(Self::with_extra(
            PlatformType::Youtube,
            room_id,
            account,
            cache_dir,
            event_channel,
            update_interval,
            enabled,
            YoutubeExtra {
                live_stream: Arc::new(RwLock::new(None)),
                pending: Arc::new(RwLock::new(None)),
            },
        ))
    }

    fn room_url(&self) -> Result<String, RecorderError> {
        youtube_room_url(&self.room_id)
    }

    async fn fetch_page(&self) -> Result<YoutubePage, RecorderError> {
        let (mut html, mut final_url) = self.fetch_html(&self.room_url()?).await?;
        let mut page = parse_page(&html, &final_url)?;

        // Channel `/live` pages often contain a live video card but no player
        // response. Resolve that card to its watch page before asking the
        // Innertube player endpoint for the HLS manifest.
        if page.is_live
            && page.hls_url.is_none()
            && final_url.query_pairs().all(|(key, _)| key != "v")
        {
            let watch_url = format!("{YOUTUBE_WATCH_URL}{}", page.video_id);
            (html, final_url) = self.fetch_html(&watch_url).await?;
            page = parse_page(&html, &final_url)?;
        }

        // The web player now returns SABR/DASH data for many public live
        // streams instead of `hlsManifestUrl`. The Android Innertube client
        // still exposes the same live broadcast as HLS, which the existing
        // recorder can segment without introducing a separate media pipeline.
        if page.is_live && page.hls_url.is_none() {
            page.hls_url = self.fetch_live_hls(&html, &page.video_id).await?;
        }
        Ok(page)
    }

    async fn fetch_live_hls(
        &self,
        watch_html: &str,
        video_id: &str,
    ) -> Result<Option<String>, RecorderError> {
        let config = extract_json_values_after_marker(watch_html, "ytcfg.set")
            .into_iter()
            .find(|config| config.get("INNERTUBE_API_KEY").is_some())
            .ok_or_else(|| RecorderError::ApiError {
                error: "YouTube Innertube configuration was not found".to_string(),
            })?;
        let api_key = config
            .get("INNERTUBE_API_KEY")
            .and_then(Value::as_str)
            .filter(|key| !key.is_empty())
            .ok_or_else(|| RecorderError::ApiError {
                error: "YouTube Innertube API key was not found".to_string(),
            })?;
        let mut client_context = json!({
            "clientName": "ANDROID",
            "clientVersion": YOUTUBE_ANDROID_CLIENT_VERSION,
            "androidSdkVersion": 35,
            "hl": "en",
            "gl": "US",
            "platform": "MOBILE",
        });
        if let Some(visitor_data) = config
            .get("VISITOR_DATA")
            .and_then(Value::as_str)
            .filter(|visitor_data| !visitor_data.is_empty())
        {
            client_context["visitorData"] = Value::String(visitor_data.to_string());
        }
        let body = json!({
            "context": {"client": client_context},
            "videoId": video_id,
            "contentCheckOk": true,
            "racyCheckOk": true,
        });
        let mut request = self
            .client
            .post(format!(
                "https://www.youtube.com/youtubei/v1/player?key={api_key}"
            ))
            .header("User-Agent", YOUTUBE_ANDROID_USER_AGENT)
            .header("X-YouTube-Client-Name", "3")
            .header("X-YouTube-Client-Version", YOUTUBE_ANDROID_CLIENT_VERSION)
            .header("Origin", "https://www.youtube.com")
            .json(&body);
        if !self.account.cookies.trim().is_empty() {
            let cookie = HeaderValue::from_str(&self.account.cookies)
                .map_err(|_| RecorderError::InvalidCookies)?;
            request = request.header("Cookie", cookie);
        }

        let response = request
            .send()
            .await
            .map_err(|error| RecorderError::ApiError {
                error: format!("YouTube player request failed: {}", error.without_url()),
            })?;
        if !response.status().is_success() {
            return Err(RecorderError::InvalidResponseStatus {
                status: response.status(),
            });
        }
        let player: Value = response
            .json()
            .await
            .map_err(|error| RecorderError::ApiError {
                error: format!("Invalid YouTube player response: {}", error.without_url()),
            })?;
        Ok(player
            .get("streamingData")
            .and_then(|streaming| streaming.get("hlsManifestUrl"))
            .and_then(Value::as_str)
            .map(str::to_string))
    }

    async fn fetch_html(&self, url: &str) -> Result<(String, Url), RecorderError> {
        let mut request = self
            .client
            .get(url)
            .header("User-Agent", YOUTUBE_USER_AGENT)
            .header("Accept-Language", "en-US,en;q=0.8");
        if !self.account.cookies.trim().is_empty() {
            let cookie = HeaderValue::from_str(&self.account.cookies)
                .map_err(|_| RecorderError::InvalidCookies)?;
            request = request.header("Cookie", cookie);
        }

        let response = request.send().await?.error_for_status()?;
        let final_url = response.url().clone();
        Ok((response.text().await?, final_url))
    }

    fn log_error(&self, message: &str) {
        log::error!("[YouTube][{}]{message}", self.room_id);
    }
}

#[async_trait]
impl PlatformApi for YoutubeRecorder {
    async fn poll_room(&self) -> Result<RoomPoll, RecorderError> {
        let page = self.fetch_page().await?;
        let stream = YoutubeStreamInfo {
            video_id: page.video_id.clone(),
            hls_url: page.hls_url.clone(),
        };
        *self.extra.pending.write().await = page.is_live.then_some(stream);

        let user =
            (!page.channel_id.is_empty() || !page.channel_name.is_empty()).then_some(UserInfo {
                user_id: page.channel_id,
                user_name: page.channel_name,
                // Player metadata exposes the video thumbnail, not the
                // channel avatar. Leave this empty so the UI uses its
                // platform-specific fallback icon instead of a cover image.
                user_avatar: String::new(),
            });

        Ok(RoomPoll {
            live: page.is_live,
            room_title: page.title,
            room_cover: page.cover,
            user,
            platform_live_id: page.is_live.then_some(page.video_id),
        })
    }

    async fn poll_stream(&self) -> bool {
        let Some(stream) = self.extra.pending.write().await.take() else {
            return false;
        };
        *self.platform_live_id.write().await = stream.video_id.clone();

        if stream.hls_url.is_none() {
            self.log_error("Live video has no HLS manifest yet");
            return false;
        }
        *self.extra.live_stream.write().await = Some(stream);
        self.last_update
            .store(Utc::now().timestamp(), std::sync::atomic::Ordering::Relaxed);
        true
    }

    async fn open_pull(&self, live_id: &str) -> Result<StreamPull, RecorderError> {
        let Some(stream) = self.extra.live_stream.read().await.clone() else {
            return Err(RecorderError::NoStreamAvailable);
        };
        let Some(hls_url) = stream.hls_url else {
            return Err(RecorderError::NoStreamAvailable);
        };
        let cookies =
            (!self.account.cookies.trim().is_empty()).then(|| self.account.cookies.clone());
        StreamPull::hls(live_id, &hls_url, cookies).await
    }

    async fn clear_stream(&self) {
        *self.extra.live_stream.write().await = None;
    }

    fn danmu_config(&self) -> Option<DanmuConfig> {
        Some(DanmuConfig {
            provider: ProviderType::Youtube,
            spawn: DanmuSpawn::PerRecording,
        })
    }

    async fn danmu_room_id(&self) -> String {
        self.platform_live_id.read().await.clone()
    }
}

#[async_trait]
impl RecorderTrait for YoutubeRecorder {
    async fn run(&self) {
        self.run_recording_loop().await;
    }
}

/// Convert the configured identifier into a page that resolves the current
/// live video.  Plain eleven-character values are treated as video ids;
/// channel ids and handles use their `/live` page so the next broadcast is
/// picked up automatically.
fn youtube_room_url(identifier: &str) -> Result<String, RecorderError> {
    let identifier = identifier.trim();
    let identifier = identifier
        .strip_prefix("bsr://")
        .map(|value| format!("https://{value}"))
        .unwrap_or_else(|| identifier.to_string());
    if identifier.starts_with("http://") || identifier.starts_with("https://") {
        let url = Url::parse(&identifier).map_err(|_| RecorderError::InvalidValue)?;
        let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
        let is_youtube_host =
            host == "youtube.com" || host.ends_with(".youtube.com") || host == "youtu.be";
        if !is_youtube_host {
            return Err(RecorderError::InvalidValue);
        }
        if let Some(video_id) = url
            .query_pairs()
            .find(|(key, _)| key == "v")
            .map(|(_, value)| value.into_owned())
        {
            return Ok(format!("https://www.youtube.com/watch?v={video_id}"));
        }
        let path = url.path().trim_matches('/');
        if path.is_empty() {
            return Err(RecorderError::InvalidValue);
        }
        if host == "youtu.be" {
            return Ok(format!("https://www.youtube.com/watch?v={path}"));
        }
        let path = if !path.ends_with("/live")
            && !path.starts_with("watch/")
            && !path.starts_with("live/")
            && !path.starts_with("shorts/")
            && !path.starts_with("embed/")
        {
            format!("{path}/live")
        } else {
            path.to_string()
        };
        return Ok(format!("https://www.youtube.com/{path}"));
    }

    if is_video_id(&identifier) {
        return Ok(format!("https://www.youtube.com/watch?v={identifier}"));
    }
    if identifier.starts_with("UC") {
        return Ok(format!("https://www.youtube.com/channel/{identifier}/live"));
    }

    let path = if identifier.starts_with('@') {
        identifier.to_string()
    } else if identifier.starts_with('/') {
        identifier.trim_start_matches('/').to_string()
    } else {
        format!("@{identifier}")
    };
    Ok(format!("https://www.youtube.com/{path}/live"))
}

fn is_video_id(value: &str) -> bool {
    value.len() == 11
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn parse_page(html: &str, final_url: &Url) -> Result<YoutubePage, RecorderError> {
    if let Some(player) = extract_player_response(html) {
        return parse_player_page(&player, final_url);
    }

    let initial_data = extract_json_after_marker(html, "ytInitialData").ok_or_else(|| {
        RecorderError::ApiError {
            error: "YouTube page data not found".to_string(),
        }
    })?;
    parse_channel_live_page(&initial_data, final_url)
}

fn parse_player_page(player: &Value, final_url: &Url) -> Result<YoutubePage, RecorderError> {
    let details = player.get("videoDetails").cloned().unwrap_or(Value::Null);
    let url_video_id = final_url
        .query_pairs()
        .find(|(key, _)| key == "v")
        .map(|(_, value)| value.into_owned());
    let video_id = details
        .get("videoId")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or(url_video_id)
        .unwrap_or_default();
    if video_id.is_empty() {
        return Err(RecorderError::ApiError {
            error: "YouTube video id not found".to_string(),
        });
    }

    let title = details
        .get("title")
        .and_then(Value::as_str)
        .or_else(|| details.get("shortDescription").and_then(Value::as_str))
        .unwrap_or("YouTube live")
        .to_string();
    let channel_id = details
        .get("channelId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let channel_name = details
        .get("author")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let cover = details
        .get("thumbnail")
        .and_then(thumbnails_url)
        .unwrap_or_default();
    let hls_url = player
        .get("streamingData")
        .and_then(|streaming| streaming.get("hlsManifestUrl"))
        .and_then(Value::as_str)
        .map(str::to_string);

    let is_live_now = player
        .get("microformat")
        .and_then(|microformat| microformat.get("playerMicroformatRenderer"))
        .and_then(|microformat| microformat.get("liveBroadcastDetails"))
        .and_then(|live| live.get("isLiveNow"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let is_live = is_live_now
        || details
            .get("isLive")
            .and_then(Value::as_bool)
            .unwrap_or(false);

    Ok(YoutubePage {
        video_id,
        title,
        cover,
        channel_id,
        channel_name,
        is_live,
        hls_url,
    })
}

fn parse_channel_live_page(
    initial_data: &Value,
    final_url: &Url,
) -> Result<YoutubePage, RecorderError> {
    let channel = find_object_with_key(initial_data, "channelMetadataRenderer");
    let channel_id = channel
        .and_then(|value| value.get("externalId"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let channel_name = channel
        .and_then(|value| value.get("title"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    if let Some(card) = find_live_video_card(initial_data) {
        let (video_id, title, cover, card_channel_id, card_channel_name) = match card {
            LiveVideoCard::Renderer(renderer) => (
                renderer
                    .get("videoId")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                renderer
                    .get("title")
                    .and_then(text_value)
                    .unwrap_or_else(|| "YouTube live".to_string()),
                renderer
                    .get("thumbnail")
                    .and_then(thumbnails_url)
                    .unwrap_or_default(),
                renderer
                    .get("ownerText")
                    .and_then(first_browse_id)
                    .unwrap_or_default(),
                renderer
                    .get("ownerText")
                    .and_then(text_value)
                    .unwrap_or_default(),
            ),
            LiveVideoCard::Lockup(lockup) => (
                lockup
                    .get("contentId")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                lockup
                    .get("metadata")
                    .and_then(|metadata| metadata.get("lockupMetadataViewModel"))
                    .and_then(|metadata| metadata.get("title"))
                    .and_then(text_value)
                    .unwrap_or_else(|| "YouTube live".to_string()),
                lockup
                    .get("contentImage")
                    .and_then(|image| image.get("thumbnailViewModel"))
                    .and_then(lockup_thumbnail_url)
                    .unwrap_or_default(),
                String::new(),
                String::new(),
            ),
        };
        if video_id.is_empty() {
            return Err(RecorderError::ApiError {
                error: "YouTube live video id not found".to_string(),
            });
        }
        return Ok(YoutubePage {
            video_id,
            title,
            cover,
            channel_id: if card_channel_id.is_empty() {
                channel_id
            } else {
                card_channel_id
            },
            channel_name: if card_channel_name.is_empty() {
                channel_name
            } else {
                card_channel_name
            },
            is_live: true,
            hls_url: None,
        });
    }

    let video_id = final_url
        .query_pairs()
        .find(|(key, _)| key == "v")
        .map(|(_, value)| value.into_owned())
        .unwrap_or_default();
    Ok(YoutubePage {
        video_id,
        title: if channel_name.is_empty() {
            "YouTube live".to_string()
        } else {
            channel_name.clone()
        },
        cover: String::new(),
        channel_id,
        channel_name,
        is_live: false,
        hls_url: None,
    })
}

enum LiveVideoCard<'a> {
    Renderer(&'a Value),
    Lockup(&'a Value),
}

fn find_live_video_card(value: &Value) -> Option<LiveVideoCard<'_>> {
    match value {
        Value::Object(object) => {
            if let Some(renderer) = object.get("videoRenderer") {
                if renderer
                    .get("videoId")
                    .and_then(Value::as_str)
                    .is_some_and(|id| !id.is_empty())
                    && is_live_video_renderer(renderer)
                {
                    return Some(LiveVideoCard::Renderer(renderer));
                }
            }
            if let Some(lockup) = object.get("lockupViewModel") {
                if lockup
                    .get("contentId")
                    .and_then(Value::as_str)
                    .is_some_and(|id| !id.is_empty())
                    && is_live_lockup(lockup)
                {
                    return Some(LiveVideoCard::Lockup(lockup));
                }
            }
            object.values().find_map(find_live_video_card)
        }
        Value::Array(values) => values.iter().find_map(find_live_video_card),
        _ => None,
    }
}

fn is_live_video_renderer(renderer: &Value) -> bool {
    let Some(object) = renderer.as_object() else {
        return false;
    };
    let has_live_badge = object
        .get("badges")
        .and_then(Value::as_array)
        .is_some_and(|badges| {
            badges.iter().any(|badge| {
                badge
                    .get("metadataBadgeRenderer")
                    .and_then(|renderer| renderer.get("label"))
                    .and_then(text_value)
                    .is_some_and(|label| label.to_ascii_uppercase().contains("LIVE"))
            })
        });
    let has_live_overlay = object
        .get("thumbnailOverlays")
        .and_then(Value::as_array)
        .is_some_and(|overlays| {
            overlays.iter().any(|overlay| {
                let renderer = overlay.get("thumbnailOverlayTimeStatusRenderer");
                renderer
                    .and_then(|renderer| renderer.get("style"))
                    .and_then(Value::as_str)
                    .is_some_and(|style| style.eq_ignore_ascii_case("LIVE"))
                    || renderer
                        .and_then(|renderer| renderer.get("text"))
                        .and_then(text_value)
                        .is_some_and(|text| text.eq_ignore_ascii_case("LIVE"))
            })
        });
    has_live_badge || has_live_overlay
}

fn is_live_lockup(lockup: &Value) -> bool {
    lockup
        .get("contentImage")
        .and_then(|image| image.get("thumbnailViewModel"))
        .and_then(|thumbnail| thumbnail.get("overlays"))
        .and_then(Value::as_array)
        .is_some_and(|overlays| overlays.iter().any(contains_live_badge))
}

fn contains_live_badge(value: &Value) -> bool {
    match value {
        Value::Object(object) => {
            if let Some(badge) = object.get("thumbnailBadgeViewModel") {
                let text_is_live = badge
                    .get("text")
                    .and_then(text_value)
                    .is_some_and(|text| is_live_label(&text));
                let style_is_live = badge
                    .get("badgeStyle")
                    .and_then(Value::as_str)
                    .is_some_and(|style| style.to_ascii_uppercase().contains("LIVE"));
                if text_is_live || style_is_live {
                    return true;
                }
            }
            object.values().any(contains_live_badge)
        }
        Value::Array(values) => values.iter().any(contains_live_badge),
        _ => false,
    }
}

fn is_live_label(label: &str) -> bool {
    let label = label.trim();
    label.eq_ignore_ascii_case("live")
        || label.eq_ignore_ascii_case("live now")
        || label == "直播中"
        || label == "正在直播"
}

fn lockup_thumbnail_url(thumbnail: &Value) -> Option<String> {
    thumbnail
        .get("image")
        .and_then(|image| image.get("sources"))
        .and_then(Value::as_array)
        .and_then(|sources| sources.last())
        .and_then(|source| source.get("url"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn find_object_with_key<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    match value {
        Value::Object(object) => {
            if let Some(found) = object.get(key) {
                return Some(found);
            }
            object
                .values()
                .find_map(|child| find_object_with_key(child, key))
        }
        Value::Array(values) => values
            .iter()
            .find_map(|child| find_object_with_key(child, key)),
        _ => None,
    }
}

fn first_browse_id(value: &Value) -> Option<String> {
    value
        .get("runs")
        .and_then(Value::as_array)
        .and_then(|runs| runs.first())
        .and_then(|run| run.get("navigationEndpoint"))
        .and_then(|endpoint| endpoint.get("browseEndpoint"))
        .and_then(|endpoint| endpoint.get("browseId"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn thumbnails_url(value: &Value) -> Option<String> {
    value
        .get("thumbnails")
        .and_then(Value::as_array)
        .and_then(|thumbnails| thumbnails.last())
        .and_then(|thumbnail| thumbnail.get("url"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn text_value(value: &Value) -> Option<String> {
    if let Some(text) = value.get("simpleText").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    if let Some(text) = value.get("text").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    if let Some(text) = value.get("content").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    value.get("runs").and_then(Value::as_array).map(|runs| {
        runs.iter()
            .filter_map(|run| run.get("text").and_then(Value::as_str))
            .collect::<String>()
    })
}

fn extract_player_response(html: &str) -> Option<Value> {
    extract_json_values_after_marker(html, "ytInitialPlayerResponse")
        .into_iter()
        .find(|value| {
            value.get("videoDetails").is_some()
                || value.get("streamingData").is_some()
                || value.get("playabilityStatus").is_some()
        })
}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn url(value: &str) -> Url {
        Url::parse(value).unwrap()
    }

    #[test]
    fn resolves_video_channel_and_handle_identifiers() {
        assert_eq!(
            youtube_room_url("dQw4w9WgXcQ").unwrap(),
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ"
        );
        assert_eq!(
            youtube_room_url("UC123").unwrap(),
            "https://www.youtube.com/channel/UC123/live"
        );
        assert_eq!(
            youtube_room_url("@creator").unwrap(),
            "https://www.youtube.com/@creator/live"
        );
        assert!(youtube_room_url("https://evil-youtube.com/@creator").is_err());
    }

    #[test]
    fn extracts_page_metadata_and_hls_manifest() {
        let html = r#"
            <script>var ytInitialPlayerResponse = {
              "videoDetails": {
                "videoId": "video123456",
                "title": "Live title",
                "author": "Creator",
                "channelId": "UCcreator",
                "isLive": true,
                "thumbnail": {"thumbnails": [{"url": "https://img.test/cover.jpg"}]}
              },
              "streamingData": {"hlsManifestUrl": "https://video.test/live.m3u8"}
            };</script>
        "#;
        let page = parse_page(html, &url("https://www.youtube.com/watch?v=video123456")).unwrap();
        assert!(page.is_live);
        assert_eq!(page.video_id, "video123456");
        assert_eq!(
            page.hls_url.as_deref(),
            Some("https://video.test/live.m3u8")
        );
        assert_eq!(page.channel_id, "UCcreator");
    }

    #[test]
    fn parses_live_video_from_channel_page_without_a_player_response() {
        let html = r#"
            <script>var ytInitialData = {
              "contents": {"videoRenderer": {
                "videoId": "video123456",
                "title": {"simpleText": "Live title"},
                "ownerText": {"runs": [{"text": "Creator", "navigationEndpoint": {"browseEndpoint": {"browseId": "UCcreator"}}}]},
                "badges": [{"metadataBadgeRenderer": {"label": {"simpleText": "LIVE NOW"}}}],
                "thumbnail": {"thumbnails": [{"url": "https://img.test/cover.jpg"}]}
              }}
            };</script>
        "#;
        let page = parse_page(html, &url("https://www.youtube.com/@creator/live")).unwrap();
        assert!(page.is_live);
        assert_eq!(page.video_id, "video123456");
        assert_eq!(page.channel_id, "UCcreator");
        assert_eq!(page.hls_url, None);
    }

    #[test]
    fn parses_live_video_from_modern_lockup_cards() {
        let html = r#"
            <script>var ytInitialData = {
              "contents": {"richItemRenderer": {"content": {"lockupViewModel": {
                "contentId": "video123456",
                "contentType": "LOCKUP_CONTENT_TYPE_VIDEO",
                "metadata": {"lockupMetadataViewModel": {"title": {"content": "Live title"}}},
                "contentImage": {"thumbnailViewModel": {
                  "image": {"sources": [{"url": "https://img.test/cover.jpg"}]},
                  "overlays": [{"thumbnailBottomOverlayViewModel": {"badges": [{
                    "thumbnailBadgeViewModel": {"text": "LIVE", "badgeStyle": "THUMBNAIL_BADGE_STYLE_LIVE"}
                  }]}}]
                }}
              }}}
            }};</script>
        "#;
        let page = parse_page(html, &url("https://www.youtube.com/@creator/live")).unwrap();
        assert!(page.is_live);
        assert_eq!(page.video_id, "video123456");
        assert_eq!(page.title, "Live title");
        assert_eq!(page.cover, "https://img.test/cover.jpg");
    }

    #[test]
    fn balanced_json_ignores_braces_in_strings() {
        let value = balanced_json(r#"{"message":"{not json}","ok":true} trailing"#).unwrap();
        assert_eq!(value, r#"{"message":"{not json}","ok":true}"#);
    }
}
