//! Single-object publication: status and retry receipts change together.
use super::*;

impl CatalogWriter {
    /// Durable deletion markers are also the cleanup queue after a restart.
    pub async fn pending_deletions(&self) -> io::Result<Vec<CatalogRecord>> {
        let guard = self.inner.state.lock().await;
        let state = guard.as_ref().ok_or_else(|| invalid("catalog is closed"))?;
        state.check()?;
        Ok(state
            .documents
            .values()
            .filter(|d| d.status == RecordStatus::Deleted && !d.assets_removed)
            .map(|d| d.entry.record.clone())
            .collect())
    }

    pub async fn finish_deletion(&self, id: Uuid) -> io::Result<()> {
        let inner = self.inner.clone();
        tokio::spawn(async move {
            let mut guard = inner.state.lock().await;
            let state = guard.as_mut().ok_or_else(|| invalid("catalog is closed"))?;
            state.check()?;
            let mut doc = state
                .documents
                .get(&id)
                .cloned()
                .ok_or_else(|| invalid("deleted record missing"))?;
            if doc.status != RecordStatus::Deleted {
                return Err(conflict("record is not deleted"));
            }
            if !doc.assets_removed {
                doc.assets_removed = true;
                doc.entry.revision = state.next_revision()?;
                state.persist(doc).await?;
            }
            Ok(())
        })
        .await
        .map_err(io::Error::other)?
    }

    pub async fn find_recording(
        &self,
        platform: &str,
        room: &str,
        live: &str,
    ) -> io::Result<Option<CatalogEntry>> {
        let guard = self.inner.state.lock().await;
        let state = guard.as_ref().ok_or_else(|| invalid("catalog is closed"))?;
        state.check()?;
        let key = serde_json::to_string(&(platform, room, live)).expect("strings serialize");
        for doc in state
            .documents
            .values()
            .filter(|d| d.entry.record.identity_key() == key)
        {
            match doc.status {
                RecordStatus::Active => return Ok(Some(doc.entry.clone())),
                RecordStatus::Deleted => {
                    return Err(io::Error::new(
                        io::ErrorKind::NotFound,
                        "recording was deleted",
                    ))
                }
                _ => {}
            }
        }
        Ok(None)
    }

    pub async fn delete_recording(&self, record: CatalogRecord, revision: u64) -> io::Result<()> {
        let inner = self.inner.clone();
        tokio::spawn(async move {
            let mut guard = inner.state.lock().await;
            let state = guard.as_mut().ok_or_else(|| invalid("catalog is closed"))?;
            state.check()?;
            let mut doc = state
                .documents
                .get(&record.recording_id)
                .cloned()
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "recording is missing"))?;
            if doc.entry.record != record {
                return Err(conflict("recording snapshot changed"));
            }
            if doc.status == RecordStatus::Deleted {
                return Ok(());
            }
            if doc.status != RecordStatus::Active || doc.entry.revision != revision {
                return Err(conflict("record revision changed"));
            }
            doc.status = RecordStatus::Deleted;
            doc.entry.revision = state.next_revision()?;
            state.persist(doc).await
        })
        .await
        .map_err(io::Error::other)?
    }

    pub async fn receive_transfer(
        &self,
        token: Uuid,
        record: CatalogRecord,
    ) -> io::Result<CatalogEntry> {
        if token.is_nil() {
            return Err(invalid("invalid transfer token"));
        }
        let inner = self.inner.clone();
        tokio::spawn(async move {
            let mut guard = inner.state.lock().await;
            let state = guard.as_mut().ok_or_else(|| invalid("catalog is closed"))?;
            state.check()?;
            let digest = migration::record_digest(&record)?;
            if let Some(doc) = state
                .documents
                .values()
                .find(|d| d.received.contains_key(&token))
            {
                if doc.status == RecordStatus::Active
                    && doc.received[&token] == digest
                    && doc.entry.record == record
                {
                    return Ok(doc.entry.clone());
                }
                return Err(conflict("received recording was removed or changed"));
            }
            let mut doc = state.prepare_put(record, None)?;
            doc.received.insert(token, digest);
            let entry = doc.entry.clone();
            state.persist(doc).await?;
            Ok(entry)
        })
        .await
        .map_err(io::Error::other)?
    }

    pub async fn retire_transfer(&self, token: Uuid, id: Uuid, revision: u64) -> io::Result<()> {
        if token.is_nil() {
            return Err(invalid("invalid transfer token"));
        }
        self.retire(id, revision, RecordStatus::Transferred, Some(token))
            .await
    }

    pub(super) async fn retire(
        &self,
        id: Uuid,
        revision: u64,
        status: RecordStatus,
        token: Option<Uuid>,
    ) -> io::Result<()> {
        let inner = self.inner.clone();
        tokio::spawn(async move {
            let mut guard = inner.state.lock().await;
            let state = guard.as_mut().ok_or_else(|| invalid("catalog is closed"))?;
            state.check()?;
            if let Some(token) = token {
                if let Some(doc) = state
                    .documents
                    .values()
                    .find(|d| d.retired.contains_key(&token))
                {
                    return if doc.entry.record.recording_id == id && doc.retired[&token] == revision
                    {
                        Ok(())
                    } else {
                        Err(conflict("transfer receipt changed"))
                    };
                }
            }
            let mut doc = state
                .documents
                .get(&id)
                .cloned()
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "recording missing"))?;
            if doc.status != RecordStatus::Active || doc.entry.revision != revision {
                return Err(conflict("record revision changed"));
            }
            doc.status = status;
            if let Some(token) = token {
                doc.retired.insert(token, revision);
            }
            doc.entry.revision = state.next_revision()?;
            state.persist(doc).await
        })
        .await
        .map_err(io::Error::other)?
    }
}
