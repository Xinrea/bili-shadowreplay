use super::Database;
use super::DatabaseError;
use crate::events::DataChange;

/// An upload (投稿): a Bilibili submission of a material, a project or any
/// video file, kept as a draft until posted and as history afterwards.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct UploadRow {
    pub id: i64,
    /// `material`, `project` or `file`.
    pub source: String,
    /// Material or project id; 0 for files.
    pub source_id: i64,
    /// Absolute path of a `file` source.
    pub file: String,
    pub room_id: String,
    /// Bilibili account that posts it.
    pub uid: String,
    /// Cover relative to the output directory; empty to use the source's.
    pub cover: String,
    /// Bilibili `Profile` JSON.
    pub profile: String,
    /// Burn a material's danmaku into the posted video. A project keeps this
    /// setting in its own timeline.
    pub danmu: bool,
    /// `draft`, `exporting`, `posting`, `posted` or `failed`.
    pub status: String,
    /// Why the last attempt failed.
    pub message: String,
    pub bvid: String,
    pub created_at: String,
    pub updated_at: String,
}

impl UploadRow {
    /// Exporting or posting right now.
    pub fn is_active(&self) -> bool {
        matches!(self.status.as_str(), "exporting" | "posting")
    }
}

impl Database {
    pub async fn get_uploads(&self) -> Result<Vec<UploadRow>, DatabaseError> {
        Ok(
            sqlx::query_as::<_, UploadRow>("SELECT * FROM uploads ORDER BY updated_at DESC;")
                .fetch_all(&self.pool)
                .await?,
        )
    }

    pub async fn get_upload(&self, id: i64) -> Result<UploadRow, DatabaseError> {
        Ok(
            sqlx::query_as::<_, UploadRow>("SELECT * FROM uploads WHERE id = $1")
                .bind(id)
                .fetch_one(&self.pool)
                .await?,
        )
    }

    pub async fn add_upload(&self, upload: &UploadRow) -> Result<UploadRow, DatabaseError> {
        let result = sqlx::query(
            "INSERT INTO uploads (source, source_id, file, room_id, uid, cover, profile, danmu, status, message, bvid, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
        )
        .bind(&upload.source)
        .bind(upload.source_id)
        .bind(&upload.file)
        .bind(&upload.room_id)
        .bind(&upload.uid)
        .bind(&upload.cover)
        .bind(&upload.profile)
        .bind(upload.danmu)
        .bind(&upload.status)
        .bind(&upload.message)
        .bind(&upload.bvid)
        .bind(&upload.created_at)
        .bind(&upload.updated_at)
        .execute(&self.pool)
        .await?;
        self.notify_change(DataChange::Uploads);
        Ok(UploadRow {
            id: result.last_insert_rowid(),
            ..upload.clone()
        })
    }

    /// Save every column except `id`, the source and `created_at`.
    pub async fn update_upload(&self, upload: &UploadRow) -> Result<(), DatabaseError> {
        sqlx::query(
            "UPDATE uploads SET room_id = $1, uid = $2, cover = $3, profile = $4, danmu = $5, status = $6, message = $7, bvid = $8, updated_at = $9 WHERE id = $10",
        )
        .bind(&upload.room_id)
        .bind(&upload.uid)
        .bind(&upload.cover)
        .bind(&upload.profile)
        .bind(upload.danmu)
        .bind(&upload.status)
        .bind(&upload.message)
        .bind(&upload.bvid)
        .bind(&upload.updated_at)
        .bind(upload.id)
        .execute(&self.pool)
        .await?;
        self.notify_change(DataChange::Uploads);
        Ok(())
    }

    pub async fn delete_upload(&self, id: i64) -> Result<(), DatabaseError> {
        sqlx::query("DELETE FROM uploads WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        self.notify_change(DataChange::Uploads);
        Ok(())
    }

    /// Uploads cannot survive a restart; leave them failed so they can be retried.
    pub async fn fail_interrupted_uploads(&self) -> Result<(), DatabaseError> {
        sqlx::query(
            "UPDATE uploads SET status = 'failed', message = '应用重启，投稿已中断' WHERE status IN ('exporting', 'posting')",
        )
        .execute(&self.pool)
        .await?;
        self.notify_change(DataChange::Uploads);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::credentials::CredentialCipher;
    use crate::events::EventBus;

    /// A database migrated up to uploads, and the SQL of the later migrations.
    async fn database_before_uploads() -> (Database, Vec<&'static str>) {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let (before, after): (Vec<_>, Vec<_>) = crate::get_migrations()
            .into_iter()
            .partition(|m| m.version < 19);
        for migration in before {
            sqlx::raw_sql(migration.sql).execute(&pool).await.unwrap();
        }
        let db = Database::new(
            pool,
            CredentialCipher::new(&[0; 32]).unwrap(),
            EventBus::new(),
        );
        (db, after.into_iter().map(|m| m.sql).collect())
    }

    fn draft(status: &str) -> UploadRow {
        UploadRow {
            id: 0,
            source: "file".into(),
            source_id: 0,
            file: "/videos/a.mp4".into(),
            room_id: String::new(),
            uid: "1".into(),
            cover: String::new(),
            profile: "{}".into(),
            danmu: false,
            status: status.into(),
            message: String::new(),
            bvid: String::new(),
            created_at: "2026-10-18T00:00:00+08:00".into(),
            updated_at: "2026-10-18T00:00:00+08:00".into(),
        }
    }

    #[tokio::test]
    async fn posted_projects_keep_their_submission_as_an_upload() {
        let (db, after) = database_before_uploads().await;
        sqlx::raw_sql(
            "INSERT INTO projects (room_id, title, profile, bvid, status, created_at, updated_at)
             VALUES ('1', 'Posted', '{\"title\":\"Posted\"}', 'BV1xx', 1, '2026-01-01', '2026-01-02'),
                    ('1', 'Draft', '', '', 0, '2026-01-01', '2026-01-03');",
        )
        .execute(&db.pool)
        .await
        .unwrap();
        for sql in after {
            sqlx::raw_sql(sql).execute(&db.pool).await.unwrap();
        }

        let uploads = db.get_uploads().await.unwrap();
        assert_eq!(uploads.len(), 1);
        let upload = &uploads[0];
        assert_eq!(
            (
                upload.source.as_str(),
                upload.source_id,
                upload.status.as_str()
            ),
            ("project", 1, "posted")
        );
        assert_eq!(
            (
                upload.bvid.as_str(),
                upload.profile.as_str(),
                upload.room_id.as_str()
            ),
            ("BV1xx", "{\"title\":\"Posted\"}", "1")
        );
    }

    #[tokio::test]
    async fn uploads_interrupted_by_a_restart_can_be_retried() {
        let (db, after) = database_before_uploads().await;
        for sql in after {
            sqlx::raw_sql(sql).execute(&db.pool).await.unwrap();
        }
        let mut ids = Vec::new();
        for status in ["draft", "exporting", "posting", "posted"] {
            ids.push(db.add_upload(&draft(status)).await.unwrap().id);
        }

        db.fail_interrupted_uploads().await.unwrap();

        let mut statuses = Vec::new();
        for id in ids {
            let upload = db.get_upload(id).await.unwrap();
            assert!(!upload.is_active());
            statuses.push(upload.status);
        }
        assert_eq!(statuses, ["draft", "failed", "failed", "posted"]);
    }
}
