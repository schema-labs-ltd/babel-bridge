//! WebSocket endpoint for live events.

use super::*;

pub(super) async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_websocket(socket, state))
}

async fn handle_websocket(socket: WebSocket, state: Arc<AppState>) {
    let (mut sender, mut receiver) = socket.split();

    // Subscribe to broadcast events
    let mut rx = state.broadcast_tx.subscribe();

    // Send current status
    let status = WebSocketEvent::Status {
        connected: *state.connected.read().await,
        phone: state.phone.read().await.clone(),
        name: state.name.read().await.clone(),
        connection_state: *state.connection_state.read().await,
        qr: state.qr_code.read().await.clone(),
    };

    if let Ok(json) = serde_json::to_string(&status) {
        let _ = sender.send(Message::Text(json)).await;
    }

    // Send current QR if available
    if let Some(qr) = state.qr_code.read().await.clone() {
        let event = WebSocketEvent::Qr { data: qr };
        if let Ok(json) = serde_json::to_string(&event) {
            let _ = sender.send(Message::Text(json)).await;
        }
    }

    // Handle incoming messages and broadcast events
    let mut heartbeat = tokio::time::interval(Duration::from_secs(20));
    let mut shutdown = state.shutdown.subscribe();
    if *shutdown.borrow() {
        let _ = sender.send(Message::Close(None)).await;
        return;
    }
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_ok() && *shutdown.borrow() {
                    let _ = sender.send(Message::Close(None)).await;
                    break;
                }
            }
            _ = heartbeat.tick() => {
                if sender.send(Message::Text("{\"type\":\"heartbeat\"}".into())).await.is_err() { break; }
            }
            // Broadcast events to client
            received = rx.recv() => {
                let event = match received {
                    Ok(event) => event,
                    Err(broadcast::error::RecvError::Lagged(_)) => WebSocketEvent::Resync,
                    Err(broadcast::error::RecvError::Closed) => break,
                };
                if let Ok(json) = serde_json::to_string(&event) {
                    if sender.send(Message::Text(json)).await.is_err() {
                        break;
                    }
                }
            }

            // Handle client messages (for future use)
            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Ping(data))) => {
                        let _ = sender.send(Message::Pong(data)).await;
                    }
                    _ => {}
                }
            }
        }
    }
}
