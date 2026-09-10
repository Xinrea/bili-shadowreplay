use sqlx::Pool;
use sqlx::Sqlite;
use thiserror::Error;
use tokio::sync::RwLock;

pub mod account;
pub mod bilibili_user;
pub mod message;
pub mod record;
pub mod recorder;
pub mod summary;
pub mod task;
pub mod video;

pub struct Database {
    db: RwLock<Option<Pool<Sqlite>>>,
}

#[derive(Error, Debug)]
pub enum DatabaseError {
    #[error("Entry insert failed")]
    Insert,
    #[error("Entry not found")]
    NotFound,
    #[error("Cookies are invalid")]
    InvalidCookies,
    #[error("Number exceed i64 range")]
    NumberExceedI64Range,
    #[error("Database has not been initialized")]
    NotInitialized,
    #[error("DB error: {0}")]
    DB(#[from] sqlx::Error),
    #[error("SQL is incorret: {sql}")]
    Sql { sql: String },
}

impl From<DatabaseError> for String {
    fn from(err: DatabaseError) -> Self {
        err.to_string()
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

    /// Borrow the connection pool.
    ///
    /// Every query goes through this helper so a query issued before
    /// [`Database::set`] returns a typed [`DatabaseError::NotInitialized`]
    /// instead of panicking.
    async fn pool(&self) -> Result<Pool<Sqlite>, DatabaseError> {
        self.db
            .read()
            .await
            .clone()
            .ok_or(DatabaseError::NotInitialized)
    }
}
