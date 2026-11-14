//! Local HTTP protocol fixture; no external credentials or configured storage.
use axum::{
    body::Bytes,
    extract::{Request, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::any,
    Router,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

#[derive(Default)]
pub struct Server {
    pub files: Mutex<BTreeMap<String, Vec<u8>>>,
    pub dirs: Mutex<BTreeSet<String>>,
    pub fail_move: AtomicBool,
    pub ignore_range: AtomicBool,
    pub wrong_range: AtomicBool,
    pub chunked: AtomicBool,
}
pub struct Fixture {
    pub server: Arc<Server>,
    pub endpoint: String,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Fixture {
    pub async fn new() -> Self {
        let server = Arc::new(Server::default());
        server.dirs.lock().unwrap().insert("/dav".into());
        let app = Router::new()
            .fallback(any(handle))
            .with_state(server.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/dav/", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            server,
            endpoint,
            task,
        }
    }
}
fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
fn etag(bytes: &[u8]) -> String {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut hash = DefaultHasher::new();
    bytes.hash(&mut hash);
    format!("\"{:x}\"", hash.finish())
}
async fn handle(State(server): State<Arc<Server>>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    if parts
        .headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        != Some("Basic dXNlcjpwYXNz")
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let path = percent_encoding::percent_decode_str(parts.uri.path())
        .decode_utf8()
        .unwrap()
        .trim_end_matches('/')
        .to_owned();
    let method = parts.method.as_str();
    let body = if method == "PUT" {
        axum::body::to_bytes(body, 32 * 1024 * 1024)
            .await
            .unwrap()
            .to_vec()
    } else {
        Vec::new()
    };
    let mut files = server.files.lock().unwrap();
    let mut dirs = server.dirs.lock().unwrap();
    match method {
        "MKCOL" => {
            if dirs.contains(&path) || files.contains_key(&path) {
                return StatusCode::METHOD_NOT_ALLOWED.into_response();
            }
            if !dirs.contains(path.rsplit_once('/').unwrap().0) {
                return StatusCode::CONFLICT.into_response();
            }
            dirs.insert(path);
            StatusCode::CREATED.into_response()
        }
        "PUT" => {
            if !dirs.contains(path.rsplit_once('/').unwrap().0) {
                return StatusCode::CONFLICT.into_response();
            }
            if parts.headers.contains_key(header::IF_NONE_MATCH) && files.contains_key(&path) {
                return StatusCode::PRECONDITION_FAILED.into_response();
            }
            files.insert(path, body);
            StatusCode::CREATED.into_response()
        }
        "MOVE" => {
            if server.fail_move.load(Ordering::SeqCst) {
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
            let destination =
                url::Url::parse(parts.headers.get("Destination").unwrap().to_str().unwrap())
                    .unwrap();
            let target = percent_encoding::percent_decode_str(destination.path())
                .decode_utf8()
                .unwrap()
                .into_owned();
            if parts.headers.get("Overwrite").unwrap() == "F" && files.contains_key(&target) {
                return StatusCode::PRECONDITION_FAILED.into_response();
            }
            let Some(bytes) = files.remove(&path) else {
                return StatusCode::NOT_FOUND.into_response();
            };
            let existed = files.insert(target, bytes).is_some();
            if existed {
                StatusCode::NO_CONTENT
            } else {
                StatusCode::CREATED
            }
            .into_response()
        }
        "GET" | "HEAD" => {
            let Some(bytes) = files.get(&path) else {
                return StatusCode::NOT_FOUND.into_response();
            };
            let tag = etag(bytes);
            if parts
                .headers
                .get(header::IF_MATCH)
                .is_some_and(|v| v != tag.as_str())
            {
                return StatusCode::PRECONDITION_FAILED.into_response();
            }
            let mut builder = axum::http::Response::builder().header(header::ETAG, tag);
            if method == "HEAD" {
                return builder
                    .header(header::CONTENT_LENGTH, bytes.len())
                    .body(axum::body::Body::empty())
                    .unwrap();
            }
            if let Some(range) = parts
                .headers
                .get(header::RANGE)
                .filter(|_| !server.ignore_range.load(Ordering::SeqCst))
            {
                let (start, end) = range
                    .to_str()
                    .unwrap()
                    .strip_prefix("bytes=")
                    .unwrap()
                    .split_once('-')
                    .unwrap();
                let start: usize = start.parse().unwrap();
                let end: usize = end.parse().unwrap();
                if start >= bytes.len() {
                    return StatusCode::RANGE_NOT_SATISFIABLE.into_response();
                }
                let end = end.min(bytes.len() - 1);
                let reported = if server.wrong_range.load(Ordering::SeqCst) {
                    start + 1
                } else {
                    start
                };
                return builder
                    .status(StatusCode::PARTIAL_CONTENT)
                    .header(
                        header::CONTENT_RANGE,
                        format!("bytes {reported}-{end}/{}", bytes.len()),
                    )
                    .body(axum::body::Body::from(bytes[start..=end].to_vec()))
                    .unwrap();
            }
            if server.chunked.load(Ordering::SeqCst) {
                let chunks: Vec<_> = bytes
                    .chunks(3)
                    .map(|v| Ok::<_, std::io::Error>(Bytes::copy_from_slice(v)))
                    .collect();
                builder
                    .body(axum::body::Body::from_stream(futures::stream::iter(chunks)))
                    .unwrap()
            } else {
                builder = builder.header(header::CONTENT_LENGTH, bytes.len());
                builder.body(axum::body::Body::from(bytes.clone())).unwrap()
            }
        }
        "DELETE" => {
            if files.remove(&path).is_some() {
                StatusCode::NO_CONTENT.into_response()
            } else {
                StatusCode::NOT_FOUND.into_response()
            }
        }
        "PROPFIND" => {
            if !files.contains_key(&path) && !dirs.contains(&path) {
                return StatusCode::NOT_FOUND.into_response();
            }
            // Exercise the common canonical collection redirect without allowing arbitrary redirects.
            if dirs.contains(&path) && !parts.uri.path().ends_with('/') {
                return (
                    StatusCode::MOVED_PERMANENTLY,
                    [(header::LOCATION, format!("{}/", parts.uri.path()))],
                )
                    .into_response();
            }
            let children = parts.headers.get("Depth").is_some_and(|v| v == "1");
            let mut paths = vec![(path.clone(), dirs.contains(&path))];
            if children {
                for child in dirs.iter().chain(files.keys()) {
                    if child
                        .rsplit_once('/')
                        .is_some_and(|(parent, _)| parent == path)
                    {
                        paths.push((child.clone(), dirs.contains(child)));
                    }
                }
            }
            // Reverse to ensure the client does not rely on server ordering.
            paths.reverse();
            let mut xml = String::from("<d:multistatus xmlns:d=\"DAV:\">");
            for (path, collection) in paths {
                let mut url = url::Url::parse("http://fixture.invalid/").unwrap();
                url.path_segments_mut()
                    .unwrap()
                    .clear()
                    .extend(path.trim_start_matches('/').split('/'));
                if collection {
                    url.path_segments_mut().unwrap().push("");
                }
                xml.push_str(&format!("<d:response><d:href>{}</d:href><d:propstat><d:prop><d:resourcetype>{}</d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat><d:propstat><d:prop><d:displayname/></d:prop><d:status>HTTP/1.1 404 Not Found</d:status></d:propstat></d:response>", escape(url.path()), if collection { "<d:collection/>" } else { "" }));
            }
            xml.push_str("</d:multistatus>");
            (
                StatusCode::MULTI_STATUS,
                [(header::CONTENT_TYPE, "application/xml")],
                xml,
            )
                .into_response()
        }
        _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
    }
}
