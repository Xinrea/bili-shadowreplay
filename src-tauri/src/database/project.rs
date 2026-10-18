use super::Database;
use super::DatabaseError;
use crate::events::DataChange;

/// A project (切片): an edit of one or more materials that is exported and
/// uploaded. `timeline` holds [`crate::project::Timeline`] JSON and `profile`
/// the Bilibili upload form.
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
    /// Latest export relative to the output directory, empty when never exported.
    pub output: String,
    /// [`crate::project::Timeline::render_hash`] of the exported timeline.
    pub output_hash: String,
    /// 0 = not posted, 1 = posted.
    pub status: i64,
    pub bvid: String,
    pub created_at: String,
    pub updated_at: String,
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
        let result = sqlx::query(
            "INSERT INTO projects (room_id, platform, title, note, cover, timeline, profile, output, output_hash, status, bvid, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
        )
        .bind(&project.room_id)
        .bind(&project.platform)
        .bind(&project.title)
        .bind(&project.note)
        .bind(&project.cover)
        .bind(&project.timeline)
        .bind(&project.profile)
        .bind(&project.output)
        .bind(&project.output_hash)
        .bind(project.status)
        .bind(&project.bvid)
        .bind(&project.created_at)
        .bind(&project.updated_at)
        .execute(&self.pool)
        .await?;
        self.notify_change(DataChange::Projects);
        Ok(ProjectRow {
            id: result.last_insert_rowid(),
            ..project.clone()
        })
    }

    /// Save every column except `id` and `created_at`.
    pub async fn update_project(&self, project: &ProjectRow) -> Result<(), DatabaseError> {
        sqlx::query(
            "UPDATE projects SET room_id = $1, platform = $2, title = $3, note = $4, cover = $5, timeline = $6, profile = $7, output = $8, output_hash = $9, status = $10, bvid = $11, updated_at = $12 WHERE id = $13",
        )
        .bind(&project.room_id)
        .bind(&project.platform)
        .bind(&project.title)
        .bind(&project.note)
        .bind(&project.cover)
        .bind(&project.timeline)
        .bind(&project.profile)
        .bind(&project.output)
        .bind(&project.output_hash)
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
