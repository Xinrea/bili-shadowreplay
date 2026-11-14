//! Resolve recorder selections and immutable archive bindings at the app edge.
//! Backend IDs are not paths. Missing definitions never fall back to another root.
use std::collections::{BTreeSet, HashMap};
use std::io;
use std::sync::Arc;

use recording_storage::{
    RecordingInitializer, RecordingResolver, RecordingStorage, RecordingStore,
};
use tokio::sync::RwLock;

use crate::config::{Config, StorageConfig, DEFAULT_RECORDING_STORAGE_ID};
use crate::database::{Database, DatabaseError};

type ArchiveLocks = std::sync::Mutex<HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>>;
type ArchiveReaders = std::sync::Mutex<HashMap<String, std::sync::Weak<RwLock<()>>>>;

#[path = "storage_transfer.rs"]
mod transfer;
pub use transfer::StorageTransferPreview;

#[path = "archive_storage/catalog.rs"]
mod catalog;

struct RecordingLease {
    _mutation: tokio::sync::OwnedMutexGuard<()>,
}

pub struct ArchiveStorage {
    db: Arc<Database>,
    config: Arc<RwLock<Config>>,
    registered: RwLock<HashMap<String, RecordingStore>>,
    mutations: ArchiveLocks,
    initialized: tokio::sync::OnceCell<()>,
    configuration_gate: Arc<RwLock<()>>,
    active: std::sync::Mutex<HashMap<String, Vec<std::sync::Weak<RecordingLease>>>>,
    transfer_gate: Arc<tokio::sync::Mutex<()>>,
    pending_transfers: std::sync::Mutex<BTreeSet<String>>,
    readers: ArchiveReaders,
    catalog_started: std::sync::atomic::AtomicBool,
    catalog_readonly: std::sync::atomic::AtomicBool,
    catalog_startup_deferred: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    transfer_available_bytes: std::sync::Mutex<Option<u64>>,
    #[cfg(test)]
    transfer_force_copy: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    transfer_rename_error: std::sync::Mutex<Option<i32>>,
}

struct ResolveRecording {
    storage: Arc<ArchiveStorage>,
}

#[async_trait::async_trait]
impl RecordingResolver for ResolveRecording {
    async fn resolve_room(&self, platform: &str, room_id: &str) -> io::Result<RecordingStore> {
        self.storage.recorder_store(platform, room_id).await
    }

    async fn resolve(
        &self,
        platform: &str,
        room_id: &str,
        live_id: &str,
    ) -> io::Result<RecordingStore> {
        self.storage.initialize().await?;
        // Cover setup and pending log/native writes, not just is_recording=true.
        let mutation = self
            .storage
            .try_lock_recording(platform, room_id, live_id)?;
        let _gate = self.storage.configuration_gate.read().await;
        let bound = self
            .storage
            .db
            .recording_storage_id(platform, room_id, live_id)
            .await
            .map_err(database_error)?;
        let store = match bound {
            Some(id) => self.storage.store(&id).await?,
            None => self.storage.recorder_store(platform, room_id).await?,
        };
        let id = store
            .identity()
            .expect("resolved stores have an identity")
            .to_owned();
        if let Some(runtime) = self.storage.db.catalogs.get() {
            runtime
                .validate_recording(&id, platform, room_id, live_id)
                .await
                .map_err(database_error)?;
        }
        self.storage
            .db
            .bind_recording_storage(platform, room_id, live_id, &id)
            .await
            .map_err(database_error)?;
        let lease = Arc::new(RecordingLease {
            _mutation: mutation,
        });
        let mut active = self
            .storage
            .active
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let leases = active.entry(id).or_default();
        leases.retain(|lease| lease.strong_count() > 0);
        leases.push(Arc::downgrade(&lease));
        Ok(store.with_lease(lease))
    }
}

struct PersistRecordingBinding {
    db: Arc<Database>,
    storage_id: String,
}

#[async_trait::async_trait]
impl RecordingInitializer for PersistRecordingBinding {
    async fn initialize(&self, platform: &str, room_id: &str, live_id: &str) -> io::Result<()> {
        self.db
            .bind_recording_storage(platform, room_id, live_id, &self.storage_id)
            .await
            .map_err(database_error)
    }
}

fn database_error(error: DatabaseError) -> io::Error {
    let kind = match &error {
        DatabaseError::RecordingDeleted => io::ErrorKind::NotFound,
        DatabaseError::StorageBindingConflict | DatabaseError::StorageBackendConflict(_) => {
            io::ErrorKind::AlreadyExists
        }
        _ => io::ErrorKind::Other,
    };
    io::Error::new(kind, error)
}

fn validate_component(value: &str) -> io::Result<()> {
    if value.contains('/') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "storage identities must be single path components",
        ));
    }
    recording_storage::validate_key(value)
}

impl ArchiveStorage {
    pub fn new(db: Arc<Database>, config: Arc<RwLock<Config>>) -> Self {
        Self {
            db,
            config,
            registered: RwLock::new(HashMap::new()),
            mutations: ArchiveLocks::default(),
            initialized: tokio::sync::OnceCell::new(),
            configuration_gate: Arc::new(RwLock::new(())),
            active: std::sync::Mutex::new(HashMap::new()),
            transfer_gate: Arc::new(tokio::sync::Mutex::new(())),
            pending_transfers: std::sync::Mutex::new(BTreeSet::new()),
            readers: ArchiveReaders::default(),
            catalog_started: std::sync::atomic::AtomicBool::new(false),
            catalog_readonly: std::sync::atomic::AtomicBool::new(false),
            catalog_startup_deferred: std::sync::atomic::AtomicBool::new(false),
            #[cfg(test)]
            transfer_available_bytes: std::sync::Mutex::new(None),
            #[cfg(test)]
            transfer_force_copy: std::sync::atomic::AtomicBool::new(false),
            #[cfg(test)]
            transfer_rename_error: std::sync::Mutex::new(None),
        }
    }

    pub async fn initialize(&self) -> io::Result<()> {
        self.initialized
            .get_or_try_init(|| async {
                // Read DB first: an old TOML file must never resurrect a deleted ID.
                match self.db.storage_config().await {
                    Ok(_) => {}
                    Err(DatabaseError::DB(sqlx::Error::RowNotFound)) => {
                        let seed =
                            Self::normalize_config(self.config.read().await.storage.clone())?;
                        self.db
                            .initialize_storage_config(&seed)
                            .await
                            .map_err(database_error)?;
                    }
                    Err(error) => return Err(database_error(error)),
                }
                let snapshot = self.db.storage_config().await.map_err(database_error)?;
                let pending = self
                    .db
                    .storage_transfer_keys()
                    .await
                    .map_err(database_error)?;
                self.pending_transfers
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .extend(pending);
                let mut config = self.config.write().await;
                config.storage = snapshot;
                config.storage_db_initialized = true;
                Ok::<_, io::Error>(())
            })
            .await?;
        Ok(())
    }

    fn normalize_config(mut storage: StorageConfig) -> io::Result<StorageConfig> {
        validate_component(&storage.default_backend)?;
        for (id, definition) in &mut storage.backends {
            validate_component(id)?;
            *definition = definition.normalized()?;
        }
        Ok(storage)
    }

    pub async fn configuration(&self) -> io::Result<StorageConfig> {
        self.initialize().await?;
        self.db.storage_config().await.map_err(database_error)
    }

    #[cfg(test)]
    pub async fn replace_configuration(&self, storage: StorageConfig) -> io::Result<StorageConfig> {
        let expected = self.configuration().await?;
        self.replace_configuration_if_current(storage, &expected)
            .await
    }

    #[cfg(test)]
    pub async fn replace_configuration_if_current(
        &self,
        storage: StorageConfig,
        expected: &StorageConfig,
    ) -> io::Result<StorageConfig> {
        self.replace_configuration_with_credentials(storage, expected, &HashMap::new())
            .await
    }

    pub async fn replace_configuration_with_credentials(
        &self,
        storage: StorageConfig,
        expected: &StorageConfig,
        credentials: &HashMap<String, recording_storage::StorageCredentials>,
    ) -> io::Result<StorageConfig> {
        self.initialize().await?;
        let next = Self::normalize_config(storage)?;
        let _gate = self.configuration_gate.try_write().map_err(|_| {
            io::Error::new(io::ErrorKind::WouldBlock, "存储正在使用中，请稍后修改设置")
        })?;
        let current = self.db.storage_config().await.map_err(database_error)?;
        if expected != &current {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "storage configuration changed; reload settings before saving",
            ));
        }
        self.validate_config(&current, &next).await?;
        if self
            .catalog_started
            .load(std::sync::atomic::Ordering::Acquire)
            && self.db.catalogs.get().is_none()
            && current
                .backends
                .keys()
                .any(|id| !next.backends.contains_key(id))
        {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "请先完成旧版本转移并重启应用，再卸载存储",
            ));
        }
        for journal in self
            .db
            .storage_transfer_journals()
            .await
            .map_err(database_error)?
        {
            if !next.backends.contains_key(&journal.source_id)
                || !next.backends.contains_key(&journal.target_id)
            {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "存储转移尚未完成，请使用原来源和目标重试后再移除后端",
                ));
            }
        }
        {
            let mut active = self.active.lock().unwrap_or_else(|p| p.into_inner());
            for (id, leases) in active.iter_mut() {
                leases.retain(|lease| lease.strong_count() > 0);
                if current.backends.contains_key(id)
                    && (!next.backends.contains_key(id) || credentials.contains_key(id))
                    && !leases.is_empty()
                {
                    return Err(io::Error::new(
                        io::ErrorKind::WouldBlock,
                        format!(
                            "storage backend {id} is in use by a recording; stop it before removal or credential update"
                        ),
                    ));
                }
            }
        }
        if self.db.catalogs.get().is_some() {
            for id in current.backends.keys() {
                if (!next.backends.contains_key(id) || credentials.contains_key(id))
                    && self.backend_in_use(id).await?
                {
                    return Err(io::Error::new(
                        io::ErrorKind::WouldBlock,
                        "存储仍有录播处理任务，请结束后再卸载或更新凭证",
                    ));
                }
            }
        }
        self.db
            .replace_storage_config(&next, credentials)
            .await
            .map_err(database_error)?;
        let saved = self.db.storage_config().await.map_err(database_error)?;
        self.config.write().await.storage = saved.clone();
        if let Some(runtime) = self.db.catalogs.get() {
            for id in current.backends.keys() {
                if !saved.backends.contains_key(id) || credentials.contains_key(id) {
                    runtime.detach(id).await.map_err(database_error)?;
                }
            }
        }
        if self
            .catalog_started
            .load(std::sync::atomic::Ordering::Acquire)
        {
            self.reconcile_catalogs(false).await?;
        }
        Ok(saved)
    }

    pub fn recording_writer(self: &Arc<Self>) -> RecordingStore {
        // The placeholder is never used by recorder writes: resolve_recording
        // resolves a fresh DB selection before returning the actual namespace.
        RecordingStore::local("").with_resolver(Arc::new(ResolveRecording {
            storage: self.clone(),
        }))
    }

    /// Validate while the caller holds the config write lock. Do not read
    /// self.config or reserve new IDs: a failed config save must have no effect.
    pub async fn validate_config(
        &self,
        _current: &StorageConfig,
        next: &StorageConfig,
    ) -> io::Result<()> {
        validate_component(&next.default_backend)?;
        let registered = self.registered.read().await;
        for (id, definition) in &next.backends {
            validate_component(id)?;
            if self
                .catalog_started
                .load(std::sync::atomic::Ordering::Acquire)
            {
                catalog::validate_catalog_namespace(id, next)?;
            }
            if registered.contains_key(id) {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("storage ID {id} is already registered"),
                ));
            }
            let location = definition.location()?;
            self.db
                .validate_storage_backend(id, &definition.kind, &location)
                .await
                .map_err(database_error)?;
        }
        if !next.backends.contains_key(&next.default_backend)
            && !registered.contains_key(&next.default_backend)
        {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "default recording storage backend {} is not configured or registered",
                    next.default_backend
                ),
            ));
        }
        Ok(())
    }

    /// Serialize destructive changes and derived-asset publication. Refuse a
    /// conflicting operation instead of waiting behind a long transcription.
    pub fn try_lock_recording(
        &self,
        platform: &str,
        room_id: &str,
        live_id: &str,
    ) -> io::Result<tokio::sync::OwnedMutexGuard<()>> {
        let _configuration = self
            .configuration_gate
            .try_read()
            .map_err(|_| io::Error::new(io::ErrorKind::WouldBlock, "存储设置正在更新"))?;
        if let Some(runtime) = self.db.catalogs.get() {
            runtime
                .check_mutation(platform, room_id, live_id)
                .map_err(database_error)?;
        }
        let key = format!("{platform}/{room_id}/{live_id}");
        let pending = self
            .pending_transfers
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if pending.contains(&key) {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "录制有未完成的存储转移，请使用原来源和目标重试",
            ));
        }
        self.try_lock_recording_for_transfer(platform, room_id, live_id)
    }

    fn try_lock_recording_for_transfer(
        &self,
        platform: &str,
        room_id: &str,
        live_id: &str,
    ) -> io::Result<tokio::sync::OwnedMutexGuard<()>> {
        for component in [platform, room_id, live_id] {
            validate_component(component)?;
        }
        let key = format!("{platform}/{room_id}/{live_id}");
        let lock = {
            let mut locks = self.mutations.lock().unwrap_or_else(|p| p.into_inner());
            locks.retain(|_, weak| weak.strong_count() != 0);
            match locks.get(&key).and_then(std::sync::Weak::upgrade) {
                Some(lock) => lock,
                None => {
                    let lock = Arc::new(tokio::sync::Mutex::new(()));
                    locks.insert(key, Arc::downgrade(&lock));
                    lock
                }
            }
        };
        lock.try_lock_owned().map_err(|_| {
            io::Error::new(
                io::ErrorKind::WouldBlock,
                "recording has an active mutation",
            )
        })
    }

    fn reader_barrier(&self, key: &str) -> Arc<RwLock<()>> {
        let mut readers = self.readers.lock().unwrap_or_else(|p| p.into_inner());
        readers.retain(|_, weak| weak.strong_count() != 0);
        match readers.get(key).and_then(std::sync::Weak::upgrade) {
            Some(lock) => lock,
            None => {
                let lock = Arc::new(RwLock::new(()));
                readers.insert(key.into(), Arc::downgrade(&lock));
                lock
            }
        }
    }

    /// Register a programmatic adapter before selecting it. Registration cannot
    /// replace a configured backend or another registered adapter.
    /// The adapter owner must preserve the ID's object namespace across restarts;
    /// credentials and adapter configuration are never stored in the archive DB.
    #[allow(dead_code)] // Extension entry point for programmatically supplied adapters.
    pub async fn register_backend(&self, id: &str, store: RecordingStore) -> io::Result<()> {
        validate_component(id)?;
        let config = self.configuration().await?;
        let mut registered = self.registered.write().await;
        if config.backends.contains_key(id) || registered.contains_key(id) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("storage ID {id} is already defined"),
            ));
        }
        self.db
            .remember_storage_backend(id, "registered", "")
            .await
            .map_err(database_error)?;
        registered.insert(id.to_owned(), store.with_identity(id));
        Ok(())
    }

    async fn store(&self, id: &str) -> io::Result<RecordingStore> {
        validate_component(id)?;
        let config = self.configuration().await?;
        let registered = self.registered.read().await;
        let definition = config.backends.get(id);
        if let Some(store) = registered.get(id) {
            if definition.is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("storage ID {id} is both configured and registered"),
                ));
            }
            return Ok(store.clone());
        }
        let definition = definition.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("recording storage backend {id} is not configured or registered"),
            )
        })?;
        // Absolute paths also detect accidental reassignment of relative roots
        // when the process is launched with a different working directory.
        let definition = definition.normalized()?;
        let location = definition.location()?;
        self.db
            .remember_storage_backend(id, &definition.kind, &location)
            .await
            .map_err(database_error)?;
        let store = if let Some(webdav) = &definition.webdav {
            let credentials = self
                .db
                .storage_credentials(id)
                .await
                .map_err(database_error)?;
            RecordingStore::with_backend(
                Arc::new(recording_storage::WebDavBackend::new(
                    webdav,
                    credentials.webdav()?,
                )?),
                std::env::temp_dir().join("shadowreplay-webdav").join(id),
            )
        } else {
            match &definition.s3 {
                Some(s3) => RecordingStore::with_backend(
                    Arc::new(recording_storage::S3Backend::new(
                        s3,
                        self.db
                            .storage_credentials(id)
                            .await
                            .map_err(database_error)?
                            .s3()?,
                    )?),
                    // Compatibility path only; all normal asset processing uses
                    // explicit LocalAssets leases, never a persistent local replica.
                    std::env::temp_dir().join("shadowreplay-s3").join(id),
                ),
                None => RecordingStore::local(&definition.root),
            }
        };
        Ok(store.with_identity(id))
    }

    pub async fn recorder_store(
        &self,
        platform: &str,
        room_id: &str,
    ) -> io::Result<RecordingStore> {
        validate_component(platform)?;
        validate_component(room_id)?;
        let id = self
            .db
            .recorder_storage_id(platform, room_id)
            .await
            .map_err(database_error)?;
        self.selected_store(id.as_deref()).await
    }

    /// Resolve a selection before a new recorder row exists. The initializer
    /// captures this exact ID and pins it before any recording asset is written.
    pub async fn selected_store(&self, id: Option<&str>) -> io::Result<RecordingStore> {
        let id = match id {
            Some(id) => id.to_owned(),
            None => self.configuration().await?.default_backend,
        };
        if let Some(runtime) = self.db.catalogs.get() {
            runtime.writable(&id).await.map_err(database_error)?;
        }
        Ok(self
            .store(&id)
            .await?
            .with_initializer(Arc::new(PersistRecordingBinding {
                db: self.db.clone(),
                storage_id: id,
            })))
    }

    pub async fn recording(
        &self,
        platform: &str,
        room_id: &str,
        live_id: &str,
    ) -> io::Result<RecordingStorage> {
        self.initialize().await?;
        let _configuration = self.configuration_gate.read().await;
        // Fail fast even for a queued transfer. Awaiting here could deadlock a
        // caller that already owns a read lease and resolves another scope;
        // the queued writer must drain that original lease before admitting it.
        let read_lease = self
            .reader_barrier(&format!("{platform}/{room_id}/{live_id}"))
            .try_read_owned()
            .map_err(|_| io::Error::new(io::ErrorKind::WouldBlock, "录制正在转移，请稍后读取"))?;
        if self
            .db
            .recording_awaits_rename_binding(platform, room_id, live_id)
            .await
            .map_err(database_error)?
        {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "录制有未完成的快速移动，请使用原来源和目标重试转移以恢复读取",
            ));
        }
        let id = self
            .db
            .recording_storage_id(platform, room_id, live_id)
            .await
            .map_err(database_error)?;
        self.initialize().await?;
        let id = match id {
            Some(id) => id,
            None if self.db.catalogs.get().is_some() => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "录播不在已挂载的目录中",
                ));
            }
            None => self.db.legacy_storage_id().await.map_err(database_error)?,
        };
        if let Some(runtime) = self.db.catalogs.get() {
            runtime.readable(&id).await.map_err(database_error)?;
        }
        self.store(&id)
            .await?
            .with_lease(Arc::new(read_lease))
            .recording(platform, room_id, live_id)
    }

    pub async fn room(&self, platform: &str, room_id: &str) -> io::Result<RecordingStorage> {
        self.recorder_store(platform, room_id)
            .await?
            .scope(&format!("{platform}/{room_id}"))
    }

    /// Gather room-cover scopes before deleting/forgetting historical archives.
    /// Include the legacy backend when defined or required by unbound archives.
    /// A missing historical backend is an error, not partial cleanup.
    pub async fn room_stores(
        &self,
        platform: &str,
        room_id: &str,
    ) -> io::Result<Vec<RecordingStorage>> {
        validate_component(platform)?;
        validate_component(room_id)?;
        let config = self.configuration().await?;
        let legacy_id = self.db.legacy_storage_id().await.map_err(database_error)?;
        let mut ids: BTreeSet<String> = self
            .db
            .room_storage_ids(platform, room_id)
            .await
            .map_err(database_error)?
            .into_iter()
            .collect();
        if config.backends.contains_key(&legacy_id)
            || self.registered.read().await.contains_key(&legacy_id)
            || self
                .db
                .room_has_unbound_recordings(platform, room_id)
                .await
                .map_err(database_error)?
        {
            ids.insert(legacy_id);
        }
        let selected = self
            .db
            .recorder_storage_id(platform, room_id)
            .await
            .map_err(database_error)?;
        ids.insert(match selected {
            Some(id) => id,
            None => config.default_backend,
        });
        let mut scopes = Vec::with_capacity(ids.len());
        for id in ids {
            scopes.push(
                self.store(&id)
                    .await?
                    .scope(&format!("{platform}/{room_id}"))?,
            );
        }
        Ok(scopes)
    }

    /// Pin the ID captured for the recording attempt, not today's room
    /// selection. Do not resolve it here: an event remains bindable after the
    /// recorder stops or its configuration is removed.
    #[cfg(test)]
    pub async fn bind_recording(
        &self,
        platform: &str,
        room_id: &str,
        live_id: &str,
        storage_id: Option<&str>,
    ) -> io::Result<()> {
        let storage_id = storage_id.unwrap_or(DEFAULT_RECORDING_STORAGE_ID);
        for component in [platform, room_id, live_id, storage_id] {
            validate_component(component)?;
        }
        self.db
            .bind_recording_storage(platform, room_id, live_id, storage_id)
            .await
            .map_err(database_error)
    }

    pub async fn forget_recording(
        &self,
        platform: &str,
        room_id: &str,
        live_id: &str,
    ) -> io::Result<()> {
        self.db
            .forget_recording_storage(platform, room_id, live_id)
            .await
            .map_err(database_error)
    }

    pub async fn confirm_recording_start(
        &self,
        platform: &str,
        room_id: &str,
        live_id: &str,
        storage_id: Option<&str>,
    ) -> io::Result<()> {
        let storage_id = storage_id.unwrap_or(DEFAULT_RECORDING_STORAGE_ID);
        for component in [platform, room_id, live_id, storage_id] {
            validate_component(component)?;
        }
        self.db
            .confirm_recording_start_storage(platform, room_id, live_id, storage_id)
            .await
            .map_err(database_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RecordingBackendConfig;
    use crate::database::credentials::CredentialCipher;
    use crate::events::EventBus;

    #[tokio::test]
    async fn webdav_settings_validate_credentials_round_trip_and_rotate_without_leaking_secrets() {
        let (_directory, storage) = fixture().await;
        let initial = storage.configuration().await.unwrap();
        let mut next = initial.clone();
        next.backends.insert(
            "dav".into(),
            RecordingBackendConfig {
                kind: "webdav".into(),
                root: String::new(),
                s3: None,
                webdav: Some(recording_storage::WebDavConfig {
                    endpoint: "https://cloud.example/dav".into(),
                    prefix: "recordings".into(),
                }),
            },
        );
        next.default_backend = "dav".into();
        assert!(storage.replace_configuration(next.clone()).await.is_err());
        assert_eq!(storage.configuration().await.unwrap(), initial);
        let wrong = HashMap::from([(
            "dav".into(),
            recording_storage::S3Credentials {
                access_key_id: "access".into(),
                secret_access_key: "secret".into(),
                session_token: String::new(),
            }
            .into(),
        )]);
        assert!(storage
            .replace_configuration_with_credentials(next.clone(), &initial, &wrong)
            .await
            .is_err());
        let credentials = HashMap::from([(
            "dav".into(),
            recording_storage::WebDavCredentials {
                username: "private-user".into(),
                password: " private-password ".into(),
            }
            .into(),
        )]);
        let saved = storage
            .replace_configuration_with_credentials(next, &initial, &credentials)
            .await
            .unwrap();
        assert_eq!(
            saved.backends["dav"].webdav.as_ref().unwrap().endpoint,
            "https://cloud.example/dav/"
        );
        let public = serde_json::to_string(&saved).unwrap();
        assert!(!public.contains("private-user"));
        assert!(!public.contains("private-password"));
        let definition = &saved.backends["dav"];
        assert_eq!(
            RecordingBackendConfig::from_location("webdav".into(), definition.location().unwrap())
                .unwrap(),
            *definition
        );
        let reopened = ArchiveStorage::new(storage.db.clone(), storage.config.clone());
        assert_eq!(reopened.configuration().await.unwrap(), saved);
        assert_eq!(
            reopened
                .db
                .storage_credentials("dav")
                .await
                .unwrap()
                .webdav()
                .unwrap()
                .password,
            " private-password "
        );
        assert_eq!(
            reopened
                .selected_store(Some("dav"))
                .await
                .unwrap()
                .identity(),
            Some("dav")
        );
        let credentials = HashMap::from([(
            "dav".into(),
            recording_storage::WebDavCredentials {
                username: "rotated-user".into(),
                password: "rotated-password".into(),
            }
            .into(),
        )]);
        let rotated = storage
            .replace_configuration_with_credentials(saved.clone(), &saved, &credentials)
            .await
            .unwrap();
        assert_ne!(rotated.credential_revisions, saved.credential_revisions);
        assert_eq!(rotated.backends, saved.backends);
        assert!(storage
            .replace_configuration_with_credentials(saved.clone(), &saved, &credentials)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn s3_settings_keep_secrets_separate_and_allow_safe_credential_rotation() {
        let (directory, storage) = fixture().await;
        let storage = Arc::new(storage);
        let (_, pool) = database(&directory.path().join("test.db")).await;
        let initial = storage.configuration().await.unwrap();
        let mut next = initial.clone();
        let definition = RecordingBackendConfig {
            webdav: None,
            kind: "s3".into(),
            root: String::new(),
            s3: Some(recording_storage::S3Config {
                endpoint: "http://localhost:9000/".into(),
                bucket: "recordings".into(),
                region: "us-east-1".into(),
                prefix: "archive".into(),
                path_style: true,
            }),
        };
        next.backends.insert("remote".into(), definition);
        next.default_backend = "remote".into();
        assert!(storage.replace_configuration(next.clone()).await.is_err());
        assert_eq!(
            storage.configuration().await.unwrap(),
            initial,
            "failed save must not reserve an ID"
        );
        let mut credentials = HashMap::from([(
            "remote".to_owned(),
            recording_storage::S3Credentials {
                access_key_id: "test-access".into(),
                secret_access_key: "test-secret".into(),
                session_token: "test-token".into(),
            }
            .into(),
        )]);
        let saved = storage
            .replace_configuration_with_credentials(next, &initial, &credentials)
            .await
            .unwrap();
        let public_json = serde_json::to_string(&saved).unwrap();
        assert!(!public_json.contains("test-access"));
        assert!(!public_json.contains("test-secret"));
        assert!(!public_json.contains("test-token"));
        let (encoded, encrypted): (String, bool) = sqlx::query_as(
            "SELECT credentials, encrypted FROM recording_storage_credentials WHERE storage_id = 'remote'",
        ).fetch_one(&pool).await.unwrap();
        assert_eq!(encrypted, CredentialCipher::ENCRYPTED);
        if encrypted {
            assert!(!encoded.contains("test-secret"));
            assert!(CredentialCipher::new(&[1; 32])
                .unwrap()
                .decrypt(&encoded)
                .is_err());
        }
        assert_eq!(
            saved.backends["remote"].s3.as_ref().unwrap().endpoint,
            "http://localhost:9000"
        );
        let reopened = ArchiveStorage::new(storage.db.clone(), storage.config.clone());
        assert_eq!(reopened.configuration().await.unwrap(), saved);
        assert_eq!(
            reopened
                .selected_store(Some("remote"))
                .await
                .unwrap()
                .identity(),
            Some("remote")
        );
        assert_eq!(
            reopened
                .db
                .storage_credentials("remote")
                .await
                .unwrap()
                .s3()
                .unwrap()
                .secret_access_key,
            "test-secret"
        );
        assert_eq!(
            storage.replace_configuration(saved.clone()).await.unwrap(),
            saved,
            "ordinary config writes preserve credentials"
        );
        let mut changed = saved.clone();
        changed
            .backends
            .get_mut("remote")
            .unwrap()
            .s3
            .as_mut()
            .unwrap()
            .bucket = "another-bucket".into();
        assert!(storage.replace_configuration(changed).await.is_err());
        assert_eq!(storage.configuration().await.unwrap(), saved);
        assert!(storage
            .preview_storage_transfer("one", "remote")
            .await
            .is_err());
        // Updates do not redirect the namespace; blank token explicitly clears it.
        let recording_storage::StorageCredentials::S3(credential) =
            credentials.get_mut("remote").unwrap()
        else {
            panic!("expected S3 credentials")
        };
        credential.secret_access_key = "rotated-secret".into();
        credential.session_token.clear();
        let active = storage
            .recording_writer()
            .resolve_recording("bilibili", "remote-room", "active")
            .await
            .unwrap();
        assert_eq!(active.identity(), Some("remote"));
        assert!(storage
            .replace_configuration_with_credentials(saved.clone(), &saved, &credentials)
            .await
            .is_err());
        drop(active);
        let rotated = storage
            .replace_configuration_with_credentials(saved.clone(), &saved, &credentials)
            .await
            .unwrap();
        assert_eq!(rotated.backends, saved.backends);
        assert_ne!(rotated.credential_revisions, saved.credential_revisions);
        assert!(storage
            .replace_configuration_with_credentials(saved.clone(), &saved, &credentials)
            .await
            .is_err());
        assert_eq!(
            storage
                .db
                .storage_credentials("remote")
                .await
                .unwrap()
                .s3()
                .unwrap()
                .secret_access_key,
            "rotated-secret"
        );
        assert!(storage
            .db
            .storage_credentials("remote")
            .await
            .unwrap()
            .s3()
            .unwrap()
            .session_token
            .is_empty());

        storage
            .db
            .bind_recording_storage("bilibili", "123", "remote-live", "remote")
            .await
            .unwrap();
        sqlx::query("INSERT INTO records (platform, room_id, live_id) VALUES ('bilibili', '123', 'remote-live')")
            .execute(&pool).await.unwrap();
        let available: bool = sqlx::query_scalar(
            "SELECT storage_available FROM recording_records WHERE live_id = 'remote-live'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(
            available,
            "availability is configuration presence, not a network health check"
        );
        let mut removed = rotated;
        removed.default_backend = "one".into();
        removed.backends.remove("remote");
        storage.replace_configuration(removed).await.unwrap();
        let row: (String, bool) = sqlx::query_as("SELECT storage_backend_id, storage_available FROM recording_records WHERE live_id = 'remote-live'")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(row, ("remote".into(), false));
        assert!(storage.db.storage_credentials("remote").await.is_err());
        assert!(storage
            .recording("bilibili", "123", "remote-live")
            .await
            .is_err());
    }

    pub(super) async fn database(path: &std::path::Path) -> (Arc<Database>, sqlx::SqlitePool) {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(path)
                    .create_if_missing(true),
            )
            .await
            .unwrap();
        let db = Arc::new(Database::new(
            pool.clone(),
            CredentialCipher::new(&[0; 32]).unwrap(),
            EventBus::new(),
        ));
        sqlx::raw_sql(
            "CREATE TABLE IF NOT EXISTS recorders (
                room_id TEXT, platform TEXT, created_at TEXT NOT NULL DEFAULT '',
                auto_start INTEGER NOT NULL DEFAULT 1, extra TEXT NOT NULL DEFAULT '',
                storage_backend_id TEXT, PRIMARY KEY(platform, room_id));
             CREATE TABLE IF NOT EXISTS records (platform TEXT, room_id TEXT, live_id TEXT);",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::raw_sql(crate::database::storage::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        (db, pool)
    }

    async fn fixture() -> (tempfile::TempDir, ArchiveStorage) {
        let dir = tempfile::tempdir().unwrap();
        let mut config: Config = toml::from_str(include_str!("../config.example.toml")).unwrap();
        config.storage.backends.get_mut("default").unwrap().root =
            dir.path().join("legacy").to_string_lossy().into_owned();
        config.config_path = dir
            .path()
            .join("config.toml")
            .to_string_lossy()
            .into_owned();
        for id in ["one", "two"] {
            config.storage.backends.insert(
                id.into(),
                RecordingBackendConfig {
                    kind: "local".into(),
                    s3: None,
                    webdav: None,
                    root: dir.path().join(id).to_string_lossy().into_owned(),
                },
            );
        }
        let (db, _) = database(&dir.path().join("test.db")).await;
        for (room, id) in [("123", "one"), ("456", "two")] {
            db.add_recorder_with_storage(
                recorder::platforms::PlatformType::BiliBili,
                room,
                "",
                Some(id),
            )
            .await
            .unwrap();
        }
        (dir, ArchiveStorage::new(db, Arc::new(RwLock::new(config))))
    }

    #[tokio::test]
    async fn storage_writer_selects_each_archive_and_resumes_its_original_binding() {
        let (_dir, registry) = fixture().await;
        let registry = Arc::new(registry);
        let writer = registry.recording_writer();
        // This room inherits the DB default.
        let first = writer
            .resolve_recording("bilibili", "inherited", "first")
            .await
            .unwrap();
        first.prepare().await.unwrap();
        first.write("cover.jpg", b"first").await.unwrap();
        assert_eq!(first.identity(), Some("default"));
        let mut next = registry.configuration().await.unwrap();
        next.default_backend = "two".into();
        registry.replace_configuration(next.clone()).await.unwrap();
        let second = writer
            .resolve_recording("bilibili", "inherited", "second")
            .await
            .unwrap();
        assert_eq!(second.identity(), Some("two"));
        drop(first);
        let resumed = writer
            .resolve_recording("bilibili", "inherited", "first")
            .await
            .unwrap();
        assert_eq!(resumed.identity(), Some("default"));
        assert_eq!(resumed.read("cover.jpg").await.unwrap(), b"first");

        next.backends.remove("default");
        assert_eq!(
            registry
                .replace_configuration(next.clone())
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::WouldBlock
        );
        // A log still owns its backend after the recording handle is dropped.
        let log = resumed.open_log("events.jsonl").await.unwrap();
        drop(resumed);
        assert!(registry.replace_configuration(next.clone()).await.is_err());
        log.flush().await.unwrap();
        drop(log);
        registry.replace_configuration(next).await.unwrap();
        assert_eq!(
            writer
                .resolve_recording("bilibili", "inherited", "first")
                .await
                .err()
                .unwrap()
                .kind(),
            io::ErrorKind::NotFound
        );
        assert_eq!(
            registry
                .db
                .recording_storage_id("bilibili", "inherited", "first")
                .await
                .unwrap()
                .as_deref(),
            Some("default")
        );
    }

    #[tokio::test]
    async fn storage_configuration_survives_restart_and_imports_toml_only_once() {
        let (dir, registry) = fixture().await;
        let original = registry.config.read().await.clone();
        let (_, pool) = database(&dir.path().join("test.db")).await;
        sqlx::query(
            "INSERT INTO records (platform, room_id, live_id) VALUES ('bilibili', '123', 'legacy')",
        )
        .execute(&pool)
        .await
        .unwrap();
        registry.initialize().await.unwrap();
        assert_eq!(
            registry
                .db
                .recording_storage_id("bilibili", "123", "legacy")
                .await
                .unwrap()
                .as_deref(),
            Some("default")
        );
        let mut next = registry.configuration().await.unwrap();
        next.default_backend = "two".into();
        next.backends.remove("one");
        registry.replace_configuration(next.clone()).await.unwrap();
        registry.config.read().await.try_save().unwrap();
        let saved = std::fs::read_to_string(&original.config_path).unwrap();
        assert!(!toml::from_str::<toml::Value>(&saved)
            .unwrap()
            .as_table()
            .unwrap()
            .contains_key("storage"));

        // Reopen SQLite with a stale pre-import TOML snapshot.
        let (db, _) = database(&dir.path().join("test.db")).await;
        let restarted = ArchiveStorage::new(db, Arc::new(RwLock::new(original)));
        restarted.initialize().await.unwrap();
        assert_eq!(restarted.configuration().await.unwrap(), next);
        assert_eq!(
            restarted
                .db
                .recording_storage_id("bilibili", "123", "legacy")
                .await
                .unwrap()
                .as_deref(),
            Some("default")
        );
        assert!(restarted.recorder_store("bilibili", "123").await.is_err());
        let stale_expected = StorageConfig::local("/stale".into());
        assert_eq!(
            restarted
                .replace_configuration_if_current(next.clone(), &stale_expected)
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::WouldBlock
        );
        assert_eq!(restarted.configuration().await.unwrap(), next);
    }

    #[tokio::test]
    async fn archive_mutations_are_isolated_and_conflicts_are_retryable() {
        let (_directory, registry) = fixture().await;
        let guard = registry
            .try_lock_recording("bilibili", "123", "live")
            .unwrap();
        assert_eq!(
            registry
                .try_lock_recording("bilibili", "123", "live")
                .unwrap_err()
                .kind(),
            io::ErrorKind::WouldBlock
        );
        let _other_platform = registry.try_lock_recording("douyu", "123", "live").unwrap();
        let _other_archive = registry
            .try_lock_recording("bilibili", "123", "other")
            .unwrap();
        drop(guard);
        assert!(registry
            .try_lock_recording("bilibili", "123", "live")
            .is_ok());
    }

    #[tokio::test]
    async fn preparing_a_recording_persists_binding_before_creating_assets() {
        let directory = tempfile::tempdir().unwrap();
        let (db, pool) = database(&directory.path().join("metadata.db")).await;
        let mut config: Config = toml::from_str(include_str!("../config.example.toml")).unwrap();
        config.storage.backends.get_mut("default").unwrap().root = directory
            .path()
            .join("legacy")
            .to_string_lossy()
            .into_owned();
        config.storage.backends.insert(
            "named".into(),
            RecordingBackendConfig {
                kind: "local".into(),
                s3: None,
                webdav: None,
                root: directory
                    .path()
                    .join("named")
                    .to_string_lossy()
                    .into_owned(),
            },
        );
        let registry = ArchiveStorage::new(db.clone(), Arc::new(RwLock::new(config)));
        let store = registry.selected_store(Some("named")).await.unwrap();
        let recording = store.recording("rtmp", "room", "first").unwrap();
        recording.prepare().await.unwrap();
        assert_eq!(
            db.recording_storage_id("rtmp", "room", "first")
                .await
                .unwrap()
                .as_deref(),
            Some("named")
        );
        // Metadata failure stops startup before the backend creates its namespace.
        pool.close().await;
        let failed = store.recording("rtmp", "room", "second").unwrap();
        assert!(failed.prepare().await.is_err());
        assert!(!directory.path().join("named/rtmp/room/second").exists());
    }

    #[tokio::test]
    async fn two_rooms_are_isolated_and_legacy_archives_use_default() {
        let (_dir, storage) = fixture().await;
        let one = storage.recorder_store("bilibili", "123").await.unwrap();
        let two = storage.recorder_store("bilibili", "456").await.unwrap();
        assert_eq!(one.identity(), Some("one"));
        assert_eq!(two.identity(), Some("two"));
        assert_eq!(
            storage
                .recorder_store("douyu", "123")
                .await
                .unwrap()
                .identity(),
            Some("default")
        );
        storage
            .room("bilibili", "123")
            .await
            .unwrap()
            .write("cover.jpg", b"one")
            .await
            .unwrap();
        storage
            .room("bilibili", "456")
            .await
            .unwrap()
            .write("cover.jpg", b"two")
            .await
            .unwrap();
        assert_eq!(
            storage
                .room("bilibili", "123")
                .await
                .unwrap()
                .read("cover.jpg")
                .await
                .unwrap(),
            b"one"
        );
        assert_eq!(
            storage
                .room("bilibili", "456")
                .await
                .unwrap()
                .read("cover.jpg")
                .await
                .unwrap(),
            b"two"
        );

        let legacy = storage.recording("bilibili", "123", "old").await.unwrap();
        legacy.write("cover.jpg", b"legacy").await.unwrap();
        let root = storage.config.read().await.storage.backends["default"]
            .root
            .clone();
        assert_eq!(
            std::fs::read(std::path::Path::new(&root).join("bilibili/123/old/cover.jpg")).unwrap(),
            b"legacy"
        );
        assert!(!one
            .recording("bilibili", "123", "old")
            .unwrap()
            .exists("cover.jpg")
            .await
            .unwrap());
        storage
            .bind_recording("bilibili", "123", "old", None)
            .await
            .unwrap();
        assert_eq!(
            storage
                .db
                .recording_storage_id("bilibili", "123", "old")
                .await
                .unwrap()
                .as_deref(),
            Some("default")
        );
    }

    #[tokio::test]
    async fn snapshot_binding_survives_selection_changes_and_reopen() {
        let (dir, storage) = fixture().await;
        let recorder = storage.recorder_store("bilibili", "123").await.unwrap();
        recorder
            .recording("bilibili", "123", "live")
            .unwrap()
            .write("cover.jpg", b"original")
            .await
            .unwrap();
        storage
            .db
            .set_recorder_storage("bilibili", "123", Some("two"))
            .await
            .unwrap();
        storage
            .bind_recording("bilibili", "123", "live", recorder.identity())
            .await
            .unwrap();
        assert_eq!(
            storage
                .recorder_store("bilibili", "123")
                .await
                .unwrap()
                .identity(),
            Some("two")
        );
        assert_eq!(
            storage
                .recording("bilibili", "123", "live")
                .await
                .unwrap()
                .read("cover.jpg")
                .await
                .unwrap(),
            b"original"
        );
        assert_eq!(
            storage.room_stores("bilibili", "123").await.unwrap().len(),
            3
        );

        let config = storage.config.clone();
        drop(storage);
        let (db, pool) = database(&dir.path().join("test.db")).await;
        let reopened = ArchiveStorage::new(db, config);
        assert_eq!(
            reopened
                .recording("bilibili", "123", "live")
                .await
                .unwrap()
                .read("cover.jpg")
                .await
                .unwrap(),
            b"original"
        );
        reopened
            .forget_recording("bilibili", "123", "live")
            .await
            .unwrap();
        assert_eq!(
            reopened
                .db
                .recording_storage_id("bilibili", "123", "live")
                .await
                .unwrap(),
            None
        );
        pool.close().await;
    }

    #[tokio::test]
    async fn binding_conflicts_fail_but_late_events_do_not_need_current_config() {
        let (_dir, storage) = fixture().await;
        let (first, second) = tokio::join!(
            storage.bind_recording("bilibili", "123", "same", Some("one")),
            storage.bind_recording("bilibili", "123", "same", Some("two"))
        );
        assert_ne!(first.is_ok(), second.is_ok());
        let failure = first.err().or_else(|| second.err()).unwrap();
        assert_eq!(failure.kind(), io::ErrorKind::AlreadyExists);
        let winner = storage
            .db
            .recording_storage_id("bilibili", "123", "same")
            .await
            .unwrap()
            .unwrap();
        storage
            .bind_recording("bilibili", "123", "same", Some(&winner))
            .await
            .unwrap();
        storage.config.write().await.storage.backends.remove("one");
        storage
            .bind_recording("bilibili", "123", "late", Some("one"))
            .await
            .unwrap();
        assert_eq!(
            storage
                .recording("bilibili", "123", "late")
                .await
                .err()
                .unwrap()
                .kind(),
            io::ErrorKind::NotFound
        );
        // Full key isolates platforms as well as rooms.
        storage
            .bind_recording("douyu", "123", "same", Some("two"))
            .await
            .unwrap();
        storage
            .bind_recording("bilibili", "456", "same", Some("two"))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn missing_unsupported_and_redefined_backends_never_fall_back() {
        let (dir, storage) = fixture().await;
        storage.recorder_store("bilibili", "123").await.unwrap();
        let original = storage.configuration().await.unwrap();
        let mut changed = original.clone();
        changed.backends.get_mut("one").unwrap().root =
            dir.path().join("changed").to_string_lossy().into_owned();
        assert_eq!(
            storage
                .replace_configuration(changed)
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
        let mut unsupported = original.clone();
        unsupported.backends.get_mut("one").unwrap().kind = "s3".into();
        assert!(storage.replace_configuration(unsupported).await.is_err());
        let mut removed = original.clone();
        removed.backends.remove("one");
        storage.replace_configuration(removed).await.unwrap();
        // Stale TOML cannot resurrect a deleted backend after reopening.
        let (db, _) = database(&dir.path().join("test.db")).await;
        storage.config.write().await.storage = original.clone();
        let reopened = ArchiveStorage::new(db, storage.config.clone());
        assert_eq!(
            reopened
                .recorder_store("bilibili", "123")
                .await
                .err()
                .unwrap()
                .kind(),
            io::ErrorKind::NotFound
        );
        assert!(reopened.replace_configuration(original).await.is_err());
        assert_eq!(
            storage
                .recorder_store("bilibili", "123")
                .await
                .err()
                .unwrap()
                .kind(),
            io::ErrorKind::NotFound
        );
    }

    #[tokio::test]
    async fn registered_adapters_reject_collisions_and_keep_identity() {
        let (dir, storage) = fixture().await;
        let adapter = RecordingStore::local(dir.path().join("adapter"));
        storage
            .register_backend("custom", adapter.clone())
            .await
            .unwrap();
        assert_eq!(
            storage.store("custom").await.unwrap().identity(),
            Some("custom")
        );
        for id in ["custom", "default", "one"] {
            assert_eq!(
                storage
                    .register_backend(id, adapter.clone())
                    .await
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::AlreadyExists
            );
        }
        storage
            .bind_recording("bilibili", "123", "custom-live", Some("custom"))
            .await
            .unwrap();
        storage
            .recording("bilibili", "123", "custom-live")
            .await
            .unwrap()
            .write("cover.jpg", b"adapter")
            .await
            .unwrap();
        assert_eq!(
            adapter
                .recording("bilibili", "123", "custom-live")
                .unwrap()
                .read("cover.jpg")
                .await
                .unwrap(),
            b"adapter"
        );
    }

    #[tokio::test]
    async fn default_selector_never_redirects_legacy_or_pinned_archives() {
        let (dir, storage) = fixture().await;
        let named_before = storage.recorder_store("bilibili", "123").await.unwrap();
        let default_before = storage.store("default").await.unwrap();
        let mut next = storage.configuration().await.unwrap();
        next.default_backend = "two".into();
        storage.replace_configuration(next).await.unwrap();
        assert_eq!(
            storage.selected_store(None).await.unwrap().identity(),
            Some("two")
        );
        assert_eq!(
            storage.store("default").await.unwrap().local_root(),
            default_before.local_root()
        );
        assert_eq!(
            storage
                .recording("bilibili", "123", "legacy")
                .await
                .unwrap()
                .workspace()
                .root(),
            default_before
                .recording("bilibili", "123", "legacy")
                .unwrap()
                .workspace()
                .root(),
        );
        assert_eq!(
            storage
                .recorder_store("bilibili", "123")
                .await
                .unwrap()
                .local_root(),
            named_before.local_root()
        );
        let mut next = storage.configuration().await.unwrap();
        next.backends.get_mut("default").unwrap().root =
            dir.path().join("moved").to_string_lossy().into_owned();
        assert_eq!(
            storage
                .replace_configuration(next)
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists,
        );
    }

    #[tokio::test]
    async fn validation_under_config_write_lock_is_read_only_and_rejects_redefinitions() {
        let (dir, storage) = fixture().await;
        storage.store("default").await.unwrap();
        let config = storage.config.write().await;
        let mut next = config.storage.clone();
        next.default_backend = "two".into();
        // This is the same lock ordering used by the settings handler.
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            storage.validate_config(&config.storage, &next),
        )
        .await
        .unwrap()
        .unwrap();
        next.backends.insert(
            "new".into(),
            RecordingBackendConfig {
                kind: "local".into(),
                s3: None,
                webdav: None,
                root: dir.path().join("first").to_string_lossy().into_owned(),
            },
        );
        storage
            .validate_config(&config.storage, &next)
            .await
            .unwrap();
        next.backends.get_mut("new").unwrap().root =
            dir.path().join("second").to_string_lossy().into_owned();
        // The previous validation did not reserve the first root.
        storage
            .validate_config(&config.storage, &next)
            .await
            .unwrap();
        next.backends.remove("default");
        assert!(storage
            .validate_config(&config.storage, &next)
            .await
            .is_ok());
        next = config.storage.clone();
        next.backends.get_mut("default").unwrap().root = "/different".into();
        assert!(storage
            .validate_config(&config.storage, &next)
            .await
            .is_err());
        next = config.storage.clone();
        next.default_backend = "missing".into();
        assert!(storage
            .validate_config(&config.storage, &next)
            .await
            .is_err());
        next.default_backend = "../one".into();
        assert!(storage
            .validate_config(&config.storage, &next)
            .await
            .is_err());
        drop(config);

        storage
            .register_backend("adapter", RecordingStore::local(dir.path().join("adapter")))
            .await
            .unwrap();
        let config = storage.config.write().await;
        next = config.storage.clone();
        next.default_backend = "adapter".into();
        storage
            .validate_config(&config.storage, &next)
            .await
            .unwrap();
        next.backends.insert(
            "adapter".into(),
            RecordingBackendConfig {
                kind: "local".into(),
                s3: None,
                webdav: None,
                root: "/adapter".into(),
            },
        );
        assert!(storage
            .validate_config(&config.storage, &next)
            .await
            .is_err());
        // A definition removed by a manual config edit is still protected by DB.
        next = config.storage.clone();
        next.backends.get_mut("default").unwrap().root = "/different".into();
        let mut current = config.storage.clone();
        current.backends.remove("default");
        assert_eq!(
            storage
                .validate_config(&current, &next)
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
    }

    #[tokio::test]
    async fn already_migrated_startup_does_not_rewrite_the_config() {
        let (dir, storage) = fixture().await;
        storage.initialize().await.unwrap();
        storage.config.read().await.try_save().unwrap();
        let path = dir.path().join("config.toml");
        let mut original = std::fs::read_to_string(&path).unwrap();
        original.push_str("\n# Externally managed, preserve this file on startup\n");
        std::fs::write(&path, &original).unwrap();
        let config = Config::load(&path, dir.path(), dir.path()).unwrap();
        let restarted = ArchiveStorage::new(storage.db.clone(), Arc::new(RwLock::new(config)));
        restarted.initialize().await.unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        // A config path that cannot be written must also be harmless after import.
        restarted.config.write().await.config_path = dir
            .path()
            .join("missing/config.toml")
            .to_string_lossy()
            .into_owned();
        restarted.initialize().await.unwrap();
    }

    #[tokio::test]
    async fn default_id_can_be_registered_when_not_configured() {
        let (dir, storage) = fixture().await;
        storage.config.write().await.storage.default_backend = "one".into();
        storage
            .config
            .write()
            .await
            .storage
            .backends
            .remove("default");
        storage
            .register_backend("default", RecordingStore::local(dir.path().join("adapter")))
            .await
            .unwrap();
        assert_eq!(
            storage.store("default").await.unwrap().identity(),
            Some("default")
        );
    }

    #[tokio::test]
    async fn room_cleanup_requires_default_only_when_defined_or_historically_needed() {
        let (dir, storage) = fixture().await;
        let mut next = storage.configuration().await.unwrap();
        next.default_backend = "one".into();
        next.backends.remove("default");
        storage.replace_configuration(next).await.unwrap();
        assert_eq!(
            storage.room_stores("bilibili", "123").await.unwrap().len(),
            1
        );
        let (_, pool) = database(&dir.path().join("test.db")).await;
        sqlx::query(
            "INSERT INTO records (platform, room_id, live_id) VALUES ('bilibili', '123', 'legacy')",
        )
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(
            storage
                .room_stores("bilibili", "123")
                .await
                .err()
                .unwrap()
                .kind(),
            io::ErrorKind::NotFound
        );
    }
}
