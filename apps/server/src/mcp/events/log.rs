//! The in-memory record of incoming messages that MCP event subscriptions read.
//!
//! One task per `AppState` listens to the live-event broadcast, the same one the
//! web UI uses, and keeps the latest incoming messages in a bounded ring buffer.
//! Nothing here touches the database. Positions are sequence numbers within one
//! log; a cursor from another log (for example from before a restart) or one
//! older than the buffer is served with `truncated: true`.

use super::super::FeedMessage;
use crate::storage::StoredMessage;
use crate::web::{AppState, WebSocketEvent};
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, watch};

/// How many incoming messages are kept for poll and push resumption.
const CAPACITY: usize = 500;
/// The most push streams open at once, so idle subscribers cannot pile up.
const MAX_STREAMS: usize = 32;
/// The most push streams one OAuth client can hold, so one client cannot
/// take every slot.
pub(super) const MAX_STREAMS_PER_CLIENT: usize = 8;
/// History-sync message ids waiting to be skipped, bounded.
const MAX_HISTORY_IDS: usize = 4096;

pub(super) struct Entry {
    pub seq: u64,
    /// Canonical chat id, used only for the `contact_id` filter. Never logged.
    pub chat: String,
    /// When the server saw the message, in Unix milliseconds (for `maxAgeMs`).
    pub arrived_at: i64,
    pub timestamp: i64,
    pub id: String,
    pub data: Arc<Value>,
}

#[derive(Default)]
struct Buffer {
    entries: VecDeque<Arc<Entry>>,
    next_seq: u64,
    /// A sequence number spent on a lost stretch of the broadcast. Cursors
    /// before it cannot be served without a gap.
    gap_floor: u64,
}

/// Where a subscription starts reading, and whether events were skipped.
pub(super) struct Start {
    pub position: u64,
    pub truncated: bool,
}

pub struct EventLog {
    boot: String,
    buffer: Mutex<Buffer>,
    head: watch::Sender<u64>,
    /// Open push streams per OAuth client id.
    streams: Mutex<HashMap<String, usize>>,
    /// Ids of history-sync messages, which are old and never events.
    history: Mutex<(HashSet<String>, VecDeque<String>)>,
}

/// Holds one push stream's slot until the stream ends.
pub(super) struct StreamSlot(Arc<EventLog>, String);

impl Drop for StreamSlot {
    fn drop(&mut self) {
        let mut streams = self.0.streams.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(open) = streams.get_mut(&self.1) {
            *open -= 1;
            if *open == 0 {
                streams.remove(&self.1);
            }
        }
    }
}

impl EventLog {
    /// The log for this state, started on first use.
    pub(super) fn for_state(state: &AppState) -> Arc<Self> {
        state
            .mcp_events
            .get_or_init(|| {
                let log = Arc::new(Self {
                    boot: uuid::Uuid::new_v4().simple().to_string(),
                    buffer: Mutex::new(Buffer {
                        next_seq: 1,
                        ..Default::default()
                    }),
                    head: watch::channel(0).0,
                    streams: Mutex::default(),
                    history: Mutex::default(),
                });
                tokio::spawn(Self::listen(log.clone(), state.broadcast_tx.subscribe()));
                log
            })
            .clone()
    }

    async fn listen(log: Arc<Self>, mut events: broadcast::Receiver<WebSocketEvent>) {
        loop {
            match events.recv().await {
                Ok(WebSocketEvent::Message { message }) => log.record(message),
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(_)) => log.mark_gap(),
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    }

    /// Mark a history-sync message, which is broadcast like a new one, so that
    /// it does not become an event. Call before broadcasting it. Does nothing
    /// until an MCP client has used events.
    pub fn note_history(state: &AppState, id: &str) {
        let Some(log) = state.mcp_events.get() else {
            return;
        };
        let mut history = log.history.lock().unwrap_or_else(|e| e.into_inner());
        let (ids, order) = &mut *history;
        if ids.insert(id.to_owned()) {
            order.push_back(id.to_owned());
            if order.len() > MAX_HISTORY_IDS {
                if let Some(oldest) = order.pop_front() {
                    ids.remove(&oldest);
                }
            }
        }
    }

    fn is_history(&self, id: &str) -> bool {
        let mut history = self.history.lock().unwrap_or_else(|e| e.into_inner());
        let (ids, order) = &mut *history;
        let found = ids.remove(id);
        if found {
            order.retain(|known| known != id);
        }
        found
    }

    /// Whether a live message is a new incoming message in a private or group
    /// chat. Broadcast lists and status updates are not chats here; reactions,
    /// deletions and protocol messages are not messages, as for unread counts.
    fn is_incoming(message: &StoredMessage) -> bool {
        let not_a_message =
            |kind: &str| ["reaction", "revoked", "protocol", "unknown"].contains(&kind);
        let content_type = message.content_type.to_lowercase();
        !message.is_from_me
            && matches!(message.chat_type.as_str(), "private" | "group")
            && message.contact_id != "status@broadcast"
            && !not_a_message(&content_type)
            && !message
                .content
                .as_ref()
                .and_then(|content| content.get("type"))
                .and_then(Value::as_str)
                .is_some_and(|kind| not_a_message(&kind.to_lowercase()))
    }

    pub(crate) fn record(&self, message: StoredMessage) {
        if self.is_history(&message.id) || !Self::is_incoming(&message) {
            return;
        }
        let chat = crate::identity::canonical_chat_id(&message.contact_id).into_owned();
        let (id, timestamp) = (message.id.clone(), message.timestamp);
        let Ok(data) = serde_json::to_value(FeedMessage::from(message)) else {
            return;
        };
        let head = {
            let mut buffer = self.buffer.lock().unwrap_or_else(|e| e.into_inner());
            let seq = buffer.next_seq;
            buffer.next_seq += 1;
            buffer.entries.push_back(Arc::new(Entry {
                seq,
                chat,
                arrived_at: chrono::Utc::now().timestamp_millis(),
                timestamp,
                id,
                data: Arc::new(data),
            }));
            if buffer.entries.len() > CAPACITY {
                buffer.entries.pop_front();
            }
            seq
        };
        self.head.send_replace(head);
    }

    fn mark_gap(&self) {
        let head = {
            let mut buffer = self.buffer.lock().unwrap_or_else(|e| e.into_inner());
            buffer.gap_floor = buffer.next_seq;
            buffer.next_seq += 1;
            buffer.gap_floor
        };
        self.head.send_replace(head);
    }

    pub(super) fn cursor(&self, position: u64) -> String {
        format!("{}:{position}", self.boot)
    }

    pub(super) fn subscribe_head(&self) -> watch::Receiver<u64> {
        self.head.subscribe()
    }

    /// The position to read from for a client cursor. `None` starts from now.
    /// A cursor this log cannot vouch for starts from the oldest complete
    /// position with `truncated: true`.
    pub(super) fn start(&self, cursor: Option<&str>) -> Start {
        let buffer = self.buffer.lock().unwrap_or_else(|e| e.into_inner());
        let head = buffer.next_seq - 1;
        let Some(cursor) = cursor else {
            return Start {
                position: head,
                truncated: false,
            };
        };
        let oldest = buffer.entries.front().map_or(buffer.next_seq, |e| e.seq);
        let floor = buffer.gap_floor.max(oldest - 1);
        let parsed = cursor
            .split_once(':')
            .filter(|(boot, _)| *boot == self.boot)
            .and_then(|(_, seq)| seq.parse::<u64>().ok())
            .filter(|seq| *seq <= head);
        match parsed {
            Some(seq) if seq >= floor => Start {
                position: seq,
                truncated: false,
            },
            _ => Start {
                position: floor,
                truncated: true,
            },
        }
    }

    /// Entries after `position`, whether the log lost events after it, and
    /// the head position of this snapshot.
    pub(super) fn after(&self, position: u64) -> (Vec<Arc<Entry>>, bool, u64) {
        let buffer = self.buffer.lock().unwrap_or_else(|e| e.into_inner());
        let oldest = buffer.entries.front().map_or(buffer.next_seq, |e| e.seq);
        let gap = position < buffer.gap_floor || position + 1 < oldest;
        let entries = buffer
            .entries
            .iter()
            .filter(|entry| entry.seq > position)
            .cloned()
            .collect();
        (entries, gap, buffer.next_seq - 1)
    }

    pub(super) fn open_stream(self: &Arc<Self>, client_id: &str) -> Option<StreamSlot> {
        let mut streams = self.streams.lock().unwrap_or_else(|e| e.into_inner());
        let total: usize = streams.values().sum();
        let open = streams.get(client_id).copied().unwrap_or_default();
        if total >= MAX_STREAMS || open >= MAX_STREAMS_PER_CLIENT {
            return None;
        }
        streams.insert(client_id.to_owned(), open + 1);
        Some(StreamSlot(self.clone(), client_id.to_owned()))
    }
}
