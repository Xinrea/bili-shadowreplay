use serde_json::{json, Value};
use socketioxide::{
    extract::{Data, SocketRef},
    layer::SocketIoLayer,
    SocketIo,
};
use tokio::sync::broadcast;

use crate::state::State;
use recorder::events::RecorderEvent;

pub async fn create_websocket_server(state: State) -> SocketIoLayer {
    let (layer, io) = SocketIo::new_layer();

    // Clone the state for the namespace handler
    let state_clone = state.clone();

    io.ns("/ws", move |socket: SocketRef| {
        let state = state_clone.clone();

        // Subscribe to progress events
        let mut rx = state.progress_manager.subscribe();

        // Spawn a task to handle progress events for this socket
        let socket_clone = socket.clone();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        let (event_type, message) = match event {
                            RecorderEvent::ProgressUpdate { id, content } => (
                                "progress-update",
                                json!({
                                        "id": id,
                                        "content": content
                                }),
                            ),
                            RecorderEvent::ProgressFinished {
                                id,
                                success,
                                message,
                            } => (
                                "progress-finished",
                                json!({
                                        "id": id,
                                        "success": success,
                                        "message": message
                                }),
                            ),
                            RecorderEvent::DanmuReceived { room, ts, content } => (
                                "danmu",
                                json!({
                                        "room": room,
                                        "ts": ts,
                                        "content": content
                                }),
                            ),
                            _ => continue,
                        };

                        if let Err(e) = socket_clone.emit(event_type, &message) {
                            log::warn!("Failed to emit progress event to WebSocket client: {}", e);
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        log::info!("Progress channel closed, stopping WebSocket progress stream");
                        break;
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        log::warn!("WebSocket client lagged, skipped {} events", skipped);
                    }
                }
            }
        });

        // Handle client messages
        socket.on("message", |socket: SocketRef, Data::<Value>(data)| {
            log::debug!("Received WebSocket message: {:?}", data);
            // Echo back the message for testing
            socket.emit("echo", &data).ok();
        });

        // Handle client disconnect
        socket.on_disconnect(|socket: SocketRef| {
            log::info!("WebSocket client disconnected: {}", socket.id);
        });

        log::info!("WebSocket client connected: {}", socket.id);
    });

    layer
}
