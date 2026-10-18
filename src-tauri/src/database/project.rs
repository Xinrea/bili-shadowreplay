use sqlx::query::Query;
use sqlx::sqlite::SqliteArguments;
use sqlx::Sqlite;

use super::upload::{insert_upload, UploadRow};
use super::Database;
use super::DatabaseError;
use crate::events::DataChange;

/// A project (切片): an edit of one or more materials that is exported and
/// uploaded. `timeline` holds [`crate::project::Timeline`] JSON and `profile`
/// the upload form carried over from before uploads existed.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct ProjectRow {
    pub id: i64,
    pub room_id: String,
    pub platform: String,
    pub title: String,
    pub note: String,
    /// Upload cover relative to the output directory; empty to use the first
    /// material's thumbnail.
    pub cover: String,
    pub timeline: String,
    pub profile: String,
    /// 0 = not posted, 1 = posted.
    pub status: i64,
    pub bvid: String,
    pub created_at: String,
    pub updated_at: String,
}

fn insert_project(project: &ProjectRow) -> Query<'_, Sqlite, SqliteArguments<'_>> {
    sqlx::query(
        "INSERT INTO projects (room_id, platform, title, note, cover, timeline, profile, status, bvid, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(&project.room_id)
    .bind(&project.platform)
    .bind(&project.title)
    .bind(&project.note)
    .bind(&project.cover)
    .bind(&project.timeline)
    .bind(&project.profile)
    .bind(project.status)
    .bind(&project.bvid)
    .bind(&project.created_at)
    .bind(&project.updated_at)
}

impl Database {
    pub async fn get_projects(&self) -> Result<Vec<ProjectRow>, DatabaseError> {
        Ok(
            sqlx::query_as::<_, ProjectRow>("SELECT * FROM projects ORDER BY updated_at DESC;")
                .fetch_all(&self.pool)
                .await?,
        )
    }

    pub async fn get_project(&self, id: i64) -> Result<ProjectRow, DatabaseError> {
        Ok(
            sqlx::query_as::<_, ProjectRow>("SELECT * FROM projects WHERE id = $1")
                .bind(id)
                .fetch_one(&self.pool)
                .await?,
        )
    }

    pub async fn add_project(&self, project: &ProjectRow) -> Result<ProjectRow, DatabaseError> {
        let result = insert_project(project).execute(&self.pool).await?;
        self.notify_change(DataChange::Projects);
        Ok(ProjectRow {
            id: result.last_insert_rowid(),
            ..project.clone()
        })
    }

    /// Give a video from before projects its project, the record of its upload
    /// when it was posted, and mark it migrated, all at once: a failure or crash
    /// must not create a second project for it on the next start.
    pub async fn add_legacy_project(
        &self,
        video_id: i64,
        project: &ProjectRow,
        upload: Option<UploadRow>,
    ) -> Result<ProjectRow, DatabaseError> {
        let mut transaction = self.pool.begin().await?;
        let id = insert_project(project)
            .execute(&mut *transaction)
            .await?
            .last_insert_rowid();
        if let Some(upload) = upload {
            let upload = UploadRow {
                source_id: id,
                ..upload
            };
            insert_upload(&upload).execute(&mut *transaction).await?;
        }
        sqlx::query("UPDATE videos SET needs_project = 0 WHERE id = $1")
            .bind(video_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        self.notify_change(DataChange::Projects);
        self.notify_change(DataChange::Uploads);
        Ok(ProjectRow {
            id,
            ..project.clone()
        })
    }

    // Each update writes only its own columns: a whole-row write from a stale
    // read would undo a concurrent change to another column, such as the
    // editor saving the timeline while the title is saved.

    /// Save the title, the note or both; `None` keeps a column as it is.
    pub async fn update_project_info(
        &self,
        id: i64,
        title: Option<&str>,
        note: Option<&str>,
        updated_at: &str,
    ) -> Result<(), DatabaseError> {
        sqlx::query(
            "UPDATE projects SET title = COALESCE($1, title), note = COALESCE($2, note), updated_at = $3 WHERE id = $4",
        )
            .bind(title)
            .bind(note)
            .bind(updated_at)
            .bind(id)
            .execute(&self.pool)
            .await?;
        self.notify_change(DataChange::Projects);
        Ok(())
    }

    /// Replace the timeline only if it is still `expected`. False when it
    /// changed since it was read.
    pub async fn replace_project_timeline(
        &self,
        id: i64,
        expected: &str,
        timeline: &str,
        updated_at: &str,
    ) -> Result<bool, DatabaseError> {
        let replaced = sqlx::query(
            "UPDATE projects SET timeline = $1, updated_at = $2 WHERE id = $3 AND timeline = $4",
        )
        .bind(timeline)
        .bind(updated_at)
        .bind(id)
        .bind(expected)
        .execute(&self.pool)
        .await?
        .rows_affected()
            == 1;
        if replaced {
            self.notify_change(DataChange::Projects);
        }
        Ok(replaced)
    }

    pub async fn set_project_cover(&self, id: i64, cover: &str) -> Result<(), DatabaseError> {
        sqlx::query("UPDATE projects SET cover = $1 WHERE id = $2")
            .bind(cover)
            .bind(id)
            .execute(&self.pool)
            .await?;
        self.notify_change(DataChange::Projects);
        Ok(())
    }

    pub async fn mark_project_posted(&self, id: i64, bvid: &str) -> Result<(), DatabaseError> {
        sqlx::query("UPDATE projects SET status = 1, bvid = $1 WHERE id = $2")
            .bind(bvid)
            .bind(id)
            .execute(&self.pool)
            .await?;
        self.notify_change(DataChange::Projects);
        Ok(())
    }

    pub async fn delete_project(&self, id: i64) -> Result<(), DatabaseError> {
        sqlx::query("DELETE FROM projects WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        self.notify_change(DataChange::Projects);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::credentials::CredentialCipher;
    use crate::events::EventBus;

    async fn database() -> Database {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        for migration in crate::get_migrations() {
            sqlx::raw_sql(migration.sql).execute(&pool).await.unwrap();
        }
        Database::new(
            pool,
            CredentialCipher::new(&[0; 32]).unwrap(),
            EventBus::new(),
        )
    }

    fn project() -> ProjectRow {
        ProjectRow {
            id: 0,
            room_id: "1".into(),
            platform: "bilibili".into(),
            title: "old title".into(),
            note: String::new(),
            cover: String::new(),
            timeline: "old timeline".into(),
            profile: String::new(),
            status: 0,
            bvid: String::new(),
            created_at: "2026-10-18".into(),
            updated_at: "2026-10-18".into(),
        }
    }

    #[tokio::test]
    async fn saving_one_part_of_a_project_keeps_the_others() {
        let db = database().await;
        let id = db.add_project(&project()).await.unwrap().id;

        // The editor saves the timeline and title while the list saves a note.
        assert!(db
            .replace_project_timeline(id, "old timeline", "new timeline", "t1")
            .await
            .unwrap());
        db.update_project_info(id, Some("new title"), None, "t2")
            .await
            .unwrap();
        db.update_project_info(id, None, Some("note"), "t3")
            .await
            .unwrap();
        db.mark_project_posted(id, "BV1xx").await.unwrap();

        let saved = db.get_project(id).await.unwrap();
        assert_eq!(
            (
                saved.title.as_str(),
                saved.note.as_str(),
                saved.timeline.as_str()
            ),
            ("new title", "note", "new timeline")
        );
        assert_eq!((saved.status, saved.bvid.as_str()), (1, "BV1xx"));
    }

    #[tokio::test]
    async fn a_timeline_edit_from_a_stale_read_is_refused() {
        let db = database().await;
        let id = db.add_project(&project()).await.unwrap().id;
        assert!(db
            .replace_project_timeline(id, "old timeline", "saved by the editor", "t1")
            .await
            .unwrap());

        assert!(!db
            .replace_project_timeline(id, "old timeline", "edited", "t2")
            .await
            .unwrap());
        assert!(db
            .replace_project_timeline(id, "saved by the editor", "edited", "t2")
            .await
            .unwrap());
        assert_eq!(db.get_project(id).await.unwrap().timeline, "edited");
    }
}
