//! Outgoing translation and caption tests.

use super::*;

#[tokio::test]
async fn disabled_translation_sends_original_text_and_captions_for_people_groups_and_unified_replies_without_ai(
) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/responses", listener.local_addr().unwrap());
    let mock = Router::new().route(
        "/responses",
        post(move || {
            counter.fetch_add(1, Ordering::SeqCst);
            async { StatusCode::BAD_GATEWAY }
        }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let (_, dir) = test_state(None);
    let state = AppState::new(
        MessageStore::new(&dir).unwrap(),
        PathBuf::from("../web/public"),
        dir.clone(),
        Some(Arc::new(TranslationService::new_with_api_url(url))),
        None,
        None,
    );
    let ids = ["123@s.whatsapp.net", "family@g.us"];
    for id in ids {
        state
            .store
            .upsert_contact(
                id,
                None,
                None,
                Some(if id.ends_with("g.us") {
                    "group"
                } else {
                    "private"
                }),
                0,
            )
            .unwrap();
        for (suffix, time) in [("selected", 100), ("newer", 200)] {
            let mut message = feed_test_message(&format!("{id}-{suffix}"), id, time);
            message.original_text = Some("Jó reggelt".into());
            message.source_language = Some("Hungarian".into());
            state.store.add_message(&message).unwrap();
        }
    }
    state.set_connected(true, None, None).await;
    let (tx, mut rx) = mpsc::channel(8);
    state.set_command_tx(tx).await;
    let app = create_router(state.clone());
    for (index, id) in ids.into_iter().enumerate() {
        for other in ids {
            state
                .store
                .update_conversation_settings(
                    other,
                    &ConversationSettings {
                        reply_only: false,
                        translation_enabled: other != id,
                        language_override: Some("Hungarian".into()),
                        translation_style: Some("friendly".into()),
                        send_original_follow_up: true,
                    },
                )
                .unwrap();
        }
        for unified in [false, true] {
            let reply = unified.then(|| format!("{id}-selected"));
            let body = serde_json::json!({"contactId":id,"text":"Good morning, see you soon!","replyTo":reply,"replyToText":"Jó reggelt","replyOnlyIfNotLatest":unified});
            let request = HttpRequest::builder()
                .method("POST")
                .uri("/api/send")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap();
            let sending = tokio::spawn(app.clone().oneshot(request));
            let command = tokio::time::timeout(Duration::from_secs(2), rx.recv())
                .await
                .unwrap()
                .unwrap();
            let BridgeCommand::Send {
                request_id: Some(request_id),
                to,
                text,
                reply_to,
                ..
            } = command
            else {
                panic!("Expected text send")
            };
            assert_eq!(to, id);
            assert_eq!(text, "Good morning, see you soon!");
            assert_eq!(reply_to, reply);
            let sent_id = format!("confirmed-{index}-{unified}");
            state
                .handle_send_result(BridgeSendResult {
                    request_id,
                    success: true,
                    message_id: Some(sent_id.clone()),
                    timestamp: Some(1000),
                    message_ids: vec![],
                    timestamps: vec![],
                    error: None,
                })
                .await;
            let response = sending.await.unwrap().unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert!(
                !state
                    .store
                    .get_message_by_id(&sent_id)
                    .unwrap()
                    .unwrap()
                    .is_translated
            );
            assert!(
                rx.try_recv().is_err(),
                "disabled conversations must not send an original follow-up twice"
            );
        }
        assert_eq!(
            translate_caption(
                &state,
                id,
                Some(&format!("{id}-selected")),
                Some("Good morning")
            )
            .await
            .unwrap(),
            (Some("Good morning".into()), None)
        );
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "off must bypass all detection and translation API calls"
    );
    server.abort();
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn conversation_and_unified_history_only_queue_translation_for_enabled_contacts() {
    let (_, dir) = test_state(None);
    let state = AppState::new(
        MessageStore::new(&dir).unwrap(),
        PathBuf::from("../web/public"),
        dir.clone(),
        Some(Arc::new(TranslationService::new_with_api_url(
            "http://127.0.0.1:9/responses".into(),
        ))),
        None,
        None,
    );
    for (id, message_id) in [
        ("123@s.whatsapp.net", "person-message"),
        ("family@g.us", "group-message"),
    ] {
        state.store.upsert_contact(id, None, None, None, 0).unwrap();
        let mut message = feed_test_message(message_id, id, 100);
        message.original_text = Some("Jó reggelt".into());
        state
            .store
            .add_message_with_notification(&message, Some(true))
            .unwrap();
    }
    let app = create_router(state.clone());
    for uri in [
        "/api/feed",
        "/api/messages/123%40s.whatsapp.net",
        "/api/messages/family%40g.us",
    ] {
        assert_eq!(
            app.clone()
                .oneshot(empty_request(uri))
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
    }
    assert!(state.store.claim_translation().unwrap().is_none());
    assert_eq!(state.store.ready_notification_ids().unwrap().len(), 2);
    state
        .store
        .update_conversation_settings(
            "family@g.us",
            &ConversationSettings {
                translation_enabled: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(
        app.clone()
            .oneshot(empty_request("/api/feed"))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        state.store.claim_translation().unwrap().as_deref(),
        Some("group-message")
    );
    assert!(state.store.claim_translation().unwrap().is_none());
    state
        .store
        .update_conversation_settings("family@g.us", &ConversationSettings::default())
        .unwrap();
    assert_eq!(
        app.oneshot(empty_request("/api/messages/family%40g.us"))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert!(state.store.claim_translation().unwrap().is_none());
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn required_translation_without_a_translator_never_sends_english() {
    let (state, data_dir) = test_state(None);
    let contact_id = "33612345678@s.whatsapp.net";
    state
        .store
        .upsert_contact(
            contact_id,
            Some("French Contact"),
            Some("33612345678"),
            Some("private"),
            1_700_000_000_000,
        )
        .expect("insert contact");
    state
        .store
        .update_conversation_settings(
            contact_id,
            &ConversationSettings {
                reply_only: false,
                translation_enabled: true,
                language_override: Some("French".to_string()),
                translation_style: None,
                send_original_follow_up: false,
            },
        )
        .expect("settings");
    state.set_connected(true, None, None).await;

    let response = send_message(
        State(state.clone()),
        Json(SendMessageRequest {
            reply_only_if_not_latest: false,
            contact_id: contact_id.to_string(),
            text: "Hello".to_string(),
            reply_to: None,
            reply_to_sender: None,
            reply_to_text: None,
            reply_to_sender_name: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(state
        .store
        .get_messages(contact_id)
        .expect("messages")
        .is_empty());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert!(payload["error"]
        .as_str()
        .is_some_and(|error| error.contains("English message was not sent")));

    std::fs::remove_dir_all(data_dir).expect("remove test data");
}

#[tokio::test]
async fn captions_translate_and_manual_already_target_saves_detection() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/responses", listener.local_addr().unwrap());
    let mock = Router::new().route("/responses", post(|Json(body): Json<serde_json::Value>| async move {
        let instructions = body["instructions"].as_str().unwrap();
        let text = if instructions.contains("Detect the language") {
            r#"{"language":"English","isTargetLanguage":true}"#
        } else { "Jó reggelt" };
        // Outgoing detection targets Hungarian, so the English caption needs translation.
        let text = if instructions.contains("primarily in Hungarian") {
            r#"{"language":"English","isTargetLanguage":false}"#
        } else { text };
        Json(serde_json::json!({"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":text}]}]}))
    }));
    let server = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let (_, dir) = test_state(None);
    let state = AppState::new(
        MessageStore::new(&dir).unwrap(),
        PathBuf::from("../web/public"),
        dir.clone(),
        Some(Arc::new(TranslationService::new_with_api_url(url))),
        None,
        None,
    );
    state
        .store
        .upsert_contact("chat@example.test", None, None, None, 0)
        .unwrap();
    state
        .store
        .update_conversation_settings(
            "chat@example.test",
            &ConversationSettings {
                translation_enabled: true,
                language_override: Some("Hungarian".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let (caption, language) =
        translate_caption(&state, "chat@example.test", None, Some("Good morning"))
            .await
            .unwrap();
    assert_eq!(caption.as_deref(), Some("Jó reggelt"));
    assert_eq!(language.as_deref(), Some("Hungarian"));
    let mut message = test_outgoing_message("incoming", 1);
    message.is_from_me = false;
    message.original_text = Some("Good morning".into());
    state.store.add_message(&message).unwrap();
    state.store.enqueue_translation("incoming").unwrap();
    let mut events = state.broadcast_tx.subscribe();
    let response = translate_message(
        State(state.clone()),
        Json(TranslateMessageRequest {
            text: "client text is ignored".into(),
            message_id: "incoming".into(),
            contact_id: "chat@example.test".into(),
        }),
    )
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::OK);
    let saved = state.store.get_message_by_id("incoming").unwrap().unwrap();
    assert!(!saved.is_translated);
    assert_eq!(saved.source_language.as_deref(), Some("English"));
    assert!(state.store.claim_translation().unwrap().is_none());
    assert!(matches!(
        events.try_recv().unwrap(),
        WebSocketEvent::MessageUpdated { .. }
    ));
    server.abort();
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn outgoing_language_prefers_reply_then_override_and_rejects_other_chat() {
    let (state, dir) = test_state(None);
    state
        .store
        .upsert_contact("chat@example.test", None, None, None, 0)
        .unwrap();
    state
        .store
        .update_conversation_settings(
            "chat@example.test",
            &ConversationSettings {
                translation_enabled: true,
                ..Default::default()
            },
        )
        .unwrap();
    for n in 0..4 {
        let mut message = test_outgoing_message(&format!("hungarian-{n}"), n);
        message.is_from_me = false;
        message.source_language = Some("Hungarian".into());
        state.store.add_message(&message).unwrap();
    }
    let mut reply = test_outgoing_message("reply", 5);
    reply.is_from_me = false;
    reply.source_language = Some("French".into());
    state.store.add_message(&reply).unwrap();
    assert_eq!(
        crate::incoming::outgoing_language(&state, "chat@example.test", None)
            .await
            .unwrap()
            .as_deref(),
        Some("Hungarian")
    );
    assert_eq!(
        crate::incoming::outgoing_language(&state, "chat@example.test", Some("reply"))
            .await
            .unwrap()
            .as_deref(),
        Some("French")
    );
    state
        .store
        .update_conversation_settings(
            "chat@example.test",
            &ConversationSettings {
                translation_enabled: true,
                language_override: Some("German".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(
        crate::incoming::outgoing_language(&state, "chat@example.test", Some("reply"))
            .await
            .unwrap()
            .as_deref(),
        Some("German")
    );
    assert!(
        crate::incoming::outgoing_language(&state, "other@example.test", Some("reply"))
            .await
            .is_err()
    );
    let error = translate_caption(&state, "chat@example.test", None, Some("Good morning"))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("not sent"));
    std::fs::remove_dir_all(dir).unwrap();
}
