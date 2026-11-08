use super::Database;
use super::DatabaseError;
use crate::events::DataChange;
use chrono::Utc;
use recorder::platforms::PlatformType;
/// Recorder in database is pretty simple
/// because many room infos are collected in realtime
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct RecorderRow {
    pub room_id: String,
    pub created_at: String,
    pub platform: String,
    pub auto_start: bool,
    pub extra: String,
}

// recorders
impl Database {
    pub async fn add_recorder(
        &self,
        platform: PlatformType,
        room_id: &str,
        extra: &str,
    ) -> Result<RecorderRow, DatabaseError> {
        let recorder = RecorderRow {
            room_id: room_id.to_string(),
            created_at: Utc::now().to_rfc3339(),
            platform: platform.as_str().to_string(),
            auto_start: true,
            extra: extra.to_string(),
        };
        let _ = sqlx::query(
            "INSERT OR REPLACE INTO recorders (room_id, created_at, platform, auto_start, extra) VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(room_id)
        .bind(&recorder.created_at)
        .bind(platform.as_str())
        .bind(recorder.auto_start)
        .bind(extra)
        .execute(&self.pool)
        .await?;
        self.notify_change(DataChange::Recorders);
        Ok(recorder)
    }

    pub async fn remove_recorder(
        &self,
        platform: &str,
        room_id: &str,
    ) -> Result<RecorderRow, DatabaseError> {
        let recorder = sqlx::query_as::<_, RecorderRow>(
            "SELECT * FROM recorders WHERE platform = $1 AND room_id = $2",
        )
        .bind(platform)
        .bind(room_id)
        .fetch_one(&self.pool)
        .await?;
        let sql = sqlx::query("DELETE FROM recorders WHERE platform = $1 AND room_id = $2")
            .bind(platform)
            .bind(room_id)
            .execute(&self.pool)
            .await?;
        if sql.rows_affected() != 1 {
            return Err(DatabaseError::NotFound);
        }

        // Remove this platform's archive index only. The same room number on
        // another platform is a different stream and must keep its history.
        let _ = self.remove_archive(platform, room_id).await;
        self.notify_change(DataChange::Recorders);
        Ok(recorder)
    }

    pub async fn get_recorders(&self) -> Result<Vec<RecorderRow>, DatabaseError> {
        Ok(sqlx::query_as::<_, RecorderRow>(
            "SELECT room_id, created_at, platform, auto_start, extra FROM recorders",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn remove_archive(&self, platform: &str, room_id: &str) -> Result<(), DatabaseError> {
        let _ = sqlx::query("DELETE FROM records WHERE platform = $1 AND room_id = $2")
            .bind(platform)
            .bind(room_id)
            .execute(&self.pool)
            .await?;
        self.notify_change(DataChange::Records);
        Ok(())
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
    use recorder::platforms::PlatformType;

    async fn migrated_db() -> Database {
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
            crate::database::credentials::CredentialCipher::new(&[0; 32]).unwrap(),
            crate::events::EventBus::new(),
        )
    }

    #[tokio::test]
    async fn same_room_number_on_two_platforms_is_removed_independently() {
        let db = migrated_db().await;
        db.add_recorder(PlatformType::BiliBili, "6", "")
            .await
            .unwrap();
        db.add_recorder(PlatformType::Douyu, "6", "").await.unwrap();
        db.add_record(
            PlatformType::BiliBili,
            "bili-parent",
            "bili-live",
            "6",
            "bili",
            None,
        )
        .await
        .unwrap();
        db.add_record(
            PlatformType::Douyu,
            "douyu-parent",
            "douyu-live",
            "6",
            "douyu",
            None,
        )
        .await
        .unwrap();

        let removed = db.remove_recorder("bilibili", "6").await.unwrap();
        assert_eq!(removed.platform, "bilibili");

        let recorders = db.get_recorders().await.unwrap();
        assert_eq!(recorders.len(), 1);
        assert_eq!(recorders[0].platform, "douyu");
        assert_eq!(recorders[0].room_id, "6");

        let remaining = db.get_room_records("douyu", "6", 0, 10).await.unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].live_id, "douyu-live");
        assert!(db
            .get_room_records("bilibili", "6", 0, 10)
            .await
            .unwrap()
            .is_empty());
    }
}
