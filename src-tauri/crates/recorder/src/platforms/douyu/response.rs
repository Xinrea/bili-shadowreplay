use serde::de::{self, DeserializeOwned};
use serde::{Deserialize, Deserializer};

pub(super) const MAX_ENCRYPTION_ITERATIONS: u32 = 16;

/// The response returned by Douyu's public RoomApi endpoint.
#[derive(Debug, Clone, Deserialize)]
pub struct DouyuRoomInfoResponse {
    #[serde(default)]
    pub error: i32,
    #[serde(default)]
    pub msg: String,
    #[serde(default, deserialize_with = "deserialize_optional_room_info")]
    pub data: Option<DouyuRoomInfo>,
}

/// Room metadata and the live status exposed by RoomApi.
#[derive(Debug, Clone, Deserialize)]
pub struct DouyuRoomInfo {
    #[serde(default, deserialize_with = "deserialize_string")]
    pub room_id: String,
    #[serde(default, deserialize_with = "deserialize_string")]
    pub room_thumb: String,
    #[serde(default, deserialize_with = "deserialize_string")]
    pub room_name: String,
    /// RoomApi uses `"1"` while live and `"2"` while offline.
    #[serde(default, deserialize_with = "deserialize_string")]
    pub room_status: String,
    /// The value is normally a formatted date, but older responses sometimes
    /// expose a unix timestamp. Keep it as a string for a stable live id.
    #[serde(default, deserialize_with = "deserialize_string")]
    pub start_time: String,
    #[serde(default, deserialize_with = "deserialize_string")]
    pub owner_name: String,
    #[serde(default, deserialize_with = "deserialize_string")]
    pub owner_uid: String,
    #[serde(default, deserialize_with = "deserialize_string")]
    pub avatar: String,
    #[serde(default, deserialize_with = "deserialize_u64")]
    pub online: u64,
}

/// The response returned by `getH5PlayV1/{rid}`.
#[derive(Debug, Clone, Deserialize)]
pub struct DouyuH5PlayResponse {
    #[serde(default)]
    pub error: i32,
    #[serde(default)]
    pub msg: String,
    #[serde(default, deserialize_with = "deserialize_optional_play_data")]
    pub data: Option<DouyuH5PlayData>,
}

/// The FLV endpoint information nested in a successful H5 play response.
#[derive(Debug, Clone, Deserialize)]
pub struct DouyuH5PlayData {
    #[allow(dead_code)]
    #[serde(default, deserialize_with = "deserialize_u64")]
    pub room_id: u64,
    #[serde(default, deserialize_with = "deserialize_string")]
    pub rtmp_url: String,
    #[serde(default, deserialize_with = "deserialize_string")]
    pub rtmp_live: String,
    /// Some responses include a direct HEVC URL. It is parsed for compatibility
    /// but not selected because FLV recordings are muxed to TS.
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub player_1: Option<String>,
    #[serde(default, rename = "cdnsWithName")]
    pub cdns: Vec<DouyuCdn>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DouyuCdn {
    #[serde(default, deserialize_with = "deserialize_string")]
    pub cdn: String,
}

/// The response returned by `getEncryption`.
#[derive(Debug, Clone, Deserialize)]
pub struct DouyuEncryptionResponse {
    #[serde(default)]
    pub error: i32,
    #[serde(default)]
    pub msg: String,
    #[serde(default, deserialize_with = "deserialize_optional_encryption_data")]
    pub data: Option<DouyuEncryptionData>,
}

/// Key material used by Douyu's server-side H5 signature.
#[derive(Debug, Clone, Deserialize)]
pub struct DouyuEncryptionData {
    #[serde(default, deserialize_with = "deserialize_string")]
    pub rand_str: String,
    #[serde(default, deserialize_with = "deserialize_u32")]
    pub enc_time: u32,
    #[serde(default, deserialize_with = "deserialize_u64")]
    pub expire_at: u64,
    #[serde(default, deserialize_with = "deserialize_string")]
    pub key: String,
    #[serde(default, deserialize_with = "deserialize_bool")]
    pub is_special: bool,
    #[serde(default, deserialize_with = "deserialize_string")]
    pub enc_data: String,
}

/// Parse a RoomApi response without performing any I/O.
pub fn parse_room_response(body: &str) -> Result<DouyuRoomInfoResponse, serde_json::Error> {
    serde_json::from_str(body)
}

/// Parse a getH5PlayV1 response without performing any I/O.
pub fn parse_play_response(body: &str) -> Result<DouyuH5PlayResponse, serde_json::Error> {
    serde_json::from_str(body)
}

/// Parse a getEncryption response without performing any I/O.
pub fn parse_encryption_response(body: &str) -> Result<DouyuEncryptionResponse, serde_json::Error> {
    serde_json::from_str(body)
}

/// RoomApi's status mapping. `online` is only a fallback for older responses
/// that omitted `room_status`; an explicit offline status always wins.
pub fn map_room_status(room_status: &str, online: u64) -> bool {
    match room_status.trim().to_ascii_lowercase().as_str() {
        "1" | "live" | "living" | "on" => true,
        "2" | "0" | "offline" | "closed" | "close" => false,
        _ => online > 0,
    }
}

pub fn room_is_live(room: &DouyuRoomInfo) -> bool {
    map_room_status(&room.room_status, room.online)
}

/// Use Douyu's opening time as the session id. A room id is a stable fallback
/// for older RoomApi responses that omit `start_time`.
pub fn platform_live_id(room: &DouyuRoomInfo, requested_room_id: u64) -> String {
    let start_time = room.start_time.trim();
    if !start_time.is_empty() && start_time != "0" {
        // Some RoomApi versions return a formatted datetime such as
        // `2025-01-02 03:04:05`. Keep it readable but make it valid as a path
        // component on Windows, where `:` is reserved.
        let safe: String = start_time
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                    character
                } else {
                    '_'
                }
            })
            .collect();
        if !safe.trim_matches('_').is_empty() {
            return safe;
        }
    }
    if !room.room_id.trim().is_empty() {
        return room.room_id.trim().to_string();
    }
    requested_room_id.to_string()
}

/// Douyu uses a handful of negative codes for a room that went offline between
/// the status and play requests. Messages are included because the platform
/// has returned the same condition with different codes over time.
pub fn is_offline_error(code: i32, message: &str) -> bool {
    if (-5..=-3).contains(&code) {
        return true;
    }

    let message = message.to_ascii_lowercase();
    [
        "offline",
        "closeroom",
        "closed",
        "not live",
        "直播已结束",
        "房间关闭",
        "未开播",
        "关播",
        "没有开放",
        "不存在",
    ]
    .iter()
    .any(|needle| message.contains(needle))
}

/// Detect an authentication failure in either an HTTP body or an API message.
pub fn is_auth_failure_text(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    [
        "鉴权",
        "签名",
        "authentication",
        "signature",
        "unauthorized",
        "forbidden",
        "auth failed",
        "token expired",
        "token invalid",
        "invalid token",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

/// Select the AVC-compatible FLV URL from a successful H5 play response.
///
/// Prefer Douyu's normal `rtmp_url` plus `rtmp_live` path. `player_1` is not
/// selected because it commonly points at HEVC, which the FLV-to-TS recorder
/// cannot safely mux.
pub fn flv_url(data: &DouyuH5PlayData) -> Option<String> {
    if data.rtmp_live.trim().is_empty() {
        return None;
    }

    if data.rtmp_live.starts_with("http://") || data.rtmp_live.starts_with("https://") {
        return Some(data.rtmp_live.trim().to_string());
    }

    if data.rtmp_url.trim().is_empty() {
        return Some(data.rtmp_live.trim().to_string());
    }

    Some(format!(
        "{}/{}",
        data.rtmp_url.trim_end_matches('/'),
        data.rtmp_live.trim_start_matches('/')
    ))
}

fn deserialize_optional_room_info<'de, D>(
    deserializer: D,
) -> Result<Option<DouyuRoomInfo>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_optional::<D, DouyuRoomInfo>(deserializer)
}

fn deserialize_optional_play_data<'de, D>(
    deserializer: D,
) -> Result<Option<DouyuH5PlayData>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_optional::<D, DouyuH5PlayData>(deserializer)
}

fn deserialize_optional_encryption_data<'de, D>(
    deserializer: D,
) -> Result<Option<DouyuEncryptionData>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_optional::<D, DouyuEncryptionData>(deserializer)
}

fn deserialize_optional<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    if value.is_null() || value.as_str().is_some_and(str::is_empty) {
        return Ok(None);
    }
    serde_json::from_value(value)
        .map(Some)
        .map_err(de::Error::custom)
}

fn deserialize_optional_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::String(value) if value.is_empty() => Ok(None),
        serde_json::Value::String(value) => Ok(Some(value)),
        value => Ok(Some(value.to_string())),
    }
}

fn deserialize_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Null => Ok(String::new()),
        serde_json::Value::String(value) => Ok(value),
        serde_json::Value::Number(value) => Ok(value.to_string()),
        serde_json::Value::Bool(value) => Ok(value.to_string()),
        value => Err(de::Error::custom(format!("expected scalar, got {value}"))),
    }
}

fn deserialize_u64<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Null => Ok(0),
        serde_json::Value::Number(value) => value
            .as_u64()
            .ok_or_else(|| de::Error::custom("expected an unsigned integer")),
        serde_json::Value::String(value) => value
            .trim()
            .parse()
            .map_err(|_| de::Error::custom("expected an unsigned integer string")),
        value => Err(de::Error::custom(format!("expected integer, got {value}"))),
    }
}

fn deserialize_u32<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_u64(deserializer).and_then(|value| {
        let value = u32::try_from(value).map_err(|_| de::Error::custom("integer exceeds u32"))?;
        if value > MAX_ENCRYPTION_ITERATIONS {
            return Err(de::Error::custom(format!(
                "enc_time exceeds maximum {MAX_ENCRYPTION_ITERATIONS}"
            )));
        }
        Ok(value)
    })
}

fn deserialize_bool<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Bool(value) => Ok(value),
        serde_json::Value::Number(value) => Ok(value.as_i64().unwrap_or_default() != 0),
        serde_json::Value::String(value) => match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" => Ok(true),
            "0" | "false" | "no" | "" => Ok(false),
            _ => Err(de::Error::custom("expected a boolean or 0/1 string")),
        },
        serde_json::Value::Null => Ok(false),
        value => Err(de::Error::custom(format!("expected boolean, got {value}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_room_response_and_map_status() {
        let response = parse_room_response(
            r#"{
                "error": 0,
                "data": {
                    "room_id": 123,
                    "room_thumb": "https://img.example/cover.jpg",
                    "room_name": "测试直播",
                    "room_status": "1",
                    "start_time": "2025-01-02 03:04:05",
                    "owner_name": "主播",
                    "avatar": "https://img.example/avatar.jpg",
                    "online": 42
                }
            }"#,
        )
        .unwrap();
        let room = response.data.unwrap();

        assert!(room_is_live(&room));
        assert_eq!(room.room_id, "123");
        assert_eq!(platform_live_id(&room, 123), "2025-01-02_03_04_05");
        assert!(!map_room_status("2", 42));
        assert!(map_room_status("", 42));
    }

    #[test]
    fn live_id_is_safe_as_a_windows_path_component() {
        let response = parse_room_response(
            r#"{"error":0,"data":{"room_id":"77","room_status":"1","start_time":"2025-01-02 03:04:05"}}"#,
        )
        .unwrap();
        let room = response.data.unwrap();
        let live_id = platform_live_id(&room, 77);

        assert!(!live_id.contains(':'));
        assert!(!live_id.contains(' '));
        assert_eq!(live_id, "2025-01-02_03_04_05");
    }

    #[test]
    fn parse_offline_room_and_use_room_id_fallback() {
        let response = parse_room_response(
            r#"{"error":0,"data":{"room_id":"77","room_status":"2","start_time":""}}"#,
        )
        .unwrap();
        let room = response.data.unwrap();

        assert!(!room_is_live(&room));
        assert_eq!(platform_live_id(&room, 77), "77");
    }

    #[test]
    fn parse_play_response_and_build_flv_url() {
        let response = parse_play_response(
            r#"{
                "error": 0,
                "msg": "OK",
                "data": {
                    "room_id": "123",
                    "rtmp_url": "https://cdn.example/live/",
                    "rtmp_live": "stream.flv?token=abc",
                    "player_1": "https://cdn.example/hevc.flv?token=abc"
                }
            }"#,
        )
        .unwrap();
        let data = response.data.unwrap();

        assert_eq!(
            flv_url(&data).as_deref(),
            Some("https://cdn.example/live/stream.flv?token=abc")
        );
    }

    #[test]
    fn hevc_player_url_is_not_used_without_the_primary_avc_stream() {
        let response = parse_play_response(
            r#"{
                "error": 0,
                "data": {
                    "rtmp_url": "",
                    "rtmp_live": "",
                    "player_1": "https://cdn.example/hevc.flv?token=abc"
                }
            }"#,
        )
        .unwrap();

        assert!(flv_url(&response.data.unwrap()).is_none());
    }

    #[test]
    fn empty_play_data_is_not_a_stream() {
        let response = parse_play_response(r#"{"error":-5,"msg":"closeRoom","data":""}"#).unwrap();
        assert!(response.data.is_none());
        assert!(is_offline_error(response.error, &response.msg));
    }

    #[test]
    fn encryption_iteration_count_is_bounded() {
        let body = r#"{"error":0,"data":{"rand_str":"r","enc_time":17,"key":"k","is_special":false,"enc_data":"e"}}"#;
        assert!(parse_encryption_response(body).is_err());
    }

    #[test]
    fn encryption_response_accepts_integer_boolean_fields() {
        let response = parse_encryption_response(
            r#"{
                "error": 0,
                "data": {
                    "rand_str": "rand",
                    "enc_time": "2",
                    "expire_at": 2000000000,
                    "key": "key",
                    "is_special": 1,
                    "enc_data": "encoded"
                }
            }"#,
        )
        .unwrap();
        let data = response.data.unwrap();

        assert_eq!(data.enc_time, 2);
        assert!(data.is_special);
        assert_eq!(data.enc_data, "encoded");
    }

    #[test]
    fn authentication_text_is_classified_without_marking_offline() {
        assert!(is_auth_failure_text("鉴权失败"));
        assert!(is_auth_failure_text("invalid token"));
        assert!(!is_auth_failure_text("closeRoom"));
    }
}
