use crate::events::{AppEvent, DataChange, EventBus};
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
pub mod project;
pub mod record;
pub mod recorder;
pub mod summary;
pub mod task;
pub mod upload;
pub mod video;

pub struct Database {
    pool: SqlitePool,
    credentials: credentials::CredentialCipher,
    events: EventBus,
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

#[cfg(test)]
mod event_tests;

impl Database {
    pub fn notify_change(&self, change: DataChange) {
        self.events.publish(AppEvent::DataChanged(change));
    }

    pub fn notify_coalesced_change(&self, change: DataChange) {
        self.events.publish_coalesced_change(change);
    }

    pub fn new(
        pool: SqlitePool,
        credentials: credentials::CredentialCipher,
        events: EventBus,
    ) -> Database {
        Database {
            pool,
            credentials,
            events,
        }
    }
}
