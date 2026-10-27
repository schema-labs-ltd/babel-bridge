use super::*;

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
