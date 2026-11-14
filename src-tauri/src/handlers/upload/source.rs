//! A saved web draft owns a durable copy, not a path into the HTTP session.
use std::{path::Path, path::PathBuf, sync::Arc};

use crate::{database::upload::UploadRow, database::Database, temp_storage::TempStorage};

fn source_root(output: &Path) -> PathBuf {
    output.join("uploads").join("sources")
}

/// Copy before inserting the row, without holding a SQLite transaction over
/// large file I/O. Each draft gets its own directory, including repeated picks.
/// The task owns the session and staged copy through the DB acknowledgement.
pub(super) async fn save_draft(
    db: Arc<Database>,
    session: TempStorage,
    output: PathBuf,
    mut upload: UploadRow,
) -> Result<UploadRow, String> {
    tokio::spawn(async move {
        let staged = if upload.source == "file" {
            let file = PathBuf::from(&upload.file);
            let root = source_root(&output);
            let staged = tokio::task::spawn_blocking(move || -> std::io::Result<_> {
                let file = file.canonicalize()?;
                let session_root = session.root().canonicalize()?;
                let owned_root = root.canonicalize().ok();
                if !file.starts_with(session_root)
                    && !owned_root.is_some_and(|root| file.starts_with(root))
                {
                    // Desktop-selected user files remain external references.
                    return Ok(None);
                }
                std::fs::create_dir_all(&root)?;
                let directory = tempfile::Builder::new()
                    .prefix("source-")
                    .tempdir_in(root.canonicalize()?)?;
                let target = directory.path().join(
                    file.file_name()
                        .ok_or_else(|| std::io::Error::other("投稿源文件没有文件名"))?,
                );
                std::fs::copy(file, &target)?;
                std::fs::OpenOptions::new()
                    .write(true)
                    .open(&target)?
                    .sync_all()?;
                Ok(Some((directory, target)))
            })
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| format!("保存投稿源文件失败: {error}"))?;
            if let Some((_, target)) = &staged {
                upload.file = target.to_string_lossy().into_owned();
            }
            staged
        } else {
            None
        };
        let saved = db.add_upload(&upload).await?;
        if let Some((directory, _)) = staged {
            // No await between DB acknowledgement and relinquishing cleanup.
            let _ = directory.keep();
        }
        Ok(saved)
    })
    .await
    .map_err(|error| format!("保存投稿草稿失败: {error}"))?
}

/// Only remove this feature's private source directories, never a user-selected
/// file. Preserve a source if an older/imported row still references it.
pub(super) async fn remove_owned_source(
    db: &Database,
    output: &Path,
    upload: &UploadRow,
) -> Result<(), String> {
    if upload.source != "file" {
        return Ok(());
    }
    let file = Path::new(&upload.file);
    let Some(directory) = file.parent() else {
        return Ok(());
    };
    let Ok(root) = source_root(output).canonicalize() else {
        return Ok(());
    };
    if directory.parent() != Some(root.as_path())
        || !directory
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with("source-"))
        || db.upload_file_in_use(&upload.file).await?
    {
        return Ok(());
    }
    if let Err(error) = tokio::fs::remove_dir_all(directory).await {
        if error.kind() != std::io::ErrorKind::NotFound {
            log::warn!("删除投稿源文件目录失败: {} - {error}", directory.display());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{database::credentials::CredentialCipher, events::EventBus};

    async fn database() -> (Arc<Database>, sqlx::SqlitePool) {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        for migration in crate::get_migrations() {
            sqlx::raw_sql(migration.sql).execute(&pool).await.unwrap();
        }
        let db = Arc::new(Database::new(
            pool.clone(),
            CredentialCipher::new(&[0; 32]).unwrap(),
            EventBus::new(),
        ));
        (db, pool)
    }

    fn draft(file: &Path) -> UploadRow {
        UploadRow {
            id: 0,
            source: "file".into(),
            source_id: 0,
            file: file.to_string_lossy().into_owned(),
            room_id: String::new(),
            uid: String::new(),
            cover: String::new(),
            cover43: String::new(),
            profile: "{}".into(),
            danmu: false,
            render_settings_json: "{}".into(),
            status: "draft".into(),
            message: String::new(),
            bvid: String::new(),
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    fn uploaded(session: &TempStorage) -> PathBuf {
        let path = session.root().join("uploads").join("视频.mp4");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"uploaded video").unwrap();
        path
    }

    #[cfg(feature = "headless")]
    #[tokio::test]
    async fn saved_web_upload_keeps_preview_cover_and_deletion_after_session_change() {
        use crate::{
            config::Config,
            recorder_manager::RecorderManager,
            state::State,
            task::TaskManager,
            webhook::poster::{WebhookConfig, WebhookPoster},
        };
        use tokio::sync::RwLock;

        let (db, _) = database().await;
        let root = tempfile::tempdir().unwrap();
        let output = root.path().join("output");
        let config = Arc::new(RwLock::new(
            Config::load(
                &root.path().join("config.toml"),
                &root.path().join("cache"),
                &output,
            )
            .unwrap(),
        ));
        let events = EventBus::new();
        let task_manager = Arc::new(TaskManager::new());
        let webhook_poster = WebhookPoster::new(WebhookConfig::default()).unwrap();
        let manager = Arc::new(RecorderManager::new(
            events.clone(),
            db.clone(),
            config.clone(),
            task_manager.clone(),
            root.path().into(),
            webhook_poster.clone(),
        ));
        let mut state = State {
            mcp_server: Arc::new(crate::mcp::McpServer::default()),
            temp_storage: TempStorage::new().unwrap(),
            db,
            config,
            webhook_poster,
            recorder_manager: manager,
            task_manager,
            events,
            resource_dir: root.path().into(),
            readonly: false,
        };
        let input = uploaded(&state.temp_storage);
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("crates/ffmpeg_utils/src/hls/fixtures/live.flv"),
            &input,
        )
        .unwrap();
        let created = super::super::create_upload(
            state.clone(),
            "file".into(),
            0,
            input.to_string_lossy().into_owned(),
            serde_json::json!({"title": "persistent source"}),
        )
        .await
        .unwrap();
        assert!(!created.display_cover.is_empty());
        let cover = output.join(&created.display_cover);
        assert!(cover.is_file());
        state.temp_storage = TempStorage::new().unwrap();
        assert!(!input.exists());
        let loaded = super::super::get_upload(state.clone(), created.upload.id)
            .await
            .unwrap();
        assert_eq!(loaded.source_state, "file");
        let video = loaded.video.unwrap();
        assert_eq!(video.base, "output");
        assert!(output.join(video.path).is_file());
        assert!(super::super::frame_video_path(&state, &loaded.upload)
            .await
            .is_some());
        super::super::delete_upload(state, loaded.upload.id)
            .await
            .unwrap();
        assert!(!Path::new(&loaded.upload.file).exists());
        assert!(!cover.exists());
    }

    #[tokio::test]
    async fn web_drafts_survive_session_cleanup_and_own_independent_sources() {
        let (db, pool) = database().await;
        let output = tempfile::tempdir().unwrap();
        let session = TempStorage::new().unwrap();
        let input = uploaded(&session);
        let first = save_draft(
            db.clone(),
            session.clone(),
            output.path().into(),
            draft(&input),
        )
        .await
        .unwrap();
        let second = save_draft(
            db.clone(),
            session.clone(),
            output.path().into(),
            draft(&input),
        )
        .await
        .unwrap();
        assert_ne!(first.file, second.file);
        drop(session);
        assert!(!input.exists(), "the session really has been cleaned up");
        // Rebuild application ownership around the persisted database rows.
        drop(db);
        let db = Database::new(
            pool,
            CredentialCipher::new(&[0; 32]).unwrap(),
            EventBus::new(),
        );
        let session = TempStorage::new().unwrap();
        for row in [&first, &second] {
            let saved = db.get_upload(row.id).await.unwrap();
            let path = Path::new(&saved.file);
            assert_eq!(std::fs::read(path).unwrap(), b"uploaded video");
            assert!(path.starts_with(output.path().canonicalize().unwrap()));
            assert!(!path.starts_with(session.root()));
        }
        db.delete_upload(first.id).await.unwrap();
        remove_owned_source(&db, output.path(), &first)
            .await
            .unwrap();
        assert!(!Path::new(&first.file).exists());
        assert!(Path::new(&second.file).exists());
        db.delete_upload(second.id).await.unwrap();
        remove_owned_source(&db, output.path(), &second)
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_dir(source_root(output.path()))
                .unwrap()
                .count(),
            0
        );
    }

    #[tokio::test]
    async fn external_files_are_not_copied_or_deleted() {
        let (db, _) = database().await;
        let output = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let input = external.path().join("user.mp4");
        std::fs::write(&input, b"user file").unwrap();
        let saved = save_draft(
            db.clone(),
            TempStorage::new().unwrap(),
            output.path().into(),
            draft(&input),
        )
        .await
        .unwrap();
        assert_eq!(saved.file, input.to_string_lossy());
        db.delete_upload(saved.id).await.unwrap();
        remove_owned_source(&db, output.path(), &saved)
            .await
            .unwrap();
        assert_eq!(std::fs::read(input).unwrap(), b"user file");
        assert!(!source_root(output.path()).exists());
    }

    #[tokio::test]
    async fn failed_draft_insert_cleans_copy_but_preserves_uploaded_input() {
        let (db, pool) = database().await;
        sqlx::raw_sql("CREATE TRIGGER reject_upload BEFORE INSERT ON uploads BEGIN SELECT RAISE(FAIL, 'injected'); END")
            .execute(&pool).await.unwrap();
        let output = tempfile::tempdir().unwrap();
        let session = TempStorage::new().unwrap();
        let input = uploaded(&session);
        assert!(save_draft(
            db.clone(),
            session.clone(),
            output.path().into(),
            draft(&input),
        )
        .await
        .is_err());
        assert!(input.exists());
        assert!(db.get_uploads().await.unwrap().is_empty());
        assert_eq!(
            std::fs::read_dir(source_root(output.path()))
                .unwrap()
                .count(),
            0
        );
    }

    #[tokio::test]
    async fn cleanup_preserves_sources_referenced_by_another_saved_row() {
        let (db, _) = database().await;
        let output = tempfile::tempdir().unwrap();
        let session = TempStorage::new().unwrap();
        let input = uploaded(&session);
        let saved = save_draft(db.clone(), session, output.path().into(), draft(&input))
            .await
            .unwrap();
        let other = db.add_upload(&saved).await.unwrap();
        db.delete_upload(saved.id).await.unwrap();
        remove_owned_source(&db, output.path(), &saved)
            .await
            .unwrap();
        assert!(Path::new(&other.file).is_file());
        db.delete_upload(other.id).await.unwrap();
        remove_owned_source(&db, output.path(), &other)
            .await
            .unwrap();
        assert!(!Path::new(&other.file).exists());
    }
}
