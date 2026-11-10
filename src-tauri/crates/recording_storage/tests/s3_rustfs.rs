//! Opt-in live S3 test. See docs/development/backend/recording-storage.md.
use recording_storage::{RecordingStore, S3Backend, S3Config, S3Credentials, StorageBackend};
use std::{io, sync::Arc};

#[tokio::test]
#[ignore = "requires a running RustFS/S3 service, existing bucket and environment credentials"]
async fn rustfs_recording_lifecycle() {
    let config = S3Config {
        endpoint: std::env::var("S3_TEST_ENDPOINT").expect("set S3_TEST_ENDPOINT"),
        bucket: std::env::var("S3_TEST_BUCKET").expect("set S3_TEST_BUCKET"),
        region: std::env::var("S3_TEST_REGION").unwrap_or_else(|_| "us-east-1".into()),
        prefix: format!(
            "shadowreplay-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ),
        path_style: true,
    };
    // Test harness input only. The backend itself never reads environment credentials.
    let credentials = S3Credentials {
        access_key_id: std::env::var("S3_TEST_ACCESS_KEY_ID").expect("set S3_TEST_ACCESS_KEY_ID"),
        secret_access_key: std::env::var("S3_TEST_SECRET_ACCESS_KEY")
            .expect("set S3_TEST_SECRET_ACCESS_KEY"),
        session_token: String::new(),
    };
    // Only write/delete inside this unique prefix, never the user's bucket root.
    eprintln!(
        "Testing bucket {} under prefix {}",
        config.bucket, config.prefix
    );
    let backend = Arc::new(S3Backend::new(&config, &credentials).unwrap());
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("must-not-exist");
    let store = RecordingStore::with_backend(backend.clone(), &workspace);
    let storage = store.recording("bilibili", "123", "live").unwrap();
    storage.prepare().await.unwrap();
    storage
        .write("录像 100%+&.ts", b"0123456789")
        .await
        .unwrap();
    assert_eq!(
        storage.read_range("录像 100%+&.ts", 3..7).await.unwrap(),
        b"3456"
    );
    assert!(storage
        .read_range("录像 100%+&.ts", 10..20)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        storage
            .read_bounded("录像 100%+&.ts", 9)
            .await
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
    storage.write("playlist.m3u8", b"#EXTM3U\n").await.unwrap();
    storage
        .write("playlist.m3u8", b"#EXTM3U\n#EXT-X-ENDLIST\n")
        .await
        .unwrap();
    for index in 0..7 {
        storage
            .write(&format!("segment-{index}.ts"), b"segment")
            .await
            .unwrap();
    }
    let mut cursor = None;
    let mut keys = Vec::new();
    loop {
        let page = storage.list("", cursor.as_deref(), 2).await.unwrap();
        assert!(page.keys.len() <= 2);
        keys.extend(page.keys);
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(keys.len(), 9);
    assert!(keys.windows(2).all(|pair| pair[0] < pair[1]));
    storage
        .open_log("events.jsonl")
        .await
        .unwrap()
        .append(b"first\n")
        .await
        .unwrap();
    let resumed = RecordingStore::with_backend(
        Arc::new(S3Backend::new(&config, &credentials).unwrap()),
        &workspace,
    )
    .recording("bilibili", "123", "live")
    .unwrap();
    resumed
        .open_log("events.jsonl")
        .await
        .unwrap()
        .append(b"second\n")
        .await
        .unwrap();
    assert_eq!(
        resumed.read_log("events.jsonl").await.unwrap(),
        b"first\nsecond\n"
    );
    let _local = storage
        .materialize(&["playlist.m3u8".into()])
        .await
        .unwrap();
    assert!(!workspace.exists());
    let sibling = store.recording("bilibili", "123", "live2").unwrap();
    sibling.write("keep", b"keep").await.unwrap();
    storage.remove_all().await.unwrap();
    storage.remove_all().await.unwrap();
    assert_eq!(sibling.read("keep").await.unwrap(), b"keep");
    assert_eq!(
        storage.read("playlist.m3u8").await.unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
    sibling.remove_all().await.unwrap();
    assert!(backend.list("", None, 1).await.unwrap().keys.is_empty());
}
