//! Golden JSON for the HTTP and WebSocket API that the Apple apps decode.
//!
//! The server writes each response into `fixtures/api/responses/` by driving the
//! real router, and reads each request body the apps send from
//! `fixtures/api/requests/`. `scripts/check-api-contract.sh` decodes the same
//! files with `APIModels.swift`, so a renamed or retyped field fails on Linux
//! instead of only in the Apple job or at runtime.
//!
//! After an intended API change, rewrite the fixtures with
//! `UPDATE_FIXTURES=1 cargo test api_contract` and review the diff.

use crate::bridge::BridgeEvent;
use crate::storage::MessageStore;
use crate::web::{
    create_router, AiReplyRequest, AiReplyResponse, AppState, CreatePhotoAlbumRequest,
    SendImageRequest, SendImageResponse, SendMessageRequest, SendMessageResponse,
    SendReactionRequest, SendReactionResponse, TranslateMessageRequest, TranslateMessageResponse,
    WebSocketEvent,
};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tower::ServiceExt;

mod contract_tests;

const ANA: &str = "447700900123@s.whatsapp.net";
const FAMILY: &str = "120363000000000001@g.us";
/// A 1x1 PNG, so media endpoints return real image bytes.
const PIXEL_PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/api")
}

/// Compares generated JSON with the committed fixture, or rewrites it on request.
struct Fixtures {
    dir: PathBuf,
    update: bool,
    written: BTreeSet<String>,
    mismatches: Vec<String>,
}

impl Fixtures {
    fn new(subdir: &str) -> Self {
        let dir = fixtures_dir().join(subdir);
        let update = std::env::var_os("UPDATE_FIXTURES").is_some();
        if update {
            std::fs::create_dir_all(&dir).expect("create fixture directory");
        }
        Self {
            dir,
            update,
            written: BTreeSet::new(),
            mismatches: Vec::new(),
        }
    }

    fn check(&mut self, name: &str, value: &Value) {
        let file = format!("{name}.json");
        assert!(self.written.insert(file.clone()), "{file} checked twice");
        let path = self.dir.join(&file);
        let expected = format!("{}\n", serde_json::to_string_pretty(value).unwrap());
        if self.update {
            std::fs::write(&path, expected).expect("write fixture");
            return;
        }
        match std::fs::read_to_string(&path) {
            Ok(actual) if actual == expected => {}
            Ok(_) => self.mismatches.push(format!("{file} changed:\n{expected}")),
            Err(_) => self.mismatches.push(format!("{file} is missing")),
        }
    }

    /// Every committed fixture must still be produced, or the Swift side decodes stale JSON.
    fn finish(self) {
        let mut problems = self.mismatches;
        for entry in std::fs::read_dir(&self.dir).expect("read fixture directory") {
            let file = entry.unwrap().file_name().to_string_lossy().into_owned();
            if file.ends_with(".json") && !self.written.contains(&file) {
                if self.update {
                    std::fs::remove_file(self.dir.join(&file)).unwrap();
                } else {
                    problems.push(format!("{file} is no longer produced"));
                }
            }
        }
        assert!(
            problems.is_empty(),
            "The API the apps decode changed. If that is intended, run \
             `UPDATE_FIXTURES=1 cargo test api_contract`, then \
             `scripts/check-api-contract.sh`, and update APIModels.swift.\n\n{}",
            problems.join("\n\n")
        );
    }
}

/// Reads a request body the apps send, as written in `fixtures/api/requests/`.
fn request_fixture(name: &str) -> Value {
    let path = fixtures_dir().join("requests").join(format!("{name}.json"));
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("missing request fixture {}", path.display()));
    serde_json::from_str(&text).expect("request fixture is JSON")
}

/// Replaces a value that differs between runs, such as a token or the current
/// time. The value must already have the replacement's JSON type, so pinning
/// never hides a type or nullability change.
fn pin(value: &mut Value, pointer: &str, replacement: Value) {
    let missing = format!("{pointer} is missing from {value}");
    let slot = value.pointer_mut(pointer).expect(&missing);
    assert_eq!(
        std::mem::discriminant(slot),
        std::mem::discriminant(&replacement),
        "{pointer} is {slot}, not the same JSON type as {replacement}"
    );
    *slot = replacement;
}

fn test_state(password: Option<&str>) -> (Arc<AppState>, PathBuf) {
    let data_dir = std::env::temp_dir().join(format!(
        "whatsapp-translator-api-contract-{}",
        uuid::Uuid::new_v4()
    ));
    let store = MessageStore::new(&data_dir).expect("test message store");
    let web_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("web/public");
    let state = AppState::new(
        store,
        web_dir,
        data_dir.clone(),
        None,
        password.map(str::to_string),
        None,
    );
    (state, data_dir)
}

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let builder = Request::builder().method(method).uri(uri);
    let request = match body {
        Some(body) => builder
            .header("content-type", "application/json")
            .body(Body::from(body.to_string())),
        None => builder.body(Body::empty()),
    }
    .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json = serde_json::from_slice(&bytes).unwrap_or_else(|_| {
        panic!(
            "{method} {uri} returned {status} without JSON: {}",
            String::from_utf8_lossy(&bytes)
        )
    });
    (status, json)
}

async fn get_ok(app: &Router, uri: &str) -> Value {
    let (status, json) = call(app, "GET", uri, None).await;
    assert_eq!(status, StatusCode::OK, "GET {uri}: {json}");
    json
}

async fn send_ok(app: &Router, method: &str, uri: &str, body: Option<Value>) -> Value {
    let (status, json) = call(app, method, uri, body).await;
    assert_eq!(status, StatusCode::OK, "{method} {uri}: {json}");
    json
}

/// Feeds bridge events through the same handler the running server uses.
async fn ingest(state: &Arc<AppState>, event: Value) {
    let event: BridgeEvent = serde_json::from_value(event).expect("bridge event");
    crate::handle_web_event(event, state, &state.store, None)
        .await
        .expect("ingest bridge event");
}

fn ana() -> Value {
    json!({"type": "private", "jid": ANA, "name": "Ana"})
}

fn family() -> Value {
    json!({"type": "group", "jid": FAMILY, "name": "Family", "participant_count": 4})
}

fn from_ana() -> Value {
    json!({"jid": ANA, "phone": "447700900123", "name": "Ana"})
}

fn from_me() -> Value {
    json!({"jid": "447700900999@s.whatsapp.net", "phone": "447700900999", "name": null})
}

fn from_ben() -> Value {
    json!({"jid": "447700900456@s.whatsapp.net", "phone": "447700900456", "name": "Ben"})
}

fn bridge_message(id: &str, timestamp: i64, from: Value, chat: Value, content: Value) -> Value {
    let is_from_me = from["name"].is_null();
    json!({
        "type": "message",
        "id": id,
        "timestamp": timestamp,
        "from": from,
        "chat": chat,
        "content": content,
        "is_from_me": is_from_me,
        "is_forwarded": false,
        "is_history": false,
        "push_name": null,
        "unread_count": null
    })
}

/// One of each message shape the apps render, ingested as the bridge sends them.
async fn seed_messages(state: &Arc<AppState>) {
    let mut events = vec![
        bridge_message(
            "ana-hello",
            1_767_225_600,
            from_ana(),
            ana(),
            json!({"type": "text", "body": "Hola, ¿cómo estás?"}),
        ),
        {
            let mut reply = bridge_message(
                "me-reply",
                1_767_225_660,
                from_me(),
                ana(),
                json!({"type": "text", "body": "Very well, thanks!"}),
            );
            reply["reply_context"] = json!({
                "messageId": "ana-hello",
                "senderName": "Ana",
                "text": "Hola, ¿cómo estás?"
            });
            reply
        },
        {
            let mut edit = bridge_message(
                "me-reply",
                1_767_225_660,
                from_me(),
                ana(),
                json!({"type": "text", "body": "Very well, thank you!"}),
            );
            edit["type"] = json!("message_edit");
            edit["edited_at_ms"] = json!(1_767_225_700_000_i64);
            edit
        },
        bridge_message(
            "ana-reaction",
            1_767_225_720,
            from_ana(),
            ana(),
            json!({
                "type": "reaction",
                "emoji": "👍",
                "target_message_id": "me-reply",
                "sender_timestamp_ms": 1_767_225_720_500_i64
            }),
        ),
        json!({"type": "receipt", "message_ids": ["me-reply"], "status": "read"}),
    ];
    for index in 0..2 {
        events.push(bridge_message(
            &format!("ben-photo-{index}"),
            1_767_312_000 + index,
            from_ben(),
            family(),
            json!({
                "type": "image",
                "caption": if index == 0 { Some("Beach day") } else { None },
                "mime_type": "image/png",
                "file_size": 70,
                "file_hash": null,
                "media_data": PIXEL_PNG,
                "album_id": "ben-album",
                "album_index": index,
                "width": 1,
                "height": 1
            }),
        ));
    }
    let mut voice = bridge_message(
        "ben-voice",
        1_767_312_060,
        from_ben(),
        family(),
        json!({
            "type": "audio",
            "mime_type": "audio/ogg; codecs=opus",
            "file_size": 4096,
            "duration_seconds": 7,
            "is_voice_note": true,
            "media_data": null
        }),
    );
    // History sync skips voice-note translation, which needs OpenAI.
    voice["is_history"] = json!(true);
    events.push(voice);
    events.extend([
        bridge_message(
            "ben-location",
            1_767_312_120,
            from_ben(),
            family(),
            json!({
                "type": "location",
                "latitude": 51.5007,
                "longitude": -0.1246,
                "name": "Big Ben",
                "address": "London SW1A 0AA"
            }),
        ),
        bridge_message(
            "ben-document",
            1_767_312_180,
            from_ben(),
            family(),
            json!({
                "type": "document",
                "caption": null,
                "mime_type": "application/pdf",
                "file_name": "tickets.pdf",
                "file_size": 2048,
                "media_data": null
            }),
        ),
        bridge_message(
            "ben-poll",
            1_767_312_240,
            from_ben(),
            family(),
            json!({"type": "poll", "question": "Dinner?", "options": ["Pizza", "Curry"]}),
        ),
    ]);
    for event in events {
        ingest(state, event).await;
    }
}

/// Every live event variant, by name. The match has no wildcard, so a new
/// variant does not compile until this test covers it.
fn live_event_name(event: &WebSocketEvent) -> &'static str {
    match event {
        WebSocketEvent::Status { .. } => "status",
        WebSocketEvent::Qr { .. } => "qr",
        WebSocketEvent::Connected { .. } => "connected",
        WebSocketEvent::Disconnected => "disconnected",
        WebSocketEvent::Message { .. } => "message",
        WebSocketEvent::Reaction { .. } => "reaction",
        WebSocketEvent::VoiceReady { .. } => "voice_ready",
        WebSocketEvent::MessageUpdated { .. } => "message_updated",
        WebSocketEvent::ConversationSettingsUpdated { .. } => "conversation_settings_updated",
        WebSocketEvent::Resync => "resync",
        WebSocketEvent::TopicsUpdated => "topics_updated",
        WebSocketEvent::Heartbeat => "heartbeat",
        WebSocketEvent::Typing { .. } => "typing",
        WebSocketEvent::MarkAsRead { .. } => "mark_as_read",
        WebSocketEvent::MessagesViewed { .. } => "messages_viewed",
        WebSocketEvent::Receipt { .. } => "receipt",
        WebSocketEvent::SendProgress { .. } => "send_progress",
        WebSocketEvent::SendResult { .. } => "send_result",
        WebSocketEvent::Error { .. } => "error",
    }
}

const LIVE_EVENT_NAMES: &[&str] = &[
    "status",
    "qr",
    "connected",
    "disconnected",
    "message",
    "reaction",
    "voice_ready",
    "message_updated",
    "conversation_settings_updated",
    "resync",
    "topics_updated",
    "heartbeat",
    "typing",
    "mark_as_read",
    "messages_viewed",
    "receipt",
    "send_progress",
    "send_result",
    "error",
];
