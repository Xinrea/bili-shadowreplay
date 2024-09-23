use chrono::DateTime;
use chrono::Utc;
use custom_error::custom_error;
use sqlx::Pool;
use sqlx::Row;
use sqlx::Sqlite;
use sqlx::Value;
use sqlx::ValueRef;
use std::str::FromStr;
use tokio::sync::RwLock;

pub struct Database {
    db: RwLock<Option<Pool<Sqlite>>>,
}

/// Recorder in database is pretty simple
/// because many room infos are collected in realtime
#[derive(Debug, Clone, serde::Serialize)]
pub struct RecorderRow {
    pub room_id: u64,
    pub created_at: DateTime<Utc>,
}

custom_error! { pub DatabaseError
    InsertError = "Entry insert failed",
    NotFoundError = "Entry not found",
    DBError {err: sqlx::Error } = "DB error",
    SQLError { sql: String } = "SQL is incorret: {sql}"
}

impl From<DatabaseError> for String {
    fn from(value: DatabaseError) -> Self {
        value.to_string()
    }
}

impl From<sqlx::Error> for DatabaseError {
    fn from(value: sqlx::Error) -> Self {
        DatabaseError::DBError { err: value }
    }
}

impl Database {
    pub fn new() -> Database {
        Database {
            db: RwLock::new(None),
        }
    }

    /// db *must* be set in tauri setup
    pub async fn set(&self, p: Pool<Sqlite>) {
        *self.db.write().await = Some(p);
    }

    // recorders
    pub async fn add_recorder(&self, room_id: u64) -> Result<RecorderRow, DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        let recorder = RecorderRow {
            room_id,
            created_at: Utc::now(),
        };
        let sql = sqlx::query("INSERT INTO recorders (room_id, created_at) VALUES ($1, $2)")
            .bind(room_id as i64)
            .bind(recorder.created_at.to_rfc3339())
            .execute(&lock)
            .await?;
        if sql.rows_affected() != 1 {
            return Err(DatabaseError::InsertError);
        }
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
        Ok(())
    }

    pub async fn get_recorders(&self) -> Vec<RecorderRow> {
        let lock = self.db.read().await.clone().unwrap();
        let query = sqlx::query("SELECT * FROM recorders");
        let mut ret = Vec::new();
        if let Ok(rows) = query.fetch_all(&lock).await {
            for r in rows {
                let room_id = r
                    .try_get_raw(0)
                    .unwrap()
                    .to_owned()
                    .try_decode::<u64>()
                    .unwrap();
                let time = r
                    .try_get_raw(1)
                    .unwrap()
                    .to_owned()
                    .try_decode::<String>()
                    .unwrap();
                ret.push(RecorderRow {
                    room_id,
                    created_at: DateTime::from_str(&time).unwrap(),
                })
            }
        }
        ret
    }
}
