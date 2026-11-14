//! Per-archive journal -> rename/copy publication -> transactional binding -> cleanup.
//! Filesystem operations belong to recording_storage; this module owns leases,
//! durable recovery state, task progress and fail-stop orchestration.
use super::{database_error, ArchiveStorage};
use crate::database::storage_transfer::TransferJournal;
use crate::progress::progress_reporter::{ProgressReporter, ProgressReporterTrait};
use recording_storage::{is_cross_device_error, LocalTransfer, RenameManifest, TransferManifest};
use serde::Serialize;
use std::{collections::BTreeMap, io, sync::Arc, time::Duration};
use tokio::sync::{OwnedMutexGuard, OwnedRwLockReadGuard};

const READER_DRAIN_TIMEOUT: Duration = Duration::from_secs(30);

#[path = "storage_transfer/objects.rs"]
mod objects;

#[derive(Clone, Debug, Serialize)]
pub struct StorageTransferPreview {
    pub source_backend_id: String,
    pub target_backend_id: String,
    pub recording_count: usize,
    pub total_bytes: u64,
    pub required_bytes: u64,
    pub available_bytes: Option<u64>,
    pub cleanup_count: usize,
    /// Root-level prediction, also accounting for durable pending copy journals.
    pub transfer_mode: &'static str,
    pub can_transfer: bool,
    pub blockers: Vec<String>,
}

#[derive(Default, Debug, Serialize)]
pub struct StorageTransferResult {
    pub transferred_recordings: usize,
    pub cleaned_recordings: usize,
    pub failed_recording: Option<String>,
    pub error: Option<String>,
}

struct Lease {
    _configuration: OwnedRwLockReadGuard<()>,
    _recordings: Vec<OwnedMutexGuard<()>>,
    _transfer: Arc<OwnedMutexGuard<()>>,
}

struct Plan {
    preview: StorageTransferPreview,
    transfer: Arc<LocalTransfer>,
    entries: Vec<(TransferJournal, TransferPlan, bool)>,
    estimated_sizes: BTreeMap<String, u64>,
    lease: Arc<Lease>,
}

#[derive(Clone)]
enum TransferPlan {
    Copy(TransferManifest),
    Rename(RenameManifest),
}

impl TransferPlan {
    fn from_journal(journal: &TransferJournal) -> io::Result<Self> {
        match journal.mode.as_str() {
            "copy" => serde_json::from_str(&journal.manifest)
                .map(Self::Copy)
                .map_err(io::Error::other),
            "rename" => serde_json::from_str(&journal.manifest)
                .map(Self::Rename)
                .map_err(io::Error::other),
            _ => Err(io::Error::other("unknown transfer journal mode")),
        }
    }

    fn serialized(&self) -> io::Result<String> {
        match self {
            Self::Copy(manifest) => serde_json::to_string(manifest),
            Self::Rename(manifest) => serde_json::to_string(manifest),
        }
        .map_err(io::Error::other)
    }

    fn mode(&self) -> &'static str {
        match self {
            Self::Copy(_) => "copy",
            Self::Rename(_) => "rename",
        }
    }

    fn check(&self, transfer: &LocalTransfer, key: &str, token: &str) -> io::Result<()> {
        match self {
            Self::Copy(manifest) => transfer.check_destination(key, token, manifest),
            Self::Rename(manifest) => transfer.check_rename(key, manifest),
        }
    }

    fn publish(&self, transfer: &LocalTransfer, key: &str, token: &str) -> io::Result<()> {
        match self {
            Self::Copy(manifest) => transfer.copy(key, token, manifest),
            Self::Rename(manifest) => transfer.rename(key, manifest),
        }
    }

    fn cleanup(&self, transfer: &LocalTransfer, key: &str, token: &str) -> io::Result<()> {
        match self {
            Self::Copy(manifest) => transfer.cleanup_source(key, token, manifest),
            Self::Rename(manifest) => transfer.finish_rename(key, manifest),
        }
    }

    fn finish(&self, transfer: &LocalTransfer, key: &str, token: &str) -> io::Result<()> {
        match self {
            Self::Copy(_) => transfer.finish(key, token),
            Self::Rename(manifest) => transfer.finish_rename(key, manifest),
        }
    }
}

async fn blocking<T: Send + 'static>(
    lease: impl Send + 'static,
    operation: impl FnOnce() -> io::Result<T> + Send + 'static,
) -> io::Result<T> {
    tokio::task::spawn_blocking(move || {
        // Retain guards even if the scheduler drops the awaiting future on shutdown.
        let _lease = lease;
        operation()
    })
    .await
    .map_err(io::Error::other)?
}

impl ArchiveStorage {
    fn copy_capacity(&self, transfer: &LocalTransfer, namespace: &str) -> io::Result<u64> {
        let available = transfer.copy_available_bytes(namespace)?;
        #[cfg(test)]
        let available = self
            .transfer_available_bytes
            .lock()
            .unwrap()
            .unwrap_or(available);
        Ok(available)
    }

    fn predicts_rename(&self, transfer: &LocalTransfer) -> bool {
        #[cfg(test)]
        if self
            .transfer_force_copy
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return false;
        }
        transfer.same_filesystem()
    }

    pub fn reserve_transfer(&self) -> io::Result<Arc<OwnedMutexGuard<()>>> {
        self.transfer_gate
            .clone()
            .try_lock_owned()
            .map(Arc::new)
            .map_err(|_| io::Error::new(io::ErrorKind::WouldBlock, "已有存储转移任务排队或运行中"))
    }

    pub async fn preview_storage_transfer(
        &self,
        source: &str,
        target: &str,
    ) -> io::Result<StorageTransferPreview> {
        let reservation = match self.reserve_transfer() {
            Ok(reservation) => reservation,
            Err(error) => {
                return Ok(StorageTransferPreview {
                    source_backend_id: source.into(),
                    target_backend_id: target.into(),
                    recording_count: 0,
                    total_bytes: 0,
                    required_bytes: 0,
                    available_bytes: None,
                    cleanup_count: 0,
                    transfer_mode: "copy",
                    can_transfer: false,
                    blockers: vec![error.to_string()],
                })
            }
        };
        if self.uses_object_transfer(source, target).await? {
            return self
                .preview_object_transfer(source, target, reservation)
                .await;
        }
        let (_gate, transfer) = self.local_transfer(source, target).await?;
        let rename = self.predicts_rename(&transfer);
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
            available_bytes: None,
            cleanup_count: 0,
            transfer_mode: if rename { "rename" } else { "copy" },
            can_transfer: false,
            blockers: Vec::new(),
        };
        let mut fresh_bytes = 0u64;
        let mut has_copy = false;
        let mut has_rename = false;
        for record in records {
            let key = format!("{}/{}/{}", record.platform, record.room_id, record.live_id);
            let matching_journal = record.source_id.as_deref() == Some(source)
                && record.target_id.as_deref() == Some(target);
            let copies = if matching_journal {
                record.mode.as_deref() != Some("rename")
            } else {
                !rename
            };
            has_copy |= copies;
            has_rename |= !copies;
            if record.phase.is_some() && !matching_journal {
                preview.blockers.push(format!(
                    "{key} 有另一项未完成的转移，请先重试 {} → {}",
                    record.source_id.as_deref().unwrap_or_default(),
                    record.target_id.as_deref().unwrap_or_default(),
                ));
            }
            if matching_journal && record.phase.as_deref() != Some("copying") {
                // Cleanup/finalizing already committed the binding. Neither
                // charge another copy nor credit space not yet reclaimed.
                preview.cleanup_count += 1;
            } else {
                preview.recording_count += 1;
                let bytes = u64::try_from(record.size).unwrap_or(0);
                preview.total_bytes = preview
                    .total_bytes
                    .checked_add(bytes)
                    .ok_or_else(|| io::Error::other("转移数据量溢出"))?;
                if copies {
                    preview.required_bytes = preview
                        .required_bytes
                        .checked_add(bytes)
                        .ok_or_else(|| io::Error::other("转移数据量溢出"))?;
                }
                if copies && !matching_journal {
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
        // Ordinary reads are transient, not mutation blockers. Preview cannot
        // reserve their absence; execution queues behind readers per archive.
        // A copying retry conservatively estimates a full copy: determining
        // reusable destination bytes would require reading archive objects.
        preview.transfer_mode = match (has_copy, has_rename) {
            (true, true) => "mixed",
            (true, false) => "copy",
            (false, true) => "rename",
            (false, false) => {
                if rename {
                    "rename"
                } else {
                    "copy"
                }
            }
        };
        let available = blocking((), move || transfer.available_bytes()).await?;
        #[cfg(test)]
        let available = self
            .transfer_available_bytes
            .lock()
            .unwrap()
            .unwrap_or(available);
        preview.available_bytes = Some(available);
        // A copying journal may already have a verified published destination
        // awaiting DB commit. Do not prevent recovery based on its conservative
        // full-size estimate; execution checks actual remaining bytes before
        // writing. Fresh archives still have to fit the preview estimate.
        if fresh_bytes > available {
            preview.blockers.push(format!(
                "目标空间不足：预计需要 {} 字节，可用 {} 字节",
                preview.required_bytes, available
            ));
        }
        preview.can_transfer = preview.blockers.is_empty();
        // Preview does not reserve a task or create/reconcile a durable journal.
        drop(reservation);
        Ok(preview)
    }

    /// Shared root policy; only root metadata is inspected, never archive files.
    async fn local_transfer(
        &self,
        source: &str,
        target: &str,
    ) -> io::Result<(OwnedRwLockReadGuard<()>, Arc<LocalTransfer>)> {
        self.initialize().await?;
        if source == target {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "来源与目标存储不能相同",
            ));
        }
        let gate = self.configuration_gate.clone().read_owned().await;
        let config = self.configuration().await?;
        let roots = [source, target].map(|id| {
            let backend = config.backends.get(id).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("存储后端 {id} 不存在或已移除"),
                )
            })?;
            if backend.kind != "local" {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "仅支持本地存储之间转移",
                ));
            }
            Ok(std::path::PathBuf::from(&backend.root))
        });
        let [source_root, target_root] = roots;
        let (source_root, target_root) = (source_root?, target_root?);
        let transfer = blocking((), move || {
            LocalTransfer::new(&source_root, &target_root).map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!("存储根目录不可用：请先创建目标目录并确认来源可访问，两端目录不得重叠或包含符号链接；{error}"),
                )
            })
        })
        .await?;
        Ok((gate, Arc::new(transfer)))
    }

    async fn plan_transfer(
        &self,
        source: &str,
        target: &str,
        reservation: Arc<OwnedMutexGuard<()>>,
    ) -> io::Result<Plan> {
        let (gate, transfer) = self.local_transfer(source, target).await?;
        // Root identity is only a preview hint. A namespace mount may differ
        // in either direction, so let the no-replace rename decide.
        let rename = true;
        #[cfg(test)]
        let rename = rename
            && !self
                .transfer_force_copy
                .load(std::sync::atomic::Ordering::Relaxed);
        let journals = self
            .db
            .storage_transfer_journals()
            .await
            .map_err(database_error)?;
        // The exclusive transfer reservation also lets a retry reconcile a
        // task dropped at a DB acknowledgement boundary in this same process.
        *self
            .pending_transfers
            .lock()
            .unwrap_or_else(|p| p.into_inner()) =
            journals.iter().map(TransferJournal::key).collect();
        let records = self
            .db
            .storage_transfer_recordings(source)
            .await
            .map_err(database_error)?;
        let estimated_sizes: BTreeMap<_, _> = self
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
        let mut work = BTreeMap::new();
        let mut blockers = Vec::new();
        for (platform, room_id, live_id) in records {
            let key = format!("{platform}/{room_id}/{live_id}");
            work.insert(key, (platform, room_id, live_id, None));
        }
        for journal in journals {
            if journal.source_id == source && journal.target_id == target {
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
                blockers.push(format!(
                    "{} 有另一项未完成的转移，请先重试 {} → {}",
                    journal.key(),
                    journal.source_id,
                    journal.target_id
                ));
            }
        }
        let mut guards = Vec::new();
        for (platform, room, live, _) in work.values() {
            match self.try_lock_recording_for_transfer(platform, room, live) {
                Ok(guard) => guards.push(guard),
                Err(error) => blockers.push(format!("{platform}/{room}/{live}: {error}")),
            }
        }
        let lease = Arc::new(Lease {
            _configuration: gate,
            _recordings: guards,
            _transfer: reservation,
        });
        let mut preview = StorageTransferPreview {
            source_backend_id: source.into(),
            target_backend_id: target.into(),
            recording_count: work
                .values()
                .filter(|(_, _, _, j)| j.as_ref().is_none_or(|j| j.phase == "copying"))
                .count(),
            total_bytes: 0,
            required_bytes: 0,
            available_bytes: None,
            cleanup_count: work
                .values()
                .filter(|(_, _, _, j)| j.as_ref().is_some_and(|j| j.phase != "copying"))
                .count(),
            transfer_mode: if rename { "rename" } else { "copy" },
            can_transfer: false,
            blockers,
        };
        let mut entries = Vec::new();
        // Never inspect a namespace while its writer/deleter is active.
        if preview.blockers.is_empty() {
            for (key, (platform, room_id, live_id, existing)) in work {
                let was_existing = existing.is_some();
                let estimated = estimated_sizes.get(&key).copied().unwrap_or(0);
                // Only DB metadata is used for space estimates. A retry can
                // already own destination bytes, so let writes report exhaustion.
                if !was_existing && !rename {
                    let available = self.copy_capacity(&transfer, &key)?;
                    if estimated > available {
                        preview.blockers.push(format!(
                            "{key}：目标空间不足：预计需要 {estimated} 字节，可用 {available} 字节"
                        ));
                        break;
                    }
                }
                let transfer = transfer.clone();
                let source = source.to_owned();
                let target = target.to_owned();
                let inspected = blocking(lease.clone(), move || {
                    let (journal, manifest) = match existing {
                        Some(journal) => {
                            let manifest = TransferPlan::from_journal(&journal)?;
                            (journal, manifest)
                        }
                        None => {
                            let manifest = if rename {
                                TransferPlan::Rename(transfer.prepare_rename(&key)?)
                            } else {
                                TransferPlan::Copy(transfer.inspect(&key)?)
                            };
                            let journal = TransferJournal {
                                platform,
                                room_id,
                                live_id,
                                source_id: source,
                                target_id: target,
                                token: uuid::Uuid::new_v4().to_string(),
                                manifest: manifest.serialized()?,
                                phase: "copying".into(),
                                mode: manifest.mode().into(),
                            };
                            (journal, manifest)
                        }
                    };
                    // Copy verification happens when this archive executes, not
                    // in a batch-wide capacity scan. Rename checks only metadata.
                    if journal.phase != "finalizing" && matches!(manifest, TransferPlan::Rename(_))
                    {
                        manifest.check(&transfer, &key, &journal.token)?;
                    }
                    Ok((journal, manifest))
                })
                .await;
                match inspected {
                    Ok((journal, manifest)) => {
                        let copying = journal.phase == "copying";
                        preview.total_bytes = preview
                            .total_bytes
                            .checked_add(if copying { estimated } else { 0 })
                            .ok_or_else(|| io::Error::other("转移数据量溢出"))?;
                        preview.required_bytes = preview
                            .required_bytes
                            .checked_add(if copying && journal.mode == "copy" {
                                estimated
                            } else {
                                0
                            })
                            .ok_or_else(|| io::Error::other("转移数据量溢出"))?;
                        entries.push((journal, manifest, was_existing));
                    }
                    Err(error) => {
                        preview.blockers.push(error.to_string());
                        break;
                    }
                }
            }
        }
        // Never scan archive contents to refine these estimates. Actual write
        // failures stop the current per-archive transaction without deleting A.
        preview.available_bytes = Some(transfer.available_bytes()?);
        #[cfg(test)]
        if let Some(available) = *self.transfer_available_bytes.lock().unwrap() {
            preview.available_bytes = Some(available);
        }
        preview.can_transfer = preview.blockers.is_empty();
        Ok(Plan {
            preview,
            transfer,
            entries,
            estimated_sizes,
            lease,
        })
    }

    pub async fn execute_storage_transfer(
        &self,
        source: &str,
        target: &str,
        reservation: Arc<OwnedMutexGuard<()>>,
        reporter: &ProgressReporter,
    ) -> StorageTransferResult {
        match self.uses_object_transfer(source, target).await {
            Ok(true) => {
                return self
                    .execute_object_transfer(source, target, reservation, reporter)
                    .await
            }
            Ok(false) => {}
            Err(error) => {
                return StorageTransferResult {
                    error: Some(error.to_string()),
                    ..Default::default()
                }
            }
        }
        let mut result = StorageTransferResult::default();
        reporter
            .update("正在检查存储路径、转移日志和可用空间")
            .await;
        let plan = match self.plan_transfer(source, target, reservation).await {
            Ok(plan) if plan.preview.can_transfer => plan,
            Ok(plan) => {
                result.error = Some(plan.preview.blockers.join("；"));
                return result;
            }
            Err(error) => {
                result.error = Some(error.to_string());
                return result;
            }
        };
        let count = plan.entries.len();
        for (index, (mut journal, mut manifest, existing)) in plan.entries.into_iter().enumerate() {
            let key = journal.key();
            let estimated = plan.estimated_sizes.get(&key).copied().unwrap_or(0);
            let operation: io::Result<()> = async {
                // Only the archive currently moving is unavailable to readers.
                // Hold through cleanup so a reader can never retain the old root.
                reporter
                    .update(&format!("转移 {}/{}：等待录播读取结束 {key}", index + 1, count))
                    .await;
                // Tokio's fair writer queue prevents fresh list/cover/HLS reads
                // from stealing the barrier while existing read leases drain.
                // Do not create a journal or mutate paths until it is acquired.
                let barrier = Arc::new(
                    tokio::time::timeout(
                        READER_DRAIN_TIMEOUT,
                        self.reader_barrier(&key).write_owned(),
                    )
                    .await
                    .map_err(|_| io::Error::new(
                        io::ErrorKind::TimedOut,
                        "等待录播读取结束超时（30 秒），本次未更改当前录播的文件和存储绑定；请结束播放或读取任务后重试",
                    ))?,
                );
                if !existing {
                    self.pending_transfers
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .insert(key.clone());
                    // Reserve before awaiting the durable write: dropping the
                    // task at the commit acknowledgement boundary must not
                    // permit a writer to mutate a journaled source.
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
                }
                if journal.phase == "copying" {
                    let copy_available = if !existing && matches!(manifest, TransferPlan::Copy(_)) {
                        Some(self.copy_capacity(&plan.transfer, &key)?)
                    } else {
                        None
                    };
                    reporter
                        .update(&format!("转移 {}/{}：{} {key}", index + 1, count,
                            if journal.mode == "rename" { "快速移动" } else { "复制并校验" }))
                        .await;
                    let transfer = plan.transfer.clone();
                    let copy_key = key.clone();
                    let token = journal.token.clone();
                    let publish_manifest = manifest.clone();
                    #[cfg(test)]
                    let rename_error = *self.transfer_rename_error.lock().unwrap();
                    let published = blocking((plan.lease.clone(), barrier.clone()), move || {
                        if let Some(available) = copy_available {
                            if estimated > available {
                                return Err(io::Error::other(format!(
                                    "目标空间不足：预计需要 {estimated} 字节，可用 {available} 字节"
                                )));
                            }
                        }
                        #[cfg(test)]
                        if matches!(publish_manifest, TransferPlan::Rename(_)) {
                            if let Some(code) = rename_error {
                                return Err(io::Error::from_raw_os_error(code));
                            }
                        }
                        publish_manifest.publish(&transfer, &copy_key, &token)
                    })
                    .await;
                    match published {
                        Err(error) if journal.mode == "rename" && is_cross_device_error(&error) => {
                            // Root volume is only a prediction: a nested mount
                            // can cross devices. No other rename failure permits
                            // copying. Persist the replacement manifest BEFORE
                            // any payload write, so restart never reinterprets it.
                            let available = self.copy_capacity(&plan.transfer, &key)?;
                            if estimated > available {
                                return Err(io::Error::other(format!(
                                    "跨文件系统复制预计空间不足：需要 {estimated} 字节，可用 {available} 字节；请重试转移"
                                )));
                            }
                            let transfer = plan.transfer.clone();
                            let fallback_key = key.clone();
                            let rename_manifest = manifest.clone();
                            let token = journal.token.clone();
                            let copy = blocking((plan.lease.clone(), barrier.clone()), move || {
                                rename_manifest.check(&transfer, &fallback_key, &token)?;
                                let copy = TransferPlan::Copy(transfer.inspect(&fallback_key)?);
                                rename_manifest.check(&transfer, &fallback_key, &token)?;
                                copy.check(&transfer, &fallback_key, &token)?;
                                Ok(copy)
                            }).await?;
                            let serialized = copy.serialized()?;
                            self.db.fallback_storage_transfer(&journal, &serialized)
                                .await.map_err(database_error)?;
                            journal.mode = "copy".into();
                            journal.manifest = serialized;
                            manifest = copy;
                            let transfer = plan.transfer.clone();
                            let copy_key = key.clone();
                            let token = journal.token.clone();
                            let copy = manifest.clone();
                            blocking((plan.lease.clone(), barrier.clone()), move || {
                                copy.publish(&transfer, &copy_key, &token)
                            }).await?;
                        }
                        Err(error) => return Err(io::Error::new(error.kind(),
                            format!("转移未完成，请使用原来源和目标重试：{error}"))),
                        Ok(()) => {}
                    }
                    reporter
                        .update(&format!("转移 {}/{}：提交存储绑定 {key}", index + 1, count))
                        .await;
                    self.db
                        .commit_storage_transfer(&journal)
                        .await
                        .map_err(|error| {
                            io::Error::other(format!(
                                "文件已转移，但数据库绑定提交未确认，请使用原来源和目标重试恢复：{error}"
                            ))
                        })?;
                    result.transferred_recordings += 1;
                }
                if journal.phase != "finalizing" {
                    reporter
                        .update(&format!(
                            "转移 {}/{}：绑定已提交，清理来源 {key}",
                            index + 1,
                            count
                        ))
                        .await;
                    let transfer = plan.transfer.clone();
                    let cleanup_key = key.clone();
                    let token = journal.token.clone();
                    let cleanup_manifest = manifest.clone();
                    blocking((plan.lease.clone(), barrier.clone()), move || {
                        cleanup_manifest.cleanup(&transfer, &cleanup_key, &token)
                    })
                    .await
                    .map_err(|error| {
                        io::Error::new(error.kind(), format!("绑定已提交，来源清理未完成：{error}"))
                    })?;
                    self.db
                        .finalize_storage_transfer(&journal.token)
                        .await
                        .map_err(|error| {
                            io::Error::other(format!("绑定已提交，无法确认来源清理完成：{error}"))
                        })?;
                }
                reporter
                    .update(&format!("转移 {}/{}：清理转移凭据 {key}", index + 1, count))
                    .await;
                // Persist a terminal phase before releasing proof. Retain the
                // reservation until proof deletion is durable; a crash can retry
                // finish even with no proof, but must never repeat source cleanup.
                let transfer = plan.transfer.clone();
                let finish_key = key.clone();
                let token = journal.token.clone();
                blocking((plan.lease.clone(), barrier), move || {
                    manifest.finish(&transfer, &finish_key, &token)
                })
                .await
                .map_err(|error| {
                    io::Error::new(
                        error.kind(),
                        format!("绑定已提交，来源已清理，转移凭据清理未完成：{error}"),
                    )
                })?;
                self.db
                    .finish_storage_transfer(&journal.token)
                    .await
                    .map_err(|error| {
                        io::Error::other(format!("绑定已提交，清理日志确认失败：{error}"))
                    })?;
                self.pending_transfers
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .remove(&key);
                result.cleaned_recordings += 1;
                Ok(())
            }
            .await;
            if let Err(error) = operation {
                result.failed_recording = Some(key);
                result.error = Some(error.to_string());
                break;
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, RecordingBackendConfig};
    use crate::events::EventBus;
    use std::path::Path;
    use tokio::sync::RwLock;

    pub(super) async fn fixture() -> (
        tempfile::TempDir,
        ArchiveStorage,
        sqlx::SqlitePool,
        ProgressReporter,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let (db, pool) = crate::archive_storage::tests::database(&dir.path().join("test.db")).await;
        sqlx::query("ALTER TABLE records ADD COLUMN size INTEGER NOT NULL DEFAULT 0")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::raw_sql("CREATE TABLE tasks (id TEXT PRIMARY KEY, type TEXT, status TEXT, message TEXT, metadata TEXT, created_at TEXT)")
            .execute(&pool).await.unwrap();
        let mut config: Config = toml::from_str(include_str!("../config.example.toml")).unwrap();
        config.storage.backends.clear();
        config.storage.default_backend = "a".into();
        for id in ["a", "b"] {
            std::fs::create_dir(dir.path().join(id)).unwrap();
            config.storage.backends.insert(
                id.into(),
                RecordingBackendConfig {
                    kind: "local".into(),
                    s3: None,
                    webdav: None,
                    root: dir
                        .path()
                        .join(id)
                        .canonicalize()
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .into(),
                },
            );
        }
        let storage = ArchiveStorage::new(db.clone(), Arc::new(RwLock::new(config)));
        // Existing tests exercise the copy engine deliberately, even though
        // their temporary roots happen to share a filesystem.
        storage
            .transfer_force_copy
            .store(true, std::sync::atomic::Ordering::Relaxed);
        storage.initialize().await.unwrap();
        let task = db
            .generate_task("storage_transfer", "", "{}")
            .await
            .unwrap();
        let reporter = ProgressReporter::new(db, &EventBus::new(), &task.id)
            .await
            .unwrap();
        (dir, storage, pool, reporter)
    }

    pub(super) async fn add(
        storage: &ArchiveStorage,
        pool: &sqlx::SqlitePool,
        root: &Path,
        live: &str,
    ) {
        sqlx::query(
            "INSERT INTO records(platform, room_id, live_id, size) VALUES ('bilibili', '123', ?, 7)",
        )
        .bind(live)
        .execute(pool)
        .await
        .unwrap();
        storage
            .db
            .bind_recording_storage("bilibili", "123", live, "a")
            .await
            .unwrap();
        let namespace = root.join("a/bilibili/123").join(live);
        std::fs::create_dir_all(namespace.join(".private")).unwrap();
        for (name, bytes) in [
            ("playlist.m3u8", b"manifest".as_slice()),
            ("1.ts", b"actual bytes".as_slice()),
            ("cover.jpg", b"cover".as_slice()),
            ("subtitle.srt", b"subtitle".as_slice()),
            (".private/checkpoint", b"checkpoint".as_slice()),
        ] {
            std::fs::write(namespace.join(name), bytes).unwrap();
        }
    }

    async fn execute(
        storage: &ArchiveStorage,
        reporter: &ProgressReporter,
    ) -> StorageTransferResult {
        storage
            .execute_storage_transfer("a", "b", storage.reserve_transfer().unwrap(), reporter)
            .await
    }

    fn enable_rename(storage: &ArchiveStorage) {
        storage
            .transfer_force_copy
            .store(false, std::sync::atomic::Ordering::Relaxed);
    }

    /// Drive the real transfer with its normal wakeups until its writer is
    /// queued. The caller holds a reader, so this cannot observe an acquired
    /// write guard. No timer, polling loop, or scheduler ordering is needed.
    async fn drive_until_reader_wait(
        transfer: &mut std::pin::Pin<&mut impl std::future::Future<Output = StorageTransferResult>>,
        barrier: &Arc<RwLock<()>>,
    ) {
        std::future::poll_fn(|cx| {
            assert!(
                transfer.as_mut().poll(cx).is_pending(),
                "transfer did not wait"
            );
            if barrier.try_read().is_err() {
                std::task::Poll::Ready(())
            } else {
                std::task::Poll::Pending
            }
        })
        .await;
    }

    async fn assert_unmoved(storage: &ArchiveStorage, root: &Path, live: &str) {
        assert_eq!(
            std::fs::read(root.join("a/bilibili/123").join(live).join("1.ts")).unwrap(),
            b"actual bytes"
        );
        assert!(!root.join("b/bilibili/123").join(live).exists());
        assert_eq!(
            storage
                .db
                .recording_storage_id("bilibili", "123", live)
                .await
                .unwrap()
                .as_deref(),
            Some("a")
        );
        assert!(storage
            .db
            .storage_transfer_journals()
            .await
            .unwrap()
            .is_empty());
        assert!(!storage
            .pending_transfers
            .lock()
            .unwrap()
            .contains(&format!("bilibili/123/{live}")));
    }

    #[tokio::test]
    async fn transfer_drains_readers_without_retry_or_starvation_for_rename_and_copy() {
        for rename in [true, false] {
            let (dir, storage, pool, reporter) = fixture().await;
            if rename {
                enable_rename(&storage);
            }
            add(&storage, &pool, dir.path(), "live").await;
            let reader = storage.recording("bilibili", "123", "live").await.unwrap();
            let preview = storage.preview_storage_transfer("a", "b").await.unwrap();
            assert!(preview.can_transfer, "{preview:?}");
            let barrier = storage.reader_barrier("bilibili/123/live");
            let transfer = execute(&storage, &reporter);
            tokio::pin!(transfer);
            drive_until_reader_wait(&mut transfer, &barrier).await;

            assert_unmoved(&storage, dir.path(), "live").await;
            // New polling readers fail fast rather than overtaking the writer.
            assert!(matches!(
                storage.recording("bilibili", "123", "live").await,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock
            ));
            // Existing readers can finish normally, and all clones must drain.
            assert_eq!(reader.read("1.ts").await.unwrap(), b"actual bytes");
            let last_reader = reader.clone();
            drop(reader);
            assert!(futures::poll!(transfer.as_mut()).is_pending());
            assert_unmoved(&storage, dir.path(), "live").await;
            let progress: String = sqlx::query_scalar("SELECT message FROM tasks WHERE id = ?")
                .bind(&reporter.event_id)
                .fetch_one(&pool)
                .await
                .unwrap();
            assert!(progress.contains("等待录播读取结束"), "{progress}");
            drop(last_reader);

            let result = transfer.await;
            assert!(result.error.is_none(), "{result:?}");
            assert_eq!(
                (result.transferred_recordings, result.cleaned_recordings),
                (1, 1)
            );
            assert!(!dir.path().join("a/bilibili/123/live").exists());
            let moved = storage.recording("bilibili", "123", "live").await.unwrap();
            assert_eq!(moved.read("1.ts").await.unwrap(), b"actual bytes");
            assert_eq!(
                storage
                    .db
                    .recording_storage_id("bilibili", "123", "live")
                    .await
                    .unwrap()
                    .as_deref(),
                Some("b")
            );
        }
    }

    #[tokio::test]
    async fn reader_timeout_preserves_current_archive_and_prior_completed_results() {
        for rename in [true, false] {
            let (dir, storage, pool, reporter) = fixture().await;
            if rename {
                enable_rename(&storage);
            }
            add(&storage, &pool, dir.path(), "first").await;
            add(&storage, &pool, dir.path(), "second").await;
            let reader = storage
                .recording("bilibili", "123", "second")
                .await
                .unwrap();
            let barrier = storage.reader_barrier("bilibili/123/second");
            let transfer = execute(&storage, &reporter);
            tokio::pin!(transfer);
            drive_until_reader_wait(&mut transfer, &barrier).await;
            assert_unmoved(&storage, dir.path(), "second").await;
            assert!(!dir.path().join("a/bilibili/123/first").exists());
            assert_eq!(
                storage
                    .db
                    .recording_storage_id("bilibili", "123", "first")
                    .await
                    .unwrap()
                    .as_deref(),
                Some("b")
            );
            tokio::time::pause();
            tokio::time::advance(READER_DRAIN_TIMEOUT + Duration::from_secs(1)).await;
            let result = transfer.await;
            tokio::time::resume();
            assert_eq!(
                (result.transferred_recordings, result.cleaned_recordings),
                (1, 1)
            );
            assert_eq!(
                result.failed_recording.as_deref(),
                Some("bilibili/123/second")
            );
            assert!(result
                .error
                .as_deref()
                .unwrap()
                .contains("等待录播读取结束超时"));
            assert_unmoved(&storage, dir.path(), "second").await;
            // Timeout cancels the queued writer and releases mutation/task
            // reservations without invalidating the still-live original read.
            assert!(storage.recording("bilibili", "123", "second").await.is_ok());
            assert!(storage
                .try_lock_recording("bilibili", "123", "second")
                .is_ok());
            assert!(storage.reserve_transfer().is_ok());
            assert_eq!(reader.read("1.ts").await.unwrap(), b"actual bytes");
        }
    }

    #[tokio::test]
    async fn transfer_schema_requires_an_explicit_valid_mode() {
        let dir = tempfile::tempdir().unwrap();
        let (db, pool) = crate::archive_storage::tests::database(&dir.path().join("test.db")).await;
        sqlx::raw_sql(crate::database::storage_transfer::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        let missing_mode = sqlx::query(
            "INSERT INTO recording_storage_transfers \
             (platform, room_id, live_id, source_id, target_id, token, manifest, phase) \
             VALUES ('bilibili', '123', 'live', 'a', 'b', 'old-token', 'old-manifest', 'cleanup')",
        )
        .execute(&pool)
        .await;
        assert!(missing_mode.is_err());
        for mode in ["invalid", "copy"] {
            let inserted = sqlx::query(
                "INSERT INTO recording_storage_transfers \
                 (platform, room_id, live_id, source_id, target_id, token, manifest, phase, mode) \
                 VALUES ('bilibili', '123', 'live', 'a', 'b', 'token', 'manifest', 'cleanup', ?)",
            )
            .bind(mode)
            .execute(&pool)
            .await;
            assert_eq!(inserted.is_ok(), mode == "copy");
        }
        let journals = db.storage_transfer_journals().await.unwrap();
        assert_eq!(journals.len(), 1);
        assert_eq!(journals[0].mode, "copy");
        assert_eq!(journals[0].manifest, "manifest");
        assert_eq!(journals[0].phase, "cleanup");
        assert_eq!(journals[0].token, "token");
    }

    #[tokio::test]
    async fn rename_ignores_payload_size_and_needs_no_copy_capacity() {
        let (dir, storage, pool, reporter) = fixture().await;
        enable_rename(&storage);
        add(&storage, &pool, dir.path(), "live").await;
        sqlx::query("UPDATE records SET size = 9223372036854775807")
            .execute(&pool)
            .await
            .unwrap();
        *storage.transfer_available_bytes.lock().unwrap() = Some(0);
        let preview = storage.preview_storage_transfer("a", "b").await.unwrap();
        assert!(preview.can_transfer, "{preview:?}");
        assert_eq!(preview.transfer_mode, "rename");
        assert_eq!(preview.required_bytes, 0);
        assert_eq!(preview.total_bytes, i64::MAX as u64);
        let result = execute(&storage, &reporter).await;
        assert!(result.error.is_none(), "{result:?}");
        assert!(!dir.path().join("a/bilibili/123/live").exists());
        assert_eq!(
            std::fs::read(dir.path().join("b/bilibili/123/live/1.ts")).unwrap(),
            b"actual bytes"
        );
        let reverse = storage
            .execute_storage_transfer("b", "a", storage.reserve_transfer().unwrap(), &reporter)
            .await;
        assert!(reverse.error.is_none(), "{reverse:?}");
        let forward = execute(&storage, &reporter).await;
        assert!(forward.error.is_none(), "{forward:?}");
    }

    #[tokio::test]
    async fn rename_db_failure_blocks_source_reads_and_recovers_after_restart() {
        let (dir, storage, pool, reporter) = fixture().await;
        enable_rename(&storage);
        add(&storage, &pool, dir.path(), "live").await;
        sqlx::raw_sql("CREATE TRIGGER reject_switch BEFORE UPDATE OF storage_id ON recording_storage_bindings \
            BEGIN SELECT RAISE(ABORT, 'injected switch failure'); END;")
            .execute(&pool).await.unwrap();
        let result = execute(&storage, &reporter).await;
        assert!(result.error.is_some(), "{result:?}");
        assert!(!dir.path().join("a/bilibili/123/live").exists());
        assert!(dir.path().join("b/bilibili/123/live/1.ts").exists());
        assert_eq!(
            storage
                .db
                .recording_storage_id("bilibili", "123", "live")
                .await
                .unwrap()
                .as_deref(),
            Some("a")
        );
        assert!(matches!(storage.recording("bilibili", "123", "live").await,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock));
        let restarted = ArchiveStorage::new(storage.db.clone(), storage.config.clone());
        assert!(
            matches!(restarted.recording("bilibili", "123", "live").await,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock)
        );
        sqlx::raw_sql("DROP TRIGGER reject_switch")
            .execute(&pool)
            .await
            .unwrap();
        let result = execute(&restarted, &reporter).await;
        assert!(result.error.is_none(), "{result:?}");
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
    }

    #[tokio::test]
    async fn rename_restart_recovers_every_durable_phase_without_rollback() {
        for phase in ["copying", "cleanup", "finalizing"] {
            let (dir, storage, pool, reporter) = fixture().await;
            enable_rename(&storage);
            add(&storage, &pool, dir.path(), "live").await;
            let plan = storage
                .plan_transfer("a", "b", storage.reserve_transfer().unwrap())
                .await
                .unwrap();
            let (journal, manifest, _) = &plan.entries[0];
            storage.db.begin_storage_transfer(journal).await.unwrap();
            manifest
                .publish(&plan.transfer, &journal.key(), &journal.token)
                .unwrap();
            if phase != "copying" {
                storage.db.commit_storage_transfer(journal).await.unwrap();
            }
            if phase == "finalizing" {
                manifest
                    .cleanup(&plan.transfer, &journal.key(), &journal.token)
                    .unwrap();
                storage
                    .db
                    .finalize_storage_transfer(&journal.token)
                    .await
                    .unwrap();
            }
            drop(plan); // crash/unknown commit acknowledgement
            let restarted = ArchiveStorage::new(storage.db.clone(), storage.config.clone());
            if phase != "copying" {
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
            }
            let result = execute(&restarted, &reporter).await;
            assert!(result.error.is_none(), "{phase}: {result:?}");
            assert_eq!(
                result.transferred_recordings,
                usize::from(phase == "copying")
            );
            assert!(storage
                .db
                .storage_transfer_journals()
                .await
                .unwrap()
                .is_empty());
            assert!(!dir.path().join("a/bilibili/123/live").exists());
        }
    }

    #[tokio::test]
    async fn rename_finalization_failure_keeps_journal_resumable() {
        let (dir, storage, pool, reporter) = fixture().await;
        enable_rename(&storage);
        add(&storage, &pool, dir.path(), "live").await;
        sqlx::raw_sql(
            "CREATE TRIGGER reject_finish BEFORE DELETE ON recording_storage_transfers \
            BEGIN SELECT RAISE(ABORT, 'injected finalization failure'); END;",
        )
        .execute(&pool)
        .await
        .unwrap();
        let result = execute(&storage, &reporter).await;
        assert!(result.error.is_some(), "{result:?}");
        assert_eq!(
            storage.db.storage_transfer_journals().await.unwrap()[0].phase,
            "finalizing"
        );
        sqlx::raw_sql("DROP TRIGGER reject_finish")
            .execute(&pool)
            .await
            .unwrap();
        let result = execute(&storage, &reporter).await;
        assert!(result.error.is_none(), "{result:?}");
        assert_eq!(
            (result.transferred_recordings, result.cleaned_recordings),
            (0, 1)
        );
    }

    #[tokio::test]
    async fn rename_recreated_paths_never_rebind_or_delete_unrelated_data() {
        for recreate_source in [true, false] {
            for phase in ["copying", "cleanup", "finalizing"] {
                let (dir, storage, pool, reporter) = fixture().await;
                enable_rename(&storage);
                add(&storage, &pool, dir.path(), "live").await;
                let plan = storage
                    .plan_transfer("a", "b", storage.reserve_transfer().unwrap())
                    .await
                    .unwrap();
                let (journal, manifest, _) = &plan.entries[0];
                storage.db.begin_storage_transfer(journal).await.unwrap();
                manifest
                    .publish(&plan.transfer, &journal.key(), &journal.token)
                    .unwrap();
                if phase != "copying" {
                    storage.db.commit_storage_transfer(journal).await.unwrap();
                }
                if phase == "finalizing" {
                    manifest
                        .cleanup(&plan.transfer, &journal.key(), &journal.token)
                        .unwrap();
                    storage
                        .db
                        .finalize_storage_transfer(&journal.token)
                        .await
                        .unwrap();
                }
                drop(plan);
                let path = dir.path().join(if recreate_source {
                    "a/bilibili/123/live"
                } else {
                    "b/bilibili/123/live"
                });
                if !recreate_source {
                    std::fs::rename(&path, dir.path().join("original")).unwrap();
                }
                std::fs::create_dir(&path).unwrap();
                std::fs::write(path.join("unrelated"), b"keep").unwrap();
                let result = execute(&storage, &reporter).await;
                assert!(result.error.is_some(), "{result:?}");
                assert_eq!(std::fs::read(path.join("unrelated")).unwrap(), b"keep");
                assert_eq!(
                    storage
                        .db
                        .recording_storage_id("bilibili", "123", "live")
                        .await
                        .unwrap()
                        .as_deref(),
                    Some(if phase == "copying" { "a" } else { "b" })
                );
            }
        }
    }

    #[tokio::test]
    async fn only_cross_device_errors_persist_copy_fallback_and_recheck_capacity() {
        #[cfg(unix)]
        let cross_device = 18; // EXDEV on supported Linux/macOS targets
        #[cfg(windows)]
        let cross_device = 17;
        for (code, available) in [
            (cross_device, 0),
            (cross_device, 20), // DB estimate fits even though actual files exceed it.
            (cross_device, u64::MAX),
            (13, u64::MAX),
        ] {
            let (dir, storage, pool, reporter) = fixture().await;
            enable_rename(&storage);
            add(&storage, &pool, dir.path(), "live").await;
            *storage.transfer_rename_error.lock().unwrap() = Some(code);
            *storage.transfer_available_bytes.lock().unwrap() = Some(available);
            let result = execute(&storage, &reporter).await;
            if code == cross_device && available != 0 {
                assert!(result.error.is_none(), "{result:?}");
                assert!(!dir.path().join("a/bilibili/123/live").exists());
                assert!(dir.path().join("b/bilibili/123/live/1.ts").exists());
            } else {
                assert!(result.error.is_some(), "{result:?}");
                assert!(dir.path().join("a/bilibili/123/live/1.ts").exists());
                assert!(!dir.path().join("b/bilibili/123/live").exists());
                let journal = storage
                    .db
                    .storage_transfer_journals()
                    .await
                    .unwrap()
                    .remove(0);
                // Insufficient DB capacity stops before preparing a copy
                // manifest, leaving the durable rename intent unchanged.
                assert_eq!(journal.mode, "rename");
                if code == cross_device {
                    *storage.transfer_available_bytes.lock().unwrap() = Some(u64::MAX);
                    // Retry receives EXDEV again, then copies once its estimate fits.
                    let result = execute(&storage, &reporter).await;
                    assert!(result.error.is_none(), "{result:?}");
                }
            }
        }
    }

    #[tokio::test]
    async fn preview_reports_mixed_without_deserializing_legacy_copy_manifest() {
        let (dir, storage, pool, _reporter) = fixture().await;
        add(&storage, &pool, dir.path(), "old").await;
        let plan = storage
            .plan_transfer("a", "b", storage.reserve_transfer().unwrap())
            .await
            .unwrap();
        storage
            .db
            .begin_storage_transfer(&plan.entries[0].0)
            .await
            .unwrap();
        drop(plan);
        sqlx::query(
            "UPDATE recording_storage_transfers SET manifest = 'invalid, preview must not read'",
        )
        .execute(&pool)
        .await
        .unwrap();
        add(&storage, &pool, dir.path(), "new").await;
        enable_rename(&storage);
        *storage.transfer_available_bytes.lock().unwrap() = Some(0);
        let preview = storage.preview_storage_transfer("a", "b").await.unwrap();
        assert_eq!(preview.transfer_mode, "mixed");
        assert_eq!((preview.total_bytes, preview.required_bytes), (14, 7));
        assert!(preview.can_transfer, "{preview:?}");
    }

    #[tokio::test]
    async fn transfer_preserves_all_bytes_preferences_and_reads_without_recorder() {
        let (dir, storage, pool, reporter) = fixture().await;
        for live in ["first", "second"] {
            add(&storage, &pool, dir.path(), live).await;
        }
        let before = storage.configuration().await.unwrap();
        let preview = storage.preview_storage_transfer("a", "b").await.unwrap();
        assert!(preview.can_transfer);
        assert_eq!(preview.recording_count, 2);
        assert_eq!(preview.total_bytes, 14); // DB estimate; actual files total 86 bytes.
        assert_eq!(preview.required_bytes, 14);
        let result = execute(&storage, &reporter).await;
        assert!(result.error.is_none(), "{result:?}");
        assert_eq!(
            (result.transferred_recordings, result.cleaned_recordings),
            (2, 2)
        );
        for live in ["first", "second"] {
            assert!(!dir.path().join("a/bilibili/123").join(live).exists());
            let recording = storage.recording("bilibili", "123", live).await.unwrap();
            assert_eq!(recording.read("1.ts").await.unwrap(), b"actual bytes");
            assert_eq!(
                recording.read(".private/checkpoint").await.unwrap(),
                b"checkpoint"
            );
            assert_eq!(
                storage
                    .db
                    .recording_storage_id("bilibili", "123", live)
                    .await
                    .unwrap()
                    .as_deref(),
                Some("b")
            );
        }
        assert_eq!(storage.configuration().await.unwrap(), before);
        // A queued resume event still names A after its writer lease ended.
        storage
            .confirm_recording_start("bilibili", "123", "first", Some("a"))
            .await
            .unwrap();
        assert_eq!(
            storage
                .db
                .recording_storage_id("bilibili", "123", "first")
                .await
                .unwrap()
                .as_deref(),
            Some("b")
        );
        // The normal binding API must remain immutable.
        assert!(storage
            .bind_recording("bilibili", "123", "first", Some("a"))
            .await
            .is_err());
        assert!(storage
            .db
            .storage_transfer_journals()
            .await
            .unwrap()
            .is_empty());
        assert_eq!(execute(&storage, &reporter).await.transferred_recordings, 0);
    }

    #[tokio::test]
    async fn preview_includes_legacy_and_pinned_rows_without_reading_missing_archives() {
        let (dir, storage, pool, reporter) = fixture().await;
        sqlx::raw_sql(
            "INSERT INTO records(platform, room_id, live_id, size) VALUES
             ('bilibili', '123', 'legacy', 100), ('bilibili', '123', 'pinned', 200),
             ('bilibili', '123', 'negative', -10), ('bilibili', '123', 'other', 400);
             INSERT INTO recording_storage_bindings(platform, room_id, live_id, storage_id) VALUES
             ('bilibili', '123', 'pinned', 'a'), ('bilibili', '123', 'other', 'b');",
        )
        .execute(&pool)
        .await
        .unwrap();
        let preview = storage.preview_storage_transfer("a", "b").await.unwrap();
        assert!(preview.can_transfer, "{preview:?}");
        assert_eq!(
            (
                preview.recording_count,
                preview.total_bytes,
                preview.required_bytes
            ),
            (3, 300, 300)
        );
        assert_eq!(std::fs::read_dir(dir.path().join("a")).unwrap().count(), 0);
        assert_eq!(std::fs::read_dir(dir.path().join("b")).unwrap().count(), 0);
        assert!(storage
            .db
            .storage_transfer_journals()
            .await
            .unwrap()
            .is_empty());
        assert!(storage
            .db
            .recording_storage_id("bilibili", "123", "legacy")
            .await
            .unwrap()
            .is_none());
        assert!(storage
            .try_lock_recording("bilibili", "123", "legacy")
            .is_ok());
        assert!(execute(&storage, &reporter).await.error.is_some());
        assert!(storage
            .db
            .storage_transfer_journals()
            .await
            .unwrap()
            .is_empty());
        assert_eq!(std::fs::read_dir(dir.path().join("b")).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn preview_rejects_size_overflow_without_wrapping_or_sql_sum_overflow() {
        let (_dir, storage, pool, _reporter) = fixture().await;
        for live in ["one", "two", "three"] {
            sqlx::query("INSERT INTO records(platform, room_id, live_id, size) VALUES ('bilibili', '123', ?, ?)")
                .bind(live).bind(i64::MAX).execute(&pool).await.unwrap();
        }
        assert!(storage
            .preview_storage_transfer("a", "b")
            .await
            .unwrap_err()
            .to_string()
            .contains("溢出"));
        assert!(storage.reserve_transfer().is_ok());
    }

    #[tokio::test]
    async fn execution_budgets_db_size_not_full_manifest_size() {
        let (dir, storage, pool, reporter) = fixture().await;
        add(&storage, &pool, dir.path(), "live").await;
        *storage.transfer_available_bytes.lock().unwrap() = Some(20);
        let preview = storage.preview_storage_transfer("a", "b").await.unwrap();
        assert!(preview.can_transfer);
        assert_eq!(preview.required_bytes, 7);
        let plan = storage
            .plan_transfer("a", "b", storage.reserve_transfer().unwrap())
            .await
            .unwrap();
        assert!(plan.preview.can_transfer, "{:?}", plan.preview);
        assert_eq!(
            (plan.preview.total_bytes, plan.preview.required_bytes),
            (7, 7)
        );
        drop(plan);
        let result = execute(&storage, &reporter).await;
        assert!(result.error.is_none(), "{result:?}");
        assert_eq!(
            (result.transferred_recordings, result.cleaned_recordings),
            (1, 1)
        );
        assert!(storage
            .db
            .storage_transfer_journals()
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            storage
                .db
                .recording_storage_id("bilibili", "123", "live")
                .await
                .unwrap()
                .as_deref(),
            Some("b")
        );
        assert!(!dir.path().join("a/bilibili/123/live").exists());
        assert_eq!(
            std::fs::read(dir.path().join("b/bilibili/123/live/1.ts")).unwrap(),
            b"actual bytes"
        );
    }

    #[tokio::test]
    async fn copy_validation_is_per_archive_not_a_batch_capacity_scan() {
        let (dir, storage, pool, reporter) = fixture().await;
        for live in ["first", "second"] {
            add(&storage, &pool, dir.path(), live).await;
        }
        let plan = storage
            .plan_transfer("a", "b", storage.reserve_transfer().unwrap())
            .await
            .unwrap();
        for (journal, _, _) in &plan.entries {
            storage.db.begin_storage_transfer(journal).await.unwrap();
        }
        drop(plan);
        std::fs::write(dir.path().join("a/bilibili/123/second/1.ts"), b"changed").unwrap();
        let result = execute(&storage, &reporter).await;
        assert_eq!(
            (result.transferred_recordings, result.cleaned_recordings),
            (1, 1)
        );
        assert_eq!(
            result.failed_recording.as_deref(),
            Some("bilibili/123/second")
        );
        assert!(result.error.is_some());
        assert!(!dir.path().join("a/bilibili/123/first").exists());
        assert_eq!(
            std::fs::read(dir.path().join("b/bilibili/123/first/1.ts")).unwrap(),
            b"actual bytes"
        );
        assert_eq!(
            storage
                .db
                .recording_storage_id("bilibili", "123", "second")
                .await
                .unwrap()
                .as_deref(),
            Some("a")
        );
        assert_eq!(
            std::fs::read(dir.path().join("a/bilibili/123/second/1.ts")).unwrap(),
            b"changed"
        );
        assert!(!dir.path().join("b/bilibili/123/second").exists());
    }

    #[tokio::test]
    async fn preview_does_not_read_corrupted_pending_source_but_execution_rejects_it() {
        let (dir, storage, pool, reporter) = fixture().await;
        add(&storage, &pool, dir.path(), "live").await;
        let plan = storage
            .plan_transfer("a", "b", storage.reserve_transfer().unwrap())
            .await
            .unwrap();
        let (journal, _, _) = &plan.entries[0];
        storage.db.begin_storage_transfer(journal).await.unwrap();
        drop(plan);
        std::fs::write(dir.path().join("a/bilibili/123/live/1.ts"), b"corrupt data").unwrap();
        // No payload hashing during batch capacity planning. Per-archive
        // execution must still reject a changed source.
        let plan = storage
            .plan_transfer("a", "b", storage.reserve_transfer().unwrap())
            .await
            .unwrap();
        assert!(plan.preview.can_transfer, "{:?}", plan.preview);
        drop(plan);
        let preview = storage.preview_storage_transfer("a", "b").await.unwrap();
        assert!(preview.can_transfer, "{preview:?}");
        assert_eq!(
            (
                preview.recording_count,
                preview.total_bytes,
                preview.required_bytes
            ),
            (1, 7, 7)
        );
        let result = execute(&storage, &reporter).await;
        assert!(result.error.is_some(), "{result:?}");
        assert_eq!(result.transferred_recordings, 0);
        assert_eq!(
            storage
                .db
                .recording_storage_id("bilibili", "123", "live")
                .await
                .unwrap()
                .as_deref(),
            Some("a")
        );
        assert_eq!(
            std::fs::read(dir.path().join("a/bilibili/123/live/1.ts")).unwrap(),
            b"corrupt data"
        );
        assert!(!dir.path().join("b/bilibili/123/live").exists());
    }

    #[tokio::test]
    async fn copying_retry_does_not_require_full_manifest_capacity_upfront() {
        let (dir, storage, pool, reporter) = fixture().await;
        add(&storage, &pool, dir.path(), "live").await;
        let plan = storage
            .plan_transfer("a", "b", storage.reserve_transfer().unwrap())
            .await
            .unwrap();
        let (journal, _, _) = &plan.entries[0];
        storage.db.begin_storage_transfer(journal).await.unwrap();
        drop(plan);
        *storage.transfer_available_bytes.lock().unwrap() = Some(0);
        let preview = storage.preview_storage_transfer("a", "b").await.unwrap();
        assert!(preview.can_transfer, "{preview:?}");
        assert_eq!(
            (preview.required_bytes, preview.available_bytes),
            (7, Some(0))
        );
        // This override controls the estimate, not the real temporary disk.
        // Streaming writes still report actual exhaustion independently.
        let result = execute(&storage, &reporter).await;
        assert!(result.error.is_none(), "{result:?}");
        assert_eq!(
            (result.transferred_recordings, result.cleaned_recordings),
            (1, 1)
        );
        assert_eq!(
            storage
                .db
                .recording_storage_id("bilibili", "123", "live")
                .await
                .unwrap()
                .as_deref(),
            Some("b")
        );
        assert!(!dir.path().join("a/bilibili/123/live").exists());
        assert_eq!(
            std::fs::read(dir.path().join("b/bilibili/123/live/1.ts")).unwrap(),
            b"actual bytes"
        );
    }

    #[tokio::test]
    async fn preview_uses_journal_phase_not_manifests_and_never_credits_cleanup_space() {
        let (dir, storage, pool, _reporter) = fixture().await;
        add(&storage, &pool, dir.path(), "fresh").await;
        for live in ["copying", "cleanup", "finalizing"] {
            add(&storage, &pool, dir.path(), live).await;
            let journal = TransferJournal {
                platform: "bilibili".into(),
                room_id: "123".into(),
                live_id: live.into(),
                source_id: "a".into(),
                target_id: "b".into(),
                token: live.into(),
                // Invalid manifests prove preview does not deserialize them.
                manifest: "not a manifest".into(),
                phase: "copying".into(),
                mode: "copy".into(),
            };
            storage.db.begin_storage_transfer(&journal).await.unwrap();
            if live != "copying" {
                storage.db.commit_storage_transfer(&journal).await.unwrap();
            }
            if live == "finalizing" {
                storage
                    .db
                    .finalize_storage_transfer(&journal.token)
                    .await
                    .unwrap();
                // A persisted journal still counts after the record disappears.
                sqlx::query("DELETE FROM records WHERE live_id = 'finalizing'")
                    .execute(&pool)
                    .await
                    .unwrap();
            }
        }
        // Restart also loads only journal keys, not manifests.
        let restarted = ArchiveStorage::new(storage.db.clone(), storage.config.clone());
        restarted
            .transfer_force_copy
            .store(true, std::sync::atomic::Ordering::Relaxed);
        *restarted.transfer_available_bytes.lock().unwrap() = Some(6);
        let preview = restarted.preview_storage_transfer("a", "b").await.unwrap();
        assert_eq!((preview.recording_count, preview.cleanup_count), (2, 2));
        assert_eq!(
            (
                preview.total_bytes,
                preview.required_bytes,
                preview.available_bytes
            ),
            (14, 14, Some(6))
        );
        assert!(!preview.can_transfer);
        assert!(preview.blockers.iter().any(|b| b.contains("空间不足")));
        let reverse = restarted.preview_storage_transfer("b", "a").await.unwrap();
        assert!(!reverse.can_transfer);
        assert!(reverse.blockers.iter().any(|b| b.contains("另一项未完成")));
        assert_eq!(
            storage.db.storage_transfer_journals().await.unwrap().len(),
            3
        );
        assert!(dir.path().join("a/bilibili/123/cleanup/1.ts").exists());
        assert_eq!(std::fs::read_dir(dir.path().join("b")).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn delayed_start_cannot_rebind_unpersisted_or_deleted_archives() {
        let (_dir, storage, _pool, _reporter) = fixture().await;
        storage
            .bind_recording("bilibili", "123", "unpersisted", Some("a"))
            .await
            .unwrap();
        assert!(storage
            .confirm_recording_start("bilibili", "123", "unpersisted", Some("b"))
            .await
            .is_err());
        storage
            .forget_recording("bilibili", "123", "unpersisted")
            .await
            .unwrap();
        assert!(storage
            .confirm_recording_start("bilibili", "123", "unpersisted", Some("a"))
            .await
            .is_err());
        assert!(storage
            .db
            .recording_storage_id("bilibili", "123", "unpersisted")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn insufficient_space_and_stale_preview_make_no_writes() {
        let (dir, storage, pool, reporter) = fixture().await;
        add(&storage, &pool, dir.path(), "live").await;
        assert!(
            storage
                .preview_storage_transfer("a", "b")
                .await
                .unwrap()
                .can_transfer
        );
        *storage.transfer_available_bytes.lock().unwrap() = Some(0);
        let preview = storage.preview_storage_transfer("a", "b").await.unwrap();
        assert!(!preview.can_transfer);
        let result = execute(&storage, &reporter).await;
        assert!(result.error.unwrap().contains("空间不足"));
        assert_eq!(std::fs::read_dir(dir.path().join("b")).unwrap().count(), 0);
        assert!(storage
            .db
            .storage_transfer_journals()
            .await
            .unwrap()
            .is_empty());
        assert!(dir.path().join("a/bilibili/123/live/1.ts").exists());
    }

    #[tokio::test]
    async fn unknown_target_conflict_is_not_overwritten() {
        let (dir, storage, pool, reporter) = fixture().await;
        add(&storage, &pool, dir.path(), "live").await;
        let target = dir.path().join("b/bilibili/123/live");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("mine"), b"unrelated").unwrap();
        assert!(
            storage
                .preview_storage_transfer("a", "b")
                .await
                .unwrap()
                .can_transfer
        );
        assert!(execute(&storage, &reporter).await.error.is_some());
        assert_eq!(std::fs::read(target.join("mine")).unwrap(), b"unrelated");
        assert_eq!(
            storage
                .db
                .recording_storage_id("bilibili", "123", "live")
                .await
                .unwrap()
                .as_deref(),
            Some("a")
        );
    }

    #[tokio::test]
    async fn db_switch_failure_preserves_source_and_journal_retries_after_restart() {
        let (dir, storage, pool, reporter) = fixture().await;
        add(&storage, &pool, dir.path(), "first").await;
        add(&storage, &pool, dir.path(), "second").await;
        sqlx::raw_sql("CREATE TRIGGER reject_switch BEFORE UPDATE OF storage_id ON recording_storage_bindings \
            WHEN OLD.live_id = 'second' BEGIN SELECT RAISE(ABORT, 'injected switch failure'); END;")
            .execute(&pool).await.unwrap();
        let result = execute(&storage, &reporter).await;
        assert_eq!(
            (result.transferred_recordings, result.cleaned_recordings),
            (1, 1)
        );
        assert_eq!(
            result.failed_recording.as_deref(),
            Some("bilibili/123/second")
        );
        assert!(dir.path().join("a/bilibili/123/second/1.ts").exists());
        assert_eq!(
            storage
                .db
                .recording_storage_id("bilibili", "123", "second")
                .await
                .unwrap()
                .as_deref(),
            Some("a")
        );
        assert!(storage
            .try_lock_recording("bilibili", "123", "second")
            .is_err());
        let restarted = ArchiveStorage::new(storage.db.clone(), storage.config.clone());
        restarted.initialize().await.unwrap();
        assert!(restarted
            .try_lock_recording("bilibili", "123", "second")
            .is_err());
        // The copy is already published, but its binding commit failed. The
        // conservative DB estimate must not make a zero-byte retry unreachable.
        *restarted.transfer_available_bytes.lock().unwrap() = Some(0);
        let preview = restarted.preview_storage_transfer("a", "b").await.unwrap();
        assert!(preview.can_transfer, "{preview:?}");
        assert_eq!(
            (
                preview.recording_count,
                preview.required_bytes,
                preview.available_bytes
            ),
            (1, 7, Some(0))
        );
        let mut config = restarted.configuration().await.unwrap();
        config.backends.remove("b");
        assert!(restarted.replace_configuration(config).await.is_err());
        sqlx::raw_sql("DROP TRIGGER reject_switch")
            .execute(&pool)
            .await
            .unwrap();
        let retry = execute(&restarted, &reporter).await;
        assert!(retry.error.is_none(), "{retry:?}");
        assert_eq!(
            (retry.transferred_recordings, retry.cleaned_recordings),
            (1, 1)
        );
    }

    #[tokio::test]
    async fn committed_cleanup_journal_is_retried_without_switching_again() {
        let (dir, storage, pool, reporter) = fixture().await;
        add(&storage, &pool, dir.path(), "live").await;
        sqlx::raw_sql(
            "CREATE TRIGGER reject_finish BEFORE DELETE ON recording_storage_transfers \
            BEGIN SELECT RAISE(ABORT, 'injected cleanup completion failure'); END;",
        )
        .execute(&pool)
        .await
        .unwrap();
        let result = execute(&storage, &reporter).await;
        assert_eq!(
            (result.transferred_recordings, result.cleaned_recordings),
            (1, 0)
        );
        assert!(result.error.is_some());
        assert_eq!(
            storage
                .recording("bilibili", "123", "live")
                .await
                .unwrap()
                .read("1.ts")
                .await
                .unwrap(),
            b"actual bytes"
        );
        let preview = storage.preview_storage_transfer("a", "b").await.unwrap();
        assert_eq!(
            (
                preview.recording_count,
                preview.cleanup_count,
                preview.required_bytes
            ),
            (0, 1, 0)
        );
        sqlx::raw_sql("DROP TRIGGER reject_finish")
            .execute(&pool)
            .await
            .unwrap();
        let result = execute(&storage, &reporter).await;
        assert!(result.error.is_none(), "{result:?}");
        assert_eq!(
            (result.transferred_recordings, result.cleaned_recordings),
            (0, 1)
        );
    }

    #[tokio::test]
    async fn terminal_journal_retries_proof_cleanup_and_allows_round_trips() {
        let (dir, storage, pool, reporter) = fixture().await;
        add(&storage, &pool, dir.path(), "live").await;
        let plan = storage
            .plan_transfer("a", "b", storage.reserve_transfer().unwrap())
            .await
            .unwrap();
        let (journal, manifest, _) = &plan.entries[0];
        let key = journal.key();
        storage.db.begin_storage_transfer(journal).await.unwrap();
        manifest
            .publish(&plan.transfer, &key, &journal.token)
            .unwrap();
        storage.db.commit_storage_transfer(journal).await.unwrap();
        manifest
            .cleanup(&plan.transfer, &key, &journal.token)
            .unwrap();
        storage
            .db
            .finalize_storage_transfer(&journal.token)
            .await
            .unwrap();
        // Simulate restart before proof reclamation. The live journal prevents
        // another pair from taking ownership of this archive until finish.
        drop(plan);
        let restarted = ArchiveStorage::new(storage.db.clone(), storage.config.clone());
        restarted.initialize().await.unwrap();
        assert!(
            !restarted
                .preview_storage_transfer("b", "a")
                .await
                .unwrap()
                .can_transfer
        );
        let result = execute(&restarted, &reporter).await;
        assert!(result.error.is_none(), "{result:?}");
        assert_eq!(
            (result.transferred_recordings, result.cleaned_recordings),
            (0, 1)
        );
        let reverse = restarted
            .execute_storage_transfer("b", "a", restarted.reserve_transfer().unwrap(), &reporter)
            .await;
        assert!(reverse.error.is_none(), "{reverse:?}");
        let forward = execute(&restarted, &reporter).await;
        assert!(forward.error.is_none(), "{forward:?}");
        assert_eq!(forward.transferred_recordings, 1);
    }

    #[tokio::test]
    async fn active_mutations_and_concurrent_transfer_block_but_readers_do_not() {
        let (dir, storage, pool, reporter) = fixture().await;
        add(&storage, &pool, dir.path(), "live").await;
        let storage = Arc::new(storage);
        let writer = storage
            .recording_writer()
            .resolve_recording("bilibili", "123", "live")
            .await
            .unwrap();
        assert!(
            !storage
                .preview_storage_transfer("a", "b")
                .await
                .unwrap()
                .can_transfer
        );
        let result = execute(&storage, &reporter).await;
        assert!(result.error.is_some(), "{result:?}");
        assert!(storage
            .db
            .storage_transfer_journals()
            .await
            .unwrap()
            .is_empty());
        drop(writer);
        let mutation = storage
            .try_lock_recording("bilibili", "123", "live")
            .unwrap();
        assert!(
            !storage
                .preview_storage_transfer("a", "b")
                .await
                .unwrap()
                .can_transfer
        );
        let result = execute(&storage, &reporter).await;
        assert!(result.error.is_some(), "{result:?}");
        assert!(storage
            .db
            .storage_transfer_journals()
            .await
            .unwrap()
            .is_empty());
        // Read leases do not conflict with recording writer/mutation locks.
        let reader = storage.recording("bilibili", "123", "live").await.unwrap();
        drop(mutation);
        assert!(
            storage
                .preview_storage_transfer("a", "b")
                .await
                .unwrap()
                .can_transfer
        );
        drop(reader);
        assert!(
            storage
                .preview_storage_transfer("a", "b")
                .await
                .unwrap()
                .can_transfer
        );
        let _reservation = storage.reserve_transfer().unwrap();
        assert!(storage.reserve_transfer().is_err());
        assert!(
            !storage
                .preview_storage_transfer("a", "b")
                .await
                .unwrap()
                .can_transfer
        );
    }

    #[tokio::test]
    async fn missing_same_and_overlapping_roots_are_rejected() {
        let (dir, storage, _pool, _reporter) = fixture().await;
        assert!(storage.preview_storage_transfer("a", "a").await.is_err());
        assert!(storage
            .preview_storage_transfer("a", "missing")
            .await
            .is_err());
        std::fs::remove_dir(dir.path().join("b")).unwrap();
        assert!(storage.preview_storage_transfer("a", "b").await.is_err());
        let mut config = storage.configuration().await.unwrap();
        config.backends.insert(
            "child".into(),
            RecordingBackendConfig {
                kind: "local".into(),
                s3: None,
                webdav: None,
                root: dir.path().join("a/child").to_str().unwrap().into(),
            },
        );
        std::fs::create_dir(dir.path().join("a/child")).unwrap();
        storage.replace_configuration(config).await.unwrap();
        assert!(storage
            .preview_storage_transfer("a", "child")
            .await
            .is_err());
        assert!(storage
            .preview_storage_transfer("child", "a")
            .await
            .is_err());
    }
}
