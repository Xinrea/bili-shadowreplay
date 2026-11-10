//! Storage-owned catalog migration/projection data layer.
//! Startup freezes recorder creation before taking legacy snapshots. Production
//! metadata mutations use `runtime`; existing SQLite recording tables are its
//! compatibility projection, not an ongoing source for re-import.
use super::{record::RecordRow, summary::RecordSummaryRow, Database, DatabaseError};
use recording_catalog::{
    CatalogEntry, CatalogRecord, CatalogScan, CatalogWriter, ImportDigest, ImportPlan,
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

pub mod projection;
pub mod runtime;

pub const SCHEMA_SQL: &str = "
    CREATE TABLE IF NOT EXISTS catalog_legacy_imports (
        source_id TEXT PRIMARY KEY NOT NULL,
        import_id TEXT UNIQUE NOT NULL,
        storage_uuid TEXT UNIQUE NOT NULL,
        plan TEXT NOT NULL,
        complete INTEGER NOT NULL DEFAULT 0 CHECK(complete IN (0, 1))
    );
    CREATE TABLE IF NOT EXISTS catalog_legacy_entries (
        import_id TEXT NOT NULL,
        ordinal INTEGER NOT NULL CHECK(ordinal >= 0),
        record TEXT NOT NULL,
        PRIMARY KEY(import_id, ordinal)
    );
    CREATE TABLE IF NOT EXISTS catalog_mounts (
        alias TEXT PRIMARY KEY NOT NULL CHECK(length(alias) > 0),
        storage_uuid TEXT UNIQUE NOT NULL,
        epoch TEXT UNIQUE NOT NULL,
        online INTEGER NOT NULL DEFAULT 0 CHECK(online IN (0, 1)),
        error TEXT
    );
    CREATE TABLE IF NOT EXISTS catalog_projection_heads (
        storage_uuid TEXT PRIMARY KEY NOT NULL,
        generation TEXT,
        revision INTEGER NOT NULL DEFAULT -1
    );
    CREATE TABLE IF NOT EXISTS catalog_projection_entries (
        storage_uuid TEXT NOT NULL,
        generation TEXT NOT NULL,
        recording_uuid TEXT NOT NULL,
        created_at_ms INTEGER NOT NULL,
        length REAL NOT NULL CHECK(length >= 0),
        size INTEGER NOT NULL CHECK(size >= 0),
        entry TEXT NOT NULL,
        PRIMARY KEY(storage_uuid, generation, recording_uuid)
    );
    CREATE INDEX IF NOT EXISTS catalog_projection_time
        ON catalog_projection_entries(created_at_ms DESC, storage_uuid, recording_uuid);
";

fn error(message: impl ToString) -> DatabaseError {
    DatabaseError::Catalog(message.to_string())
}

fn json<T: Serialize>(value: &T) -> Result<String, DatabaseError> {
    serde_json::to_string(value).map_err(error)
}

fn from_json<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, DatabaseError> {
    serde_json::from_str(value).map_err(error)
}

/// Versioned, portable results; intentionally no SQLite ID or running task ID.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchiveResults {
    pub version: u32,
    pub summary: Option<CompletedSummary>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompletedSummary {
    pub subtitle_srt: Option<String>,
    pub subtitle_text: Option<String>,
    pub summary_markdown: Option<String>,
    pub highlights_json: Option<String>,
    pub model_provider: Option<String>,
    pub model_name: Option<String>,
    pub prompt_version: i64,
    pub source_duration: Option<f64>,
    pub created_at: String,
    pub updated_at: String,
}

pub(super) fn legacy_record(
    row: RecordRow,
    summary: Option<RecordSummaryRow>,
) -> Result<CatalogRecord, DatabaseError> {
    let created_at_ms = chrono::DateTime::parse_from_rfc3339(&row.created_at)
        .map(|date| date.timestamp_millis())
        .or_else(|_| {
            chrono::NaiveDateTime::parse_from_str(&row.created_at, "%Y-%m-%d %H:%M:%S%.f")
                .map(|date| date.and_utc().timestamp_millis())
        })
        .map_err(|_| {
            error(format!(
                "Invalid legacy timestamp for {}/{}/{}",
                row.platform, row.room_id, row.live_id
            ))
        })?;
    for value in [&row.platform, &row.room_id, &row.live_id] {
        if value.contains('/') {
            return Err(error("Invalid legacy recording namespace"));
        }
        recording_storage::validate_key(value).map_err(error)?;
    }
    if summary.as_ref().is_some_and(|row| {
        row.status == "success"
            && row
                .source_duration
                .is_some_and(|value| !value.is_finite() || value < 0.0)
    }) {
        return Err(error("Invalid legacy summary duration"));
    }
    let results = ArchiveResults {
        version: 1,
        summary: summary
            .filter(|row| row.status == "success")
            .map(|row| CompletedSummary {
                subtitle_srt: row.subtitle_srt,
                subtitle_text: row.subtitle_text,
                summary_markdown: row.summary_markdown,
                highlights_json: row.highlights_json,
                model_provider: row.model_provider,
                model_name: row.model_name,
                prompt_version: row.prompt_version,
                source_duration: row.source_duration,
                created_at: row.created_at,
                updated_at: row.updated_at,
            }),
    };
    Ok(CatalogRecord {
        recording_id: Uuid::new_v4(),
        platform: row.platform,
        room_id: row.room_id,
        live_id: row.live_id,
        parent_id: row.parent_id,
        title: row.title,
        created_at_ms,
        length: row.length,
        size: u64::try_from(row.size).map_err(|_| error("Negative legacy recording size"))?,
        cover: row.cover,
        results: serde_json::to_value(results).map_err(error)?,
    })
}

impl Database {
    /// Atomically freeze legacy rows, completed results and generated UUIDs.
    /// A retry returns the original snapshot, never today's mutable old tables.
    /// Old rows, bindings, materials and operational tasks are untouched.
    pub async fn prepare_legacy_catalog_import(
        &self,
        source_id: &str,
        storage_uuid: Uuid,
    ) -> Result<ImportPlan, DatabaseError> {
        if storage_uuid.is_nil() {
            return Err(error("Invalid storage UUID"));
        }
        self.ensure_storage_schema().await?;
        let mut tx = self.pool.begin().await?;
        if let Some(saved) = sqlx::query_scalar::<_, String>(
            "SELECT plan FROM catalog_legacy_imports WHERE source_id = ?",
        )
        .bind(source_id)
        .fetch_optional(&mut *tx)
        .await?
        {
            let plan: ImportPlan = from_json(&saved)?;
            if plan.storage_uuid != storage_uuid {
                return Err(error("Legacy snapshot belongs to another storage UUID"));
            }
            return Ok(plan);
        }
        let configured: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM recording_storage_enabled WHERE storage_id = ?)",
        )
        .bind(source_id)
        .fetch_one(&mut *tx)
        .await?;
        if !configured {
            return Err(error("Legacy source is not configured"));
        }
        let ambiguous: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT live_id FROM recording_records \
            WHERE storage_backend_id=? GROUP BY live_id HAVING COUNT(*)>1)",
        )
        .bind(source_id)
        .fetch_one(&mut *tx)
        .await?;
        if ambiguous {
            return Err(error(
                "旧库包含重复 live_id；请先消除歧义，迁移未修改任何录播",
            ));
        }
        let transferring: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM recording_storage_transfers WHERE source_id = ?1 OR target_id = ?1)",
        ).bind(source_id).fetch_one(&mut *tx).await?;
        if transferring {
            return Err(error(
                "Finish pending transfers before preparing a legacy snapshot",
            ));
        }
        let mut plan = ImportPlan {
            import_id: Uuid::new_v4(),
            storage_uuid,
            count: 0,
            digest: ImportDigest::default(),
        };
        // Keyset pages under one SQLite read transaction: no unbounded catalog
        // allocation, and no network I/O while holding the database transaction.
        let mut after: Option<String> = None;
        loop {
            let rows = sqlx::query_as::<_, RecordRow>(
                "SELECT r.* FROM recording_records r WHERE storage_backend_id = ?1 \
                 AND (?2 IS NULL OR r.live_id > ?2) \
                 AND NOT EXISTS(SELECT 1 FROM recording_storage_deletions d \
                     WHERE d.platform = r.platform AND d.room_id = r.room_id AND d.live_id = r.live_id) \
                 ORDER BY r.live_id LIMIT 128",
            ).bind(source_id).bind(&after).fetch_all(&mut *tx).await?;
            if rows.is_empty() {
                break;
            }
            for row in rows {
                after = Some(row.live_id.clone());
                let summary = sqlx::query_as::<_, RecordSummaryRow>(
                    "SELECT * FROM record_summaries WHERE platform = ? AND room_id = ? AND live_id = ? AND status = 'success'",
                ).bind(&row.platform).bind(&row.room_id).bind(&row.live_id).fetch_optional(&mut *tx).await?;
                let record = legacy_record(row, summary)?;
                plan.digest.append(&record).map_err(error)?;
                sqlx::query("INSERT INTO catalog_legacy_entries (import_id, ordinal, record) VALUES (?, ?, ?)")
                    .bind(plan.import_id.to_string())
                    .bind(i64::try_from(plan.count).map_err(error)?).bind(json(&record)?)
                    .execute(&mut *tx).await?;
                plan.count = plan
                    .count
                    .checked_add(1)
                    .ok_or_else(|| error("Legacy record count overflow"))?;
            }
        }
        sqlx::query("INSERT INTO catalog_legacy_imports (source_id, import_id, storage_uuid, plan) VALUES (?, ?, ?, ?)")
            .bind(source_id).bind(plan.import_id.to_string()).bind(storage_uuid.to_string()).bind(json(&plan)?)
            .execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(plan)
    }

    /// A remote commit can succeed before the local acknowledgement. The
    /// storage-owned receipt makes this restart-safe, including after deletion.
    pub async fn resume_legacy_catalog_import(
        &self,
        source_id: &str,
        writer: &CatalogWriter,
    ) -> Result<ImportPlan, DatabaseError> {
        let saved: String =
            sqlx::query_scalar("SELECT plan FROM catalog_legacy_imports WHERE source_id = ?")
                .bind(source_id)
                .fetch_one(&self.pool)
                .await?;
        let plan: ImportPlan = from_json(&saved)?;
        let progress = writer.begin_import(plan.clone()).await.map_err(error)?;
        if !progress.complete {
            let mut index = progress.next_index;
            while index < plan.count {
                let rows: Vec<(i64, String)> = sqlx::query_as(
                    "SELECT ordinal, record FROM catalog_legacy_entries \
                     WHERE import_id = ? AND ordinal >= ? ORDER BY ordinal LIMIT 128",
                )
                .bind(plan.import_id.to_string())
                .bind(i64::try_from(index).map_err(error)?)
                .fetch_all(&self.pool)
                .await?;
                if rows.is_empty() {
                    return Err(error("Legacy snapshot is incomplete"));
                }
                for (ordinal, bytes) in rows {
                    if u64::try_from(ordinal).ok() != Some(index) {
                        return Err(error("Legacy snapshot has a gap"));
                    }
                    writer
                        .import_record(plan.clone(), index, from_json(&bytes)?)
                        .await
                        .map_err(error)?;
                    index += 1;
                }
            }
            writer.finish_import(plan.clone()).await.map_err(error)?;
        }
        sqlx::query(
            "UPDATE catalog_legacy_imports SET complete = 1 WHERE import_id = ? AND plan = ?",
        )
        .bind(plan.import_id.to_string())
        .bind(saved)
        .execute(&self.pool)
        .await?;
        Ok(plan)
    }
}

#[cfg(test)]
mod tests;
