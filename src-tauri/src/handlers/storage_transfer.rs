use crate::archive_storage::StorageTransferPreview;
use crate::database::task::TaskRow;
use crate::progress::progress_reporter::{ProgressReporter, ProgressReporterTrait};
use crate::state::State;
use crate::state_type;
use crate::task::{Task, TaskPriority};

#[cfg(feature = "gui")]
use tauri::State as TauriState;

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn preview_storage_transfer(
    state: state_type!(),
    source_backend_id: String,
    target_backend_id: String,
) -> Result<StorageTransferPreview, String> {
    state
        .recorder_manager
        .archive_storage()
        .preview_storage_transfer(&source_backend_id, &target_backend_id)
        .await
        .map_err(|error| error.to_string())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn start_storage_transfer(
    state: state_type!(),
    source_backend_id: String,
    target_backend_id: String,
) -> Result<TaskRow, String> {
    state.db.ensure_catalog_startup_complete()?;
    #[cfg(feature = "headless")]
    if state.readonly {
        return Err("只读模式不能转移录制存储".into());
    }
    let storage = state.recorder_manager.archive_storage().clone();
    let reservation = storage
        .reserve_transfer()
        .map_err(|error| error.to_string())?;
    let metadata = serde_json::json!({
        "source_backend_id": source_backend_id, "target_backend_id": target_backend_id,
    });
    let task = state
        .db
        .generate_task(
            "storage_transfer",
            "存储转移已排队，执行前将重新扫描",
            &metadata.to_string(),
        )
        .await?;
    let reporter = ProgressReporter::new(state.db.clone(), &state.events, &task.id).await?;
    let db = state.db.clone();
    let id = task.id.clone();
    let enqueue = state.task_manager.add_task(Task::new(
        id.clone(), TaskPriority::Normal, async move {
            let result = storage.execute_storage_transfer(
                &source_backend_id, &target_backend_id, reservation, &reporter).await;
            let success = result.error.is_none();
            let message = match &result.error {
                None => format!("存储转移完成：已转移 {} 条，已清理 {} 条",
                    result.transferred_recordings, result.cleaned_recordings),
                Some(error) => format!("存储转移停止：已转移 {} 条，已清理 {} 条；{}。已提交的绑定不会回退，请使用相同来源和目标重试",
                    result.transferred_recordings, result.cleaned_recordings, error),
            };
            let mut metadata = metadata;
            metadata["result"] = serde_json::to_value(&result).map_err(|error| error.to_string())?;
            db.update_task(&id, if success { "success" } else { "failed" }, &message,
                Some(&metadata.to_string())).await.map_err(|error| error.to_string())?;
            reporter.finish(success, &message).await;
            if success { Ok(()) } else { Err(message) }
        },
    )).await;
    if let Err(error) = enqueue {
        state
            .db
            .update_task(&task.id, "failed", &format!("加入队列失败：{error}"), None)
            .await?;
        return Err(error);
    }
    Ok(task)
}
