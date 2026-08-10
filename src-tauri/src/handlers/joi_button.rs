use crate::database::account::AccountRow;
use crate::database::task::TaskRow;
use crate::ffmpeg::{estimate_submission_audio_size, extract_submission_audio};
use crate::progress::progress_reporter::{EventEmitter, ProgressReporter, ProgressReporterTrait};
use crate::state::State;
use crate::state_type;
use chrono::Utc;
use recorder::platforms::bilibili;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::fs;
use tokio::time::sleep;
use url::Url;

#[cfg(feature = "gui")]
use tauri::State as TauriState;

const PLATFORM: &str = "joi-button";
const CLIENT_LABEL: &str = "bili-shadowreplay";
const MAX_AUDIO_BYTES: u64 = 5 * 1024 * 1024;
const MAX_RATE_RETRIES: usize = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JoiButtonSubmitter {
    pub open_id: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct JoiButtonAuthState {
    pub state: String,
    pub challenge: Option<String>,
    pub poll_token: Option<String>,
    pub room_id: Option<u64>,
    pub expires_at: Option<String>,
    pub expires_in_seconds: Option<i64>,
    pub listening_since: Option<String>,
    pub can_assert_not_seen: Option<bool>,
    pub resend: Option<bool>,
    pub poll_after_ms: Option<i64>,
    pub detail: Option<String>,
    pub token: Option<String>,
    pub submitter: Option<JoiButtonSubmitter>,
    pub revoked_oldest: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JoiButtonSourceForm {
    pub kind: String,
    pub title: String,
    pub date: String,
    pub time: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JoiButtonSubmitForm {
    pub caption_locale: String,
    pub name: String,
    pub caption: String,
    pub group_id: Option<String>,
    pub new_group: Option<String>,
    pub note: String,
    pub source: JoiButtonSourceForm,
}

fn clean_endpoint(endpoint: &str) -> Result<(String, Url), String> {
    let trimmed = endpoint.trim().trim_end_matches('/');
    let url = Url::parse(trimmed).map_err(|_| "轴伊按钮地址不是有效的 URL".to_string())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("轴伊按钮地址必须使用 http 或 https".to_string());
    }
    if !url.username().is_empty() || url.password().is_some() || url.query().is_some() {
        return Err("轴伊按钮地址不能包含账号、密码或查询参数".to_string());
    }
    Ok((trimmed.to_string(), url))
}

fn api_url(endpoint: &str, path: &str) -> String {
    format!("{}{}", endpoint.trim_end_matches('/'), path)
}

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(format!("{CLIENT_LABEL}/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| format!("创建轴伊按钮连接失败: {e}"))
}

async fn decode_json_response<T: for<'de> Deserialize<'de>>(
    response: reqwest::Response,
) -> Result<T, String> {
    let status = response.status();
    let retry_after_header = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok());
    let body = response
        .text()
        .await
        .map_err(|e| format!("读取轴伊按钮响应失败: {e}"))?;
    let value: Value =
        serde_json::from_str(&body).map_err(|e| format!("轴伊按钮返回了无法识别的响应: {e}"))?;
    if !status.is_success() {
        let error_value = value.get("error");
        let code = error_value
            .and_then(|error| match error {
                Value::Object(fields) => fields.get("code"),
                _ => Some(error),
            })
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let message = error_value
            .and_then(|error| match error {
                Value::Object(fields) => fields.get("message").or_else(|| value.get("message")),
                Value::String(_) => value.get("message").or(Some(error)),
                _ => Some(error),
            })
            .or_else(|| value.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("轴伊按钮请求失败");
        let retry_after = retry_after_seconds(&value, retry_after_header)
            .map(|seconds| format!(";retryAfterSeconds={seconds}"))
            .unwrap_or_default();
        return Err(format!(
            "joi_error_code:{code};http_status={};message={message}{retry_after}",
            status.as_u16()
        ));
    }
    serde_json::from_value(value).map_err(|e| format!("解析轴伊按钮响应失败: {e}"))
}

fn retry_after_seconds(value: &Value, header_value: Option<u64>) -> Option<u64> {
    value
        .get("retryAfterSeconds")
        .or_else(|| {
            value
                .get("error")
                .and_then(|error| error.get("retryAfterSeconds"))
        })
        .and_then(Value::as_u64)
        .or(header_value)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn joi_button_challenge(
    _state: state_type!(),
    endpoint: String,
) -> Result<JoiButtonAuthState, String> {
    let (endpoint, _) = clean_endpoint(&endpoint)?;
    let client = http_client()?;
    let response = client
        .post(api_url(&endpoint, "/api/auth/challenge"))
        .json(&json!({ "client": CLIENT_LABEL }))
        .send()
        .await
        .map_err(|e| format!("连接轴伊按钮失败: {e}"))?;
    decode_json_response(response).await
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn joi_button_poll(
    _state: state_type!(),
    endpoint: String,
    poll_token: String,
) -> Result<JoiButtonAuthState, String> {
    let (endpoint, _) = clean_endpoint(&endpoint)?;
    if poll_token.trim().is_empty() {
        return Err("轴伊按钮验证句柄为空".to_string());
    }
    let client = http_client()?;
    let response = client
        .post(api_url(&endpoint, "/api/auth/poll"))
        .json(&json!({ "pollToken": poll_token }))
        .send()
        .await
        .map_err(|e| format!("连接轴伊按钮失败: {e}"))?;
    decode_json_response(response).await
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn add_joi_button_account(
    state: state_type!(),
    endpoint: String,
    access_token: String,
    token_expires_at: String,
    open_id: String,
    display_name: String,
) -> Result<(), String> {
    let (endpoint, url) = clean_endpoint(&endpoint)?;
    if access_token.trim().is_empty() || open_id.trim().is_empty() {
        return Err("轴伊按钮验证没有返回可保存的身份".to_string());
    }
    let host = url
        .host_str()
        .ok_or_else(|| "轴伊按钮地址缺少主机名".to_string())?;
    let host = match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    };
    let account = AccountRow {
        platform: PLATFORM.to_string(),
        uid: format!("{host}#{}", open_id.trim()),
        name: if display_name.trim().is_empty() {
            open_id.trim().to_string()
        } else {
            display_name.trim().to_string()
        },
        avatar: String::new(),
        csrf: String::new(),
        cookies: String::new(),
        created_at: Utc::now().to_rfc3339(),
        endpoint: Some(endpoint),
        access_token: Some(access_token),
        token_expires_at: Some(token_expires_at),
    };
    state.db.upsert_account(&account).await?;
    // Do not return the persisted row: it contains the bearer token and the
    // caller only needs the success result before refreshing the redacted list.
    Ok(())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn joi_button_get_contract(
    _state: state_type!(),
    endpoint: String,
) -> Result<Value, String> {
    let (endpoint, _) = clean_endpoint(&endpoint)?;
    let client = http_client()?;
    let response = client
        .get(api_url(&endpoint, "/api/submit/contract"))
        .send()
        .await
        .map_err(|e| format!("连接轴伊按钮失败: {e}"))?;
    decode_json_response(response).await
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn joi_button_send_danmaku(
    state: state_type!(),
    uid: String,
    room_id: String,
    message: String,
) -> Result<(), String> {
    let account = state.db.get_account("bilibili", &uid).await?;
    let client = reqwest::Client::new();
    bilibili::api::send_danmaku(&client, &account.to_account(), &room_id, &message)
        .await
        .map_err(|e| e.to_string())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct JoiMetadata {
    items: Vec<JoiMetadataItem>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct JoiMetadataItem {
    key: String,
    name: String,
    caption: JoiCaption,
    #[serde(skip_serializing_if = "Option::is_none")]
    group_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    new_group: Option<String>,
    note: Option<String>,
    source: JoiMetadataSource,
}

#[derive(Debug, Serialize)]
struct JoiCaption {
    locale: String,
    text: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct JoiMetadataSource {
    kind: String,
    title: String,
    date: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    seconds: Option<i64>,
    url: String,
}

fn parse_time_seconds(value: &str) -> Result<f64, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(0.0);
    }
    if let Ok(seconds) = trimmed.parse::<f64>() {
        if seconds.is_finite() && seconds >= 0.0 {
            return Ok(seconds);
        }
    }
    let mut parts = trimmed.split(':');
    let minutes = parts
        .next()
        .and_then(|part| part.parse::<u64>().ok())
        .ok_or_else(|| "来源时间必须是 mm:ss".to_string())?;
    let seconds = parts
        .next()
        .and_then(|part| part.parse::<f64>().ok())
        .ok_or_else(|| "来源时间必须是 mm:ss".to_string())?;
    if parts.next().is_some() || !seconds.is_finite() || !(0.0..60.0).contains(&seconds) {
        return Err("来源时间必须是 mm:ss".to_string());
    }
    Ok(minutes as f64 * 60.0 + seconds)
}

fn metadata_for(form: &JoiButtonSubmitForm) -> Result<JoiMetadata, String> {
    let source_seconds = if form.source.time.trim().is_empty() {
        None
    } else {
        let seconds = parse_time_seconds(&form.source.time)?;
        if seconds.fract() != 0.0 {
            return Err("来源时间必须是整秒 mm:ss".to_string());
        }
        Some(seconds as i64)
    };
    let source_kind = if form.source.kind.trim().is_empty() {
        "stream"
    } else {
        form.source.kind.trim()
    };
    Ok(JoiMetadata {
        items: vec![JoiMetadataItem {
            key: "clip-1".to_string(),
            name: form.name.trim().to_string(),
            caption: JoiCaption {
                locale: form.caption_locale.trim().to_string(),
                text: form.caption.clone(),
            },
            group_id: form
                .group_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
            new_group: form
                .new_group
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
            note: (!form.note.trim().is_empty()).then(|| form.note.clone()),
            source: JoiMetadataSource {
                kind: source_kind.to_string(),
                title: form.source.title.clone(),
                date: form.source.date.clone(),
                seconds: source_seconds,
                url: form.source.url.clone(),
            },
        }],
    })
}

fn error_code(error: &str) -> &str {
    error
        .strip_prefix("joi_error_code:")
        .and_then(|value| value.split(';').next())
        .unwrap_or("")
}

async fn submit_once(
    client: &reqwest::Client,
    endpoint: &str,
    account: &AccountRow,
    metadata: &str,
    file_name: &str,
    bytes: &[u8],
) -> Result<Value, String> {
    let part = reqwest::multipart::Part::bytes(bytes.to_vec())
        .file_name(file_name.to_string())
        .mime_str(if file_name.ends_with(".m4a") {
            "audio/mp4"
        } else {
            "audio/mpeg"
        })
        .map_err(|e| format!("构造音频上传失败: {e}"))?;
    let form = reqwest::multipart::Form::new()
        .text("metadata", metadata.to_string())
        .part("file:clip-1", part);
    let response = client
        .post(api_url(endpoint, "/api/submit"))
        .bearer_auth(account.access_token.as_deref().unwrap_or_default())
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("提交到轴伊按钮失败: {e}"))?;
    decode_json_response(response).await
}

async fn submit_with_retry(
    client: &reqwest::Client,
    endpoint: &str,
    account: &AccountRow,
    metadata: &str,
    file_name: &str,
    bytes: &[u8],
    reporter: &ProgressReporter,
) -> Result<Value, String> {
    for attempt in 0..=MAX_RATE_RETRIES {
        match submit_once(client, endpoint, account, metadata, file_name, bytes).await {
            Ok(value) => return Ok(value),
            Err(error) if error_code(&error) == "rate_limited" => {
                if attempt < MAX_RATE_RETRIES {
                    let retry_after = error
                        .split("retryAfterSeconds=")
                        .nth(1)
                        .and_then(|value| value.split(';').next())
                        .and_then(|value| value.parse::<u64>().ok())
                        .unwrap_or(1)
                        .min(60);
                    reporter
                        .update(&format!(
                            "投稿太频繁。每分钟只能投一批，{retry_after} 秒后自动继续。"
                        ))
                        .await;
                    sleep(Duration::from_secs(retry_after)).await;
                } else {
                    return Err(
                        "joi_error_code:rate_limited_exhausted;message=自动重试次数已达上限，请稍后重新投稿"
                            .to_string(),
                    );
                }
            }
            Err(error) => return Err(error),
        }
    }
    Err("joi_error_code:rate_limited;message=投稿重试次数已达上限".to_string())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn joi_button_submit(
    state: state_type!(),
    event_id: String,
    uid: String,
    video_id: i64,
    form: JoiButtonSubmitForm,
) -> Result<Value, String> {
    let account = state.db.get_account(PLATFORM, &uid).await?;
    let endpoint = account
        .endpoint
        .clone()
        .ok_or_else(|| "轴伊按钮账号缺少站点地址".to_string())?;
    if account
        .access_token
        .as_deref()
        .filter(|token| !token.is_empty())
        .is_none()
    {
        return Err("joi_error_code:expired_api_token;message=轴伊按钮令牌为空".to_string());
    }
    let metadata = metadata_for(&form)?;
    if metadata.items[0].name.is_empty() {
        return Err("joi_error_code:invalid_name;message=切片名称不能为空".to_string());
    }

    let video = state.db.get_video(video_id).await?;
    let estimate = estimate_submission_audio_size(video.length as f64);
    if estimate > MAX_AUDIO_BYTES {
        return Err(format!(
            "joi_error_code:audio_too_large;durationSeconds={};suggestedMaxSeconds=210;message=音频预估超过 5 MB",
            video.length
        ));
    }
    let config = state.config.read().await.clone();
    let input_path = Path::new(&config.output).join(&video.file);
    if !input_path.exists() {
        return Err("joi_error_code:unreadable_audio;message=切片文件不存在".to_string());
    }

    let task = TaskRow {
        id: event_id.clone(),
        task_type: "joi_button_submit".to_string(),
        status: "pending".to_string(),
        message: "准备投稿".to_string(),
        metadata: serde_json::to_string(&form).map_err(|e| e.to_string())?,
        created_at: Utc::now().to_rfc3339(),
    };
    state.db.add_task(&task).await?;
    #[cfg(feature = "gui")]
    let emitter = EventEmitter::new(state.app_handle.clone());
    #[cfg(feature = "headless")]
    let emitter = EventEmitter::new(state.progress_manager.get_event_sender());
    let reporter = ProgressReporter::new(state.db.clone(), &emitter, &event_id).await?;

    reporter.update("准备投稿音频").await;
    let temp_dir = PathBuf::from(&config.cache).join("joi-button-submissions");
    if let Err(error) = fs::create_dir_all(&temp_dir).await {
        let message = format!("创建投稿临时目录失败: {error}");
        let _ = state
            .db
            .update_task(&event_id, "failed", &message, None)
            .await;
        reporter.finish(false, &message).await;
        return Err(message);
    }
    let temp_mp3 = temp_dir.join(format!("{}.mp3", uuid::Uuid::new_v4()));
    let encoded = match extract_submission_audio(&input_path, &temp_mp3).await {
        Ok(path) => path,
        Err(error) => {
            let _ = fs::remove_file(&temp_mp3).await;
            let _ = state
                .db
                .update_task(&event_id, "failed", &error, None)
                .await;
            reporter.finish(false, &error).await;
            return Err(format!(
                "joi_error_code:audio_processing_failed;message={error}"
            ));
        }
    };
    let bytes = match fs::read(&encoded).await {
        Ok(bytes) => bytes,
        Err(error) => {
            let _ = fs::remove_file(&encoded).await;
            let message = format!("joi_error_code:unreadable_audio;message={error}");
            let _ = state
                .db
                .update_task(&event_id, "failed", &message, None)
                .await;
            reporter.finish(false, &message).await;
            return Err(message);
        }
    };
    if bytes.len() as u64 > MAX_AUDIO_BYTES {
        let _ = fs::remove_file(&encoded).await;
        let message = format!(
            "joi_error_code:audio_too_large;durationSeconds={};suggestedMaxSeconds=210;message=音频编码后超过 5 MB",
            video.length
        );
        let _ = state
            .db
            .update_task(&event_id, "failed", &message, None)
            .await;
        reporter.finish(false, &message).await;
        return Err(message);
    }

    reporter.update("正在提交音频").await;
    let client = match http_client() {
        Ok(client) => client,
        Err(error) => {
            let _ = fs::remove_file(&encoded).await;
            let _ = state
                .db
                .update_task(&event_id, "failed", &error, None)
                .await;
            reporter.finish(false, &error).await;
            return Err(error);
        }
    };
    let metadata_json = match serde_json::to_string(&metadata) {
        Ok(metadata) => metadata,
        Err(error) => {
            let _ = fs::remove_file(&encoded).await;
            let message = error.to_string();
            let _ = state
                .db
                .update_task(&event_id, "failed", &message, None)
                .await;
            reporter.finish(false, &message).await;
            return Err(message);
        }
    };
    let file_name = encoded
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("clip.mp3");
    let result = submit_with_retry(
        &client,
        &endpoint,
        &account,
        &metadata_json,
        file_name,
        &bytes,
        &reporter,
    )
    .await;
    let _ = fs::remove_file(&encoded).await;
    match result {
        Ok(value) => {
            state
                .db
                .update_task(&event_id, "success", "投稿完成", None)
                .await?;
            reporter.finish(true, "投稿完成").await;
            Ok(value)
        }
        Err(error) => {
            state
                .db
                .update_task(&event_id, "failed", &error, None)
                .await?;
            reporter.finish(false, &error).await;
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        estimate_submission_audio_size, metadata_for, parse_time_seconds, retry_after_seconds,
        JoiButtonSourceForm, JoiButtonSubmitForm,
    };
    use serde_json::json;

    #[test]
    fn submission_estimate_keeps_the_whisper_path_separate() {
        assert!(estimate_submission_audio_size(90.0) < 5 * 1024 * 1024);
        assert!(estimate_submission_audio_size(300.0) > 5 * 1024 * 1024);
    }

    #[test]
    fn source_time_is_serialized_as_seconds() {
        assert_eq!(parse_time_seconds("04:20").unwrap(), 260.0);
        assert!(parse_time_seconds("04:60").is_err());
        let form = JoiButtonSubmitForm {
            caption_locale: "zh-CN".to_string(),
            name: "clip".to_string(),
            caption: "caption".to_string(),
            group_id: Some("group".to_string()),
            new_group: None,
            note: String::new(),
            source: JoiButtonSourceForm {
                kind: "stream".to_string(),
                title: "source".to_string(),
                date: "2026-08-10".to_string(),
                time: "04:20".to_string(),
                url: String::new(),
            },
        };
        let value = serde_json::to_value(metadata_for(&form).unwrap()).unwrap();
        assert_eq!(value["items"][0]["source"]["seconds"], 260.0);
    }

    #[test]
    fn missing_source_time_is_not_serialized_as_zero() {
        let form = JoiButtonSubmitForm {
            caption_locale: "zh-CN".to_string(),
            name: "clip".to_string(),
            caption: "caption".to_string(),
            group_id: Some("group".to_string()),
            new_group: None,
            note: String::new(),
            source: JoiButtonSourceForm {
                kind: "stream".to_string(),
                title: String::new(),
                date: String::new(),
                time: String::new(),
                url: String::new(),
            },
        };
        let value = serde_json::to_value(metadata_for(&form).unwrap()).unwrap();
        assert!(value["items"][0]["source"].get("seconds").is_none());
    }

    #[test]
    fn retry_after_uses_the_header_when_the_body_omits_it() {
        assert_eq!(
            retry_after_seconds(&json!({ "error": "rate_limited" }), Some(42)),
            Some(42)
        );
        assert_eq!(
            retry_after_seconds(
                &json!({ "error": "rate_limited", "retryAfterSeconds": 7 }),
                Some(42)
            ),
            Some(7)
        );
    }
}
