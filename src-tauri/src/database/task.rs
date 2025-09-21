use super::Database;
use super::DatabaseError;

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct TaskRow {
    pub id: String,
    #[sqlx(rename = "type")]
    pub task_type: String,
    pub status: String,
    pub message: String,
    pub metadata: String,
    pub created_at: String,
}

impl Database {
    pub async fn generate_task(
        &self,
        task_type: &str,
        message: &str,
        metadata: &str,
    ) -> Result<TaskRow, DatabaseError> {
        let task_id = uuid::Uuid::new_v4().to_string();
        let task = TaskRow {
            id: task_id,
            task_type: task_type.to_string(),
            status: "pending".to_string(),
            message: message.to_string(),
            metadata: metadata.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };

        self.add_task(&task).await?;

        Ok(task)
    }

    pub async fn add_task(&self, task: &TaskRow) -> Result<(), DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        let _ = sqlx::query(
            "INSERT INTO tasks (id, type, status, message, metadata, created_at) VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(&task.id)
        .bind(&task.task_type)
        .bind(&task.status)
        .bind(&task.message)
        .bind(&task.metadata)
        .bind(&task.created_at)
        .execute(&lock)
        .await?;
        Ok(())
    }

    pub async fn get_tasks(&self) -> Result<Vec<TaskRow>, DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        let tasks = sqlx::query_as::<_, TaskRow>("SELECT * FROM tasks")
            .fetch_all(&lock)
            .await?;
        Ok(tasks)
    }

    pub async fn update_task(
        &self,
        id: &str,
        status: &str,
        message: &str,
        metadata: Option<&str>,
    ) -> Result<(), DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        if let Some(metadata) = metadata {
            let _ = sqlx::query(
                "UPDATE tasks SET status = $1, message = $2, metadata = $3 WHERE id = $4",
            )
            .bind(status)
            .bind(message)
            .bind(metadata)
            .bind(id)
            .execute(&lock)
            .await?;
        } else {
            let _ = sqlx::query("UPDATE tasks SET status = $1, message = $2 WHERE id = $3")
                .bind(status)
                .bind(message)
                .bind(id)
                .execute(&lock)
                .await?;
        }

        Ok(())
    }

    pub async fn delete_task(&self, id: &str) -> Result<(), DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        let _ = sqlx::query("DELETE FROM tasks WHERE id = $1")
            .bind(id)
            .execute(&lock)
            .await?;
        Ok(())
    }

    pub async fn finish_pending_tasks(&self) -> Result<(), DatabaseError> {
        let lock = self.db.read().await.clone().unwrap();
        let _ = sqlx::query("UPDATE tasks SET status = 'failed' WHERE status = 'pending'")
            .execute(&lock)
            .await?;
        Ok(())
    }
}
