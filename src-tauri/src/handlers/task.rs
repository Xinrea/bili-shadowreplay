#[cfg(feature = "gui")]
use tauri::State as TauriState;

use crate::state::State;
use crate::{database::task::TaskRow, state_type};

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_tasks(state: state_type!()) -> Result<Vec<TaskRow>, String> {
    Ok(state.db.get_tasks().await?)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn delete_task(state: state_type!(), id: &str) -> Result<(), String> {
    Ok(state.db.delete_task(id).await?)
}
