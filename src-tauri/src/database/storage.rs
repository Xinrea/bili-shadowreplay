//! Storage identity metadata is separate from record rows and never contains
//! backend credentials. Bindings deliberately have no foreign key: RecordStart
//! can pin the recorder's snapshot before the archive row is created.
use super::{Database, DatabaseError};
use crate::config::{RecordingBackendConfig, StorageConfig, DEFAULT_RECORDING_STORAGE_ID};

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
            Self::write_storage_config(&mut tx, seed, true).await?;
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
        tx.commit().await?;
        Ok(StorageConfig {
            default_backend,
            backends: definitions
                .into_iter()
                .map(|(id, kind, root)| (id, RecordingBackendConfig { kind, root }))
                .collect(),
        })
    }

    pub async fn legacy_storage_id(&self) -> Result<String, DatabaseError> {
        Ok(sqlx::query_scalar(
            "SELECT legacy_backend FROM recording_storage_configuration WHERE singleton = 1",
        )
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn replace_storage_config(&self, next: &StorageConfig) -> Result<(), DatabaseError> {
        self.ensure_storage_schema().await?;
        let mut tx = self.pool.begin().await?;
        Self::write_storage_config(&mut tx, next, false).await?;
        tx.commit().await?;
        self.notify_change(crate::events::DataChange::Records);
        self.notify_change(crate::events::DataChange::Recorders);
        Ok(())
    }

    async fn write_storage_config(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        next: &StorageConfig,
        importing: bool,
    ) -> Result<(), DatabaseError> {
        if !next.backends.contains_key(&next.default_backend) {
            return Err(DatabaseError::InvalidStorageConfig(
                "default backend must reference a configured backend".into(),
            ));
        }
        for (id, definition) in &next.backends {
            let previous: Option<(String, String, bool)> = sqlx::query_as(
                "SELECT kind, location, EXISTS(SELECT 1 FROM recording_storage_enabled e \
                 WHERE e.storage_id = b.storage_id) FROM recording_storage_backends b WHERE storage_id = ?",
            ).bind(id).fetch_optional(&mut **tx).await?;
            if previous.is_some_and(|(kind, root, enabled)| {
                kind != definition.kind || root != definition.root || (!importing && !enabled)
            }) {
                return Err(DatabaseError::StorageBackendConflict(id.clone()));
            }
            sqlx::query("INSERT OR IGNORE INTO recording_storage_backends (storage_id, kind, location) VALUES (?, ?, ?)")
                .bind(id).bind(&definition.kind).bind(&definition.root).execute(&mut **tx).await?;
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
                sqlx::raw_sql(DELETION_SCHEMA_SQL)
                    .execute(&self.pool)
                    .await?;
                sqlx::raw_sql(super::storage_transfer::SCHEMA_SQL)
                    .execute(&self.pool)
                    .await?;
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
