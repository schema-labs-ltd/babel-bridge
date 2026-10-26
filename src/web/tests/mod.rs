use super::*;
use crate::storage::ConversationSettings;
use axum::body::Body;
use axum::http::Request as HttpRequest;
use std::path::PathBuf;
use tower::ServiceExt;

mod auth;
mod feed;
mod images;
mod live_events;
mod mcp;
mod oauth;
mod read_receipts;
mod send_reliability;
mod sending;
mod settings;
mod status;
mod translation;

fn test_state(password: Option<&str>) -> (Arc<AppState>, PathBuf) {
    let data_dir = std::env::temp_dir().join(format!(
        "whatsapp-translator-web-test-{}",
        uuid::Uuid::new_v4()
    ));
    let store = MessageStore::new(&data_dir).expect("test message store");
    let web_dir = std::env::current_dir()
        .expect("current dir")
        .join("web/public");
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

fn empty_request(uri: &str) -> HttpRequest<Body> {
    HttpRequest::builder()
        .uri(uri)
        .body(Body::empty())
        .expect("request")
}

fn feed_test_message(id: &str, contact_id: &str, timestamp: i64) -> StoredMessage {
    let content = serde_json::json!({"type":"text", "body":id});
    StoredMessage {
        id: id.into(),
        contact_id: contact_id.into(),
        timestamp,
        is_from_me: false,
        is_forwarded: false,
        sender_name: Some("Tester".into()),
        sender_phone: None,
        contact_name: None,
        contact_phone: None,
        chat_type: "group".into(),
        content_type: "Text".into(),
        content_json: content.to_string(),
        content: Some(content),
        original_text: None,
        translated_text: None,
        source_language: None,
        is_translated: false,
        delivery_status: None,
    }
}

fn test_outgoing_message(id: &str, timestamp: i64) -> StoredMessage {
    let content = serde_json::json!({
        "type": "text",
        "body": "hello",
    });

    StoredMessage {
        id: id.to_string(),
        contact_id: "chat@example.test".to_string(),
        timestamp,
        is_from_me: true,
        is_forwarded: false,
        sender_name: Some("Me".to_string()),
        sender_phone: Some("123".to_string()),
        contact_name: Some("Test Chat".to_string()),
        contact_phone: None,
        chat_type: "private".to_string(),
        content_type: "Text".to_string(),
        content_json: content.to_string(),
        content: Some(content),
        original_text: None,
        translated_text: None,
        source_language: None,
        is_translated: false,
        delivery_status: Some("sent".to_string()),
    }
}

async fn extension_rpc(
    state: Arc<AppState>,
    authorization: Option<&str>,
    method: &str,
    params: serde_json::Value,
) -> axum::response::Response {
    let mut request = HttpRequest::builder()
        .method("POST")
        .uri("/mcp")
        .header("host", "127.0.0.1:3000")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("mcp-protocol-version", "2025-11-25");
    if let Some(value) = authorization {
        request = request.header("authorization", value);
    }
    create_router(state)
        .oneshot(
            request
                .body(Body::from(
                    serde_json::json!({"jsonrpc":"2.0","id":1,"method":method,"params":params})
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn extension_rpc_body(response: axum::response::Response) -> serde_json::Value {
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    // The stateless MCP HTTP transport can return JSON or a single SSE event.
    serde_json::from_str(
        text.trim()
            .strip_prefix("data: ")
            .unwrap_or(text.trim())
            .trim(),
    )
    .unwrap()
}
