//! Send confirmation, outbox and restart recovery tests.

use super::*;

#[tokio::test]
async fn send_result_reconciles_pending_message_and_releases_waiter() {
    let (state, data_dir) = test_state(None);
    state
        .store
        .upsert_contact(
            "chat@example.test",
            Some("Test Chat"),
            None,
            Some("private"),
            1_700_000_000_000,
        )
        .expect("insert contact");
    state
        .store
        .add_message(&test_outgoing_message("pending_1", 1_700_000_000_000))
        .expect("insert pending");

    let rx = state
        .register_pending_send(42, Some("pending_1".to_string()))
        .await;
    state
        .handle_send_result(BridgeSendResult {
            request_id: 42,
            success: true,
            message_id: Some("real_1".to_string()),
            timestamp: Some(1_700_000_005),
            message_ids: Vec::new(),
            timestamps: Vec::new(),
            error: None,
        })
        .await;

    let result = rx.await.expect("send result");
    assert!(result.success);
    assert_eq!(result.message_id.as_deref(), Some("real_1"));
    assert_eq!(result.timestamp, Some(1_700_000_005_000));

    let messages = state
        .store
        .get_messages_paginated("chat@example.test", None, None, None, true)
        .expect("messages");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].id, "real_1");
    assert_eq!(messages[0].timestamp, 1_700_000_005_000);

    let _ = std::fs::remove_dir_all(data_dir);
}

#[tokio::test]
async fn failed_send_result_preserves_uncertain_message_and_releases_waiter() {
    let (state, data_dir) = test_state(None);
    state
        .store
        .upsert_contact(
            "chat@example.test",
            Some("Test Chat"),
            None,
            Some("private"),
            1_700_000_000_000,
        )
        .expect("insert contact");
    state
        .store
        .add_message(&test_outgoing_message("pending_1", 1_700_000_000_000))
        .expect("insert pending");

    let rx = state
        .register_pending_send(43, Some("pending_1".to_string()))
        .await;
    state
        .handle_send_result(BridgeSendResult {
            request_id: 43,
            success: false,
            message_id: None,
            timestamp: None,
            message_ids: Vec::new(),
            timestamps: Vec::new(),
            error: Some("rate limited".to_string()),
        })
        .await;

    let result = rx.await.expect("send result");
    assert!(!result.success);
    assert_eq!(result.error.as_deref(), Some("rate limited"));

    let messages = state
        .store
        .get_messages_paginated("chat@example.test", None, None, None, true)
        .expect("messages");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].delivery_status.as_deref(), Some("uncertain"));

    let _ = std::fs::remove_dir_all(data_dir);
}

#[tokio::test]
async fn login_attempts_are_limited_and_outbox_requires_auth() {
    let (state, dir) = test_state(Some("correct"));
    let router = create_router(state);
    for index in 0..16 {
        let response = router
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api/auth")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"password":"incorrect"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            if index < 15 {
                StatusCode::OK
            } else {
                StatusCode::TOO_MANY_REQUESTS
            }
        );
        if index == 15 {
            assert_eq!(response.headers()["retry-after"], "60");
        }
    }
    assert_eq!(
        router
            .oneshot(empty_request("/api/outbox/private-operation"))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn http_send_survives_disconnect_retries_and_restart_without_resending() {
    let (state, dir) = test_state(None);
    state
        .store
        .upsert_contact(
            "test@s.whatsapp.net",
            Some("Test contact"),
            None,
            Some("private"),
            1,
        )
        .unwrap();
    *state.connected.write().await = true;
    let (commands, mut receiver) = mpsc::channel(8);
    state.set_command_tx(commands).await;
    let router = create_router(state.clone());
    let request = || {
        HttpRequest::builder()
            .method("POST")
            .uri("/api/send")
            .header("content-type", "application/json")
            .header("idempotency-key", "durable-send-one")
            .body(Body::from(
                r#"{"contactId":"test@s.whatsapp.net","text":"Hello"}"#,
            ))
            .unwrap()
    };
    let sending = tokio::spawn(router.clone().oneshot(request()));
    let command = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    let BridgeCommand::Send {
        request_id: Some(request_id),
        ..
    } = command
    else {
        panic!("expected send")
    };
    sending.abort(); // Simulate the browser losing its HTTP connection after server acceptance.
    state
        .handle_send_result(BridgeSendResult {
            request_id,
            success: true,
            message_id: Some("confirmed-one".into()),
            timestamp: Some(1_700_000_000),
            message_ids: vec![],
            timestamps: vec![],
            error: None,
        })
        .await;
    for _ in 0..100 {
        if state
            .store
            .outbox_entry("durable-send-one")
            .unwrap()
            .unwrap()
            .state
            == "confirmed"
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let response = router.clone().oneshot(request()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-delivery-state"], "confirmed");
    assert!(receiver.try_recv().is_err());
    let changed = HttpRequest::builder()
        .method("POST")
        .uri("/api/send")
        .header("content-type", "application/json")
        .header("idempotency-key", "durable-send-one")
        .body(Body::from(
            r#"{"contactId":"test@s.whatsapp.net","text":"Different"}"#,
        ))
        .unwrap();
    assert_eq!(
        router.oneshot(changed).await.unwrap().status(),
        StatusCode::CONFLICT
    );
    let restarted = AppState::new(
        MessageStore::new(&dir).unwrap(),
        PathBuf::from("web/public"),
        dir.clone(),
        None,
        None,
        None,
    );
    assert!(restarted.next_request_id() > request_id);
    let response = create_router(restarted).oneshot(request()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(json["messageId"], "confirmed-one");
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn timeout_preserves_uncertain_message_and_late_ack_reconciles_after_restart() {
    let (state, dir) = test_state(None);
    state
        .store
        .upsert_contact("chat@example.test", None, None, None, 1)
        .unwrap();
    state
        .store
        .add_message(&test_outgoing_message("pending-late", 1_700_000_000_000))
        .unwrap();
    state
        .store
        .claim_outbox("late-send", "fingerprint", "/api/send")
        .unwrap();
    let rx = crate::outbox::OPERATION_ID
        .scope(
            "late-send".to_string(),
            state.register_pending_send(42, Some("pending-late".into())),
        )
        .await;
    assert_eq!(
        state
            .wait_for_send_result_with_timeout(42, rx, Duration::from_millis(5))
            .await
            .unwrap_err(),
        SendConfirmationError::Timeout
    );
    assert_eq!(
        state
            .store
            .get_message_by_id("pending-late")
            .unwrap()
            .unwrap()
            .delivery_status
            .as_deref(),
        Some("uncertain")
    );
    state
        .store
        .finish_outbox("late-send", 504, r#"{"error":"timeout"}"#)
        .unwrap();
    let restarted = AppState::new(
        MessageStore::new(&dir).unwrap(),
        PathBuf::from("web/public"),
        dir.clone(),
        None,
        None,
        None,
    );
    restarted
        .handle_send_result(BridgeSendResult {
            request_id: 42,
            success: true,
            message_id: Some("late-confirmed".into()),
            timestamp: Some(1_700_000_003),
            message_ids: vec![],
            timestamps: vec![],
            error: None,
        })
        .await;
    assert!(restarted
        .store
        .get_message_by_id("pending-late")
        .unwrap()
        .is_none());
    assert_eq!(
        restarted
            .store
            .get_message_by_id("late-confirmed")
            .unwrap()
            .unwrap()
            .delivery_status
            .as_deref(),
        Some("sent")
    );
    assert_eq!(
        restarted
            .store
            .outbox_entry("late-send")
            .unwrap()
            .unwrap()
            .state,
        "confirmed"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn persisted_receipt_recovers_crash_before_message_and_http_commit() {
    let (state, dir) = test_state(None);
    state
        .store
        .upsert_contact("chat@example.test", None, None, None, 1)
        .unwrap();
    state
        .store
        .add_message(&test_outgoing_message("pending-crash", 1_700_000_000_000))
        .unwrap();
    state
        .store
        .claim_outbox("crashed", "fingerprint", "/api/send")
        .unwrap();
    state
        .store
        .register_outbox_attempt(70, Some("crashed"), Some("pending-crash"))
        .unwrap();
    let receipt = BridgeSendResult {
        request_id: 70,
        success: true,
        message_id: Some("confirmed-crash".into()),
        timestamp: Some(1_700_000_001),
        message_ids: vec![],
        timestamps: vec![],
        error: None,
    };
    state
        .store
        .complete_attempt(70, true, &serde_json::to_string(&receipt).unwrap())
        .unwrap();
    drop(state);
    let restarted = AppState::new(
        MessageStore::new(&dir).unwrap(),
        PathBuf::from("web/public"),
        dir.clone(),
        None,
        None,
        None,
    );
    assert!(restarted
        .store
        .get_message_by_id("pending-crash")
        .unwrap()
        .is_none());
    assert_eq!(
        restarted
            .store
            .get_message_by_id("confirmed-crash")
            .unwrap()
            .unwrap()
            .delivery_status
            .as_deref(),
        Some("sent")
    );
    let operation = restarted.store.outbox_entry("crashed").unwrap().unwrap();
    assert_eq!(operation.state, "confirmed");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&operation.response.unwrap()).unwrap()
            ["messageId"],
        "confirmed-crash"
    );
    assert!(!restarted
        .store
        .claim_outbox("crashed", "fingerprint", "/api/send")
        .unwrap());
    drop(restarted);
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn incoming_translation_does_not_block_persistence_or_send_confirmations() {
    let entered = Arc::new(tokio::sync::Notify::new());
    let entered_handler = entered.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/responses", listener.local_addr().unwrap());
    let slow = Router::new().route(
        "/responses",
        post(move || {
            let entered = entered_handler.clone();
            async move {
                entered.notify_one();
                tokio::time::sleep(Duration::from_secs(5)).await;
                StatusCode::SERVICE_UNAVAILABLE
            }
        }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, slow).await.unwrap() });
    let (_, dir) = test_state(None);
    let translator = Arc::new(TranslationService::new_with_api_url(url));
    let state = AppState::new(
        MessageStore::new(&dir).unwrap(),
        PathBuf::from("web/public"),
        dir.clone(),
        Some(translator.clone()),
        None,
        None,
    );
    state
        .store
        .upsert_contact("123@s.whatsapp.net", None, None, Some("private"), 0)
        .unwrap();
    state
        .store
        .update_conversation_settings(
            "123@s.whatsapp.net",
            &ConversationSettings {
                translation_enabled: true,
                ..Default::default()
            },
        )
        .unwrap();
    crate::incoming::start(state.clone()).unwrap();
    let event:crate::bridge::BridgeEvent=serde_json::from_value(serde_json::json!({"type":"message","id":"incoming-one","timestamp":1700000000,"from":{"jid":"123@s.whatsapp.net","phone":"123"},"chat":{"type":"private","jid":"123@s.whatsapp.net"},"content":{"type":"text","body":"Hola"},"is_from_me":false,"is_forwarded":false})).unwrap();
    tokio::time::timeout(
        Duration::from_millis(100),
        crate::handle_web_event(event.clone(), &state, &state.store, Some(&translator)),
    )
    .await
    .expect("ingestion must not wait for OpenAI")
    .unwrap();
    assert_eq!(
        state
            .store
            .get_message_by_id("incoming-one")
            .unwrap()
            .unwrap()
            .original_text
            .as_deref(),
        Some("Hola")
    );
    tokio::time::timeout(Duration::from_secs(2), entered.notified())
        .await
        .expect("worker started the slow AI call");
    let rx = state.register_pending_send(81, None).await;
    tokio::time::timeout(
        Duration::from_millis(100),
        crate::handle_web_event(
            crate::bridge::BridgeEvent::SendResult {
                request_id: 81,
                success: true,
                message_id: Some("fast-confirmation".into()),
                timestamp: Some(1700000001),
                message_ids: vec![],
                timestamps: vec![],
                error: None,
            },
            &state,
            &state.store,
            Some(&translator),
        ),
    )
    .await
    .expect("send acknowledgements stay responsive")
    .unwrap();
    assert!(rx.await.unwrap().success);
    crate::handle_web_event(event, &state, &state.store, Some(&translator))
        .await
        .unwrap();
    assert_eq!(
        state
            .store
            .get_contact("123@s.whatsapp.net")
            .unwrap()
            .unwrap()
            .unread_count,
        1
    );
    server.abort();
    let _ = std::fs::remove_dir_all(dir);
}
