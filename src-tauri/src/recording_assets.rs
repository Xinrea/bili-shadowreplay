//! Shared public cache serving for GUI and headless mode.
//!
//! Recording URLs resolve through the archive registry, never the session
//! directory. Only explicit public namespaces are served from session temp.
use std::{io, ops::Range, str::FromStr, sync::Arc};

use axum::{
    body::Body,
    extract::{Path, Request, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use recorder::platforms::PlatformType;
use recording_storage::{validate_key, RecordingStorage};
use tower_http::services::ServeDir;

use crate::recorder_manager::RecorderManager;

#[derive(Clone)]
struct CacheState {
    manager: Arc<RecorderManager>,
    temp_storage: crate::temp_storage::TempStorage,
}

pub fn cache_router(
    manager: Arc<RecorderManager>,
    temp_storage: crate::temp_storage::TempStorage,
) -> Router {
    Router::new()
        .route("/*path", get(serve_cache))
        .with_state(CacheState {
            manager,
            temp_storage,
        })
}

#[derive(Debug, PartialEq)]
enum AssetPath<'a> {
    Local,
    RoomCover {
        platform: PlatformType,
        room: &'a str,
    },
    Recording {
        platform: PlatformType,
        room: &'a str,
        live: &'a str,
        key: &'a str,
    },
}

fn asset_path(path: &str) -> Result<AssetPath<'_>, StatusCode> {
    validate_key(path).map_err(|_| StatusCode::BAD_REQUEST)?;
    if path.split('/').any(|part| part.starts_with('.')) {
        return Err(StatusCode::NOT_FOUND);
    }
    let mut parts = path.splitn(4, '/');
    let namespace = parts.next().unwrap_or("");
    let platform = match PlatformType::from_str(namespace) {
        Ok(platform) => platform,
        // Do not let case-insensitive local filesystems turn noncanonical
        // recording namespaces into an obsolete-global-cache fallback.
        Err(_) if PlatformType::from_str(&namespace.to_ascii_lowercase()).is_ok() => {
            return Err(StatusCode::NOT_FOUND);
        }
        Err(_) if crate::temp_storage::public_namespace(namespace) => {
            return Ok(AssetPath::Local);
        }
        Err(_) => return Err(StatusCode::NOT_FOUND),
    };
    // Once the platform is recognized, malformed paths and missing assets must
    // never fall through to ServeDir and leak an obsolete global-cache copy.
    let room = parts.next().ok_or(StatusCode::NOT_FOUND)?;
    let live = parts.next().ok_or(StatusCode::NOT_FOUND)?;
    match parts.next() {
        None if live == "cover.jpg" => Ok(AssetPath::RoomCover { platform, room }),
        Some(key) if public_asset(key) => Ok(AssetPath::Recording {
            platform,
            room,
            live,
            key,
        }),
        _ => Err(StatusCode::NOT_FOUND),
    }
}

fn public_asset(key: &str) -> bool {
    // Physical append-log chunks and checkpoints are implementation details.
    // Keep legacy logical logs/subtitles available alongside public media.
    !key.split('/')
        .any(|part| part.starts_with('.') || part == "chunks")
        && (logical_log(key)
            || matches!(
                key.rsplit('.').next(),
                Some(
                    "m3u8"
                        | "ts"
                        | "m4s"
                        | "mp4"
                        | "flv"
                        | "webm"
                        | "aac"
                        | "mp3"
                        | "wav"
                        | "jpg"
                        | "jpeg"
                        | "png"
                        | "webp"
                        | "srt"
                        | "vtt"
                )
            ))
}

fn logical_log(key: &str) -> bool {
    matches!(
        key,
        "entries.log" | "events.jsonl" | "skipped_ad_ranges.jsonl" | "danmu.txt"
    )
}

async fn serve_cache(
    State(state): State<CacheState>,
    Path(path): Path<String>,
    request: Request,
) -> Response {
    let asset = match asset_path(&path) {
        Ok(asset) => asset,
        Err(status) => return status.into_response(),
    };
    let (storage, key) = match asset {
        AssetPath::Local => {
            return match ServeDir::new(state.temp_storage.root())
                .try_call(request)
                .await
            {
                Ok(response) => response.map(Body::new),
                Err(error) => storage_error(error),
            };
        }
        AssetPath::RoomCover { platform, room } => {
            let storage = match state
                .manager
                .archive_storage()
                .room(platform.as_str(), room)
                .await
            {
                Ok(storage) => storage,
                Err(error) => return storage_error(error),
            };
            (storage, "cover.jpg")
        }
        AssetPath::Recording {
            platform,
            room,
            live,
            key,
        } => {
            let storage = match state.manager.recording_storage(platform, room, live).await {
                Ok(storage) => storage,
                Err(error) => {
                    log::warn!("Cannot resolve recording asset {path}: {error}");
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
            };
            (storage, key)
        }
    };
    serve_asset(storage, key, request.method(), request.headers()).await
}

fn storage_error(error: io::Error) -> Response {
    let status = match error.kind() {
        io::ErrorKind::NotFound => StatusCode::NOT_FOUND,
        io::ErrorKind::InvalidInput => StatusCode::BAD_REQUEST,
        _ => {
            log::warn!("Cannot serve recording asset: {error}");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    };
    status.into_response()
}

/// Parse one byte range; suffix and open-ended ranges are supported. Reject
/// malformed, unsatisfiable and multipart requests explicitly with 416.
fn byte_range(value: &str, size: u64) -> Result<Range<u64>, ()> {
    let value = value.strip_prefix("bytes=").ok_or(())?;
    let (start, end) = value.split_once('-').ok_or(())?;
    let decimal = |value: &str| -> Result<u64, ()> {
        if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(());
        }
        value.parse().map_err(|_| ())
    };
    if size == 0 {
        return Err(());
    }
    if start.is_empty() {
        let suffix = decimal(end)?;
        if suffix == 0 {
            return Err(());
        }
        return Ok(size.saturating_sub(suffix)..size);
    }
    let start = decimal(start)?;
    let end = if end.is_empty() {
        size - 1
    } else {
        decimal(end)?.min(size - 1)
    };
    if start >= size || start > end {
        return Err(());
    }
    Ok(start..end + 1)
}

fn content_type(key: &str) -> &'static str {
    match key.rsplit('.').next().unwrap_or("") {
        "m3u8" => "application/vnd.apple.mpegurl",
        "ts" => "video/mp2t",
        "m4s" => "video/iso.segment",
        "mp4" => "video/mp4",
        "flv" => "video/x-flv",
        "webm" => "video/webm",
        "aac" => "audio/aac",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "srt" | "log" | "txt" => "text/plain; charset=utf-8",
        "vtt" => "text/vtt; charset=utf-8",
        "jsonl" => "application/x-ndjson",
        _ => "application/octet-stream",
    }
}

async fn serve_asset(
    storage: RecordingStorage,
    key: &str,
    method: &Method,
    headers: &HeaderMap,
) -> Response {
    // Playlists are rewritten atomically while recording. Read one snapshot so
    // its length and response body cannot describe different generations.
    let snapshot = if logical_log(key) || key.ends_with(".m3u8") {
        let result = if logical_log(key) {
            storage.read_log(key).await
        } else {
            storage.read(key).await
        };
        match result {
            Ok(bytes) => Some(bytes),
            Err(error) => return storage_error(error),
        }
    } else {
        None
    };
    let size = match &snapshot {
        Some(bytes) => bytes.len() as u64,
        None => match storage.size(key).await {
            Ok(size) => size,
            Err(error) => return storage_error(error),
        },
    };
    // Without a validator we cannot establish an If-Range match; send the
    // complete representation rather than mistakenly serving a stale range.
    let requested = if headers.contains_key(header::IF_RANGE) {
        None
    } else {
        headers.get(header::RANGE)
    };
    let range = match requested {
        Some(value) => {
            let range = value
                .to_str()
                .map_err(|_| ())
                .and_then(|value| byte_range(value, size));
            match range {
                Ok(range) if headers.get_all(header::RANGE).iter().count() == 1 => range,
                _ => {
                    let mut response = StatusCode::RANGE_NOT_SATISFIABLE.into_response();
                    response.headers_mut().insert(
                        header::CONTENT_RANGE,
                        HeaderValue::from_str(&format!("bytes */{size}")).unwrap(),
                    );
                    return response;
                }
            }
        }
        None => 0..size,
    };
    let length = range.end - range.start;
    let body = if *method == Method::HEAD || length == 0 {
        Body::empty()
    } else if let Some(snapshot) = snapshot {
        Body::from(snapshot[range.start as usize..range.end as usize].to_vec())
    } else {
        let key = key.to_string();
        let stream = futures::stream::try_unfold(
            (storage, key, range.clone()),
            |(storage, key, remaining)| async move {
                if remaining.is_empty() {
                    return Ok::<_, io::Error>(None);
                }
                let end = remaining
                    .start
                    .saturating_add(1024 * 1024)
                    .min(remaining.end);
                let bytes = storage.read_range(&key, remaining.start..end).await?;
                if bytes.len() as u64 != end - remaining.start {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "asset changed while serving",
                    ));
                }
                Ok(Some((bytes, (storage, key, end..remaining.end))))
            },
        );
        Body::from_stream(stream)
    };
    let mut response = Response::new(body);
    if requested.is_some() {
        *response.status_mut() = StatusCode::PARTIAL_CONTENT;
        response.headers_mut().insert(
            header::CONTENT_RANGE,
            HeaderValue::from_str(&format!("bytes {}-{}/{size}", range.start, range.end - 1))
                .unwrap(),
        );
    }
    response
        .headers_mut()
        .insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(content_type(key)),
    );
    response
        .headers_mut()
        .insert(header::CONTENT_LENGTH, HeaderValue::from(length));
    if key.ends_with(".m3u8") {
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-cache, no-store, must-revalidate"),
        );
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_namespaces_never_fall_back_to_local_files() {
        assert!(matches!(
            asset_path("uploads/source.mp4"),
            Ok(AssetPath::Local)
        ));
        assert!(matches!(
            asset_path("preview_audio/chunk.wav"),
            Ok(AssetPath::Local)
        ));
        assert!(matches!(
            asset_path("exports/42/export.mp4"),
            Ok(AssetPath::Local)
        ));
        assert!(matches!(
            asset_path("bilibili/42/cover.jpg"),
            Ok(AssetPath::RoomCover { .. })
        ));
        assert!(matches!(
            asset_path("bilibili/42/live/media/segment.ts"),
            Ok(AssetPath::Recording {
                key: "media/segment.ts",
                ..
            })
        ));
        assert!(matches!(
            asset_path("bilibili/42/live/entries.log"),
            Ok(AssetPath::Recording { .. })
        ));
        for path in [
            "bilibili",
            "BiliBili/42/live/output.mp4",
            "bilibili/42",
            "bilibili/42/live/checkpoint.json",
            "bilibili/42/live/.logs/part.ts",
            "project_segments/private.mp4",
            "unknown/file.mp4",
            "Uploads/source.mp4",
            "uploads/.upload-partial",
        ] {
            assert!(asset_path(path).is_err(), "{path}");
        }
        for path in [
            "bilibili/../live/output.mp4",
            "bilibili/42/live\\bad/output.mp4",
            "bilibili//live/output.mp4",
            "uploads/../bilibili/42/live/output.mp4",
            "uploads\\source.mp4",
        ] {
            assert_eq!(asset_path(path), Err(StatusCode::BAD_REQUEST), "{path}");
        }
    }

    #[test]
    fn single_ranges_cover_browser_seek_and_suffix_requests() {
        assert_eq!(byte_range("bytes=0-3", 10), Ok(0..4));
        assert_eq!(byte_range("bytes=7-", 10), Ok(7..10));
        assert_eq!(byte_range("bytes=-3", 10), Ok(7..10));
        assert_eq!(byte_range("bytes=-20", 10), Ok(0..10));
        assert_eq!(byte_range("bytes=1-999", 10), Ok(1..10));
        for value in [
            "bytes=10-",
            "bytes=4-2",
            "bytes=-0",
            "bytes=0-1,4-5",
            "items=0-1",
            "bytes=+1-2",
            "bytes=--1",
        ] {
            assert!(byte_range(value, 10).is_err(), "{value}");
        }
        assert!(byte_range("bytes=0-", 0).is_err());
    }

    #[tokio::test]
    async fn serves_selected_storage_with_ranges_head_and_mime() {
        let old = tempfile::tempdir().unwrap();
        let selected = tempfile::tempdir().unwrap();
        let old_store = RecordingStorage::local(old.path());
        let store = RecordingStorage::local(selected.path());
        old_store.write("output.mp4", b"old!").await.unwrap();
        store.write("output.mp4", b"selected").await.unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(header::RANGE, HeaderValue::from_static("bytes=1-3"));
        let response = serve_asset(store.clone(), "output.mp4", &Method::GET, &headers).await;
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(response.headers()[header::CONTENT_RANGE], "bytes 1-3/8");
        assert_eq!(response.headers()[header::CONTENT_TYPE], "video/mp4");
        assert_eq!(
            axum::body::to_bytes(response.into_body(), 100)
                .await
                .unwrap(),
            "ele"
        );
        let response = serve_asset(
            store.clone(),
            "output.mp4",
            &Method::HEAD,
            &HeaderMap::new(),
        )
        .await;
        assert_eq!(response.headers()[header::CONTENT_LENGTH], "8");
        assert!(axum::body::to_bytes(response.into_body(), 100)
            .await
            .unwrap()
            .is_empty());
        headers.insert(header::RANGE, HeaderValue::from_static("bytes=99-"));
        let response = serve_asset(store, "output.mp4", &Method::GET, &headers).await;
        assert_eq!(response.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(response.headers()[header::CONTENT_RANGE], "bytes */8");
    }
}
