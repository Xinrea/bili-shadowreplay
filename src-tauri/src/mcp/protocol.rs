//! Stateless Streamable HTTP (JSON responses), MCP 2025-11-25 and earlier.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{header, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use futures::future::BoxFuture;
use serde_json::{json, Value};

const VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26"];

pub(super) trait Backend: Send + Sync {
    fn definitions(&self) -> Vec<Value>;
    fn call<'a>(&'a self, name: &'a str, arguments: Value) -> BoxFuture<'a, Result<Value, String>>;
}

#[derive(Clone)]
pub(super) struct RpcContext {
    pub backend: Arc<dyn Backend>,
    pub active: Arc<AtomicBool>,
}

pub(super) fn router(context: RpcContext, port: u16) -> Router {
    Router::new()
        .route(
            "/mcp",
            post(message)
                .get(method_not_allowed)
                .delete(method_not_allowed),
        )
        .layer(DefaultBodyLimit::max(16 * 1024 * 1024))
        .layer(middleware::from_fn_with_state(
            (context.active.clone(), port),
            guard,
        ))
        .with_state(context)
}

async fn guard(
    State((active, port)): State<(Arc<AtomicBool>, u16)>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    if !active.load(Ordering::Acquire) {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            [(header::CONNECTION, "close")],
        )
            .into_response();
    }
    if !safe_headers(request.headers(), port) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if let Some(version) = request.headers().get("mcp-protocol-version") {
        if request
            .headers()
            .get_all("mcp-protocol-version")
            .iter()
            .count()
            != 1
            || !version
                .to_str()
                .is_ok_and(|version| VERSIONS.contains(&version))
        {
            return StatusCode::BAD_REQUEST.into_response();
        }
    }
    next.run(request).await
}

fn safe_headers(headers: &HeaderMap, port: u16) -> bool {
    if headers.get_all(header::HOST).iter().count() != 1
        || headers.get_all(header::ORIGIN).iter().count() > 1
    {
        return false;
    }
    let Some(host) = headers
        .get(header::HOST)
        .and_then(|host| host.to_str().ok())
    else {
        return false;
    };
    if ![format!("127.0.0.1:{port}"), format!("localhost:{port}")]
        .iter()
        .any(|allowed| host.eq_ignore_ascii_case(allowed))
    {
        return false;
    }
    // Native MCP clients omit Origin. Browser origins must be local; never inherit
    // the web API's permissive CORS policy on this privileged endpoint.
    match headers.get(header::ORIGIN) {
        None => true,
        Some(origin) => origin
            .to_str()
            .ok()
            .and_then(|origin| url::Url::parse(origin).ok())
            .is_some_and(|origin| {
                matches!(origin.scheme(), "http" | "https")
                    && matches!(origin.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
                    && origin.username().is_empty()
                    && origin.password().is_none()
                    && origin.path() == "/"
                    && origin.query().is_none()
                    && origin.fragment().is_none()
            }),
    }
}

async fn method_not_allowed() -> impl IntoResponse {
    // No server-initiated notifications or stateful sessions are advertised.
    (StatusCode::METHOD_NOT_ALLOWED, [(header::ALLOW, "POST")])
}

fn rpc_error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message.into()}})
}

fn error_response(
    id: Value,
    code: i64,
    message: impl Into<String>,
    status: StatusCode,
) -> Response {
    (status, Json(rpc_error(id, code, message))).into_response()
}

async fn message(State(context): State<RpcContext>, headers: HeaderMap, body: Bytes) -> Response {
    let is_json = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .eq_ignore_ascii_case("application/json")
        });
    if !is_json {
        return StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response();
    }
    let accepts = |expected: &str| {
        headers
            .get_all(header::ACCEPT)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .any(|value| {
                value
                    .split(',')
                    .any(|part| part.split(';').next().unwrap_or("").trim() == expected)
            })
    };
    if !accepts("application/json") || !accepts("text/event-stream") {
        return StatusCode::NOT_ACCEPTABLE.into_response();
    }
    let request: Value = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(_) => {
            return error_response(Value::Null, -32700, "Parse error", StatusCode::BAD_REQUEST)
        }
    };
    let id = request.get("id").cloned();
    let valid_id = id
        .as_ref()
        .is_none_or(|id| id.is_string() || id.is_i64() || id.is_u64());
    if !request.is_object() || request["jsonrpc"] != "2.0" || !valid_id {
        return error_response(
            Value::Null,
            -32600,
            "Invalid Request",
            StatusCode::BAD_REQUEST,
        );
    }
    // A client response is legal input, even though this service never requests sampling.
    if request.get("method").is_none()
        && id.is_some()
        && (request.get("result").is_some() ^ request.get("error").is_some())
    {
        return StatusCode::ACCEPTED.into_response();
    }
    let Some(method) = request["method"].as_str() else {
        return error_response(
            id.unwrap_or(Value::Null),
            -32600,
            "Invalid Request",
            StatusCode::BAD_REQUEST,
        );
    };
    let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
    if !params.is_object() || request.get("result").is_some() || request.get("error").is_some() {
        return error_response(
            id.unwrap_or(Value::Null),
            -32600,
            "Invalid Request",
            StatusCode::BAD_REQUEST,
        );
    }
    let Some(id) = id else {
        // Never execute tools from a notification: only requests receive a result.
        if method.starts_with("notifications/") {
            return StatusCode::ACCEPTED.into_response();
        }
        return error_response(
            Value::Null,
            -32600,
            "Expected a JSON-RPC request ID",
            StatusCode::BAD_REQUEST,
        );
    };
    let result = match method {
        "initialize" => {
            let Some(requested) = params["protocolVersion"].as_str() else {
                return error_response(id, -32602, "Missing protocolVersion", StatusCode::OK);
            };
            if !params["capabilities"].is_object()
                || !params["clientInfo"]["name"].is_string()
                || !params["clientInfo"]["version"].is_string()
            {
                return error_response(
                    id,
                    -32602,
                    "Invalid client capabilities or clientInfo",
                    StatusCode::OK,
                );
            }
            json!({
                "protocolVersion": if VERSIONS.contains(&requested) { requested } else { VERSIONS[0] },
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "bsr", "version": env!("CARGO_PKG_VERSION"), "title": "BiliBili ShadowReplay"},
                "instructions": "BSR records live rooms (Recorder) into archives, cuts archives into materials (Video), combines materials into projects (切片) and prepares Bilibili uploads (投稿). Workflow: find an archive (get_recorder_list, get_archives); choose times from its subtitles and danmu (get_archive_subtitle, analyze_danmu_highlights, search_danmu_keywords, get_danmu_record), whose times are seconds from the start of the recording, the same base as clip ranges; cut materials (generate_materials or clip_range); arrange them in a project (create_project, set_project_clips); then create an upload draft (create_upload_draft) for a person to review and post in BSR, which this server never does. Long tools return a task_id: poll get_task until success, failed or cancelled; the result is in metadata.result. State-changing tools execute immediately; obtain the user's approval in your client before calling them."
            })
        }
        "ping" => json!({}),
        "tools/list" => {
            if params.get("cursor").is_some() {
                return error_response(
                    id,
                    -32602,
                    "This server does not paginate tools",
                    StatusCode::OK,
                );
            }
            json!({"tools": context.backend.definitions()})
        }
        "tools/call" => {
            let Some(name) = params["name"].as_str() else {
                return error_response(id, -32602, "Missing tool name", StatusCode::OK);
            };
            let Some(definition) = context
                .backend
                .definitions()
                .into_iter()
                .find(|tool| tool["name"] == name)
            else {
                return error_response(id, -32602, format!("Unknown tool: {name}"), StatusCode::OK);
            };
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            if let Err(error) =
                validate_arguments(&definition["inputSchema"], &arguments, "arguments")
            {
                return error_response(id, -32602, error, StatusCode::OK);
            }
            match context.backend.call(name, arguments).await {
                Ok(value) => tool_result(value),
                Err(error) => {
                    json!({"content": [{"type": "text", "text": error}], "isError": true})
                }
            }
        }
        _ => {
            return error_response(
                id,
                -32601,
                format!("Method not found: {method}"),
                StatusCode::OK,
            )
        }
    };
    Json(json!({"jsonrpc": "2.0", "id": id, "result": result})).into_response()
}

fn tool_result(value: Value) -> Value {
    let data = value
        .get("response")
        .filter(|_| value["parts"].is_array())
        .unwrap_or(&value);
    let structured = if data.is_object() {
        data.clone()
    } else {
        json!({"result": data})
    };
    let mut content = vec![json!({"type": "text", "text": structured.to_string()})];
    if let Some(parts) = value["parts"].as_array() {
        content.extend(
            parts
                .iter()
                .filter(|part| part["type"] == "image")
                .filter(|part| part["data"].is_string() && part["mimeType"].is_string())
                .cloned(),
        );
    }
    json!({"content": content, "structuredContent": structured, "isError": false})
}

/// Validate the subset of JSON Schema used by BSR's tool definitions recursively.
/// Keeping it here means malformed inputs never reach a state-changing handler.
fn validate_arguments(schema: &Value, value: &Value, path: &str) -> Result<(), String> {
    let valid_type = match schema["type"].as_str() {
        Some("object") => value.is_object(),
        Some("array") => value.is_array(),
        Some("string") => value.is_string(),
        Some("integer") => value.is_i64() || value.is_u64(),
        Some("number") => value.is_number(),
        Some("boolean") => value.is_boolean(),
        _ => true,
    };
    if !valid_type {
        return Err(format!("{path}: expected {}", schema["type"]));
    }
    if let Some(allowed) = schema["enum"].as_array() {
        if !allowed.contains(value) {
            return Err(format!("{path}: value is not in the allowed enum"));
        }
    }
    if let Some(number) = value.as_f64() {
        if schema["minimum"]
            .as_f64()
            .is_some_and(|minimum| number < minimum)
            || schema["maximum"]
                .as_f64()
                .is_some_and(|maximum| number > maximum)
            || schema["exclusiveMinimum"]
                .as_f64()
                .is_some_and(|minimum| number <= minimum)
        {
            return Err(format!("{path}: number is outside the allowed range"));
        }
    }
    if let Some(text) = value.as_str() {
        let length = text.chars().count() as u64;
        if schema["minLength"]
            .as_u64()
            .is_some_and(|minimum| length < minimum)
        {
            return Err(format!("{path}: string is too short"));
        }
        if schema["maxLength"]
            .as_u64()
            .is_some_and(|maximum| length > maximum)
        {
            return Err(format!("{path}: string is too long"));
        }
    }
    if let Some(properties) = value.as_object() {
        if let Some(required) = schema["required"].as_array() {
            for key in required.iter().filter_map(Value::as_str) {
                if !properties.contains_key(key) {
                    return Err(format!("{path}: missing required argument {key}"));
                }
            }
        }
        for (key, value) in properties {
            if let Some(child) = schema["properties"].get(key) {
                validate_arguments(child, value, &format!("{path}.{key}"))?;
            } else if schema["additionalProperties"] == false {
                return Err(format!("{path}: unknown argument {key}"));
            }
        }
    }
    if let Some(items) = value.as_array() {
        if schema["minItems"]
            .as_u64()
            .is_some_and(|minimum| (items.len() as u64) < minimum)
            || schema["maxItems"]
                .as_u64()
                .is_some_and(|maximum| (items.len() as u64) > maximum)
        {
            return Err(format!("{path}: array length is outside the allowed range"));
        }
        for (index, item) in items.iter().enumerate() {
            validate_arguments(&schema["items"], item, &format!("{path}[{index}]"))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod mcp_server_tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[derive(Default)]
    struct TestBackend(AtomicUsize);

    impl Backend for TestBackend {
        fn definitions(&self) -> Vec<Value> {
            vec![
                json!({
                    "name": "echo",
                    "inputSchema": {"type": "object", "properties": {"count": {"type": "integer", "minimum": 1, "maximum": 10}}, "required": ["count"], "additionalProperties": false}
                }),
                json!({"name": "fail", "inputSchema": {"type": "object", "additionalProperties": false}}),
                json!({"name": "frames", "inputSchema": {"type": "object", "additionalProperties": false}}),
            ]
        }

        fn call<'a>(
            &'a self,
            name: &'a str,
            arguments: Value,
        ) -> BoxFuture<'a, Result<Value, String>> {
            Box::pin(async move {
                self.0.fetch_add(1, Ordering::Relaxed);
                match name {
                    "fail" => Err("Video not found".into()),
                    "frames" => Ok(
                        json!({"response": {"timestamp": 12}, "parts": [{"type": "image", "data": "frame-bytes", "mimeType": "image/jpeg"}]}),
                    ),
                    _ => Ok(arguments),
                }
            })
        }
    }

    struct TestServer {
        url: String,
        client: reqwest::Client,
        active: Arc<AtomicBool>,
        backend: Arc<TestBackend>,
        task: tokio::task::JoinHandle<()>,
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    impl TestServer {
        async fn start() -> Self {
            let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
                .await
                .unwrap();
            let port = listener.local_addr().unwrap().port();
            let active = Arc::new(AtomicBool::new(true));
            let backend = Arc::new(TestBackend::default());
            let app = router(
                RpcContext {
                    backend: backend.clone(),
                    active: active.clone(),
                },
                port,
            );
            Self {
                url: format!("http://127.0.0.1:{port}/mcp"),
                client: reqwest::Client::builder().no_proxy().build().unwrap(),
                active,
                backend,
                task: tokio::spawn(async move {
                    axum::serve(listener, app).await.unwrap();
                }),
            }
        }

        fn post(&self, request: Value) -> reqwest::RequestBuilder {
            self.client
                .post(&self.url)
                .header("accept", "application/json, text/event-stream")
                .header("mcp-protocol-version", "2025-11-25")
                .json(&request)
        }

        async fn rpc(&self, method: &str, params: Value) -> Value {
            let response = self
                .post(json!({"jsonrpc": "2.0", "id": "test", "method": method, "params": params}))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()["content-type"], "application/json");
            response.json().await.unwrap()
        }
    }

    #[tokio::test]
    async fn initialize_list_call_and_image_results_over_http() {
        let server = TestServer::start().await;
        for requested in ["2025-11-25", "2025-06-18", "2025-03-26", "unknown"] {
            let response = server.rpc("initialize", json!({"protocolVersion": requested, "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}})).await;
            assert_eq!(
                response["result"]["protocolVersion"],
                if requested == "unknown" {
                    VERSIONS[0]
                } else {
                    requested
                }
            );
            assert_eq!(
                response["result"]["capabilities"]["tools"]["listChanged"],
                false
            );
            assert_eq!(response["id"], "test");
        }
        let initialized = server
            .post(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
            .send()
            .await
            .unwrap();
        assert_eq!(initialized.status(), StatusCode::ACCEPTED);
        assert!(initialized.bytes().await.unwrap().is_empty());
        assert_eq!(
            server.rpc("tools/list", json!({})).await["result"]["tools"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        let result = server
            .rpc(
                "tools/call",
                json!({"name": "echo", "arguments": {"count": 2}}),
            )
            .await;
        assert_eq!(result["result"]["structuredContent"]["count"], 2);
        assert_eq!(result["result"]["isError"], false);
        let failed = server.rpc("tools/call", json!({"name": "fail"})).await;
        assert_eq!(failed["result"]["isError"], true);
        let frame = server.rpc("tools/call", json!({"name": "frames"})).await;
        assert_eq!(frame["result"]["content"][1]["type"], "image");
        assert_eq!(frame["result"]["content"][1]["mimeType"], "image/jpeg");
        assert!(!frame["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("frame-bytes"));
    }

    #[tokio::test]
    async fn invalid_calls_never_reach_the_backend() {
        let server = TestServer::start().await;
        for arguments in [
            json!({}),
            json!({"count": "2"}),
            json!({"count": 0}),
            json!({"count": 11}),
            json!({"count": 2, "unknown": true}),
            json!([]),
        ] {
            let response = server
                .rpc(
                    "tools/call",
                    json!({"name": "echo", "arguments": arguments}),
                )
                .await;
            assert_eq!(response["error"]["code"], -32602);
        }
        assert_eq!(
            server.rpc("tools/call", json!({"name": "unknown"})).await["error"]["code"],
            -32602
        );
        assert_eq!(
            server.rpc("not_a_method", json!({})).await["error"]["code"],
            -32601
        );
        let missing_id = server.post(json!({"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "echo", "arguments": {"count": 2}}})).send().await.unwrap();
        assert_eq!(missing_id.status(), StatusCode::BAD_REQUEST);
        assert_eq!(server.backend.0.load(Ordering::Relaxed), 0);
        let invalid_json = server
            .client
            .post(&server.url)
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .body("{broken")
            .send()
            .await
            .unwrap();
        assert_eq!(
            invalid_json.json::<Value>().await.unwrap()["error"]["code"],
            -32700
        );
        let batch = server
            .post(json!([{"jsonrpc": "2.0", "id": 1, "method": "ping"}]))
            .send()
            .await
            .unwrap();
        assert_eq!(batch.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn origin_version_and_disabled_gate_apply_to_all_methods() {
        let server = TestServer::start().await;
        let ping = json!({"jsonrpc": "2.0", "id": 1, "method": "ping"});
        for origin in [
            "https://evil.example",
            "null",
            "http://localhost.evil.example",
            "http://evil.example@localhost",
        ] {
            assert_eq!(
                server
                    .post(ping.clone())
                    .header("origin", origin)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::FORBIDDEN
            );
        }
        assert_eq!(
            server
                .post(ping.clone())
                .header("origin", "http://localhost:6274")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            server
                .post(ping.clone())
                .header("host", "evil.example:3001")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            server
                .post(ping.clone())
                .header("mcp-protocol-version", "invalid")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            server
                .client
                .post(&server.url)
                .header("accept", "text/event-stream")
                .json(&ping)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_ACCEPTABLE
        );
        assert_eq!(
            server
                .client
                .get(&server.url)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
        assert_eq!(
            server
                .client
                .delete(&server.url)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
        server.active.store(false, Ordering::Release);
        let stopped = server.post(ping).send().await.unwrap();
        assert_eq!(stopped.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(stopped.headers()[header::CONNECTION], "close");
    }

    #[test]
    fn nested_argument_limits_are_enforced() {
        let schema = json!({"type": "object", "properties": {"ranges": {"type": "array", "minItems": 1, "maxItems": 2, "items": {"type": "object", "properties": {"start": {"type": "number", "minimum": 0}}, "required": ["start"], "additionalProperties": false}}}});
        assert!(validate_arguments(&schema, &json!({"ranges": [{"start": 0}]}), "args").is_ok());
        for invalid in [
            json!({"ranges": []}),
            json!({"ranges": [{"start": -1}]}),
            json!({"ranges": [{"start": 0}, {"start": 1}, {"start": 2}]}),
            json!({"ranges": [{"other": 1}]}),
        ] {
            assert!(validate_arguments(&schema, &invalid, "args").is_err());
        }
    }

    #[test]
    fn string_schema_limits_count_unicode_characters() {
        let schema = json!({"type": "string", "minLength": 2, "maxLength": 3});
        assert!(validate_arguments(&schema, &json!("é漢🙂"), "text").is_ok());
        assert!(validate_arguments(&schema, &json!("漢"), "text").is_err());
        assert!(validate_arguments(&schema, &json!("é漢🙂x"), "text").is_err());
    }
}
