use super::Database;
use super::DatabaseError;
use chrono::Utc;

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct RecordRow {
    pub live_id: u64,
    pub room_id: u64,
    pub title: String,
    pub length: i64,
    pub size: i64,
    pub created_at: String,
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

    pub async fn get_record(&self, room_id: u64, live_id: u64) -> Result<RecordRow, DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        Ok(sqlx::query_as::<_, RecordRow>(
            "SELECT * FROM records WHERE live_id = $1 and room_id = $2",
        )
        .bind(live_id as i64)
        .bind(room_id as i64)
        .fetch_one(&lock)
        .await?)
    }

    pub async fn add_record(
        &self,
        live_id: u64,
        room_id: u64,
        title: &str,
    ) -> Result<RecordRow, DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        let record = RecordRow {
            live_id,
            room_id,
            title: title.into(),
            length: 0,
            size: 0,
            created_at: Utc::now().to_rfc3339(),
        };
        if let Err(e) = sqlx::query("INSERT INTO records (live_id, room_id, title, length, size, created_at) VALUES ($1, $2, $3, $4, $5, $6)").bind(record.live_id as i64)
            .bind(record.room_id as i64).bind(&record.title).bind(0).bind(0).bind(&record.created_at).execute(&lock).await {
                // if the record already exists, return the existing record
                if e.to_string().contains("UNIQUE constraint failed") {
                    return self.get_record(room_id, live_id).await;
                }
            }
        Ok(record)
    }

    pub async fn remove_record(&self, live_id: u64) -> Result<(), DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        sqlx::query("DELETE FROM records WHERE live_id = $1")
            .bind(live_id as i64)
            .execute(&lock)
            .await?;
        Ok(())
    }

    pub async fn update_record(
        &self,
        live_id: u64,
        length: i64,
        size: u64,
    ) -> Result<(), DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        sqlx::query("UPDATE records SET length = $1, size = $2 WHERE live_id = $3")
            .bind(length)
            .bind(size as i64)
            .bind(live_id as i64)
            .execute(&lock)
            .await?;
        Ok(())
    }
}
