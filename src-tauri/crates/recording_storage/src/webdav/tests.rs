use super::*;
use crate::{ObjectTransfer, RecordingStore};
use std::sync::{atomic::Ordering, Arc};
#[path = "../../tests/support/webdav.rs"]
mod fixture;

fn backend(endpoint: &str, prefix: &str) -> WebDavBackend {
    WebDavBackend::new(
        &WebDavConfig {
            endpoint: endpoint.into(),
            prefix: prefix.into(),
        },
        &WebDavCredentials {
            username: "user".into(),
            password: "pass".into(),
        },
    )
    .unwrap()
}
#[test]
fn validates_and_canonicalizes_namespaces_and_redacts_secrets() {
    for endpoint in [
        "ftp://host/dav",
        "https://user:pass@host/dav",
        "https://host/dav?q=x",
        "https://host/dav#x",
        "https://host/dav/%2fescaped",
    ] {
        assert!(WebDavConfig {
            endpoint: endpoint.into(),
            prefix: "".into()
        }
        .normalized()
        .is_err());
    }
    let a = WebDavConfig {
        endpoint: "https://HOST:443/dav/%61pp".into(),
        prefix: "中文".into(),
    };
    let b = WebDavConfig {
        endpoint: "https://host/dav/app/".into(),
        prefix: "中文/nested".into(),
    };
    assert!(a.overlaps(&b).unwrap());
    assert!(!a
        .overlaps(&WebDavConfig {
            prefix: "中文2".into(),
            ..b
        })
        .unwrap());
    assert!(backend("https://host/dav/", "").url("../escape").is_err());
    assert_eq!(
        format!(
            "{:?}",
            WebDavCredentials {
                username: "secret-user".into(),
                password: "secret-pass".into()
            }
        ),
        "WebDavCredentials([redacted])"
    );
}
#[tokio::test]
async fn objects_ranges_atomic_publication_and_errors() {
    let f = fixture::Fixture::new().await;
    let b = backend(&f.endpoint, "app");
    b.write("room/video.ts", b"0123456789").await.unwrap();
    assert_eq!(b.stat("room/video.ts").await.unwrap().size, 10);
    assert_eq!(b.read_range("room/video.ts", 2..6).await.unwrap(), b"2345");
    assert_eq!(b.read_range("room/video.ts", 8..20).await.unwrap(), b"89");
    assert!(b
        .read_range("room/video.ts", 20..30)
        .await
        .unwrap()
        .is_empty());
    assert!(b
        .read_range("room/video.ts", Range { start: 5, end: 4 })
        .await
        .is_err());
    assert_eq!(
        b.read_bounded("room/video.ts", 9).await.unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    f.server.chunked.store(true, Ordering::SeqCst);
    assert_eq!(
        b.read_bounded("room/video.ts", 4).await.unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    f.server.ignore_range.store(true, Ordering::SeqCst);
    assert!(b.read_range("room/video.ts", 2..4).await.is_err());
    f.server.ignore_range.store(false, Ordering::SeqCst);
    f.server.wrong_range.store(true, Ordering::SeqCst);
    assert!(b.read_range("room/video.ts", 2..4).await.is_err());
    assert_eq!(
        b.create("room/video.ts", b"overwrite")
            .await
            .unwrap_err()
            .kind(),
        io::ErrorKind::AlreadyExists
    );
    f.server.fail_move.store(true, Ordering::SeqCst);
    assert!(b.write("room/video.ts", b"replacement").await.is_err());
    assert_eq!(b.read("room/video.ts").await.unwrap(), b"0123456789");
    assert!(!f
        .server
        .files
        .lock()
        .unwrap()
        .keys()
        .any(|s| s.contains(UPLOAD_PREFIX)));
    assert!(b.remove("room").await.is_err());
    b.remove("room/video.ts").await.unwrap();
    b.remove("room/video.ts").await.unwrap();
    assert_eq!(
        b.read("missing").await.unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
    let bad = WebDavBackend::new(
        &WebDavConfig {
            endpoint: f.endpoint.clone(),
            prefix: "".into(),
        },
        &WebDavCredentials {
            username: "wrong".into(),
            password: "secret".into(),
        },
    )
    .unwrap();
    let error = bad.read("file").await.unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert!(!error.to_string().contains("secret"));
}
#[tokio::test]
async fn listing_logs_and_deletion_respect_subtrees_and_encoded_names() {
    let f = fixture::Fixture::new().await;
    let b = Arc::new(backend(&f.endpoint, "app"));
    for name in [
        "room1/a.ts",
        "room1/nested/中 文%&+#.ts",
        "room1/z.ts",
        "room10/sibling.ts",
    ] {
        b.write(name, b"video").await.unwrap();
    }
    let first = b.list("room1", None, 2).await.unwrap();
    assert_eq!(first.keys, ["room1/a.ts", "room1/nested/中 文%&+#.ts"]);
    let second = b
        .list("room1", first.next_cursor.as_deref(), 2)
        .await
        .unwrap();
    assert_eq!(second.keys, ["room1/z.ts"]);
    assert!(second.next_cursor.is_none());
    assert_eq!(
        b.list("room1/a.ts", None, 2).await.unwrap().keys,
        ["room1/a.ts"]
    );
    assert!(b.list("missing", None, 2).await.unwrap().keys.is_empty());
    let store = RecordingStore::with_backend(b.clone(), std::env::temp_dir().join("unused-webdav"));
    let recording = store.scope("room1").unwrap();
    let log = recording.open_log("events.jsonl").await.unwrap();
    log.append(b"hello\n").await.unwrap();
    log.append(b"world\n").await.unwrap();
    assert_eq!(
        recording.read_log("events.jsonl").await.unwrap(),
        b"hello\nworld\n"
    );
    recording.remove_all().await.unwrap();
    assert!(b.list("room1", None, 10).await.unwrap().keys.is_empty());
    assert_eq!(b.read("room10/sibling.ts").await.unwrap(), b"video");
}
#[tokio::test]
async fn transfers_round_trip_with_streamed_files_and_no_clobber() {
    let f = fixture::Fixture::new().await;
    let local = tempfile::tempdir().unwrap();
    let source = RecordingStore::local(local.path().canonicalize().unwrap())
        .scope("recording")
        .unwrap();
    source.write("a.ts", b"media").await.unwrap();
    source.write("b.ts", b"second").await.unwrap();
    source
        .open_log("events.jsonl")
        .await
        .unwrap()
        .append(b"event\n")
        .await
        .unwrap();
    let remote = RecordingStore::with_backend(
        Arc::new(backend(&f.endpoint, "app")),
        local.path().join("unused"),
    );
    let target = remote.scope("recording").unwrap();
    let transfer = ObjectTransfer::new(
        source.clone(),
        target.clone(),
        "recording".into(),
        "local".into(),
        "webdav".into(),
    )
    .unwrap();
    let manifest = transfer.inspect().await.unwrap();
    transfer.copy("token", &manifest).await.unwrap();
    transfer.copy("token", &manifest).await.unwrap();
    transfer.cleanup_source("token", &manifest).await.unwrap();
    transfer.finish("token", &manifest).await.unwrap();
    assert_eq!(target.read("b.ts").await.unwrap(), b"second");
    assert_eq!(target.read_log("events.jsonl").await.unwrap(), b"event\n");
    let back = ObjectTransfer::new(
        target,
        source.clone(),
        "recording".into(),
        "webdav".into(),
        "local".into(),
    )
    .unwrap();
    let manifest = back.inspect().await.unwrap();
    back.copy("return", &manifest).await.unwrap();
    back.cleanup_source("return", &manifest).await.unwrap();
    back.finish("return", &manifest).await.unwrap();
    assert_eq!(source.read("a.ts").await.unwrap(), b"media");
}
#[test]
fn malformed_or_escaped_multistatus_fails_closed() {
    let url = Url::parse("https://host/dav/").unwrap();
    for xml in ["<bad/>", "<multistatus xmlns='DAV:'></multistatus>", "<multistatus xmlns='DAV:'><response><href>https://other/dav/file</href></response></multistatus>", "<multistatus xmlns='DAV:'><response><href>/dav/file</href><status>HTTP/1.1 403 Forbidden</status></response></multistatus>", "<!DOCTYPE x [<!ENTITY x SYSTEM 'file:///etc/passwd'>]><multistatus xmlns='DAV:'>&x;</multistatus>"] {
        assert!(parse_properties(xml.as_bytes(), &url).is_err());
    }
}
