//! Push delivery: one long-lived `events/stream` request per subscription,
//! answered with an SSE stream of `notifications/events/*` messages.

use super::log::{EventLog, StreamSlot};
use super::{occurrence, optional_count, Caller, EventError, Subscription, RESOURCE_EXHAUSTED};
use crate::web::AppState;
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use serde_json::{json, Value};
use std::convert::Infallible;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time::{Duration, Instant};

const DEFAULT_HEARTBEAT_MS: u64 = 30_000;
const MIN_HEARTBEAT_MS: u64 = 1_000;
const MAX_HEARTBEAT_MS: u64 = 60_000;

struct Stream {
    state: Arc<AppState>,
    log: Arc<EventLog>,
    caller: Caller,
    subscription: Subscription,
    id: Value,
    heartbeat: Duration,
    tx: mpsc::Sender<Value>,
    _slot: StreamSlot,
}

/// Whether the client is gone, in which case the stream ends quietly.
struct Gone;

pub(super) fn open(
    state: Arc<AppState>,
    caller: Caller,
    subscription: Subscription,
    id: Value,
) -> Result<Response, EventError> {
    let heartbeat_ms = optional_count(&subscription.params, "heartbeatMs")?
        .unwrap_or(DEFAULT_HEARTBEAT_MS)
        .clamp(MIN_HEARTBEAT_MS, MAX_HEARTBEAT_MS);
    let log = EventLog::for_state(&state);
    let slot = log.open_stream(&caller.client_id).ok_or_else(|| {
        EventError::new(RESOURCE_EXHAUSTED, "ResourceExhausted").with_data(json!({
            "limit": "subscriptions",
            "max": super::log::MAX_STREAMS_PER_CLIENT,
        }))
    })?;
    let (tx, rx) = mpsc::channel(16);
    let stream = Stream {
        state,
        log,
        caller,
        subscription,
        id,
        heartbeat: Duration::from_millis(heartbeat_ms),
        tx,
        _slot: slot,
    };
    tokio::spawn(stream.run());
    let frames = futures::stream::unfold(rx, |mut rx| async move {
        let message: Value = rx.recv().await?;
        Some((
            Ok::<_, Infallible>(Event::default().data(message.to_string())),
            rx,
        ))
    });
    Ok(Sse::new(frames).into_response())
}

impl Stream {
    fn notification(&self, method: &str, mut params: Value) -> Value {
        params["_meta"] = json!({"io.modelcontextprotocol/subscriptionId": self.id});
        json!({"jsonrpc": "2.0", "method": method, "params": params})
    }

    async fn send(&mut self, message: Value) -> Result<(), Gone> {
        self.tx.send(message).await.map_err(|_| Gone)
    }

    async fn active(&mut self, position: u64, truncated: bool) -> Result<(), Gone> {
        let params = json!({
            "cursor": self.log.cursor(position),
            "truncated": truncated,
            "heartbeatMs": self.heartbeat.as_millis() as u64,
        });
        let message = self.notification("notifications/events/active", params);
        self.send(message).await
    }

    /// Send the final `StreamEventsResult` and close the stream.
    async fn finish(&mut self) {
        let meta = json!({"io.modelcontextprotocol/subscriptionId": self.id});
        let result = json!({"jsonrpc": "2.0", "id": self.id,
            "result": {"resultType": "complete", "_meta": meta}});
        let _ = self.send(result).await;
    }

    /// End the subscription because the token no longer grants read access.
    async fn terminate(&mut self) {
        let params = json!({"error": {"code": super::FORBIDDEN, "message": "Forbidden",
            "data": {"reason": "Access revoked"}}});
        let message = self.notification("notifications/events/terminated", params);
        if self.send(message).await.is_ok() {
            self.finish().await;
        }
        crate::log_privacy::log_mcp_call(
            "events/stream",
            Some(super::EVENT_NAME),
            "terminated",
            Some(super::FORBIDDEN),
            0,
            Duration::ZERO,
        );
    }

    async fn run(mut self) {
        let mut head = self.log.subscribe_head();
        let mut shutdown = self.state.shutdown.subscribe();
        let start = self.log.start(self.subscription.cursor.as_deref());
        let mut position = start.position;
        let mut truncated = start.truncated;
        // Apply maxAgeMs to the replay before confirming the subscription.
        let now = chrono::Utc::now().timestamp_millis();
        for entry in self.log.after(position).0 {
            if !self.subscription.too_old(&entry, now) {
                break;
            }
            position = entry.seq;
            truncated = true;
        }
        if self.active(position, truncated).await.is_err() {
            return;
        }
        let mut last_sent = Instant::now();
        loop {
            let (entries, gap, snapshot_head) = self.log.after(position);
            if gap {
                let restart = self.log.start(Some(&self.log.cursor(position)));
                position = restart.position;
                if self.active(position, true).await.is_err() {
                    return;
                }
                last_sent = Instant::now();
                continue;
            }
            for entry in entries {
                if !self.caller.still_allowed(&self.state) {
                    return self.terminate().await;
                }
                position = entry.seq;
                if self.subscription.matches(&entry) {
                    let event = occurrence(&entry, Some(self.log.cursor(entry.seq)));
                    let message = self.notification("notifications/events/event", event);
                    if self.send(message).await.is_err() {
                        return;
                    }
                    last_sent = Instant::now();
                }
            }
            position = position.max(snapshot_head);
            if *shutdown.borrow() {
                return self.finish().await;
            }
            tokio::select! {
                changed = head.changed() => {
                    if changed.is_err() {
                        return self.finish().await;
                    }
                }
                _ = shutdown.changed() => {}
                // The client went away: free the stream slot at once.
                _ = self.tx.closed() => return,
                _ = tokio::time::sleep_until(last_sent + self.heartbeat) => {
                    if !self.caller.still_allowed(&self.state) {
                        return self.terminate().await;
                    }
                    let params = json!({"cursor": self.log.cursor(position)});
                    let message = self.notification("notifications/events/heartbeat", params);
                    if self.send(message).await.is_err() {
                        return;
                    }
                    last_sent = Instant::now();
                }
            }
        }
    }
}
