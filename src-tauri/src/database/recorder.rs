use super::Database;
use super::DatabaseError;
use crate::recorder::PlatformType;
use chrono::Utc;
/// Recorder in database is pretty simple
/// because many room infos are collected in realtime
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct RecorderRow {
    pub room_id: i64,
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
        room_id: i64,
        extra: &str,
    ) -> Result<RecorderRow, DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        let recorder = RecorderRow {
            room_id,
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
        .execute(&lock)
        .await?;
        Ok(recorder)
    }

    pub async fn remove_recorder(&self, room_id: i64) -> Result<RecorderRow, DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        let recorder =
            sqlx::query_as::<_, RecorderRow>("SELECT * FROM recorders WHERE room_id = $1")
                .bind(room_id)
                .fetch_one(&lock)
                .await?;
        let sql = sqlx::query("DELETE FROM recorders WHERE room_id = $1")
            .bind(room_id)
            .execute(&lock)
            .await?;
        if sql.rows_affected() != 1 {
            return Err(DatabaseError::NotFound);
        }

        // remove related archive
        let _ = self.remove_archive(room_id).await;
        Ok(recorder)
    }

    pub async fn get_recorders(&self) -> Result<Vec<RecorderRow>, DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        Ok(sqlx::query_as::<_, RecorderRow>(
            "SELECT room_id, created_at, platform, auto_start, extra FROM recorders",
        )
        .fetch_all(&lock)
        .await?)
    }

    pub async fn remove_archive(&self, room_id: i64) -> Result<(), DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        let _ = sqlx::query("DELETE FROM records WHERE room_id = $1")
            .bind(room_id)
            .execute(&lock)
            .await?;
        Ok(())
    }

    pub async fn update_recorder(
        &self,
        platform: PlatformType,
        room_id: i64,
        auto_start: bool,
    ) -> Result<(), DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        let _ = sqlx::query(
            "UPDATE recorders SET auto_start = $1 WHERE platform = $2 AND room_id = $3",
        )
        .bind(auto_start)
        .bind(platform.as_str().to_string())
        .bind(room_id)
        .execute(&lock)
        .await?;
        Ok(())
    }
}
