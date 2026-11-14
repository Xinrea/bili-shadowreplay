//! Resumable import of the application's frozen SQLite snapshot.
//! The per-record receipt is written with the record; progress follows it.
use super::*;
use sha2::{Digest, Sha256};

pub(super) const IMPORT_PATH: &str = ".bsr/import.json";
pub(super) const MAX_IMPORT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportDigest([u8; 32]);
impl ImportDigest {
    pub fn append(&mut self, record: &CatalogRecord) -> io::Result<()> {
        let bytes = canonical_record(record)?;
        let mut hash = Sha256::new();
        hash.update(self.0);
        hash.update((bytes.len() as u64).to_be_bytes());
        hash.update(bytes);
        self.0 = hash.finalize().into();
        Ok(())
    }
}
fn canonical_record(record: &CatalogRecord) -> io::Result<Vec<u8>> {
    record.validate()?;
    let mut value = serde_json::to_value(record).map_err(io::Error::other)?;
    value.sort_all_objects();
    let bytes = serde_json::to_vec(&value).map_err(io::Error::other)?;
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(invalid("legacy record exceeds size limit"));
    }
    Ok(bytes)
}
pub(super) fn record_digest(record: &CatalogRecord) -> io::Result<[u8; 32]> {
    Ok(Sha256::digest(canonical_record(record)?).into())
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportPlan {
    pub import_id: Uuid,
    pub storage_uuid: Uuid,
    pub count: u64,
    pub digest: ImportDigest,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportProgress {
    pub plan: ImportPlan,
    pub next_index: u64,
    pub complete: bool,
    pub(super) digest: ImportDigest,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ImportReceipt {
    pub import_id: Uuid,
    pub index: u64,
    pub digest: [u8; 32],
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ImportState {
    pub progress: ImportProgress,
    pub receipts: Vec<[u8; 32]>,
}
impl ImportState {
    fn validate(&self) -> io::Result<()> {
        let p = &self.progress;
        if p.plan.import_id.is_nil()
            || p.plan.storage_uuid.is_nil()
            || p.next_index != self.receipts.len() as u64
            || p.next_index > p.plan.count
            || (p.complete && (p.next_index != p.plan.count || p.digest != p.plan.digest))
        {
            return Err(invalid("invalid import progress"));
        }
        Ok(())
    }
}
impl CatalogLocation {
    pub(super) async fn load_import(&self) -> io::Result<Option<ImportState>> {
        match self.store.read_bounded(IMPORT_PATH, MAX_IMPORT_BYTES).await {
            Ok(bytes) => {
                let state: ImportState = serde_json::from_slice(&bytes)
                    .map_err(|_| invalid("invalid import journal"))?;
                state.validate()?;
                Ok(Some(state))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
}
impl WriterState {
    pub(super) fn check_import_ready(&self) -> io::Result<()> {
        if self.import.as_ref().is_some_and(|s| !s.progress.complete) {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "finish legacy import before rebuilding the index",
            ));
        }
        Ok(())
    }
    async fn persist_import(&mut self, import: ImportState) -> io::Result<()> {
        import.validate()?;
        if let Err(e) = self
            .location
            .write_json(IMPORT_PATH, &import, MAX_IMPORT_BYTES)
            .await
        {
            self.failed = true;
            return Err(e);
        }
        self.import = Some(import);
        Ok(())
    }
    fn import_state(&self, plan: &ImportPlan) -> io::Result<ImportState> {
        let state = self
            .import
            .clone()
            .ok_or_else(|| invalid("legacy import has not started"))?;
        if state.progress.plan != *plan {
            return Err(conflict("legacy import snapshot changed"));
        }
        Ok(state)
    }
}
impl CatalogWriter {
    pub async fn begin_import(&self, plan: ImportPlan) -> io::Result<ImportProgress> {
        if plan.import_id.is_nil()
            || plan.storage_uuid != self.identity.storage_uuid
            || (plan.count == 0 && plan.digest != ImportDigest::default())
        {
            return Err(invalid("invalid import plan"));
        }
        let inner = self.inner.clone();
        tokio::spawn(async move {
            let mut guard = inner.state.lock().await;
            let state = guard.as_mut().ok_or_else(|| invalid("catalog is closed"))?;
            state.check()?;
            if state.import.is_some() {
                return Ok(state.import_state(&plan)?.progress);
            }
            let progress = ImportProgress {
                plan,
                next_index: 0,
                complete: false,
                digest: ImportDigest::default(),
            };
            state
                .persist_import(ImportState {
                    progress: progress.clone(),
                    receipts: vec![],
                })
                .await?;
            Ok(progress)
        })
        .await
        .map_err(io::Error::other)?
    }
    pub async fn import_record(
        &self,
        plan: ImportPlan,
        index: u64,
        record: CatalogRecord,
    ) -> io::Result<()> {
        if plan.storage_uuid != self.identity.storage_uuid || index >= plan.count {
            return Err(invalid("invalid import index or storage"));
        }
        let digest = record_digest(&record)?;
        let inner = self.inner.clone();
        tokio::spawn(async move {
            let mut guard = inner.state.lock().await;
            let state = guard.as_mut().ok_or_else(|| invalid("catalog is closed"))?;
            state.check()?;
            let mut import = state.import_state(&plan)?;
            if index < import.progress.next_index {
                return if import.receipts[index as usize] == digest {
                    Ok(())
                } else {
                    Err(conflict("legacy import record changed"))
                };
            }
            if index != import.progress.next_index || import.progress.complete {
                return Err(conflict("legacy import must proceed in order"));
            }
            import.progress.digest.append(&record)?;
            import.progress.next_index += 1;
            if import.progress.next_index == plan.count && import.progress.digest != plan.digest {
                return Err(conflict("legacy import digest mismatch"));
            }
            let receipt = ImportReceipt {
                import_id: plan.import_id,
                index,
                digest,
            };
            let already_written = state
                .documents
                .get(&record.recording_id)
                .is_some_and(|doc| doc.import.as_ref() == Some(&receipt));
            if !already_written {
                let mut doc = state.prepare_put(record, None)?;
                doc.import = Some(receipt);
                state.persist(doc).await?;
            }
            import.receipts.push(digest);
            state.persist_import(import).await
        })
        .await
        .map_err(io::Error::other)?
    }
    pub async fn finish_import(&self, plan: ImportPlan) -> io::Result<()> {
        if plan.storage_uuid != self.identity.storage_uuid {
            return Err(invalid("import storage mismatch"));
        }
        let inner = self.inner.clone();
        tokio::spawn(async move {
            let mut guard = inner.state.lock().await;
            let state = guard.as_mut().ok_or_else(|| invalid("catalog is closed"))?;
            state.check()?;
            let mut import = state.import_state(&plan)?;
            if import.progress.next_index != plan.count || import.progress.digest != plan.digest {
                return Err(conflict("legacy import is incomplete"));
            }
            if !import.progress.complete {
                import.progress.complete = true;
                state.persist_import(import).await?;
            }
            Ok(())
        })
        .await
        .map_err(io::Error::other)?
    }
}
