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

    /// Save every column except `id` and `created_at`.
    pub async fn update_project(&self, project: &ProjectRow) -> Result<(), DatabaseError> {
        sqlx::query(
            "UPDATE projects SET room_id = $1, platform = $2, title = $3, note = $4, cover = $5, timeline = $6, profile = $7, status = $8, bvid = $9, updated_at = $10 WHERE id = $11",
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
        .bind(&project.updated_at)
        .bind(project.id)
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
