use crate::database::message::MessageRow;
use crate::state::State;
use tauri::State as TauriState;

#[tauri::command]
pub async fn get_messages(state: TauriState<'_, State>) -> Result<Vec<MessageRow>, String> {
    Ok(state.db.get_messages().await?)
}

#[tauri::command]
pub async fn read_message(state: TauriState<'_, State>, id: i64) -> Result<(), String> {
    Ok(state.db.read_message(id).await?)
}

#[tauri::command]
pub async fn delete_message(state: TauriState<'_, State>, id: i64) -> Result<(), String> {
    Ok(state.db.delete_message(id).await?)
} 