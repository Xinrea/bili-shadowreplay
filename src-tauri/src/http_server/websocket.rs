use serde_json::Value;
use socketioxide::{
    extract::{AckSender, Data, SocketRef, TryData},
    layer::SocketIoLayer,
    SocketIo,
};

use super::event_delivery::EventDelivery;
use crate::events::{RoomForwarder, RoomTopic};
use crate::state::State;
use std::sync::{Arc, Mutex};

pub async fn create_websocket_server(state: State) -> SocketIoLayer {
    let (layer, io) = SocketIo::new_layer();

    io.ns("/ws", move |socket: SocketRef| {
        let mut events = state.events.subscribe();
        let event_socket = socket.clone();
        let delivery = EventDelivery::default();
        let control_delivery = delivery.clone();
        let event_task = tokio::spawn(async move {
            while let Ok(event) = events.recv().await {
                let event = event.client_event();
                if let Err(error) = control_delivery
                    .control(|| event_socket.emit(&event.name, &event.payload))
                    .await
                {
                    log::warn!("Failed to emit application event to WebSocket client: {error}");
                    break;
                }
            }
        });

        let rooms = Arc::new(Mutex::new(RoomForwarder::default()));
        let room_subscriptions = rooms.clone();
        let bus = state.events.clone();
        socket.on(
            "event-rooms",
            move |socket: SocketRef, TryData::<Vec<RoomTopic>>(topics), ack: AckSender| {
                let delivery = delivery.clone();
                let result = topics
                    .map_err(|error| error.to_string())
                    .and_then(|topics| {
                        room_subscriptions
                            .lock()
                            .unwrap()
                            .set_rooms(&bus, topics, move |event| {
                                delivery
                                    .preview(|| socket.emit(&event.name, &event.payload))
                                    .map_err(|error| error.to_string())
                            })
                    });
                if let Err(error) = &result {
                    log::warn!("Invalid room event subscription: {error}");
                }
                let response = match result {
                    Ok(()) => serde_json::json!({ "ok": true }),
                    Err(error) => serde_json::json!({ "ok": false, "error": error }),
                };
                if let Err(error) = ack.send(&response) {
                    log::warn!("Failed to acknowledge room subscription: {error}");
                }
            },
        );

        socket.on("message", |socket: SocketRef, Data::<Value>(data)| {
            log::debug!("Received WebSocket message: {data:?}");
            socket.emit("echo", &data).ok();
        });

        let abort = event_task.abort_handle();
        socket.on_disconnect(move |socket: SocketRef| {
            abort.abort();
            rooms.lock().unwrap().clear();
            log::info!("WebSocket client disconnected: {}", socket.id);
        });

        log::info!("WebSocket client connected: {}", socket.id);
    });

    layer
}
