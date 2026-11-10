//! Explicit relocation journal. Ordinary recording binding remains immutable.
use super::{Database, DatabaseError};

pub const SCHEMA_SQL: &str = "
CREATE TABLE IF NOT EXISTS recording_storage_transfers (
    platform TEXT NOT NULL, room_id TEXT NOT NULL, live_id TEXT NOT NULL,
    source_id TEXT NOT NULL, target_id TEXT NOT NULL, token TEXT NOT NULL UNIQUE,
    manifest TEXT NOT NULL, phase TEXT NOT NULL CHECK(phase IN ('copying', 'cleanup', 'finalizing')),
    mode TEXT NOT NULL CHECK(mode IN ('copy', 'rename')),
    PRIMARY KEY(platform, room_id, live_id)
);";

// Migration 30 remains unchanged. SQLite CHECK constraints require a rebuild.
pub const OBJECT_SCHEMA_SQL: &str = "
CREATE TABLE recording_storage_transfers_objects (
    platform TEXT NOT NULL, room_id TEXT NOT NULL, live_id TEXT NOT NULL,
    source_id TEXT NOT NULL, target_id TEXT NOT NULL, token TEXT NOT NULL UNIQUE,
    manifest TEXT NOT NULL, phase TEXT NOT NULL CHECK(phase IN ('copying', 'cleanup', 'finalizing')),
    mode TEXT NOT NULL CHECK(mode IN ('copy', 'rename', 'object')),
    PRIMARY KEY(platform, room_id, live_id)
);
INSERT INTO recording_storage_transfers_objects
    SELECT platform, room_id, live_id, source_id, target_id, token, manifest, phase, mode
    FROM recording_storage_transfers;
DROP TABLE recording_storage_transfers;
ALTER TABLE recording_storage_transfers_objects RENAME TO recording_storage_transfers;
";

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct TransferJournal {
    pub platform: String,
    pub room_id: String,
    pub live_id: String,
    pub source_id: String,
    pub target_id: String,
    pub token: String,
    pub manifest: String,
    pub phase: String,
    pub mode: String,
}

impl TransferJournal {
    pub fn key(&self) -> String {
        format!("{}/{}/{}", self.platform, self.room_id, self.live_id)
    }
}

/// Preview deliberately excludes the potentially large per-file manifest.
#[derive(Debug, sqlx::FromRow)]
pub struct TransferEstimate {
    pub platform: String,
    pub room_id: String,
    pub live_id: String,
    pub size: i64,
    pub source_id: Option<String>,
    pub target_id: Option<String>,
    pub phase: Option<String>,
    pub mode: Option<String>,
}

impl Database {
    pub(super) async fn ensure_object_transfer_schema(&self) -> Result<(), DatabaseError> {
        let mut tx = self.pool.begin().await?;
        let schema: String = sqlx::query_scalar(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'recording_storage_transfers'",
        ).fetch_one(&mut *tx).await?;
        if !schema.contains("'object'") {
            sqlx::query(OBJECT_SCHEMA_SQL).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// An uncommitted rename may already have removed the source namespace.
    /// Query durable phase (not an in-memory guess after an unknown commit).
    pub async fn recording_awaits_rename_binding(
        &self,
        platform: &str,
        room_id: &str,
        live_id: &str,
    ) -> Result<bool, DatabaseError> {
        self.ensure_storage_schema().await?;
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM recording_storage_transfers \
             WHERE platform = ? AND room_id = ? AND live_id = ? AND mode = 'rename' AND phase = 'copying')",
        )
        .bind(platform)
        .bind(room_id)
        .bind(live_id)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn storage_transfer_keys(&self) -> Result<Vec<String>, DatabaseError> {
        self.ensure_storage_schema().await?;
        Ok(sqlx::query_scalar(
            "SELECT platform || '/' || room_id || '/' || live_id FROM recording_storage_transfers",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn storage_transfer_estimates(
        &self,
        source: &str,
        target: &str,
    ) -> Result<Vec<TransferEstimate>, DatabaseError> {
        self.ensure_storage_schema().await?;
        // The view includes pinned bindings and unbound legacy archives. Join
        // once per archive so a copying journal never doubles its record's size.
        // Committed journals remain visible after the binding moves to target,
        // even when the archive row is absent.
        Ok(sqlx::query_as(
            "SELECT r.platform, r.room_id, r.live_id, COALESCE(r.size, 0) AS size, \
                    j.source_id, j.target_id, j.phase, j.mode \
             FROM recording_records r LEFT JOIN recording_storage_transfers j \
               ON j.platform = r.platform AND j.room_id = r.room_id AND j.live_id = r.live_id \
             WHERE r.storage_backend_id = ?1 OR (j.source_id = ?1 AND j.target_id = ?2) \
             UNION ALL \
             SELECT j.platform, j.room_id, j.live_id, 0 AS size, j.source_id, j.target_id, j.phase, j.mode \
             FROM recording_storage_transfers j \
             WHERE j.source_id = ?1 AND j.target_id = ?2 \
               AND NOT EXISTS(SELECT 1 FROM recording_records r \
                 WHERE r.platform = j.platform AND r.room_id = j.room_id AND r.live_id = j.live_id)",
        )
        .bind(source)
        .bind(target)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn storage_transfer_journals(&self) -> Result<Vec<TransferJournal>, DatabaseError> {
        self.ensure_storage_schema().await?;
        Ok(sqlx::query_as(
            // Explicit projection avoids stale SELECT * column metadata on a
            // pooled SQLite connection after the additive mode-column upgrade.
            "SELECT platform, room_id, live_id, source_id, target_id, token, manifest, phase, mode \
             FROM recording_storage_transfers ORDER BY platform, room_id, live_id",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn storage_transfer_recordings(
        &self,
        source: &str,
    ) -> Result<Vec<(String, String, String)>, DatabaseError> {
        self.ensure_storage_schema().await?;
        Ok(sqlx::query_as(
            "SELECT platform, room_id, live_id FROM recording_records WHERE storage_backend_id = ? \
             ORDER BY platform, room_id, live_id",
        ).bind(source).fetch_all(&self.pool).await?)
    }

    pub async fn begin_storage_transfer(
        &self,
        journal: &TransferJournal,
    ) -> Result<(), DatabaseError> {
        if let Some(catalogs) = self.catalogs.get() {
            catalogs.preflight_transfer(journal).await?;
        }
        let mut tx = self.pool.begin().await?;
        // Pin legacy rows too, without weakening the normal immutable bind rule.
        sqlx::query(
            "INSERT OR IGNORE INTO recording_storage_bindings (platform, room_id, live_id, storage_id) \
             SELECT platform, room_id, live_id, storage_backend_id FROM recording_records \
             WHERE platform = ? AND room_id = ? AND live_id = ? AND storage_backend_id = ?",
        ).bind(&journal.platform).bind(&journal.room_id).bind(&journal.live_id)
            .bind(&journal.source_id).execute(&mut *tx).await?;
        let result = sqlx::query(
            "INSERT INTO recording_storage_transfers \
             (platform, room_id, live_id, source_id, target_id, token, manifest, phase, mode) \
             SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, 'copying', ?8 \
             WHERE EXISTS(SELECT 1 FROM recording_storage_bindings WHERE platform = ?1 AND room_id = ?2 \
             AND live_id = ?3 AND storage_id = ?4) \
             AND NOT EXISTS(SELECT 1 FROM recording_storage_deletions WHERE platform = ?1 AND room_id = ?2 AND live_id = ?3) \
             AND EXISTS(SELECT 1 FROM recording_storage_enabled WHERE storage_id = ?5)",
        ).bind(&journal.platform).bind(&journal.room_id).bind(&journal.live_id)
            .bind(&journal.source_id).bind(&journal.target_id).bind(&journal.token)
            .bind(&journal.manifest).bind(&journal.mode).execute(&mut *tx).await?;
        if result.rows_affected() != 1 {
            return Err(DatabaseError::StorageBindingConflict);
        }
        tx.commit().await?;
        Ok(())
    }

    /// Persist an EXDEV fallback before copying any payload. The old rename
    /// manifest remains durable until the replacement copy manifest commits.
    pub async fn fallback_storage_transfer(
        &self,
        journal: &TransferJournal,
        copy_manifest: &str,
    ) -> Result<(), DatabaseError> {
        let result = sqlx::query(
            "UPDATE recording_storage_transfers SET mode = 'copy', manifest = ? \
             WHERE token = ? AND phase = 'copying' AND mode = 'rename' AND manifest = ?",
        )
        .bind(copy_manifest)
        .bind(&journal.token)
        .bind(&journal.manifest)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() != 1 {
            return Err(DatabaseError::StorageBindingConflict);
        }
        Ok(())
    }

    /// Commit binding and cleanup obligation together, after publication.
    pub async fn commit_storage_transfer(
        &self,
        journal: &TransferJournal,
    ) -> Result<(), DatabaseError> {
        if let Some(catalogs) = self.catalogs.get() {
            return catalogs.commit_transfer(journal.clone()).await;
        }
        let mut tx = self.pool.begin().await?;
        commit_binding(&mut tx, journal).await?;
        tx.commit().await?;
        self.notify_change(crate::events::DataChange::Records);
        Ok(())
    }

    pub async fn finalize_storage_transfer(&self, token: &str) -> Result<(), DatabaseError> {
        let result = sqlx::query(
            "UPDATE recording_storage_transfers SET phase = 'finalizing' WHERE token = ? AND phase = 'cleanup'",
        ).bind(token).execute(&self.pool).await?;
        if result.rows_affected() != 1 {
            return Err(DatabaseError::StorageBindingConflict);
        }
        Ok(())
    }

    pub async fn finish_storage_transfer(&self, token: &str) -> Result<(), DatabaseError> {
        sqlx::query(
            "DELETE FROM recording_storage_transfers WHERE token = ? AND phase = 'finalizing'",
        )
        .bind(token)
        .execute(&self.pool)
        .await?;
        if self.catalogs.get().is_some() {
            sqlx::query(
                "DELETE FROM catalog_transfer_intents WHERE token=? \
                AND NOT EXISTS(SELECT 1 FROM recording_storage_transfers WHERE token=?)",
            )
            .bind(token)
            .bind(token)
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }
}

pub(super) async fn commit_binding(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    journal: &TransferJournal,
) -> Result<(), DatabaseError> {
    let result = sqlx::query(
            "UPDATE recording_storage_bindings SET storage_id = ?5 \
             WHERE platform = ?1 AND room_id = ?2 AND live_id = ?3 AND storage_id = ?4 \
             AND EXISTS(SELECT 1 FROM recording_storage_transfers WHERE token = ?6 AND phase = 'copying') \
             AND NOT EXISTS(SELECT 1 FROM recording_storage_deletions WHERE platform = ?1 AND room_id = ?2 AND live_id = ?3) \
             AND EXISTS(SELECT 1 FROM recording_storage_enabled WHERE storage_id = ?5)",
        ).bind(&journal.platform).bind(&journal.room_id).bind(&journal.live_id)
            .bind(&journal.source_id).bind(&journal.target_id).bind(&journal.token)
            .execute(&mut **tx).await?;
    if result.rows_affected() != 1 {
        return Err(DatabaseError::StorageBindingConflict);
    }
    sqlx::query("UPDATE recording_storage_transfers SET phase = 'cleanup' WHERE token = ?")
        .bind(&journal.token)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
