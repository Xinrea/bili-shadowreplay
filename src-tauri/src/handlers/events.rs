use crate::{events::RoomTopic, state::State};

#[tauri::command]
pub async fn set_event_rooms(
    state: tauri::State<'_, State>,
    window: tauri::WebviewWindow,
    rooms: Vec<RoomTopic>,
) -> Result<(), String> {
    state.events.set_window_rooms(&window, rooms)
}
