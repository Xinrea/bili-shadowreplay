use sqlx::query::Query;
use sqlx::sqlite::SqliteArguments;
use sqlx::Sqlite;

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
    /// 4:3 cover for the homepage feed, relative to the output directory;
    /// empty to let Bilibili crop `cover`.
    pub cover43: String,
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

/// Uploads that can still be edited and posted.
const EDITABLE: &str = "status IN ('draft', 'failed')";
/// Uploads being exported or posted.
const ACTIVE: &str = "status IN ('exporting', 'posting')";

pub(super) fn insert_upload(upload: &UploadRow) -> Query<'_, Sqlite, SqliteArguments<'_>> {
    sqlx::query(
        "INSERT INTO uploads (source, source_id, file, room_id, uid, cover, cover43, profile, danmu, status, message, bvid, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
    )
    .bind(&upload.source)
    .bind(upload.source_id)
    .bind(&upload.file)
    .bind(&upload.room_id)
    .bind(&upload.uid)
    .bind(&upload.cover)
    .bind(&upload.cover43)
    .bind(&upload.profile)
    .bind(upload.danmu)
    .bind(&upload.status)
    .bind(&upload.message)
    .bind(&upload.bvid)
    .bind(&upload.created_at)
    .bind(&upload.updated_at)
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
        let result = insert_upload(upload).execute(&self.pool).await?;
        self.notify_change(DataChange::Uploads);
        Ok(UploadRow {
            id: result.last_insert_rowid(),
            ..upload.clone()
        })
    }

    // Each update writes only its own columns. Edits of a draft only apply
    // while it is still a draft or failed, and its status only moves through
    // the posting job, so a save in flight cannot undo a claim to post it.

    /// Run `query`, which updates one upload, and tell whether it applied.
    async fn apply_upload_update<'q>(
        &self,
        query: Query<'q, Sqlite, SqliteArguments<'q>>,
    ) -> Result<bool, DatabaseError> {
        let applied = query.execute(&self.pool).await?.rows_affected() == 1;
        if applied {
            self.notify_change(DataChange::Uploads);
        }
        Ok(applied)
    }

    /// Save a draft's account and form over `base`, the account and form the
    /// edit started from. False when it is no longer editable, or when another
    /// editor saved it since.
    pub async fn update_upload_form(
        &self,
        id: i64,
        (uid, profile): (&str, &str),
        (base_uid, base_profile): (&str, &str),
        updated_at: &str,
    ) -> Result<bool, DatabaseError> {
        self.apply_upload_update(
            sqlx::query(&format!(
                "UPDATE uploads SET uid = $1, profile = $2, updated_at = $3
                 WHERE id = $4 AND uid = $5 AND profile = $6 AND {EDITABLE}"
            ))
            .bind(uid)
            .bind(profile)
            .bind(updated_at)
            .bind(id)
            .bind(base_uid)
            .bind(base_profile),
        )
        .await
    }

    /// False when the upload is no longer editable.
    pub async fn set_upload_cover(
        &self,
        id: i64,
        cover: &str,
        cover43: &str,
        updated_at: &str,
    ) -> Result<bool, DatabaseError> {
        self.apply_upload_update(
            sqlx::query(&format!(
                "UPDATE uploads SET cover = $1, cover43 = $2, updated_at = $3 WHERE id = $4 AND {EDITABLE}"
            ))
            .bind(cover)
            .bind(cover43)
            .bind(updated_at)
            .bind(id),
        )
        .await
    }

    /// False when the upload is no longer editable.
    pub async fn set_upload_danmu(
        &self,
        id: i64,
        danmu: bool,
        updated_at: &str,
    ) -> Result<bool, DatabaseError> {
        self.apply_upload_update(
            sqlx::query(&format!(
                "UPDATE uploads SET danmu = $1, updated_at = $2 WHERE id = $3 AND {EDITABLE}"
            ))
            .bind(danmu)
            .bind(updated_at)
            .bind(id),
        )
        .await
    }

    /// Move a running upload between `exporting` and `posting`.
    pub async fn set_upload_progress(
        &self,
        id: i64,
        status: &str,
        updated_at: &str,
    ) -> Result<(), DatabaseError> {
        self.apply_upload_update(
            sqlx::query(&format!(
                "UPDATE uploads SET status = $1, updated_at = $2 WHERE id = $3 AND {ACTIVE}"
            ))
            .bind(status)
            .bind(updated_at)
            .bind(id),
        )
        .await?;
        Ok(())
    }

    /// Record a running upload's submission with the form Bilibili accepted.
    pub async fn mark_upload_posted(
        &self,
        id: i64,
        bvid: &str,
        profile: &str,
        updated_at: &str,
    ) -> Result<(), DatabaseError> {
        self.apply_upload_update(
            sqlx::query(&format!(
                "UPDATE uploads SET status = 'posted', message = '', bvid = $1, profile = $2, updated_at = $3 WHERE id = $4 AND {ACTIVE}"
            ))
            .bind(bvid)
            .bind(profile)
            .bind(updated_at)
            .bind(id),
        )
        .await?;
        Ok(())
    }

    /// Leave a running upload failed with `message`, ready to be retried.
    pub async fn mark_upload_failed(
        &self,
        id: i64,
        message: &str,
        updated_at: &str,
    ) -> Result<(), DatabaseError> {
        self.apply_upload_update(
            sqlx::query(&format!(
                "UPDATE uploads SET status = 'failed', message = $1, updated_at = $2 WHERE id = $3 AND {ACTIVE}"
            ))
            .bind(message)
            .bind(updated_at)
            .bind(id),
        )
        .await?;
        Ok(())
    }

    /// Mark a draft or failed upload as posting, unless another request did so
    /// first. True when this call claimed it.
    pub async fn claim_upload(&self, id: i64, updated_at: &str) -> Result<bool, DatabaseError> {
        self.apply_upload_update(
            sqlx::query(&format!(
                "UPDATE uploads SET status = 'posting', message = '', updated_at = $1 WHERE id = $2 AND {EDITABLE}"
            ))
            .bind(updated_at)
            .bind(id),
        )
        .await
    }

    /// False while the upload is running, when it is kept.
    pub async fn delete_upload(&self, id: i64) -> Result<bool, DatabaseError> {
        self.apply_upload_update(
            sqlx::query(&format!(
                "DELETE FROM uploads WHERE id = $1 AND NOT {ACTIVE}"
            ))
            .bind(id),
        )
        .await
    }

    /// Point a `file` source to where the file has moved.
    #[cfg_attr(not(feature = "gui"), allow(dead_code))]
    pub async fn set_upload_file(&self, id: i64, file: &str) -> Result<bool, DatabaseError> {
        self.apply_upload_update(
            sqlx::query("UPDATE uploads SET file = $1 WHERE id = $2 AND source = 'file'")
                .bind(file)
                .bind(id),
        )
        .await
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
            cover43: String::new(),
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
    async fn only_one_request_claims_an_upload_for_posting() {
        let (db, after) = database_before_uploads().await;
        for sql in after {
            sqlx::raw_sql(sql).execute(&db.pool).await.unwrap();
        }
        let unposted = db.add_upload(&draft("draft")).await.unwrap();
        let posted = db.add_upload(&draft("posted")).await.unwrap();

        assert!(db.claim_upload(unposted.id, "later").await.unwrap());
        assert!(!db.claim_upload(unposted.id, "later").await.unwrap());
        assert!(!db.claim_upload(posted.id, "later").await.unwrap());
        let claimed = db.get_upload(unposted.id).await.unwrap();
        assert_eq!(
            (claimed.status.as_str(), claimed.updated_at.as_str()),
            ("posting", "later")
        );
    }

    #[tokio::test]
    async fn saves_keep_each_others_columns_and_cannot_undo_a_claim() {
        let (db, after) = database_before_uploads().await;
        for sql in after {
            sqlx::raw_sql(sql).execute(&db.pool).await.unwrap();
        }
        let id = db.add_upload(&draft("draft")).await.unwrap().id;

        // A debounced form save landing after the danmaku switch keeps it.
        assert!(db.set_upload_danmu(id, true, "t1").await.unwrap());
        assert!(db
            .update_upload_form(id, ("2", "{\"title\":\"t\"}"), ("1", "{}"), "t2")
            .await
            .unwrap());
        let saved = db.get_upload(id).await.unwrap();
        assert_eq!((saved.danmu, saved.uid.as_str()), (true, "2"));

        // A save from a form loaded before that one is refused, not applied
        // over it.
        assert!(!db
            .update_upload_form(id, ("3", "{}"), ("1", "{}"), "t2")
            .await
            .unwrap());
        let saved = db.get_upload(id).await.unwrap();
        assert_eq!(
            (saved.uid.as_str(), saved.profile.as_str()),
            ("2", "{\"title\":\"t\"}")
        );

        // Both covers are saved together, without touching the form.
        assert!(db
            .set_upload_cover(id, "uploads/1/c.jpg", "uploads/1/c43.jpg", "t2")
            .await
            .unwrap());
        let saved = db.get_upload(id).await.unwrap();
        assert_eq!(
            (
                saved.cover.as_str(),
                saved.cover43.as_str(),
                saved.uid.as_str()
            ),
            ("uploads/1/c.jpg", "uploads/1/c43.jpg", "2")
        );

        // Once claimed, late saves and deletion are refused and it stays posting.
        assert!(db.claim_upload(id, "t3").await.unwrap());
        assert!(!db
            .update_upload_form(id, ("3", "{}"), ("2", "{\"title\":\"t\"}"), "t4")
            .await
            .unwrap());
        assert!(!db.set_upload_danmu(id, false, "t4").await.unwrap());
        assert!(!db
            .set_upload_cover(id, "c.jpg", "c43.jpg", "t4")
            .await
            .unwrap());
        assert!(!db.delete_upload(id).await.unwrap());
        let running = db.get_upload(id).await.unwrap();
        assert_eq!(
            (running.status.as_str(), running.uid.as_str(), running.danmu),
            ("posting", "2", true)
        );

        db.mark_upload_posted(id, "BV1xx", "{}", "t5")
            .await
            .unwrap();
        let posted = db.get_upload(id).await.unwrap();
        assert_eq!(
            (posted.status.as_str(), posted.bvid.as_str()),
            ("posted", "BV1xx")
        );
        assert!(db.delete_upload(id).await.unwrap());
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
