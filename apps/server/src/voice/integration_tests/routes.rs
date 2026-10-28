use super::*;

#[tokio::test]
async fn all_voice_routes_require_web_authentication() {
    let dir = TestDirectory::new();
    let state = state(&dir.0, None);
    for (method, url) in [
        ("POST", "/api/voice/prepare"),
        ("POST", "/api/voice/send"),
        ("POST", "/api/voice/translate/example"),
        ("GET", "/api/voice/settings/outgoing"),
        ("PUT", "/api/voice/settings/outgoing"),
    ] {
        let response = create_router(state.clone())
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(url)
                    .header("Content-Type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{url}");
    }
}
#[tokio::test]
async fn voice_preferences_and_duplicate_protection_survive_restart() {
    let dir = TestDirectory::new();
    let state = state(&dir.0, None);
    state
        .store
        .set_voice_setting("outgoing", "masculine")
        .unwrap();
    let note = note(uuid::Uuid::new_v4().to_string());
    state
        .store
        .save_voice_note(&note.id, &serde_json::to_string(&note).unwrap())
        .unwrap();
    assert!(state.store.claim_voice_send(&note.id).unwrap());
    let reopened = MessageStore::new(&dir.0).unwrap();
    assert_eq!(reopened.voice_setting("outgoing").unwrap(), "masculine");
    assert!(!reopened.claim_voice_send(&note.id).unwrap());
    assert!(json_response(
        send(
            State(state),
            Json(SendRequest {
                preparation_id: note.id
            })
        )
        .await
    )
    .await["error"]
        .as_str()
        .unwrap()
        .contains("awaiting confirmation"));
}
#[tokio::test]
async fn reviewed_voice_sends_translation_then_original_and_retries_do_not_resend() {
    let dir = TestDirectory::new();
    let state = state(&dir.0, None);
    *state.connected.write().await = true;
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    state.set_command_tx(tx).await;
    let note = note(uuid::Uuid::new_v4().to_string());
    state
        .store
        .save_voice_note(&note.id, &serde_json::to_string(&note).unwrap())
        .unwrap();
    let bridge_state = state.clone();
    let bridge = tokio::spawn(async move {
        for (index, expected) in ["OggS translated fixture", "OggS original fixture"]
            .iter()
            .enumerate()
        {
            let command = tokio::time::timeout(Duration::from_secs(3), rx.recv())
                .await
                .unwrap()
                .unwrap();
            if let BridgeCommand::SendAudio {
                request_id: Some(request_id),
                media_data,
                to,
                ..
            } = command
            {
                assert_eq!(to, "test@s.whatsapp.net");
                assert_eq!(B64.decode(media_data).unwrap(), expected.as_bytes());
                bridge_state
                    .handle_send_result(BridgeSendResult {
                        request_id,
                        success: true,
                        message_id: Some(format!("confirmed-{index}")),
                        timestamp: Some(1700000000 + index as i64),
                        message_ids: vec![],
                        timestamps: vec![],
                        error: None,
                    })
                    .await;
            } else {
                panic!("Expected an audio command");
            }
        }
        rx
    });
    let first = json_response(
        send(
            State(state.clone()),
            Json(SendRequest {
                preparation_id: note.id.clone(),
            }),
        )
        .await,
    )
    .await;
    assert_eq!(first["success"], true);
    assert_eq!(first["originalFollowUpSent"], true);
    let mut rx = bridge.await.unwrap();
    let again = json_response(
        send(
            State(state.clone()),
            Json(SendRequest {
                preparation_id: note.id.clone(),
            }),
        )
        .await,
    )
    .await;
    assert_eq!(first, again);
    assert!(rx.try_recv().is_err());
    assert_eq!(state.store.get_messages(&note.contact_id).unwrap().len(), 2);
    assert!(state
        .store
        .voice_note("sent-confirmed-0")
        .unwrap()
        .is_some());
}
#[tokio::test]
async fn unified_recording_rechecks_reply_at_send_time_and_persists_quote() {
    for (newer_arrived, reply_only) in [(false, false), (true, false), (false, true)] {
        let dir = TestDirectory::new();
        let state = state(&dir.0, None);
        *state.connected.write().await = true;
        let mut note = note(uuid::Uuid::new_v4().to_string());
        note.original_follow_up = false;
        // Create a stored chat message as the selected target before recording.
        store_audio_message(
            &state,
            &note,
            false,
            BridgeSendResult {
                request_id: 0,
                success: true,
                message_id: Some("selected".into()),
                timestamp: Some(100),
                message_ids: vec![],
                timestamps: vec![],
                error: None,
            },
            "sent",
        )
        .unwrap();
        if reply_only {
            let mut incoming = state.store.get_message_by_id("selected").unwrap().unwrap();
            incoming.is_from_me = false;
            state.store.delete_message("selected").unwrap();
            state.store.add_message(&incoming).unwrap();
            let mut settings = state
                .store
                .get_conversation_settings(&note.contact_id)
                .unwrap();
            settings.reply_only = true;
            state
                .store
                .update_conversation_settings(&note.contact_id, &settings)
                .unwrap();
        }
        note.reply_to = Some("selected".into());
        note.reply_to_sender = Some("sam@s.whatsapp.net".into());
        note.reply_to_text = Some("Original".into());
        note.reply_only_if_not_latest = true;
        state
            .store
            .save_voice_note(&note.id, &serde_json::to_string(&note).unwrap())
            .unwrap();
        if newer_arrived {
            store_audio_message(
                &state,
                &note,
                false,
                BridgeSendResult {
                    request_id: 0,
                    success: true,
                    message_id: Some("newer".into()),
                    timestamp: Some(200),
                    message_ids: vec![],
                    timestamps: vec![],
                    error: None,
                },
                "sent",
            )
            .unwrap();
        }
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        state.set_command_tx(tx).await;
        let bridge_state = state.clone();
        let preparation = note.id.clone();
        let bridge = tokio::spawn(async move {
            let cmd = tokio::time::timeout(Duration::from_secs(3), rx.recv())
                .await
                .unwrap()
                .unwrap();
            let BridgeCommand::SendAudio {
                request_id: Some(id),
                reply_to,
                reply_to_sender,
                to,
                ..
            } = cmd
            else {
                panic!("Expected audio")
            };
            assert_eq!(to, "test@s.whatsapp.net");
            assert_eq!(
                reply_to.as_deref(),
                if newer_arrived || reply_only {
                    Some("selected")
                } else {
                    None
                }
            );
            assert_eq!(reply_to_sender.is_some(), newer_arrived || reply_only);
            let (payload, status, _, _) = bridge_state
                .store
                .voice_note(&preparation)
                .unwrap()
                .unwrap();
            assert_eq!(status, "sending");
            let persisted: VoiceNote = serde_json::from_str(&payload).unwrap();
            assert_eq!(persisted.reply_to, reply_to);
            bridge_state
                .handle_send_result(BridgeSendResult {
                    request_id: id,
                    success: true,
                    message_id: Some("confirmed".into()),
                    timestamp: Some(1_700_000_000),
                    message_ids: vec![],
                    timestamps: vec![],
                    error: None,
                })
                .await;
        });
        let result = json_response(
            send(
                State(state.clone()),
                Json(SendRequest {
                    preparation_id: note.id,
                }),
            )
            .await,
        )
        .await;
        bridge.await.unwrap();
        assert_eq!(result["success"], true);
        let stored = state.store.get_message_by_id("confirmed").unwrap().unwrap();
        let content: Value = serde_json::from_str(&stored.content_json).unwrap();
        assert_eq!(
            content["reply_context"]["messageId"].as_str(),
            if newer_arrived || reply_only {
                Some("selected")
            } else {
                None
            }
        );
    }
}

#[tokio::test]
async fn incoming_assets_cannot_be_used_as_send_preparations() {
    let dir = TestDirectory::new();
    let state = state(&dir.0, None);
    let result = json_response(
        send(
            State(state),
            Json(SendRequest {
                preparation_id: "sent-known-message".into(),
            }),
        )
        .await,
    )
    .await;
    assert!(result["error"].as_str().unwrap().contains("cannot be sent"));
}
#[tokio::test]
async fn playback_decodes_whatsapp_opus_without_ai_and_preserves_long_notes() {
    let dir = TestDirectory::new();
    let source = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=240:duration=245",
            "-c:a",
            "libopus",
            "-f",
            "ogg",
            "pipe:1",
        ])
        .output()
        .await
        .expect("ffmpeg is required");
    assert!(source.status.success());
    let encoded = playable_audio(&B64.encode(source.stdout)).await.unwrap();
    tokio::fs::write(dir.0.join("playback.mp3"), B64.decode(encoded).unwrap())
        .await
        .unwrap();
    let probe = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(dir.0.join("playback.mp3"))
        .output()
        .await
        .unwrap();
    assert!(probe.status.success());
    let seconds: f64 = String::from_utf8(probe.stdout)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(
        (245.0..246.0).contains(&seconds),
        "full recording must survive: {seconds}"
    );
    assert!(playable_audio(&B64.encode([0_u8; 64])).await.is_err());
}
