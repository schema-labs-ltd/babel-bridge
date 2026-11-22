//! MCP Events extension over HTTP: capability, discovery, scopes and poll delivery.

use super::*;

pub(super) const CHAT_A: &str = "447700900123@s.whatsapp.net";
pub(super) const CHAT_B: &str = "synthetic-family@g.us";

/// A message as the fake bridge delivers it.
pub(super) fn wire(id: &str, chat: &str, from_me: bool, content: Value) -> Value {
    json!({
        "type":"message","id":id,"timestamp":1700000000,
        "from":{"jid":"447700900456@s.whatsapp.net","phone":"447700900456"},
        "chat":{"type":if chat.ends_with("@g.us") {"group"} else {"private"},"jid":chat,"name":"Synthetic chat"},
        "content":content,"is_from_me":from_me,"is_forwarded":false
    })
}

pub(super) fn text(body: &str) -> Value {
    json!({"type":"text","body":body})
}

pub(super) async fn bridge_delivers(state: &Arc<AppState>, message: Value) {
    crate::handle_web_event(
        serde_json::from_value(message).unwrap(),
        state,
        &state.store,
        None,
    )
    .await
    .unwrap();
}

pub(super) async fn events_request(
    state: Arc<AppState>,
    authorization: &str,
    method: &str,
    params: Value,
    mcp_name: Option<&str>,
) -> axum::response::Response {
    use tower::ServiceExt;
    let mut request = axum::http::Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("host", "127.0.0.1:3000")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("mcp-protocol-version", "2025-11-25")
        .header("authorization", authorization);
    if let Some(name) = mcp_name {
        request = request.header("mcp-name", name);
    }
    crate::web::create_router(state)
        .oneshot(
            request
                .body(axum::body::Body::from(
                    json!({"jsonrpc":"2.0","id":7,"method":method,"params":params}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn events_rpc(state: &Arc<AppState>, token: &str, method: &str, params: Value) -> Value {
    feed_rpc_body(
        events_request(
            state.clone(),
            &format!("Bearer {token}"),
            method,
            params,
            None,
        )
        .await,
    )
    .await
}

fn poll_params(arguments: Value, cursor: &Value) -> Value {
    json!({"name":"message.received","arguments":arguments,"cursor":cursor})
}

/// Poll until `count` events have arrived, then return them and the cursor.
async fn poll_until(
    state: &Arc<AppState>,
    token: &str,
    arguments: Value,
    cursor: &mut Value,
    count: usize,
) -> Vec<Value> {
    let mut events = Vec::new();
    for _ in 0..200 {
        let result = events_rpc(
            state,
            token,
            "events/poll",
            poll_params(arguments.clone(), cursor),
        )
        .await;
        let result = &result["result"];
        assert_eq!(result["resultType"], "complete");
        assert_eq!(result["truncated"], false);
        assert_eq!(result["nextPollMs"], 5000);
        *cursor = result["cursor"].clone();
        events.extend(result["events"].as_array().unwrap().iter().cloned());
        if events.len() >= count {
            return events;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("expected {count} events, got {events:?}");
}

#[tokio::test]
async fn events_capability_discovery_and_scope_checks() {
    let fixture = FeedFixture::new();
    let state = fixture.state.clone();
    for (token, scope, expiration) in [
        ("events-read", "whatsapp.read", i64::MAX),
        ("events-send-only", "whatsapp.send", i64::MAX),
        ("events-expired", "whatsapp.read", 1),
        ("events-revoked", "whatsapp.read", i64::MAX),
    ] {
        feed_oauth_token(&state, token, scope, expiration);
    }
    state.store.oauth_revoke_token("events-revoked").unwrap();

    let initialize = json!({"protocolVersion":"2025-11-25","capabilities":{},
        "clientInfo":{"name":"synthetic","version":"1"}});
    let info = events_rpc(&state, "events-read", "initialize", initialize).await;
    let capabilities = &info["result"]["capabilities"];
    assert_eq!(
        capabilities["extensions"]["io.modelcontextprotocol/events"],
        json!({})
    );
    assert!(capabilities["tools"].is_object());

    let listed = events_rpc(&state, "events-read", "events/list", json!({})).await;
    let events = listed["result"]["events"].as_array().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["name"], "message.received");
    assert_eq!(events[0]["delivery"], json!(["push", "poll"]));
    let tools = feed_rpc_body(
        feed_rpc(
            state.clone(),
            Some("Bearer events-read"),
            "tools/list",
            json!({}),
        )
        .await,
    )
    .await;
    let feed_tool = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "read_unified_messages")
        .unwrap()
        .clone();
    assert_eq!(
        events[0]["payloadSchema"],
        feed_tool["outputSchema"]["properties"]["messages"]["items"]
    );

    // A send-only token sees no event types and cannot subscribe in either mode.
    let hidden = events_rpc(&state, "events-send-only", "events/list", json!({})).await;
    assert_eq!(hidden["result"]["events"], json!([]));
    for method in ["events/poll", "events/stream"] {
        let denied = events_rpc(
            &state,
            "events-send-only",
            method,
            poll_params(json!({}), &Value::Null),
        )
        .await;
        assert_eq!(denied["error"]["code"], -32024, "{method}");
    }
    // Expired, revoked and unknown tokens get the OAuth challenge, as for tools.
    for token in ["events-expired", "events-revoked", "events-unknown"] {
        for method in ["events/poll", "events/stream", "events/list"] {
            let response = events_request(
                state.clone(),
                &format!("Bearer {token}"),
                method,
                poll_params(json!({}), &Value::Null),
                None,
            )
            .await;
            assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
            assert!(response.headers().contains_key("www-authenticate"));
        }
    }

    let unknown = events_rpc(
        &state,
        "events-read",
        "events/poll",
        json!({"name":"other","cursor":null}),
    )
    .await;
    assert_eq!(unknown["error"]["code"], -32023);
    for arguments in [
        json!({"tenant":"x"}),
        json!({"contact_id":""}),
        json!({"contact_id":5}),
        json!([]),
    ] {
        let rejected = events_rpc(
            &state,
            "events-read",
            "events/poll",
            poll_params(arguments.clone(), &Value::Null),
        )
        .await;
        assert_eq!(rejected["error"]["code"], -32602, "{arguments}");
    }
    for extra in [
        json!({"maxEvents":0}),
        json!({"maxEvents":-1}),
        json!({"cursor":5}),
    ] {
        let mut params = poll_params(json!({}), &Value::Null);
        params
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let rejected = events_rpc(&state, "events-read", "events/poll", params).await;
        assert_eq!(rejected["error"]["code"], -32602, "{extra}");
    }
    let webhook = events_rpc(
        &state,
        "events-read",
        "events/subscribe",
        poll_params(json!({}), &Value::Null),
    )
    .await;
    assert_eq!(webhook["error"]["code"], -32026);
    let missing = events_rpc(&state, "events-read", "events/unknown", json!({})).await;
    assert_eq!(missing["error"]["code"], -32601);
    let mismatch = events_request(
        state.clone(),
        "Bearer events-read",
        "events/poll",
        poll_params(json!({}), &Value::Null),
        Some("other.event"),
    )
    .await;
    assert_eq!(mismatch.status(), axum::http::StatusCode::BAD_REQUEST);
    let matched = events_request(
        state.clone(),
        "Bearer events-read",
        "events/poll",
        poll_params(json!({}), &Value::Null),
        Some("message.received"),
    )
    .await;
    assert!(feed_rpc_body(matched).await["result"]["cursor"].is_string());
}

#[tokio::test]
async fn poll_delivers_each_new_incoming_message_once_with_feed_fields() {
    let fixture = FeedFixture::new();
    let state = fixture.state.clone();
    feed_oauth_token(&state, "poll-read", "whatsapp.read", i64::MAX);
    feed_oauth_token(&state, "poll-legacy", "mcp", i64::MAX);
    let mut live = state.broadcast_tx.subscribe();

    // Subscribing writes nothing and starts from now.
    let before = feed_database_snapshot(&fixture.dir);
    let mut all = events_rpc(
        &state,
        "poll-read",
        "events/poll",
        poll_params(json!({}), &Value::Null),
    )
    .await["result"]
        .clone();
    assert_eq!(all["events"], json!([]));
    let mut chat_a = events_rpc(
        &state,
        "poll-legacy",
        "events/poll",
        poll_params(json!({"contact_id":CHAT_A}), &Value::Null),
    )
    .await["result"]
        .clone();
    assert_eq!(feed_database_snapshot(&fixture.dir), before);
    assert!(matches!(
        live.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Empty)
    ));

    // Own sends, reactions, receipts and typing are not events.
    bridge_delivers(&state, wire("own-a", CHAT_A, true, text("Sent by me"))).await;
    bridge_delivers(
        &state,
        wire(
            "reaction-a",
            CHAT_A,
            false,
            json!({"type":"reaction","emoji":"👍","target_message_id":"own-a"}),
        ),
    )
    .await;
    state.broadcast_receipt(vec!["own-a".into()], "read".into());
    state.broadcast_typing(CHAT_A.into(), CHAT_A.into(), "composing".into());
    bridge_delivers(
        &state,
        wire("incoming-b", CHAT_B, false, text("Group hello")),
    )
    .await;
    bridge_delivers(
        &state,
        wire("incoming-a", CHAT_A, false, text("Private hello")),
    )
    .await;

    let delivered = feed_database_snapshot(&fixture.dir);
    let mut cursor = all["cursor"].take();
    let received = poll_until(&state, "poll-read", json!({}), &mut cursor, 2).await;
    let ids: Vec<_> = received
        .iter()
        .map(|event| event["eventId"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["incoming-b", "incoming-a"]);
    let mut a_cursor = chat_a["cursor"].take();
    let filtered = poll_until(
        &state,
        "poll-legacy",
        json!({"contact_id":CHAT_A}),
        &mut a_cursor,
        1,
    )
    .await;
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0]["eventId"], "incoming-a");

    // The payload is the read_unified_messages FeedMessage for that message.
    let feed = feed_rpc_body(
        feed_rpc(
            state.clone(),
            Some("Bearer poll-read"),
            "tools/call",
            json!({"name":"read_unified_messages","arguments":{"limit":10}}),
        )
        .await,
    )
    .await;
    let feed = feed["result"]["structuredContent"]["messages"]
        .as_array()
        .unwrap()
        .clone();
    for event in &received {
        assert_eq!(event["name"], "message.received");
        assert!(event["timestamp"]
            .as_str()
            .unwrap()
            .starts_with("2023-11-14T"));
        let id = &event["eventId"];
        assert_eq!(&event["data"]["id"], id);
        assert_eq!(event["data"]["isFromMe"], false);
        assert_eq!(
            &event["data"],
            feed.iter().find(|message| &message["id"] == id).unwrap()
        );
    }
    assert_eq!(received[0]["data"]["chatType"], "group");

    // Nothing is delivered twice, polling writes nothing, and the cursor resumes.
    let again = events_rpc(
        &state,
        "poll-read",
        "events/poll",
        poll_params(json!({}), &cursor),
    )
    .await;
    assert_eq!(again["result"]["events"], json!([]));
    assert_eq!(feed_database_snapshot(&fixture.dir), delivered);
    let stale = format!("{}:0", "0".repeat(32));
    let restarted = events_rpc(
        &state,
        "poll-read",
        "events/poll",
        poll_params(json!({}), &json!(stale)),
    )
    .await;
    assert_eq!(restarted["result"]["truncated"], true);
    let mut capped = poll_params(json!({}), &json!(stale));
    capped["maxEvents"] = json!(1);
    let capped = events_rpc(&state, "poll-read", "events/poll", capped).await;
    assert_eq!(capped["result"]["events"].as_array().unwrap().len(), 1);
    assert_eq!(capped["result"]["hasMore"], true);
}
