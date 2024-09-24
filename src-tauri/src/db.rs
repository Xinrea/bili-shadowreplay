use chrono::Utc;
use custom_error::custom_error;
use sqlx::Pool;
use sqlx::Sqlite;
use tokio::sync::RwLock;

pub struct Database {
    db: RwLock<Option<Pool<Sqlite>>>,
}

/// Recorder in database is pretty simple
/// because many room infos are collected in realtime
#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct RecorderRow {
    pub room_id: u64,
    pub created_at: String,
}

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct AccountRow {
    pub uid: u64,
    pub name: String,
    pub avatar: String,
    pub csrf: String,
    pub cookies: String,
    pub created_at: String,
}

custom_error! { pub DatabaseError
    InsertError = "Entry insert failed",
    NotFoundError = "Entry not found",
    InvalidCookiesError = "Cookies are invalid",
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
        Ok(())
    }

    pub async fn get_recorders(&self) -> Result<Vec<RecorderRow>, DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        Ok(sqlx::query_as::<_, RecorderRow>("SELECT * FROM recorders")
            .fetch_all(&lock)
            .await?)
    }

    // accounts
    pub async fn add_account(&self, cookies: &str) -> Result<AccountRow, DatabaseError> {
        todo!("add new account")
    }

    pub async fn remove_account(&self, uid: u64) -> Result<(), DatabaseError> {
        todo!("remove target account")
    }

    // CREATE TABLE accounts (uid INTEGER PRIMARY KEY, name TEXT, avatar TEXT, csrf TEXT, cookies TEXT, created_at TEXT);
    pub async fn get_accounts(&self) -> Result<Vec<AccountRow>, DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        Ok(sqlx::query_as::<_, AccountRow>("SELECT * FROM accounts")
            .fetch_all(&lock)
            .await?)
    }
}
