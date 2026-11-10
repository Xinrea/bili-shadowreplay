//! Storage identity metadata is separate from record rows and never contains
//! backend credentials. Bindings deliberately have no foreign key: RecordStart
//! can pin the recorder's snapshot before the archive row is created.
use super::{Database, DatabaseError};
use crate::config::{RecordingBackendConfig, StorageConfig, DEFAULT_RECORDING_STORAGE_ID};
use recording_storage::S3Credentials;
use std::collections::HashMap;

pub const SCHEMA_SQL: &str = "
    CREATE TABLE IF NOT EXISTS recording_storage_bindings (
        platform TEXT NOT NULL,
        room_id TEXT NOT NULL,
        live_id TEXT NOT NULL,
        storage_id TEXT NOT NULL CHECK(length(storage_id) > 0),
        PRIMARY KEY (platform, room_id, live_id)
    );
    CREATE TABLE IF NOT EXISTS recording_storage_backends (
        storage_id TEXT PRIMARY KEY NOT NULL,
        kind TEXT NOT NULL,
        location TEXT NOT NULL
    );
";

pub const DELETION_SCHEMA_SQL: &str = "
    CREATE TABLE IF NOT EXISTS recording_storage_deletions (
        platform TEXT NOT NULL,
        room_id TEXT NOT NULL,
        live_id TEXT NOT NULL,
        PRIMARY KEY (platform, room_id, live_id)
    );
";

pub const CONFIG_SCHEMA_SQL: &str = "
    CREATE TABLE IF NOT EXISTS recording_storage_configuration (
        singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
        default_backend TEXT NOT NULL,
        legacy_backend TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS recording_storage_enabled (
        storage_id TEXT PRIMARY KEY NOT NULL
    );
    CREATE VIEW IF NOT EXISTS recording_records AS
        SELECT r.*,
            COALESCE(binding.storage_id, config.legacy_backend, 'default') AS storage_backend_id,
            (enabled.storage_id IS NOT NULL AND backend.kind = 'local') AS storage_available,
            CASE WHEN enabled.storage_id IS NULL THEN 'Storage backend is removed or unconfigured'
                 WHEN backend.kind != 'local' THEN 'Storage backend type is unavailable'
                 ELSE NULL END AS storage_unavailable_reason
        FROM records r
        LEFT JOIN recording_storage_bindings binding
          ON binding.platform = r.platform AND binding.room_id = r.room_id AND binding.live_id = r.live_id
        LEFT JOIN recording_storage_configuration config ON config.singleton = 1
        LEFT JOIN recording_storage_enabled enabled
          ON enabled.storage_id = COALESCE(binding.storage_id, config.legacy_backend, 'default')
        LEFT JOIN recording_storage_backends backend ON backend.storage_id = enabled.storage_id;
";

// Keep CONFIG_SCHEMA_SQL unchanged: it is the checksum of released migration 28.
pub const S3_SCHEMA_SQL: &str = "
    DROP VIEW IF EXISTS recording_records;
    CREATE VIEW recording_records AS
        SELECT r.*,
            COALESCE(binding.storage_id, config.legacy_backend, 'default') AS storage_backend_id,
            (enabled.storage_id IS NOT NULL AND backend.kind IN ('local', 's3')) AS storage_available,
            CASE WHEN enabled.storage_id IS NULL THEN 'Storage backend is removed or unconfigured'
                 WHEN backend.kind NOT IN ('local', 's3') THEN 'Storage backend type is unavailable'
                 ELSE NULL END AS storage_unavailable_reason
        FROM records r
        LEFT JOIN recording_storage_bindings binding
          ON binding.platform = r.platform AND binding.room_id = r.room_id AND binding.live_id = r.live_id
        LEFT JOIN recording_storage_configuration config ON config.singleton = 1
        LEFT JOIN recording_storage_enabled enabled
          ON enabled.storage_id = COALESCE(binding.storage_id, config.legacy_backend, 'default')
        LEFT JOIN recording_storage_backends backend ON backend.storage_id = enabled.storage_id;
";

pub const CREDENTIALS_SCHEMA_SQL: &str = "
    CREATE TABLE IF NOT EXISTS recording_storage_credentials (
        storage_id TEXT PRIMARY KEY NOT NULL,
        credentials TEXT NOT NULL,
        encrypted INTEGER NOT NULL,
        revision TEXT NOT NULL
    );
    UPDATE recording_storage_backends
       SET location = json_remove(location, '$.credentials_env')
     WHERE kind = 's3' AND json_valid(location)
       AND json_type(location, '$.credentials_env') IS NOT NULL;
";

/// Called before the database service exists, including when upgrading an old DB.
pub(super) async fn encrypted_storage_credentials(
    pool: &sqlx::SqlitePool,
) -> Result<Vec<String>, sqlx::Error> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'recording_storage_credentials')",
    ).fetch_one(pool).await?;
    if !exists {
        return Ok(Vec::new());
    }
    sqlx::query_scalar("SELECT credentials FROM recording_storage_credentials WHERE encrypted = 1")
        .fetch_all(pool)
        .await
}

impl Database {
    /// The singleton is also the durable import marker. Never import TOML again,
    /// even when every legacy backend has since been removed.
    pub async fn initialize_storage_config(
        &self,
        seed: &StorageConfig,
    ) -> Result<(), DatabaseError> {
        self.ensure_storage_schema().await?;
        let mut tx = self.pool.begin().await?;
        let imported = sqlx::query(
            "INSERT OR IGNORE INTO recording_storage_configuration \
             (singleton, default_backend, legacy_backend) VALUES (1, ?, ?)",
        )
        .bind(&seed.default_backend)
        .bind(
            if seed.backends.contains_key(DEFAULT_RECORDING_STORAGE_ID) {
                DEFAULT_RECORDING_STORAGE_ID
            } else {
                &seed.default_backend
            },
        )
        .execute(&mut *tx)
        .await?
        .rows_affected()
            != 0;
        if imported {
            Self::write_storage_config(&mut tx, seed, true, false).await?;
            sqlx::query(
                "INSERT OR IGNORE INTO recording_storage_bindings (platform, room_id, live_id, storage_id) \
                 SELECT platform, room_id, live_id, \
                 (SELECT legacy_backend FROM recording_storage_configuration WHERE singleton = 1) FROM records",
            ).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn storage_config(&self) -> Result<StorageConfig, DatabaseError> {
        self.ensure_storage_schema().await?;
        let mut tx = self.pool.begin().await?;
        let default_backend = sqlx::query_scalar(
            "SELECT default_backend FROM recording_storage_configuration WHERE singleton = 1",
        )
        .fetch_one(&mut *tx)
        .await?;
        let definitions: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT b.storage_id, b.kind, b.location FROM recording_storage_backends b \
             JOIN recording_storage_enabled e ON e.storage_id = b.storage_id",
        )
        .fetch_all(&mut *tx)
        .await?;
        let credential_revisions = sqlx::query_as::<_, (String, String)>(
            "SELECT c.storage_id, c.revision FROM recording_storage_credentials c \
             JOIN recording_storage_enabled e ON e.storage_id = c.storage_id",
        )
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .collect();
        tx.commit().await?;
        Ok(StorageConfig {
            default_backend,
            credential_revisions,
            backends: definitions
                .into_iter()
                .map(|(id, kind, location)| {
                    RecordingBackendConfig::from_location(kind, location)
                        .map(|definition| (id, definition))
                        .map_err(|error| DatabaseError::InvalidStorageConfig(error.to_string()))
                })
                .collect::<Result<_, _>>()?,
        })
    }

    pub async fn legacy_storage_id(&self) -> Result<String, DatabaseError> {
        Ok(sqlx::query_scalar(
            "SELECT legacy_backend FROM recording_storage_configuration WHERE singleton = 1",
        )
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn replace_storage_config(
        &self,
        next: &StorageConfig,
        credentials: &HashMap<String, S3Credentials>,
    ) -> Result<(), DatabaseError> {
        self.ensure_storage_schema().await?;
        let mut tx = self.pool.begin().await?;
        let existing: std::collections::HashSet<String> =
            sqlx::query_scalar("SELECT storage_id FROM recording_storage_enabled")
                .fetch_all(&mut *tx)
                .await?
                .into_iter()
                .collect();
        let catalog_mode = self.catalogs.get().is_some();
        if catalog_mode {
            for id in existing
                .iter()
                .filter(|id| !next.backends.contains_key(*id))
            {
                let unimported: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM recording_records r WHERE storage_backend_id=? \
                     AND NOT EXISTS(SELECT 1 FROM catalog_record_links l WHERE l.live_id=r.live_id))",
                ).bind(id).fetch_one(&mut *tx).await?;
                if unimported {
                    return Err(DatabaseError::Catalog(
                        "旧录播迁移尚未完成，请恢复存储连接后再卸载".into(),
                    ));
                }
                let lives: Vec<String> = sqlx::query_scalar(
                    "SELECT r.live_id FROM recording_records r WHERE storage_backend_id=?",
                )
                .bind(id)
                .fetch_all(&mut *tx)
                .await?;
                for live in lives {
                    super::catalog::runtime::remove_cache(&mut tx, &live).await?;
                }
                sqlx::query("INSERT OR IGNORE INTO recording_storage_deletions(platform,room_id,live_id) \
                    SELECT platform,room_id,live_id FROM recording_storage_bindings WHERE storage_id=?")
                    .bind(id).execute(&mut *tx).await?;
                sqlx::query("DELETE FROM recording_storage_bindings WHERE storage_id=?")
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
                sqlx::query("DELETE FROM catalog_mounts WHERE alias=?")
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
                sqlx::query("DELETE FROM catalog_runtime_backends WHERE alias=?")
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
                sqlx::query("DELETE FROM recording_storage_backends WHERE storage_id=?")
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        Self::write_storage_config(&mut tx, next, false, catalog_mode).await?;
        for (id, credential) in credentials {
            if next
                .backends
                .get(id)
                .is_none_or(|definition| definition.kind != "s3")
            {
                return Err(DatabaseError::InvalidStorageConfig(
                    "credentials require a configured S3 backend".into(),
                ));
            }
            credential.validate()?;
            let serialized = serde_json::to_string(credential).map_err(std::io::Error::other)?;
            let encoded = self.credentials.encrypt(&serialized)?;
            sqlx::query(
                "INSERT INTO recording_storage_credentials (storage_id, credentials, encrypted, revision) VALUES (?, ?, ?, ?) \
                 ON CONFLICT(storage_id) DO UPDATE SET credentials = excluded.credentials, encrypted = excluded.encrypted, revision = excluded.revision",
            ).bind(id).bind(encoded).bind(super::credentials::CredentialCipher::ENCRYPTED)
                .bind(uuid::Uuid::new_v4().to_string()).execute(&mut *tx).await?;
        }
        for (id, definition) in &next.backends {
            if definition.kind == "s3" {
                let saved: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM recording_storage_credentials WHERE storage_id = ?)",
                ).bind(id).fetch_one(&mut *tx).await?;
                // Legacy/bootstrap definitions can be repaired one at a time in
                // settings. Never introduce a new S3 backend without credentials.
                if !saved && !existing.contains(id) {
                    return Err(DatabaseError::InvalidStorageConfig(format!(
                        "S3 backend {id} requires credentials"
                    )));
                }
            }
        }
        sqlx::query(
            "DELETE FROM recording_storage_credentials WHERE storage_id NOT IN (SELECT storage_id FROM recording_storage_enabled)",
        ).execute(&mut *tx).await?;
        tx.commit().await?;
        self.notify_change(crate::events::DataChange::Records);
        self.notify_change(crate::events::DataChange::Recorders);
        Ok(())
    }

    pub async fn storage_credentials(&self, id: &str) -> Result<S3Credentials, DatabaseError> {
        self.ensure_storage_schema().await?;
        let (value, encrypted): (String, bool) = sqlx::query_as(
            "SELECT c.credentials, c.encrypted FROM recording_storage_credentials c \
             JOIN recording_storage_enabled e ON e.storage_id = c.storage_id WHERE c.storage_id = ?",
        ).bind(id).fetch_optional(&self.pool).await?
            .ok_or_else(|| DatabaseError::InvalidStorageConfig(format!("S3 backend {id} requires credentials")))?;
        if encrypted && !super::credentials::CredentialCipher::ENCRYPTED {
            return Err(DatabaseError::EncryptionRequired);
        }
        let value = if encrypted {
            self.credentials.decrypt(&value)?
        } else {
            value
        };
        let credentials: S3Credentials = serde_json::from_str(&value).map_err(|_| {
            DatabaseError::InvalidStorageConfig("invalid saved S3 credentials".into())
        })?;
        credentials.validate()?;
        Ok(credentials)
    }

    async fn write_storage_config(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        next: &StorageConfig,
        importing: bool,
        catalog_mode: bool,
    ) -> Result<(), DatabaseError> {
        if !next.backends.contains_key(&next.default_backend) {
            return Err(DatabaseError::InvalidStorageConfig(
                "default backend must reference a configured backend".into(),
            ));
        }
        for (id, definition) in &next.backends {
            let location = definition
                .location()
                .map_err(|error| DatabaseError::InvalidStorageConfig(error.to_string()))?;
            let previous: Option<(String, String, bool)> = sqlx::query_as(
                "SELECT kind, location, EXISTS(SELECT 1 FROM recording_storage_enabled e \
                 WHERE e.storage_id = b.storage_id) FROM recording_storage_backends b WHERE storage_id = ?",
            ).bind(id).fetch_optional(&mut **tx).await?;
            if previous.is_some_and(|(kind, root, enabled)| {
                kind != definition.kind
                    || root != location
                    || (!importing && !enabled && !catalog_mode)
            }) {
                return Err(DatabaseError::StorageBackendConflict(id.clone()));
            }
            sqlx::query("INSERT OR IGNORE INTO recording_storage_backends (storage_id, kind, location) VALUES (?, ?, ?)")
                .bind(id).bind(&definition.kind).bind(&location).execute(&mut **tx).await?;
        }
        sqlx::query("DELETE FROM recording_storage_enabled")
            .execute(&mut **tx)
            .await?;
        for id in next.backends.keys() {
            sqlx::query("INSERT INTO recording_storage_enabled (storage_id) VALUES (?)")
                .bind(id)
                .execute(&mut **tx)
                .await?;
        }
        sqlx::query(
            "UPDATE recording_storage_configuration SET default_backend = ? WHERE singleton = 1",
        )
        .bind(&next.default_backend)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub(super) async fn ensure_storage_schema(&self) -> Result<(), DatabaseError> {
        self.storage_initialized
            .get_or_try_init(|| async {
                sqlx::raw_sql(SCHEMA_SQL).execute(&self.pool).await?;
                sqlx::raw_sql(CONFIG_SCHEMA_SQL).execute(&self.pool).await?;
                let mut tx = self.pool.begin().await?;
                sqlx::query(S3_SCHEMA_SQL).execute(&mut *tx).await?;
                sqlx::query(CREDENTIALS_SCHEMA_SQL)
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await?;
                sqlx::raw_sql(DELETION_SCHEMA_SQL)
                    .execute(&self.pool)
                    .await?;
                sqlx::raw_sql(super::storage_transfer::SCHEMA_SQL)
                    .execute(&self.pool)
                    .await?;
                self.ensure_object_transfer_schema().await?;
                if self.catalogs.get().is_some() {
                    sqlx::raw_sql(super::catalog::runtime::VIEW_SQL)
                        .execute(&self.pool)
                        .await?;
                }
                Ok::<_, DatabaseError>(())
            })
            .await?;
        Ok(())
    }

    pub async fn recording_storage_id(
        &self,
        platform: &str,
        room_id: &str,
        live_id: &str,
    ) -> Result<Option<String>, DatabaseError> {
        self.ensure_storage_schema().await?;
        Ok(sqlx::query_scalar(
            "SELECT storage_id FROM recording_storage_bindings \
             WHERE platform = ? AND room_id = ? AND live_id = ?",
        )
        .bind(platform)
        .bind(room_id)
        .bind(live_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Idempotent for the same ID, but never overwrites a previous assignment.
    pub async fn bind_recording_storage(
        &self,
        platform: &str,
        room_id: &str,
        live_id: &str,
        storage_id: &str,
    ) -> Result<(), DatabaseError> {
        self.ensure_storage_schema().await?;
        let result: Option<String> = sqlx::query_scalar(
            "INSERT INTO recording_storage_bindings (platform, room_id, live_id, storage_id) \
             SELECT ?1, ?2, ?3, ?4 WHERE NOT EXISTS ( \
                 SELECT 1 FROM recording_storage_deletions \
                 WHERE platform = ?1 AND room_id = ?2 AND live_id = ?3) \
             ON CONFLICT (platform, room_id, live_id) DO UPDATE SET storage_id = excluded.storage_id \
             WHERE recording_storage_bindings.storage_id = excluded.storage_id \
             RETURNING storage_id",
        )
        .bind(platform)
        .bind(room_id)
        .bind(live_id)
        .bind(storage_id)
        .fetch_optional(&self.pool)
        .await?;
        if result.is_none() {
            let deleted: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM recording_storage_deletions \
                 WHERE platform = ? AND room_id = ? AND live_id = ?)",
            )
            .bind(platform)
            .bind(room_id)
            .bind(live_id)
            .fetch_one(&self.pool)
            .await?;
            return Err(if deleted {
                DatabaseError::RecordingDeleted
            } else {
                DatabaseError::StorageBindingConflict
            });
        }
        Ok(())
    }

    /// A recorder has already pinned its store before publishing RecordStart.
    /// For a delayed resume event, an existing archive's binding is authoritative:
    /// a completed transfer may have moved it since the writer lease ended.
    pub async fn confirm_recording_start_storage(
        &self,
        platform: &str,
        room_id: &str,
        live_id: &str,
        storage_id: &str,
    ) -> Result<(), DatabaseError> {
        self.ensure_storage_schema().await?;
        let bound: Option<String> = sqlx::query_scalar(
            "INSERT INTO recording_storage_bindings (platform, room_id, live_id, storage_id) \
             SELECT ?1, ?2, ?3, ?4 WHERE NOT EXISTS (SELECT 1 FROM recording_storage_deletions \
             WHERE platform = ?1 AND room_id = ?2 AND live_id = ?3) \
             ON CONFLICT(platform, room_id, live_id) DO UPDATE \
             SET storage_id = recording_storage_bindings.storage_id \
             WHERE recording_storage_bindings.storage_id = excluded.storage_id \
             OR EXISTS(SELECT 1 FROM records WHERE platform = ?1 AND room_id = ?2 AND live_id = ?3) \
             RETURNING storage_id",
        )
        .bind(platform).bind(room_id).bind(live_id).bind(storage_id)
        .fetch_optional(&self.pool).await?;
        bound.ok_or(DatabaseError::StorageBindingConflict)?;
        Ok(())
    }

    /// Call only after assets and the archive row were successfully deleted.
    pub async fn forget_recording_storage(
        &self,
        platform: &str,
        room_id: &str,
        live_id: &str,
    ) -> Result<(), DatabaseError> {
        self.ensure_storage_schema().await?;
        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            "INSERT OR IGNORE INTO recording_storage_deletions (platform, room_id, live_id) VALUES (?, ?, ?)",
        ).bind(platform).bind(room_id).bind(live_id).execute(&mut *transaction).await?;
        sqlx::query(
            "DELETE FROM recording_storage_bindings \
             WHERE platform = ? AND room_id = ? AND live_id = ?",
        )
        .bind(platform)
        .bind(room_id)
        .bind(live_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn room_storage_ids(
        &self,
        platform: &str,
        room_id: &str,
    ) -> Result<Vec<String>, DatabaseError> {
        self.ensure_storage_schema().await?;
        Ok(sqlx::query_scalar(
            "SELECT DISTINCT storage_id FROM recording_storage_bindings \
             WHERE platform = ? AND room_id = ? ORDER BY storage_id",
        )
        .bind(platform)
        .bind(room_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn room_has_unbound_recordings(
        &self,
        platform: &str,
        room_id: &str,
    ) -> Result<bool, DatabaseError> {
        self.ensure_storage_schema().await?;
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM records AS r \
             WHERE r.platform = ? AND r.room_id = ? AND NOT EXISTS( \
                 SELECT 1 FROM recording_storage_bindings AS b \
                 WHERE b.platform = r.platform AND b.room_id = r.room_id AND b.live_id = r.live_id))",
        )
        .bind(platform)
        .bind(room_id)
        .fetch_one(&self.pool)
        .await?)
    }

    /// Read-only preflight for a proposed configuration. Schema migrations must
    /// have run first; unlike remember_storage_backend this never reserves IDs.
    pub async fn validate_storage_backend(
        &self,
        storage_id: &str,
        kind: &str,
        location: &str,
    ) -> Result<(), DatabaseError> {
        let previous: Option<(String, String)> = sqlx::query_as(
            "SELECT kind, location FROM recording_storage_backends WHERE storage_id = ?",
        )
        .bind(storage_id)
        .fetch_optional(&self.pool)
        .await?;
        if previous.is_some_and(|previous| previous.0 != kind || previous.1 != location) {
            return Err(DatabaseError::StorageBackendConflict(storage_id.to_owned()));
        }
        Ok(())
    }

    /// Persist local locations, not secrets. Retain definitions even after the
    /// last archive is deleted, so an ID cannot silently acquire a new meaning.
    pub async fn remember_storage_backend(
        &self,
        storage_id: &str,
        kind: &str,
        location: &str,
    ) -> Result<(), DatabaseError> {
        self.ensure_storage_schema().await?;
        let result: Option<String> = sqlx::query_scalar(
            "INSERT INTO recording_storage_backends (storage_id, kind, location) VALUES (?, ?, ?) \
             ON CONFLICT (storage_id) DO UPDATE SET storage_id = excluded.storage_id \
             WHERE recording_storage_backends.kind = excluded.kind \
             AND recording_storage_backends.location = excluded.location \
             RETURNING storage_id",
        )
        .bind(storage_id)
        .bind(kind)
        .bind(location)
        .fetch_optional(&self.pool)
        .await?;
        result.ok_or_else(|| DatabaseError::StorageBackendConflict(storage_id.to_owned()))?;
        Ok(())
    }
}
