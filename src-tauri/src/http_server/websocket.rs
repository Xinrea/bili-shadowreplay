use serde_json::Value;
use socketioxide::{
    extract::{Data, SocketRef},
    layer::SocketIoLayer,
    SocketIo,
};

use crate::state::State;

pub async fn create_websocket_server(state: State) -> SocketIoLayer {
    let (layer, io) = SocketIo::new_layer();

    io.ns("/ws", move |socket: SocketRef| {
        let mut events = state.events.subscribe();
        let event_socket = socket.clone();
        let event_task = tokio::spawn(async move {
            while let Ok(event) = events.recv().await {
                let event = event.client_event();
                if let Err(error) = event_socket.emit(&event.name, &event.payload) {
                    log::warn!("Failed to emit application event to WebSocket client: {error}");
                    break;
                }
            }
        });

        socket.on("message", |socket: SocketRef, Data::<Value>(data)| {
            log::debug!("Received WebSocket message: {data:?}");
            socket.emit("echo", &data).ok();
        });

        let abort = event_task.abort_handle();
        socket.on_disconnect(move |socket: SocketRef| {
            abort.abort();
            log::info!("WebSocket client disconnected: {}", socket.id);
        });

        log::info!("WebSocket client connected: {}", socket.id);
    });

    layer
}
