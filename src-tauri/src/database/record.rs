use crate::recorder::PlatformType;

use super::Database;
use super::DatabaseError;
use chrono::Utc;

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct RecordRow {
    pub platform: String,
    pub live_id: String,
    pub room_id: u64,
    pub title: String,
    pub length: i64,
    pub size: i64,
    pub created_at: String,
    pub cover: Option<String>,
}

// CREATE TABLE records (live_id INTEGER PRIMARY KEY, room_id INTEGER, title TEXT, length INTEGER, size INTEGER, created_at TEXT);
impl Database {
    pub async fn get_records(&self, room_id: u64) -> Result<Vec<RecordRow>, DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        Ok(
            sqlx::query_as::<_, RecordRow>("SELECT * FROM records WHERE room_id = $1")
                .bind(room_id as i64)
                .fetch_all(&lock)
                .await?,
        )
    }

    pub async fn get_record(
        &self,
        room_id: u64,
        live_id: &str,
    ) -> Result<RecordRow, DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        Ok(sqlx::query_as::<_, RecordRow>(
            "SELECT * FROM records WHERE live_id = $1 and room_id = $2",
        )
        .bind(live_id)
        .bind(room_id as i64)
        .fetch_one(&lock)
        .await?)
    }

    pub async fn add_record(
        &self,
        platform: PlatformType,
        live_id: &str,
        room_id: u64,
        title: &str,
        cover: Option<String>,
        created_at: Option<&str>,
    ) -> Result<RecordRow, DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        let record = RecordRow {
            platform: platform.as_str().to_string(),
            live_id: live_id.to_string(),
            room_id,
            title: title.into(),
            length: 0,
            size: 0,
            created_at: created_at.unwrap_or(&Utc::now().to_rfc3339()).to_string(),
            cover,
        };
        if let Err(e) = sqlx::query("INSERT INTO records (live_id, room_id, title, length, size, cover, created_at, platform) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)").bind(record.live_id.clone())
            .bind(record.room_id as i64).bind(&record.title).bind(0).bind(0).bind(&record.cover).bind(&record.created_at).bind(platform.as_str().to_string()).execute(&lock).await {
                // if the record already exists, return the existing record
                if e.to_string().contains("UNIQUE constraint failed") {
                    return self.get_record(room_id, live_id).await;
                }
            }
        Ok(record)
    }

    pub async fn remove_record(&self, live_id: &str) -> Result<(), DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        sqlx::query("DELETE FROM records WHERE live_id = $1")
            .bind(live_id)
            .execute(&lock)
            .await?;
        Ok(())
    }

    pub async fn update_record(
        &self,
        live_id: &str,
        length: i64,
        size: u64,
    ) -> Result<(), DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        sqlx::query("UPDATE records SET length = $1, size = $2 WHERE live_id = $3")
            .bind(length)
            .bind(size as i64)
            .bind(live_id)
            .execute(&lock)
            .await?;
        Ok(())
    }

    pub async fn get_total_length(&self) -> Result<i64, DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        let result: (i64,) = sqlx::query_as("SELECT SUM(length) FROM records;")
            .fetch_one(&lock)
            .await?;
        Ok(result.0)
    }

    pub async fn get_today_record_count(&self) -> Result<i64, DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        let result: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM records WHERE created_at >= $1;")
            .bind(
                Utc::now()
                    .date_naive()
                    .and_hms_opt(0, 0, 0)
                    .unwrap()
                    .to_string(),
            )
            .fetch_one(&lock)
            .await?;
        Ok(result.0)
    }

    pub async fn get_recent_record(
        &self,
        offset: u64,
        limit: u64,
    ) -> Result<Vec<RecordRow>, DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        Ok(sqlx::query_as::<_, RecordRow>(
            "SELECT * FROM records ORDER BY created_at DESC LIMIT $1 OFFSET $2",
        )
        .bind(limit as i64)
        .bind(offset as i64)
        .fetch_all(&lock)
        .await?)
    }
}
