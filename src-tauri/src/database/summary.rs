use chrono::Utc;

use super::{Database, DatabaseError};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct RecordSummaryRow {
    pub id: i64,
    pub platform: String,
    pub room_id: String,
    pub live_id: String,
    pub status: String,
    pub stage: String,
    pub subtitle_srt: Option<String>,
    pub subtitle_text: Option<String>,
    pub summary_markdown: Option<String>,
    pub highlights_json: Option<String>,
    pub model_provider: Option<String>,
    pub model_name: Option<String>,
    pub prompt_version: i64,
    pub source_duration: Option<f64>,
    pub error_message: Option<String>,
    pub task_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct RecordSummaryStatusRow {
    pub platform: String,
    pub room_id: String,
    pub live_id: String,
    pub status: String,
    pub stage: String,
}

impl Database {
    pub async fn get_record_summary_statuses(
        &self,
    ) -> Result<Vec<RecordSummaryStatusRow>, DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        Ok(sqlx::query_as::<_, RecordSummaryStatusRow>(
            "SELECT platform, room_id, live_id, status, stage FROM record_summaries",
        )
        .fetch_all(&lock)
        .await?)
    }

    pub async fn finish_pending_record_summaries(&self) -> Result<(), DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        sqlx::query(
            "UPDATE record_summaries SET status = 'failed', error_message = '任务因应用退出而中断', updated_at = $1 WHERE status = 'processing'",
        )
        .bind(Utc::now().to_rfc3339())
        .execute(&lock)
        .await?;
        Ok(())
    }

    pub async fn get_record_summary(
        &self,
        platform: &str,
        room_id: &str,
        live_id: &str,
    ) -> Result<Option<RecordSummaryRow>, DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        Ok(sqlx::query_as::<_, RecordSummaryRow>(
            "SELECT * FROM record_summaries WHERE platform = $1 AND room_id = $2 AND live_id = $3",
        )
        .bind(platform)
        .bind(room_id)
        .bind(live_id)
        .fetch_optional(&lock)
        .await?)
    }

    pub async fn start_record_summary(
        &self,
        platform: &str,
        room_id: &str,
        live_id: &str,
        task_id: &str,
        force: bool,
    ) -> Result<RecordSummaryRow, DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        let now = Utc::now().to_rfc3339();
        if force {
            sqlx::query(
                r#"INSERT INTO record_summaries (
                    platform, room_id, live_id, status, stage, prompt_version,
                    task_id, created_at, updated_at
                ) VALUES ($1, $2, $3, 'processing', 'extracting_audio', 1, $4, $5, $5)
                ON CONFLICT(platform, room_id, live_id) DO UPDATE SET
                    status = 'processing', stage = 'extracting_audio',
                    subtitle_srt = NULL, subtitle_text = NULL,
                    summary_markdown = NULL, highlights_json = NULL,
                    model_provider = NULL, model_name = NULL,
                    source_duration = NULL, error_message = NULL,
                    task_id = excluded.task_id, updated_at = excluded.updated_at"#,
            )
            .bind(platform)
            .bind(room_id)
            .bind(live_id)
            .bind(task_id)
            .bind(&now)
            .execute(&lock)
            .await?;
        } else {
            sqlx::query(
                r#"INSERT INTO record_summaries (
                    platform, room_id, live_id, status, stage, prompt_version,
                    task_id, created_at, updated_at
                ) VALUES ($1, $2, $3, 'processing', 'extracting_audio', 1, $4, $5, $5)
                ON CONFLICT(platform, room_id, live_id) DO UPDATE SET
                    status = 'processing',
                    stage = CASE WHEN record_summaries.subtitle_text IS NULL
                        THEN 'extracting_audio' ELSE 'summarizing' END,
                    error_message = NULL, task_id = excluded.task_id,
                    updated_at = excluded.updated_at"#,
            )
            .bind(platform)
            .bind(room_id)
            .bind(live_id)
            .bind(task_id)
            .bind(&now)
            .execute(&lock)
            .await?;
        }
        drop(lock);
        self.get_record_summary(platform, room_id, live_id)
            .await?
            .ok_or(DatabaseError::NotFound)
    }

    pub async fn update_record_summary_stage(
        &self,
        platform: &str,
        room_id: &str,
        live_id: &str,
        stage: &str,
    ) -> Result<(), DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        sqlx::query(
            "UPDATE record_summaries SET status = 'processing', stage = $1, updated_at = $2 WHERE platform = $3 AND room_id = $4 AND live_id = $5",
        )
        .bind(stage)
        .bind(Utc::now().to_rfc3339())
        .bind(platform)
        .bind(room_id)
        .bind(live_id)
        .execute(&lock)
        .await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn save_record_summary_subtitle(
        &self,
        platform: &str,
        room_id: &str,
        live_id: &str,
        subtitle_srt: &str,
        subtitle_text: &str,
        source_duration: f64,
    ) -> Result<(), DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        sqlx::query(
            "UPDATE record_summaries SET subtitle_srt = $1, subtitle_text = $2, source_duration = $3, stage = 'summarizing', updated_at = $4 WHERE platform = $5 AND room_id = $6 AND live_id = $7",
        )
        .bind(subtitle_srt)
        .bind(subtitle_text)
        .bind(source_duration)
        .bind(Utc::now().to_rfc3339())
        .bind(platform)
        .bind(room_id)
        .bind(live_id)
        .execute(&lock)
        .await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn complete_record_summary(
        &self,
        platform: &str,
        room_id: &str,
        live_id: &str,
        summary_markdown: &str,
        highlights_json: &str,
        model_provider: &str,
        model_name: &str,
    ) -> Result<(), DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        sqlx::query(
            "UPDATE record_summaries SET status = 'success', stage = 'completed', summary_markdown = $1, highlights_json = $2, model_provider = $3, model_name = $4, error_message = NULL, updated_at = $5 WHERE platform = $6 AND room_id = $7 AND live_id = $8",
        )
        .bind(summary_markdown)
        .bind(highlights_json)
        .bind(model_provider)
        .bind(model_name)
        .bind(Utc::now().to_rfc3339())
        .bind(platform)
        .bind(room_id)
        .bind(live_id)
        .execute(&lock)
        .await?;
        Ok(())
    }

    pub async fn fail_record_summary(
        &self,
        platform: &str,
        room_id: &str,
        live_id: &str,
        error: &str,
    ) -> Result<(), DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        sqlx::query(
            "UPDATE record_summaries SET status = 'failed', error_message = $1, updated_at = $2 WHERE platform = $3 AND room_id = $4 AND live_id = $5",
        )
        .bind(error)
        .bind(Utc::now().to_rfc3339())
        .bind(platform)
        .bind(room_id)
        .bind(live_id)
        .execute(&lock)
        .await?;
        Ok(())
    }

    pub async fn delete_record_summary(
        &self,
        platform: &str,
        room_id: &str,
        live_id: &str,
    ) -> Result<(), DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        sqlx::query(
            "DELETE FROM record_summaries WHERE platform = $1 AND room_id = $2 AND live_id = $3",
        )
        .bind(platform)
        .bind(room_id)
        .bind(live_id)
        .execute(&lock)
        .await?;
        Ok(())
    }
}
