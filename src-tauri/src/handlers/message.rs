use crate::database::message::MessageRow;
use crate::state::State;
use crate::state_type;

#[cfg(feature = "gui")]
use tauri::State as TauriState;

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_messages(state: state_type!()) -> Result<Vec<MessageRow>, String> {
    Ok(state.db.get_messages().await?)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn read_message(state: state_type!(), id: i64) -> Result<(), String> {
    Ok(state.db.read_message(id).await?)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn delete_message(state: state_type!(), id: i64) -> Result<(), String> {
    Ok(state.db.delete_message(id).await?)
}
