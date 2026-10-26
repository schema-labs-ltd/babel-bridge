use super::*;
use crate::storage::{ConversationSettings, MessageStore};

fn test_state() -> (Arc<AppState>, std::path::PathBuf) {
    let data_dir = std::env::temp_dir().join(format!(
        "whatsapp-translator-mcp-test-{}",
        uuid::Uuid::new_v4()
    ));
    let store = MessageStore::new(&data_dir).expect("test store");
    let state = AppState::new(
        store,
        std::env::current_dir().expect("cwd").join("web/public"),
        data_dir.clone(),
        None,
        None,
        None,
    );
    (state, data_dir)
}

struct FeedFixture {
    state: Arc<AppState>,
    dir: std::path::PathBuf,
}

impl FeedFixture {
    fn new() -> Self {
        let (state, dir) = test_state();
        Self { state, dir }
    }
}

impl Drop for FeedFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn seed_feed(state: &AppState) {
    let private = "447700900123@s.whatsapp.net";
    let group = "synthetic-group@g.us";
    state
        .store
        .upsert_contact(
            private,
            Some("Fixture Friend"),
            Some("447700900123"),
            Some("private"),
            0,
        )
        .unwrap();
    state
        .store
        .upsert_contact(group, Some("Fixture Group"), None, Some("group"), 0)
        .unwrap();
    // The feed sources are beyond the initial contacts page.
    for index in 0..60 {
        state
            .store
            .upsert_contact(
                &format!("other-{index}@example.test"),
                Some("Other"),
                None,
                Some("private"),
                1000 + index,
            )
            .unwrap();
    }
    let mut a = extension_message("a", "447700900123:22@s.whatsapp.net", 100, true);
    a.original_text = None;
    a.translated_text = None;
    a.source_language = None;
    a.is_translated = false;
    state.store.add_message(&a).unwrap();
    let mut b = extension_message("b", group, 100, true);
    b.chat_type = "group".into();
    b.sender_name = Some("Fixture Member".into());
    b.sender_phone = Some("900000000001@lid".into());
    b.content_type = "Image".into();
    b.content_json = json!({"type":"image", "caption":"Regarde", "media_data":"SYNTHETIC_MEDIA_BYTES",
        "reply_context":{"messageId":"outside-history", "senderName":"Quoted Member", "text":"Earlier quote", "unexpected":"must not leak"}}).to_string();
    b.original_text = Some("Regarde".into());
    b.translated_text = Some("Look".into());
    b.source_language = Some("French".into());
    state.store.add_message(&b).unwrap();
    let mut c = extension_message("c", private, 200, false);
    c.sender_name = Some("Me".into());
    c.sender_phone = Some("447700900000".into());
    c.original_text = Some("See you".into());
    c.translated_text = Some("À bientôt".into());
    c.source_language = Some("French".into());
    c.is_translated = true;
    c.delivery_status = Some("uncertain".into());
    state.store.add_message(&c).unwrap();
    state
        .store
        .upsert_contact("status@broadcast", None, None, Some("status"), 0)
        .unwrap();
    for (id, chat, content_type, content) in [
        (
            "status",
            "status@broadcast",
            "text",
            json!({"type":"text","body":"Status"}),
        ),
        (
            "reaction-label",
            private,
            "Reaction",
            json!({"type":"text","body":"Reaction label"}),
        ),
        (
            "reaction-payload",
            group,
            "text",
            json!({"type":"reaction","emoji":"👍"}),
        ),
    ] {
        let mut excluded = extension_message(id, chat, 300, true);
        excluded.content_type = content_type.into();
        excluded.content_json = content.to_string();
        state.store.add_message(&excluded).unwrap();
    }
}

fn extension_message(id: &str, contact_id: &str, timestamp: i64, incoming: bool) -> StoredMessage {
    StoredMessage {
        id: id.into(),
        contact_id: contact_id.into(),
        timestamp,
        is_from_me: !incoming,
        is_forwarded: false,
        sender_name: Some("Synthetic Sender".into()),
        sender_phone: None,
        contact_name: None,
        contact_phone: None,
        chat_type: "private".into(),
        content_type: "text".into(),
        content_json: r#"{"type":"text","body":"hello"}"#.into(),
        content: Some(json!({"type":"text","body":"hello"})),
        original_text: Some("hello".into()),
        translated_text: incoming.then(|| "bonjour".into()),
        source_language: Some("English".into()),
        is_translated: incoming,
        delivery_status: Some("delivered".into()),
    }
}

fn invalid_feed_arguments() -> Vec<Value> {
    vec![
        json!(null),
        json!([]),
        json!({"tenant":"other"}),
        json!({"contact_id":"ignored"}),
        json!({"limit":null}),
        json!({"limit":0}),
        json!({"limit":101}),
        json!({"limit":-1}),
        json!({"limit":1.5}),
        json!({"limit":"2"}),
        json!({"limit":true}),
        json!({"before_timestamp":100}),
        json!({"before_message_id":"a"}),
        json!({"before_timestamp":null,"before_message_id":"a"}),
        json!({"before_timestamp":100,"before_message_id":null}),
        json!({"before_timestamp":-1,"before_message_id":"a"}),
        json!({"before_timestamp":1.5,"before_message_id":"a"}),
        json!({"before_timestamp":JS_SAFE_INTEGER+1,"before_message_id":"a"}),
        json!({"before_timestamp":u64::MAX,"before_message_id":"a"}),
        json!({"before_timestamp":"100","before_message_id":"a"}),
        json!({"before_timestamp":true,"before_message_id":"a"}),
        json!({"before_timestamp":100,"before_message_id":""}),
        json!({"before_timestamp":100,"before_message_id":2}),
    ]
}

fn feed_oauth_token(state: &AppState, token: &str, scope: &str, expires_at: i64) {
    state
        .store
        .oauth_store_client(&crate::oauth::OAuthClientRegistration {
            client_id: token.into(),
            client_name: Some("Synthetic feed client".into()),
            redirect_uris: vec!["http://127.0.0.1/callback".into()],
            scope: scope.into(),
            created_at: 0,
        })
        .unwrap();
    state
        .store
        .oauth_store_access_token(&crate::oauth::AccessToken {
            token: token.into(),
            client_id: token.into(),
            scope: scope.into(),
            created_at: 0,
            expires_at,
            refresh_token: None,
        })
        .unwrap();
}

async fn feed_rpc(
    state: Arc<AppState>,
    authorization: Option<&str>,
    method: &str,
    params: Value,
) -> axum::response::Response {
    use tower::ServiceExt;
    let mut request = axum::http::Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("host", "127.0.0.1:3000")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("mcp-protocol-version", "2025-11-25");
    if let Some(authorization) = authorization {
        request = request.header("authorization", authorization);
    }
    crate::web::create_router(state)
        .oneshot(
            request
                .body(axum::body::Body::from(
                    json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn feed_rpc_body(response: axum::response::Response) -> Value {
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .unwrap();
    let text = std::str::from_utf8(&bytes).unwrap();
    serde_json::from_str(text).unwrap_or_else(|_| {
        serde_json::from_str(
            text.lines()
                .find_map(|line| line.strip_prefix("data: "))
                .expect("SSE data"),
        )
        .unwrap()
    })
}

mod extension;
mod feed;
mod feed_http;
mod logging;
mod prepare;
mod tools;
