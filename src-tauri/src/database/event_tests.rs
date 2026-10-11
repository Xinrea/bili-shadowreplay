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
            title TEXT, desc TEXT, tags TEXT, area INTEGER, created_at TEXT, platform TEXT
        );
        CREATE TABLE tasks (
            id TEXT PRIMARY KEY, type TEXT, status TEXT, message TEXT,
            metadata TEXT, created_at TEXT
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
        db.update_task(&task.id, status, status, None)
            .await
            .unwrap();
        assert_eq!(
            changes.recv().await.unwrap().client_event().name,
            "tasks-changed"
        );
        assert_eq!(db.get_task(&task.id).await.unwrap().status, status);
    }
    db.delete_task(&task.id).await.unwrap();
    assert_eq!(
        changes.recv().await.unwrap().client_event().name,
        "tasks-changed"
    );
    assert!(db.get_tasks().await.unwrap().is_empty());
}
