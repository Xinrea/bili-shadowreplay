use super::*;
use crate::RecordingStore;
use axum::{
    body::Bytes,
    extract::{Path as HttpPath, Query, State},
    http::{HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::any,
    Router,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

#[derive(Default)]
struct Server {
    objects: Mutex<BTreeMap<String, Vec<u8>>>,
    ranges: Mutex<Vec<Option<String>>>,
}

#[tokio::test]
async fn custom_endpoints_put_the_bucket_in_the_selected_address_style() {
    let credentials = S3Credentials {
        access_key_id: "test-id".into(),
        secret_access_key: "test-only-secret".into(),
        session_token: String::new(),
    };
    for (endpoint, path_style, host, port, path) in [
        (
            "https://s3.oss-cn-shanghai.aliyuncs.com",
            false,
            "bsr-cache.s3.oss-cn-shanghai.aliyuncs.com",
            None,
            "/app/video.ts",
        ),
        (
            "https://bsr-cache.s3.oss-cn-shanghai.aliyuncs.com/",
            false,
            "bsr-cache.s3.oss-cn-shanghai.aliyuncs.com",
            None,
            "/app/video.ts",
        ),
        (
            "https://s3.oss-cn-shanghai.aliyuncs.com",
            true,
            "s3.oss-cn-shanghai.aliyuncs.com",
            None,
            "/bsr-cache/app/video.ts",
        ),
        (
            "http://objects.example:9000",
            false,
            "bsr-cache.objects.example",
            Some(9000),
            "/app/video.ts",
        ),
        (
            "http://127.0.0.1:9000",
            true,
            "127.0.0.1",
            Some(9000),
            "/bsr-cache/app/video.ts",
        ),
        (
            "",
            false,
            "bsr-cache.s3.us-east-1.amazonaws.com",
            None,
            "/app/video.ts",
        ),
        (
            "",
            true,
            "s3.us-east-1.amazonaws.com",
            None,
            "/bsr-cache/app/video.ts",
        ),
    ] {
        let config = S3Config {
            endpoint: endpoint.into(),
            bucket: "bsr-cache".into(),
            region: "us-east-1".into(),
            prefix: "app".into(),
            path_style,
        };
        let original = config.clone();
        let backend = S3Backend::new(&config, &credentials).unwrap();
        for method in [reqwest::Method::GET, reqwest::Method::PUT] {
            let url = backend
                .store
                .signed_url(
                    method,
                    &backend.path("video.ts").unwrap(),
                    Duration::from_secs(60),
                )
                .await
                .unwrap();
            assert_eq!(url.host_str(), Some(host));
            assert_eq!(url.port(), port);
            assert_eq!(url.path(), path);
        }
        assert_eq!(
            config, original,
            "client construction must not rewrite saved configuration"
        );
        assert_eq!(
            config.clone().normalized().unwrap().endpoint,
            endpoint.trim_end_matches('/')
        );
    }
}

#[test]
fn virtual_hosted_endpoints_require_dns_and_have_one_service_identity() {
    let config = S3Config {
        endpoint: "https://s3.oss-cn-shanghai.aliyuncs.com".into(),
        bucket: "bsr-cache".into(),
        region: "cn-shanghai".into(),
        prefix: String::new(),
        path_style: false,
    };
    let qualified = S3Config {
        endpoint: "https://bsr-cache.s3.oss-cn-shanghai.aliyuncs.com/".into(),
        ..config.clone()
    };
    assert_eq!(
        config.client_endpoint().unwrap(),
        qualified.client_endpoint().unwrap()
    );
    assert_eq!(
        config.service_endpoint().unwrap(),
        qualified.service_endpoint().unwrap()
    );
    assert_eq!(
        config.service_endpoint().unwrap(),
        S3Config {
            path_style: true,
            ..config.clone()
        }
        .service_endpoint()
        .unwrap()
    );
    for endpoint in ["http://127.0.0.1:9000", "http://[::1]:9000"] {
        let mut ip = S3Config {
            endpoint: endpoint.into(),
            ..config.clone()
        };
        assert!(ip.validate().is_err());
        assert!(ip.client_endpoint().is_err());
        ip.path_style = true;
        assert!(ip.client_endpoint().is_ok());
    }
}

fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

async fn list(
    State(state): State<Arc<Server>>,
    Query(query): Query<BTreeMap<String, String>>,
) -> String {
    assert_eq!(query.get("list-type").map(String::as_str), Some("2"));
    let objects = state.objects.lock().unwrap();
    let prefix = query.get("prefix").map(String::as_str).unwrap_or("");
    let offset = query
        .get("continuation-token")
        .or_else(|| query.get("start-after"));
    let mut matches = objects
        .iter()
        .filter(|(key, _)| key.starts_with(prefix) && offset.is_none_or(|offset| *key > offset));
    // Small service pages force the SDK to follow its own continuation tokens,
    // independently of our backend's public page limit.
    let page: Vec<_> = matches.by_ref().take(2).collect();
    let more = matches.next().is_some();
    let contents: String = page.iter().map(|(key, value)| format!(
        "<Contents><Key>{}</Key><Size>{}</Size><LastModified>2025-01-01T00:00:00Z</LastModified><ETag>\"test\"</ETag></Contents>",
        xml(key), value.len(),
    )).collect();
    let token = if more {
        format!(
            "<NextContinuationToken>{}</NextContinuationToken>",
            xml(page.last().unwrap().0)
        )
    } else {
        String::new()
    };
    format!(
        "<ListBucketResult><IsTruncated>{more}</IsTruncated>{contents}{token}</ListBucketResult>"
    )
}

async fn object(
    State(state): State<Arc<Server>>,
    HttpPath(key): HttpPath<String>,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    assert!(headers["authorization"]
        .to_str()
        .unwrap()
        .starts_with("AWS4-HMAC-SHA256 Credential=test-id/"));
    if key.ends_with("/denied") {
        return StatusCode::FORBIDDEN.into_response();
    }
    let mut objects = state.objects.lock().unwrap();
    if method == Method::PUT {
        assert!(
            !headers.contains_key("if-none-match"),
            "single-writer transfers use ordinary PUT"
        );
        objects.insert(key, body.to_vec());
        return (StatusCode::OK, [("etag", "\"test\"")]).into_response();
    }
    if method == Method::DELETE {
        objects.remove(&key);
        return StatusCode::NO_CONTENT.into_response();
    }
    let Some(bytes) = objects.get(&key) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut response = Response::builder()
        .header("last-modified", "Wed, 01 Jan 2025 00:00:00 GMT")
        .header("etag", "\"test\"");
    if method == Method::HEAD {
        return response
            .header("content-length", bytes.len())
            .body(axum::body::Body::empty())
            .unwrap();
    }
    let range = headers
        .get("range")
        .map(|value| value.to_str().unwrap().to_owned());
    state.ranges.lock().unwrap().push(range.clone());
    if let Some(range) = range.filter(|_| !key.ends_with("/ignores-range")) {
        assert_eq!(headers["if-match"], "\"test\"");
        let (start, end) = range
            .strip_prefix("bytes=")
            .unwrap()
            .split_once('-')
            .unwrap();
        let start: usize = start.parse().unwrap();
        let end = end.parse::<usize>().unwrap().min(bytes.len() - 1);
        response = response.status(StatusCode::PARTIAL_CONTENT).header(
            "content-range",
            format!("bytes {start}-{end}/{}", bytes.len()),
        );
        return response.body(bytes[start..=end].to_vec().into()).unwrap();
    }
    response.body(bytes.clone().into()).unwrap()
}

struct RunningServer(tokio::task::JoinHandle<()>);

impl Drop for RunningServer {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn backend() -> (S3Backend, Arc<Server>, RunningServer) {
    let state = Arc::new(Server::default());
    let app = Router::new()
        .route("/recordings", any(list))
        .route("/recordings/*key", any(object))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let backend = S3Backend {
        transfer_client: reqwest::Client::new(),
        create_gate: tokio::sync::Mutex::new(()),
        store: AmazonS3Builder::new()
            .with_bucket_name("recordings")
            .with_region("us-east-1")
            .with_endpoint(endpoint)
            .with_allow_http(true)
            .with_access_key_id("test-id")
            .with_secret_access_key("test-only-not-a-real-secret")
            .with_retry(object_store::RetryConfig {
                max_retries: 0,
                ..Default::default()
            })
            .build()
            .unwrap(),
        prefix: "app".into(),
    };
    (backend, state, RunningServer(task))
}

#[tokio::test]
async fn transfer_create_checks_existing_targets_without_conditional_puts() {
    let (backend, _, _task) = backend().await;
    let (a, b) = tokio::join!(
        backend.create("new", b"first"),
        backend.create("new", b"second")
    );
    assert!(a.is_ok() ^ b.is_ok());
    let error = a.err().or_else(|| b.err()).unwrap();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    let saved = backend.read("new").await.unwrap();
    assert!(saved == b"first" || saved == b"second");
    assert_eq!(
        backend.create("denied", b"x").await.unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
}

#[tokio::test]
async fn signed_object_io_preserves_ranges_limits_and_errors() {
    let (backend, server, _task) = backend().await;
    let key = "房间/录像 100%+&.ts";
    backend.write(key, b"abcdef").await.unwrap();
    assert_eq!(
        server
            .objects
            .lock()
            .unwrap()
            .get(&format!("app/{key}"))
            .unwrap(),
        b"abcdef"
    );
    assert_eq!(backend.stat(key).await.unwrap().size, 6);
    assert_eq!(backend.read_bounded(key, 6).await.unwrap(), b"abcdef");
    assert_eq!(
        backend.read_bounded(key, 5).await.unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(backend.read_range(key, 2..100).await.unwrap(), b"cdef");
    assert_eq!(
        server.ranges.lock().unwrap().last().unwrap().as_deref(),
        Some("bytes=2-5")
    );
    let gets = server.ranges.lock().unwrap().len();
    assert!(backend.read_range(key, 6..99).await.unwrap().is_empty());
    assert!(backend.read_range(key, 2..2).await.unwrap().is_empty());
    assert_eq!(
        server.ranges.lock().unwrap().len(),
        gets,
        "empty ranges must not issue GET"
    );
    assert_eq!(
        backend
            .read_range(key, Range { start: 2, end: 1 })
            .await
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidInput
    );
    backend.write("room/empty", b"").await.unwrap();
    assert!(backend
        .read_bounded("room/empty", 0)
        .await
        .unwrap()
        .is_empty());
    assert!(backend
        .read_range("room/empty", 0..100)
        .await
        .unwrap()
        .is_empty());
    backend
        .write("room/ignores-range", b"abcdef")
        .await
        .unwrap();
    assert!(backend
        .read_range("room/ignores-range", 2..4)
        .await
        .is_err());
    assert_eq!(
        backend.read("room/denied").await.unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    assert_eq!(
        backend.read("room/missing").await.unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
    assert_eq!(
        backend
            .read_range("room/missing", 0..0)
            .await
            .unwrap_err()
            .kind(),
        io::ErrorKind::NotFound
    );
    backend.write(key, b"new").await.unwrap();
    assert_eq!(backend.read(key).await.unwrap(), b"new");
    backend.remove(key).await.unwrap();
    backend.remove(key).await.unwrap();
    assert_eq!(
        backend.stat(key).await.unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
    assert!(backend.write("../outside", b"bad").await.is_err());
}

#[tokio::test]
async fn listing_logs_materialization_and_deletion_stay_in_the_namespace() {
    let (backend, server, _task) = backend().await;
    for key in [
        "room",
        "room/a",
        "room/b",
        "room/c",
        "room/d",
        "room10/keep",
    ] {
        backend.write(key, key.as_bytes()).await.unwrap();
    }
    server
        .objects
        .lock()
        .unwrap()
        .insert("app-other/keep".into(), b"outside".to_vec());
    let first = backend.list("room", None, 3).await.unwrap();
    assert_eq!(first.keys, ["room", "room/a", "room/b"]);
    let next = backend
        .list("room", first.next_cursor.as_deref(), 3)
        .await
        .unwrap();
    assert_eq!(next.keys, ["room/c", "room/d"]);
    assert_eq!(next.next_cursor, None);
    assert!(backend
        .list("missing", None, 2)
        .await
        .unwrap()
        .keys
        .is_empty());
    assert!(backend.list("room", None, 0).await.is_err());
    assert!(backend.list("room", Some("room10/keep"), 2).await.is_err());
    // A recording namespace itself is not an asset key. Remove the exact
    // prefix object used above to exercise the backend-level subtree contract.
    backend.remove("room").await.unwrap();
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("no-local-replica");
    let store = RecordingStore::with_backend(Arc::new(backend), &workspace);
    let storage = store.scope("room").unwrap();
    let log = storage.open_log("events.jsonl").await.unwrap();
    log.append(b"one\n").await.unwrap();
    drop(log);
    store
        .scope("room")
        .unwrap()
        .open_log("events.jsonl")
        .await
        .unwrap()
        .append(b"two\n")
        .await
        .unwrap();
    assert_eq!(
        storage.read_log("events.jsonl").await.unwrap(),
        b"one\ntwo\n"
    );
    let local = storage.materialize(&["a".into()]).await.unwrap();
    assert!(!workspace.exists());
    drop(local);
    storage.remove_all().await.unwrap();
    storage.remove_all().await.unwrap();
    let keys: Vec<_> = server.objects.lock().unwrap().keys().cloned().collect();
    assert_eq!(keys, ["app-other/keep", "app/room10/keep"]);
}
