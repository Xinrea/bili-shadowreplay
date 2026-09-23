use std::sync::{Arc, LazyLock};
use std::time::{SystemTime, UNIX_EPOCH};

use regex::Regex;
use reqwest::header::{HeaderMap, HeaderValue, COOKIE, REFERER, USER_AGENT};
use reqwest::{Client, RequestBuilder, StatusCode};
use thiserror::Error;
use tokio::sync::RwLock;

use crate::account::Account;
use crate::errors::RecorderError;

use super::response::{self, DouyuEncryptionData, DouyuRoomInfo};

pub const ROOM_API_BASE: &str = "https://open.douyucdn.cn/api/RoomApi/room";
pub const ENCRYPTION_API: &str = "https://www.douyu.com/wgapi/livenc/liveweb/websec/getEncryption";
pub const PLAY_API_BASE: &str = "https://www.douyu.com/lapi/live/getH5PlayV1";
pub const DOUYU_REFERER: &str = "https://www.douyu.com/";
pub const DEFAULT_DID: &str = "10000000000000000000000000001501";
pub const DEFAULT_CDN: &str = "ws-h5";
pub const DEFAULT_RATE: &str = "0";
pub const DEFAULT_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
const KEY_FALLBACK_TTL_SECS: u64 = 24 * 60 * 60;

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
pub fn pull_http_identity(account: &Account, room_id: u64) -> (String, Vec<(String, String)>) {
    let referer = room_referer(room_id);
    let mut headers = vec![("Referer".to_string(), referer)];
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
pub async fn resolve_room_id(
    client: &Client,
    account: &Account,
    room_id: &str,
) -> Result<u64, DouyuApiError> {
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
        .get(format!("https://m.douyu.com/{path}"))
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
) -> Result<CachedEncryptionKey, DouyuApiError> {
    if let Some(cached) = cache.read().await.clone() {
        if cached.is_valid() {
            return Ok(cached);
        }
    }

    let endpoint = "getEncryption";
    let request = client
        .get(ENCRYPTION_API)
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
pub fn md5_signature(data: &DouyuEncryptionData, room_id: u64, timestamp: u64) -> String {
    let mut secret = data.rand_str.clone();
    for _ in 0..data.enc_time {
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
pub fn build_play_form(
    data: &DouyuEncryptionData,
    room_id: u64,
    timestamp: u64,
) -> Vec<(String, String)> {
    vec![
        ("enc_data".to_string(), data.enc_data.clone()),
        ("tt".to_string(), timestamp.to_string()),
        ("did".to_string(), DEFAULT_DID.to_string()),
        ("auth".to_string(), md5_signature(data, room_id, timestamp)),
        ("cdn".to_string(), DEFAULT_CDN.to_string()),
        ("rate".to_string(), DEFAULT_RATE.to_string()),
        ("ver".to_string(), "Douyu_new".to_string()),
        ("iar".to_string(), "0".to_string()),
        ("ive".to_string(), "0".to_string()),
        ("rid".to_string(), room_id.to_string()),
        ("hevc".to_string(), "1".to_string()),
        ("fa".to_string(), "0".to_string()),
        ("sov".to_string(), "0".to_string()),
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
    let endpoint = "getH5PlayV1";

    for attempt in 0..2 {
        if attempt > 0 {
            *cache.write().await = None;
        }

        let key = get_encryption_key(client, account, room_id, cache).await?;
        let timestamp = unix_timestamp();
        let form = build_play_form(&key.data, room_id, timestamp);
        let request = client.post(format!("{PLAY_API_BASE}/{room_id}")).headers(
            request_headers_with_user_agent(account, room_id, endpoint, &key.user_agent)?,
        );
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
        return response::flv_url(&data).ok_or_else(|| DouyuApiError::InvalidResponse {
            endpoint,
            detail: "getH5PlayV1 returned no FLV URL".to_string(),
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
    room_id: u64,
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
    headers.insert(
        REFERER,
        HeaderValue::from_str(&room_referer(room_id)).map_err(|_| {
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
    Ok(headers)
}

fn room_referer(room_id: u64) -> String {
    format!("{DOUYU_REFERER}{room_id}")
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
        let form = build_play_form(&test_key(), 123, 456);
        let fields: std::collections::HashMap<_, _> = form.into_iter().collect();

        assert_eq!(fields.get("rid").map(String::as_str), Some("123"));
        assert_eq!(fields.get("tt").map(String::as_str), Some("456"));
        assert_eq!(fields.get("ver").map(String::as_str), Some("Douyu_new"));
        assert_eq!(fields.get("enc_data").map(String::as_str), Some("encoded"));
        assert_eq!(
            fields.get("auth").map(String::as_str),
            Some("77467e273c90028657264b6061592b5b")
        );
    }

    #[test]
    fn numeric_room_ids_are_required() {
        assert_eq!(parse_numeric_room_id(" 123 ").unwrap(), 123);
        assert!(parse_numeric_room_id("vanity-name").is_err());
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
        assert!(headers.contains(&(
            "Referer".to_string(),
            "https://www.douyu.com/123".to_string()
        )));
        assert!(headers.contains(&("Cookie".to_string(), "sid=abc".to_string())));
    }
}
