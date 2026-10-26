use super::*;

#[tokio::test]
async fn disabled_translation_prepares_and_sends_original_voice_without_ai_or_translation_lock() {
    let tone = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=240:duration=1",
            "-f",
            "mp3",
            "pipe:1",
        ])
        .output()
        .await
        .expect("ffmpeg is required for voice tests");
    assert!(tone.status.success());
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let mock = Router::new().fallback(move || {
        let calls = observed.clone();
        async move {
            calls.fetch_add(1, Ordering::SeqCst);
            StatusCode::SERVICE_UNAVAILABLE
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    for configured in [false, true] {
        let dir = TestDirectory::new();
        let translator = configured.then(|| {
            Arc::new(TranslationService::new_with_api_url(format!(
                "http://{address}/v1/responses"
            )))
        });
        let state = state(&dir.0, translator);
        let contact_id = if configured {
            "family@g.us"
        } else {
            "test@s.whatsapp.net"
        };
        state
            .store
            .upsert_contact(
                contact_id,
                Some("Synthetic chat"),
                None,
                Some(if configured { "group" } else { "private" }),
                1,
            )
            .unwrap();
        state
            .store
            .update_conversation_settings(
                contact_id,
                &ConversationSettings {
                    reply_only: false,
                    translation_enabled: false,
                    language_override: Some("Hungarian".into()),
                    translation_style: Some("friendly".into()),
                    send_original_follow_up: true,
                },
            )
            .unwrap();
        // Even another chat's busy translation must not delay original recordings.
        let guard = state.voice_lock.lock().await;
        let response = tokio::time::timeout(
            Duration::from_secs(10),
            prepare(
                State(state.clone()),
                Json(PrepareRequest {
                    contact_id: contact_id.into(),
                    media_data: B64.encode(&tone.stdout),
                    reply_only_if_not_latest: false,
                    reply_to: None,
                    reply_to_sender: None,
                    reply_to_text: None,
                }),
            ),
        )
        .await
        .expect("original recording must not wait for AI lock");
        assert_eq!(response.status(), StatusCode::OK);
        let payload = json_response(response).await;
        assert_eq!(payload["isTranslated"], false);
        assert_eq!(payload["originalFollowUp"], false);
        assert_eq!(payload["transcript"], "");
        let id = payload["id"].as_str().unwrap().to_string();
        let stored_note = state.store.voice_note(&id).unwrap().unwrap();
        let note: VoiceNote = serde_json::from_str(&stored_note.0).unwrap();
        assert!(B64.decode(&note.send_audio).unwrap().starts_with(b"OggS"));
        assert!(playable_audio(&note.audio_data).await.is_ok());
        drop(guard);
        *state.connected.write().await = true;
        let (tx, mut rx) = tokio::sync::mpsc::channel(2);
        state.set_command_tx(tx).await;
        let bridge_state = state.clone();
        let destination = contact_id.to_string();
        let bridge = tokio::spawn(async move {
            let command = tokio::time::timeout(Duration::from_secs(3), rx.recv())
                .await
                .unwrap()
                .unwrap();
            let BridgeCommand::SendAudio {
                request_id: Some(request_id),
                to,
                ..
            } = command
            else {
                panic!("expected original audio");
            };
            assert_eq!(to, destination);
            bridge_state
                .handle_send_result(BridgeSendResult {
                    request_id,
                    success: true,
                    message_id: Some("original-voice".into()),
                    timestamp: Some(1700000000),
                    message_ids: vec![],
                    timestamps: vec![],
                    error: None,
                })
                .await;
            rx
        });
        let sent = json_response(
            send(
                State(state.clone()),
                Json(SendRequest { preparation_id: id }),
            )
            .await,
        )
        .await;
        assert_eq!(sent["success"], true);
        assert_eq!(sent["originalFollowUpSent"], false);
        assert!(bridge.await.unwrap().try_recv().is_err());
        let messages = state.store.get_messages(contact_id).unwrap();
        assert_eq!(messages.len(), 1);
        assert!(!messages[0].is_translated);
        assert!(messages[0].translated_text.is_none());
        assert!(messages[0].source_language.is_none());
        let content: Value = serde_json::from_str(&messages[0].content_json).unwrap();
        assert_eq!(content["aiGenerated"], false);
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    server.abort();
}

#[tokio::test]
async fn voice_pipeline_converts_audio_and_translates_into_default_language() {
    let dir = TestDirectory::new();
    // Exercise the same codec conversion used in production; no microphone or external service.
    let tone = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=240:duration=1",
            "-f",
            "mp3",
            "pipe:1",
        ])
        .output()
        .await
        .expect("ffmpeg is required for voice tests");
    assert!(tone.status.success());
    let audio = tone.stdout;
    let calls = Arc::new(AtomicUsize::new(0));
    let speech_calls = calls.clone();
    let mode = Arc::new(AtomicUsize::new(0));
    let transcription_mode = mode.clone();
    let translation_mode = mode.clone();
    let speech_mode = mode.clone();
    let output_audio = audio.clone();
    let mock = Router::new()
        .route("/v1/audio/transcriptions", post(move || {let mode = transcription_mode.clone(); async move {Json(json!({"text":if mode.load(Ordering::SeqCst) == 0 {"Jó reggelt"} else {"Good morning"}}))}}))
        .route("/v1/responses", post(move |Json(body): Json<Value>| {let mode = translation_mode.clone(); async move {
            let incoming = mode.load(Ordering::SeqCst) == 0;
            let text = if body["model"] == "test-detect" {
                if incoming {r#"{"language":"Hungarian","isTargetLanguage":false}"#} else {r#"{"language":"English","isTargetLanguage":false}"#}
            } else if incoming {"Good morning"} else {"Jó reggelt"};
            Json(json!({"output":[{"type":"message","content":[{"type":"output_text","text":text}]}],"usage":{"input_tokens":10,"output_tokens":3}}))
        }}))
        .route("/v1/audio/speech", post(move |Json(body): Json<Value>| {let audio = output_audio.clone(); let calls = speech_calls.clone(); let mode = speech_mode.clone(); async move {
            if mode.load(Ordering::SeqCst) == 2 { return StatusCode::SERVICE_UNAVAILABLE.into_response(); }
            assert_eq!(body["voice"], if mode.load(Ordering::SeqCst) == 0 {"shimmer"} else {"onyx"});
            assert_eq!(body["input"], if mode.load(Ordering::SeqCst) == 0 {"Good morning"} else {"Jó reggelt"});
            calls.fetch_add(1, Ordering::SeqCst);
            ([("content-type","audio/mpeg")],audio).into_response()
        }}));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let state = state(
        &dir.0,
        Some(Arc::new(TranslationService::new_with_api_url(format!(
            "http://{address}/v1/responses"
        )))),
    );
    state
        .store
        .set_voice_setting("test@s.whatsapp.net", "feminine")
        .unwrap();
    let content = crate::bridge::MessageContent::Audio {
        mime_type: "audio/mpeg".into(),
        file_size: audio.len() as u64,
        duration_seconds: Some(1),
        is_voice_note: true,
        media_data: Some(B64.encode(audio)),
    };
    let incoming = StoredMessage {
        id: "received-bridge-voice".into(),
        contact_id: "test@s.whatsapp.net".into(),
        timestamp: 1,
        is_from_me: false,
        is_forwarded: false,
        sender_name: None,
        sender_phone: None,
        contact_name: None,
        contact_phone: None,
        chat_type: "private".into(),
        content_type: content.type_name().into(),
        content_json: serde_json::to_string(&content).unwrap(),
        content: Some(serde_json::to_value(&content).unwrap()),
        original_text: None,
        translated_text: None,
        source_language: None,
        is_translated: false,
        delivery_status: None,
    };
    assert_eq!(incoming.content_type, "Voice Note");
    assert!(incoming.is_audio());
    for label in ["Voice Note", "Audio", "audio"] {
        let mut legacy = incoming.clone();
        legacy.content_type = label.into();
        assert!(legacy.is_audio());
    }
    let mut text = incoming.clone();
    text.content_type = "Text".into();
    text.content_json = r#"{"type":"text","body":"hello"}"#.into();
    assert!(!text.is_audio());
    state.store.add_message(&incoming).unwrap();
    // A manual click waits for automatic processing instead of rejecting a valid voice note.
    let guard = state.voice_lock.lock().await;
    let manual_state = state.clone();
    let manual = tokio::spawn(async move {
        translate_received(State(manual_state), Path("received-bridge-voice".into())).await
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(!manual.is_finished());
    drop(guard);
    let response = manual.await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let translated = cached_received(&state, &incoming.id).unwrap().unwrap();
    let again = translate_received(State(state.clone()), Path(incoming.id.clone())).await;
    assert_eq!(again.status(), StatusCode::OK);
    assert_eq!(translated.target_language, "English");
    assert_eq!(translated.source_language, "Hungarian");
    assert_eq!(translated.transcript, "Jó reggelt");
    assert_eq!(translated.translation, "Good morning");
    assert_eq!(translated.voice, "shimmer");
    assert!(B64.decode(&translated.audio_data).unwrap().len() > 100);
    assert!(!translated.original_follow_up);
    assert!(translated.send_audio.is_empty());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    mode.store(1, Ordering::SeqCst);
    state
        .store
        .set_voice_setting("outgoing", "masculine")
        .unwrap();
    let request = || PrepareRequest {
        contact_id: "test@s.whatsapp.net".into(),
        media_data: translated.original_data.clone(),
        reply_only_if_not_latest: false,
        reply_to: None,
        reply_to_sender: None,
        reply_to_text: None,
    };
    let outgoing = build_note(&state, request(), false, uuid::Uuid::new_v4().to_string())
        .await
        .unwrap();
    assert_eq!(outgoing.voice, "onyx");
    assert_eq!(outgoing.target_language, "Hungarian");
    assert!(B64
        .decode(&outgoing.send_audio)
        .unwrap()
        .starts_with(b"OggS"));
    assert!(B64
        .decode(&outgoing.send_original)
        .unwrap()
        .starts_with(b"OggS"));
    assert!(outgoing.original_follow_up);
    mode.store(2, Ordering::SeqCst);
    let (tx, mut commands) = tokio::sync::mpsc::channel(1);
    state.set_command_tx(tx).await;
    let response = json_response(prepare(State(state.clone()), Json(request())).await).await;
    assert!(response["error"].as_str().unwrap().contains("503"));
    assert!(
        commands.try_recv().is_err(),
        "A synthesis error must never send the original as fallback"
    );
    let stored = state.store.get_messages("test@s.whatsapp.net").unwrap();
    assert_eq!(stored.len(), 1, "preview must not send a message");
    assert_eq!(stored[0].id, incoming.id);
    assert!(stored[0].is_translated);

    assert_eq!(
        std::fs::read_dir(&dir.0)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with("voice-tmp-"))
            .count(),
        0
    );
    server.abort();
}
