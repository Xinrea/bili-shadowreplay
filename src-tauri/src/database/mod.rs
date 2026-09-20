use sqlx::SqlitePool;
use thiserror::Error;

pub mod account;
pub mod bilibili_user;
#[cfg_attr(
    not(feature = "credential-encryption"),
    path = "credentials_plaintext.rs"
)]
pub mod credentials;
pub mod message;
pub mod record;
pub mod recorder;
pub mod summary;
pub mod task;
pub mod video;

pub struct Database {
    pool: SqlitePool,
    credentials: credentials::CredentialCipher,
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
    #[error("Account credential storage error: {0}")]
    Credentials(#[from] std::io::Error),
    #[error("Database contains encrypted accounts; enable the credential-encryption feature")]
    EncryptionRequired,
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
    pub fn new(pool: SqlitePool, credentials: credentials::CredentialCipher) -> Database {
        Database { pool, credentials }
    }
}
