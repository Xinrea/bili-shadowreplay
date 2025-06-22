use custom_error::custom_error;
use sqlx::Pool;
use sqlx::Sqlite;
use tokio::sync::RwLock;

pub mod account;
pub mod message;
pub mod record;
pub mod recorder;
pub mod task;
pub mod video;

pub struct Database {
    db: RwLock<Option<Pool<Sqlite>>>,
}

custom_error! { pub DatabaseError
    InsertError = "Entry insert failed",
    NotFoundError = "Entry not found",
    InvalidCookiesError = "Cookies are invalid",
    DBError {err: sqlx::Error } = "DB error: {err}",
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
}
