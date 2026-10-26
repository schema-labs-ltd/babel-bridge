use super::*;

// Read only the disposable test database, never the service's session data.
// Snapshot every persisted table so unexpected writes outside the message
// and receipt tables are caught as well.
fn feed_database_snapshot(
    dir: &std::path::Path,
) -> std::collections::BTreeMap<String, Vec<String>> {
    use rusqlite::{types::ValueRef, Connection, OpenFlags};
    let connection =
        Connection::open_with_flags(dir.join("messages.db"), OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let names = connection.prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap()
        .query_map([], |row| row.get::<_, String>(0)).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
    let mut snapshot = std::collections::BTreeMap::new();
    for name in names {
        let mut statement = connection
            .prepare(&format!("SELECT * FROM \"{}\"", name.replace('"', "\"\"")))
            .unwrap();
        let columns = statement.column_count();
        let mut rows = statement
            .query_map([], |row| {
                let values = (0..columns)
                    .map(|index| {
                        Ok(match row.get_ref(index)? {
                            ValueRef::Null => Value::Null,
                            ValueRef::Integer(value) => json!(value),
                            ValueRef::Real(value) => json!(value),
                            ValueRef::Text(value) => json!(std::str::from_utf8(value).unwrap()),
                            ValueRef::Blob(value) => json!({"blob":value}),
                        })
                    })
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(json!(values).to_string())
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        rows.sort();
        snapshot.insert(name, rows);
    }
    snapshot
}

#[tokio::test]
async fn unified_feed_http_auth_isolation_and_repeated_reads_have_no_side_effects() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let ai_requests = Arc::new(AtomicUsize::new(0));
    let observed = ai_requests.clone();
    let mock = axum::Router::new().route(
        "/responses",
        axum::routing::post(move || {
            let observed = observed.clone();
            async move {
                observed.fetch_add(1, Ordering::SeqCst);
                axum::http::StatusCode::INTERNAL_SERVER_ERROR
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api_url = format!("http://{}/responses", listener.local_addr().unwrap());
    let mock_task = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let mut first = FeedFixture::new();
    let mutable = Arc::get_mut(&mut first.state).unwrap();
    mutable.password = Some("synthetic-password".into());
    mutable.ai_service = Some(Arc::new(
        crate::translation::TranslationService::new_with_api_url(api_url),
    ));
    seed_feed(&first.state);
    let private = "447700900123@s.whatsapp.net";
    first
        .state
        .store
        .update_conversation_settings(
            private,
            &ConversationSettings {
                translation_enabled: true,
                reply_only: true,
                ..Default::default()
            },
        )
        .unwrap();
    let mut untranslated = extension_message("needs-translation", private, 50, true);
    untranslated.original_text = Some("Bonjour, ceci attend une traduction".into());
    untranslated.translated_text = None;
    untranslated.source_language = None;
    untranslated.is_translated = false;
    first
        .state
        .store
        .add_message_with_notification(&untranslated, Some(false))
        .unwrap();
    let mut audio = extension_message("needs-voice", private, 40, true);
    audio.content_type = "Audio".into();
    audio.content_json = json!({"type":"audio","media_data":"SYNTHETIC_AUDIO_BYTES"}).to_string();
    audio.original_text = None;
    audio.translated_text = None;
    audio.source_language = None;
    audio.is_translated = false;
    first
        .state
        .store
        .add_message_with_notification(&audio, Some(false))
        .unwrap();
    for (token, scope, expiration) in [
        ("feed-read", "whatsapp.read", i64::MAX),
        ("feed-full", "whatsapp.read whatsapp.send", i64::MAX),
        ("feed-legacy", "mcp", i64::MAX),
        ("feed-send-only", "whatsapp.send", i64::MAX),
        ("feed-expired", "whatsapp.read", 1),
    ] {
        feed_oauth_token(&first.state, token, scope, expiration);
    }
    let (tx, mut commands) = tokio::sync::mpsc::channel(8);
    first.state.set_command_tx(tx).await;
    *first.state.connected.write().await = true;
    let mut events = first.state.broadcast_tx.subscribe();
    let request_id = first.state.request_id_counter.load(Ordering::SeqCst);
    let before = feed_database_snapshot(&first.dir);
    assert!(before["translation_jobs"].is_empty());
    assert!(before["viewed_messages"].is_empty());
    assert!(!before["unread_messages"].is_empty());
    assert!(!before["pending_notifications"].is_empty());
    let call = json!({"name":"read_unified_messages","arguments":{"limit":2}});
    for auth in [
        None,
        Some("Bearer invalid-feed-token"),
        Some("Basic feed-read"),
        Some("Bearer feed-expired"),
    ] {
        let response = feed_rpc(first.state.clone(), auth, "tools/call", call.clone()).await;
        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
        assert!(response.headers().contains_key("www-authenticate"));
    }
    let tools = feed_rpc_body(
        feed_rpc(
            first.state.clone(),
            Some("Bearer feed-send-only"),
            "tools/list",
            json!({}),
        )
        .await,
    )
    .await;
    assert!(!tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| tool["name"] == "read_unified_messages"));
    let denied = feed_rpc_body(
        feed_rpc(
            first.state.clone(),
            Some("Bearer feed-send-only"),
            "tools/call",
            call.clone(),
        )
        .await,
    )
    .await;
    assert!(denied.get("error").is_some());
    for args in invalid_feed_arguments()
        .into_iter()
        .filter(Value::is_object)
    {
        let rejected = feed_rpc_body(
            feed_rpc(
                first.state.clone(),
                Some("Bearer feed-read"),
                "tools/call",
                json!({"name":"read_unified_messages","arguments":args}),
            )
            .await,
        )
        .await;
        assert!(rejected.get("error").is_some(), "accepted {args}");
    }
    for token in ["feed-read", "feed-full", "feed-legacy"] {
        let authorization = format!("Bearer {token}");
        for _ in 0..3 {
            let page = feed_rpc_body(
                feed_rpc(
                    first.state.clone(),
                    Some(&authorization),
                    "tools/call",
                    call.clone(),
                )
                .await,
            )
            .await;
            let page = &page["result"]["structuredContent"];
            assert_eq!(
                page["messages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|m| m["id"].as_str().unwrap())
                    .collect::<Vec<_>>(),
                ["b", "c"]
            );
            assert_eq!(
                page["nextCursor"],
                json!({"beforeTimestamp":100,"beforeMessageId":"b"})
            );
            let older = feed_rpc_body(feed_rpc(first.state.clone(), Some(&authorization), "tools/call", json!({"name":"read_unified_messages","arguments":{"limit":100,"before_timestamp":100,"before_message_id":"b"}})).await).await;
            let older = &older["result"]["structuredContent"];
            assert_eq!(
                older["messages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|m| m["id"].as_str().unwrap())
                    .collect::<Vec<_>>(),
                ["needs-voice", "needs-translation", "a"]
            );
            assert_eq!(older["hasMore"], false);
            assert!(older["nextCursor"].is_null());
        }
    }
    let mut second = FeedFixture::new();
    Arc::get_mut(&mut second.state).unwrap().password = Some("other-synthetic-password".into());
    second
        .state
        .store
        .upsert_contact(private, Some("Second instance"), None, Some("private"), 0)
        .unwrap();
    second
        .state
        .store
        .add_message(&extension_message("second-only", private, 100, true))
        .unwrap();
    feed_oauth_token(&second.state, "second-read", "whatsapp.read", i64::MAX);
    let second_before = feed_database_snapshot(&second.dir);
    assert_eq!(
        feed_rpc(
            second.state.clone(),
            Some("Bearer feed-read"),
            "tools/call",
            call.clone()
        )
        .await
        .status(),
        axum::http::StatusCode::UNAUTHORIZED
    );
    let isolated = feed_rpc_body(
        feed_rpc(
            second.state.clone(),
            Some("Bearer second-read"),
            "tools/call",
            call,
        )
        .await,
    )
    .await;
    assert_eq!(
        isolated["result"]["structuredContent"]["messages"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        isolated["result"]["structuredContent"]["messages"][0]["id"],
        "second-only"
    );
    assert_eq!(
        isolated["result"]["structuredContent"]["messages"][0]["contactName"],
        "Second instance"
    );
    assert_eq!(feed_database_snapshot(&first.dir), before);
    assert_eq!(feed_database_snapshot(&second.dir), second_before);
    assert!(matches!(
        commands.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    ));
    assert!(matches!(
        events.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Empty)
    ));
    assert_eq!(ai_requests.load(Ordering::SeqCst), 0);
    assert_eq!(
        first.state.request_id_counter.load(Ordering::SeqCst),
        request_id
    );
    assert!(first.state.mcp_prepared_messages.read().await.is_empty());
    assert!(first.state.pending_send_requests.read().await.is_empty());
    assert_eq!(first.state.voice_queue.available_permits(), 4);
    mock_task.abort();
}
