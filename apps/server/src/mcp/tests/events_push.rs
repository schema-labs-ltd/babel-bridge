//! MCP Events push delivery (`events/stream`) and revocation.

use super::events_http::{bridge_delivers, events_request, text, wire, CHAT_A, CHAT_B};
use super::*;
use futures::StreamExt;
use std::time::Duration;

/// Reads JSON-RPC messages from an `events/stream` SSE response.
struct SseReader {
    body: axum::body::BodyDataStream,
    buffer: String,
}

impl SseReader {
    fn new(response: axum::response::Response) -> Self {
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let content_type = response.headers()["content-type"]
            .to_str()
            .unwrap()
            .to_owned();
        assert!(
            content_type.starts_with("text/event-stream"),
            "{content_type}"
        );
        Self {
            body: response.into_body().into_data_stream(),
            buffer: String::new(),
        }
    }

    /// The next message, or `None` once the stream has ended.
    async fn next(&mut self) -> Option<Value> {
        loop {
            if let Some(end) = self.buffer.find("\n\n") {
                let frame: String = self.buffer.drain(..end + 2).collect();
                if let Some(data) = frame.lines().find_map(|line| line.strip_prefix("data:")) {
                    return Some(serde_json::from_str(data.trim()).unwrap());
                }
                continue;
            }
            let chunk = tokio::time::timeout(Duration::from_secs(10), self.body.next())
                .await
                .expect("stream stalled")?;
            self.buffer
                .push_str(std::str::from_utf8(&chunk.unwrap()).unwrap());
        }
    }

    async fn expect(&mut self, method: &str) -> Value {
        let message = self.next().await.expect("stream ended");
        assert_eq!(message["method"], method, "{message}");
        assert_eq!(
            message["params"]["_meta"]["io.modelcontextprotocol/subscriptionId"],
            7
        );
        message["params"].clone()
    }

    /// The final result frame, then the end of the stream.
    async fn expect_closed(&mut self) {
        let result = self.next().await.expect("final result");
        assert_eq!(result["id"], 7);
        assert_eq!(result["result"]["resultType"], "complete");
        assert!(self.next().await.is_none());
    }
}

async fn open_stream(
    state: &Arc<AppState>,
    token: &str,
    arguments: Value,
    extra: Value,
) -> SseReader {
    let mut params = json!({"name":"message.received","arguments":arguments,"cursor":null});
    params
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    SseReader::new(
        events_request(
            state.clone(),
            &format!("Bearer {token}"),
            "events/stream",
            params,
            None,
        )
        .await,
    )
}

#[tokio::test]
async fn push_stream_delivers_filtered_incoming_messages_until_revoked() {
    let fixture = FeedFixture::new();
    let state = fixture.state.clone();
    feed_oauth_token(&state, "push-read", "whatsapp.read whatsapp.send", i64::MAX);
    feed_oauth_token(&state, "push-filtered", "whatsapp.read", i64::MAX);

    let mut all = open_stream(&state, "push-read", json!({}), json!({})).await;
    let active = all.expect("notifications/events/active").await;
    assert_eq!(active["truncated"], false);
    assert_eq!(active["heartbeatMs"], 30000);
    let mut chat_a = open_stream(
        &state,
        "push-filtered",
        json!({"contact_id":CHAT_A}),
        json!({}),
    )
    .await;
    chat_a.expect("notifications/events/active").await;

    bridge_delivers(&state, wire("own-a", CHAT_A, true, text("Sent by me"))).await;
    state.broadcast_receipt(vec!["own-a".into()], "read".into());
    bridge_delivers(&state, wire("push-b", CHAT_B, false, text("Group hello"))).await;
    bridge_delivers(&state, wire("push-a", CHAT_A, false, text("Private hello"))).await;

    let first = all.expect("notifications/events/event").await;
    assert_eq!(first["eventId"], "push-b");
    assert_eq!(first["data"]["contactId"], CHAT_B);
    assert!(first["cursor"].is_string());
    let second = all.expect("notifications/events/event").await;
    assert_eq!(second["eventId"], "push-a");
    assert_eq!(second["data"]["text"], "Private hello");
    let filtered = chat_a.expect("notifications/events/event").await;
    assert_eq!(filtered["eventId"], "push-a");

    // Revocation stops delivery: the next message terminates the stream instead.
    state.store.oauth_revoke_token("push-filtered").unwrap();
    bridge_delivers(
        &state,
        wire("after-revoke", CHAT_A, false, text("Not for you")),
    )
    .await;
    let terminated = chat_a.expect("notifications/events/terminated").await;
    assert_eq!(terminated["error"]["code"], -32024);
    chat_a.expect_closed().await;
    let still = all.expect("notifications/events/event").await;
    assert_eq!(still["eventId"], "after-revoke");

    // Resuming from a cursor replays what came after it.
    let mut resumed = open_stream(
        &state,
        "push-read",
        json!({}),
        json!({"cursor": first["cursor"]}),
    )
    .await;
    assert_eq!(
        resumed.expect("notifications/events/active").await["truncated"],
        false
    );
    assert_eq!(
        resumed.expect("notifications/events/event").await["eventId"],
        "push-a"
    );
    assert_eq!(
        resumed.expect("notifications/events/event").await["eventId"],
        "after-revoke"
    );
}

#[tokio::test]
async fn idle_push_stream_heartbeats_and_ends_when_the_token_expires_or_is_revoked() {
    let fixture = FeedFixture::new();
    let state = fixture.state.clone();
    feed_oauth_token(&state, "idle-read", "whatsapp.read", i64::MAX);
    let mut stream =
        open_stream(&state, "idle-read", json!({}), json!({"heartbeatMs": 1000})).await;
    let active = stream.expect("notifications/events/active").await;
    assert_eq!(active["heartbeatMs"], 1000);
    let heartbeat = stream.expect("notifications/events/heartbeat").await;
    assert_eq!(heartbeat["cursor"], active["cursor"]);
    state.store.oauth_revoke_token("idle-read").unwrap();
    let terminated = stream.expect("notifications/events/terminated").await;
    assert_eq!(terminated["error"]["message"], "Forbidden");
    stream.expect_closed().await;

    // A token that expires while the stream is open also ends delivery.
    let expires_at = chrono::Utc::now().timestamp() + 2;
    feed_oauth_token(&state, "soon-expired", "whatsapp.read", expires_at);
    let mut expiring = open_stream(
        &state,
        "soon-expired",
        json!({}),
        json!({"heartbeatMs": 1000}),
    )
    .await;
    expiring.expect("notifications/events/active").await;
    loop {
        let message = expiring.next().await.expect("stream ended early");
        if message["method"] == "notifications/events/terminated" {
            break;
        }
        assert_eq!(message["method"], "notifications/events/heartbeat");
    }
    expiring.expect_closed().await;
}
