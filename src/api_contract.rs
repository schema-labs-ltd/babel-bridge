//! Golden JSON for the HTTP and WebSocket API that the Apple apps decode.
//!
//! The server writes each response into `fixtures/api/responses/` by driving the
//! real router, and reads each request body the apps send from
//! `fixtures/api/requests/`. `scripts/check-api-contract.sh` decodes the same
//! files with `APIModels.swift`, so a renamed or retyped field fails on Linux
//! instead of only in the Apple job or at runtime.
//!
//! After an intended API change, rewrite the fixtures with
//! `UPDATE_API_FIXTURES=1 cargo test api_contract` and review the diff.

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
        let update = std::env::var_os("UPDATE_API_FIXTURES").is_some();
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
             `UPDATE_API_FIXTURES=1 cargo test api_contract`, then \
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

#[tokio::test]
async fn api_contract_matches_fixtures() {
    let (state, data_dir) = test_state(None);
    let app = create_router(state.clone());
    let mut live = state.broadcast_tx.subscribe();
    let mut responses = Fixtures::new("responses");

    responses.check("health", &get_ok(&app, "/api/health").await);
    ingest(&state, json!({"type": "qr", "data": "2@linking-code"})).await;
    responses.check("status", &get_ok(&app, "/api/status").await);
    responses.check("qr", &get_ok(&app, "/api/qr").await);
    ingest(
        &state,
        json!({"type": "connected", "phone": "447700900999", "name": "Me", "platform": "ios"}),
    )
    .await;

    seed_messages(&state).await;
    ingest(
        &state,
        json!({"type": "chat_presence", "chat_id": ANA, "user_id": ANA, "state": "typing"}),
    )
    .await;
    ingest(
        &state,
        json!({"type": "send_progress", "request_id": 7, "progress_id": "album-1",
               "stage": "uploading", "completed": 1, "total": 2}),
    )
    .await;

    let pin_response = send_ok(&app, "POST", &format!("/api/contacts/{FAMILY}/pin"), None).await;
    responses.check("pin", &pin_response);

    let mut contacts = get_ok(&app, "/api/contacts").await;
    for contact in contacts.as_array_mut().unwrap() {
        if contact["pinnedAt"].is_i64() {
            contact["pinnedAt"] = json!(1_767_400_000_000_i64);
        }
    }
    responses.check("contacts", &contacts);
    responses.check(
        "messages",
        &get_ok(&app, &format!("/api/messages/{ANA}?limit=30")).await,
    );
    responses.check("feed", &get_ok(&app, "/api/feed?limit=50").await);
    responses.check("media", &get_ok(&app, "/api/media/ben-photo-0").await);

    responses.check(
        "viewed-messages",
        &send_ok(
            &app,
            "POST",
            &format!("/api/contacts/{ANA}/read"),
            Some(request_fixture("viewed-messages")),
        )
        .await,
    );
    ingest(&state, json!({"type": "mark_as_read", "chat_id": FAMILY})).await;

    let settings_uri = format!("/api/contacts/{ANA}/settings");
    responses.check(
        "update-conversation-settings",
        &send_ok(
            &app,
            "PUT",
            &settings_uri,
            Some(request_fixture("conversation-settings")),
        )
        .await,
    );
    responses.check("conversation-settings", &get_ok(&app, &settings_uri).await);

    responses.check(
        "update-openai-settings",
        &send_ok(
            &app,
            "PUT",
            "/api/settings/openai",
            Some(request_fixture("openai-settings")),
        )
        .await,
    );
    responses.check(
        "openai-settings",
        &get_ok(&app, "/api/settings/openai").await,
    );
    responses.check("chatgpt-status", &get_ok(&app, "/api/chatgpt").await);

    let tone_uri = format!("/api/settings/message-tone?contactId={ANA}");
    responses.check(
        "update-message-tone",
        &send_ok(
            &app,
            "PUT",
            &tone_uri,
            Some(request_fixture("message-tone")),
        )
        .await,
    );
    responses.check("message-tone-settings", &get_ok(&app, &tone_uri).await);

    let voice_uri = format!("/api/voice/settings/{ANA}");
    responses.check(
        "update-voice-settings",
        &send_ok(
            &app,
            "PUT",
            &voice_uri,
            Some(request_fixture("voice-settings")),
        )
        .await,
    );
    responses.check("voice-settings", &get_ok(&app, &voice_uri).await);
    let (status, error) = call(&app, "PUT", &voice_uri, Some(json!({"voice": "robot"}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    responses.check("error", &error);

    state
        .store
        .record_usage(
            Some(ANA),
            Some("ana-hello"),
            &crate::translation::UsageInfo {
                input_tokens: 120,
                cached_input_tokens: 40,
                output_tokens: 30,
                cost_usd: 0.0125,
            },
            "translate_incoming",
        )
        .unwrap();
    responses.check("usage", &get_ok(&app, &format!("/api/usage/{ANA}")).await);

    state.store.set_topics_enabled(ANA, true).unwrap();
    let batch = state
        .store
        .next_topic_batch()
        .unwrap()
        .expect("topic batch");
    let assignments: Vec<_> = batch
        .messages
        .iter()
        .map(|message| crate::topics::TopicAssignment {
            message_id: message.id.clone(),
            topic: "Catching up".into(),
        })
        .collect();
    state
        .store
        .finish_topic_batch(&batch, &assignments)
        .unwrap();
    let mut topics = get_ok(&app, "/api/topics").await;
    pin(&mut topics, "/topics/0/id", json!("topic-catching-up"));
    responses.check("topics", &topics);
    responses.check(
        "message-topics",
        &send_ok(
            &app,
            "POST",
            "/api/topics/messages",
            Some(request_fixture("message-topics")),
        )
        .await,
    );

    // Responses that need the bridge or OpenAI to complete are pinned by
    // serializing the exact types the handlers return.
    responses.check(
        "send-message",
        &serde_json::to_value(SendMessageResponse {
            message_id: "3EB0SENT".into(),
            timestamp: 1_767_225_780_000,
            is_translated: true,
            translated_text: Some("¡Muy bien, gracias!".into()),
            source_language: Some("Spanish".into()),
            original_follow_up_sent: true,
            original_message_id: Some("3EB0ORIGINAL".into()),
            original_timestamp: Some(1_767_225_781_000),
            original_follow_up_error: None,
        })
        .unwrap(),
    );
    responses.check(
        "send-image",
        &serde_json::to_value(SendImageResponse {
            message_id: "3EB0IMAGE".into(),
            timestamp: 1_767_225_790_000,
        })
        .unwrap(),
    );
    responses.check(
        "reaction",
        &serde_json::to_value(SendReactionResponse {
            success: true,
            reaction: state.store.get_message_by_id("ana-reaction").unwrap(),
        })
        .unwrap(),
    );
    responses.check(
        "translate",
        &serde_json::to_value(TranslateMessageResponse {
            success: true,
            translated_text: Some("Hello, how are you?".into()),
            source_language: Some("Spanish".into()),
            error: None,
        })
        .unwrap(),
    );
    responses.check(
        "ai-reply",
        &serde_json::to_value(AiReplyResponse {
            success: true,
            reply_text: Some("All good here, you?".into()),
            error: None,
            cost_usd: Some(0.0021),
        })
        .unwrap(),
    );

    // Live events: the ones the steps above broadcast, then the rest.
    let mut events = Vec::new();
    while let Ok(event) = live.try_recv() {
        events.push(event);
    }
    events.extend([
        WebSocketEvent::Disconnected,
        WebSocketEvent::VoiceReady {
            message_id: "ben-voice".into(),
        },
        WebSocketEvent::Resync,
        WebSocketEvent::TopicsUpdated,
        WebSocketEvent::Heartbeat,
        WebSocketEvent::SendResult {
            request_id: 7,
            success: true,
            temp_message_id: Some("temp-1".into()),
            message_id: Some("3EB0SENT".into()),
            timestamp: Some(1_767_225_780_000),
            error: None,
        },
        WebSocketEvent::Error {
            error: "WhatsApp is not connected".into(),
        },
    ]);
    let covered: BTreeSet<_> = events.iter().map(live_event_name).collect();
    let all: BTreeSet<_> = LIVE_EVENT_NAMES.iter().copied().collect();
    assert_eq!(covered, all, "live-events.json must show every live event");
    responses.check(
        "live-events",
        &Value::Array(
            events
                .iter()
                .map(|event| serde_json::to_value(event).unwrap())
                .collect(),
        ),
    );

    let (password_state, password_dir) = test_state(Some("secret"));
    let password_app = create_router(password_state);
    responses.check(
        "auth-check",
        &get_ok(&password_app, "/api/auth/check").await,
    );
    let mut auth = send_ok(
        &password_app,
        "POST",
        "/api/auth",
        Some(request_fixture("auth")),
    )
    .await;
    pin(&mut auth, "/token", json!("session-token"));
    responses.check("auth", &auth);

    responses.finish();
    let _ = std::fs::remove_dir_all(data_dir);
    let _ = std::fs::remove_dir_all(password_dir);
}

/// The server accepts every request body the apps send. Bodies that need the
/// bridge or OpenAI are checked against the handler's request type here; the
/// others go through the router in `api_contract_matches_fixtures`.
#[test]
fn api_contract_requests_deserialize() {
    let send: SendMessageRequest = serde_json::from_value(request_fixture("send-message")).unwrap();
    assert_eq!(send.contact_id, ANA);
    assert_eq!(send.reply_to.as_deref(), Some("ana-hello"));
    assert!(send.reply_only_if_not_latest);

    let image: SendImageRequest = serde_json::from_value(request_fixture("send-image")).unwrap();
    assert_eq!(image.mime_type, "image/png");
    assert_eq!(image.media_kind.as_deref(), Some("image"));

    let album: CreatePhotoAlbumRequest =
        serde_json::from_value(request_fixture("create-photo-album")).unwrap();
    assert_eq!(album.photo_count, 2);

    let reaction: SendReactionRequest =
        serde_json::from_value(request_fixture("send-reaction")).unwrap();
    assert_eq!(reaction.emoji, "❤️");

    let translate: TranslateMessageRequest =
        serde_json::from_value(request_fixture("translate")).unwrap();
    assert_eq!(translate.message_id, "ana-hello");

    let reply: AiReplyRequest = serde_json::from_value(request_fixture("ai-reply")).unwrap();
    assert_eq!(reply.message_id, "ana-hello");

    let mut expected: BTreeSet<String> = [
        "ai-reply",
        "auth",
        "conversation-settings",
        "create-photo-album",
        "message-tone",
        "message-topics",
        "openai-settings",
        "send-image",
        "send-message",
        "send-reaction",
        "translate",
        "viewed-messages",
        "voice-settings",
    ]
    .into_iter()
    .map(|name| format!("{name}.json"))
    .collect();
    for entry in std::fs::read_dir(fixtures_dir().join("requests")).unwrap() {
        let file = entry.unwrap().file_name().to_string_lossy().into_owned();
        assert!(
            expected.remove(&file),
            "{file} is not read by any server test"
        );
    }
    assert!(
        expected.is_empty(),
        "missing request fixtures: {expected:?}"
    );
}
