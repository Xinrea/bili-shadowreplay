use super::*;
use crate::danmu::batch::QUEUED_EVENTS;
use recording_storage::{RecordingStore, LOG_CHUNK_BYTES};
use tokio::time::{advance, Instant};

fn event(ts: i64, content: &str) -> LiveEvent {
    LiveEvent {
        ts,
        platform: "bilibili".into(),
        room_id: "room".into(),
        event_type: "danmu".into(),
        data: serde_json::json!({"content": content}),
        raw: Value::Null,
    }
}

#[tokio::test(start_paused = true)]
async fn lost_head_reply_and_failed_readback_do_not_duplicate_a_batch() {
    let backend = Arc::new(HistoryBackend::new("", 2));
    let storage = backend.storage();
    let writer = DanmuWriter::from_storage(&storage, "events.jsonl")
        .await
        .unwrap();
    backend.allow_history.store(true, Ordering::Relaxed);
    backend.lose_head_reply.store(true, Ordering::Relaxed);
    for ts in 100..110 {
        writer.add_event(&event(ts, "batch")).await.unwrap();
    }
    settle().await;
    advance(Duration::from_secs(5)).await;
    settle().await;
    assert_eq!(
        backend.writes.load(Ordering::Relaxed),
        2,
        "batch committed but reply lost"
    );
    writer.add_event(&event(110, "next batch")).await.unwrap();
    advance(Duration::from_secs(5)).await;
    settle().await;
    assert_eq!(
        backend.writes.load(Ordering::Relaxed),
        2,
        "readback failure must retain original position"
    );
    advance(Duration::from_secs(5)).await;
    settle().await;
    writer.close().await.unwrap();
    assert_eq!(
        backend.writes.load(Ordering::Relaxed),
        4,
        "retry verifies rather than rewrites committed bytes"
    );
    assert_eq!(
        backend.history_reads.load(Ordering::Relaxed),
        1,
        "only the attempted batch is verified"
    );
    let entries = DanmuStorage::read_entries(&storage, "events.jsonl")
        .await
        .unwrap();
    assert_eq!(
        entries.iter().map(|e| e.ts).collect::<Vec<_>>(),
        [vec![0, 1], (100..111).collect()].concat()
    );
}

fn sized_event(bytes: usize) -> LiveEvent {
    let mut event = event(0, "");
    let padding = bytes - event_line(&event).unwrap().len();
    // A character threshold would get this wrong; measure serialized UTF-8.
    event.data["content"] = Value::String(format!(
        "{}{}",
        "弹".repeat(padding / 3),
        "x".repeat(padding % 3)
    ));
    assert_eq!(event_line(&event).unwrap().len(), bytes);
    event
}

fn chunks(backend: &HistoryBackend) -> Vec<Vec<u8>> {
    backend
        .objects
        .lock()
        .unwrap()
        .iter()
        .filter(|(key, _)| key.contains(".recording-logs/") && !key.ends_with("/head"))
        .map(|(_, bytes)| bytes.clone())
        .collect()
}

async fn settle() {
    // Let the channel worker, spawned append and virtual timer all run without
    // advancing time (tokio's auto-advance would conceal an early flush).
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
}

#[tokio::test(start_paused = true)]
async fn empty_logs_do_not_write_and_arrivals_do_not_postpone_five_second_deadline() {
    let backend = Arc::new(HistoryBackend::new("", 0));
    let writer = DanmuWriter::from_storage(&backend.storage(), "events.jsonl")
        .await
        .unwrap();
    advance(Duration::from_secs(30)).await;
    settle().await;
    assert_eq!(backend.writes.load(Ordering::Relaxed), 0);
    let first = event(1, "first");
    let second = event(2, "second");
    writer.add_event(&first).await.unwrap();
    settle().await;
    advance(Duration::from_millis(4999)).await;
    settle().await;
    assert!(chunks(&backend).is_empty());
    writer.add_event(&second).await.unwrap();
    settle().await;
    assert!(chunks(&backend).is_empty());
    advance(Duration::from_millis(1)).await;
    settle().await;
    let expected = [event_line(&first).unwrap(), event_line(&second).unwrap()].concat();
    assert_eq!(chunks(&backend), [expected]);
    assert_eq!(
        backend.writes.load(Ordering::Relaxed),
        2,
        "one chunk PUT and one head PUT"
    );

    advance(Duration::from_secs(30)).await;
    settle().await;
    assert_eq!(
        backend.writes.load(Ordering::Relaxed),
        2,
        "no empty timer writes"
    );
    writer.add_event(&event(3, "new batch")).await.unwrap();
    settle().await;
    advance(Duration::from_secs(4)).await;
    settle().await;
    assert_eq!(chunks(&backend).len(), 1);
    advance(Duration::from_secs(1)).await;
    settle().await;
    assert_eq!(chunks(&backend).len(), 2);
    writer.close().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn sixty_four_kib_triggers_early_and_oversized_events_remain_complete_jsonl() {
    let backend = Arc::new(HistoryBackend::new("", 0));
    let storage = backend.storage();
    let writer = DanmuWriter::from_storage(&storage, "events.jsonl")
        .await
        .unwrap();
    let start = Instant::now();
    let half = sized_event(LOG_CHUNK_BYTES / 2);
    writer.add_event(&half).await.unwrap();
    settle().await;
    assert!(chunks(&backend).is_empty());
    writer.add_event(&half).await.unwrap();
    settle().await;
    assert_eq!(
        Instant::now(),
        start,
        "size flush must not wait for the timer"
    );
    assert_eq!(
        chunks(&backend).iter().map(Vec::len).collect::<Vec<_>>(),
        [LOG_CHUNK_BYTES]
    );
    let big = sized_event(LOG_CHUNK_BYTES + 12);
    writer.add_event(&big).await.unwrap();
    settle().await;
    writer.close().await.unwrap();
    assert!(chunks(&backend)
        .iter()
        .all(|bytes| bytes.len() <= LOG_CHUNK_BYTES));
    backend.allow_history.store(true, Ordering::Relaxed);
    let contents = storage.read_log("events.jsonl").await.unwrap();
    assert_eq!(
        contents,
        [
            event_line(&half).unwrap(),
            event_line(&half).unwrap(),
            event_line(&big).unwrap()
        ]
        .concat()
    );
    assert_eq!(
        DanmuStorage::read_entries(&storage, "events.jsonl")
            .await
            .unwrap()
            .len(),
        3
    );
}

#[tokio::test(start_paused = true)]
async fn close_flushes_partial_batch_immediately_and_resume_keeps_event_order() {
    let backend = Arc::new(HistoryBackend::new("", 0));
    let storage = backend.storage();
    let start = Instant::now();
    for ts in 0..2 {
        let writer = DanmuWriter::from_storage(&storage, "events.jsonl")
            .await
            .unwrap();
        writer.add_event(&event(ts, "one")).await.unwrap();
        writer.add_event(&event(ts, "two")).await.unwrap();
        writer.close().await.unwrap();
    }
    assert_eq!(Instant::now(), start);
    assert_eq!(chunks(&backend).len(), 2);
    backend.allow_history.store(true, Ordering::Relaxed);
    let entries = DanmuStorage::read_entries(&storage, "events.jsonl")
        .await
        .unwrap();
    assert_eq!(
        entries
            .iter()
            .map(|e| (e.ts, e.content.as_str()))
            .collect::<Vec<_>>(),
        [(0, "one"), (0, "two"), (1, "one"), (1, "two")]
    );
}

#[tokio::test(start_paused = true)]
async fn failed_chunk_or_head_retains_batch_and_retries_without_losing_queued_events() {
    for fail_head in [false, true] {
        let backend = Arc::new(HistoryBackend::new("", 0));
        backend.fail_head.store(fail_head, Ordering::Relaxed);
        backend.fail_writes.store(!fail_head, Ordering::Relaxed);
        let storage = backend.storage();
        let writer = DanmuWriter::from_storage(&storage, "events.jsonl")
            .await
            .unwrap();
        writer.add_event(&event(1, "failed batch")).await.unwrap();
        settle().await;
        advance(Duration::from_secs(5)).await;
        settle().await;
        let attempted = backend.writes.load(Ordering::Relaxed);
        assert!(attempted > 0);
        for ts in 2..12 {
            writer.add_event(&event(ts, "queued")).await.unwrap();
        }
        settle().await;
        advance(Duration::from_secs(4)).await;
        settle().await;
        assert_eq!(
            backend.writes.load(Ordering::Relaxed),
            attempted,
            "no per-event retry storm"
        );
        backend.fail_writes.store(false, Ordering::Relaxed);
        backend.fail_head.store(false, Ordering::Relaxed);
        advance(Duration::from_secs(1)).await;
        settle().await;
        writer.close().await.unwrap();
        assert_eq!(
            chunks(&backend).len(),
            2,
            "catch-up events are also batched"
        );
        backend.allow_history.store(true, Ordering::Relaxed);
        let entries = DanmuStorage::read_entries(&storage, "events.jsonl")
            .await
            .unwrap();
        assert_eq!(
            entries.iter().map(|entry| entry.ts).collect::<Vec<_>>(),
            (1..12).collect::<Vec<_>>()
        );
    }
}

#[tokio::test(start_paused = true)]
async fn permanent_failure_is_reported_on_close_instead_of_leaving_a_retrying_worker() {
    let backend = Arc::new(HistoryBackend::new("", 0));
    let writer = DanmuWriter::from_storage(&backend.storage(), "events.jsonl")
        .await
        .unwrap();
    backend.fail_writes.store(true, Ordering::Relaxed);
    writer
        .add_event(&sized_event(LOG_CHUNK_BYTES))
        .await
        .unwrap();
    settle().await;
    assert_eq!(backend.writes.load(Ordering::Relaxed), 1);
    assert_eq!(
        writer.close().await.unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    let attempts = backend.writes.load(Ordering::Relaxed);
    advance(Duration::from_secs(60)).await;
    settle().await;
    assert_eq!(backend.writes.load(Ordering::Relaxed), attempts);
    assert!(chunks(&backend).is_empty());
}

#[tokio::test(start_paused = true)]
async fn slow_upload_accepts_more_events_but_applies_backpressure_at_queue_limit() {
    let backend = Arc::new(HistoryBackend::new("", 0));
    let storage = backend.storage();
    let writer = DanmuWriter::from_storage(&storage, "events.jsonl")
        .await
        .unwrap();
    backend.pause_write.store(true, Ordering::Relaxed);
    writer
        .add_event(&sized_event(LOG_CHUNK_BYTES))
        .await
        .unwrap();
    backend.writing.notified().await;
    for ts in 0..QUEUED_EVENTS {
        writer.add_event(&event(ts as i64, "queued")).await.unwrap();
    }
    assert!(tokio::time::timeout(
        Duration::from_millis(10),
        writer.add_event(&event(-1, "not accepted"))
    )
    .await
    .is_err());
    // Let the queued events age; recovery must still coalesce them, not create
    // one object for every expired event.
    advance(Duration::from_secs(6)).await;
    backend.release.notify_one();
    settle().await;
    writer.close().await.unwrap();
    assert_eq!(chunks(&backend).len(), 2);
    backend.allow_history.store(true, Ordering::Relaxed);
    let entries = DanmuStorage::read_entries(&storage, "events.jsonl")
        .await
        .unwrap();
    assert_eq!(entries.len(), 1 + QUEUED_EVENTS);
    assert!(entries.iter().all(|e| e.ts != -1));
}

#[tokio::test(start_paused = true)]
async fn dropped_or_cancelled_close_is_drained_before_storage_can_be_deleted() {
    for cancel_close in [false, true] {
        let backend = Arc::new(HistoryBackend::new("bilibili/room/live/", 0));
        let store = RecordingStore::with_backend(backend.clone(), "unused-batch-probe");
        let storage = store.recording("bilibili", "room", "live").unwrap();
        let writer = DanmuWriter::from_storage(&storage, "events.jsonl")
            .await
            .unwrap();
        backend.pause_write.store(true, Ordering::Relaxed);
        writer.add_event(&event(1, "final batch")).await.unwrap();
        if cancel_close {
            let closing = tokio::spawn(writer.close());
            backend.writing.notified().await;
            closing.abort();
            assert!(closing.await.unwrap_err().is_cancelled());
        } else {
            drop(writer);
            backend.writing.notified().await;
        }
        let mut draining = tokio::spawn(async move { store.flush_logs().await });
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut draining)
                .await
                .is_err()
        );
        backend.release.notify_one();
        draining.await.unwrap();
        assert_eq!(chunks(&backend).len(), 1);
        let before = backend.writes.load(Ordering::Relaxed);
        advance(Duration::from_secs(60)).await;
        settle().await;
        assert_eq!(backend.writes.load(Ordering::Relaxed), before);
    }
}

#[tokio::test(start_paused = true)]
async fn close_rejects_a_waiting_enqueue_without_deadlocking_on_a_full_failed_queue() {
    let backend = Arc::new(HistoryBackend::new("", 0));
    let writer = DanmuWriter::from_storage(&backend.storage(), "events.jsonl")
        .await
        .unwrap();
    backend.fail_writes.store(true, Ordering::Relaxed);
    writer
        .add_event(&sized_event(LOG_CHUNK_BYTES))
        .await
        .unwrap();
    settle().await;
    for ts in 0..QUEUED_EVENTS {
        writer.add_event(&event(ts as i64, "queued")).await.unwrap();
    }
    let mut enqueue = tokio_test::task::spawn(writer.add_event(&event(-1, "waiting")));
    assert!(enqueue.poll().is_pending());
    assert_eq!(
        writer.close().await.unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    let std::task::Poll::Ready(result) = enqueue.poll() else {
        panic!("close must release capacity waiters")
    };
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::BrokenPipe);
}

#[tokio::test]
#[ignore = "requires RustFS and explicit S3_TEST_* test-harness credentials"]
async fn rustfs_batches_events_and_resumes_existing_log() {
    use recording_storage::{S3Backend, S3Config, S3Credentials};
    let config = S3Config {
        endpoint: std::env::var("S3_TEST_ENDPOINT").expect("set S3_TEST_ENDPOINT"),
        bucket: std::env::var("S3_TEST_BUCKET").expect("set S3_TEST_BUCKET"),
        region: std::env::var("S3_TEST_REGION").unwrap_or_else(|_| "us-east-1".into()),
        prefix: format!("event-batch-probe-{}", uuid::Uuid::new_v4()),
        path_style: true,
    };
    // Harness input only; production credentials still come from settings.
    let credentials = S3Credentials {
        access_key_id: std::env::var("S3_TEST_ACCESS_KEY_ID").expect("set S3_TEST_ACCESS_KEY_ID"),
        secret_access_key: std::env::var("S3_TEST_SECRET_ACCESS_KEY")
            .expect("set S3_TEST_SECRET_ACCESS_KEY"),
        session_token: String::new(),
    };
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("must-not-exist");
    let store = RecordingStore::with_backend(
        Arc::new(S3Backend::new(&config, &credentials).unwrap()),
        &workspace,
    );
    let storage = store.recording("bilibili", "room", "live").unwrap();
    storage.prepare().await.unwrap();
    let writer = DanmuWriter::from_storage(&storage, "events.jsonl")
        .await
        .unwrap();
    for ts in 0..100 {
        writer.add_event(&event(ts, "batch")).await.unwrap();
    }
    writer.close().await.unwrap();
    let keys = storage.list("", None, 10).await.unwrap().keys;
    assert_eq!(
        keys.len(),
        2,
        "100 events should use one data object and one head"
    );
    let writer = DanmuWriter::from_storage(&storage, "events.jsonl")
        .await
        .unwrap();
    writer
        .add_event(&sized_event(LOG_CHUNK_BYTES))
        .await
        .unwrap();
    writer.add_event(&event(101, "tail")).await.unwrap();
    writer.close().await.unwrap();
    assert_eq!(storage.list("", None, 10).await.unwrap().keys.len(), 4);
    let entries = DanmuStorage::read_entries(&storage, "events.jsonl")
        .await
        .unwrap();
    assert_eq!(entries.len(), 102);
    assert_eq!(
        entries[..100].iter().map(|e| e.ts).collect::<Vec<_>>(),
        (0..100).collect::<Vec<_>>()
    );
    assert_eq!(entries.last().unwrap().content, "tail");
    assert!(!workspace.exists());
    // Only remove this test's unique recording namespace.
    storage.remove_all().await.unwrap();
    assert!(storage.list("", None, 10).await.unwrap().keys.is_empty());
    eprintln!(
        "RustFS event batching: 100 events -> 1 data object; resume/size/tail/readback passed"
    );
}
