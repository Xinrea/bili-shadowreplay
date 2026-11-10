//! Storage-owned recording JSON. SQLite in the application is a rebuildable index.
//! One BSR instance may write a storage at a time; no remote leases or CAS writes.
use futures::TryStreamExt;
use object_store::{local::LocalFileSystem, path::Path, ObjectStore, ObjectStoreExt};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    path::PathBuf,
    sync::{Arc, Mutex as StdMutex, OnceLock, Weak},
};
use tokio::sync::{Mutex, OwnedMutexGuard};
use uuid::Uuid;

mod lifecycle;
mod migration;
pub use migration::{ImportDigest, ImportPlan, ImportProgress};
pub use object_store;

const IDENTITY_PATH: &str = ".bsr/storage.json";
const RECORDS_PATH: &str = ".bsr/records";
const FORMAT_VERSION: u32 = 2;
const MAX_RECORD_BYTES: usize = 4 * 1024 * 1024;
const TIME_PREFIX: &str = "time/";

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn conflict(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::AlreadyExists, message)
}
fn store_error(error: object_store::Error) -> io::Error {
    let kind = match error {
        object_store::Error::NotFound { .. } => io::ErrorKind::NotFound,
        object_store::Error::PermissionDenied { .. }
        | object_store::Error::Unauthenticated { .. } => io::ErrorKind::PermissionDenied,
        _ => io::ErrorKind::Other,
    };
    io::Error::new(kind, error)
}
async fn read_bounded(store: &dyn ObjectStore, key: &str, max: usize) -> io::Result<Vec<u8>> {
    let result = store
        .get(&Path::parse(key).map_err(io::Error::other)?)
        .await
        .map_err(store_error)?;
    if result.meta.size > max as u64 {
        return Err(invalid("catalog object exceeds size limit"));
    }
    let mut bytes = Vec::new();
    let mut stream = result.into_stream();
    while let Some(chunk) = stream.try_next().await.map_err(store_error)? {
        if chunk.len() > max - bytes.len() {
            return Err(invalid("catalog object exceeds size limit"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageIdentity {
    pub format_version: u32,
    pub storage_uuid: Uuid,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogRecord {
    pub recording_id: Uuid,
    pub platform: String,
    pub room_id: String,
    pub live_id: String,
    pub parent_id: String,
    pub title: String,
    pub created_at_ms: i64,
    pub length: f64,
    pub size: u64,
    pub cover: Option<String>,
    #[serde(default)]
    pub results: serde_json::Value,
}
impl CatalogRecord {
    fn validate(&self) -> io::Result<()> {
        if self.recording_id.is_nil()
            || !self.length.is_finite()
            || self.length < 0.0
            || [&self.platform, &self.room_id, &self.live_id]
                .iter()
                .any(|v| v.is_empty())
        {
            return Err(invalid("invalid recording identity or duration"));
        }
        Ok(())
    }
    fn identity_key(&self) -> String {
        serde_json::to_string(&(&self.platform, &self.room_id, &self.live_id))
            .expect("strings serialize")
    }
    fn time_key(&self) -> String {
        let reversed = u64::MAX - ((self.created_at_ms as u64) ^ (1 << 63));
        format!("{TIME_PREFIX}{reversed:016x}/{}", self.recording_id)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub revision: u64,
    pub record: CatalogRecord,
}
#[derive(Debug)]
pub struct CatalogPage {
    pub entries: Vec<CatalogEntry>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RecordStatus {
    Active,
    Removed,
    Deleted,
    Transferred,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    format_version: u32,
    storage_uuid: Uuid,
    entry: CatalogEntry,
    status: RecordStatus,
    #[serde(default)]
    assets_removed: bool,
    // Retry receipts live with the current record, not in a second remote DB.
    #[serde(default)]
    received: BTreeMap<Uuid, [u8; 32]>,
    #[serde(default)]
    retired: BTreeMap<Uuid, u64>,
    #[serde(default)]
    import: Option<migration::ImportReceipt>,
}

fn record_path(id: Uuid) -> String {
    format!("{RECORDS_PATH}/{id}/recording.json")
}

fn prepare_local_path(root: &std::path::Path, key: &str) -> io::Result<PathBuf> {
    let path = root.join(key);
    let relative_parent = std::path::Path::new(key)
        .parent()
        .ok_or_else(|| invalid("invalid catalog path"))?;
    let mut current = root.to_path_buf();
    for part in relative_parent.components() {
        let std::path::Component::Normal(part) = part else {
            return Err(invalid("invalid catalog path component"));
        };
        let parent = current.clone();
        current.push(part);
        match std::fs::symlink_metadata(&current) {
            Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
            Ok(_) => return Err(invalid("catalog parent must be a real directory")),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                std::fs::create_dir(&current)?;
                #[cfg(unix)]
                std::fs::File::open(parent)?.sync_all()?;
                #[cfg(not(unix))]
                let _ = parent;
            }
            Err(e) => return Err(e),
        }
    }
    if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(invalid("catalog files must not be symlinks"));
    }
    Ok(path)
}
fn decode_document(bytes: &[u8], uuid: Uuid, key: &str) -> io::Result<Document> {
    let doc: Document =
        serde_json::from_slice(bytes).map_err(|_| invalid("invalid recording.json"))?;
    doc.entry.record.validate()?;
    if doc.format_version != FORMAT_VERSION
        || doc.storage_uuid != uuid
        || doc.entry.revision == 0
        || record_path(doc.entry.record.recording_id) != key
    {
        return Err(invalid("recording.json identity, path or format mismatch"));
    }
    Ok(doc)
}

#[derive(Clone)]
pub struct CatalogLocation {
    store: Arc<dyn ObjectStore>,
    local_root: Option<PathBuf>,
}
// Only an in-process guard. Never presented as a cross-machine lock.
fn writer_gate(uuid: Uuid) -> Arc<Mutex<()>> {
    static GATES: OnceLock<StdMutex<HashMap<Uuid, Weak<Mutex<()>>>>> = OnceLock::new();
    let mut gates = GATES
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    gates.retain(|_, gate| gate.strong_count() > 0);
    if let Some(gate) = gates.get(&uuid).and_then(Weak::upgrade) {
        return gate;
    }
    let gate = Arc::new(Mutex::new(()));
    gates.insert(uuid, Arc::downgrade(&gate));
    gate
}
impl CatalogLocation {
    pub fn local(root: impl Into<PathBuf>) -> io::Result<Self> {
        let root = root.into().canonicalize()?;
        let store = LocalFileSystem::new_with_prefix(&root)
            .map_err(store_error)?
            .with_fsync(true);
        Ok(Self {
            store: Arc::new(store),
            local_root: Some(root),
        })
    }
    pub fn object_store(store: Arc<dyn ObjectStore>) -> Self {
        Self {
            store,
            local_root: None,
        }
    }
    pub async fn identity(&self) -> io::Result<StorageIdentity> {
        let bytes = read_bounded(self.store.as_ref(), IDENTITY_PATH, 4096).await?;
        let identity: StorageIdentity =
            serde_json::from_slice(&bytes).map_err(|_| invalid("invalid storage identity"))?;
        if identity.storage_uuid.is_nil() || identity.format_version != FORMAT_VERSION {
            return Err(invalid(
                "unsupported catalog format or invalid storage UUID",
            ));
        }
        Ok(identity)
    }
    /// Publish a complete object and resolve lost acknowledgements by readback.
    /// Local publication uses fsync + atomic replacement, never truncate.
    async fn write_json<T: Serialize>(&self, key: &str, value: &T, max: usize) -> io::Result<()> {
        let bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
        if bytes.len() > max {
            return Err(invalid("catalog JSON exceeds size limit"));
        }
        if let Some(root) = &self.local_root {
            let root = root.clone();
            let key = key.to_owned();
            let content = bytes.clone();
            tokio::task::spawn_blocking(move || {
                use std::io::Write;
                let path = prepare_local_path(&root, &key)?;
                let parent = path
                    .parent()
                    .ok_or_else(|| invalid("invalid catalog path"))?;
                let mut temp = tempfile::NamedTempFile::new_in(parent)?;
                temp.write_all(&content)?;
                temp.as_file().sync_all()?;
                temp.persist(&path).map_err(|e| e.error)?;
                #[cfg(unix)]
                std::fs::File::open(parent)?.sync_all()?;
                Ok::<_, io::Error>(())
            })
            .await
            .map_err(io::Error::other)??;
        } else {
            let result = self
                .store
                .put(
                    &Path::parse(key).map_err(io::Error::other)?,
                    bytes.clone().into(),
                )
                .await
                .map_err(store_error);
            if let Err(error) = result {
                if read_bounded(self.store.as_ref(), key, max)
                    .await
                    .ok()
                    .as_deref()
                    == Some(bytes.as_slice())
                {
                    return Ok(());
                }
                return Err(error);
            }
        }
        if read_bounded(self.store.as_ref(), key, max).await? != bytes {
            return Err(invalid(
                "catalog write verification failed; do not write from multiple instances",
            ));
        }
        Ok(())
    }
    pub async fn open_writer(&self, create: bool) -> io::Result<CatalogWriter> {
        let location = self.clone();
        tokio::spawn(async move { location.open_writer_inner(create).await })
            .await
            .map_err(io::Error::other)?
    }
    async fn open_writer_inner(self, create: bool) -> io::Result<CatalogWriter> {
        // Serialize first-time identity publication within this process too.
        static OPEN_GATE: OnceLock<Mutex<()>> = OnceLock::new();
        let _opening = OPEN_GATE.get_or_init(|| Mutex::new(())).lock().await;
        // Local ownership is released automatically when the process exits.
        let local_lock = if let Some(root) = &self.local_root {
            let root = root.clone();
            Some(
                tokio::task::spawn_blocking(move || {
                    let path = prepare_local_path(&root, ".bsr/writer.lock")?;
                    let file = std::fs::OpenOptions::new()
                        .create(true)
                        .truncate(false)
                        .read(true)
                        .write(true)
                        .open(path)?;
                    fs2::FileExt::try_lock_exclusive(&file).map_err(|e| {
                        if e.raw_os_error() == fs2::lock_contended_error().raw_os_error() {
                            io::Error::new(
                                io::ErrorKind::WouldBlock,
                                "storage already has a local writer",
                            )
                        } else {
                            e
                        }
                    })?;
                    Ok::<_, io::Error>(file)
                })
                .await
                .map_err(io::Error::other)??,
            )
        } else {
            None
        };
        let identity = match self.identity().await {
            Ok(identity) => identity,
            Err(error) if create && error.kind() == io::ErrorKind::NotFound => {
                let existing = self
                    .store
                    .list(Some(&Path::from(".bsr")))
                    .try_collect::<Vec<_>>()
                    .await
                    .map_err(store_error)?;
                if existing
                    .iter()
                    .any(|o| o.location.as_ref() != ".bsr/writer.lock")
                {
                    return Err(invalid("catalog contents exist without storage identity"));
                }
                let identity = StorageIdentity {
                    format_version: FORMAT_VERSION,
                    storage_uuid: Uuid::new_v4(),
                };
                self.write_json(IDENTITY_PATH, &identity, 4096).await?;
                identity
            }
            Err(error) => return Err(error),
        };
        let gate = writer_gate(identity.storage_uuid)
            .try_lock_owned()
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "storage already has a writer in this process",
                )
            })?;
        let documents = self.load_documents(identity.storage_uuid).await?;
        let import = self.load_import().await?;
        if import
            .as_ref()
            .is_some_and(|s| s.progress.plan.storage_uuid != identity.storage_uuid)
        {
            return Err(invalid("import belongs to another storage"));
        }
        if import.is_none() && documents.values().any(|doc| doc.import.is_some()) {
            return Err(invalid(
                "record receipts exist but import journal is missing",
            ));
        }
        Ok(CatalogWriter {
            identity: identity.clone(),
            inner: Arc::new(WriterInner {
                state: Mutex::new(Some(WriterState {
                    location: self,
                    identity,
                    documents,
                    import,
                    failed: false,
                    _gate: gate,
                    _local_lock: local_lock,
                })),
            }),
        })
    }
    async fn load_documents(&self, uuid: Uuid) -> io::Result<BTreeMap<Uuid, Document>> {
        let prefix = Path::from(RECORDS_PATH);
        let before: BTreeMap<_, _> = self
            .store
            .list(Some(&prefix))
            .map_ok(|m| (m.location.to_string(), m))
            .try_collect()
            .await
            .map_err(store_error)?;
        let mut documents = BTreeMap::new();
        let mut namespaces = HashMap::new();
        for key in before.keys().filter(|key| key.ends_with("/recording.json")) {
            let doc = decode_document(
                &read_bounded(self.store.as_ref(), key, MAX_RECORD_BYTES).await?,
                uuid,
                key,
            )?;
            if matches!(doc.status, RecordStatus::Active | RecordStatus::Deleted)
                && namespaces
                    .insert(
                        doc.entry.record.identity_key(),
                        doc.entry.record.recording_id,
                    )
                    .is_some()
            {
                return Err(conflict("duplicate recording namespace in catalog"));
            }
            documents.insert(doc.entry.record.recording_id, doc);
        }
        let after: BTreeMap<_, _> = self
            .store
            .list(Some(&prefix))
            .map_ok(|m| (m.location.to_string(), m))
            .try_collect()
            .await
            .map_err(store_error)?;
        if before != after {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "catalog changed during scan; retry",
            ));
        }
        Ok(documents)
    }
    pub async fn open_reader(&self) -> io::Result<CatalogReader> {
        let identity = self.identity().await?;
        Ok(CatalogReader {
            identity,
            location: Some(self.clone()),
        })
    }
}
struct WriterState {
    location: CatalogLocation,
    identity: StorageIdentity,
    documents: BTreeMap<Uuid, Document>,
    import: Option<migration::ImportState>,
    failed: bool,
    _gate: OwnedMutexGuard<()>,
    _local_lock: Option<std::fs::File>,
}
impl WriterState {
    fn check(&self) -> io::Result<()> {
        if self.failed {
            Err(invalid(
                "catalog write outcome is unknown; close and reopen",
            ))
        } else {
            Ok(())
        }
    }
    fn revision(&self) -> u64 {
        self.documents
            .values()
            .map(|d| d.entry.revision)
            .max()
            .unwrap_or(0)
    }
    fn next_revision(&self) -> io::Result<u64> {
        self.revision()
            .checked_add(1)
            .ok_or_else(|| invalid("catalog revision overflow"))
    }
    fn prepare_put(&self, record: CatalogRecord, expected: Option<u64>) -> io::Result<Document> {
        record.validate()?;
        let previous = self.documents.get(&record.recording_id);
        if previous
            .filter(|d| d.status == RecordStatus::Active)
            .map(|d| d.entry.revision)
            != expected
        {
            return Err(conflict("record revision changed"));
        }
        if previous.is_some_and(|d| d.entry.record.identity_key() != record.identity_key()) {
            return Err(conflict("recording identity cannot be changed"));
        }
        for doc in self.documents.values() {
            if doc.entry.record.identity_key() == record.identity_key()
                && (doc.status == RecordStatus::Deleted
                    || (doc.status == RecordStatus::Active
                        && doc.entry.record.recording_id != record.recording_id))
            {
                return Err(conflict(
                    "recording namespace is deleted or belongs to another UUID",
                ));
            }
        }
        Ok(Document {
            format_version: FORMAT_VERSION,
            storage_uuid: self.identity.storage_uuid,
            entry: CatalogEntry {
                revision: self.next_revision()?,
                record,
            },
            status: RecordStatus::Active,
            assets_removed: false,
            received: previous.map(|d| d.received.clone()).unwrap_or_default(),
            retired: previous.map(|d| d.retired.clone()).unwrap_or_default(),
            import: previous.and_then(|d| d.import.clone()),
        })
    }
    async fn persist(&mut self, document: Document) -> io::Result<()> {
        let id = document.entry.record.recording_id;
        if let Err(error) = self
            .location
            .write_json(&record_path(id), &document, MAX_RECORD_BYTES)
            .await
        {
            self.failed = true;
            return Err(error);
        }
        self.documents.insert(id, document);
        Ok(())
    }
}
struct WriterInner {
    state: Mutex<Option<WriterState>>,
}
#[derive(Clone)]
pub struct CatalogWriter {
    identity: StorageIdentity,
    inner: Arc<WriterInner>,
}
impl CatalogWriter {
    pub fn identity(&self) -> &StorageIdentity {
        &self.identity
    }
    pub async fn put(
        &self,
        record: CatalogRecord,
        expected_revision: Option<u64>,
    ) -> io::Result<CatalogEntry> {
        let inner = self.inner.clone();
        tokio::spawn(async move {
            let mut guard = inner.state.lock().await;
            let state = guard.as_mut().ok_or_else(|| invalid("catalog is closed"))?;
            state.check()?;
            let doc = state.prepare_put(record, expected_revision)?;
            let entry = doc.entry.clone();
            state.persist(doc).await?;
            Ok(entry)
        })
        .await
        .map_err(io::Error::other)?
    }
    pub async fn get(&self, id: Uuid) -> io::Result<Option<CatalogEntry>> {
        let guard = self.inner.state.lock().await;
        let state = guard.as_ref().ok_or_else(|| invalid("catalog is closed"))?;
        state.check()?;
        Ok(state
            .documents
            .get(&id)
            .filter(|d| d.status == RecordStatus::Active)
            .map(|d| d.entry.clone()))
    }
    pub async fn delete(&self, id: Uuid, expected_revision: u64) -> io::Result<()> {
        self.retire(id, expected_revision, RecordStatus::Removed, None)
            .await
    }
    pub async fn list(&self, cursor: Option<&str>, limit: usize) -> io::Result<CatalogPage> {
        let guard = self.inner.state.lock().await;
        let state = guard.as_ref().ok_or_else(|| invalid("catalog is closed"))?;
        state.check()?;
        page(entries(&state.documents), cursor, limit)
    }
    pub async fn scan(&self) -> io::Result<CatalogScan> {
        let guard = self.inner.state.lock().await;
        let state = guard.as_ref().ok_or_else(|| invalid("catalog is closed"))?;
        state.check()?;
        state.check_import_ready()?;
        Ok(make_scan(self.identity.storage_uuid, &state.documents))
    }
    pub async fn close(&self) -> io::Result<()> {
        // Joins any uncancelled publication task before releasing local ownership.
        self.inner.state.lock().await.take();
        Ok(())
    }
}
pub struct CatalogReader {
    identity: StorageIdentity,
    location: Option<CatalogLocation>,
}
impl CatalogReader {
    pub fn identity(&self) -> &StorageIdentity {
        &self.identity
    }
    async fn documents(&self) -> io::Result<BTreeMap<Uuid, Document>> {
        let location = self
            .location
            .as_ref()
            .ok_or_else(|| invalid("catalog reader is closed"))?;
        if location.identity().await? != self.identity {
            return Err(invalid("storage identity changed"));
        }
        location.load_documents(self.identity.storage_uuid).await
    }
    pub async fn get(&self, id: Uuid) -> io::Result<Option<CatalogEntry>> {
        Ok(self
            .documents()
            .await?
            .get(&id)
            .filter(|d| d.status == RecordStatus::Active)
            .map(|d| d.entry.clone()))
    }
    pub async fn list(&self, cursor: Option<&str>, limit: usize) -> io::Result<CatalogPage> {
        page(entries(&self.documents().await?), cursor, limit)
    }
    pub async fn scan(&self) -> io::Result<CatalogScan> {
        let location = self
            .location
            .as_ref()
            .ok_or_else(|| invalid("catalog reader is closed"))?;
        let import = location.load_import().await?;
        if import
            .as_ref()
            .is_some_and(|state| !state.progress.complete)
        {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "finish import before rebuilding the index",
            ));
        }
        let documents = self.documents().await?;
        if import != location.load_import().await? {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "import changed during scan",
            ));
        }
        if import
            .as_ref()
            .is_some_and(|state| state.progress.plan.storage_uuid != self.identity.storage_uuid)
            || (import.is_none() && documents.values().any(|doc| doc.import.is_some()))
        {
            return Err(invalid("import identity or journal mismatch"));
        }
        Ok(make_scan(self.identity.storage_uuid, &documents))
    }
    pub async fn close(mut self) -> io::Result<()> {
        self.location.take();
        Ok(())
    }
}
pub struct CatalogScan {
    storage_uuid: Uuid,
    revision: u64,
    iterator: std::vec::IntoIter<CatalogEntry>,
}
impl CatalogScan {
    pub fn storage_uuid(&self) -> Uuid {
        self.storage_uuid
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub async fn next_page(&mut self, limit: usize) -> io::Result<Vec<CatalogEntry>> {
        validate_page(None, limit)?;
        Ok(self.iterator.by_ref().take(limit).collect())
    }
}
fn entries(documents: &BTreeMap<Uuid, Document>) -> Vec<CatalogEntry> {
    let mut entries: Vec<_> = documents
        .values()
        .filter(|d| d.status == RecordStatus::Active)
        .map(|d| d.entry.clone())
        .collect();
    entries.sort_by_key(|e| e.record.time_key());
    entries
}
fn make_scan(uuid: Uuid, documents: &BTreeMap<Uuid, Document>) -> CatalogScan {
    CatalogScan {
        storage_uuid: uuid,
        revision: documents
            .values()
            .map(|d| d.entry.revision)
            .max()
            .unwrap_or(0),
        iterator: entries(documents).into_iter(),
    }
}
fn validate_page(cursor: Option<&str>, limit: usize) -> io::Result<()> {
    let valid = |key: &str| {
        key.strip_prefix(TIME_PREFIX)
            .and_then(|s| s.split_once('/'))
            .is_some_and(|(time, id)| {
                time.len() == 16
                    && time
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                    && Uuid::parse_str(id).is_ok_and(|uuid| uuid.to_string() == id)
            })
    };
    if !(1..=1000).contains(&limit) || cursor.is_some_and(|s| !valid(s)) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid catalog page",
        ));
    }
    Ok(())
}
fn page(entries: Vec<CatalogEntry>, cursor: Option<&str>, limit: usize) -> io::Result<CatalogPage> {
    validate_page(cursor, limit)?;
    let mut iter = entries
        .into_iter()
        .filter(|e| cursor.is_none_or(|c| e.record.time_key().as_str() > c));
    let entries: Vec<_> = iter.by_ref().take(limit).collect();
    let next_cursor = iter
        .next()
        .and_then(|_| entries.last().map(|e| e.record.time_key()));
    Ok(CatalogPage {
        entries,
        next_cursor,
    })
}
