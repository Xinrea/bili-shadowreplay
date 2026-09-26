use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use regex::Regex;
use reqwest::header::{HeaderMap, HeaderValue, COOKIE, REFERER, USER_AGENT};
use reqwest::{Client, RequestBuilder, StatusCode};
use thiserror::Error;
use tokio::sync::RwLock;
use tokio::time::{sleep, timeout_at, Instant};

use crate::account::Account;
use crate::errors::RecorderError;

use super::response::{self, DouyuEncryptionData, DouyuRoomInfo};

pub const ROOM_API_BASE: &str = "https://open.douyucdn.cn/api/RoomApi/room";
pub const ENCRYPTION_API: &str = "https://www.douyu.com/wgapi/livenc/liveweb/websec/getEncryption";
pub const PLAY_API_BASE: &str = "https://www.douyu.com/lapi/live/getH5PlayV1";
pub const DOUYU_REFERER: &str = "https://www.douyu.com/";
pub const DEFAULT_DID: &str = "10000000000000000000000000001501";
pub const DEFAULT_CDN: &str = "";
pub const DEFAULT_RATE: u64 = 0;
pub const DEFAULT_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:154.0) Gecko/20100101 Firefox/154.0";
const KEY_FALLBACK_TTL_SECS: u64 = 24 * 60 * 60;
const STREAM_SELECTION_TIMEOUT: Duration = Duration::from_secs(30);
// The highest source quality can exceed 20 Mbps. The tested 4 Mbps tier
// records reliably without forcing users to opt into the P2P web player.
const PREFERRED_MAX_BITRATE_KBPS: u64 = 4000;

/// A key cached for this recorder. Douyu occasionally returns a short-lived
/// `expire_at`; when it does not, the conservative fallback is one day.
#[derive(Debug, Clone)]
pub struct CachedEncryptionKey {
    pub data: DouyuEncryptionData,
    pub expire_at: u64,
    pub user_agent: String,
}

pub type EncryptionCache = Arc<RwLock<Option<CachedEncryptionKey>>>;

impl CachedEncryptionKey {
    fn new(data: DouyuEncryptionData, user_agent: String) -> Self {
        let now = unix_timestamp();
        let expire_at = if data.expire_at > now + 5 {
            data.expire_at
        } else {
            now.saturating_add(KEY_FALLBACK_TTL_SECS)
        };
        Self {
            data,
            expire_at,
            user_agent,
        }
    }

    fn is_valid(&self) -> bool {
        unix_timestamp().saturating_add(5) < self.expire_at
    }
}

#[derive(Debug, Error)]
pub enum DouyuApiError {
    #[error("Douyu {endpoint} request failed: {source}")]
    Request {
        endpoint: &'static str,
        #[source]
        source: reqwest::Error,
    },
    #[error("Douyu {endpoint} has an invalid {name} header")]
    InvalidHeader {
        endpoint: &'static str,
        name: &'static str,
    },
    #[error("Douyu room is offline: {detail}")]
    Offline { detail: String },
    #[error("Douyu authentication failed: {detail}")]
    Authentication { detail: String },
    #[error("Douyu {endpoint} returned HTTP {status}")]
    Http {
        endpoint: &'static str,
        status: StatusCode,
    },
    #[error("Douyu {endpoint} returned API error {code}: {message}")]
    Api {
        endpoint: &'static str,
        code: i32,
        message: String,
    },
    #[error("Douyu {endpoint} returned an invalid response: {detail}")]
    InvalidResponse {
        endpoint: &'static str,
        detail: String,
    },
}

impl DouyuApiError {
    pub fn into_recorder_error(self) -> RecorderError {
        match self {
            Self::Request { source, .. } => RecorderError::ClientError(source),
            Self::InvalidHeader { name, .. } => RecorderError::InvalidHeaderValue {
                name: name.to_string(),
            },
            Self::Offline { .. } => RecorderError::NotLive,
            Self::Authentication { detail } => RecorderError::ApiError {
                error: format!("Douyu authentication failed: {detail}"),
            },
            Self::Http { endpoint, status } => RecorderError::ApiError {
                error: format!("Douyu {endpoint} returned HTTP {status}"),
            },
            Self::Api {
                endpoint,
                code,
                message,
            } => RecorderError::ApiError {
                error: format!("Douyu {endpoint} error {code}: {message}"),
            },
            Self::InvalidResponse { endpoint, detail } => RecorderError::ApiError {
                error: format!("Douyu {endpoint} invalid response: {detail}"),
            },
        }
    }
}

/// Build the identity used both by Douyu API requests and the ffmpeg FLV pull.
pub fn pull_http_identity(account: &Account, _room_id: u64) -> (String, Vec<(String, String)>) {
    let mut headers = vec![("Referer".to_string(), DOUYU_REFERER.to_string())];
    if !account.cookies.trim().is_empty() {
        headers.push(("Cookie".to_string(), account.cookies.clone()));
    }
    (DEFAULT_USER_AGENT.to_string(), headers)
}

pub fn parse_numeric_room_id(room_id: &str) -> Result<u64, RecorderError> {
    room_id
        .trim()
        .parse::<u64>()
        .map_err(|_| RecorderError::ApiError {
            error: format!("Douyu room id must be numeric: {room_id}"),
        })
}

static REAL_ROOM_ID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"roomInfo(?:&quot;|")\s*:\s*\{\s*(?:&quot;|")?rid(?:&quot;|")?\s*:\s*(\d+)"#)
        .expect("Douyu room id regex is valid")
});

fn extract_real_room_id(body: &str) -> Option<u64> {
    REAL_ROOM_ID
        .captures(body)
        .and_then(|captures| captures.get(1))
        .and_then(|value| value.as_str().parse().ok())
}

/// Resolve a numeric room id from a normal id or a Douyu vanity URL/path.
///
/// The public RoomApi only accepts numeric ids. Douyu's mobile page embeds the
/// numeric id in `roomInfo`, so resolve a short name once before polling it.
fn numeric_rid_query(room_id: &str) -> Option<u64> {
    let query = room_id.split_once('?')?.1.split('#').next()?;
    url::form_urlencoded::parse(query.as_bytes())
        .find_map(|(key, value)| (key == "rid").then(|| value.parse::<u64>().ok()).flatten())
}

pub async fn resolve_room_id(
    client: &Client,
    account: &Account,
    room_id: &str,
) -> Result<u64, DouyuApiError> {
    resolve_room_id_at(client, account, room_id, "https://m.douyu.com").await
}

async fn resolve_room_id_at(
    client: &Client,
    account: &Account,
    room_id: &str,
    mobile_base: &str,
) -> Result<u64, DouyuApiError> {
    if let Some(room_id) = numeric_rid_query(room_id) {
        return Ok(room_id);
    }
    if let Ok(room_id) = room_id.trim().parse::<u64>() {
        return Ok(room_id);
    }

    let path = room_id
        .trim()
        .split(['?', '#'])
        .next()
        .unwrap_or_default()
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default();
    if path.is_empty() {
        return Err(DouyuApiError::InvalidResponse {
            endpoint: "roomResolve",
            detail: "room id/path is empty".to_string(),
        });
    }
    if let Ok(room_id) = path.parse::<u64>() {
        return Ok(room_id);
    }

    let endpoint = "roomResolve";
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static(DEFAULT_USER_AGENT));
    headers.insert(
        REFERER,
        HeaderValue::from_str(&format!("https://m.douyu.com/{path}")).map_err(|_| {
            DouyuApiError::InvalidHeader {
                endpoint,
                name: "Referer",
            }
        })?,
    );
    if !account.cookies.trim().is_empty() {
        headers.insert(
            COOKIE,
            HeaderValue::from_str(&account.cookies).map_err(|_| DouyuApiError::InvalidHeader {
                endpoint,
                name: "Cookie",
            })?,
        );
    }
    let request = client
        .get(format!("{}/{path}", mobile_base.trim_end_matches('/')))
        .headers(headers);
    let (status, body) = send_text(request, endpoint).await?;
    if !status.is_success() {
        return Err(DouyuApiError::Http { endpoint, status });
    }

    extract_real_room_id(&body).ok_or_else(|| DouyuApiError::InvalidResponse {
        endpoint,
        detail: format!("could not resolve room path {path}"),
    })
}

pub async fn get_room_info(
    client: &Client,
    account: &Account,
    room_id: u64,
) -> Result<DouyuRoomInfo, DouyuApiError> {
    let endpoint = "RoomApi";
    let url = format!("{ROOM_API_BASE}/{room_id}");
    let request = client
        .get(url)
        .headers(request_headers(account, room_id, endpoint)?);
    let (status, body) = send_text(request, endpoint).await?;

    if !status.is_success() {
        if is_auth_failure(status, &body) {
            return Err(DouyuApiError::Authentication {
                detail: format!("RoomApi HTTP {status}"),
            });
        }
        return Err(DouyuApiError::Http { endpoint, status });
    }

    let response =
        response::parse_room_response(&body).map_err(|error| DouyuApiError::InvalidResponse {
            endpoint,
            detail: error.to_string(),
        })?;
    if response.error != 0 {
        return Err(classify_api_response(
            endpoint,
            response.error,
            &response.msg,
        ));
    }

    response.data.ok_or_else(|| DouyuApiError::InvalidResponse {
        endpoint,
        detail: "RoomApi returned no room data".to_string(),
    })
}

/// Fetch and cache the encryption key used for H5 signing.
async fn get_encryption_key(
    client: &Client,
    account: &Account,
    room_id: u64,
    cache: &EncryptionCache,
    encryption_endpoint: &str,
) -> Result<CachedEncryptionKey, DouyuApiError> {
    if let Some(cached) = cache.read().await.clone() {
        if cached.is_valid() {
            return Ok(cached);
        }
    }

    let endpoint = "getEncryption";
    let request = client
        .get(encryption_endpoint)
        .query(&[("did", DEFAULT_DID)])
        .headers(request_headers(account, room_id, endpoint)?);
    let (status, body) = send_text(request, endpoint).await?;

    if !status.is_success() {
        if is_auth_failure(status, &body) {
            return Err(DouyuApiError::Authentication {
                detail: format!("getEncryption HTTP {status}"),
            });
        }
        return Err(DouyuApiError::Http { endpoint, status });
    }

    let response = response::parse_encryption_response(&body).map_err(|error| {
        DouyuApiError::InvalidResponse {
            endpoint,
            detail: error.to_string(),
        }
    })?;
    if response.error != 0 {
        return Err(classify_api_response(
            endpoint,
            response.error,
            &response.msg,
        ));
    }

    let data = response
        .data
        .ok_or_else(|| DouyuApiError::InvalidResponse {
            endpoint,
            detail: "getEncryption returned no key data".to_string(),
        })?;
    let cached = CachedEncryptionKey::new(data, DEFAULT_USER_AGENT.to_string());
    *cache.write().await = Some(cached.clone());
    Ok(cached)
}

/// Sign one H5 request with the key returned by `getEncryption`.
///
/// Douyu first hashes `rand_str + key` `enc_time` times, then appends the key
/// and either an empty salt (special keys) or `rid + timestamp`.
fn md5_signature(data: &DouyuEncryptionData, room_id: u64, timestamp: u64) -> String {
    let mut secret = data.rand_str.clone();
    for _ in 0..data.enc_time.min(response::MAX_ENCRYPTION_ITERATIONS) {
        secret = format!(
            "{:x}",
            md5::compute(format!("{secret}{}", data.key).as_bytes())
        );
    }

    let salt = if data.is_special {
        String::new()
    } else {
        format!("{room_id}{timestamp}")
    };
    format!(
        "{:x}",
        md5::compute(format!("{secret}{}{salt}", data.key).as_bytes())
    )
}

/// Form fields expected by `getH5PlayV1/{rid}`.
fn build_play_form(
    data: &DouyuEncryptionData,
    room_id: u64,
    timestamp: u64,
    cdn: &str,
    rate: u64,
) -> Vec<(String, String)> {
    vec![
        ("enc_data".to_string(), data.enc_data.clone()),
        ("tt".to_string(), timestamp.to_string()),
        ("did".to_string(), DEFAULT_DID.to_string()),
        ("auth".to_string(), md5_signature(data, room_id, timestamp)),
        ("cdn".to_string(), cdn.to_string()),
        ("rate".to_string(), rate.to_string()),
        ("hevc".to_string(), "0".to_string()),
        ("fa".to_string(), "0".to_string()),
        ("ive".to_string(), "0".to_string()),
    ]
}

/// Get one temporary FLV URL. An authentication failure invalidates the cached
/// key and retries exactly once with a newly fetched key.
pub async fn get_stream_url(
    client: &Client,
    account: &Account,
    room_id: u64,
    cache: &EncryptionCache,
) -> Result<String, DouyuApiError> {
    Ok(
        get_stream_url_avoiding(client, account, room_id, cache, None)
            .await?
            .url,
    )
}

/// Select an alternate CDN after a previous recording attempt failed. The
/// signed URL itself is never pre-fetched or logged.
pub(crate) async fn get_stream_url_avoiding(
    client: &Client,
    account: &Account,
    room_id: u64,
    cache: &EncryptionCache,
    excluded_cdn: Option<&str>,
) -> Result<DouyuSelectedStream, DouyuApiError> {
    get_stream_url_at(
        client,
        account,
        room_id,
        cache,
        DouyuPlayEndpoints {
            encryption: ENCRYPTION_API,
            play: PLAY_API_BASE,
        },
        unix_timestamp,
        DouyuSelectionOptions {
            budget: STREAM_SELECTION_TIMEOUT,
            excluded_cdn,
        },
    )
    .await
}

#[derive(Debug)]
pub(crate) struct DouyuSelectedStream {
    pub url: String,
    pub cdn: String,
}

#[derive(Debug)]
struct DouyuStreamCandidate {
    url: Option<String>,
    cdns: Vec<String>,
    preferred_rate: Option<u64>,
}

#[derive(Clone, Copy)]
struct DouyuPlayEndpoints<'a> {
    encryption: &'a str,
    play: &'a str,
}

#[derive(Clone, Copy)]
struct DouyuPlaySelection<'a> {
    cdn: &'a str,
    rate: u64,
}

#[derive(Clone, Copy)]
struct DouyuSelectionOptions<'a> {
    budget: Duration,
    excluded_cdn: Option<&'a str>,
}

/// Request Douyu's automatic CDN selection and prefer a supported live quality
/// over the highest-bitrate source. Never probe the signed FLV URL: a GET can
/// consume its live session, leaving only a few seconds for FFmpeg's next GET.
async fn get_stream_url_at<F>(
    client: &Client,
    account: &Account,
    room_id: u64,
    cache: &EncryptionCache,
    endpoints: DouyuPlayEndpoints<'_>,
    timestamp: F,
    options: DouyuSelectionOptions<'_>,
) -> Result<DouyuSelectedStream, DouyuApiError>
where
    F: FnMut() -> u64 + Send,
{
    let deadline = Instant::now() + options.budget;
    let mut pending_cdns = VecDeque::from([DEFAULT_CDN.to_string()]);
    let mut tried_cdns = HashSet::new();
    let mut last_error = None;
    let mut timestamp = timestamp;
    let mut selected_rate = DEFAULT_RATE;
    let mut source_fallback = None;

    while let Some(cdn) = pending_cdns.pop_front() {
        if !tried_cdns.insert(cdn.clone()) || cdn.starts_with("scdn") {
            continue;
        }

        let candidate = match timeout_at(
            deadline,
            get_stream_candidate_at(
                client,
                account,
                room_id,
                cache,
                DouyuPlaySelection {
                    cdn: &cdn,
                    rate: selected_rate,
                },
                endpoints,
                &mut timestamp,
            ),
        )
        .await
        {
            Ok(Ok(candidate)) => candidate,
            Ok(Err(error)) if tried_cdns.len() == 1 => return Err(error),
            Ok(Err(error)) => {
                last_error = Some(error);
                continue;
            }
            Err(_) => return source_fallback.ok_or_else(stream_selection_timeout),
        };

        // A rate-specific response may omit the advertised CDN list. Keep
        // routes from the initial request even if the preferred rate fails.
        for alternate in &candidate.cdns {
            if !alternate.is_empty() && !tried_cdns.contains(alternate) {
                pending_cdns.push_back(alternate.clone());
            }
        }

        if cdn == DEFAULT_CDN && selected_rate == DEFAULT_RATE {
            if options.excluded_cdn != Some(cdn.as_str()) {
                source_fallback = valid_flv_url(&candidate).map(|url| DouyuSelectedStream {
                    url,
                    cdn: cdn.clone(),
                });
            }
            if let Some(rate) = candidate
                .preferred_rate
                .filter(|rate| *rate != DEFAULT_RATE)
            {
                selected_rate = rate;
                if options.excluded_cdn == Some(cdn.as_str()) {
                    // The initial request only discovered qualities and CDN
                    // routes. Do not spend the remaining budget signing the
                    // CDN that just failed; try its advertised alternatives.
                    continue;
                }
                // Use a new signature for the advertised quality, but retain
                // the source URL in case that quality/CDN is unavailable.
                match timeout_at(
                    deadline,
                    get_stream_candidate_at(
                        client,
                        account,
                        room_id,
                        cache,
                        DouyuPlaySelection { cdn: &cdn, rate },
                        endpoints,
                        &mut timestamp,
                    ),
                )
                .await
                {
                    Ok(Ok(preferred)) => {
                        for alternate in &preferred.cdns {
                            if !alternate.is_empty() && !tried_cdns.contains(alternate) {
                                pending_cdns.push_back(alternate.clone());
                            }
                        }
                        if options.excluded_cdn != Some(cdn.as_str()) {
                            if let Some(url) = valid_flv_url(&preferred) {
                                log::info!(
                                    "[Douyu][{room_id}] Selected FLV rate {rate}, CDN {cdn:?}"
                                );
                                return Ok(DouyuSelectedStream { url, cdn });
                            }
                        }
                        last_error = Some(DouyuApiError::InvalidResponse {
                            endpoint: "getH5PlayV1",
                            detail: format!(
                                "CDN {cdn} did not return an AVC FLV URL at rate {rate}"
                            ),
                        });
                        continue;
                    }
                    Ok(Err(error)) => {
                        log::warn!("[Douyu][{room_id}] Preferred rate {rate} failed: {error}");
                        selected_rate = DEFAULT_RATE;
                        last_error = Some(error);
                    }
                    Err(_) => return source_fallback.ok_or_else(stream_selection_timeout),
                }
            }
        }

        if options.excluded_cdn == Some(cdn.as_str()) {
            continue;
        }
        if let Some(url) = valid_flv_url(&candidate) {
            log::info!("[Douyu][{room_id}] Selected FLV rate {selected_rate}, CDN {cdn:?}");
            return Ok(DouyuSelectedStream { url, cdn });
        }
        last_error = Some(DouyuApiError::InvalidResponse {
            endpoint: "getH5PlayV1",
            detail: format!("CDN {cdn} did not return an AVC FLV URL"),
        });
    }

    source_fallback.ok_or_else(|| {
        last_error.unwrap_or_else(|| DouyuApiError::InvalidResponse {
            endpoint: "getH5PlayV1",
            detail: "Douyu did not advertise any usable FLV CDN".to_string(),
        })
    })
}

fn valid_flv_url(candidate: &DouyuStreamCandidate) -> Option<String> {
    let url = candidate.url.as_ref()?;
    let parsed = url::Url::parse(url).ok()?;
    (matches!(parsed.scheme(), "http" | "https") && parsed.path().ends_with(".flv"))
        .then(|| url.clone())
}

fn stream_selection_timeout() -> DouyuApiError {
    DouyuApiError::InvalidResponse {
        endpoint: "getH5PlayV1",
        detail: "timed out selecting a playable FLV CDN".to_string(),
    }
}

async fn get_stream_candidate_at<F>(
    client: &Client,
    account: &Account,
    room_id: u64,
    cache: &EncryptionCache,
    selection: DouyuPlaySelection<'_>,
    endpoints: DouyuPlayEndpoints<'_>,
    mut timestamp: F,
) -> Result<DouyuStreamCandidate, DouyuApiError>
where
    F: FnMut() -> u64 + Send,
{
    let endpoint = "getH5PlayV1";

    let mut minimum_retry_timestamp = None;
    for attempt in 0..2 {
        if attempt > 0 {
            *cache.write().await = None;
        }

        let key = get_encryption_key(client, account, room_id, cache, endpoints.encryption).await?;
        let mut request_timestamp = timestamp();
        while minimum_retry_timestamp.is_some_and(|previous| request_timestamp <= previous) {
            sleep(std::time::Duration::from_millis(50)).await;
            request_timestamp = timestamp();
        }
        let form = build_play_form(
            &key.data,
            room_id,
            request_timestamp,
            selection.cdn,
            selection.rate,
        );
        let request = client
            .post(format!(
                "{}/{room_id}",
                endpoints.play.trim_end_matches('/')
            ))
            .headers(request_headers_with_user_agent(
                account,
                room_id,
                endpoint,
                &key.user_agent,
            )?);
        let (status, body) = send_text(request.form(&form), endpoint).await?;

        if is_auth_failure(status, &body) {
            if attempt == 0 {
                continue;
            }
            return Err(DouyuApiError::Authentication {
                detail: format!("getH5PlayV1 HTTP {status}"),
            });
        }
        if !status.is_success() {
            return Err(DouyuApiError::Http { endpoint, status });
        }

        let response = response::parse_play_response(&body).map_err(|error| {
            DouyuApiError::InvalidResponse {
                endpoint,
                detail: error.to_string(),
            }
        })?;
        if response.error != 0 {
            if response::is_auth_failure_text(&response.msg) {
                if attempt == 0 {
                    continue;
                }
                return Err(DouyuApiError::Authentication {
                    detail: response.msg,
                });
            }
            // Douyu uses -9 for a timestamp mismatch. Re-sign once using a
            // fresh timestamp before surfacing the API error.
            if response.error == -9 && attempt == 0 {
                minimum_retry_timestamp = Some(request_timestamp);
                continue;
            }
            return Err(classify_api_response(
                endpoint,
                response.error,
                &response.msg,
            ));
        }

        let data = response
            .data
            .ok_or_else(|| DouyuApiError::InvalidResponse {
                endpoint,
                detail: "getH5PlayV1 returned no play data".to_string(),
            })?;
        let preferred_rate = data
            .multirates
            .iter()
            .filter(|quality| quality.bit > 0 && quality.bit <= PREFERRED_MAX_BITRATE_KBPS)
            .max_by_key(|quality| quality.bit)
            .map(|quality| quality.rate);
        let url = response::flv_url(&data);
        let cdns = data
            .cdns
            .into_iter()
            .map(|candidate| candidate.cdn)
            .collect();
        return Ok(DouyuStreamCandidate {
            url,
            cdns,
            preferred_rate,
        });
    }

    unreachable!("the two-attempt authentication loop always returns")
}

fn request_headers(
    account: &Account,
    room_id: u64,
    endpoint: &'static str,
) -> Result<HeaderMap, DouyuApiError> {
    request_headers_with_user_agent(account, room_id, endpoint, DEFAULT_USER_AGENT)
}

fn request_headers_with_user_agent(
    account: &Account,
    _room_id: u64,
    endpoint: &'static str,
    user_agent: &str,
) -> Result<HeaderMap, DouyuApiError> {
    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_str(user_agent).map_err(|_| DouyuApiError::InvalidHeader {
            endpoint,
            name: "User-Agent",
        })?,
    );
    headers.insert(REFERER, HeaderValue::from_static(DOUYU_REFERER));
    if !account.cookies.trim().is_empty() {
        headers.insert(
            COOKIE,
            HeaderValue::from_str(&account.cookies).map_err(|_| DouyuApiError::InvalidHeader {
                endpoint,
                name: "Cookie",
            })?,
        );
    }
    Ok(headers)
}

async fn send_text(
    request: RequestBuilder,
    endpoint: &'static str,
) -> Result<(StatusCode, String), DouyuApiError> {
    let response = request
        .send()
        .await
        .map_err(|source| DouyuApiError::Request { endpoint, source })?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|source| DouyuApiError::Request { endpoint, source })?;
    Ok((status, body))
}

fn classify_api_response(endpoint: &'static str, code: i32, message: &str) -> DouyuApiError {
    if response::is_offline_error(code, message) {
        return DouyuApiError::Offline {
            detail: format!("{endpoint} error {code}: {message}"),
        };
    }
    if response::is_auth_failure_text(message) {
        return DouyuApiError::Authentication {
            detail: format!("{endpoint} error {code}: {message}"),
        };
    }
    DouyuApiError::Api {
        endpoint,
        code,
        message: message.to_string(),
    }
}

fn is_auth_failure(status: StatusCode, body: &str) -> bool {
    status == StatusCode::UNAUTHORIZED
        || status == StatusCode::FORBIDDEN
        || response::is_auth_failure_text(body)
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platforms::douyu::response::DouyuEncryptionData;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use wiremock::matchers::{method, path as url_path};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    fn selection_options(budget: Duration) -> DouyuSelectionOptions<'static> {
        DouyuSelectionOptions {
            budget,
            excluded_cdn: None,
        }
    }

    fn test_key() -> DouyuEncryptionData {
        DouyuEncryptionData {
            rand_str: "rand".to_string(),
            enc_time: 1,
            expire_at: 0,
            key: "key".to_string(),
            is_special: false,
            enc_data: "encoded".to_string(),
        }
    }

    fn encryption_body() -> &'static str {
        r#"{"error":0,"data":{"rand_str":"rand","enc_time":1,"expire_at":2000000000,"key":"key","is_special":false,"enc_data":"encoded"}}"#
    }

    fn successful_play_body() -> &'static str {
        r#"{"error":0,"msg":"ok","data":{"room_id":123,"rtmp_url":"https://cdn.example/live","rtmp_live":"stream.flv?token=abc"}}"#
    }

    async fn mock_play_retry(first_play_body: &'static str) -> (String, Vec<Request>) {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(url_path("/encrypt"))
            .respond_with(ResponseTemplate::new(200).set_body_string(encryption_body()))
            .expect(2)
            .mount(&server)
            .await;

        let play_attempts = Arc::new(AtomicUsize::new(0));
        let attempts = play_attempts.clone();
        Mock::given(method("POST"))
            .and(url_path("/play/123"))
            .respond_with(move |_: &Request| {
                let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                let body = if attempt == 0 {
                    first_play_body
                } else {
                    successful_play_body()
                };
                ResponseTemplate::new(200).set_body_string(body)
            })
            .expect(2)
            .mount(&server)
            .await;

        let clock = Arc::new(AtomicU64::new(100));
        let clock_for_request = clock.clone();
        let cache = Arc::new(RwLock::new(None));
        let encryption_endpoint = format!("{}/encrypt", server.uri());
        let play_api_base = format!("{}/play", server.uri());
        let candidate = get_stream_candidate_at(
            &Client::new(),
            &Account::default(),
            123,
            &cache,
            DouyuPlaySelection {
                cdn: DEFAULT_CDN,
                rate: DEFAULT_RATE,
            },
            DouyuPlayEndpoints {
                encryption: &encryption_endpoint,
                play: &play_api_base,
            },
            move || clock_for_request.fetch_add(1, Ordering::SeqCst),
        )
        .await
        .unwrap();
        let requests = server.received_requests().await.unwrap();
        (candidate.url.unwrap(), requests)
    }

    fn form_value(request: &Request, name: &str) -> String {
        form_value_from_body(std::str::from_utf8(&request.body).unwrap(), name)
    }

    fn form_value_from_body(body: &str, name: &str) -> String {
        url::form_urlencoded::parse(body.as_bytes())
            .find_map(|(key, value)| (key == name).then(|| value.into_owned()))
            .unwrap()
    }

    #[tokio::test]
    async fn authentication_failure_refetches_key_and_retries_play_request() {
        let (url, requests) = mock_play_retry(r#"{"error":-7,"msg":"鉴权失败","data":""}"#).await;
        let play_requests: Vec<_> = requests
            .iter()
            .filter(|request| request.url.path() == "/play/123")
            .collect();
        let encryption_requests = requests
            .iter()
            .filter(|request| request.url.path() == "/encrypt")
            .count();

        assert_eq!(url, "https://cdn.example/live/stream.flv?token=abc");
        assert_eq!(encryption_requests, 2);
        assert_eq!(form_value(play_requests[0], "tt"), "100");
        assert_eq!(form_value(play_requests[1], "tt"), "101");
    }

    #[tokio::test]
    async fn timestamp_error_retries_play_request_with_fresh_timestamp() {
        let (url, requests) =
            mock_play_retry(r#"{"error":-9,"msg":"room_bus_checksevertime","data":""}"#).await;
        let play_requests: Vec<_> = requests
            .iter()
            .filter(|request| request.url.path() == "/play/123")
            .collect();

        assert_eq!(url, "https://cdn.example/live/stream.flv?token=abc");
        assert_eq!(form_value(play_requests[0], "tt"), "100");
        assert_eq!(form_value(play_requests[1], "tt"), "101");
    }

    #[tokio::test]
    async fn stream_url_tries_advertised_cdn_when_default_is_not_flv() {
        let server = MockServer::start().await;
        let base = server.uri();
        Mock::given(method("GET"))
            .and(url_path("/encrypt"))
            .respond_with(ResponseTemplate::new(200).set_body_string(encryption_body()))
            .expect(1)
            .mount(&server)
            .await;

        let base_for_play = base.clone();
        Mock::given(method("POST"))
            .and(url_path("/play/123"))
            .respond_with(move |request: &Request| {
                let form = std::str::from_utf8(&request.body).unwrap();
                let cdn = form_value_from_body(form, "cdn");
                let data = match cdn.as_str() {
                    "" => serde_json::json!({
                        "room_id": 123,
                        "rtmp_url": base_for_play.clone(),
                        "rtmp_live": "playlist.m3u8",
                        "cdnsWithName": [{"cdn": "ws-alt"}]
                    }),
                    "ws-alt" => serde_json::json!({
                        "room_id": 123,
                        "rtmp_url": "",
                        "rtmp_live": "",
                        "cdnsWithName": [{"cdn": "ws-alt2"}]
                    }),
                    _ => serde_json::json!({
                        "room_id": 123,
                        "rtmp_url": base_for_play.clone(),
                        "rtmp_live": "good.flv",
                        "cdnsWithName": [{"cdn": ""}]
                    }),
                };
                ResponseTemplate::new(200)
                    .set_body_string(serde_json::json!({"error": 0, "data": data}).to_string())
            })
            .expect(3)
            .mount(&server)
            .await;
        let cache = Arc::new(RwLock::new(None));
        let clock = Arc::new(AtomicU64::new(100));
        let clock_for_request = clock.clone();
        let encryption_endpoint = format!("{base}/encrypt");
        let play_api_base = format!("{base}/play");
        let url = get_stream_url_at(
            &Client::new(),
            &Account::default(),
            123,
            &cache,
            DouyuPlayEndpoints {
                encryption: &encryption_endpoint,
                play: &play_api_base,
            },
            move || clock_for_request.fetch_add(1, Ordering::SeqCst),
            selection_options(Duration::from_secs(30)),
        )
        .await
        .unwrap();

        let requests = server.received_requests().await.unwrap();
        let requested_cdns: Vec<_> = requests
            .iter()
            .filter(|request| {
                request.method.as_str() == "POST" && request.url.path() == "/play/123"
            })
            .map(|request| form_value(request, "cdn"))
            .collect();
        assert_eq!(requested_cdns, vec!["", "ws-alt", "ws-alt2"]);
        assert_eq!(url.url, format!("{base}/good.flv"));
    }

    #[tokio::test]
    async fn selects_advertised_quality_without_consuming_the_signed_url() {
        let server = MockServer::start().await;
        let base = server.uri();
        Mock::given(method("GET"))
            .and(url_path("/encrypt"))
            .respond_with(ResponseTemplate::new(200).set_body_string(encryption_body()))
            .mount(&server)
            .await;
        let base_for_play = base.clone();
        Mock::given(method("POST"))
            .and(url_path("/play/123"))
            .respond_with(move |request: &Request| {
                let rate =
                    form_value_from_body(std::str::from_utf8(&request.body).unwrap(), "rate");
                let (live, multirates) = if rate == "4" {
                    ("live.flv", serde_json::json!([]))
                } else {
                    (
                        "source.flv",
                        serde_json::json!([
                            {"rate": 0, "bit": 24217},
                            {"rate": 4, "bit": 4000},
                            {"rate": 3, "bit": 2000}
                        ]),
                    )
                };
                ResponseTemplate::new(200).set_body_string(
                    serde_json::json!({
                        "error": 0,
                        "data": {
                            "rtmp_url": base_for_play.clone(),
                            "rtmp_live": live,
                            "multirates": multirates,
                            "cdnsWithName": [{"cdn": "scdnctshh"}, {"cdn": "hw-h5"}]
                        }
                    })
                    .to_string(),
                )
            })
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(url_path("/source.flv"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"FLV\x01".to_vec()))
            .expect(0)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(url_path("/live.flv"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"FLV\x01".to_vec()))
            .expect(0)
            .mount(&server)
            .await;

        let cache = Arc::new(RwLock::new(None));
        let url = get_stream_url_at(
            &Client::new(),
            &Account::default(),
            123,
            &cache,
            DouyuPlayEndpoints {
                encryption: &format!("{base}/encrypt"),
                play: &format!("{base}/play"),
            },
            unix_timestamp,
            selection_options(Duration::from_secs(30)),
        )
        .await
        .unwrap();
        assert_eq!(url.url, format!("{base}/live.flv"));
        let requests = server.received_requests().await.unwrap();
        let play_requests: Vec<_> = requests
            .iter()
            .filter(|request| request.url.path() == "/play/123")
            .collect();
        assert_eq!(
            play_requests
                .iter()
                .map(|request| form_value(request, "cdn"))
                .collect::<Vec<_>>(),
            ["", ""]
        );
        assert_eq!(
            play_requests
                .iter()
                .map(|request| form_value(request, "rate"))
                .collect::<Vec<_>>(),
            ["0", "4"]
        );
    }

    #[tokio::test]
    async fn preserves_advertised_cdn_when_selected_rate_omits_it() {
        let server = MockServer::start().await;
        let base = server.uri();
        Mock::given(method("GET"))
            .and(url_path("/encrypt"))
            .respond_with(ResponseTemplate::new(200).set_body_string(encryption_body()))
            .mount(&server)
            .await;
        let base_for_play = base.clone();
        Mock::given(method("POST"))
            .and(url_path("/play/123"))
            .respond_with(move |request: &Request| {
                let cdn = form_value(request, "cdn");
                let rate = form_value(request, "rate");
                let data = if rate == "0" {
                    serde_json::json!({
                        "rtmp_url": base_for_play.clone(),
                        "rtmp_live": "source.flv",
                        "multirates": [{"rate": 4, "bit": 4000}],
                        "cdnsWithName": [{"cdn": "hw-h5"}]
                    })
                } else if cdn.is_empty() {
                    serde_json::json!({"rtmp_url": "", "rtmp_live": ""})
                } else {
                    serde_json::json!({"rtmp_url": base_for_play.clone(), "rtmp_live": "live.flv"})
                };
                ResponseTemplate::new(200)
                    .set_body_string(serde_json::json!({"error": 0, "data": data}).to_string())
            })
            .expect(3)
            .mount(&server)
            .await;
        let cache = Arc::new(RwLock::new(None));
        let url = get_stream_url_at(
            &Client::new(),
            &Account::default(),
            123,
            &cache,
            DouyuPlayEndpoints {
                encryption: &format!("{base}/encrypt"),
                play: &format!("{base}/play"),
            },
            unix_timestamp,
            selection_options(Duration::from_secs(30)),
        )
        .await
        .unwrap();
        assert_eq!(url.url, format!("{base}/live.flv"));
        let requests = server.received_requests().await.unwrap();
        let routes: Vec<_> = requests
            .iter()
            .filter(|request| request.url.path() == "/play/123")
            .map(|request| (form_value(request, "cdn"), form_value(request, "rate")))
            .collect();
        assert_eq!(
            routes,
            [
                ("".into(), "0".into()),
                ("".into(), "4".into()),
                ("hw-h5".into(), "4".into())
            ]
        );
    }

    #[tokio::test]
    async fn skips_a_failed_automatic_cdn_without_probing_its_signed_url() {
        let server = MockServer::start().await;
        let base = server.uri();
        Mock::given(method("GET"))
            .and(url_path("/encrypt"))
            .respond_with(ResponseTemplate::new(200).set_body_string(encryption_body()))
            .mount(&server)
            .await;
        let base_for_play = base.clone();
        Mock::given(method("POST"))
            .and(url_path("/play/123"))
            .respond_with(move |request: &Request| {
                let live = if form_value(request, "cdn").is_empty() {
                    "dead.flv"
                } else {
                    "working.flv"
                };
                ResponseTemplate::new(200).set_body_string(
                    serde_json::json!({"error": 0, "data": {
                        "rtmp_url": base_for_play.clone(), "rtmp_live": live,
                        "multirates": [{"rate": 4, "bit": 4000}],
                        "cdnsWithName": [{"cdn": "hw-h5"}]
                    }})
                    .to_string(),
                )
            })
            .expect(2)
            .mount(&server)
            .await;
        let mut options = selection_options(Duration::from_secs(30));
        options.excluded_cdn = Some(DEFAULT_CDN);
        let chosen = get_stream_url_at(
            &Client::new(),
            &Account::default(),
            123,
            &Arc::new(RwLock::new(None)),
            DouyuPlayEndpoints {
                encryption: &format!("{base}/encrypt"),
                play: &format!("{base}/play"),
            },
            unix_timestamp,
            options,
        )
        .await
        .unwrap();
        assert_eq!(chosen.cdn, "hw-h5");
        assert_eq!(chosen.url, format!("{base}/working.flv"));
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 3); // GET key + 2 POST plays; no FLV GET
        let plays: Vec<_> = requests
            .iter()
            .filter(|request| request.url.path() == "/play/123")
            .map(|request| (form_value(request, "cdn"), form_value(request, "rate")))
            .collect();
        assert_eq!(
            plays,
            [("".into(), "0".into()), ("hw-h5".into(), "4".into())]
        );
    }

    #[tokio::test]
    async fn falls_back_to_source_after_all_preferred_cdns_lack_flv() {
        let server = MockServer::start().await;
        let base = server.uri();
        Mock::given(method("GET"))
            .and(url_path("/encrypt"))
            .respond_with(ResponseTemplate::new(200).set_body_string(encryption_body()))
            .mount(&server)
            .await;
        let base_for_play = base.clone();
        Mock::given(method("POST"))
            .and(url_path("/play/123"))
            .respond_with(move |request: &Request| {
                let source = form_value(request, "rate") == "0";
                ResponseTemplate::new(200).set_body_string(
                    serde_json::json!({"error": 0, "data": {
                        "rtmp_url": base_for_play.clone(),
                        "rtmp_live": if source { "source.flv" } else { "not-flv.m3u8" },
                        "multirates": if source { serde_json::json!([{"rate": 4, "bit": 4000}]) } else { serde_json::json!([]) },
                        "cdnsWithName": [{"cdn": "hw-h5"}]
                    }})
                    .to_string(),
                )
            })
            .expect(3)
            .mount(&server)
            .await;
        let chosen = get_stream_url_at(
            &Client::new(),
            &Account::default(),
            123,
            &Arc::new(RwLock::new(None)),
            DouyuPlayEndpoints {
                encryption: &format!("{base}/encrypt"),
                play: &format!("{base}/play"),
            },
            unix_timestamp,
            selection_options(Duration::from_secs(30)),
        )
        .await
        .unwrap();
        assert_eq!(chosen.url, format!("{base}/source.flv"));
        assert_eq!(chosen.cdn, DEFAULT_CDN);
    }

    #[tokio::test]
    async fn preferred_rate_timeout_keeps_the_already_signed_source() {
        let server = MockServer::start().await;
        let base = server.uri();
        Mock::given(method("GET"))
            .and(url_path("/encrypt"))
            .respond_with(ResponseTemplate::new(200).set_body_string(encryption_body()))
            .mount(&server)
            .await;
        let base_for_play = base.clone();
        Mock::given(method("POST"))
            .and(url_path("/play/123"))
            .respond_with(move |request: &Request| {
                let source = form_value(request, "rate") == "0";
                let response = ResponseTemplate::new(200).set_body_string(
                    serde_json::json!({"error": 0, "data": {
                        "rtmp_url": base_for_play.clone(),
                        "rtmp_live": if source { "source.flv" } else { "preferred.flv" },
                        "multirates": [{"rate": 4, "bit": 4000}]
                    }})
                    .to_string(),
                );
                if source {
                    response
                } else {
                    response.set_delay(Duration::from_secs(2))
                }
            })
            .expect(2)
            .mount(&server)
            .await;
        let chosen = get_stream_url_at(
            &Client::new(),
            &Account::default(),
            123,
            &Arc::new(RwLock::new(None)),
            DouyuPlayEndpoints {
                encryption: &format!("{base}/encrypt"),
                play: &format!("{base}/play"),
            },
            unix_timestamp,
            selection_options(Duration::from_millis(500)),
        )
        .await
        .unwrap();
        assert_eq!(chosen.url, format!("{base}/source.flv"));
    }

    #[tokio::test]
    async fn stream_selection_budget_covers_api_requests() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(url_path("/encrypt"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_millis(100))
                    .set_body_string(encryption_body()),
            )
            .mount(&server)
            .await;
        let encryption_endpoint = format!("{}/encrypt", server.uri());
        let play_api_base = format!("{}/play", server.uri());
        let cache = Arc::new(RwLock::new(None));
        let error = get_stream_url_at(
            &Client::new(),
            &Account::default(),
            123,
            &cache,
            DouyuPlayEndpoints {
                encryption: &encryption_endpoint,
                play: &play_api_base,
            },
            unix_timestamp,
            selection_options(Duration::from_millis(30)),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(error, DouyuApiError::InvalidResponse { detail, .. } if detail.contains("timed out selecting"))
        );
    }

    #[tokio::test]
    async fn topic_room_query_id_is_used_before_vanity_resolution() {
        let room_id = resolve_room_id(
            &Client::new(),
            &Account::default(),
            "https://www.douyu.com/topic/lpl?rid=123456",
        )
        .await
        .unwrap();
        assert_eq!(room_id, 123456);
    }

    #[test]
    fn md5_signature_matches_douyu_fallback_algorithm() {
        // md5("randkey") = c46d1fc27e9e013e0efb4c314e063511, then the final
        // input is that digest + "key" + "123456".
        assert_eq!(
            md5_signature(&test_key(), 123, 456),
            "77467e273c90028657264b6061592b5b"
        );
    }

    #[test]
    fn special_key_omits_room_and_timestamp_salt() {
        let mut key = test_key();
        key.enc_time = 0;
        key.is_special = true;
        assert_eq!(
            md5_signature(&key, 123, 456),
            "c46d1fc27e9e013e0efb4c314e063511"
        );
    }

    #[test]
    fn play_form_contains_signed_request_fields() {
        let form = build_play_form(&test_key(), 123, 456, DEFAULT_CDN, DEFAULT_RATE);
        let fields: std::collections::HashMap<_, _> = form.into_iter().collect();

        assert_eq!(fields.get("tt").map(String::as_str), Some("456"));
        assert_eq!(fields.get("cdn").map(String::as_str), Some(""));
        assert_eq!(fields.get("rate").map(String::as_str), Some("0"));
        assert_eq!(fields.get("hevc").map(String::as_str), Some("0"));
        assert_eq!(fields.get("enc_data").map(String::as_str), Some("encoded"));
        assert_eq!(fields.len(), 9);
        assert_eq!(
            fields.get("auth").map(String::as_str),
            Some("77467e273c90028657264b6061592b5b")
        );
    }

    /// Run the actual Douyu recording loop for five wall-clock minutes. Short
    /// CDN disconnects may resume the same archive with a fresh signed URL.
    #[tokio::test]
    #[ignore = "requires five minutes, a live Douyu room, ffmpeg and network access"]
    async fn live_room_records_continuously_for_five_minutes() {
        use crate::traits::RecorderTrait;
        use std::sync::atomic::AtomicU64;

        let _ = env_logger::builder()
            .filter_module("recorder::platforms", log::LevelFilter::Info)
            .is_test(true)
            .try_init();
        let room_id = std::env::var("DOUYU_TEST_ROOM_ID")
            .expect("set DOUYU_TEST_ROOM_ID to a currently live numeric room")
            .parse::<u64>()
            .expect("DOUYU_TEST_ROOM_ID must be numeric");
        let account = Account::default();
        let room = get_room_info(&Client::new(), &account, room_id)
            .await
            .unwrap();
        assert!(response::room_is_live(&room), "room {room_id} is offline");
        let test_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
        std::fs::create_dir_all(&test_root).unwrap();
        let work_dir = tempfile::tempdir_in(test_root).unwrap();
        let (events, _) = tokio::sync::broadcast::channel(32);
        let recorder = crate::platforms::douyu::DouyuRecorder::new(
            &room_id.to_string(),
            &account,
            work_dir.path().to_path_buf(),
            events,
            Arc::new(AtomicU64::new(10)),
            true,
        )
        .unwrap();
        recorder.run().await;
        let started = Instant::now();
        let room_path = work_dir.path().join("douyu").join(room_id.to_string());
        let mut previous_segments = 0;
        let mut previous_duration = 0.0;
        let mut healthy_minutes = 0;
        let test_minutes = std::env::var("DOUYU_TEST_MINUTES")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|minutes| (1..=30).contains(minutes))
            .unwrap_or(5);

        for minute in 1..=test_minutes {
            sleep(Duration::from_secs(60)).await;
            let mut segment_count = 0;
            let mut bytes = 0;
            let mut duration = 0.0;
            let mut archives = 0;
            if let Ok(entries) = std::fs::read_dir(&room_path) {
                for entry in entries.flatten() {
                    let archive = entry.path();
                    let Ok(playlist) = std::fs::read(archive.join("playlist.m3u8")) else {
                        continue;
                    };
                    let Ok((_, parsed)) = m3u8_rs::parse_media_playlist(&playlist) else {
                        continue;
                    };
                    archives += 1;
                    segment_count += parsed.segments.len();
                    for segment in parsed.segments {
                        duration += segment.duration;
                        bytes += std::fs::metadata(archive.join(segment.uri))
                            .map(|metadata| metadata.len())
                            .unwrap_or_default();
                    }
                }
            }
            let new_duration = duration - previous_duration;
            if archives == 1
                && segment_count > previous_segments
                && bytes > 0
                && (45.0..=75.0).contains(&new_duration)
            {
                healthy_minutes += 1;
            }
            println!(
                "minute {minute}: {archives} archives, {segment_count} segments, {duration:.1}s media (+{:.1}s), {bytes} bytes",
                new_duration
            );
            previous_segments = segment_count;
            previous_duration = duration;
        }

        assert!(started.elapsed() >= Duration::from_secs(60 * test_minutes));
        recorder.disable().await;
        recorder.stop().await;
        assert!(
            healthy_minutes == test_minutes
                && (54.0 * test_minutes as f32..=66.0 * test_minutes as f32)
                    .contains(&previous_duration),
            "only {healthy_minutes}/{test_minutes} minutes maintained one archive and 45–75 seconds of new media; total {previous_duration:.1}s"
        );
    }

    #[test]
    fn numeric_room_ids_are_required() {
        assert_eq!(parse_numeric_room_id(" 123 ").unwrap(), 123);
        assert!(parse_numeric_room_id("vanity-name").is_err());
        assert_eq!(
            numeric_rid_query("https://www.douyu.com/topic/lpl?rid=123456"),
            Some(123456)
        );
        assert_eq!(
            numeric_rid_query("bsr://www.douyu.com/topic/lpl?rid=123456#section"),
            Some(123456)
        );
        assert_eq!(numeric_rid_query("https://www.douyu.com/123456"), None);
    }

    #[tokio::test]
    async fn vanity_path_is_resolved_from_the_mobile_room_page() {
        use wiremock::matchers::{method, path as url_path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(url_path("/short-name"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(
                    r#"<script>roomInfo":{"rid":123456,"roomName":"room"}</script>"#,
                ),
            )
            .mount(&server)
            .await;

        let room_id = resolve_room_id_at(
            &Client::new(),
            &Account::default(),
            "https://www.douyu.com/short-name",
            &server.uri(),
        )
        .await
        .unwrap();
        assert_eq!(room_id, 123456);
    }

    #[test]
    fn embedded_room_id_is_extracted_from_mobile_page() {
        assert_eq!(
            extract_real_room_id(r#"<script>roomInfo":{"rid":987654,"roomName":"test"}</script>"#),
            Some(987654)
        );
        assert_eq!(
            extract_real_room_id(r#"roomInfo&quot;:{&quot;rid&quot;:123456}"#),
            Some(123456)
        );
        assert_eq!(extract_real_room_id("roomInfo: {}"), None);
    }

    #[test]
    fn pull_identity_contains_referer_and_cookie() {
        let account = Account {
            cookies: "sid=abc".to_string(),
            ..Account::default()
        };
        let (user_agent, headers) = pull_http_identity(&account, 123);
        assert_eq!(user_agent, DEFAULT_USER_AGENT);
        assert!(headers.contains(&("Referer".to_string(), DOUYU_REFERER.to_string())));
        assert!(headers.contains(&("Cookie".to_string(), "sid=abc".to_string())));
    }
}
