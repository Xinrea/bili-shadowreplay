//! Production catalog authority. SQLite recording rows are a compatibility
//! projection for existing API consumers; all acknowledged metadata writes go
//! to the storage-owned catalog first.
use super::*;
use crate::events::{DataChange, EventBus};
use recording_catalog::CatalogReader;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::Mutex;

#[cfg(test)]
mod tests;
mod transfer;

pub const SCHEMA_SQL: &str = "
    CREATE TABLE catalog_original_records AS SELECT * FROM records;
    CREATE TABLE catalog_original_summaries AS SELECT * FROM record_summaries;
    CREATE TABLE catalog_runtime_backends (
        alias TEXT PRIMARY KEY, storage_uuid TEXT UNIQUE,
        ready INTEGER NOT NULL DEFAULT 0, writable INTEGER NOT NULL DEFAULT 0,
        error TEXT
    );
    CREATE TABLE catalog_record_links (
        live_id TEXT PRIMARY KEY, storage_uuid TEXT NOT NULL,
        recording_uuid TEXT NOT NULL, revision INTEGER NOT NULL
    );
    CREATE TABLE catalog_record_keys (
        live_id TEXT PRIMARY KEY, recording_uuid TEXT NOT NULL
    );
    CREATE TABLE catalog_active_recordings (
        live_id TEXT PRIMARY KEY, storage_uuid TEXT NOT NULL
    );
    CREATE TABLE catalog_transfer_intents (
        token TEXT PRIMARY KEY, source_uuid TEXT NOT NULL, target_uuid TEXT NOT NULL,
        entry TEXT NOT NULL
    );
    CREATE TABLE catalog_room_preferences (
        platform TEXT NOT NULL, room_id TEXT NOT NULL, storage_uuid TEXT NOT NULL,
        PRIMARY KEY(platform, room_id)
    );
";

pub const VIEW_SQL: &str = "
    DROP VIEW IF EXISTS recording_records;
    CREATE VIEW recording_records AS
        SELECT r.*, COALESCE(binding.storage_id, config.legacy_backend, 'default') AS storage_backend_id,
            COALESCE(c.ready, 0) AS storage_available,
            CASE WHEN c.ready = 1 THEN NULL ELSE COALESCE(c.error, 'Storage catalog is unavailable') END AS storage_unavailable_reason
        FROM records r
        LEFT JOIN recording_storage_bindings binding
          ON binding.platform = r.platform AND binding.room_id = r.room_id AND binding.live_id = r.live_id
        LEFT JOIN recording_storage_configuration config ON config.singleton = 1
        JOIN recording_storage_enabled enabled
          ON enabled.storage_id = COALESCE(binding.storage_id, config.legacy_backend, 'default')
        LEFT JOIN catalog_runtime_backends c ON c.alias = enabled.storage_id;
";

#[derive(Clone)]
pub struct Handle {
    pub uuid: Uuid,
    pub writer: Option<CatalogWriter>,
    pub _reader: Option<Arc<CatalogReader>>,
    pub credential_revision: Option<String>,
}

#[derive(Clone, Debug, Serialize, sqlx::FromRow)]
pub struct Status {
    pub alias: String,
    pub storage_uuid: Option<String>,
    pub ready: bool,
    pub writable: bool,
    pub error: Option<String>,
}

pub enum Change {
    Delta(f64, u64),
    Parent(String),
    Cover(Option<String>),
    Results(serde_json::Value),
}

pub struct CatalogRuntime {
    pub(crate) pool: SqlitePool,
    pub(crate) handles: Mutex<HashMap<String, Handle>>,
    events: EventBus,
    mutation_access: std::sync::RwLock<HashMap<String, (String, bool)>>,
}

impl Database {
    pub async fn has_legacy_catalog_transfers(&self) -> Result<bool, DatabaseError> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM recording_storage_transfers t \
            WHERE NOT EXISTS(SELECT 1 FROM catalog_record_links l WHERE l.live_id=t.live_id) \
            AND NOT EXISTS(SELECT 1 FROM catalog_transfer_intents i WHERE i.token=t.token))",
        )
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn finish_catalog_recording(&self, live: &str) -> Result<(), DatabaseError> {
        if self.catalogs.get().is_some() {
            sqlx::query("DELETE FROM catalog_active_recordings WHERE live_id=?")
                .bind(live)
                .execute(&self.pool)
                .await?;
        }
        Ok(())
    }

    pub async fn catalog_recording_active(&self, alias: &str) -> Result<bool, DatabaseError> {
        if self.catalogs.get().is_none() {
            return Ok(false);
        }
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM catalog_active_recordings a \
            JOIN catalog_runtime_backends b ON b.storage_uuid=a.storage_uuid WHERE b.alias=?)",
        )
        .bind(alias)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn record_scope(&self, live: &str) -> Result<(String, String), DatabaseError> {
        Ok(
            sqlx::query_as("SELECT platform,room_id FROM records WHERE live_id=?")
                .bind(live)
                .fetch_one(&self.pool)
                .await?,
        )
    }

    pub async fn catalog_write_result(
        &self,
        live: &str,
        result: Result<(), DatabaseError>,
    ) -> Result<(), DatabaseError> {
        if let Err(error) = &result {
            if let Ok(Some(alias)) = sqlx::query_scalar::<_, String>(
                "SELECT storage_id FROM recording_storage_bindings WHERE live_id=?",
            )
            .bind(live)
            .fetch_optional(&self.pool)
            .await
            {
                let _ = self.catalog_failed(&alias, &error.to_string()).await;
            }
        }
        result
    }

    pub async fn catalog_mount_for_alias(
        &self,
        alias: &str,
        uuid: Uuid,
    ) -> Result<super::projection::CatalogMount, DatabaseError> {
        if let Some((saved, epoch)) = sqlx::query_as::<_, (String, String)>(
            "SELECT storage_uuid,epoch FROM catalog_mounts WHERE alias=?",
        )
        .bind(alias)
        .fetch_optional(&self.pool)
        .await?
        {
            if saved != uuid.to_string() {
                return Err(error("Catalog mount UUID changed"));
            }
            return Ok(super::projection::CatalogMount {
                alias: alias.into(),
                storage_uuid: uuid,
                epoch: epoch.parse().map_err(error)?,
            });
        }
        self.mount_catalog(alias, uuid).await
    }

    pub async fn enable_catalogs(&self) -> Result<Arc<CatalogRuntime>, DatabaseError> {
        self.ensure_storage_schema().await?;
        let first = self.catalogs.get().is_none();
        let runtime = self
            .catalogs
            .get_or_init(|| {
                Arc::new(CatalogRuntime {
                    pool: self.pool.clone(),
                    handles: Mutex::new(HashMap::new()),
                    events: self.events.clone(),
                    mutation_access: std::sync::RwLock::new(HashMap::new()),
                })
            })
            .clone();
        if first {
            sqlx::query("DELETE FROM catalog_active_recordings")
                .execute(&self.pool)
                .await?;
            sqlx::query("DELETE FROM catalog_projection_entries WHERE NOT EXISTS( \
                SELECT 1 FROM catalog_projection_heads h WHERE h.storage_uuid=catalog_projection_entries.storage_uuid \
                AND h.generation=catalog_projection_entries.generation)").execute(&self.pool).await?;
            sqlx::query(
                "UPDATE catalog_runtime_backends SET ready=0,writable=0,error='正在连接存储目录'",
            )
            .execute(&self.pool)
            .await?;
            let rows: Vec<(String, String, String, String)> = sqlx::query_as(
                "SELECT platform,room_id,live_id,storage_id FROM recording_storage_bindings",
            )
            .fetch_all(&self.pool)
            .await?;
            let mut access = runtime
                .mutation_access
                .write()
                .unwrap_or_else(|p| p.into_inner());
            for (platform, room, live, alias) in rows {
                access.insert(format!("{platform}/{room}/{live}"), (alias, false));
            }
        }
        if first {
            let mut tx = self.pool.begin().await?;
            sqlx::query(VIEW_SQL).execute(&mut *tx).await?;
            tx.commit().await?;
        }
        Ok(runtime)
    }

    pub async fn catalog_status(&self) -> Result<Vec<Status>, DatabaseError> {
        Ok(sqlx::query_as("SELECT e.storage_id AS alias, COALESCE(c.storage_uuid,m.storage_uuid) AS storage_uuid, \
            COALESCE(c.ready,0) AS ready, COALESCE(c.writable,0) AS writable, \
            CASE WHEN c.ready=1 THEN NULL ELSE COALESCE(c.error,'目录等待初始化；如有旧转移请完成后重启应用') END AS error \
            FROM recording_storage_enabled e LEFT JOIN catalog_runtime_backends c ON c.alias=e.storage_id \
            LEFT JOIN catalog_mounts m ON m.alias=e.storage_id ORDER BY e.storage_id")
            .fetch_all(&self.pool).await?)
    }

    pub async fn catalog_failed(&self, alias: &str, message: &str) -> Result<(), DatabaseError> {
        sqlx::query(
            "INSERT INTO catalog_runtime_backends(alias, error) VALUES (?, ?) \
            ON CONFLICT(alias) DO UPDATE SET ready=0, writable=0, error=excluded.error",
        )
        .bind(alias)
        .bind(message)
        .execute(&self.pool)
        .await?;
        self.notify_change(DataChange::Records);
        if let Some(runtime) = self.catalogs.get() {
            runtime.deny_mutations(alias);
        }
        Ok(())
    }

    pub async fn catalog_needs_import(&self, alias: &str) -> Result<bool, DatabaseError> {
        // A persisted runtime UUID is the cutover marker. Losing a projection
        // row/link after that point is never permission to re-import SQLite.
        Ok(sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM catalog_runtime_backends WHERE alias=? AND storage_uuid IS NOT NULL) \
            AND (EXISTS(SELECT 1 FROM recording_records r \
            WHERE storage_backend_id=? AND NOT EXISTS(SELECT 1 FROM catalog_record_links l WHERE l.live_id=r.live_id)) \
            OR EXISTS(SELECT 1 FROM catalog_legacy_imports WHERE source_id=? AND complete=0))")
            .bind(alias).bind(alias).bind(alias).fetch_one(&self.pool).await?)
    }
}

impl CatalogRuntime {
    pub async fn readable(&self, alias: &str) -> Result<(), DatabaseError> {
        let ready: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM catalog_runtime_backends WHERE alias=? AND ready=1)",
        )
        .bind(alias)
        .fetch_one(&self.pool)
        .await?;
        if !ready || !self.handles.lock().await.contains_key(alias) {
            return Err(error("存储目录不可用，请恢复连接后重试"));
        }
        Ok(())
    }

    pub fn check_mutation(
        &self,
        platform: &str,
        room: &str,
        live: &str,
    ) -> Result<(), DatabaseError> {
        let access = self
            .mutation_access
            .read()
            .unwrap_or_else(|p| p.into_inner());
        if access
            .get(&format!("{platform}/{room}/{live}"))
            .is_some_and(|(_, writable)| !writable)
        {
            return Err(error("存储目录离线或只读，不能修改录播资产"));
        }
        Ok(())
    }

    fn deny_mutations(&self, alias: &str) {
        for (backend, writable) in self
            .mutation_access
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .values_mut()
        {
            if backend == alias {
                *writable = false;
            }
        }
    }

    pub async fn writable(&self, alias: &str) -> Result<(), DatabaseError> {
        let ready: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM catalog_runtime_backends WHERE alias=? AND ready=1 AND writable=1)")
            .bind(alias).fetch_one(&self.pool).await?;
        if !ready {
            return Err(error("存储目录不可写，请在存储设置中重试连接"));
        }
        let handles = self.handles.lock().await;
        writer(&handles, alias)?;
        Ok(())
    }

    pub async fn validate_recording(
        &self,
        alias: &str,
        platform: &str,
        room: &str,
        live: &str,
    ) -> Result<(), DatabaseError> {
        self.writable(alias).await?;
        let handles = self.handles.lock().await;
        writer(&handles, alias)?
            .writer
            .as_ref()
            .expect("writer")
            .find_recording(platform, room, live)
            .await
            .map_err(error)?;
        Ok(())
    }

    pub async fn detach(&self, alias: &str) -> Result<(), DatabaseError> {
        self.deny_mutations(alias);
        let mut handles = self.handles.lock().await;
        if let Some(handle) = handles.remove(alias) {
            if let Some(writer) = handle.writer {
                writer.close().await.map_err(error)?;
            }
        }
        Ok(())
    }

    pub async fn attach(
        self: &Arc<Self>,
        alias: String,
        handle: Handle,
    ) -> Result<(), DatabaseError> {
        let this = self.clone();
        tokio::spawn(async move {
            let mut handles = this.handles.lock().await;
            if handles.values().any(|other| other.uuid == handle.uuid) {
                return Err(error("This storage UUID is already mounted"));
            }
            this.publish_cache(&alias, handle.uuid).await?;
            sqlx::query("INSERT INTO catalog_runtime_backends(alias,storage_uuid,ready,writable,error) VALUES (?,?,1,?,NULL) \
                ON CONFLICT(alias) DO UPDATE SET storage_uuid=excluded.storage_uuid,ready=1,writable=excluded.writable,error=NULL")
                .bind(&alias).bind(handle.uuid.to_string()).bind(handle.writer.is_some()).execute(&this.pool).await?;
            sqlx::query("INSERT OR IGNORE INTO catalog_room_preferences(platform,room_id,storage_uuid) \
                SELECT platform,room_id,? FROM recorders WHERE storage_backend_id=?")
                .bind(handle.uuid.to_string()).bind(&alias).execute(&this.pool).await?;
            let rows: Vec<(String,String,String)> = sqlx::query_as(
                "SELECT platform,room_id,live_id FROM recording_storage_bindings WHERE storage_id=?",
            ).bind(&alias).fetch_all(&this.pool).await?;
            {
                let mut access = this.mutation_access.write().unwrap_or_else(|p| p.into_inner());
                for (platform,room,live) in rows {
                    access.insert(format!("{platform}/{room}/{live}"),(alias.clone(),handle.writer.is_some()));
                }
            }
            handles.insert(alias, handle);
            this.events.publish(crate::events::AppEvent::DataChanged(DataChange::Records));
            Ok(())
        }).await.map_err(error)?
    }

    async fn publish_cache(&self, alias: &str, uuid: Uuid) -> Result<(), DatabaseError> {
        let mut tx = self.pool.begin().await?;
        // The public legacy API uses live_id as a key. Do not overwrite another
        // mounted archive if an imported catalog has a colliding legacy ID.
        let collision: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM catalog_projection_entries e \
             JOIN catalog_projection_heads h ON h.storage_uuid=e.storage_uuid AND h.generation=e.generation \
             JOIN records r ON r.live_id=json_extract(e.entry,'$.record.live_id') \
             LEFT JOIN recording_storage_bindings b ON b.platform=r.platform AND b.room_id=r.room_id AND b.live_id=r.live_id \
             WHERE e.storage_uuid=?1 AND COALESCE(b.storage_id,?2)!=?2 \
             AND NOT EXISTS(SELECT 1 FROM recording_storage_transfers t WHERE t.live_id=r.live_id))",
        ).bind(uuid.to_string()).bind(alias).fetch_one(&mut *tx).await?;
        if collision {
            return Err(error("Recording live_id conflicts with another mounted storage; no records were overwritten"));
        }
        let ids: Vec<String> = sqlx::query_scalar(
            "SELECT live_id FROM recording_records r WHERE storage_backend_id=? \
             AND NOT EXISTS(SELECT 1 FROM recording_storage_transfers t WHERE t.live_id=r.live_id)",
        )
        .bind(alias)
        .fetch_all(&mut *tx)
        .await?;
        for id in ids {
            remove_cache(&mut tx, &id).await?;
        }
        let mut after = String::new();
        loop {
            let rows: Vec<(String, String)> = sqlx::query_as(
                "SELECT e.recording_uuid,e.entry FROM catalog_projection_entries e \
                 JOIN catalog_projection_heads h ON h.storage_uuid=e.storage_uuid AND h.generation=e.generation \
                 WHERE e.storage_uuid=? AND e.recording_uuid>? \
                 AND NOT EXISTS(SELECT 1 FROM recording_storage_transfers t WHERE t.live_id=json_extract(e.entry,'$.record.live_id')) \
                 ORDER BY e.recording_uuid LIMIT 128",
            ).bind(uuid.to_string()).bind(&after).fetch_all(&mut *tx).await?;
            if rows.is_empty() {
                break;
            }
            for (id, bytes) in rows {
                after = id;
                let entry: CatalogEntry = from_json(&bytes)?;
                cache_entry(&mut tx, alias, uuid, &entry).await?;
                restore_summary(&mut tx, &entry.record).await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn create(
        self: &Arc<Self>,
        alias: String,
        record: CatalogRecord,
    ) -> Result<(), DatabaseError> {
        let this = self.clone();
        tokio::spawn(async move {
            let handles = this.handles.lock().await;
            let handle = writer(&handles, &alias)?;
            let db = handle.writer.as_ref().expect("writer checked");
            let entry = match db
                .find_recording(&record.platform, &record.room_id, &record.live_id)
                .await
                .map_err(error)?
            {
                Some(entry) => entry,
                None => db.put(record, None).await.map_err(error)?,
            };
            let mut tx = this.pool.begin().await?;
            cache_entry(&mut tx, &alias, handle.uuid, &entry).await?;
            tx.commit().await?;
            this.mutation_access
                .write()
                .unwrap_or_else(|p| p.into_inner())
                .insert(
                    format!(
                        "{}/{}/{}",
                        entry.record.platform, entry.record.room_id, entry.record.live_id
                    ),
                    (alias, true),
                );
            Ok(())
        })
        .await
        .map_err(error)?
    }

    pub async fn update(
        self: &Arc<Self>,
        live: String,
        change: Change,
    ) -> Result<(), DatabaseError> {
        let this = self.clone();
        tokio::spawn(async move {
            let handles = this.handles.lock().await;
            let (alias, id) = this.resolve(&live).await?;
            let handle = writer(&handles, &alias)?;
            let db = handle.writer.as_ref().expect("writer checked");
            let mut entry = db
                .get(id)
                .await
                .map_err(error)?
                .ok_or(DatabaseError::NotFound)?;
            match change {
                Change::Delta(length, size) => {
                    if !length.is_finite() || length < 0.0 {
                        return Err(error("Invalid recording duration"));
                    }
                    entry.record.length += length;
                    entry.record.size = entry
                        .record
                        .size
                        .checked_add(size)
                        .ok_or(DatabaseError::NumberExceedI64Range)?;
                }
                Change::Parent(parent) => entry.record.parent_id = parent,
                Change::Cover(cover) => entry.record.cover = cover,
                Change::Results(results) => entry.record.results = results,
            }
            i64::try_from(entry.record.size).map_err(|_| DatabaseError::NumberExceedI64Range)?;
            let entry = db
                .put(entry.record, Some(entry.revision))
                .await
                .map_err(error)?;
            let mut tx = this.pool.begin().await?;
            cache_entry(&mut tx, &alias, handle.uuid, &entry).await?;
            tx.commit().await?;
            this.events.publish_coalesced_change(DataChange::Records);
            Ok(())
        })
        .await
        .map_err(error)?
    }

    /// Mark remote metadata before removing any media. SQLite is left in place
    /// until cleanup succeeds so a failed foreground delete remains retryable.
    pub async fn begin_delete(&self, live: &str) -> Result<(), DatabaseError> {
        let handles = self.handles.lock().await;
        let (alias, id) = self.resolve(live).await?;
        let handle = writer(&handles, &alias)?;
        let db = handle.writer.as_ref().expect("writer checked");
        if let Some(entry) = db.get(id).await.map_err(error)? {
            db.delete_recording(entry.record, entry.revision)
                .await
                .map_err(error)?;
        }
        Ok(())
    }

    pub async fn finish_delete(&self, live: &str) -> Result<(), DatabaseError> {
        let handles = self.handles.lock().await;
        let (alias, id) = self.resolve(live).await?;
        writer(&handles, &alias)?
            .writer
            .as_ref()
            .expect("writer checked")
            .finish_deletion(id)
            .await
            .map_err(error)
    }

    pub async fn delete(self: &Arc<Self>, live: String) -> Result<(), DatabaseError> {
        let this = self.clone();
        tokio::spawn(async move {
            let handles = this.handles.lock().await;
            let (alias, id) = this.resolve(&live).await?;
            let handle = writer(&handles, &alias)?;
            let db = handle.writer.as_ref().expect("writer checked");
            if let Some(entry) = db.get(id).await.map_err(error)? {
                db.delete_recording(entry.record, entry.revision)
                    .await
                    .map_err(error)?;
            }
            let mut tx = this.pool.begin().await?;
            remove_cache(&mut tx, &live).await?;
            tx.commit().await?;
            Ok(())
        })
        .await
        .map_err(error)?
    }

    pub(crate) async fn resolve(&self, live: &str) -> Result<(String, Uuid), DatabaseError> {
        let (alias, id): (String, String) = sqlx::query_as(
            "SELECT c.alias,l.recording_uuid FROM catalog_record_links l \
             JOIN catalog_runtime_backends c ON c.storage_uuid=l.storage_uuid WHERE l.live_id=?",
        )
        .bind(live)
        .fetch_one(&self.pool)
        .await?;
        Ok((alias, id.parse().map_err(error)?))
    }
}

pub(crate) fn writer<'a>(
    handles: &'a HashMap<String, Handle>,
    alias: &str,
) -> Result<&'a Handle, DatabaseError> {
    handles
        .get(alias)
        .filter(|handle| handle.writer.is_some())
        .ok_or_else(|| {
            error(format!(
                "Storage {alias} is offline, read-only or still awaiting catalog migration"
            ))
        })
}

pub(crate) async fn remove_cache(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    live: &str,
) -> Result<(), DatabaseError> {
    sqlx::query("DELETE FROM record_summaries WHERE live_id=?")
        .bind(live)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM records WHERE live_id=?")
        .bind(live)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM recording_storage_bindings WHERE live_id=?")
        .bind(live)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM catalog_record_links WHERE live_id=?")
        .bind(live)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub(crate) async fn cache_entry(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    alias: &str,
    uuid: Uuid,
    entry: &CatalogEntry,
) -> Result<(), DatabaseError> {
    let record = &entry.record;
    let reserved = sqlx::query(
        "INSERT INTO catalog_record_keys(live_id,recording_uuid) VALUES (?,?) \
        ON CONFLICT(live_id) DO UPDATE SET recording_uuid=excluded.recording_uuid \
        WHERE catalog_record_keys.recording_uuid=excluded.recording_uuid",
    )
    .bind(&record.live_id)
    .bind(record.recording_id.to_string())
    .execute(&mut **tx)
    .await?
    .rows_affected();
    if reserved != 1 {
        return Err(error(
            "Recording live_id belongs to another UUID; old links cannot be redirected",
        ));
    }
    let existing: Option<(String, String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT r.platform,r.room_id,l.recording_uuid,b.storage_id FROM records r \
         LEFT JOIN catalog_record_links l ON l.live_id=r.live_id \
         LEFT JOIN recording_storage_bindings b ON b.live_id=r.live_id WHERE r.live_id=?",
    )
    .bind(&record.live_id)
    .fetch_optional(&mut **tx)
    .await?;
    if existing.is_some_and(|(platform, room, id, backend)| {
        platform != record.platform
            || room != record.room_id
            || id.is_some_and(|id| id != record.recording_id.to_string())
            || backend.is_some_and(|backend| backend != alias)
    }) {
        return Err(error(
            "Recording ID conflicts with another archive; no cached record was overwritten",
        ));
    }
    let created = chrono::DateTime::from_timestamp_millis(record.created_at_ms)
        .ok_or_else(|| error("Invalid catalog timestamp"))?
        .to_rfc3339();
    sqlx::query("DELETE FROM records WHERE live_id=?")
        .bind(&record.live_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("INSERT INTO records(live_id,platform,room_id,parent_id,title,length,size,created_at,cover) VALUES (?,?,?,?,?,?,?,?,?)")
        .bind(&record.live_id).bind(&record.platform).bind(&record.room_id).bind(&record.parent_id).bind(&record.title)
        .bind(record.length).bind(i64::try_from(record.size).map_err(error)?).bind(created).bind(&record.cover).execute(&mut **tx).await?;
    sqlx::query("INSERT INTO recording_storage_bindings(platform,room_id,live_id,storage_id) VALUES (?,?,?,?) \
        ON CONFLICT(platform,room_id,live_id) DO UPDATE SET storage_id=excluded.storage_id")
        .bind(&record.platform).bind(&record.room_id).bind(&record.live_id).bind(alias).execute(&mut **tx).await?;
    sqlx::query("INSERT INTO catalog_record_links(live_id,storage_uuid,recording_uuid,revision) VALUES (?,?,?,?) \
        ON CONFLICT(live_id) DO UPDATE SET storage_uuid=excluded.storage_uuid,recording_uuid=excluded.recording_uuid,revision=excluded.revision")
        .bind(&record.live_id).bind(uuid.to_string()).bind(record.recording_id.to_string())
        .bind(i64::try_from(entry.revision).map_err(error)?).execute(&mut **tx).await?;
    Ok(())
}

async fn restore_summary(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    record: &CatalogRecord,
) -> Result<(), DatabaseError> {
    if record.results.is_null() {
        return Ok(());
    }
    let results: ArchiveResults = serde_json::from_value(record.results.clone()).map_err(error)?;
    if results.version != 1 {
        return Err(error("Unsupported archive results version"));
    }
    if let Some(s) = results.summary {
        sqlx::query("INSERT INTO record_summaries(platform,room_id,live_id,status,stage,subtitle_srt,subtitle_text,summary_markdown,highlights_json,model_provider,model_name,prompt_version,source_duration,created_at,updated_at) \
            VALUES (?,?,?,'success','completed',?,?,?,?,?,?,?,?,?,?)")
            .bind(&record.platform).bind(&record.room_id).bind(&record.live_id)
            .bind(s.subtitle_srt).bind(s.subtitle_text).bind(s.summary_markdown).bind(s.highlights_json)
            .bind(s.model_provider).bind(s.model_name).bind(s.prompt_version).bind(s.source_duration)
            .bind(s.created_at).bind(s.updated_at).execute(&mut **tx).await?;
    }
    Ok(())
}
