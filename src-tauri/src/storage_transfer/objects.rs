//! Object-copy strategy for any route containing remote storage. Shares the local strategy's
//! DB phases, archive mutation locks, reader barriers and transfer reservation.
//! The local rename/copy implementation is deliberately left intact.
use super::*;
use crate::config::RecordingBackendConfig;
use recording_storage::{
    local_transfer_root, ObjectTransfer, ObjectTransferManifest, RecordingStore,
};
use std::{collections::HashMap, path::PathBuf};

struct Endpoint {
    store: RecordingStore,
    identity: String,
    local: Option<(PathBuf, String)>,
}

impl Endpoint {
    async fn capacity(&self) -> io::Result<Option<u64>> {
        let Some((root, expected)) = &self.local else {
            return Ok(None);
        };
        let root = root.clone();
        let expected = expected.clone();
        blocking((), move || {
            let (identity, available) = local_transfer_root(&root)?;
            if identity != expected {
                return Err(io::Error::other("本地存储根目录已被替换，不能继续原转移"));
            }
            Ok(Some(available))
        })
        .await
    }
}

struct Route {
    source: Endpoint,
    target: Endpoint,
}

impl Route {
    fn archive(&self, key: &str) -> io::Result<ObjectTransfer> {
        ObjectTransfer::new(
            self.source.store.scope(key)?,
            self.target.store.scope(key)?,
            key.into(),
            self.source.identity.clone(),
            self.target.identity.clone(),
        )
    }

    async fn check_roots(&self) -> io::Result<()> {
        self.source.capacity().await?;
        self.target.capacity().await?;
        Ok(())
    }
}

fn reject_overlap(
    source: &RecordingBackendConfig,
    target: &RecordingBackendConfig,
) -> io::Result<()> {
    if let (Some(source), Some(target)) = (&source.webdav, &target.webdav) {
        if source.overlaps(target)? {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "WebDAV 来源与目标命名空间相同或重叠",
            ));
        }
    }
    if let (Some(source), Some(target)) = (&source.s3, &target.s3) {
        let contains = |parent: &str, child: &str| {
            parent.is_empty()
                || parent == child
                || child
                    .strip_prefix(parent)
                    .is_some_and(|rest| rest.starts_with('/'))
        };
        if source.service_endpoint()? == target.service_endpoint()?
            && source.bucket == target.bucket
            && (contains(&source.prefix, &target.prefix)
                || contains(&target.prefix, &source.prefix))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "S3 来源与目标命名空间相同或前缀重叠",
            ));
        }
    }
    Ok(())
}

/// A lost caller does not release the application's transfer/read leases while
/// a remote upload or cleanup is still running. The durable journal is retried.
async fn owned<T: Send + 'static>(
    lease: impl Send + 'static,
    future: impl std::future::Future<Output = io::Result<T>> + Send + 'static,
) -> io::Result<T> {
    tokio::spawn(async move {
        let _lease = lease;
        future.await
    })
    .await
    .map_err(io::Error::other)?
}

impl ArchiveStorage {
    pub(super) async fn uses_object_transfer(
        &self,
        source: &str,
        target: &str,
    ) -> io::Result<bool> {
        let config = self.configuration().await?;
        Ok([source, target].iter().any(|id| {
            config
                .backends
                .get(*id)
                .is_some_and(|backend| matches!(backend.kind.as_str(), "s3" | "webdav"))
        }))
    }

    async fn object_route(
        &self,
        source: &str,
        target: &str,
    ) -> io::Result<(OwnedRwLockReadGuard<()>, Arc<Route>)> {
        self.initialize().await?;
        if source == target {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "来源与目标存储不能相同",
            ));
        }
        let gate = self.configuration_gate.clone().read_owned().await;
        let config = self.configuration().await?;
        let definition = |id: &str| {
            config.backends.get(id).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("存储后端 {id} 不存在或已移除"),
                )
            })
        };
        reject_overlap(definition(source)?, definition(target)?)?;
        let mut endpoints = Vec::new();
        for id in [source, target] {
            let config = definition(id)?;
            let local = match config.kind.as_str() {
                "local" => {
                    let root = PathBuf::from(&config.root);
                    let path = root.clone();
                    let (identity, _) = blocking((), move || local_transfer_root(&path)).await
                        .map_err(|error| io::Error::new(error.kind(), format!("本地存储 {id} 根目录不可用，请先创建目录并检查路径及符号链接：{error}")))?;
                    Some((root, identity))
                }
                "s3" | "webdav" => None,
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "仅支持本地、S3 或 WebDAV 存储之间转移",
                    ))
                }
            };
            let identity = serde_json::to_string(&(
                config.kind.as_str(),
                config.location()?,
                local.as_ref().map(|(_, identity)| identity),
            ))
            .map_err(io::Error::other)?;
            endpoints.push(Endpoint {
                store: self.store(id).await?,
                identity,
                local,
            });
        }
        let target = endpoints.pop().expect("two endpoints");
        let source = endpoints.pop().expect("two endpoints");
        Ok((gate, Arc::new(Route { source, target })))
    }

    pub(super) async fn preview_object_transfer(
        &self,
        source: &str,
        target: &str,
        _reservation: Arc<OwnedMutexGuard<()>>,
    ) -> io::Result<StorageTransferPreview> {
        let (_gate, route) = self.object_route(source, target).await?;
        let records = self
            .db
            .storage_transfer_estimates(source, target)
            .await
            .map_err(database_error)?;
        let mut preview = StorageTransferPreview {
            source_backend_id: source.into(),
            target_backend_id: target.into(),
            recording_count: 0,
            total_bytes: 0,
            required_bytes: 0,
            available_bytes: route.target.capacity().await?,
            cleanup_count: 0,
            transfer_mode: "object",
            can_transfer: false,
            blockers: Vec::new(),
        };
        let mut fresh_bytes = 0u64;
        for record in records {
            let key = format!("{}/{}/{}", record.platform, record.room_id, record.live_id);
            let matching = record.source_id.as_deref() == Some(source)
                && record.target_id.as_deref() == Some(target);
            if record.phase.is_some() && (!matching || record.mode.as_deref() != Some("object")) {
                preview
                    .blockers
                    .push(format!("{key} 有另一项未完成的转移，请先重试原来源与目标"));
            }
            if matching && record.phase.as_deref() != Some("copying") {
                preview.cleanup_count += 1;
            } else {
                preview.recording_count += 1;
                let bytes = u64::try_from(record.size).unwrap_or(0);
                preview.total_bytes = preview
                    .total_bytes
                    .checked_add(bytes)
                    .ok_or_else(|| io::Error::other("转移数据量溢出"))?;
                if !matching {
                    fresh_bytes = fresh_bytes
                        .checked_add(bytes)
                        .ok_or_else(|| io::Error::other("转移数据量溢出"))?;
                }
            }
            if let Err(error) = self.try_lock_recording_for_transfer(
                &record.platform,
                &record.room_id,
                &record.live_id,
            ) {
                preview.blockers.push(format!("{key}: {error}"));
            }
        }
        preview.required_bytes = preview.total_bytes;
        if let Some(available) = preview.available_bytes {
            if fresh_bytes > available {
                preview.blockers.push(format!(
                    "目标空间不足：预计需要 {fresh_bytes} 字节，可用 {available} 字节"
                ));
            }
        }
        preview.can_transfer = preview.blockers.is_empty();
        Ok(preview)
    }

    pub(super) async fn execute_object_transfer(
        &self,
        source: &str,
        target: &str,
        reservation: Arc<OwnedMutexGuard<()>>,
        reporter: &ProgressReporter,
    ) -> StorageTransferResult {
        let mut result = StorageTransferResult::default();
        let operation: io::Result<()> = async {
            let (gate, route) = self.object_route(source, target).await?;
            let journals = self
                .db
                .storage_transfer_journals()
                .await
                .map_err(database_error)?;
            *self
                .pending_transfers
                .lock()
                .unwrap_or_else(|p| p.into_inner()) =
                journals.iter().map(TransferJournal::key).collect();
            let mut work = BTreeMap::new();
            for (platform, room, live) in self
                .db
                .storage_transfer_recordings(source)
                .await
                .map_err(database_error)?
            {
                work.insert(
                    format!("{platform}/{room}/{live}"),
                    (platform, room, live, None),
                );
            }
            for journal in journals {
                if journal.source_id == source && journal.target_id == target {
                    if journal.mode != "object" {
                        return Err(io::Error::other("转移日志模式与存储后端不匹配"));
                    }
                    work.insert(
                        journal.key(),
                        (
                            journal.platform.clone(),
                            journal.room_id.clone(),
                            journal.live_id.clone(),
                            Some(journal),
                        ),
                    );
                } else if work.contains_key(&journal.key()) {
                    return Err(io::Error::other(format!(
                        "{} 有另一项未完成的转移",
                        journal.key()
                    )));
                }
            }
            let estimates: HashMap<_, _> = self
                .db
                .storage_transfer_estimates(source, target)
                .await
                .map_err(database_error)?
                .into_iter()
                .map(|record| {
                    (
                        format!("{}/{}/{}", record.platform, record.room_id, record.live_id),
                        u64::try_from(record.size).unwrap_or(0),
                    )
                })
                .collect();
            let mut guards = Vec::new();
            for (platform, room, live, _) in work.values() {
                guards.push(self.try_lock_recording_for_transfer(platform, room, live)?);
            }
            let lease = Arc::new(Lease {
                _configuration: gate,
                _recordings: guards,
                _transfer: reservation,
            });
            let count = work.len();
            for (index, (key, (platform, room_id, live_id, existing))) in
                work.into_iter().enumerate()
            {
                result.failed_recording = Some(key.clone());
                reporter
                    .update(&format!(
                        "转移 {}/{}：等待录播读取结束 {key}",
                        index + 1,
                        count
                    ))
                    .await;
                let barrier = Arc::new(
                    tokio::time::timeout(
                        READER_DRAIN_TIMEOUT,
                        self.reader_barrier(&key).write_owned(),
                    )
                    .await
                    .map_err(|_| {
                        io::Error::new(
                            io::ErrorKind::TimedOut,
                            "等待录播读取结束超时（30 秒），请结束播放后重试",
                        )
                    })?,
                );
                route.check_roots().await?;
                let transfer = Arc::new(route.archive(&key)?);
                let (journal, manifest) = if let Some(journal) = existing {
                    let manifest: ObjectTransferManifest =
                        serde_json::from_str(&journal.manifest).map_err(io::Error::other)?;
                    (journal, manifest)
                } else {
                    if let Some(available) = route.target.capacity().await? {
                        if estimates.get(&key).copied().unwrap_or(0) > available {
                            return Err(io::Error::other("目标可用空间不足"));
                        }
                    }
                    reporter
                        .update(&format!(
                            "转移 {}/{}：建立对象校验清单 {key}",
                            index + 1,
                            count
                        ))
                        .await;
                    let inspect = transfer.clone();
                    let manifest = owned((lease.clone(), barrier.clone()), async move {
                        inspect.inspect().await
                    })
                    .await?;
                    let serialized = serde_json::to_string(&manifest).map_err(io::Error::other)?;
                    if serialized.len() > 64 * 1024 * 1024 {
                        return Err(io::Error::other("转移清单超过大小限制"));
                    }
                    let journal = TransferJournal {
                        platform,
                        room_id,
                        live_id,
                        source_id: source.into(),
                        target_id: target.into(),
                        token: uuid::Uuid::new_v4().to_string(),
                        manifest: serialized,
                        phase: "copying".into(),
                        mode: "object".into(),
                    };
                    self.pending_transfers
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .insert(key.clone());
                    if let Err(error) = self.db.begin_storage_transfer(&journal).await {
                        if self
                            .db
                            .storage_transfer_journals()
                            .await
                            .is_ok_and(|journals| !journals.iter().any(|entry| entry.key() == key))
                        {
                            self.pending_transfers
                                .lock()
                                .unwrap_or_else(|p| p.into_inner())
                                .remove(&key);
                        }
                        return Err(database_error(error));
                    }
                    (journal, manifest)
                };
                if journal.phase == "copying" {
                    reporter
                        .update(&format!(
                            "转移 {}/{}：复制并校验对象 {key}",
                            index + 1,
                            count
                        ))
                        .await;
                    let copy = transfer.clone();
                    let token = journal.token.clone();
                    let snapshot = manifest.clone();
                    owned((lease.clone(), barrier.clone()), async move {
                        copy.copy(&token, &snapshot).await
                    })
                    .await?;
                    route.check_roots().await?;
                    self.db
                        .commit_storage_transfer(&journal)
                        .await
                        .map_err(|error| {
                            io::Error::other(format!(
                                "对象已复制，但绑定提交未确认，请用原来源和目标重试：{error}"
                            ))
                        })?;
                    result.transferred_recordings += 1;
                }
                if journal.phase != "finalizing" {
                    reporter
                        .update(&format!(
                            "转移 {}/{}：绑定已提交，校验并清理来源 {key}",
                            index + 1,
                            count
                        ))
                        .await;
                    route.check_roots().await?;
                    let cleanup = transfer.clone();
                    let token = journal.token.clone();
                    let snapshot = manifest.clone();
                    owned((lease.clone(), barrier.clone()), async move {
                        cleanup.cleanup_source(&token, &snapshot).await
                    })
                    .await
                    .map_err(|error| {
                        io::Error::new(error.kind(), format!("绑定已提交，来源清理未完成：{error}"))
                    })?;
                    self.db
                        .finalize_storage_transfer(&journal.token)
                        .await
                        .map_err(database_error)?;
                }
                let token = journal.token.clone();
                owned((lease.clone(), barrier), async move {
                    transfer.finish(&token, &manifest).await
                })
                .await?;
                self.db
                    .finish_storage_transfer(&journal.token)
                    .await
                    .map_err(database_error)?;
                self.pending_transfers
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .remove(&key);
                result.cleaned_recordings += 1;
                result.failed_recording = None;
            }
            Ok(())
        }
        .await;
        if let Err(error) = operation {
            result.error = Some(format!("转移未完成，请使用原来源和目标重试：{error}"));
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{add, fixture};
    use super::*;

    #[test]
    fn s3_prefix_policy_rejects_aliases_and_subtrees_but_allows_distinct_namespaces() {
        let backend = |prefix: &str, endpoint: &str| RecordingBackendConfig {
            webdav: None,
            kind: "s3".into(),
            root: String::new(),
            s3: Some(recording_storage::S3Config {
                endpoint: endpoint.into(),
                bucket: "recordings".into(),
                region: "us-east-1".into(),
                prefix: prefix.into(),
                path_style: true,
            }),
        };
        for (a, b) in [("", "a"), ("a", "a"), ("a", "a/b"), ("a/b", "a")] {
            assert!(reject_overlap(
                &backend(a, "http://localhost:9000"),
                &backend(b, "http://localhost:9000")
            )
            .is_err());
        }
        assert!(reject_overlap(
            &backend("a", "http://localhost:9000"),
            &backend("ab", "http://localhost:9000")
        )
        .is_ok());
        assert!(reject_overlap(
            &backend("a", "http://first"),
            &backend("a", "http://second")
        )
        .is_ok());
        let mut qualified = backend("a/nested", "http://recordings.localhost:9000");
        qualified.s3.as_mut().unwrap().path_style = false;
        assert!(reject_overlap(&backend("a", "http://localhost:9000"), &qualified).is_err());
    }

    #[tokio::test]
    async fn object_journal_schema_upgrade_preserves_pending_local_transfers() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::raw_sql(crate::database::storage_transfer::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO recording_storage_transfers VALUES ('bilibili', 'room', 'live', 'a', 'b', 'token', 'existing-manifest', 'cleanup', 'rename')")
            .execute(&pool).await.unwrap();
        let mut tx = pool.begin().await.unwrap();
        sqlx::query(crate::database::storage_transfer::OBJECT_SCHEMA_SQL)
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        let original: (String, String, String) = sqlx::query_as(
            "SELECT manifest, phase, mode FROM recording_storage_transfers WHERE token = 'token'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            original,
            (
                "existing-manifest".into(),
                "cleanup".into(),
                "rename".into()
            )
        );
        sqlx::query("INSERT INTO recording_storage_transfers VALUES ('bilibili', 'room', 'other', 'a', 'cloud', 'object-token', '{}', 'copying', 'object')")
            .execute(&pool).await.unwrap();
    }

    #[tokio::test]
    async fn object_transfer_waits_for_readers_and_refuses_active_writers() {
        let (dir, storage, pool, reporter) = fixture().await;
        add(&storage, &pool, dir.path(), "live").await;
        let writer = storage
            .try_lock_recording("bilibili", "123", "live")
            .unwrap();
        let result = storage
            .execute_object_transfer("a", "b", storage.reserve_transfer().unwrap(), &reporter)
            .await;
        assert!(result.error.is_some());
        assert!(storage
            .db
            .storage_transfer_journals()
            .await
            .unwrap()
            .is_empty());
        drop(writer);
        let reader = storage.recording("bilibili", "123", "live").await.unwrap();
        let execution = storage.execute_object_transfer(
            "a",
            "b",
            storage.reserve_transfer().unwrap(),
            &reporter,
        );
        tokio::pin!(execution);
        assert!(
            tokio::time::timeout(Duration::from_millis(30), &mut execution)
                .await
                .is_err()
        );
        assert!(storage
            .db
            .storage_transfer_journals()
            .await
            .unwrap()
            .is_empty());
        assert!(!dir.path().join("b/bilibili/123/live").exists());
        drop(reader);
        assert!(execution.await.error.is_none());
    }

    #[tokio::test]
    async fn object_transfer_db_commit_failure_keeps_source_and_resumes_after_restart() {
        let (dir, storage, pool, reporter) = fixture().await;
        add(&storage, &pool, dir.path(), "live").await;
        sqlx::raw_sql("CREATE TRIGGER reject_object_switch BEFORE UPDATE OF storage_id ON recording_storage_bindings \
            BEGIN SELECT RAISE(FAIL, 'injected DB failure'); END;").execute(&pool).await.unwrap();
        let result = storage
            .execute_object_transfer("a", "b", storage.reserve_transfer().unwrap(), &reporter)
            .await;
        assert_eq!(result.transferred_recordings, 0);
        assert!(result.error.unwrap().contains("绑定提交"));
        assert!(dir.path().join("a/bilibili/123/live/1.ts").exists());
        assert!(dir.path().join("b/bilibili/123/live/1.ts").exists());
        let journal = storage
            .db
            .storage_transfer_journals()
            .await
            .unwrap()
            .remove(0);
        assert_eq!(
            (journal.mode.as_str(), journal.phase.as_str()),
            ("object", "copying")
        );
        assert!(storage
            .try_lock_recording("bilibili", "123", "live")
            .is_err());
        sqlx::query("DROP TRIGGER reject_object_switch")
            .execute(&pool)
            .await
            .unwrap();
        let restarted = ArchiveStorage::new(storage.db.clone(), storage.config.clone());
        let result = restarted
            .execute_object_transfer("a", "b", restarted.reserve_transfer().unwrap(), &reporter)
            .await;
        assert!(result.error.is_none(), "{result:?}");
        assert_eq!(
            (result.transferred_recordings, result.cleaned_recordings),
            (1, 1)
        );
        assert_eq!(
            restarted
                .recording("bilibili", "123", "live")
                .await
                .unwrap()
                .read("1.ts")
                .await
                .unwrap(),
            b"actual bytes"
        );
        assert!(!dir.path().join("a/bilibili/123/live/1.ts").exists());
        assert!(restarted
            .db
            .storage_transfer_journals()
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn object_transfer_recovers_cleanup_and_finalizing_without_recopying() {
        for phase in ["cleanup", "finalizing"] {
            let (dir, storage, pool, reporter) = fixture().await;
            add(&storage, &pool, dir.path(), "live").await;
            let (gate, route) = storage.object_route("a", "b").await.unwrap();
            let transfer = route.archive("bilibili/123/live").unwrap();
            let manifest = transfer.inspect().await.unwrap();
            let journal = TransferJournal {
                platform: "bilibili".into(),
                room_id: "123".into(),
                live_id: "live".into(),
                source_id: "a".into(),
                target_id: "b".into(),
                token: "test-object-recovery".into(),
                manifest: serde_json::to_string(&manifest).unwrap(),
                phase: "copying".into(),
                mode: "object".into(),
            };
            storage.db.begin_storage_transfer(&journal).await.unwrap();
            transfer.copy(&journal.token, &manifest).await.unwrap();
            storage.db.commit_storage_transfer(&journal).await.unwrap();
            if phase == "cleanup" {
                std::fs::remove_file(dir.path().join("a/bilibili/123/live/1.ts")).unwrap();
            } else {
                transfer
                    .cleanup_source(&journal.token, &manifest)
                    .await
                    .unwrap();
                storage
                    .db
                    .finalize_storage_transfer(&journal.token)
                    .await
                    .unwrap();
                transfer.finish(&journal.token, &manifest).await.unwrap();
                // A terminal retry must never delete a recreated source.
                std::fs::write(dir.path().join("a/bilibili/123/live/unrelated"), b"keep").unwrap();
            }
            drop(gate);
            let restarted = ArchiveStorage::new(storage.db.clone(), storage.config.clone());
            let result = restarted
                .execute_object_transfer("a", "b", restarted.reserve_transfer().unwrap(), &reporter)
                .await;
            assert!(result.error.is_none(), "{result:?}");
            assert_eq!(
                (result.transferred_recordings, result.cleaned_recordings),
                (0, 1)
            );
            if phase == "finalizing" {
                assert_eq!(
                    std::fs::read(dir.path().join("a/bilibili/123/live/unrelated")).unwrap(),
                    b"keep"
                );
            }
        }
    }

    #[tokio::test]
    #[ignore = "requires RustFS and S3_TEST_* test credentials"]
    async fn rustfs_storage_transfer_all_directions() {
        use recording_storage::StorageBackend;
        let (dir, storage, pool, reporter) = fixture().await;
        let before = storage.configuration().await.unwrap();
        let mut config = before.clone();
        let prefix = format!("transfer-test-{}", uuid::Uuid::new_v4());
        eprintln!("RustFS transfer test prefix: {prefix}");
        let credentials = recording_storage::S3Credentials {
            access_key_id: std::env::var("S3_TEST_ACCESS_KEY_ID")
                .expect("set S3_TEST_ACCESS_KEY_ID"),
            secret_access_key: std::env::var("S3_TEST_SECRET_ACCESS_KEY")
                .expect("set S3_TEST_SECRET_ACCESS_KEY"),
            session_token: String::new(),
        };
        for id in ["cloud1", "cloud2"] {
            config.backends.insert(
                id.into(),
                RecordingBackendConfig {
                    webdav: None,
                    kind: "s3".into(),
                    root: String::new(),
                    s3: Some(recording_storage::S3Config {
                        endpoint: std::env::var("S3_TEST_ENDPOINT").expect("set S3_TEST_ENDPOINT"),
                        bucket: std::env::var("S3_TEST_BUCKET").expect("set S3_TEST_BUCKET"),
                        region: "us-east-1".into(),
                        prefix: format!("{prefix}/{id}"),
                        path_style: true,
                    }),
                },
            );
        }
        storage
            .replace_configuration_with_credentials(
                config.clone(),
                &before,
                &HashMap::from([
                    ("cloud1".into(), credentials.clone().into()),
                    ("cloud2".into(), credentials.clone().into()),
                ]),
            )
            .await
            .unwrap();
        // Exercise the signed streaming PUT's no-clobber guarantee against the
        // real service, not just the preflight existence check.
        let backend = recording_storage::S3Backend::new(
            config.backends["cloud1"].s3.as_ref().unwrap(),
            &credentials,
        )
        .unwrap();
        let payload = dir.path().join("upload");
        std::fs::write(&payload, b"first").unwrap();
        backend
            .create_from_file("conditional-test", &payload)
            .await
            .unwrap();
        std::fs::write(&payload, b"second").unwrap();
        assert_eq!(
            backend
                .create_from_file("conditional-test", &payload)
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(backend.read("conditional-test").await.unwrap(), b"first");
        backend.remove("conditional-test").await.unwrap();

        add(&storage, &pool, dir.path(), "live").await;
        let initial = storage
            .store("a")
            .await
            .unwrap()
            .scope("bilibili/123/live")
            .unwrap();
        initial
            .write("large.ts", &vec![0x5a; 3 * 1024 * 1024])
            .await
            .unwrap();
        initial
            .write("录像 100%+&.ts", b"special-key")
            .await
            .unwrap();
        initial
            .open_log("events.jsonl")
            .await
            .unwrap()
            .append(b"first\n")
            .await
            .unwrap();
        let saved_config = storage.configuration().await.unwrap();
        for (index, (source, target)) in [
            ("a", "cloud1"),
            ("cloud1", "cloud2"),
            ("cloud2", "b"),
            ("b", "cloud2"),
            ("cloud2", "cloud1"),
            ("cloud1", "a"),
        ]
        .into_iter()
        .enumerate()
        {
            let preview = storage
                .preview_storage_transfer(source, target)
                .await
                .unwrap();
            assert!(preview.can_transfer, "{preview:?}");
            assert_eq!(preview.transfer_mode, "object");
            assert_eq!(
                preview.available_bytes.is_none(),
                target.starts_with("cloud")
            );
            let result = storage
                .execute_storage_transfer(
                    source,
                    target,
                    storage.reserve_transfer().unwrap(),
                    &reporter,
                )
                .await;
            assert!(result.error.is_none(), "{source} -> {target}: {result:?}");
            assert_eq!(
                (result.transferred_recordings, result.cleaned_recordings),
                (1, 1)
            );
            let destination = storage.recording("bilibili", "123", "live").await.unwrap();
            assert_eq!(destination.read("1.ts").await.unwrap(), b"actual bytes");
            assert_eq!(
                destination.read(".private/checkpoint").await.unwrap(),
                b"checkpoint"
            );
            assert_eq!(
                destination.read("large.ts").await.unwrap(),
                vec![0x5a; 3 * 1024 * 1024]
            );
            assert_eq!(
                destination.read("录像 100%+&.ts").await.unwrap(),
                b"special-key"
            );
            assert_eq!(
                destination.read_log("events.jsonl").await.unwrap(),
                if index == 0 {
                    b"first\n".as_slice()
                } else {
                    b"first\nremote\n".as_slice()
                }
            );
            if index == 0 {
                // Create private object-log chunks that must survive every
                // subsequent S3/local direction along with the plain base log.
                destination
                    .open_log("events.jsonl")
                    .await
                    .unwrap()
                    .append(b"remote\n")
                    .await
                    .unwrap();
            }
            assert!(storage
                .store(source)
                .await
                .unwrap()
                .scope("bilibili/123/live")
                .unwrap()
                .list("", None, 1)
                .await
                .unwrap()
                .keys
                .is_empty());
            assert!(storage
                .db
                .storage_transfer_journals()
                .await
                .unwrap()
                .is_empty());
            assert_eq!(storage.configuration().await.unwrap(), saved_config);
            eprintln!("Verified transfer {source} -> {target}");
        }
    }
}
