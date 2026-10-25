//! Long operations called over MCP run in the background. A call returns the
//! task that tracks the operation; clients poll `get_task` and may cancel it.
//! What the operation produced is added to the task's metadata as `result`.

use std::{future::Future, time::Duration};

use chrono::Utc;
use serde_json::{json, Value};

use crate::{database::task::TaskRow, state::State};

/// How long a call waits for its operation, so that failures found before
/// the work starts, such as a missing material, are reported by the call.
const STARTUP_WAIT: Duration = Duration::from_secs(1);

pub(super) fn new_task_id() -> String {
    format!("mcp-{}", uuid::Uuid::new_v4())
}

/// The task row of an operation whose handler records none of its own.
pub(super) struct TaskRecord {
    pub task_type: &'static str,
    pub metadata: Value,
}

/// Run `job`, which works under the task `task_id`, in the background.
/// Handlers that record their task themselves get `record: None`; for the
/// others, the task is recorded here.
pub(super) async fn start(
    state: &State,
    task_id: String,
    task_type: &'static str,
    record: Option<TaskRecord>,
    job: impl Future<Output = Result<Value, String>> + Send + 'static,
) -> Result<Value, String> {
    let owned = record.is_some();
    if let Some(record) = record {
        state
            .db
            .add_task(&TaskRow {
                id: task_id.clone(),
                task_type: record.task_type.to_string(),
                status: "processing".to_string(),
                message: String::new(),
                metadata: record.metadata.to_string(),
                created_at: Utc::now().to_rfc3339(),
            })
            .await
            .map_err(|e| e.to_string())?;
    }
    let background = state.clone();
    let id = task_id.clone();
    let mut running = tokio::spawn(async move {
        let result = job.await;
        finish(&background, &id, task_type, owned, &result).await;
        result
    });
    match tokio::time::timeout(STARTUP_WAIT, &mut running).await {
        Ok(Ok(Ok(result))) => {
            Ok(json!({ "task_id": task_id, "status": "success", "result": result }))
        }
        Ok(Ok(Err(error))) => Err(error),
        Ok(Err(error)) => Err(format!("Task {task_id} stopped unexpectedly: {error}")),
        // Dropping the handle leaves the operation running.
        Err(_) => {
            let status = state
                .db
                .get_task(&task_id)
                .await
                .map_or_else(|_| "pending".to_string(), |task| task.status);
            Ok(json!({
                "task_id": task_id,
                "status": status,
                "message": "Running in the background; poll get_task with task_id until its status is success, failed or cancelled."
            }))
        }
    }
}

/// Record the outcome on the task: the result in its metadata, and for tasks
/// recorded here also the status. A failure before the handler recorded its
/// task is recorded too, so that `get_task` can report it.
async fn finish(
    state: &State,
    task_id: &str,
    task_type: &str,
    owned: bool,
    result: &Result<Value, String>,
) {
    let task = state.db.get_task(task_id).await.ok();
    let recorded = match (result, task) {
        (Ok(value), Some(task)) => {
            let mut metadata = serde_json::from_str::<Value>(&task.metadata)
                .ok()
                .filter(Value::is_object)
                .unwrap_or_else(|| json!({}));
            metadata["result"] = value.clone();
            let (status, message) = if owned {
                ("success", "完成")
            } else {
                (task.status.as_str(), task.message.as_str())
            };
            state
                .db
                .update_task(task_id, status, message, Some(&metadata.to_string()))
                .await
        }
        (Err(error), Some(_)) if owned => {
            state.db.update_task(task_id, "failed", error, None).await
        }
        (Err(_), Some(_)) => Ok(()),
        (Err(error), None) => {
            state
                .db
                .add_task(&TaskRow {
                    id: task_id.to_string(),
                    task_type: task_type.to_string(),
                    status: "failed".to_string(),
                    message: error.clone(),
                    metadata: "{}".to_string(),
                    created_at: Utc::now().to_rfc3339(),
                })
                .await
        }
        (Ok(_), None) => {
            log::warn!("MCP task {task_id} finished without a task record");
            Ok(())
        }
    };
    if let Err(error) = recorded {
        log::error!("Failed to record the result of MCP task {task_id}: {error}");
    }
}

/// A task as `get_task` reports it, with its metadata parsed.
pub(super) async fn task(state: &State, id: &str) -> Result<Value, String> {
    let task = state
        .db
        .get_task(id)
        .await
        .map_err(|_| format!("Task {id} not found"))?;
    let metadata = serde_json::from_str::<Value>(&task.metadata)
        .unwrap_or_else(|_| Value::String(task.metadata.clone()));
    Ok(json!({
        "task": {
            "id": task.id,
            "task_type": task.task_type,
            "status": task.status,
            "message": task.message,
            "metadata": metadata,
            "created_at": task.created_at,
        }
    }))
}

#[cfg(all(test, feature = "headless"))]
mod tests {
    use super::*;
    use crate::{
        config::Config,
        database::{credentials::CredentialCipher, Database},
        events::EventBus,
        mcp::McpServer,
        recorder_manager::RecorderManager,
        task::TaskManager,
        webhook::poster::{WebhookConfig, WebhookPoster},
    };
    use std::sync::Arc;
    use tokio::sync::{oneshot, RwLock};

    async fn fixture(dir: &std::path::Path) -> State {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::raw_sql(
            "CREATE TABLE tasks (
                id TEXT PRIMARY KEY, type TEXT, status TEXT, message TEXT,
                metadata TEXT, created_at TEXT
            );",
        )
        .execute(&pool)
        .await
        .unwrap();
        let events = EventBus::new();
        let db = Arc::new(Database::new(
            pool,
            CredentialCipher::new(&[0; 32]).unwrap(),
            events.clone(),
        ));
        let config: Config = serde_json::from_value(json!({
            "output": dir, "cache": dir,
            "live_start_notify": false, "live_end_notify": false,
            "clip_notify": false, "post_notify": false,
        }))
        .unwrap();
        let config = Arc::new(RwLock::new(config));
        let task_manager = Arc::new(TaskManager::new());
        let webhook_poster = WebhookPoster::new(WebhookConfig::default()).unwrap();
        let recorder_manager = Arc::new(RecorderManager::new(
            events.clone(),
            db.clone(),
            config.clone(),
            task_manager.clone(),
            dir.to_path_buf(),
            webhook_poster.clone(),
        ));
        State {
            mcp_server: Arc::new(McpServer::default()),
            db,
            config,
            webhook_poster,
            recorder_manager,
            task_manager,
            resource_dir: dir.to_path_buf(),
            events,
            readonly: false,
        }
    }

    /// A handler that records its own task, as the queued handlers do.
    async fn handler(
        state: State,
        id: String,
        done: oneshot::Receiver<()>,
    ) -> Result<Value, String> {
        state
            .db
            .add_task(&TaskRow {
                id: id.clone(),
                task_type: "clip_range".into(),
                status: "pending".into(),
                message: String::new(),
                metadata: json!({ "params": { "title": "t" } }).to_string(),
                created_at: Utc::now().to_rfc3339(),
            })
            .await
            .map_err(|e| e.to_string())?;
        let _ = done.await;
        state
            .db
            .update_task(&id, "success", "done", None)
            .await
            .map_err(|e| e.to_string())?;
        Ok(json!({ "video_id": 7 }))
    }

    async fn settled(state: &State, id: &str) -> Value {
        for _ in 0..100 {
            let task = task(state, id).await.unwrap();
            if task["task"]["metadata"].get("result").is_some()
                || task["task"]["status"] == "failed"
            {
                return task;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("task {id} did not settle");
    }

    #[tokio::test]
    async fn slow_jobs_return_their_task_and_record_their_result_on_it() {
        let dir = tempfile::tempdir().unwrap();
        let state = fixture(dir.path()).await;
        let (finish, done) = oneshot::channel();
        let started = start(
            &state,
            "slow".into(),
            "clip_range",
            None,
            handler(state.clone(), "slow".into(), done),
        )
        .await
        .unwrap();
        assert_eq!(started["task_id"], "slow");
        assert_eq!(started["status"], "pending");
        finish.send(()).unwrap();
        let task = settled(&state, "slow").await;
        assert_eq!(task["task"]["status"], "success");
        assert_eq!(task["task"]["message"], "done");
        assert_eq!(task["task"]["metadata"]["result"]["video_id"], 7);
        // The handler's own metadata stays.
        assert_eq!(task["task"]["metadata"]["params"]["title"], "t");
    }

    #[tokio::test]
    async fn quick_jobs_and_early_failures_are_reported_by_the_call() {
        let dir = tempfile::tempdir().unwrap();
        let state = fixture(dir.path()).await;
        let (finish, done) = oneshot::channel();
        finish.send(()).unwrap();
        let started = start(
            &state,
            "quick".into(),
            "clip_range",
            None,
            handler(state.clone(), "quick".into(), done),
        )
        .await
        .unwrap();
        assert_eq!(started["status"], "success");
        assert_eq!(started["result"]["video_id"], 7);

        let error = start(&state, "early".into(), "clip_range", None, async {
            Err::<Value, _>("素材不存在".to_string())
        })
        .await
        .unwrap_err();
        assert_eq!(error, "素材不存在");
        // Recorded although the handler failed before recording its task.
        let task = task(&state, "early").await.unwrap();
        assert_eq!(task["task"]["status"], "failed");
        assert_eq!(task["task"]["message"], "素材不存在");
    }

    #[tokio::test]
    async fn jobs_without_their_own_task_are_recorded_here() {
        let dir = tempfile::tempdir().unwrap();
        let state = fixture(dir.path()).await;
        let (finish, done) = oneshot::channel::<Result<(), String>>();
        let record = TaskRecord {
            task_type: "generate_archive_subtitle",
            metadata: json!({ "live_id": "1" }),
        };
        let started = start(
            &state,
            "own".into(),
            "generate_archive_subtitle",
            Some(record),
            async move {
                done.await.unwrap()?;
                Ok(json!({ "live_id": "1" }))
            },
        )
        .await
        .unwrap();
        assert_eq!(started["status"], "processing");
        finish.send(Err("模型不可用".into())).unwrap();
        let task = settled(&state, "own").await;
        assert_eq!(task["task"]["task_type"], "generate_archive_subtitle");
        assert_eq!(task["task"]["status"], "failed");
        assert_eq!(task["task"]["message"], "模型不可用");
        assert_eq!(task["task"]["metadata"]["live_id"], "1");
    }
}
