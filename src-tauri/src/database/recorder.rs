use super::Database;
use super::DatabaseError;
use crate::events::DataChange;
use chrono::Utc;
use recorder::platforms::PlatformType;

pub const STORAGE_BACKEND_MIGRATION_SQL: &str =
    "ALTER TABLE recorders ADD COLUMN storage_backend_id TEXT;";

/// Recorder in database is pretty simple
/// because many room infos are collected in realtime
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct RecorderRow {
    pub room_id: String,
    pub created_at: String,
    pub platform: String,
    pub auto_start: bool,
    pub extra: String,
    #[serde(default)]
    #[sqlx(default)]
    pub storage_backend_id: Option<String>,
}

// recorders
impl Database {
    pub async fn add_recorder(
        &self,
        platform: PlatformType,
        room_id: &str,
        extra: &str,
    ) -> Result<RecorderRow, DatabaseError> {
        self.add_recorder_with_storage(platform, room_id, extra, None)
            .await
    }

    pub async fn add_recorder_with_storage(
        &self,
        platform: PlatformType,
        room_id: &str,
        extra: &str,
        storage_backend_id: Option<&str>,
    ) -> Result<RecorderRow, DatabaseError> {
        let recorder = RecorderRow {
            room_id: room_id.to_string(),
            created_at: Utc::now().to_rfc3339(),
            platform: platform.as_str().to_string(),
            auto_start: true,
            extra: extra.to_string(),
            storage_backend_id: storage_backend_id.map(str::to_owned),
        };
        let _ = sqlx::query(
            "INSERT OR REPLACE INTO recorders (room_id, created_at, platform, auto_start, extra, storage_backend_id) VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(room_id)
        .bind(&recorder.created_at)
        .bind(platform.as_str())
        .bind(recorder.auto_start)
        .bind(extra)
        .bind(storage_backend_id)
        .execute(&self.pool)
        .await?;
        self.notify_change(DataChange::Recorders);
        Ok(recorder)
    }

    /// Delete a room only after its pinned storage assets were removed. Keep
    /// bindings until the caller explicitly forgets them after this transaction.
    pub async fn remove_recorder_for_platform(
        &self,
        platform: &str,
        room_id: &str,
    ) -> Result<RecorderRow, DatabaseError> {
        let mut transaction = self.pool.begin().await?;
        let recorder = sqlx::query_as::<_, RecorderRow>(
            "DELETE FROM recorders WHERE platform = ? AND room_id = ? RETURNING *",
        )
        .bind(platform)
        .bind(room_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(DatabaseError::NotFound)?;
        sqlx::query("DELETE FROM records WHERE platform = ? AND room_id = ?")
            .bind(platform)
            .bind(room_id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("DELETE FROM record_summaries WHERE platform = ? AND room_id = ?")
            .bind(platform)
            .bind(room_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        self.notify_change(DataChange::Records);
        self.notify_change(DataChange::Recorders);
        Ok(recorder)
    }

    pub async fn get_recorders(&self) -> Result<Vec<RecorderRow>, DatabaseError> {
        Ok(sqlx::query_as::<_, RecorderRow>("SELECT * FROM recorders")
            .fetch_all(&self.pool)
            .await?)
    }

    pub async fn has_recorder(&self, platform: &str, room_id: &str) -> Result<bool, DatabaseError> {
        Ok(sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM recorders WHERE platform = ? AND room_id = ?)",
        )
        .bind(platform)
        .bind(room_id)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn recorder_storage_id(
        &self,
        platform: &str,
        room_id: &str,
    ) -> Result<Option<String>, DatabaseError> {
        Ok(sqlx::query_scalar::<_, Option<String>>(
            "SELECT storage_backend_id FROM recorders WHERE platform = ? AND room_id = ?",
        )
        .bind(platform)
        .bind(room_id)
        .fetch_optional(&self.pool)
        .await?
        .flatten())
    }

    pub async fn set_recorder_storage(
        &self,
        platform: &str,
        room_id: &str,
        id: Option<&str>,
    ) -> Result<RecorderRow, DatabaseError> {
        let row = sqlx::query_as::<_, RecorderRow>(
            "UPDATE recorders SET storage_backend_id = ? WHERE platform = ? AND room_id = ? RETURNING *",
        )
        .bind(id)
        .bind(platform)
        .bind(room_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(DatabaseError::NotFound)?;
        self.notify_change(DataChange::Recorders);
        Ok(row)
    }

    pub async fn update_recorder(
        &self,
        platform: PlatformType,
        room_id: &str,
        auto_start: bool,
    ) -> Result<(), DatabaseError> {
        let _ = sqlx::query(
            "UPDATE recorders SET auto_start = $1 WHERE platform = $2 AND room_id = $3",
        )
        .bind(auto_start)
        .bind(platform.as_str().to_string())
        .bind(room_id)
        .execute(&self.pool)
        .await?;
        self.notify_change(DataChange::Recorders);
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

    #[tokio::test]
    async fn storage_selection_defaults_and_updates_are_scoped_to_exact_recorder() {
        let db = database().await;
        let inherited = db
            .add_recorder(PlatformType::BiliBili, "123", "")
            .await
            .unwrap();
        assert_eq!(inherited.storage_backend_id, None);
        let explicit = db
            .add_recorder_with_storage(PlatformType::Douyin, "456", "", Some("archive"))
            .await
            .unwrap();
        assert_eq!(explicit.storage_backend_id.as_deref(), Some("archive"));
        assert_eq!(
            db.recorder_storage_id("douyin", "456")
                .await
                .unwrap()
                .as_deref(),
            Some("archive")
        );
        assert_eq!(db.recorder_storage_id("douyin", "123").await.unwrap(), None);
        assert!(matches!(
            db.set_recorder_storage("douyin", "123", Some("wrong"))
                .await,
            Err(DatabaseError::NotFound)
        ));
        assert_eq!(
            db.recorder_storage_id("bilibili", "123").await.unwrap(),
            None
        );
        let updated = db
            .set_recorder_storage("bilibili", "123", Some("other"))
            .await
            .unwrap();
        assert_eq!(updated.storage_backend_id.as_deref(), Some("other"));
        let rows = db.get_recorders().await.unwrap();
        assert_eq!(
            rows.iter()
                .find(|row| row.room_id == "123")
                .unwrap()
                .storage_backend_id
                .as_deref(),
            Some("other")
        );
        assert_eq!(
            db.set_recorder_storage("bilibili", "123", None)
                .await
                .unwrap()
                .storage_backend_id,
            None
        );
    }

    #[tokio::test]
    async fn platform_room_removal_preserves_other_archives_and_bindings() {
        let db = database().await;
        db.add_recorder(PlatformType::BiliBili, "123", "")
            .await
            .unwrap();
        db.add_recorder(PlatformType::Douyin, "456", "")
            .await
            .unwrap();
        for (platform, room, live) in [
            (PlatformType::BiliBili, "123", "target"),
            (PlatformType::Douyin, "123", "other-platform"),
            (PlatformType::BiliBili, "456", "other-room"),
        ] {
            db.add_record(platform, live, live, room, "title", None)
                .await
                .unwrap();
            db.start_record_summary(platform.as_str(), room, live, "task", false)
                .await
                .unwrap();
            db.bind_recording_storage(platform.as_str(), room, live, "one")
                .await
                .unwrap();
        }
        assert!(matches!(
            db.remove_recorder_for_platform("douyin", "123").await,
            Err(DatabaseError::NotFound)
        ));
        let removed = db
            .remove_recorder_for_platform("bilibili", "123")
            .await
            .unwrap();
        assert_eq!(removed.platform, "bilibili");
        assert!(db
            .get_room_records("bilibili", "123", 0, 10)
            .await
            .unwrap()
            .is_empty());
        assert!(db
            .get_record_summary("bilibili", "123", "target")
            .await
            .unwrap()
            .is_none());
        assert_eq!(
            db.recording_storage_id("bilibili", "123", "target")
                .await
                .unwrap()
                .as_deref(),
            Some("one")
        );
        for (platform, room, live) in [
            ("douyin", "123", "other-platform"),
            ("bilibili", "456", "other-room"),
        ] {
            assert_eq!(
                db.get_room_records(platform, room, 0, 10)
                    .await
                    .unwrap()
                    .len(),
                1
            );
            assert!(db
                .get_record_summary(platform, room, live)
                .await
                .unwrap()
                .is_some());
        }
        assert_eq!(db.get_recorders().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn platform_room_removal_rolls_back_on_related_delete_error() {
        let db = database().await;
        db.add_recorder(PlatformType::BiliBili, "123", "")
            .await
            .unwrap();
        db.add_record(PlatformType::BiliBili, "live", "live", "123", "title", None)
            .await
            .unwrap();
        sqlx::query("DROP TABLE record_summaries")
            .execute(&db.pool)
            .await
            .unwrap();
        assert!(db
            .remove_recorder_for_platform("bilibili", "123")
            .await
            .is_err());
        assert_eq!(db.get_recorders().await.unwrap().len(), 1);
        assert_eq!(
            db.get_room_records("bilibili", "123", 0, 10)
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn same_room_number_on_two_platforms_is_kept_and_removed_independently() {
        let db = database().await;
        db.add_recorder(PlatformType::BiliBili, "6", "bili")
            .await
            .unwrap();
        db.add_recorder(PlatformType::Douyu, "6", "douyu")
            .await
            .unwrap();
        let rows = db.get_recorders().await.unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows
            .iter()
            .any(|row| row.platform == "bilibili" && row.extra == "bili"));
        assert!(rows
            .iter()
            .any(|row| row.platform == "douyu" && row.extra == "douyu"));

        db.add_record(
            PlatformType::BiliBili,
            "bili-live",
            "bili-live",
            "6",
            "title",
            None,
        )
        .await
        .unwrap();
        db.add_record(
            PlatformType::Douyu,
            "douyu-live",
            "douyu-live",
            "6",
            "title",
            None,
        )
        .await
        .unwrap();

        let removed = db
            .remove_recorder_for_platform("bilibili", "6")
            .await
            .unwrap();
        assert_eq!(removed.platform, "bilibili");
        let rows = db.get_recorders().await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].platform, "douyu");
        assert_eq!(rows[0].extra, "douyu");
        assert!(db
            .get_room_records("bilibili", "6", 0, 10)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            db.get_room_records("douyu", "6", 0, 10)
                .await
                .unwrap()
                .len(),
            1
        );
    }
}
