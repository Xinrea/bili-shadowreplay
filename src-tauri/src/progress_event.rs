use serde::Serialize;
use tauri::{AppHandle, Emitter};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressUpdate<'a> {
    pub id: &'a str,
    pub content: &'a str,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressFinished<'a> {
    pub id: &'a str,
}

pub fn emit_progress_update(app_handle: &AppHandle, event_id: &str, content: &str) {
    app_handle
        .emit(
            "progress-update",
            ProgressUpdate {
                id: event_id,
                content,
            },
        )
        .unwrap();
}

pub fn emit_progress_finished(app_handle: &AppHandle, event_id: &str) {
    app_handle
        .emit("progress-finished", ProgressFinished { id: event_id })
        .unwrap();
}
