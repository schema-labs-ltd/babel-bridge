//! Reply targeting, unified feed sends and attachment validation tests.

use super::*;

#[tokio::test]
async fn reply_only_blocks_text_media_and_bridge_bypasses_and_preserves_older_settings() {
    let (state, dir) = test_state(None);
    for contact in ["one@g.us", "two@g.us"] {
        state
            .store
            .upsert_contact(contact, None, None, Some("group"), 1)
            .unwrap();
    }
    for (id, contact, kind, mine) in [
        ("incoming", "one@g.us", "Text", false),
        ("mine", "one@g.us", "Text", true),
        ("other", "two@g.us", "Text", false),
        ("deleted", "one@g.us", "Revoked", false),
        ("reaction", "one@g.us", "Reaction", false),
    ] {
        let mut message = feed_test_message(id, contact, 100);
        message.is_from_me = mine;
        message.content_type = kind.into();
        state.store.add_message(&message).unwrap();
    }
    state
        .store
        .update_conversation_settings(
            "one@g.us",
            &ConversationSettings {
                reply_only: true,
                ..Default::default()
            },
        )
        .unwrap();
    state.set_connected(true, None, None).await;
    let (sender, mut receiver) = mpsc::channel(8);
    state.set_command_tx(sender).await;
    let app = create_router(state.clone());
    for reply in [
        None,
        Some("mine"),
        Some("other"),
        Some("missing"),
        Some("deleted"),
        Some("reaction"),
    ] {
        assert!(state.store.validate_reply_only("one@g.us", reply).is_err());
        for (path, payload) in [
            (
                "/api/send",
                serde_json::json!({"contactId":"one@g.us","text":"No","replyTo":reply}),
            ),
            (
                "/api/send-image",
                serde_json::json!({"contactId":"one@g.us","mediaData":BASE64_STANDARD.encode(b"photo"),"mimeType":"image/jpeg","replyTo":reply}),
            ),
            (
                "/api/send-images",
                serde_json::json!({"contactId":"one@g.us","images":[{"mediaData":BASE64_STANDARD.encode(b"photo"),"mimeType":"image/jpeg"}],"replyTo":reply}),
            ),
        ] {
            let response = app
                .clone()
                .oneshot(
                    HttpRequest::builder()
                        .method("POST")
                        .uri(path)
                        .header("content-type", "application/json")
                        .body(Body::from(payload.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{path}");
        }
    }
    assert!(receiver.try_recv().is_err());
    assert!(state
        .store
        .validate_reply_only("one@g.us", Some("incoming"))
        .unwrap());
    assert!(!state.store.validate_reply_only("two@g.us", None).unwrap());
    assert!(media_reply_needs_quote(&state, "one@g.us", Some("incoming"), true).unwrap());
    // Re-check at dispatch even if settings changed while a client was preparing media.
    assert!(state
        .send_bridge_command(BridgeCommand::SendAudio {
            request_id: None,
            to: "one@g.us".into(),
            media_data: "audio".into(),
            duration_seconds: 1,
            reply_to: None,
            reply_to_sender: None,
            reply_to_text: None
        })
        .await
        .unwrap_err()
        .contains("Reply only"));
    let response = app
        .clone()
        .oneshot(
            HttpRequest::builder()
                .method("PUT")
                .uri("/api/contacts/one%40g.us/settings")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"translationEnabled":false}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        state
            .store
            .get_conversation_settings("one@g.us")
            .unwrap()
            .reply_only
    );
    assert!(
        MessageStore::new(&dir)
            .unwrap()
            .get_conversation_settings("one@g.us")
            .unwrap()
            .reply_only
    );
    state
        .store
        .update_conversation_settings("one@g.us", &ConversationSettings::default())
        .unwrap();
    assert!(!state.store.validate_reply_only("one@g.us", None).unwrap());
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn unified_feed_rejects_missing_deleted_or_wrong_group_context_before_sending() {
    let (state, dir) = test_state(None);
    state
        .store
        .upsert_contact("one@g.us", None, None, Some("group"), 1)
        .unwrap();
    state
        .store
        .upsert_contact("two@g.us", None, None, Some("group"), 1)
        .unwrap();
    state
        .store
        .add_message(&feed_test_message("other", "two@g.us", 100))
        .unwrap();
    state
        .store
        .add_message(&feed_test_message("deleted", "one@g.us", 100))
        .unwrap();
    state.store.delete_message("deleted").unwrap();
    state.set_connected(true, None, None).await;
    let (sender, mut receiver) = mpsc::channel(8);
    state.set_command_tx(sender).await;
    let app = create_router(state);
    for selected in [None, Some("deleted"), Some("other")] {
        let request = HttpRequest::builder().method("POST").uri("/api/send")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({"contactId":"one@g.us", "text":"Reply", "replyTo":selected, "replyOnlyIfNotLatest":true}).to_string())).unwrap();
        assert_eq!(
            app.clone().oneshot(request).await.unwrap().status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert!(receiver.try_recv().is_err());
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn unified_feed_sends_to_selected_group_and_quotes_only_its_older_messages() {
    for (selected, expected_quote, reply_only) in [
        ("a", Some("a"), false),
        ("b", None, false),
        ("b", Some("b"), true),
    ] {
        let (state, dir) = test_state(None);
        for contact in ["one@g.us", "two@g.us"] {
            state
                .store
                .upsert_contact(contact, None, None, Some("group"), 1)
                .unwrap();
        }
        for (id, contact, time) in [
            ("a", "one@g.us", 100),
            ("b", "one@g.us", 200),
            ("c", "two@g.us", 300),
        ] {
            state
                .store
                .add_message(&feed_test_message(id, contact, time))
                .unwrap();
        }
        state
            .store
            .update_conversation_settings(
                "one@g.us",
                &ConversationSettings {
                    reply_only,
                    ..Default::default()
                },
            )
            .unwrap();
        state.set_connected(true, None, None).await;
        let (sender, mut receiver) = mpsc::channel(8);
        state.set_command_tx(sender).await;
        let request = HttpRequest::builder().method("POST").uri("/api/send")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({"contactId":"one@g.us", "text":"Reply", "replyTo":selected, "replyToSender":"sender@s.whatsapp.net", "replyToText":selected, "replyOnlyIfNotLatest":true}).to_string())).unwrap();
        let sending = tokio::spawn(create_router(state.clone()).oneshot(request));
        let command = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        let BridgeCommand::Send {
            request_id: Some(request_id),
            to,
            reply_to,
            reply_to_sender,
            ..
        } = command
        else {
            panic!("expected send");
        };
        assert_eq!(to, "one@g.us");
        assert_eq!(reply_to.as_deref(), expected_quote);
        assert_eq!(reply_to_sender.is_some(), expected_quote.is_some());
        state
            .handle_send_result(BridgeSendResult {
                request_id,
                success: true,
                message_id: Some("sent".into()),
                timestamp: Some(1_700_000_000),
                message_ids: vec![],
                timestamps: vec![],
                error: None,
            })
            .await;
        assert_eq!(sending.await.unwrap().unwrap().status(), StatusCode::OK);
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[tokio::test]
async fn unified_media_routes_to_captured_chat_and_conditionally_quotes() {
    for kind in ["image", "video", "document", "album"] {
        for (selected, quote) in [("a", Some("a")), ("b", None)] {
            let (state, dir) = test_state(None);
            for (id, contact, time) in [
                ("a", "one@g.us", 100),
                ("b", "one@g.us", 200),
                ("c", "two@g.us", 300),
            ] {
                state
                    .store
                    .upsert_contact(contact, None, None, Some("group"), time)
                    .unwrap();
                state
                    .store
                    .add_message(&feed_test_message(id, contact, time))
                    .unwrap();
            }
            state.set_connected(true, None, None).await;
            let (sender, mut receiver) = mpsc::channel(8);
            state.set_command_tx(sender).await;
            let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/p9sAAAAASUVORK5CYII=";
            let mut body = serde_json::json!({"contactId":"one@g.us", "replyTo":selected, "replyToSender":"sam@s.whatsapp.net", "replyToText":"Original", "replyOnlyIfNotLatest":true});
            let path = if kind == "album" {
                body["images"] = serde_json::json!([{"mediaData":png,"mimeType":"image/png"},{"mediaData":png,"mimeType":"image/png"}]);
                "/api/send-images"
            } else {
                body["mediaKind"] = serde_json::json!(kind);
                body["fileName"] = serde_json::json!("Fixture.txt");
                body["mimeType"] = serde_json::json!(match kind {
                    "image" => "image/png",
                    "video" => "video/mp4",
                    _ => "text/plain",
                });
                body["mediaData"] = serde_json::json!(match kind {
                    "image" => png.to_string(),
                    "video" => BASE64_STANDARD.encode(b"\x00\x00\x00\x18ftypisomfixture"),
                    _ => BASE64_STANDARD.encode(b"Test document"),
                });
                if kind == "image" {
                    "/api/send-image"
                } else {
                    "/api/send-media"
                }
            };
            let request = HttpRequest::builder()
                .method("POST")
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap();
            let sending = tokio::spawn(create_router(state.clone()).oneshot(request));
            let command = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
                .await
                .unwrap()
                .unwrap();
            let (id, to, reply, sender) = match command {
                BridgeCommand::SendImage {
                    request_id: Some(id),
                    to,
                    reply_to,
                    reply_to_sender,
                    ..
                }
                | BridgeCommand::SendMedia {
                    request_id: Some(id),
                    to,
                    reply_to,
                    reply_to_sender,
                    ..
                }
                | BridgeCommand::SendImages {
                    request_id: Some(id),
                    to,
                    reply_to,
                    reply_to_sender,
                    ..
                } => (id, to, reply_to, reply_to_sender),
                _ => panic!("Expected a media send"),
            };
            assert_eq!(to, "one@g.us");
            assert_eq!(reply.as_deref(), quote, "{kind}");
            assert_eq!(sender.is_some(), quote.is_some());
            state
                .handle_send_result(BridgeSendResult {
                    request_id: id,
                    success: true,
                    message_id: Some("sent".into()),
                    timestamp: Some(1_700_000_000),
                    message_ids: if kind == "album" {
                        vec!["sent".into(), "sent2".into()]
                    } else {
                        vec![]
                    },
                    timestamps: if kind == "album" {
                        vec![1_700_000_000, 1_700_000_001]
                    } else {
                        vec![]
                    },
                    error: None,
                })
                .await;
            assert_eq!(sending.await.unwrap().unwrap().status(), StatusCode::OK);
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

#[tokio::test]
async fn unified_album_retains_reply_while_photos_transfer() {
    let (state, dir) = test_state(None);
    state
        .store
        .upsert_contact("one@g.us", None, None, Some("group"), 100)
        .unwrap();
    state
        .store
        .add_message(&feed_test_message("selected", "one@g.us", 100))
        .unwrap();
    state.set_connected(true, None, None).await;
    let app = create_router(state.clone());
    let post = |path: &str, body: serde_json::Value| {
        HttpRequest::builder()
            .method("POST")
            .uri(path)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    };
    let response = app.clone().oneshot(post("/api/photo-albums", serde_json::json!({"jobId":"staged-reply","contactId":"one@g.us","photoCount":2,"replyTo":"selected","replyToSender":"sam@s.whatsapp.net","replyOnlyIfNotLatest":true}))).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/p9sAAAAASUVORK5CYII=";
    for index in 0..2 {
        let request = HttpRequest::builder()
            .method("PUT")
            .uri(format!("/api/photo-albums/staged-reply/images/{index}"))
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"mediaData":png,"mimeType":"image/png"}).to_string(),
            ))
            .unwrap();
        assert_eq!(
            app.clone().oneshot(request).await.unwrap().status(),
            StatusCode::OK
        );
    }
    // A message arrives after selection and while the album is being prepared.
    state
        .store
        .add_message(&feed_test_message("newer", "one@g.us", 200))
        .unwrap();
    let (sender, mut receiver) = mpsc::channel(4);
    state.set_command_tx(sender).await;
    let sending = tokio::spawn(app.oneshot(post(
        "/api/photo-albums/staged-reply/send",
        serde_json::json!({}),
    )));
    let command = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    let BridgeCommand::SendImages {
        request_id: Some(id),
        to,
        reply_to,
        ..
    } = command
    else {
        panic!("Expected album")
    };
    assert_eq!(to, "one@g.us");
    assert_eq!(reply_to.as_deref(), Some("selected"));
    state
        .handle_send_result(BridgeSendResult {
            request_id: id,
            success: true,
            message_id: Some("album".into()),
            timestamp: Some(1_700_000_000),
            message_ids: vec!["photo1".into(), "photo2".into()],
            timestamps: vec![1_700_000_000, 1_700_000_001],
            error: None,
        })
        .await;
    assert_eq!(sending.await.unwrap().unwrap().status(), StatusCode::OK);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn unified_media_rejects_missing_and_wrong_chat_targets() {
    let (state, dir) = test_state(None);
    state
        .store
        .upsert_contact("two@g.us", None, None, Some("group"), 100)
        .unwrap();
    state
        .store
        .add_message(&feed_test_message("other", "two@g.us", 100))
        .unwrap();
    state.set_connected(true, None, None).await;
    let (sender, mut receiver) = mpsc::channel(4);
    state.set_command_tx(sender).await;
    for selected in [None, Some("other"), Some("deleted")] {
        let body = serde_json::json!({"contactId":"one@g.us","replyTo":selected,"replyOnlyIfNotLatest":true,"mediaKind":"document","fileName":"Note.txt","mimeType":"text/plain","mediaData":BASE64_STANDARD.encode(b"hello")});
        let request = HttpRequest::builder()
            .method("POST")
            .uri("/api/send-media")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        assert_eq!(
            create_router(state.clone())
                .oneshot(request)
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
        assert!(receiver.try_recv().is_err());
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn attachment_validation_rejects_invalid_kind_video_and_filename() {
    let data = BASE64_STANDARD.encode(b"file");
    assert!(validate_attachment_payload(&data, "text/plain", "document", Some("Note.txt")).is_ok());
    assert!(validate_attachment_payload(&data, "video/mp4", "video", None).is_err());
    assert!(
        validate_attachment_payload(&data, "text/plain", "document", Some("../Note.txt")).is_err()
    );
    assert!(validate_attachment_payload(&data, "text/plain", "document", None).is_err());
    assert!(validate_attachment_payload(&data, "text/plain", "unknown", None).is_err());
    assert!(validate_attachment_payload("", "text/plain", "document", Some("Note.txt")).is_err());
    assert!(
        validate_attachment_payload(&data, "bad\nvalue", "document", Some("Note.txt")).is_err()
    );
}
