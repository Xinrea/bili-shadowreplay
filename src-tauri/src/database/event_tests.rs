use super::*;
use crate::database::{credentials::CredentialCipher, task::TaskRow, video::VideoRow};

async fn database() -> Database {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::raw_sql(
        "CREATE TABLE videos (
            id INTEGER PRIMARY KEY, room_id TEXT, cover TEXT, file TEXT,
            note TEXT, length INTEGER, size INTEGER, status INTEGER, bvid TEXT,
            title TEXT, desc TEXT, tags TEXT, area INTEGER, created_at TEXT, platform TEXT,
            live_id TEXT, source TEXT, duration REAL, needs_project INTEGER DEFAULT 0,
            danmu_embedded INTEGER DEFAULT 0
        );
        CREATE TABLE tasks (
            id TEXT PRIMARY KEY, type TEXT, status TEXT, message TEXT,
            metadata TEXT, created_at TEXT
        );
        CREATE TABLE records (
            platform TEXT, parent_id TEXT, live_id TEXT PRIMARY KEY, room_id TEXT,
            title TEXT, length REAL, size INTEGER, created_at TEXT, cover TEXT
        );",
    )
    .execute(&pool)
    .await
    .unwrap();
    Database::new(
        pool,
        CredentialCipher::new(&[0; 32]).unwrap(),
        EventBus::new(),
    )
}

#[tokio::test]
async fn video_notifications_follow_committed_mutations() {
    let db = database().await;
    let mut changes = db.events.subscribe();
    let mut video = db
        .add_video(&VideoRow {
            id: 0,
            room_id: "1".into(),
            cover: "cover.jpg".into(),
            file: "clip.mp4".into(),
            note: "".into(),
            length: 10,
            size: 100,
            status: 0,
            bvid: "".into(),
            title: "clip".into(),
            desc: "".into(),
            tags: "".into(),
            area: 0,
            created_at: "2026-10-11".into(),
            platform: "bilibili".into(),
            live_id: "".into(),
            source: "".into(),
            duration: 10.0,
            danmu_embedded: false,
        })
        .await
        .unwrap();
    assert_eq!(
        changes.recv().await.unwrap().client_event().name,
        "videos-changed"
    );
    assert_eq!(db.get_all_videos().await.unwrap()[0].id, video.id);

    video.note = "updated".into();
    db.update_video(&video).await.unwrap();
    assert_eq!(
        changes.recv().await.unwrap().client_event().name,
        "videos-changed"
    );
    assert_eq!(db.get_video(video.id).await.unwrap().note, "updated");

    db.update_video_cover(video.id, "new.jpg").await.unwrap();
    assert_eq!(
        changes.recv().await.unwrap().client_event().name,
        "videos-changed"
    );
    assert_eq!(db.get_video(video.id).await.unwrap().cover, "new.jpg");

    db.delete_video(video.id).await.unwrap();
    assert_eq!(
        changes.recv().await.unwrap().client_event().name,
        "videos-changed"
    );
    assert!(db.get_all_videos().await.unwrap().is_empty());

    sqlx::query("DROP TABLE videos")
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(db.add_video(&video).await.is_err());
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(10), changes.recv())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn tasks_notify_on_creation_progress_completion_and_deletion() {
    let db = database().await;
    let mut changes = db.events.subscribe();
    let task = TaskRow {
        id: "test".into(),
        task_type: "clip_range".into(),
        status: "pending".into(),
        message: "".into(),
        metadata: "{}".into(),
        created_at: "2026-10-11".into(),
    };
    db.add_task(&task).await.unwrap();
    assert_eq!(
        changes.recv().await.unwrap().client_event().name,
        "tasks-changed"
    );
    // A failed duplicate insert must not advertise a successful change.
    assert!(db.add_task(&task).await.is_err());
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(10), changes.recv())
            .await
            .is_err()
    );
    for status in ["processing", "success"] {
        for _ in 0..if status == "processing" { 100 } else { 1 } {
            db.update_task(&task.id, status, status, None)
                .await
                .unwrap();
        }
        assert_eq!(
            changes.recv().await.unwrap().client_event().name,
            "tasks-changed"
        );
        assert_eq!(db.get_task(&task.id).await.unwrap().status, status);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), changes.recv())
                .await
                .is_err()
        );
    }
    db.delete_task(&task.id).await.unwrap();
    assert_eq!(
        changes.recv().await.unwrap().client_event().name,
        "tasks-changed"
    );
    assert!(db.get_tasks().await.unwrap().is_empty());
}

#[tokio::test]
async fn cancelled_task_cannot_be_revived_by_late_progress_or_completion() {
    let db = database().await;
    let task = db
        .generate_task("encode_video_subtitle", "waiting", "{}")
        .await
        .unwrap();
    db.update_task(&task.id, "cancelled", "任务取消", None)
        .await
        .unwrap();
    for status in ["processing", "success", "failed"] {
        db.update_task(&task.id, status, "late update", Some("{\"late\":true}"))
            .await
            .unwrap();
        let row = db.get_task(&task.id).await.unwrap();
        assert_eq!(row.status, "cancelled");
        assert_eq!(row.message, "任务取消");
        assert_eq!(row.metadata, "{}");
    }
}

#[tokio::test]
async fn completed_task_cannot_be_overwritten_by_cancellation() {
    let db = database().await;
    for status in ["success", "failed"] {
        let task = db
            .generate_task("encode_video_subtitle", "waiting", "{}")
            .await
            .unwrap();
        db.update_task(&task.id, status, "finished", Some("{\"finished\":true}"))
            .await
            .unwrap();
        assert!(!db.cancel_task_if_active(&task.id).await.unwrap());
        for metadata in [None, Some("{\"cancelled\":true}")] {
            db.update_task(&task.id, "cancelled", "任务取消", metadata)
                .await
                .unwrap();
            let row = db.get_task(&task.id).await.unwrap();
            assert_eq!(row.status, status);
            assert_eq!(row.message, "finished");
            assert_eq!(row.metadata, "{\"finished\":true}");
        }
    }
}

#[tokio::test]
async fn cancellation_claim_only_succeeds_once_for_active_tasks() {
    let db = database().await;
    for status in ["pending", "processing"] {
        let task = db
            .generate_task("encode_video_subtitle", "waiting", "{}")
            .await
            .unwrap();
        db.update_task(&task.id, status, "working", None)
            .await
            .unwrap();
        assert!(db.cancel_task_if_active(&task.id).await.unwrap());
        assert!(!db.cancel_task_if_active(&task.id).await.unwrap());
        let row = db.get_task(&task.id).await.unwrap();
        assert_eq!(row.status, "cancelled");
        assert_eq!(row.message, "任务取消");
    }
    assert!(!db.cancel_task_if_active("missing").await.unwrap());
}

#[tokio::test]
async fn recording_statistics_notify_after_committing_and_coalesce_bursts() {
    let db = database().await;
    let mut changes = db.events.subscribe();
    db.add_record(
        ::recorder::platforms::PlatformType::BiliBili,
        "parent",
        "live",
        "1",
        "title",
        None,
    )
    .await
    .unwrap();
    changes.recv().await.unwrap();
    for _ in 0..100 {
        db.update_record_delta("live", 0.5, 10).await.unwrap();
    }
    assert_eq!(
        changes.recv().await.unwrap().client_event().name,
        "records-changed"
    );
    let record = db.get_record("1", "live").await.unwrap();
    assert_eq!(record.length, 50.0);
    assert_eq!(record.size, 1_000);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), changes.recv())
            .await
            .is_err()
    );
    assert!(db.update_record_delta("live", 1.0, u64::MAX).await.is_err());
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(1_100), changes.recv())
            .await
            .is_err()
    );
}
