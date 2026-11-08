use super::*;
use std::{
    collections::BTreeMap,
    io,
    sync::{atomic::AtomicUsize, Mutex},
};

#[derive(Default)]
struct MemoryStore {
    objects: Mutex<BTreeMap<String, Vec<u8>>>,
    writes: Mutex<Vec<String>>,
    reads: AtomicUsize,
    fail_key: Option<&'static str>,
    fail_read: bool,
    panic_write: bool,
    limit: Option<usize>,
    block_write: bool,
    entered: AtomicBool,
    exited: AtomicBool,
}

impl HlsObjectStore for MemoryStore {
    fn read(&self, key: &str, _: &AtomicBool) -> io::Result<Vec<u8>> {
        assert_eq!(key, "playlist.m3u8");
        self.reads.fetch_add(1, Ordering::Relaxed);
        if self.fail_read {
            return Err(io::Error::other("secret backend credential"));
        }
        self.objects
            .lock()
            .unwrap()
            .get(key)
            .cloned()
            .ok_or_else(|| io::ErrorKind::NotFound.into())
    }

    fn write(&self, key: &str, bytes: &[u8], cancelled: &AtomicBool) -> io::Result<()> {
        assert!(!cancelled.load(Ordering::Acquire));
        self.entered.store(true, Ordering::Release);
        if self.block_write {
            while !cancelled.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(5));
            }
            self.exited.store(true, Ordering::Release);
            return Err(io::ErrorKind::Interrupted.into());
        }
        assert!(!self.panic_write, "intentional store panic");
        if self.fail_key == Some(key) {
            return Err(io::Error::other("secret backend credential"));
        }
        let mut objects = self.objects.lock().unwrap();
        if key == "playlist.m3u8" {
            for key in segment_keys(bytes) {
                assert!(
                    objects.contains_key(&key),
                    "manifest published before {key}"
                );
            }
        } else {
            assert!(!objects.contains_key(key), "overwrote existing {key}");
        }
        self.writes.lock().unwrap().push(key.to_owned());
        objects.insert(key.to_owned(), bytes.to_vec());
        Ok(())
    }

    fn max_object_bytes(&self) -> usize {
        self.limit.unwrap_or(64 * 1024 * 1024)
    }
}

fn segment_keys(bytes: &[u8]) -> Vec<String> {
    std::str::from_utf8(bytes)
        .unwrap()
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

async fn record_objects(store: Arc<MemoryStore>) -> Result<(), String> {
    record_to_hls_objects(
        fixture().to_str().unwrap(),
        store,
        HlsRecordingOptions::default(),
        Arc::new(AtomicBool::new(true)),
    )
    .await
}

// Demux test output directly from memory: neither recording nor verification
// creates an output directory or a local media file.
fn memory_packets(bytes: &[u8]) -> Vec<codec::Id> {
    unsafe extern "C" fn read(opaque: *mut std::ffi::c_void, buffer: *mut u8, size: i32) -> i32 {
        let bytes = &mut *opaque.cast::<&[u8]>();
        let length = bytes.len().min(size as usize);
        if length == 0 {
            return ffi::AVERROR_EOF;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), buffer, length);
        *bytes = &bytes[length..];
        length as i32
    }
    unsafe {
        let mut bytes = bytes;
        let buffer = ffi::av_malloc(32768).cast::<u8>();
        assert!(!buffer.is_null());
        let mut io = ffi::avio_alloc_context(
            buffer,
            32768,
            0,
            (&mut bytes as *mut &[u8]).cast(),
            Some(read),
            None,
            None,
        );
        assert!(!io.is_null());
        let mut context = ffi::avformat_alloc_context();
        (*context).pb = io;
        (*context).flags |= ffi::AVFMT_FLAG_CUSTOM_IO;
        assert_eq!(
            ffi::avformat_open_input(
                &mut context,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null_mut(),
            ),
            0
        );
        assert!(ffi::avformat_find_stream_info(context, std::ptr::null_mut()) >= 0);
        let mut input = format::context::Input::wrap(context);
        let mut codecs = Vec::new();
        loop {
            let mut packet = Packet::empty();
            match packet.read(&mut input) {
                Ok(()) => codecs.push(input.stream(packet.stream()).unwrap().parameters().id()),
                Err(ffmpeg::Error::Eof) => break,
                Err(error) => panic!("invalid MPEG-TS: {error}"),
            }
        }
        drop(input);
        ffi::av_free((*io).buffer.cast());
        ffi::avio_context_free(&mut io);
        codecs
    }
}

#[tokio::test]
async fn native_objects_mux_and_resume_without_any_output_directory() {
    let store = Arc::new(MemoryStore::default());
    record_objects(store.clone()).await.unwrap();
    let before = store.objects.lock().unwrap().clone();
    let manifest = &before["playlist.m3u8"];
    let text = std::str::from_utf8(manifest).unwrap();
    assert!(text.contains("#EXT-X-PLAYLIST-TYPE:EVENT"));
    assert!(!text.contains("#EXT-X-ENDLIST"));
    assert!(!text.contains("hls-object:"));
    assert!(text.contains("#EXTINF:4.000000,"));
    let keys = segment_keys(manifest);
    assert_eq!(keys, ["0.ts", "1.ts"]);
    let codecs: Vec<_> = keys
        .iter()
        .flat_map(|key| {
            let bytes = &before[key];
            assert_eq!(bytes.len() % 188, 0);
            assert!(bytes.chunks_exact(188).all(|packet| packet[0] == 0x47));
            memory_packets(bytes)
        })
        .collect();
    assert_eq!(
        codecs.iter().filter(|id| **id == codec::Id::H264).count(),
        150
    );
    assert!(codecs.iter().filter(|id| **id == codec::Id::AAC).count() >= 281);
    record_objects(store.clone()).await.unwrap();
    let after = store.objects.lock().unwrap();
    assert_eq!(
        segment_keys(&after["playlist.m3u8"]),
        ["0.ts", "1.ts", "2.ts", "3.ts"]
    );
    assert!(std::str::from_utf8(&after["playlist.m3u8"])
        .unwrap()
        .contains("#EXT-X-DISCONTINUITY"));
    for key in keys {
        assert_eq!(after[&key], before[&key]);
    }
    assert_eq!(store.reads.load(Ordering::Relaxed), 2);
}

#[tokio::test]
async fn failed_segments_never_publish_a_referencing_manifest() {
    for key in ["0.ts", "1.ts"] {
        let store = Arc::new(MemoryStore {
            fail_key: Some(key),
            ..Default::default()
        });
        let error = record_objects(store.clone()).await.unwrap_err();
        assert!(
            error.starts_with("HLS segment object write failed"),
            "{error}"
        );
        assert!(!error.contains("secret"));
        let objects = store.objects.lock().unwrap();
        assert!(!objects.contains_key(key));
        if key == "0.ts" {
            assert!(objects.is_empty());
        } else {
            assert_eq!(segment_keys(&objects["playlist.m3u8"]), ["0.ts"]);
        }
    }
}

#[tokio::test]
async fn playlist_put_failure_is_reported_even_when_ffmpeg_ignores_close_errors() {
    let store = Arc::new(MemoryStore {
        fail_key: Some("playlist.m3u8"),
        ..Default::default()
    });
    assert!(record_objects(store.clone())
        .await
        .unwrap_err()
        .starts_with("HLS playlist object write failed"));
    let objects = store.objects.lock().unwrap();
    assert!(objects.contains_key("0.ts"));
    assert!(!objects.contains_key("playlist.m3u8"));
    assert!(!objects.contains_key("1.ts"));
}

#[tokio::test]
async fn failed_or_invalid_resume_cannot_restart_numbering() {
    let store = Arc::new(MemoryStore {
        fail_read: true,
        ..Default::default()
    });
    assert!(record_objects(store.clone())
        .await
        .unwrap_err()
        .starts_with("HLS playlist object read failed"));
    assert!(store.writes.lock().unwrap().is_empty());
    for text in [
        "not a playlist",
        "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:4,\n../0.ts\n",
        "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:4,\nhttps://secret/0.ts\n",
        "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:4,\n4.ts\n",
        "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-KEY:METHOD=AES-128,URI=\"secret\"\n",
    ] {
        let store = Arc::new(MemoryStore::default());
        store
            .objects
            .lock()
            .unwrap()
            .insert("playlist.m3u8".into(), text.as_bytes().to_vec());
        assert!(record_objects(store.clone()).await.is_err());
        assert!(store.writes.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn buffer_limits_and_backend_panics_fail_without_publishing() {
    let store = Arc::new(MemoryStore {
        limit: Some(8192),
        ..Default::default()
    });
    assert!(record_objects(store.clone())
        .await
        .unwrap_err()
        .contains("limit exceeded"));
    assert!(store.objects.lock().unwrap().is_empty());
    let store = Arc::new(MemoryStore {
        panic_write: true,
        ..Default::default()
    });
    assert!(record_objects(store.clone())
        .await
        .unwrap_err()
        .starts_with("HLS object close callback panicked"));
    assert!(store.objects.lock().unwrap().is_empty());
}

#[tokio::test]
async fn cancellation_reaches_blocking_backend_and_discards_unpublished_segment() {
    let store = Arc::new(MemoryStore {
        block_write: true,
        ..Default::default()
    });
    let task = tokio::spawn(record_objects(store.clone()));
    let _guard = AbortOnDrop(task.abort_handle());
    let deadline = Instant::now() + Duration::from_secs(5);
    while !store.entered.load(Ordering::Acquire) {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    while !store.exited.load(Ordering::Acquire) {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(store.objects.lock().unwrap().is_empty());
}

#[tokio::test]
async fn graceful_stop_commits_tail_but_dropped_future_does_not() {
    for graceful in [true, false] {
        let store = Arc::new(MemoryStore::default());
        let (source, server) = stalled_source().await;
        let _server_guard = AbortOnDrop(server.abort_handle());
        let enabled = Arc::new(AtomicBool::new(true));
        let task_store = store.clone();
        let switch = enabled.clone();
        let task = tokio::spawn(async move {
            record_to_hls_objects(&source, task_store, options(None), switch).await
        });
        let _guard = AbortOnDrop(task.abort_handle());
        let deadline = Instant::now() + Duration::from_secs(5);
        while !store.objects.lock().unwrap().contains_key("playlist.m3u8") {
            assert!(!task.is_finished(), "recording failed before first segment");
            assert!(Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        if graceful {
            enabled.store(false, Ordering::Release);
            tokio::time::timeout(Duration::from_secs(5), task)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
        } else {
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
        }
        closed(server).await;
        let objects = store.objects.lock().unwrap();
        assert_eq!(
            segment_keys(&objects["playlist.m3u8"]).len(),
            if graceful { 2 } else { 1 }
        );
    }
}
