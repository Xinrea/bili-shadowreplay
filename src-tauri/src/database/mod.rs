use crate::events::{AppEvent, DataChange, EventBus};
use sqlx::SqlitePool;
use thiserror::Error;

pub mod account;
pub mod audio;
pub mod bilibili_user;
pub mod catalog;
#[cfg_attr(
    not(feature = "credential-encryption"),
    path = "credentials_plaintext.rs"
)]
pub mod credentials;
pub mod message;
pub mod project;
pub mod record;
pub mod recorder;
pub mod storage;
pub mod storage_transfer;
pub mod summary;
pub mod task;
pub mod upload;
pub mod video;

pub struct Database {
    pool: SqlitePool,
    credentials: credentials::CredentialCipher,
    events: EventBus,
    storage_initialized: tokio::sync::OnceCell<()>,
    pub(crate) catalogs: std::sync::OnceLock<std::sync::Arc<catalog::runtime::CatalogRuntime>>,
    pub(crate) catalog_startup_blocked: std::sync::atomic::AtomicBool,
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
    #[error("Credential storage error: {0}")]
    Credentials(#[from] std::io::Error),
    #[error("Database contains encrypted credentials; enable the credential-encryption feature")]
    EncryptionRequired,
    #[error("Recording is already bound to a different storage ID")]
    StorageBindingConflict,
    #[error("Recording was deleted and cannot be resumed")]
    RecordingDeleted,
    #[error("Storage ID {0} is reserved (removed or assigned to another backend); use a new ID")]
    StorageBackendConflict(String),
    #[error("Invalid storage configuration: {0}")]
    InvalidStorageConfig(String),
    #[error("Recording catalog error: {0}")]
    Catalog(String),
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
            storage_initialized: tokio::sync::OnceCell::new(),
            catalogs: std::sync::OnceLock::new(),
            catalog_startup_blocked: std::sync::atomic::AtomicBool::new(false),
        }
    }
}
