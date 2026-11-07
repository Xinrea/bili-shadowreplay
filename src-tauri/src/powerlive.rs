//! PowerLive account credentials stay in Rust and use the existing encrypted account store.
use crate::database::{Database, DatabaseError};
#[cfg(feature = "gui")]
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::Utc;
use serde::{Deserialize, Serialize};
#[cfg(feature = "gui")]
use std::sync::OnceLock;
use tokio::sync::Mutex;

pub const API_BASE: &str = "https://api.powerlive.io";
const CLIENT_ID: &str = "bsr-desktop";
static OPERATION: Mutex<()> = Mutex::const_new(());

#[derive(Serialize, Deserialize)]
pub struct Session {
    access_token: String,
    refresh_token: String,
    expires_at: i64,
    name: String,
    subject: String,
}

#[derive(Deserialize)]
struct Tokens {
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    expires_in: i64,
    token_type: String,
}

#[cfg(feature = "gui")]
#[derive(Deserialize)]
struct UserInfo {
    sub: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    preferred_username: String,
}

#[cfg(feature = "gui")]
#[derive(Default)]
struct Flow {
    id: Option<String>,
    stop: Option<tokio::sync::watch::Sender<bool>>,
    error: Option<String>,
}
#[cfg(feature = "gui")]
fn flow() -> &'static Mutex<Flow> {
    static FLOW: OnceLock<Mutex<Flow>> = OnceLock::new();
    FLOW.get_or_init(|| Mutex::new(Flow::default()))
}

pub fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|_| "无法初始化 PowerLive 连接".into())
}

async fn load(db: &Database) -> Result<Option<Session>, String> {
    match db.get_account("powerlive", "session").await {
        Ok(row) => serde_json::from_str(&row.cookies)
            .map(Some)
            .map_err(|_| "PowerLive 登录信息损坏，请退出后重新登录".into()),
        Err(DatabaseError::DB(sqlx::Error::RowNotFound)) | Err(DatabaseError::NotFound) => Ok(None),
        Err(error) => Err(error.into()),
    }
}
async fn save(db: &Database, session: &Session) -> Result<(), String> {
    db.save_powerlive_session(&serde_json::to_string(session).map_err(|e| e.to_string())?)
        .await?;
    Ok(())
}
async fn clear_session(db: &Database) -> Result<(), String> {
    match db.remove_account("powerlive", "session").await {
        Ok(()) | Err(DatabaseError::NotFound) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(feature = "gui")]
async fn token_request(values: &[(&str, &str)]) -> Result<Tokens, String> {
    token_request_at(&format!("{API_BASE}/oauth/token"), values)
        .await
        .map_err(|error| error.to_string())
}

#[derive(Debug, thiserror::Error)]
enum TokenError {
    #[error("PowerLive 授权已失效，请重新登录")]
    InvalidCredentials,
    #[error("{0}")]
    Other(String),
}

async fn token_request_at(endpoint: &str, values: &[(&str, &str)]) -> Result<Tokens, TokenError> {
    let response = client()
        .map_err(TokenError::Other)?
        .post(endpoint)
        .form(values)
        .send()
        .await
        .map_err(|_| TokenError::Other("无法连接 PowerLive 登录服务，请稍后重试".into()))?;
    let status = response.status();
    if matches!(
        status,
        reqwest::StatusCode::BAD_REQUEST | reqwest::StatusCode::UNAUTHORIZED
    ) {
        return Err(TokenError::InvalidCredentials);
    }
    if !status.is_success() {
        return Err(TokenError::Other(format!(
            "PowerLive 登录服务暂不可用（{status}），请稍后重试"
        )));
    }
    let tokens: Tokens = response
        .json()
        .await
        .map_err(|_| TokenError::Other("PowerLive 返回了无效的登录信息".into()))?;
    if tokens.access_token.is_empty()
        || tokens.expires_in <= 0
        || !tokens.token_type.eq_ignore_ascii_case("bearer")
    {
        return Err(TokenError::Other("PowerLive 返回了无效的登录信息".into()));
    }
    Ok(tokens)
}

// Only token refreshes hold the operation lock; long subtitle uploads do not block logout.
async fn session_for_request(db: &Database, endpoint: &str) -> Result<Session, String> {
    let _guard = OPERATION.lock().await;
    refresh_session(db, endpoint, None).await
}

/// Retry an unauthorized request once, without reusing a rotated refresh token or changing accounts.
pub async fn authenticated_request(
    db: &Database,
    base: &str,
    build: impl Fn(&str) -> Result<reqwest::RequestBuilder, String>,
) -> Result<(reqwest::Response, String), String> {
    let endpoint = format!("{base}/oauth/token");
    let mut session = session_for_request(db, &endpoint).await?;
    for attempt in 0..2 {
        let response = build(&session.access_token)?
            .send()
            .await
            .map_err(|_| "无法连接 PowerLive 服务，请检查网络后重试")?;
        if response.status() != reqwest::StatusCode::UNAUTHORIZED {
            return Ok((response, session.subject));
        }
        let _guard = OPERATION.lock().await;
        let current = load(db).await?.ok_or("请先在设置中登录 PowerLive 账号")?;
        if current.subject != session.subject {
            return Err("PowerLive 账号已变更，请重新操作".into());
        }
        if attempt == 1 {
            // A concurrent request may already have saved a newer, valid access token.
            if current.access_token == session.access_token {
                clear_session(db).await?;
            }
            return Err("PowerLive 登录已失效，请重新登录".into());
        }
        session = refresh_session(db, &endpoint, Some(&session.access_token)).await?;
    }
    unreachable!("unauthorized requests return after one retry")
}

async fn refresh_session(
    db: &Database,
    endpoint: &str,
    rejected_token: Option<&str>,
) -> Result<Session, String> {
    let mut session = load(db).await?.ok_or("请先在设置中登录 PowerLive 账号")?;
    if session.expires_at <= Utc::now().timestamp() + 60
        || rejected_token == Some(session.access_token.as_str())
    {
        let tokens = match token_request_at(
            endpoint,
            &[
                ("grant_type", "refresh_token"),
                ("client_id", CLIENT_ID),
                ("refresh_token", &session.refresh_token),
            ],
        )
        .await
        {
            Ok(tokens) => tokens,
            Err(error) => {
                if matches!(error, TokenError::InvalidCredentials) {
                    clear_session(db).await?;
                }
                return Err(error.to_string());
            }
        };
        session.access_token = tokens.access_token;
        if !tokens.refresh_token.is_empty() {
            session.refresh_token = tokens.refresh_token;
        }
        session.expires_at = Utc::now().timestamp() + tokens.expires_in;
        save(db, &session).await?;
    }
    Ok(session)
}

#[cfg(feature = "gui")]
#[derive(Serialize)]
pub struct Status {
    logged_in: bool,
    name: String,
    subject: String,
    pending: bool,
    error: Option<String>,
}

#[cfg(feature = "gui")]
#[tauri::command]
pub async fn get_powerlive_status(
    state: tauri::State<'_, crate::state::State>,
) -> Result<Status, String> {
    let session = load(&state.db).await?;
    let flow = flow().lock().await;
    Ok(Status {
        logged_in: session.is_some(),
        name: session.as_ref().map(|s| s.name.clone()).unwrap_or_default(),
        subject: session.map(|s| s.subject).unwrap_or_default(),
        pending: flow.id.is_some(),
        error: flow.error.clone(),
    })
}

#[cfg(any(feature = "gui", test))]
#[derive(Serialize, Deserialize)]
pub struct AccountInfo {
    subject: String,
    name: String,
    credit: i64,
}

#[cfg(feature = "gui")]
#[tauri::command]
pub async fn get_powerlive_account(
    state: tauri::State<'_, crate::state::State>,
) -> Result<AccountInfo, String> {
    account_info_from(&state.db, API_BASE).await
}

#[cfg(any(feature = "gui", test))]
async fn account_info_from(db: &Database, base: &str) -> Result<AccountInfo, String> {
    let client = client()?;
    let (response, subject) = authenticated_request(db, base, |token| {
        Ok(client
            .get(format!("{base}/service/account"))
            .bearer_auth(token))
    })
    .await?;
    if !response.status().is_success() {
        return Err(format!(
            "PowerLive 额度刷新失败（{}），请稍后重试",
            response.status()
        ));
    }
    let account: AccountInfo = response
        .json()
        .await
        .map_err(|_| "PowerLive 返回的额度信息无效")?;
    if account.subject != subject {
        return Err("PowerLive 账号信息不匹配，请重新登录".into());
    }
    Ok(account)
}

#[cfg(feature = "gui")]
#[derive(Clone)]
struct Callback {
    id: String,
    verifier: String,
    redirect: String,
    state: crate::state::State,
}

#[cfg(feature = "gui")]
#[tauri::command]
pub async fn begin_powerlive_login(
    state: tauri::State<'_, crate::state::State>,
) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let _guard = OPERATION.lock().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|_| "无法打开本机登录回调端口")?;
    let redirect = format!(
        "http://127.0.0.1:{}/oauth/callback",
        listener.local_addr().map_err(|e| e.to_string())?.port()
    );
    let id = uuid::Uuid::new_v4().to_string();
    let verifier = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let mut url =
        url::Url::parse(&format!("{API_BASE}/oauth/authorize")).map_err(|e| e.to_string())?;
    url.query_pairs_mut().extend_pairs([
        ("client_id", CLIENT_ID),
        ("response_type", "code"),
        ("redirect_uri", &redirect),
        ("scope", "openid profile offline_access subtitle:generate"),
        ("state", &id),
        ("prompt", "login"),
        ("code_challenge", &challenge),
        ("code_challenge_method", "S256"),
    ]);
    let (stop, mut receiver) = tokio::sync::watch::channel(false);
    {
        let mut flow = flow().lock().await;
        if let Some(previous) = flow.stop.take() {
            let _ = previous.send(true);
        }
        *flow = Flow {
            id: Some(id.clone()),
            stop: Some(stop),
            error: None,
        };
    }
    let callback = Callback {
        id: id.clone(),
        verifier,
        redirect,
        state: state.inner().clone(),
    };
    let router = axum::Router::new()
        .route("/oauth/callback", axum::routing::get(login_callback))
        .with_state(callback);
    tokio::spawn(async move {
        let server = axum::serve(listener, router).with_graceful_shutdown(async move {
            tokio::select! {
                _ = receiver.changed() => {},
                _ = tokio::time::sleep(std::time::Duration::from_secs(180)) => {},
            }
        });
        let _ = server.await;
        let mut flow = flow().lock().await;
        if flow.id.as_deref() == Some(&id) {
            flow.id = None;
            flow.stop = None;
            flow.error = Some("登录超时，请重新打开登录窗口".into());
        }
    });
    Ok(url.into())
}

#[cfg(feature = "gui")]
async fn login_callback(
    axum::extract::State(callback): axum::extract::State<Callback>,
    axum::extract::RawQuery(query): axum::extract::RawQuery,
) -> axum::response::Response {
    let _guard = OPERATION.lock().await;
    let result = validate_callback(query.as_deref().unwrap_or(""), &callback.id);
    if flow().lock().await.id.as_deref() != Some(&callback.id)
        || matches!(&result, Err(error) if error == "登录回调校验失败")
    {
        return login_page_response(LoginPage::InvalidCallback);
    }
    let result = match result {
        Ok(code) => complete_login(&callback, &code).await,
        Err(error) => Err(error),
    };
    let success = result.is_ok();
    {
        let mut flow = flow().lock().await;
        flow.id = None;
        flow.error = result.err();
        if let Some(stop) = flow.stop.take() {
            let _ = stop.send(true);
        }
    }
    if success {
        crate::tray::show_main_window(&callback.state.app_handle);
    }
    login_page_response(if success {
        LoginPage::Success
    } else {
        LoginPage::Failed
    })
}

#[cfg(feature = "gui")]
enum LoginPage {
    Success,
    Failed,
    InvalidCallback,
}

#[cfg(feature = "gui")]
fn login_page_response(page: LoginPage) -> axum::response::Response {
    use axum::{http::StatusCode, response::IntoResponse};

    let (status, appearance, title, description, hint, script) = match page {
        LoginPage::Success => (
            StatusCode::OK,
            "success",
            "账号连接成功",
            "PowerLive 已完成授权。返回 Bili ShadowReplay，即可使用账号额度生成字幕。",
            "如果软件没有自动打开，请点击上方按钮继续。",
            "<script>location.href='bsr://powerlive/login';</script>",
        ),
        LoginPage::Failed => (
            StatusCode::OK,
            "error",
            "未能完成登录",
            "此次 PowerLive 授权未完成。请返回 Bili ShadowReplay 查看提示，然后重新登录。",
            "请在「设置 → PowerLive 账号」中重新发起登录。",
            "",
        ),
        LoginPage::InvalidCallback => (
            StatusCode::BAD_REQUEST,
            "error",
            "登录链接已失效",
            "无法验证此次登录请求，请返回 Bili ShadowReplay 重新发起登录。",
            "请使用软件打开的最新登录页面，避免刷新或重复使用旧链接。",
            "",
        ),
    };
    // Only fixed copy is substituted; OAuth query parameters and tokens never enter the page.
    let html = include_str!("powerlive_callback.html")
        .replace("{{logo}}", include_str!("../../public/icon.svg"))
        .replace("{{appearance}}", appearance)
        .replace("{{title}}", title)
        .replace("{{description}}", description)
        .replace("{{hint}}", hint)
        .replace("{{script}}", script);
    (
        status,
        [
            ("Cache-Control", "no-store"),
            ("Referrer-Policy", "no-referrer"),
            ("Content-Security-Policy", "default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'"),
        ],
        axum::response::Html(html),
    )
        .into_response()
}

#[cfg(any(feature = "gui", test))]
fn validate_callback(query: &str, expected_state: &str) -> Result<String, String> {
    let values: Vec<_> = url::form_urlencoded::parse(query.as_bytes()).collect();
    let states: Vec<_> = values.iter().filter(|(key, _)| key == "state").collect();
    if states.len() != 1 || states[0].1 != expected_state {
        return Err("登录回调校验失败".into());
    }
    if values.iter().any(|(key, _)| key == "error") {
        return Err("PowerLive 登录已取消或授权失败".into());
    }
    let codes: Vec<_> = values.iter().filter(|(key, _)| key == "code").collect();
    if codes.len() != 1 || codes[0].1.is_empty() {
        return Err("登录回调校验失败".into());
    }
    Ok(codes[0].1.to_string())
}

#[cfg(feature = "gui")]
async fn complete_login(callback: &Callback, code: &str) -> Result<(), String> {
    let tokens = token_request(&[
        ("grant_type", "authorization_code"),
        ("client_id", CLIENT_ID),
        ("redirect_uri", &callback.redirect),
        ("code", code),
        ("code_verifier", &callback.verifier),
    ])
    .await?;
    if tokens.refresh_token.is_empty() {
        return Err("PowerLive 未授予持久登录权限，请重新登录".into());
    }
    let response = client()?
        .get(format!("{API_BASE}/oauth/userinfo"))
        .bearer_auth(&tokens.access_token)
        .send()
        .await
        .map_err(|_| "无法读取 PowerLive 账号信息")?;
    if !response.status().is_success() {
        return Err("无法验证 PowerLive 账号，请重新登录".into());
    }
    let user: UserInfo = response
        .json()
        .await
        .map_err(|_| "PowerLive 账号信息无效")?;
    if user.sub.is_empty() {
        return Err("PowerLive 账号信息无效".into());
    }
    let name = if !user.name.is_empty() {
        user.name
    } else if !user.preferred_username.is_empty() {
        user.preferred_username
    } else {
        user.sub.clone()
    };
    save(
        &callback.state.db,
        &Session {
            access_token: tokens.access_token,
            refresh_token: tokens.refresh_token,
            expires_at: Utc::now().timestamp() + tokens.expires_in,
            name,
            subject: user.sub,
        },
    )
    .await
}

#[cfg(feature = "gui")]
#[tauri::command]
pub async fn cancel_powerlive_login() {
    let _guard = OPERATION.lock().await;
    let mut flow = flow().lock().await;
    if let Some(stop) = flow.stop.take() {
        let _ = stop.send(true);
    }
    flow.id = None;
    flow.error = None;
}

#[cfg(feature = "gui")]
#[tauri::command]
pub async fn logout_powerlive(
    state: tauri::State<'_, crate::state::State>,
    local_only: Option<bool>,
) -> Result<(), String> {
    let _guard = OPERATION.lock().await;
    let mut flow = flow().lock().await;
    if let Some(stop) = flow.stop.take() {
        let _ = stop.send(true);
    }
    flow.id = None;
    flow.error = None;
    drop(flow);
    logout_session(
        &state.db,
        &format!("{API_BASE}/oauth/revoke"),
        local_only.unwrap_or(false),
    )
    .await
}

#[cfg(any(feature = "gui", test))]
async fn logout_session(db: &Database, endpoint: &str, local_only: bool) -> Result<(), String> {
    if !local_only {
        if let Some(session) = load(db).await? {
            // Retain credentials on temporary failures; explicit local-only logout bypasses the network.
            let response = client()?
                .post(endpoint)
                .form(&[
                    ("client_id", CLIENT_ID),
                    ("token", &session.refresh_token),
                    ("token_type_hint", "refresh_token"),
                ])
                .send()
                .await
                .map_err(|_| "退出登录失败，请检查网络后重试")?;
            let status = response.status();
            if !status.is_success()
                && !matches!(
                    status,
                    reqwest::StatusCode::BAD_REQUEST | reqwest::StatusCode::UNAUTHORIZED
                )
            {
                return Err("退出登录失败，请稍后重试，或仅清除本地登录".into());
            }
        }
    }
    clear_session(db).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn concurrent_subtitles_refresh_once_and_persist_rotated_credentials() {
        use axum::{
            extract::{Form, State},
            routing::post,
            Json, Router,
        };
        use std::{
            collections::HashMap,
            sync::{
                atomic::{AtomicUsize, Ordering},
                Arc,
            },
        };
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        for migration in crate::get_migrations() {
            sqlx::raw_sql(migration.sql).execute(&pool).await.unwrap();
        }
        let db = Database::new(
            pool,
            crate::database::credentials::CredentialCipher::new(&[0; 32]).unwrap(),
            crate::events::EventBus::new(),
        );
        save(
            &db,
            &Session {
                access_token: "expired".into(),
                refresh_token: "original-refresh".into(),
                expires_at: 0,
                name: "Alice".into(),
                subject: "user-1".into(),
            },
        )
        .await
        .unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let router = Router::new().route("/oauth/token", post(|State(calls): State<Arc<AtomicUsize>>, Form(form): Form<HashMap<String, String>>| async move {
            calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(form["grant_type"], "refresh_token");
            assert_eq!(form["client_id"], "bsr-desktop");
            assert_eq!(form["refresh_token"], "original-refresh");
            Json(serde_json::json!({"access_token":"new-access", "refresh_token":"rotated-refresh", "expires_in":900, "token_type":"Bearer"}))
        })).with_state(calls.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/oauth/token", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let (first, second) = tokio::join!(
            session_for_request(&db, &endpoint),
            session_for_request(&db, &endpoint)
        );
        assert_eq!(first.unwrap().access_token, "new-access");
        assert_eq!(second.unwrap().access_token, "new-access");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let stored = load(&db).await.unwrap().unwrap();
        assert_eq!(stored.refresh_token, "rotated-refresh");
        assert_eq!(stored.subject, "user-1");
        server.abort();
    }

    #[tokio::test]
    async fn balance_uses_account_bearer_including_zero_and_rejects_wrong_accounts() {
        use axum::{
            extract::State,
            http::{HeaderMap, StatusCode},
            response::IntoResponse,
            routing::get,
            Json, Router,
        };
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        for migration in crate::get_migrations() {
            sqlx::raw_sql(migration.sql).execute(&pool).await.unwrap();
        }
        let db = Database::new(
            pool,
            crate::database::credentials::CredentialCipher::new(&[0; 32]).unwrap(),
            crate::events::EventBus::new(),
        );
        save(
            &db,
            &Session {
                access_token: "account-token".into(),
                refresh_token: "refresh-token".into(),
                expires_at: Utc::now().timestamp() + 900,
                name: "Alice".into(),
                subject: "user-1".into(),
            },
        )
        .await
        .unwrap();
        let router = Router::new().route("/service/account", get(|State(calls): State<Arc<AtomicUsize>>, headers: HeaderMap| async move {
            assert_eq!(headers["authorization"], "Bearer account-token");
            match calls.fetch_add(1, Ordering::SeqCst) {
                0 => Json(serde_json::json!({"subject": "user-1", "name": "Alice", "credit": 120})).into_response(),
                1 => Json(serde_json::json!({"subject": "user-1", "name": "Alice", "credit": 0})).into_response(),
                2 => Json(serde_json::json!({"subject": "another-user", "name": "Bob", "credit": 999})).into_response(),
                _ => StatusCode::UNAUTHORIZED.into_response(),
            }
        })).with_state(Arc::new(AtomicUsize::new(0)));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        assert_eq!(account_info_from(&db, &base).await.unwrap().credit, 120);
        assert_eq!(account_info_from(&db, &base).await.unwrap().credit, 0);
        assert!(account_info_from(&db, &base)
            .await
            .err()
            .unwrap()
            .contains("不匹配"));
        server.abort();
    }

    async fn valid_session_db() -> Database {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        for migration in crate::get_migrations() {
            sqlx::raw_sql(migration.sql).execute(&pool).await.unwrap();
        }
        let db = Database::new(
            pool,
            crate::database::credentials::CredentialCipher::new(&[0; 32]).unwrap(),
            crate::events::EventBus::new(),
        );
        save(
            &db,
            &Session {
                access_token: "account-token".into(),
                refresh_token: "refresh-token".into(),
                expires_at: Utc::now().timestamp() + 900,
                name: "Alice".into(),
                subject: "user-1".into(),
            },
        )
        .await
        .unwrap();
        db
    }

    #[tokio::test]
    async fn revoke_invalid_tokens_clears_login_but_temporary_errors_allow_local_logout() {
        use axum::{http::StatusCode, routing::post, Router};
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        for status in [
            StatusCode::NO_CONTENT,
            StatusCode::BAD_REQUEST,
            StatusCode::UNAUTHORIZED,
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::SERVICE_UNAVAILABLE,
        ] {
            let db = valid_session_db().await;
            let calls = Arc::new(AtomicUsize::new(0));
            let requests = calls.clone();
            let router = Router::new().route(
                "/revoke",
                post(move || {
                    requests.fetch_add(1, Ordering::SeqCst);
                    async move { status }
                }),
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = format!("http://{}/revoke", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                axum::serve(listener, router).await.unwrap();
            });
            let result = logout_session(&db, &endpoint, false).await;
            let temporary = status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error();
            assert_eq!(result.is_err(), temporary, "{status}");
            assert_eq!(load(&db).await.unwrap().is_some(), temporary, "{status}");
            logout_session(&db, &endpoint, true).await.unwrap();
            assert!(load(&db).await.unwrap().is_none());
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            server.abort();
        }
    }

    #[tokio::test]
    async fn offline_logout_can_clear_local_and_corrupt_credentials_without_network() {
        let db = valid_session_db().await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/revoke", listener.local_addr().unwrap());
        drop(listener);
        assert!(logout_session(&db, &endpoint, false).await.is_err());
        assert!(load(&db).await.unwrap().is_some());
        logout_session(&db, "not-a-url", true).await.unwrap();
        assert!(load(&db).await.unwrap().is_none());
        db.save_powerlive_session("invalid session JSON")
            .await
            .unwrap();
        logout_session(&db, "not-a-url", true).await.unwrap();
        assert!(load(&db).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn unauthorized_account_refresh_distinguishes_invalid_and_temporary_failures() {
        use axum::{
            http::StatusCode,
            response::IntoResponse,
            routing::{get, post},
            Json, Router,
        };
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        for status in [
            StatusCode::BAD_REQUEST,
            StatusCode::UNAUTHORIZED,
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::BAD_GATEWAY,
            StatusCode::SERVICE_UNAVAILABLE,
            StatusCode::OK,
        ] {
            let db = valid_session_db().await;
            let calls = Arc::new(AtomicUsize::new(0));
            let requests = calls.clone();
            let router = Router::new()
                .route("/service/account", get(move || {
                    requests.fetch_add(1, Ordering::SeqCst);
                    async { StatusCode::UNAUTHORIZED }
                }))
                .route("/oauth/token", post(move || async move {
                    if status == StatusCode::OK {
                        Json(serde_json::json!({"access_token":"new-access", "refresh_token":"new-refresh", "expires_in":900, "token_type":"Bearer"})).into_response()
                    } else { status.into_response() }
                }));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                axum::serve(listener, router).await.unwrap();
            });
            let error = account_info_from(&db, &base).await.err().unwrap();
            let temporary = status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error();
            assert_eq!(load(&db).await.unwrap().is_some(), temporary, "{status}");
            assert!(
                error.contains(if temporary {
                    "请稍后重试"
                } else {
                    "失效"
                }),
                "{status}: {error}"
            );
            assert_eq!(
                calls.load(Ordering::SeqCst),
                if status == StatusCode::OK { 2 } else { 1 }
            );
            server.abort();
        }
    }

    #[tokio::test]
    async fn concurrent_unauthorized_accounts_refresh_once_and_retry_with_rotated_token() {
        use axum::{
            extract::Form,
            http::{HeaderMap, StatusCode},
            response::IntoResponse,
            routing::{get, post},
            Json, Router,
        };
        use std::{
            collections::HashMap,
            sync::{
                atomic::{AtomicUsize, Ordering},
                Arc,
            },
        };
        let db = valid_session_db().await;
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let refreshes = Arc::new(AtomicUsize::new(0));
        let calls = refreshes.clone();
        let router = Router::new()
            .route("/service/account", get(move |headers: HeaderMap| {
                let barrier = barrier.clone();
                async move {
                    if headers["authorization"] == "Bearer account-token" {
                        barrier.wait().await;
                        StatusCode::UNAUTHORIZED.into_response()
                    } else {
                        assert_eq!(headers["authorization"], "Bearer new-access");
                        Json(serde_json::json!({"subject":"user-1", "name":"Alice", "credit":120})).into_response()
                    }
                }
            }))
            .route("/oauth/token", post(move |Form(form): Form<HashMap<String,String>>| {
                calls.fetch_add(1, Ordering::SeqCst);
                assert_eq!(form["refresh_token"], "refresh-token");
                async { Json(serde_json::json!({"access_token":"new-access", "refresh_token":"rotated-refresh", "expires_in":900, "token_type":"Bearer"})) }
            }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let (first, second) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            tokio::join!(account_info_from(&db, &base), account_info_from(&db, &base))
        })
        .await
        .unwrap();
        assert_eq!(first.unwrap().credit, 120);
        assert_eq!(second.unwrap().credit, 120);
        assert_eq!(refreshes.load(Ordering::SeqCst), 1);
        assert_eq!(
            load(&db).await.unwrap().unwrap().refresh_token,
            "rotated-refresh"
        );
        server.abort();
    }

    #[test]
    fn callback_requires_matching_unique_state_and_code() {
        assert_eq!(
            validate_callback("state=expected&code=abc", "expected").unwrap(),
            "abc"
        );
        for query in [
            "state=wrong&code=abc",
            "code=abc",
            "state=expected&state=expected&code=abc",
            "state=expected&code=abc&code=def",
            "state=expected&code=",
        ] {
            assert!(validate_callback(query, "expected").is_err());
        }
        assert!(
            validate_callback("state=expected&error=access_denied", "expected")
                .unwrap_err()
                .contains("取消")
        );
    }
}
