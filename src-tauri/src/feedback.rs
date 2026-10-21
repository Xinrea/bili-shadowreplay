//! Anonymous feedback to PowerLive, with a redacted snapshot of the log attached.
//!
//! A submission is frozen into a draft the first time it is sent: its id, log
//! snapshot and environment stay fixed, so retrying the same text after a
//! failure resends the same feedback instead of filing a new one.
use crate::state::State;
use crate::state_type;
use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;
use tokio::sync::Mutex;

#[cfg(feature = "gui")]
use {tauri::Manager, tauri::State as TauriState};

const APP_ID: &str = "bsr";
const MAX_FILE_BYTES: usize = 5 * 1024 * 1024;
/// Leaves room under the attachment cap for redaction to grow the text.
const LOG_BYTES: usize = 4 * 1024 * 1024;
const MAX_MESSAGE_CHARS: usize = 5000;
/// The API needs some text; the description is optional in the app.
const EMPTY_MESSAGE: &str = "（用户未填写说明，请查看附带的日志）";
const PREPARE_TIMEOUT: Duration = Duration::from_secs(25);
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(60);

/// The draft being sent, kept after a failure so a retry reuses it.
static PENDING: Mutex<Option<Draft>> = Mutex::const_new(None);

#[derive(Serialize)]
pub struct FeedbackReceipt {
    id: String,
}

struct Screenshot {
    bytes: Vec<u8>,
    file_name: &'static str,
    mime: &'static str,
}

struct Draft {
    fingerprint: [u8; 32],
    submission_id: String,
    app_version: String,
    os: String,
    message: String,
    email: Option<String>,
    screenshot: Option<Screenshot>,
    log: Option<Vec<u8>>,
    token: Option<(String, DateTime<Utc>)>,
}

#[derive(Serialize)]
struct PrepareRequest<'a> {
    app_id: &'a str,
    submission_id: &'a str,
    app_version: &'a str,
    os: &'a str,
    screenshot_size: usize,
    log_size: usize,
}

#[derive(Deserialize)]
struct PrepareResponse {
    #[serde(default)]
    upload_token: String,
    expires_at: Option<DateTime<Utc>>,
    max_file_bytes: Option<usize>,
    #[serde(default)]
    id: String,
    #[serde(default)]
    already_submitted: bool,
}

#[derive(Deserialize)]
struct Receipt {
    id: String,
}

#[derive(Deserialize)]
struct ErrorBody {
    error: String,
    #[serde(default)]
    message: String,
}

enum Prepared {
    Done(String),
    Token(String, DateTime<Utc>),
}

#[derive(Debug)]
struct Failure {
    /// `0` when no response was read, so the outcome is unknown.
    status: u16,
    code: String,
    message: String,
    retry_after: Option<u64>,
}

impl Failure {
    fn local(code: &str) -> Self {
        Self {
            status: 0,
            code: code.into(),
            message: String::new(),
            retry_after: None,
        }
    }

    /// Whether the draft can never succeed as it is, so the next attempt should start over.
    fn discards_draft(&self) -> bool {
        matches!(
            self.code.as_str(),
            "validation_error" | "request_too_large" | "submission_conflict"
        )
    }

    fn user_message(&self) -> String {
        let wait = self.retry_after.map(format_wait).unwrap_or_default();
        match self.code.as_str() {
            "validation_error" if !self.message.is_empty() => {
                format!("反馈未通过校验：{}", self.message)
            }
            "validation_error" => "反馈未通过校验，请检查内容和截图后重试".into(),
            "feedback_app_disabled" => "反馈服务暂未对 BSR 开放，请前往 GitHub Issues 反馈".into(),
            "invalid_upload_token" => "提交凭证已失效，请稍后重试".into(),
            "challenge_required" | "challenge_failed" | "rate_limit_exceeded" => {
                format!("提交过于频繁，请{}再试", or_later(&wait))
            }
            "submission_conflict" => {
                "网络环境已变化，原反馈无法继续提交；再次提交将作为新反馈发送".into()
            }
            "submission_in_progress" => format!("反馈仍在上传中，请{}再确认", or_later(&wait)),
            "request_too_large" => "反馈内容或附件过大，请缩小截图后重试".into(),
            "upload_busy" => format!("反馈服务繁忙，请{}重试", or_later(&wait)),
            "upload_quota_exceeded" => "今日反馈附件额度已用完，请明天再试".into(),
            "feedback_unavailable" => "反馈服务暂时不可用，请稍后重试".into(),
            "network_error" => "无法连接反馈服务，请检查网络后重试".into(),
            "invalid_response" => "反馈服务返回了无效响应，请稍后重试".into(),
            _ => format!("反馈提交失败（HTTP {}），请稍后重试", self.status),
        }
    }
}

fn format_wait(seconds: u64) -> String {
    match seconds {
        0..=59 => format!("{seconds} 秒后"),
        60..=3599 => format!("{} 分钟后", seconds.div_ceil(60)),
        _ => format!("{} 小时后", seconds.div_ceil(3600)),
    }
}

fn or_later(wait: &str) -> &str {
    if wait.is_empty() {
        "稍后"
    } else {
        wait
    }
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn submit_feedback(
    state: state_type!(),
    message: String,
    email: Option<String>,
    screenshot: Option<String>,
) -> Result<FeedbackReceipt, String> {
    let Ok(mut pending) = PENDING.try_lock() else {
        return Err("反馈正在提交中，请稍候".into());
    };
    let message = normalize_message(&message)?;
    let email = normalize_email(email.as_deref())?;
    let screenshot = screenshot.as_deref().map(decode_screenshot).transpose()?;
    let fingerprint = fingerprint(&message, email.as_deref(), screenshot.as_ref());
    if pending.as_ref().map(|draft| draft.fingerprint) != Some(fingerprint) {
        *pending = Some(Draft {
            fingerprint,
            submission_id: uuid::Uuid::new_v4().to_string(),
            app_version: env!("CARGO_PKG_VERSION").into(),
            os: os_description(),
            message,
            email,
            screenshot,
            log: read_logs(&log_dir(&state)?).await,
            token: None,
        });
    }
    let draft = pending.as_mut().expect("draft was just set");
    match send(draft, crate::powerlive::API_BASE).await {
        Ok(id) => {
            log::info!("Feedback submitted: {id}");
            *pending = None;
            Ok(FeedbackReceipt { id })
        }
        Err(failure) => {
            log::warn!(
                "Feedback submission failed: status {}, {}",
                failure.status,
                failure.code
            );
            if failure.discards_draft() {
                *pending = None;
            }
            Err(failure.user_message())
        }
    }
}

#[cfg(feature = "gui")]
fn log_dir(state: &State) -> Result<PathBuf, String> {
    state
        .app_handle
        .path()
        .app_log_dir()
        .map_err(|e| format!("无法找到日志目录：{e}"))
}

#[cfg(not(feature = "gui"))]
fn log_dir(_state: &State) -> Result<PathBuf, String> {
    // Headless mode logs to the working directory; see `start_server` in main.rs.
    Ok(PathBuf::from("./"))
}

/// Uses a valid token from an earlier attempt first; prepares again once if it was spent.
async fn send(draft: &mut Draft, base: &str) -> Result<String, Failure> {
    let client = crate::powerlive::client().map_err(|_| Failure::local("network_error"))?;
    for attempt in 0..2 {
        let token = match draft.token.clone() {
            Some((token, expires_at))
                if expires_at > Utc::now() + chrono::Duration::seconds(30) =>
            {
                token
            }
            _ => match prepare(&client, base, draft).await? {
                Prepared::Done(id) => return Ok(id),
                Prepared::Token(token, expires_at) => {
                    draft.token = Some((token.clone(), expires_at));
                    token
                }
            },
        };
        match upload(&client, base, draft, &token).await {
            Err(failure) if failure.code == "invalid_upload_token" && attempt == 0 => {
                draft.token = None;
            }
            result => return result,
        }
    }
    unreachable!("the second attempt always returns")
}

async fn prepare(client: &reqwest::Client, base: &str, draft: &Draft) -> Result<Prepared, Failure> {
    let body = PrepareRequest {
        app_id: APP_ID,
        submission_id: &draft.submission_id,
        app_version: &draft.app_version,
        os: &draft.os,
        screenshot_size: draft.screenshot.as_ref().map_or(0, |s| s.bytes.len()),
        log_size: draft.log.as_ref().map_or(0, Vec::len),
    };
    let response = client
        .post(format!("{base}/feedback/prepare"))
        .timeout(PREPARE_TIMEOUT)
        .json(&body)
        .send()
        .await
        .map_err(|_| Failure::local("network_error"))?;
    let prepared: PrepareResponse = parse(response).await?;
    if prepared.already_submitted && !prepared.id.is_empty() {
        return Ok(Prepared::Done(prepared.id));
    }
    let Some(expires_at) = prepared
        .expires_at
        .filter(|_| !prepared.upload_token.is_empty())
    else {
        return Err(Failure::local("invalid_response"));
    };
    if prepared
        .max_file_bytes
        .is_some_and(|max| body.screenshot_size > max || body.log_size > max)
    {
        return Err(Failure::local("request_too_large"));
    }
    Ok(Prepared::Token(prepared.upload_token, expires_at))
}

async fn upload(
    client: &reqwest::Client,
    base: &str,
    draft: &Draft,
    token: &str,
) -> Result<String, Failure> {
    use reqwest::multipart::{Form, Part};
    let mut form = Form::new().text("message", draft.message.clone());
    if let Some(email) = &draft.email {
        form = form.text("email", email.clone());
    }
    if let Some(screenshot) = &draft.screenshot {
        let part = Part::bytes(screenshot.bytes.clone())
            .file_name(screenshot.file_name)
            .mime_str(screenshot.mime)
            .expect("static mime type is valid");
        form = form.part("screenshot", part);
    }
    if let Some(log) = &draft.log {
        let part = Part::bytes(log.clone())
            .file_name("bsr.log")
            .mime_str("text/plain")
            .expect("static mime type is valid");
        form = form.part("log", part);
    }
    let response = client
        .post(format!("{base}/feedback"))
        .timeout(UPLOAD_TIMEOUT)
        .header("X-Feedback-Token", token)
        .multipart(form)
        .send()
        .await
        .map_err(|_| Failure::local("network_error"))?;
    let receipt: Receipt = parse(response).await?;
    if receipt.id.is_empty() {
        return Err(Failure::local("invalid_response"));
    }
    Ok(receipt.id)
}

async fn parse<T: for<'de> Deserialize<'de>>(response: reqwest::Response) -> Result<T, Failure> {
    let status = response.status();
    let retry_after = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok()?.trim().parse().ok());
    // A body that was cut off may hide a success, so treat it as unknown.
    let body = response
        .bytes()
        .await
        .map_err(|_| Failure::local("network_error"))?;
    if status.is_success() {
        return serde_json::from_slice(&body).map_err(|_| Failure::local("invalid_response"));
    }
    let error = serde_json::from_slice::<ErrorBody>(&body).ok();
    Err(Failure {
        status: status.as_u16(),
        code: error
            .as_ref()
            .map_or_else(|| format!("http_{}", status.as_u16()), |e| e.error.clone()),
        message: error.map(|e| e.message).unwrap_or_default(),
        retry_after,
    })
}

fn normalize_message(message: &str) -> Result<String, String> {
    let message: String = message
        .replace("\r\n", "\n")
        .chars()
        .map(|c| if c == '\r' { '\n' } else { c })
        .filter(|&c| !c.is_control() || c == '\n' || c == '\t')
        .collect();
    let message = message.trim();
    match message.chars().count() {
        0 => Ok(EMPTY_MESSAGE.into()),
        n if n > MAX_MESSAGE_CHARS => Err(format!("反馈说明不能超过 {MAX_MESSAGE_CHARS} 字")),
        _ => Ok(message.into()),
    }
}

fn normalize_email(email: Option<&str>) -> Result<Option<String>, String> {
    let email = email.unwrap_or_default().trim();
    if email.is_empty() {
        return Ok(None);
    }
    let valid = email.len() <= 254
        && !email.contains(|c: char| c.is_whitespace() || c.is_control() || "<>\",;".contains(c))
        && email.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty()
                && !domain.contains('@')
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
        });
    if valid {
        Ok(Some(email.into()))
    } else {
        Err("邮箱格式不正确".into())
    }
}

fn decode_screenshot(data_url: &str) -> Result<Screenshot, String> {
    let bytes = crate::handlers::utils::decode_data_url(data_url)?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err("截图不能超过 5 MB".into());
    }
    let (file_name, mime) = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        ("screenshot.png", "image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        ("screenshot.jpg", "image/jpeg")
    } else {
        return Err("截图只支持 PNG 或 JPEG 格式".into());
    };
    Ok(Screenshot {
        bytes,
        file_name,
        mime,
    })
}

fn fingerprint(message: &str, email: Option<&str>, screenshot: Option<&Screenshot>) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for part in [
        message.as_bytes(),
        email.unwrap_or_default().as_bytes(),
        screenshot.map_or(&[][..], |s| &s.bytes),
    ] {
        hasher.update((part.len() as u64).to_le_bytes());
        hasher.update(part);
    }
    hasher.finalize().into()
}

fn os_description() -> String {
    let os = sysinfo::System::long_os_version().unwrap_or_else(|| std::env::consts::OS.into());
    // sysinfo leaves empty parts as extra spaces, e.g. "MacOS 27.0.1 ".
    let os = os.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate(format!("{os} {}", std::env::consts::ARCH), 200)
}

fn truncate(mut text: String, max_bytes: usize) -> String {
    if text.len() > max_bytes {
        let end = (0..=max_bytes)
            .rev()
            .find(|&i| text.is_char_boundary(i))
            .unwrap_or(0);
        text.truncate(end);
    }
    text
}

/// The tail of the current log, preceded by the previous run's log when there is room.
async fn read_logs(dir: &Path) -> Option<Vec<u8>> {
    let mut text = tail(&dir.join("bsr.log"), LOG_BYTES)
        .await
        .unwrap_or_default();
    let room = LOG_BYTES.saturating_sub(text.len());
    if room > 64 * 1024 {
        if let Some(previous) = previous_log(dir).await {
            if let Some(earlier) = tail(&previous, room).await {
                let name = previous.file_name().unwrap_or_default().to_string_lossy();
                text = format!("===== {name} =====\n{earlier}\n===== bsr.log =====\n{text}");
            }
        }
    }
    let mut text = redact(&text);
    if text.len() > MAX_FILE_BYTES {
        let start = text.len() - MAX_FILE_BYTES;
        let start = (start..text.len())
            .find(|&i| text.is_char_boundary(i))
            .unwrap_or(text.len());
        text.drain(..start);
    }
    (!text.trim().is_empty()).then(|| text.into_bytes())
}

/// The last `max_bytes` of a log, starting on a line boundary.
async fn tail(path: &Path, max_bytes: usize) -> Option<String> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};
    // A long session can grow the log far past `max_bytes`; read only its end.
    let read = async {
        let mut file = tokio::fs::File::open(path).await?;
        let len = file.metadata().await?.len();
        let start = len.saturating_sub(max_bytes as u64);
        file.seek(std::io::SeekFrom::Start(start)).await?;
        let mut bytes = Vec::new();
        // The log may still grow while it is read; stop at the size seen above.
        file.take(len - start).read_to_end(&mut bytes).await?;
        Ok::<_, std::io::Error>((start, bytes))
    };
    let (start, bytes) = read
        .await
        .map_err(|e| log::warn!("Failed to read {} for feedback: {e}", path.display()))
        .ok()?;
    let skip = if start > 0 {
        bytes
            .iter()
            .position(|&b| b == b'\n')
            .map_or(bytes.len(), |p| p + 1)
    } else {
        0
    };
    Some(String::from_utf8_lossy(&bytes[skip..]).into_owned())
}

/// The most recent rotated log; their timestamped names sort by age.
async fn previous_log(dir: &Path) -> Option<PathBuf> {
    let mut entries = tokio::fs::read_dir(dir).await.ok()?;
    let mut latest: Option<String> = None;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("bsr-")
            && name.ends_with(".log")
            && latest.as_ref().is_none_or(|latest| name > *latest)
        {
            latest = Some(name);
        }
    }
    latest.map(|name| dir.join(name))
}

/// Masks credentials that may end up in log lines, and drops NUL bytes.
///
/// Any key ending in `secret`, `token`, `sign` or `signature` is masked, which
/// covers CDN signatures such as `volcSecret`. Besides account secrets
/// (including `csrf`, which repeats `bili_jct` in upload URLs) this covers
/// upload `auth`, the stream keys `sk`, `flvsk`, Twitch's `sig` and Huya's `fm`, the viewer id and address
/// in stream URLs (`mid`, `oi`), and webhook URLs, whose path is itself the
/// credential. Keys also match after a JSON-escaped `&` (`\u0026sk=`).
fn redact(text: &str) -> String {
    static SECRET: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r#"(?i)\b([a-z0-9_]*(?:secret|token|signature|sign)|(?:u0026)?(?:sessdata|bili_jct|csrf|sessionid(?:_ss)?|sid_tt|sid_guard|uid_tt|ttwid|dedeuserid(?:__ckmd5)?|buvid3|access_key|api_key|apikey|password|passwd|auth|auth_key|upsig|sig|sk|flvsk|fm|mid|oi))\b(["']?\s*[:=]\s*)("[^"\n]*"|'[^'\n]*'|["']?(?:bearer\s+)?[^\s"'&;,})\]>\\]+)"#,
        )
        .expect("valid secret pattern")
    });
    static HEADER: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?i)\b(cookies?|authorization|x-feedback-token)\b(["']?\s*[:=]\s*)[^\n]*"#)
            .expect("valid header pattern")
    });
    static WEBHOOK: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?i)(webhook[^\n]*?https?://[^/\s?#"'<>]+)[^\s"'<>]*"#)
            .expect("valid webhook pattern")
    });
    let text = text.replace('\0', "");
    let text = HEADER.replace_all(&text, "$1$2[REDACTED]");
    let text = WEBHOOK.replace_all(&text, "$1/[REDACTED]");
    // A quoted value is masked up to its closing quote, since it may hold `&` or spaces.
    SECRET
        .replace_all(&text, |caps: &regex::Captures| {
            let value = &caps[3];
            let quote = value.chars().next().filter(|c| *c == '"' || *c == '\'');
            let close = quote.filter(|q| value.len() > 1 && value.ends_with(*q));
            format!(
                "{}{}{}[REDACTED]{}",
                &caps[1],
                &caps[2],
                quote.map(String::from).unwrap_or_default(),
                close.map(String::from).unwrap_or_default()
            )
        })
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        extract::State,
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::post,
        Json, Router,
    };
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    fn draft() -> Draft {
        Draft {
            fingerprint: [0; 32],
            submission_id: "714aa8ab-2f59-4765-8f7d-ae70577eacdf".into(),
            app_version: "1.2.3".into(),
            os: "macOS 15 arm64".into(),
            message: "录制中断".into(),
            email: None,
            screenshot: None,
            log: Some(b"line\n".to_vec()),
            token: None,
        }
    }

    async fn serve(router: Router) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        (base, server)
    }

    fn token_response(token: &str) -> Json<serde_json::Value> {
        Json(serde_json::json!({
            "upload_token": token,
            "expires_at": (Utc::now() + chrono::Duration::minutes(5)).to_rfc3339(),
            "max_file_bytes": MAX_FILE_BYTES,
            "max_request_bytes": 11534336,
        }))
    }

    #[test]
    fn messages_and_emails_are_normalized() {
        assert_eq!(normalize_message("  a\r\nb\0\tc \r").unwrap(), "a\nb\tc");
        assert_eq!(normalize_message(" \n\t ").unwrap(), EMPTY_MESSAGE);
        assert!(normalize_message(&"字".repeat(MAX_MESSAGE_CHARS)).is_ok());
        assert!(normalize_message(&"字".repeat(MAX_MESSAGE_CHARS + 1)).is_err());
        assert_eq!(normalize_email(Some("  ")).unwrap(), None);
        assert_eq!(
            normalize_email(Some(" user@example.com "))
                .unwrap()
                .as_deref(),
            Some("user@example.com")
        );
        for invalid in [
            "Name <user@example.com>",
            "user@",
            "@example.com",
            "a@b@c.com",
            "a@localhost",
        ] {
            assert!(normalize_email(Some(invalid)).is_err(), "{invalid}");
        }
    }

    #[test]
    fn screenshots_must_be_png_or_jpeg() {
        use base64::Engine;
        let url = |bytes: &[u8]| {
            format!(
                "data:image/png;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(bytes)
            )
        };
        assert_eq!(
            decode_screenshot(&url(b"\x89PNG\r\n\x1a\nrest"))
                .unwrap()
                .mime,
            "image/png"
        );
        assert_eq!(
            decode_screenshot(&url(&[0xFF, 0xD8, 0xFF, 0xE0]))
                .unwrap()
                .file_name,
            "screenshot.jpg"
        );
        assert!(decode_screenshot(&url(b"GIF89a")).is_err());
    }

    #[test]
    fn credentials_are_redacted() {
        let text = redact(
            "url?access_key=abc123&room=1\nCookie: SESSDATA=x; bili_jct=y\n{\"refresh_token\":\"r\",\"token_type\":\"Bearer\"}\nauthorization: Bearer zzz\0\n",
        );
        assert_eq!(
            text,
            "url?access_key=[REDACTED]&room=1\nCookie: [REDACTED]\n{\"refresh_token\":\"[REDACTED]\",\"token_type\":\"Bearer\"}\nauthorization: [REDACTED]\n"
        );
    }

    #[test]
    fn credentials_in_info_log_lines_are_redacted() {
        // The shapes BSR writes at Info level: webhook setup, upload preupload and stream URLs.
        let cases = [
            (
                "12:00:00 [INFO] Creating webhook poster with URL: https://discord.com/api/webhooks/123/abcDEF",
                "12:00:00 [INFO] Creating webhook poster with URL: https://discord.com/[REDACTED]",
            ),
            (
                "12:00:00 [INFO] Updating webhook url to https://hooks.slack.com/services/T0/B0/xyz?x=1",
                "12:00:00 [INFO] Updating webhook url to https://hooks.slack.com/[REDACTED]",
            ),
            (
                "12:00:00 [INFO] Creating webhook poster with URL: ",
                "12:00:00 [INFO] Creating webhook poster with URL: ",
            ),
            (
                r#"12:00:00 [INFO] Preupload Response: PreuploadResponse { endpoint: "//upos-cs-upcdnbda2.bilivideo.com", upos_uri: "upos://ugcfx2lf/n1.mp4", auth: "ak=1&cdn=%2F%2Fupos&os=upos&sign=f00d&timestamp=1", chunk_size: 10485760, biz_id: 1 }"#,
                r#"12:00:00 [INFO] Preupload Response: PreuploadResponse { endpoint: "//upos-cs-upcdnbda2.bilivideo.com", upos_uri: "upos://ugcfx2lf/n1.mp4", auth: "[REDACTED]", chunk_size: 10485760, biz_id: 1 }"#,
            ),
            (
                "12:00:00 [INFO] [7890]   origin: https://pull-flv.douyincdn.com/stage/s.flv?expire=1761091200&sign=0a1b2c&major_anchor_level=common",
                "12:00:00 [INFO] [7890]   origin: https://pull-flv.douyincdn.com/stage/s.flv?expire=1761091200&sign=[REDACTED]&major_anchor_level=common",
            ),
            (
                "https://cn.bilivideo.com/live/1.m4s?expires=1&len=0&oi=3070230&pt=web&mid=4242&sk=deadbeef&qn=10000",
                "https://cn.bilivideo.com/live/1.m4s?expires=1&len=0&oi=[REDACTED]&pt=web&mid=[REDACTED]&sk=[REDACTED]&qn=10000",
            ),
            (
                "12:00:00 [ERROR] Send failed error sending request for url (https://member.bilibili.com/x/vu/web/add/v3?ts=1761091200&csrf=0123abcd)",
                "12:00:00 [ERROR] Send failed error sending request for url (https://member.bilibili.com/x/vu/web/add/v3?ts=1761091200&csrf=[REDACTED])",
            ),
            (
                "douyin session sessionid=s1; sessionid_ss=s2; sid_tt=s3; passport_csrf_token=p; ttwid=t; msToken=m",
                "douyin session sessionid=[REDACTED]; sessionid_ss=[REDACTED]; sid_tt=[REDACTED]; passport_csrf_token=[REDACTED]; ttwid=[REDACTED]; msToken=[REDACTED]",
            ),
            (
                "12:00:00 [INFO] [7890]   origin: https://pull-flv-l11.douyincdn.com/stage/s.flv?arch_hrchy=c1&volcSecret=DEADBEEF&volcTime=1761091200&sign=0a1b",
                "12:00:00 [INFO] [7890]   origin: https://pull-flv-l11.douyincdn.com/stage/s.flv?arch_hrchy=c1&volcSecret=[REDACTED]&volcTime=1761091200&sign=[REDACTED]",
            ),
            (
                r#"        extra: "?expires=1761091200&len=0&oi=1&pt=web&qn=10000&trid=t&sigparams=cdn,expires,len,oi&cdn=cn-gotcha01&sign=f00&flvsk=abc123&sk=def456&p2p_type=0","#,
                r#"        extra: "?expires=1761091200&len=0&oi=[REDACTED]&pt=web&qn=10000&trid=t&sigparams=cdn,expires,len,oi&cdn=cn-gotcha01&sign=[REDACTED]&flvsk=[REDACTED]&sk=[REDACTED]&p2p_type=0","#,
            ),
            (
                r#""flv":"https://pull.douyincdn.com/s.flv?expire=1\u0026sign=a\u0026mid=7\u0026volcSecret=b""#,
                r#""flv":"https://pull.douyincdn.com/s.flv?expire=1\u0026sign=[REDACTED]\u0026mid=[REDACTED]\u0026volcSecret=[REDACTED]""#,
            ),
            (
                "https://usher.ttvnw.net/api/channel/hls/x.m3u8?sig=abc&token=%7B%22a%22%7D https://tx.flv.huya.com/src/1.flv?wsSecret=w&wsTime=1&fm=RFdxOEJjSjNoNkRK&ctype=huya_live",
                "https://usher.ttvnw.net/api/channel/hls/x.m3u8?sig=[REDACTED]&token=[REDACTED] https://tx.flv.huya.com/src/1.flv?wsSecret=[REDACTED]&wsTime=1&fm=[REDACTED]&ctype=huya_live",
            ),
        ];
        for (line, expected) in cases {
            assert_eq!(redact(line), expected);
        }
    }

    #[tokio::test]
    async fn logs_take_the_tail_and_the_previous_run() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(dir.path().join("bsr-2025-01-01_00-00-00.log"), "old\n")
            .await
            .unwrap();
        tokio::fs::write(dir.path().join("bsr-2025-02-01_00-00-00.log"), "prev\n")
            .await
            .unwrap();
        tokio::fs::write(dir.path().join("bsr.log"), "now\n")
            .await
            .unwrap();
        let log = String::from_utf8(read_logs(dir.path()).await.unwrap()).unwrap();
        assert_eq!(
            log,
            "===== bsr-2025-02-01_00-00-00.log =====\nprev\n\n===== bsr.log =====\nnow\n"
        );
        let cut = tail(&dir.path().join("bsr.log"), 2).await.unwrap();
        assert_eq!(cut, "");
        assert!(read_logs(&dir.path().join("missing")).await.is_none());
    }

    #[tokio::test]
    async fn sends_with_the_frozen_metadata_and_handles_already_submitted() {
        let router = Router::new()
            .route(
                "/feedback/prepare",
                post(|Json(body): Json<serde_json::Value>| async move {
                    assert_eq!(body["app_id"], "bsr");
                    assert_eq!(
                        body["submission_id"],
                        "714aa8ab-2f59-4765-8f7d-ae70577eacdf"
                    );
                    assert_eq!(body["screenshot_size"], 0);
                    assert_eq!(body["log_size"], 5);
                    token_response("t1")
                }),
            )
            .route(
                "/feedback",
                post(
                    |headers: HeaderMap, mut form: axum::extract::Multipart| async move {
                        assert_eq!(headers["x-feedback-token"], "t1");
                        let mut names = Vec::new();
                        while let Some(field) = form.next_field().await.unwrap() {
                            names.push(field.name().unwrap().to_string());
                        }
                        assert_eq!(names, ["message", "log"]);
                        (
                            StatusCode::CREATED,
                            Json(serde_json::json!({"id": "r1", "message": "反馈已收到"})),
                        )
                    },
                ),
            );
        let (base, server) = serve(router).await;
        assert_eq!(send(&mut draft(), &base).await.unwrap(), "r1");
        server.abort();

        let router = Router::new().route(
            "/feedback/prepare",
            post(|| async { Json(serde_json::json!({"id": "r0", "already_submitted": true})) }),
        );
        let (base, server) = serve(router).await;
        assert_eq!(send(&mut draft(), &base).await.unwrap(), "r0");
        server.abort();
    }

    #[tokio::test]
    async fn retries_reuse_a_live_token_and_prepare_again_once_it_is_spent() {
        let prepares = Arc::new(AtomicUsize::new(0));
        let uploads = Arc::new(AtomicUsize::new(0));
        let router = Router::new()
            .route(
                "/feedback/prepare",
                post(|State((prepares, _)): State<(Arc<AtomicUsize>, Arc<AtomicUsize>)>| async move {
                    let n = prepares.fetch_add(1, Ordering::SeqCst) + 1;
                    token_response(&format!("t{n}"))
                }),
            )
            .route(
                "/feedback",
                post(
                    |State((_, uploads)): State<(Arc<AtomicUsize>, Arc<AtomicUsize>)>,
                     headers: HeaderMap| async move {
                        let token = headers["x-feedback-token"].to_str().unwrap().to_string();
                        match uploads.fetch_add(1, Ordering::SeqCst) {
                            0 => (
                                StatusCode::SERVICE_UNAVAILABLE,
                                [("Retry-After", "10")],
                                Json(serde_json::json!({"error": "feedback_unavailable", "message": ""})),
                            )
                                .into_response(),
                            1 => {
                                assert_eq!(token, "t1");
                                (
                                    StatusCode::FORBIDDEN,
                                    Json(serde_json::json!({"error": "invalid_upload_token", "message": ""})),
                                )
                                    .into_response()
                            }
                            _ => {
                                assert_eq!(token, "t2");
                                Json(serde_json::json!({"id": "r2"})).into_response()
                            }
                        }
                    },
                ),
            )
            .with_state((prepares.clone(), uploads.clone()));
        let (base, server) = serve(router).await;
        let mut draft = draft();
        let failure = send(&mut draft, &base).await.unwrap_err();
        assert_eq!(
            (failure.status, failure.code.as_str()),
            (503, "feedback_unavailable")
        );
        assert_eq!(failure.retry_after, Some(10));
        assert!(!failure.discards_draft());
        assert_eq!(send(&mut draft, &base).await.unwrap(), "r2");
        assert_eq!(prepares.load(Ordering::SeqCst), 2);
        assert_eq!(uploads.load(Ordering::SeqCst), 3);
        server.abort();
    }

    #[tokio::test]
    async fn non_json_errors_keep_the_status() {
        let router = Router::new().route(
            "/feedback/prepare",
            post(|| async { (StatusCode::BAD_GATEWAY, "<html>bad gateway</html>") }),
        );
        let (base, server) = serve(router).await;
        let failure = send(&mut draft(), &base).await.unwrap_err();
        assert_eq!((failure.status, failure.code.as_str()), (502, "http_502"));
        assert!(failure.user_message().contains("502"));
        server.abort();
    }
}
