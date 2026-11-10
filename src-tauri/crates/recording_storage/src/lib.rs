//! Backend-owned recording assets and explicit temporary/legacy local adapters.
//!
//! Object replacement is atomic per file, not a transaction across the media,
//! playlist and checkpoint. Local roots must be application-controlled: key
//! validation rejects traversal, but is not a sandbox against concurrent symlink
//! replacement by another process. Backends may be selected per recording.
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::Mutex;

mod local_publication;
mod logs;
mod object_transfer;
mod relocation;
mod s3;
mod transfer;
pub use logs::LOG_CHUNK_BYTES;
pub use object_transfer::{ObjectTransfer, ObjectTransferManifest};
pub use s3::{S3Backend, S3Config, S3Credentials};
pub use transfer::{
    is_cross_device_error, local_transfer_root, LocalTransfer, RenameManifest, TransferDirectory,
    TransferFile, TransferManifest,
};

fn invalid_key() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "invalid recording object key")
}

/// Keys use `/` separators on every platform. Reject ambiguous Windows names as
/// well, so an archive's namespace has the same meaning on all supported hosts.
pub fn validate_key(key: &str) -> io::Result<()> {
    if key.is_empty() {
        return Err(invalid_key());
    }
    for component in key.split('/') {
        validate_component(component)?;
    }
    Ok(())
}

fn validate_component(component: &str) -> io::Result<()> {
    if component.is_empty()
        || component == "."
        || component == ".."
        || component.ends_with(['.', ' '])
        || component
            .chars()
            .any(|c| c.is_control() || "\\/:*?\"<>|".contains(c))
    {
        return Err(invalid_key());
    }
    let stem = component
        .split('.')
        .next()
        .unwrap_or("")
        .to_ascii_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
    {
        return Err(invalid_key());
    }
    Ok(())
}

/// Object persistence boundary. Default log and media operations use only object
/// reads/replacements; implementations may optimize them without exposing paths.
///
/// `write` returns only after the complete replacement is readable by `read`;
/// readers must never see a partial object. Native HLS relies on this ordering
/// when it commits a segment before the manifest referencing it. This is not
/// a cross-object transaction, nor a promise to undo a completed remote write
/// when its caller is cancelled or loses the acknowledgement.
#[async_trait]
pub trait StorageBackend: Send + Sync {
    async fn read(&self, key: &str) -> io::Result<Vec<u8>>;
    /// Reject oversized objects with InvalidData while reading, before buffering
    /// the whole object. A metadata preflight alone is not a bound on allocation.
    async fn read_bounded(&self, key: &str, max_bytes: usize) -> io::Result<Vec<u8>>;
    /// Read at most `end - start` bytes, with an exclusive end. Short/empty
    /// reads at EOF are valid. Reject start > end with InvalidInput. Never
    /// buffer bytes outside the requested range (e.g. a whole video object).
    async fn read_range(&self, key: &str, range: std::ops::Range<u64>) -> io::Result<Vec<u8>>;
    /// Publish a complete replacement, never a partial object.
    async fn write(&self, key: &str, bytes: &[u8]) -> io::Result<()>;
    /// Transfer primitives: reject existing targets and use bounded file I/O.
    /// Local backends provide atomic no-clobber; S3 uses existence preflight
    /// under the application's single-writer transfer gate, not distributed CAS.
    /// Unsupported by default so custom backends never silently overwrite data.
    async fn create(&self, _key: &str, _bytes: &[u8]) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    async fn create_from_file(&self, _key: &str, _file: &Path) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    async fn download_to_file(&self, _key: &str, _file: &Path) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    async fn transfer_revision(&self, _key: &str) -> io::Result<String> {
        Err(io::ErrorKind::Unsupported.into())
    }
    /// Removing a missing key succeeds.
    async fn remove(&self, key: &str) -> io::Result<()>;
    async fn stat(&self, key: &str) -> io::Result<ObjectMetadata>;
    /// List a subtree (not a raw string prefix), with keys relative to this backend.
    /// Empty prefix means the root. Return at most `limit` keys, in lexical order.
    /// Cursors are opaque and must progress; missing subtrees return an empty page.
    /// Callers must not mutate the namespace while paginating.
    async fn list(
        &self,
        prefix: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> io::Result<ObjectPage>;

    /// Prepare a namespace if needed. Object stores normally need no preparation.
    async fn prepare(&self, prefix: &str) -> io::Result<()> {
        validate_prefix(prefix)
    }

    async fn append_media(&self, target: &str, fragment: &str) -> io::Result<()> {
        validate_key(target)?;
        validate_key(fragment)?;
        if target == fragment {
            return Err(invalid_key());
        }
        let mut bytes = self.read(target).await?;
        bytes.extend_from_slice(&self.read(fragment).await?);
        self.write(target, &bytes).await
    }

    async fn open_log(&self, key: &str) -> io::Result<()> {
        logs::open(self, key).await
    }

    /// Single writer per log, including separately opened handles. Successful
    /// appends are durable; implementations must not buffer data until flush/drop.
    async fn append_log(&self, key: &str, bytes: &[u8]) -> io::Result<()> {
        logs::append(self, key, bytes).await
    }

    async fn read_log(&self, key: &str) -> io::Result<Vec<u8>> {
        logs::read(self, key).await
    }

    async fn log_exists(&self, key: &str) -> io::Result<bool> {
        logs::exists(self, key).await
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectPage {
    pub keys: Vec<String>,
    pub next_cursor: Option<String>,
}

fn validate_prefix(prefix: &str) -> io::Result<()> {
    if prefix.is_empty() {
        Ok(())
    } else {
        validate_key(prefix)
    }
}

fn in_subtree(key: &str, prefix: &str) -> bool {
    prefix.is_empty()
        || key == prefix
        || key
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('/'))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObjectMetadata {
    pub size: u64,
}

#[derive(Clone)]
pub struct LocalBackend {
    root: PathBuf,
}

impl LocalBackend {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn path(&self, key: &str) -> io::Result<PathBuf> {
        safe_path(&self.root, key)
    }
}

#[async_trait]
impl StorageBackend for LocalBackend {
    async fn read(&self, key: &str) -> io::Result<Vec<u8>> {
        tokio::fs::read(self.path(key)?).await
    }

    async fn read_bounded(&self, key: &str, max_bytes: usize) -> io::Result<Vec<u8>> {
        let file = tokio::fs::File::open(self.path(key)?).await?;
        let mut bytes = Vec::new();
        file.take((max_bytes as u64).saturating_add(1))
            .read_to_end(&mut bytes)
            .await?;
        if bytes.len() > max_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "recording object exceeds read limit",
            ));
        }
        Ok(bytes)
    }

    async fn read_range(&self, key: &str, range: std::ops::Range<u64>) -> io::Result<Vec<u8>> {
        if range.start > range.end {
            return Err(invalid_key());
        }
        let mut file = tokio::fs::File::open(self.path(key)?).await?;
        file.seek(io::SeekFrom::Start(range.start)).await?;
        let mut bytes = Vec::new();
        file.take(range.end - range.start)
            .read_to_end(&mut bytes)
            .await?;
        Ok(bytes)
    }

    async fn write(&self, key: &str, bytes: &[u8]) -> io::Result<()> {
        atomic_write(self.path(key)?, bytes.to_vec()).await
    }

    async fn create(&self, key: &str, bytes: &[u8]) -> io::Result<()> {
        let path = self.path(key)?;
        let root = self.root.clone();
        let bytes = bytes.to_vec();
        tokio::task::spawn_blocking(move || {
            local_publication::create(&root, &path, |file| file.write_all(&bytes))
        })
        .await
        .map_err(io::Error::other)?
    }

    async fn create_from_file(&self, key: &str, file: &Path) -> io::Result<()> {
        let path = self.path(key)?;
        let root = self.root.clone();
        let source = tokio::fs::File::open(file).await?;
        // Move the open source handle into the blocking task, retaining the
        // bytes even if the staging pathname disappears during shutdown.
        let mut source = source.into_std().await;
        tokio::task::spawn_blocking(move || {
            local_publication::create(&root, &path, |file| {
                std::io::copy(&mut source, file).map(|_| ())
            })
        })
        .await
        .map_err(io::Error::other)?
    }

    async fn download_to_file(&self, key: &str, file: &Path) -> io::Result<()> {
        tokio::fs::copy(self.path(key)?, file).await?;
        Ok(())
    }

    async fn transfer_revision(&self, key: &str) -> io::Result<String> {
        transfer::local_file_revision(&self.path(key)?)
    }

    async fn remove(&self, key: &str) -> io::Result<()> {
        remove_file(self.path(key)?).await
    }

    async fn stat(&self, key: &str) -> io::Result<ObjectMetadata> {
        Ok(ObjectMetadata {
            size: tokio::fs::metadata(self.path(key)?).await?.len(),
        })
    }

    async fn prepare(&self, prefix: &str) -> io::Result<()> {
        validate_prefix(prefix)?;
        let path = if prefix.is_empty() {
            reject_symlink(&self.root)?;
            self.root.clone()
        } else {
            self.path(prefix)?
        };
        tokio::fs::create_dir_all(path).await
    }

    async fn list(
        &self,
        prefix: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> io::Result<ObjectPage> {
        validate_prefix(prefix)?;
        if limit == 0 {
            return Err(invalid_key());
        }
        if let Some(cursor) = cursor {
            validate_key(cursor)?;
        }
        let root = self.root.clone();
        let prefix = prefix.to_owned();
        let cursor = cursor.map(str::to_owned);
        tokio::task::spawn_blocking(move || {
            // Keep only one page in memory, even for a large local archive.
            let mut keys = std::collections::BTreeSet::new();
            let start = if prefix.is_empty() {
                reject_symlink(&root)?;
                root.clone()
            } else {
                safe_path(&root, &prefix)?
            };
            collect_local_keys(&root, &start, cursor.as_deref(), limit, &mut keys)?;
            let has_more = keys.len() > limit;
            if has_more {
                keys.pop_last();
            }
            let keys: Vec<_> = keys.into_iter().collect();
            let next_cursor = if has_more { keys.last().cloned() } else { None };
            Ok(ObjectPage { keys, next_cursor })
        })
        .await
        .map_err(io::Error::other)?
    }

    async fn append_media(&self, target: &str, fragment: &str) -> io::Result<()> {
        if target == fragment {
            return Err(invalid_key());
        }
        append_local_media(self.path(target)?, self.path(fragment)?).await
    }

    async fn open_log(&self, key: &str) -> io::Result<()> {
        if logs::count(self, key).await?.is_some() {
            return Ok(());
        }
        let path = self.path(key)?;
        tokio::task::spawn_blocking(move || {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)?
                .sync_all()
        })
        .await
        .map_err(io::Error::other)?
    }

    async fn append_log(&self, key: &str, bytes: &[u8]) -> io::Result<()> {
        // Archives copied from an object store retain their chunk ordering.
        if logs::count(self, key).await?.is_some() {
            return logs::append(self, key, bytes).await;
        }
        let path = self.path(key)?;
        let bytes = bytes.to_vec();
        tokio::task::spawn_blocking(move || {
            let mut file = std::fs::OpenOptions::new().append(true).open(path)?;
            file.write_all(&bytes)?;
            file.sync_all()
        })
        .await
        .map_err(io::Error::other)?
    }
}

fn reject_symlink(path: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "symlink in recording path",
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn safe_path(root: &Path, key: &str) -> io::Result<PathBuf> {
    validate_key(key)?;
    reject_symlink(root)?;
    let mut path = root.to_owned();
    for component in key.split('/') {
        path.push(component);
        reject_symlink(&path)?;
    }
    Ok(path)
}

fn collect_local_keys(
    root: &Path,
    path: &Path,
    cursor: Option<&str>,
    limit: usize,
    keys: &mut std::collections::BTreeSet<String>,
) -> io::Result<()> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "symlink in recording listing",
        ));
    }
    if metadata.is_dir() {
        for entry in std::fs::read_dir(path)? {
            collect_local_keys(root, &entry?.path(), cursor, limit, keys)?;
        }
    } else if metadata.is_file() {
        let relative = path.strip_prefix(root).map_err(io::Error::other)?;
        let key = relative
            .components()
            .map(|c| c.as_os_str().to_str().ok_or_else(invalid_key))
            .collect::<io::Result<Vec<_>>>()?
            .join("/");
        validate_key(&key)?;
        if cursor.is_none_or(|cursor| key.as_str() > cursor) {
            keys.insert(key);
            if keys.len() > limit.saturating_add(1) {
                keys.pop_last();
            }
        }
    } else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "non-file recording object",
        ));
    }
    Ok(())
}

async fn remove_file(path: PathBuf) -> io::Result<()> {
    match tokio::fs::remove_file(path).await {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

// Tempfiles live beside the destination, so persist is a same-filesystem atomic
// replacement (including on Windows). A failed publication keeps the old object.
async fn atomic_write(path: PathBuf, bytes: Vec<u8>) -> io::Result<()> {
    tokio::task::spawn_blocking(move || atomic_write_sync(path, &bytes))
        .await
        .map_err(io::Error::other)?
}

fn atomic_write_sync(path: PathBuf, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().ok_or_else(invalid_key)?;
    std::fs::create_dir_all(parent)?;
    let mut staged = tempfile::NamedTempFile::new_in(parent)?;
    staged.write_all(bytes)?;
    staged.as_file().sync_all()?;
    staged.persist(path).map_err(|error| error.error)?;
    Ok(())
}

/// Application metadata that must be durable before a recorder writes assets.
/// Standalone library users need no initializer. Initialization must be
/// idempotent because a recording can resume after a temporary failure.
#[async_trait]
pub trait RecordingInitializer: Send + Sync {
    async fn initialize(&self, platform: &str, room_id: &str, live_id: &str) -> io::Result<()>;
}

/// Application-owned policy, resolved for each recording attempt rather than
/// when the long-lived recorder is constructed.
#[async_trait]
pub trait RecordingResolver: Send + Sync {
    async fn resolve(
        &self,
        platform: &str,
        room_id: &str,
        live_id: &str,
    ) -> io::Result<RecordingStore>;
    async fn resolve_room(&self, platform: &str, room_id: &str) -> io::Result<RecordingStore>;
}

#[derive(Clone)]
struct RecordingPreparation {
    initializer: Arc<dyn RecordingInitializer>,
    platform: String,
    room_id: String,
    live_id: String,
}

type PendingLogs = Arc<std::sync::Mutex<Vec<std::sync::Weak<Mutex<()>>>>>;

async fn drain_logs(pending: &PendingLogs) {
    let locks: Vec<_> = pending
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .iter()
        .filter_map(std::sync::Weak::upgrade)
        .collect();
    for lock in locks {
        let _guard = lock.lock().await;
    }
}

#[derive(Clone)]
pub struct RecordingStore {
    root: PathBuf,
    backend: Arc<dyn StorageBackend>,
    identity: Option<String>,
    initializer: Option<Arc<dyn RecordingInitializer>>,
    pending_logs: PendingLogs,
    resolver: Option<Arc<dyn RecordingResolver>>,
    lease: Option<Arc<dyn Send + Sync>>,
}

impl RecordingStore {
    pub fn local(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            backend: Arc::new(LocalBackend::new(root.clone())),
            root,
            identity: None,
            initializer: None,
            pending_logs: PendingLogs::default(),
            resolver: None,
            lease: None,
        }
    }

    /// Inject a root object namespace. `workspace_root` is only a legacy adapter;
    /// no replica or directory is created there by normal storage operations.
    pub fn with_backend(
        backend: Arc<dyn StorageBackend>,
        workspace_root: impl Into<PathBuf>,
    ) -> Self {
        Self {
            root: workspace_root.into(),
            backend,
            identity: None,
            initializer: None,
            pending_logs: PendingLogs::default(),
            resolver: None,
            lease: None,
        }
    }

    pub fn with_identity(mut self, id: impl Into<String>) -> Self {
        self.identity = Some(id.into());
        self
    }

    pub fn identity(&self) -> Option<&str> {
        self.identity.as_deref()
    }

    pub fn with_initializer(mut self, initializer: Arc<dyn RecordingInitializer>) -> Self {
        self.initializer = Some(initializer);
        self
    }

    pub fn with_resolver(mut self, resolver: Arc<dyn RecordingResolver>) -> Self {
        self.resolver = Some(resolver);
        self
    }

    pub fn with_lease(mut self, lease: Arc<dyn Send + Sync>) -> Self {
        self.lease = Some(match self.lease.take() {
            Some(existing) => Arc::new((existing, lease)),
            None => lease,
        });
        self
    }

    pub async fn resolve_recording(
        &self,
        platform: &str,
        room_id: &str,
        live_id: &str,
    ) -> io::Result<RecordingStorage> {
        match &self.resolver {
            Some(resolver) => {
                let mut store = resolver.resolve(platform, room_id, live_id).await?;
                // Stop drains logs from every attempt, including cancelled ones.
                store.pending_logs = self.pending_logs.clone();
                store.recording(platform, room_id, live_id)
            }
            None => self.recording(platform, room_id, live_id),
        }
    }

    pub async fn resolve_room(
        &self,
        platform: &str,
        room_id: &str,
    ) -> io::Result<RecordingStorage> {
        validate_component(platform)?;
        validate_component(room_id)?;
        match &self.resolver {
            Some(resolver) => resolver
                .resolve_room(platform, room_id)
                .await?
                .scope(&format!("{platform}/{room_id}")),
            None => self.scope(&format!("{platform}/{room_id}")),
        }
    }

    /// After stopping event producers, wait for appends that survived cancellation.
    /// Weak handles also cover logs whose recorder-side owner has been dropped.
    pub async fn flush_logs(&self) {
        drain_logs(&self.pending_logs).await;
    }

    /// Compatibility bridge for legacy local cache readers, not an object key.
    pub fn local_root(&self) -> &Path {
        &self.root
    }

    pub fn scope(&self, prefix: &str) -> io::Result<RecordingStorage> {
        if self.resolver.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "dynamic recording storage must be resolved before accessing assets",
            ));
        }
        validate_key(prefix)?;
        let workspace = self.root.join(prefix);
        let mut storage = RecordingStorage::with_backend(
            Arc::new(ScopedBackend {
                backend: self.backend.clone(),
                prefix: prefix.to_owned(),
                _lease: self.lease.clone(),
            }),
            LocalWorkspace::new(workspace),
        );
        storage.pending_logs = self.pending_logs.clone();
        storage.identity = self.identity.clone();
        Ok(storage)
    }

    pub fn recording(
        &self,
        platform: &str,
        room_id: &str,
        live_id: &str,
    ) -> io::Result<RecordingStorage> {
        for component in [platform, room_id, live_id] {
            validate_component(component)?;
        }
        let mut storage = self.scope(&format!("{platform}/{room_id}/{live_id}"))?;
        storage.preparation = self
            .initializer
            .as_ref()
            .map(|initializer| RecordingPreparation {
                initializer: initializer.clone(),
                platform: platform.to_owned(),
                room_id: room_id.to_owned(),
                live_id: live_id.to_owned(),
            });
        Ok(storage)
    }
}

impl From<PathBuf> for RecordingStore {
    fn from(root: PathBuf) -> Self {
        Self::local(root)
    }
}

/// Scope the injected root backend once, rather than making every caller build
/// platform/room/live prefixes or teaching backends about recorder identities.
struct ScopedBackend {
    backend: Arc<dyn StorageBackend>,
    prefix: String,
    _lease: Option<Arc<dyn Send + Sync>>,
}

impl ScopedBackend {
    fn key(&self, key: &str) -> io::Result<String> {
        validate_key(key)?;
        Ok(format!("{}/{key}", self.prefix))
    }
}

#[async_trait]
impl StorageBackend for ScopedBackend {
    async fn read(&self, key: &str) -> io::Result<Vec<u8>> {
        self.backend.read(&self.key(key)?).await
    }

    async fn read_bounded(&self, key: &str, max_bytes: usize) -> io::Result<Vec<u8>> {
        self.backend.read_bounded(&self.key(key)?, max_bytes).await
    }

    async fn read_range(&self, key: &str, range: std::ops::Range<u64>) -> io::Result<Vec<u8>> {
        if range.start > range.end {
            return Err(invalid_key());
        }
        self.backend.read_range(&self.key(key)?, range).await
    }

    async fn write(&self, key: &str, bytes: &[u8]) -> io::Result<()> {
        self.backend.write(&self.key(key)?, bytes).await
    }

    async fn create(&self, key: &str, bytes: &[u8]) -> io::Result<()> {
        self.backend.create(&self.key(key)?, bytes).await
    }

    async fn create_from_file(&self, key: &str, file: &Path) -> io::Result<()> {
        self.backend.create_from_file(&self.key(key)?, file).await
    }

    async fn download_to_file(&self, key: &str, file: &Path) -> io::Result<()> {
        self.backend.download_to_file(&self.key(key)?, file).await
    }

    async fn transfer_revision(&self, key: &str) -> io::Result<String> {
        self.backend.transfer_revision(&self.key(key)?).await
    }

    async fn remove(&self, key: &str) -> io::Result<()> {
        self.backend.remove(&self.key(key)?).await
    }

    async fn stat(&self, key: &str) -> io::Result<ObjectMetadata> {
        self.backend.stat(&self.key(key)?).await
    }

    async fn prepare(&self, prefix: &str) -> io::Result<()> {
        validate_prefix(prefix)?;
        let prefix = if prefix.is_empty() {
            self.prefix.clone()
        } else {
            self.key(prefix)?
        };
        self.backend.prepare(&prefix).await
    }

    async fn list(
        &self,
        prefix: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> io::Result<ObjectPage> {
        validate_prefix(prefix)?;
        let full_prefix = if prefix.is_empty() {
            self.prefix.clone()
        } else {
            self.key(prefix)?
        };
        let page = self.backend.list(&full_prefix, cursor, limit).await?;
        let delimiter = format!("{}/", self.prefix);
        let keys = page
            .keys
            .into_iter()
            .map(|key| {
                let key = key.strip_prefix(&delimiter).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "backend returned a key outside the recording",
                    )
                })?;
                validate_key(key)?;
                if !in_subtree(key, prefix) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "backend returned an unrelated key",
                    ));
                }
                Ok(key.to_owned())
            })
            .collect::<io::Result<_>>()?;
        Ok(ObjectPage {
            keys,
            next_cursor: page.next_cursor,
        })
    }

    async fn append_media(&self, target: &str, fragment: &str) -> io::Result<()> {
        self.backend
            .append_media(&self.key(target)?, &self.key(fragment)?)
            .await
    }

    async fn open_log(&self, key: &str) -> io::Result<()> {
        self.backend.open_log(&self.key(key)?).await
    }

    async fn append_log(&self, key: &str, bytes: &[u8]) -> io::Result<()> {
        self.backend.append_log(&self.key(key)?, bytes).await
    }

    async fn read_log(&self, key: &str) -> io::Result<Vec<u8>> {
        self.backend.read_log(&self.key(key)?).await
    }

    async fn log_exists(&self, key: &str) -> io::Result<bool> {
        self.backend.log_exists(&self.key(key)?).await
    }
}

#[derive(Clone)]
pub struct RecordingStorage {
    backend: Arc<dyn StorageBackend>,
    workspace: LocalWorkspace,
    preparation: Option<RecordingPreparation>,
    pending_logs: PendingLogs,
    identity: Option<String>,
}

impl RecordingStorage {
    pub fn identity(&self) -> Option<&str> {
        self.identity.as_deref()
    }

    pub fn local(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        Self {
            backend: Arc::new(LocalBackend::new(dir.clone())),
            workspace: LocalWorkspace::new(dir),
            preparation: None,
            pending_logs: PendingLogs::default(),
            identity: None,
        }
    }

    /// Substitute persistence while retaining an explicit legacy local adapter.
    /// Normal asset and log operations never create a local replica.
    pub fn with_backend(backend: Arc<dyn StorageBackend>, workspace: LocalWorkspace) -> Self {
        Self {
            backend,
            workspace,
            preparation: None,
            pending_logs: PendingLogs::default(),
            identity: None,
        }
    }

    pub async fn prepare(&self) -> io::Result<()> {
        if let Some(preparation) = &self.preparation {
            preparation
                .initializer
                .initialize(
                    &preparation.platform,
                    &preparation.room_id,
                    &preparation.live_id,
                )
                .await?;
        }
        self.backend.prepare("").await
    }

    pub async fn read(&self, key: &str) -> io::Result<Vec<u8>> {
        validate_key(key)?;
        self.backend.read(key).await
    }

    pub async fn read_bounded(&self, key: &str, max_bytes: usize) -> io::Result<Vec<u8>> {
        validate_key(key)?;
        self.backend.read_bounded(key, max_bytes).await
    }

    /// Read a bounded byte range; the end is exclusive and reads may be short at EOF.
    pub async fn read_range(&self, key: &str, range: std::ops::Range<u64>) -> io::Result<Vec<u8>> {
        validate_key(key)?;
        if range.start > range.end {
            return Err(invalid_key());
        }
        self.backend.read_range(key, range).await
    }

    pub async fn write(&self, key: &str, bytes: &[u8]) -> io::Result<()> {
        validate_key(key)?;
        self.backend.write(key, bytes).await
    }

    /// Repair media by replacing the target with the concatenated objects.
    /// Callers must serialize writes to the recording and remove the fragment
    /// only after success. This is not a transaction with source removal or
    /// playlist/checkpoint updates; backends must atomically replace objects.
    pub async fn append_media(&self, target: &str, fragment: &str) -> io::Result<()> {
        validate_key(target)?;
        validate_key(fragment)?;
        if target == fragment {
            return Err(invalid_key());
        }
        self.backend.append_media(target, fragment).await
    }

    pub async fn remove(&self, key: &str) -> io::Result<()> {
        validate_key(key)?;
        match self.backend.remove(key).await {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            result => result,
        }
    }

    pub async fn size(&self, key: &str) -> io::Result<u64> {
        validate_key(key)?;
        Ok(self.backend.stat(key).await?.size)
    }

    pub async fn exists(&self, key: &str) -> io::Result<bool> {
        match self.size(key).await {
            Ok(_) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub fn workspace(&self) -> &LocalWorkspace {
        &self.workspace
    }

    /// Track an asynchronous log producer, including its final buffered writes
    /// after the caller is cancelled. Stop/drop the producer before calling
    /// `RecordingStore::flush_logs`; the drain waits for the entire task.
    pub fn spawn_log_task<F>(&self, future: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: std::future::Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let lock = Arc::new(Mutex::new(()));
        let guard = lock.clone().try_lock_owned().expect("new log task lock");
        {
            let mut pending = self.pending_logs.lock().unwrap_or_else(|p| p.into_inner());
            pending.retain(|entry| entry.strong_count() != 0);
            pending.push(Arc::downgrade(&lock));
        }
        tokio::spawn(async move {
            let _guard = guard;
            future.await
        })
    }

    pub async fn open_log(&self, key: &str) -> io::Result<AppendLog> {
        validate_key(key)?;
        self.backend.open_log(key).await?;
        let lock = Arc::new(Mutex::new(()));
        {
            let mut pending = self.pending_logs.lock().unwrap_or_else(|p| p.into_inner());
            pending.retain(|entry| entry.strong_count() != 0);
            pending.push(Arc::downgrade(&lock));
        }
        Ok(AppendLog {
            backend: self.backend.clone(),
            key: key.to_owned(),
            lock,
        })
    }

    pub async fn read_log(&self, key: &str) -> io::Result<Vec<u8>> {
        validate_key(key)?;
        self.backend.read_log(key).await
    }

    pub async fn log_exists(&self, key: &str) -> io::Result<bool> {
        validate_key(key)?;
        self.backend.log_exists(key).await
    }

    pub async fn list(
        &self,
        prefix: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> io::Result<ObjectPage> {
        validate_prefix(prefix)?;
        if limit == 0 {
            return Err(invalid_key());
        }
        let page = self.backend.list(prefix, cursor, limit).await?;
        if page.keys.len() > limit
            || page
                .next_cursor
                .as_deref()
                .is_some_and(|next| Some(next) == cursor)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid backend page",
            ));
        }
        for key in &page.keys {
            validate_key(key)?;
            if !in_subtree(key, prefix) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "key outside requested subtree",
                ));
            }
        }
        Ok(page)
    }

    /// Retryable, idempotent deletion, not a transaction. Stop on the first
    /// failure; earlier deletions remain applied. Caller must stop writers first.
    pub async fn remove_all(&self) -> io::Result<()> {
        loop {
            // Restart listing after each batch, avoiding cursors invalidated by
            // removal. This also includes orphan log chunks from failed appends.
            let page = self.list("", None, 256).await?;
            if page.keys.is_empty() {
                if page.next_cursor.is_some() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "empty nonterminal page",
                    ));
                }
                return Ok(());
            }
            for key in page.keys {
                self.remove(&key).await?;
            }
        }
    }

    /// Copy only the requested objects to an isolated lease for path-based tools.
    pub async fn materialize(&self, keys: &[String]) -> io::Result<LocalAssets> {
        for key in keys {
            validate_key(key)?;
        }
        let assets = LocalAssets::new().await?;
        for key in keys {
            let size = self.size(key).await?;
            assets.write(key, b"").await?;
            let mut offset = 0;
            while offset < size {
                let end = offset.saturating_add(1024 * 1024).min(size);
                let bytes = self.read_range(key, offset..end).await?;
                if bytes.len() as u64 != end - offset {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "recording asset changed during materialization",
                    ));
                }
                assets.append_chunk(key, &bytes).await?;
                offset = end;
            }
        }
        Ok(assets)
    }

    /// Explicit legacy bridge: publish a completed workspace file as one object.
    pub async fn publish_media(&self, key: &str) -> io::Result<()> {
        validate_key(key)?;
        self.write(key, &self.workspace.read(key).await?).await
    }
}

impl From<PathBuf> for RecordingStorage {
    fn from(dir: PathBuf) -> Self {
        Self::local(dir)
    }
}

/// Explicit TEMP/legacy adapter for path-based tools, not persistent asset
/// storage. New consumers should use `LocalAssets` with a bounded lease lifetime.
#[derive(Clone)]
pub struct LocalWorkspace {
    root: PathBuf,
}

impl LocalWorkspace {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn path(&self, key: &str) -> io::Result<PathBuf> {
        safe_path(&self.root, key)
    }

    pub async fn prepare(&self) -> io::Result<()> {
        tokio::fs::create_dir_all(&self.root).await
    }

    pub async fn read(&self, key: &str) -> io::Result<Vec<u8>> {
        tokio::fs::read(self.path(key)?).await
    }

    /// Stage a complete downloaded media file before native probing. Readers
    /// never observe partially downloaded bytes, even when replacing a retry.
    pub async fn stage_media(&self, key: &str, bytes: &[u8]) -> io::Result<()> {
        atomic_write(self.path(key)?, bytes.to_vec()).await
    }

    pub async fn remove(&self, key: &str) -> io::Result<()> {
        remove_file(self.path(key)?).await
    }

    /// Repair a TS fragment lacking SPS/PPS by appending it to its predecessor.
    /// Replacement is atomic for the predecessor, not with source removal or
    /// playlist/checkpoint updates. Callers serialize repairs per recording.
    pub async fn append_media(&self, target: &str, fragment: &str) -> io::Result<()> {
        if target == fragment {
            return Err(invalid_key());
        }
        let target = self.path(target)?;
        let fragment = self.path(fragment)?;
        append_local_media(target, fragment).await
    }
}

async fn append_local_media(target: PathBuf, fragment: PathBuf) -> io::Result<()> {
    tokio::task::spawn_blocking(move || {
        let mut previous = std::fs::File::open(&target)?;
        let mut fragment = std::fs::File::open(fragment)?;
        let mut staged = tempfile::NamedTempFile::new_in(target.parent().ok_or_else(invalid_key)?)?;
        io::copy(&mut previous, &mut staged)?;
        io::copy(&mut fragment, &mut staged)?;
        staged.as_file().sync_all()?;
        // Close readers before replacing their source on Windows.
        drop(previous);
        staged.persist(target).map_err(|error| error.error)?;
        Ok(())
    })
    .await
    .map_err(io::Error::other)?
}

/// Isolated local files kept alive until the lease is dropped. Neither writes nor
/// cleanup touch the archive backend. Native consumers must retain this lease.
#[derive(Clone)]
pub struct LocalAssets {
    directory: Arc<tempfile::TempDir>,
}

impl LocalAssets {
    pub async fn new() -> io::Result<Self> {
        let directory = tokio::task::spawn_blocking(tempfile::tempdir)
            .await
            .map_err(io::Error::other)??;
        Ok(Self {
            directory: Arc::new(directory),
        })
    }

    pub fn path(&self, key: &str) -> io::Result<PathBuf> {
        safe_path(self.directory.path(), key)
    }

    pub async fn write(&self, key: &str, bytes: &[u8]) -> io::Result<()> {
        let path = self.path(key)?;
        let bytes = bytes.to_vec();
        // Cancellation must not drop the tempdir while blocking file I/O is
        // still running (and let it recreate leaked directories after cleanup).
        let directory = self.directory.clone();
        tokio::task::spawn_blocking(move || {
            let _lease = directory;
            atomic_write_sync(path, &bytes)
        })
        .await
        .map_err(io::Error::other)?
    }

    async fn append_chunk(&self, key: &str, bytes: &[u8]) -> io::Result<()> {
        let path = self.path(key)?;
        let bytes = bytes.to_vec();
        let directory = self.directory.clone();
        tokio::task::spawn_blocking(move || {
            let _lease = directory;
            let mut file = std::fs::OpenOptions::new().append(true).open(path)?;
            file.write_all(&bytes)
        })
        .await
        .map_err(io::Error::other)?
    }

    pub async fn read(&self, key: &str) -> io::Result<Vec<u8>> {
        tokio::fs::read(self.path(key)?).await
    }
}

/// Backend-owned append log. Clones serialize complete appends. Separate opens
/// require a single writer. Every successful append is durable, even without
/// flush. Cancellation does not cancel an in-flight write; it finishes holding
/// the lock, so the next append/flush cannot race its publication.
///
/// The default object format preserves the legacy base and writes chunks of at
/// most [`LOG_CHUNK_BYTES`] bytes beneath the reserved `.recording-logs`
/// directory alongside the log. Each nonempty append costs one eight-byte
/// metadata GET, `ceil(bytes / LOG_CHUNK_BYTES)` chunk PUTs, and one metadata
/// PUT committing the chunk count. Reads concatenate the base and committed
/// chunks (one GET each), with memory proportional to the whole log. Failed
/// appends may have committed if acknowledgement was lost; uncommitted chunks
/// are ignored and reused. No native remote append/seek capability is needed.
/// LocalBackend may instead append and sync a plain file.
#[derive(Clone)]
pub struct AppendLog {
    backend: Arc<dyn StorageBackend>,
    key: String,
    lock: Arc<Mutex<()>>,
}

impl AppendLog {
    /// Legacy local path wrapper; parent directories must already exist.
    pub async fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();
        let key = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(invalid_key)?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        RecordingStorage::local(parent).open_log(key).await
    }

    pub async fn append(&self, bytes: &[u8]) -> io::Result<()> {
        let guard = self.lock.clone().lock_owned().await;
        let backend = self.backend.clone();
        let key = self.key.clone();
        let bytes = bytes.to_vec();
        tokio::spawn(async move {
            let _guard = guard;
            backend.append_log(&key, &bytes).await
        })
        .await
        .map_err(io::Error::other)?
    }

    pub async fn flush(&self) -> io::Result<()> {
        let _guard = self.lock.lock().await;
        Ok(())
    }
}
