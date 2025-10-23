use recorder::platforms::PlatformType;

use super::Database;
use super::DatabaseError;
use chrono::Utc;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct RecordRow {
    pub platform: String,
    pub parent_id: String,
    pub live_id: String,
    pub room_id: i64,
    pub title: String,
    pub length: i64,
    pub size: i64,
    pub created_at: String,
    pub cover: Option<String>,
}

// CREATE TABLE records (live_id INTEGER PRIMARY KEY, room_id INTEGER, title TEXT, length INTEGER, size INTEGER, created_at TEXT);
impl Database {
    pub async fn get_records(
        &self,
        room_id: i64,
        offset: i64,
        limit: i64,
    ) -> Result<Vec<RecordRow>, DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        Ok(sqlx::query_as::<_, RecordRow>(
            "SELECT * FROM records WHERE room_id = $1 ORDER BY created_at DESC LIMIT $2 OFFSET $3",
        )
        .bind(room_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&lock)
        .await?)
    }

    pub async fn get_record(
        &self,
        room_id: i64,
        live_id: &str,
    ) -> Result<RecordRow, DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        Ok(sqlx::query_as::<_, RecordRow>(
            "SELECT * FROM records WHERE room_id = $1 and live_id = $2",
        )
        .bind(room_id)
        .bind(live_id)
        .fetch_one(&lock)
        .await?)
    }

    pub async fn get_archives_by_parent_id(
        &self,
        room_id: i64,
        parent_id: &str,
    ) -> Result<Vec<RecordRow>, DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        Ok(sqlx::query_as::<_, RecordRow>(
            "SELECT * FROM records WHERE room_id = $1 and parent_id = $2",
        )
        .bind(room_id)
        .bind(parent_id)
        .fetch_all(&lock)
        .await?)
    }

    pub async fn add_record(
        &self,
        platform: PlatformType,
        parent_id: &str,
        live_id: &str,
        room_id: i64,
        title: &str,
        cover: Option<String>,
    ) -> Result<RecordRow, DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        let record = RecordRow {
            platform: platform.as_str().to_string(),
            parent_id: parent_id.to_string(),
            live_id: live_id.to_string(),
            room_id,
            title: title.into(),
            length: 0,
            size: 0,
            created_at: Utc::now().to_rfc3339().to_string(),
            cover,
        };
        if let Err(e) = sqlx::query("INSERT INTO records (live_id, room_id, title, length, size, cover, created_at, platform, parent_id) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)").bind(record.live_id.clone())
            .bind(record.room_id).bind(&record.title).bind(0).bind(0).bind(&record.cover).bind(&record.created_at).bind(platform.as_str().to_string()).bind(parent_id).execute(&lock).await {
                // if the record already exists, return the existing record
                if e.to_string().contains("UNIQUE constraint failed") {
                    return self.get_record(room_id, live_id).await;
                }
            }
        Ok(record)
    }

    pub async fn remove_record(&self, live_id: &str) -> Result<RecordRow, DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        let to_delete = sqlx::query_as::<_, RecordRow>("SELECT * FROM records WHERE live_id = $1")
            .bind(live_id)
            .fetch_one(&lock)
            .await?;
        sqlx::query("DELETE FROM records WHERE live_id = $1")
            .bind(live_id)
            .execute(&lock)
            .await?;
        Ok(to_delete)
    }

    pub async fn update_record(
        &self,
        live_id: &str,
        length: i64,
        size: u64,
    ) -> Result<(), DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        let size = i64::try_from(size).map_err(|_| DatabaseError::NumberExceedI64Range)?;
        sqlx::query("UPDATE records SET length = $1, size = $2 WHERE live_id = $3")
            .bind(length)
            .bind(size)
            .bind(live_id)
            .execute(&lock)
            .await?;
        Ok(())
    }

    pub async fn update_record_parent_id(
        &self,
        live_id: &str,
        parent_id: &str,
    ) -> Result<(), DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        sqlx::query("UPDATE records SET parent_id = $1 WHERE live_id = $2")
            .bind(parent_id)
            .bind(live_id)
            .execute(&lock)
            .await?;
        Ok(())
    }

    pub async fn update_record_cover(
        &self,
        live_id: &str,
        cover: Option<String>,
    ) -> Result<(), DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        sqlx::query("UPDATE records SET cover = $1 WHERE live_id = $2")
            .bind(cover)
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
        room_id: i64,
        offset: i64,
        limit: i64,
    ) -> Result<Vec<RecordRow>, DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        if room_id == 0 {
            Ok(sqlx::query_as::<_, RecordRow>(
                "SELECT * FROM records ORDER BY created_at DESC LIMIT $1 OFFSET $2",
            )
            .bind(limit)
            .bind(offset)
            .fetch_all(&lock)
            .await?)
        } else {
            Ok(sqlx::query_as::<_, RecordRow>(
                "SELECT * FROM records WHERE room_id = $1 ORDER BY created_at DESC LIMIT $2 OFFSET $3",
            )
            .bind(room_id)
            .bind(limit)
            .bind(offset)
            .fetch_all(&lock)
            .await?)
        }
    }

    pub async fn get_record_disk_usage(&self) -> Result<i64, DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        let result: (i64,) = sqlx::query_as("SELECT SUM(size) FROM records;")
            .fetch_one(&lock)
            .await?;
        Ok(result.0)
    }
}
