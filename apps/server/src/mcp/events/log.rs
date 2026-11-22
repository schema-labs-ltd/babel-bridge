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
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, watch};

/// How many incoming messages are kept for poll and push resumption.
const CAPACITY: usize = 500;
/// The most push streams open at once, so idle subscribers cannot pile up.
pub(super) const MAX_STREAMS: usize = 32;

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
    streams: AtomicUsize,
}

/// Holds one push stream's slot until the stream ends.
pub(super) struct StreamSlot(Arc<EventLog>);

impl Drop for StreamSlot {
    fn drop(&mut self) {
        self.0.streams.fetch_sub(1, Ordering::SeqCst);
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
                    streams: AtomicUsize::new(0),
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

    /// Whether a live message is a new incoming chat message, using the same
    /// exclusions as `read_unified_messages`.
    fn is_incoming(message: &StoredMessage) -> bool {
        let reaction = |kind: &str| kind.eq_ignore_ascii_case("reaction");
        !message.is_from_me
            && message.contact_id != "status@broadcast"
            && !reaction(&message.content_type)
            && !message
                .content
                .as_ref()
                .and_then(|content| content.get("type"))
                .and_then(Value::as_str)
                .is_some_and(reaction)
    }

    pub(crate) fn record(&self, message: StoredMessage) {
        if !Self::is_incoming(&message) {
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

    pub(super) fn open_stream(self: &Arc<Self>) -> Option<StreamSlot> {
        self.streams
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |open| {
                (open < MAX_STREAMS).then_some(open + 1)
            })
            .ok()
            .map(|_| StreamSlot(self.clone()))
    }
}
