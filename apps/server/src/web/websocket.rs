//! WebSocket endpoint for live events.

use super::*;

pub(super) async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_websocket(socket, state))
}

/// How long an app counts as in front of the user after it last said so. Apps repeat it every
/// 30 seconds, so a suspended or vanished app stops counting even if its socket lingers.
const ACTIVE_FOR: Duration = Duration::from_secs(75);

/// Whether this socket's app is in front of the user. While any is, WhatsApp sees the account
/// online, as with WhatsApp Web, and sends typing indicators.
struct SocketPresence {
    state: Arc<AppState>,
    active_until: Option<tokio::time::Instant>,
}

impl SocketPresence {
    async fn set(&mut self, active: bool) {
        let was_active = self.active_until.is_some();
        self.active_until = active.then(|| tokio::time::Instant::now() + ACTIVE_FOR);
        if was_active != active {
            self.state.app_presence_changed(active).await;
        }
    }

    async fn expire(&mut self) {
        if self
            .active_until
            .is_some_and(|until| until <= tokio::time::Instant::now())
        {
            self.set(false).await;
        }
    }
}

impl Drop for SocketPresence {
    // Covers a handler that ends without passing `set(false)`, such as a panic.
    fn drop(&mut self) {
        if self.active_until.take().is_some() {
            let state = self.state.clone();
            tokio::spawn(async move { state.app_presence_changed(false).await });
        }
    }
}

/// Sends one frame, giving up on a client that has stopped reading. A blocked write would
/// otherwise stall this socket's loop, and with it the expiry of its app's presence.
async fn send_within<S>(sender: &mut S, message: Message) -> bool
where
    S: futures::Sink<Message> + Unpin,
{
    matches!(
        tokio::time::timeout(Duration::from_secs(15), sender.send(message)).await,
        Ok(Ok(()))
    )
}

/// An app's `{"type":"presence","active":true}`, sent while it is in front of the user.
pub(super) fn client_presence(text: &str) -> Option<bool> {
    #[derive(Deserialize)]
    struct ClientPresence {
        r#type: String,
        active: bool,
    }
    serde_json::from_str::<ClientPresence>(text)
        .ok()
        .filter(|message| message.r#type == "presence")
        .map(|message| message.active)
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
    let mut presence = SocketPresence {
        state: state.clone(),
        active_until: None,
    };
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
                presence.expire().await;
                if !send_within(&mut sender, Message::Text("{\"type\":\"heartbeat\"}".into())).await { break; }
            }
            // Broadcast events to client
            received = rx.recv() => {
                let event = match received {
                    Ok(event) => event,
                    Err(broadcast::error::RecvError::Lagged(_)) => WebSocketEvent::Resync,
                    Err(broadcast::error::RecvError::Closed) => break,
                };
                if let Ok(json) = serde_json::to_string(&event) {
                    if !send_within(&mut sender, Message::Text(json)).await {
                        break;
                    }
                }
            }

            // Handle client messages
            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Ping(data))) => {
                        if !send_within(&mut sender, Message::Pong(data)).await { break; }
                    }
                    Some(Ok(Message::Text(text))) => {
                        if let Some(active) = client_presence(&text) {
                            presence.set(active).await;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    presence.set(false).await;
}
