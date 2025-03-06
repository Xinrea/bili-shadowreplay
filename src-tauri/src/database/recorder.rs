use super::Database;
use super::DatabaseError;
use chrono::Utc;

/// Recorder in database is pretty simple
/// because many room infos are collected in realtime
#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct RecorderRow {
    pub room_id: u64,
    pub created_at: String,
}

// recorders
impl Database {
    pub async fn add_recorder(&self, room_id: u64) -> Result<RecorderRow, DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        let recorder = RecorderRow {
            room_id,
            created_at: Utc::now().to_rfc3339(),
        };
        let _ = sqlx::query("INSERT INTO recorders (room_id, created_at) VALUES ($1, $2)")
            .bind(room_id as i64)
            .bind(&recorder.created_at)
            .execute(&lock)
            .await?;
        Ok(recorder)
    }

    pub async fn remove_recorder(&self, room_id: u64) -> Result<(), DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        let sql = sqlx::query("DELETE FROM recorders WHERE room_id = $1")
            .bind(room_id as i64)
            .execute(&lock)
            .await?;
        if sql.rows_affected() != 1 {
            return Err(DatabaseError::NotFoundError);
        }

        // remove related archive
        let _ = self.remove_archive(room_id).await;
        Ok(())
    }

    pub async fn get_recorders(&self) -> Result<Vec<RecorderRow>, DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        Ok(sqlx::query_as::<_, RecorderRow>("SELECT * FROM recorders")
            .fetch_all(&lock)
            .await?)
    }

    pub async fn remove_archive(&self, room_id: u64) -> Result<(), DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        let _ = sqlx::query("DELETE FROM records WHERE room_id = $1")
            .bind(room_id as i64)
            .execute(&lock)
            .await?;
        Ok(())
    }
}
