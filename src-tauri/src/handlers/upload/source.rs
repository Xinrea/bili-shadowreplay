//! A saved web draft owns a durable copy, not a path into the HTTP session.
use std::{
    path::Path,
    path::PathBuf,
    sync::{Arc, LazyLock},
};
use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};

use crate::{database::upload::UploadRow, database::Database, temp_storage::TempStorage};

fn source_root(output: &Path) -> PathBuf {
    output.join("uploads").join("sources")
}

// Source publication/deletion and output relocation must share one boundary:
// a draft cannot publish an old-root path after a directory switch's snapshot.
static SOURCE_OPERATIONS: LazyLock<Arc<RwLock<()>>> = LazyLock::new(|| Arc::new(RwLock::new(())));

pub(super) async fn source_operation() -> Arc<OwnedRwLockReadGuard<()>> {
    Arc::new(SOURCE_OPERATIONS.clone().read_owned().await)
}

pub(crate) async fn output_change() -> OwnedRwLockWriteGuard<()> {
    SOURCE_OPERATIONS.clone().write_owned().await
}

fn copy_source(file: &Path, root: &Path) -> std::io::Result<(tempfile::TempDir, PathBuf)> {
    std::fs::create_dir_all(root)?;
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
    Ok((directory, target))
}

/// Copy before inserting the row, without holding a SQLite transaction over
/// large file I/O. Each draft gets its own directory, including repeated picks.
/// The task owns the session and staged copy through the DB acknowledgement.
pub(super) async fn save_draft(
    db: Arc<Database>,
    session: TempStorage,
    output: PathBuf,
    mut upload: UploadRow,
    operation: Arc<OwnedRwLockReadGuard<()>>,
) -> Result<UploadRow, String> {
    tokio::spawn(async move {
        let _operation = operation;
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
                copy_source(&file, &root).map(Some)
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

/// Copy first and keep originals until every row has been atomically rebound.
/// The caller holds output_change() through config publication and cleanup.
pub(crate) struct SourceRelocation {
    staged: Vec<tempfile::TempDir>,
    rows: Vec<(UploadRow, UploadRow)>,
}

pub(crate) async fn prepare_relocation(
    db: &Database,
    old_output: &Path,
    new_output: &Path,
) -> Result<SourceRelocation, String> {
    let uploads = db.get_uploads().await?;
    let old_output = old_output.to_path_buf();
    let new_output = new_output.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<_, String> {
        let mut relocation = SourceRelocation {
            staged: Vec::new(),
            rows: Vec::new(),
        };
        let old_root = match source_root(&old_output).canonicalize() {
            Ok(root) => root,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(relocation),
            Err(error) => return Err(error.to_string()),
        };
        for upload in uploads {
            if upload.source != "file" {
                continue;
            }
            let file = Path::new(&upload.file);
            let Some(directory) = file.parent() else {
                continue;
            };
            if directory.parent() != Some(old_root.as_path())
                || !directory
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("source-"))
            {
                continue;
            }
            if upload.is_active() {
                return Err("文件投稿进行中，请结束后再切换素材目录".into());
            }
            let (staged, target) = copy_source(file, &source_root(&new_output))
                .map_err(|error| format!("迁移投稿源文件失败: {error}"))?;
            let mut next = upload.clone();
            next.file = target.to_string_lossy().into_owned();
            // Upload covers also live below output. Copy only explicitly
            // referenced assets, never an arbitrary pre-existing uploads tree.
            for (cover, name) in [
                (&mut next.cover, "cover.jpg"),
                (&mut next.cover43, "cover43.jpg"),
            ] {
                if cover.is_empty() {
                    continue;
                }
                recording_storage::validate_key(cover).map_err(|error| error.to_string())?;
                let covers = staged.path().join("covers");
                std::fs::create_dir_all(&covers).map_err(|error| error.to_string())?;
                let target = covers.join(format!("{}-{name}", upload.id));
                std::fs::copy(old_output.join(&*cover), &target)
                    .map_err(|error| format!("迁移投稿封面失败: {error}"))?;
                std::fs::OpenOptions::new()
                    .write(true)
                    .open(&target)
                    .and_then(|file| file.sync_all())
                    .map_err(|error| error.to_string())?;
                *cover = target
                    .strip_prefix(
                        new_output
                            .canonicalize()
                            .map_err(|error| error.to_string())?,
                    )
                    .map_err(|error| error.to_string())?
                    .to_string_lossy()
                    .replace('\\', "/");
            }
            relocation.staged.push(staged);
            relocation.rows.push((upload, next));
        }
        Ok(relocation)
    })
    .await
    .map_err(|error| error.to_string())?
}

impl SourceRelocation {
    pub(crate) async fn commit(self, db: &Database) -> Result<Vec<UploadRow>, String> {
        if !db.relocate_upload_sources(&self.rows).await? {
            return Err("投稿已发生变化，请重试素材目录切换".into());
        }
        for directory in self.staged {
            let _ = directory.keep();
        }
        Ok(self
            .rows
            .into_iter()
            .map(|(previous, _)| previous)
            .collect())
    }
}

/// Only remove this feature's private source directories, never a user-selected
/// file. Preserve a source if an older/imported row still references it.
pub(crate) async fn remove_owned_source(
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

    async fn save_draft(
        db: Arc<Database>,
        session: TempStorage,
        output: PathBuf,
        upload: UploadRow,
    ) -> Result<UploadRow, String> {
        super::save_draft(db, session, output, upload, source_operation().await).await
    }

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
        let mut cover = output.join(&created.display_cover);
        assert!(cover.is_file());
        state.temp_storage = TempStorage::new().unwrap();
        assert!(!input.exists());
        let mut output = output;
        // Switching twice also exercises covers already relocated alongside
        // their source. Unrelated files in both output roots must survive.
        for next_output in [
            root.path().join("next-output"),
            root.path().join("third-output"),
        ] {
            let previous = state.db.get_upload(created.upload.id).await.unwrap();
            std::fs::create_dir_all(next_output.join("uploads/sources")).unwrap();
            let unrelated = next_output.join("uploads/sources/user-file.txt");
            std::fs::write(&unrelated, b"keep").unwrap();
            crate::handlers::config::set_output_path(
                state.clone(),
                next_output.to_string_lossy().into_owned(),
            )
            .await
            .unwrap();
            assert!(!Path::new(&previous.file).exists());
            assert!(!cover.exists());
            let next = state.db.get_upload(created.upload.id).await.unwrap();
            output = next_output;
            cover = output.join(&next.cover);
            assert!(cover.is_file());
            assert_eq!(std::fs::read(unrelated).unwrap(), b"keep");
        }
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
    async fn relocation_conflict_rolls_back_rows_and_cleans_only_staged_copies() {
        let (db, _) = database().await;
        let output = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
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
        let second = save_draft(db.clone(), session, output.path().into(), draft(&input))
            .await
            .unwrap();
        let _operation = output_change().await;
        let staged = prepare_relocation(&db, output.path(), target.path())
            .await
            .unwrap();
        // Posting may claim a saved row during the copy. No partially rebound
        // set of sources should be acknowledged, even if another row was first.
        assert!(db.claim_upload(second.id, "later").await.unwrap());
        assert!(staged.commit(&db).await.is_err());
        for row in [&first, &second] {
            assert_eq!(db.get_upload(row.id).await.unwrap().file, row.file);
            assert_eq!(std::fs::read(&row.file).unwrap(), b"uploaded video");
        }
        assert_eq!(
            std::fs::read_dir(source_root(target.path()))
                .unwrap()
                .count(),
            0
        );
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
