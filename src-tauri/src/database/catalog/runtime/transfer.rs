use super::*;
use crate::database::storage_transfer::TransferJournal;

impl CatalogRuntime {
    pub async fn preflight_transfer(&self, journal: &TransferJournal) -> Result<(), DatabaseError> {
        self.writable(&journal.source_id).await?;
        self.writable(&journal.target_id).await?;
        let active: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM catalog_active_recordings WHERE live_id=?)",
        )
        .bind(&journal.live_id)
        .fetch_one(&self.pool)
        .await?;
        if active {
            return Err(error("录播结束事件尚未持久化，请稍后重试转移"));
        }
        let handles = self.handles.lock().await;
        let source = writer(&handles, &journal.source_id)?;
        let target = writer(&handles, &journal.target_id)?;
        source
            .writer
            .as_ref()
            .expect("writer")
            .find_recording(&journal.platform, &journal.room_id, &journal.live_id)
            .await
            .map_err(error)?
            .ok_or(DatabaseError::NotFound)?;
        if target
            .writer
            .as_ref()
            .expect("writer")
            .find_recording(&journal.platform, &journal.room_id, &journal.live_id)
            .await
            .map_err(error)?
            .is_some()
        {
            return Err(error("目标目录已包含此录播，拒绝覆盖"));
        }
        Ok(())
    }

    /// File publication has already been verified. Persist the metadata intent
    /// before touching either catalog, then receive -> retire -> commit binding.
    /// The existing media journal prevents reads/edits and survives each gap.
    pub async fn commit_transfer(
        self: &Arc<Self>,
        journal: TransferJournal,
    ) -> Result<(), DatabaseError> {
        let this = self.clone();
        tokio::spawn(async move {
            let handles = this.handles.lock().await;
            let source = writer(&handles, &journal.source_id)?;
            let target = writer(&handles, &journal.target_id)?;
            let token = Uuid::parse_str(&journal.token).map_err(error)?;
            let saved: Option<(String,String,String)> = sqlx::query_as(
                "SELECT source_uuid,target_uuid,entry FROM catalog_transfer_intents WHERE token=?",
            ).bind(&journal.token).fetch_optional(&this.pool).await?;
            let entry: CatalogEntry = if let Some((from, to, bytes)) = saved {
                if from != source.uuid.to_string() || to != target.uuid.to_string() {
                    return Err(error("Transfer catalog UUID changed"));
                }
                from_json(&bytes)?
            } else {
                let entry = source.writer.as_ref().expect("writer")
                    .find_recording(&journal.platform, &journal.room_id, &journal.live_id).await.map_err(error)?
                    .ok_or(DatabaseError::NotFound)?;
                sqlx::query("INSERT INTO catalog_transfer_intents(token,source_uuid,target_uuid,entry) VALUES (?,?,?,?)")
                    .bind(&journal.token).bind(source.uuid.to_string()).bind(target.uuid.to_string())
                    .bind(json(&entry)?).execute(&this.pool).await?;
                entry
            };
            let received = target.writer.as_ref().expect("writer")
                .receive_transfer(token, entry.record.clone()).await.map_err(error)?;
            source.writer.as_ref().expect("writer")
                .retire_transfer(token, entry.record.recording_id, entry.revision).await.map_err(error)?;
            let mut tx = this.pool.begin().await?;
            crate::database::storage_transfer::commit_binding(&mut tx, &journal).await?;
            cache_entry(&mut tx, &journal.target_id, target.uuid, &received).await?;
            tx.commit().await?;
            this.mutation_access.write().unwrap_or_else(|p| p.into_inner()).insert(
                format!("{}/{}/{}",journal.platform,journal.room_id,journal.live_id),
                (journal.target_id.clone(), true),
            );
            this.events.publish(crate::events::AppEvent::DataChanged(DataChange::Records));
            Ok(())
        }).await.map_err(error)?
    }
}
