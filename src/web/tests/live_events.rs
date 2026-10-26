//! Reaction, edit and live event tests, plus session sidecar cleanup.

use super::*;

#[test]
fn reaction_push_notifications_require_a_message_sent_by_the_user() {
    let outgoing_target = test_outgoing_message("outgoing", 1_700_000_000_000);
    let mut incoming_target = test_outgoing_message("incoming", 1_700_000_000_001);
    incoming_target.is_from_me = false;
    let mut reaction = test_outgoing_message("reaction", 1_700_000_000_002);
    reaction.is_from_me = false;
    reaction.content_type = "Reaction".to_string();
    reaction.content = Some(serde_json::json!({
        "type": "reaction",
        "emoji": "❤️",
        "target_message_id": "outgoing",
    }));

    assert!(should_send_push_notification(
        &reaction,
        Some(&outgoing_target)
    ));
    assert!(!should_send_push_notification(
        &reaction,
        Some(&incoming_target)
    ));
    assert!(!should_send_push_notification(&reaction, None));
    assert!(should_send_push_notification(&incoming_target, None));
}

#[tokio::test]
async fn edited_wire_messages_replace_both_snapshots_without_duplicates_or_unreads() {
    for (chat, from_me) in [
        ("447700900123@s.whatsapp.net", false),
        ("family@g.us", false),
        ("family@g.us", true),
    ] {
        let (state, dir) = test_state(None);
        let wire = |kind: &str, body: &str, revision: i64| {
            serde_json::json!({
                "type":kind,"id":"original","timestamp":1700000000,"edited_at_ms":revision,
                "from":{"jid":"447700900123@s.whatsapp.net","phone":"447700900123"},
                "chat":{"type":if chat.ends_with("@g.us") {"group"} else {"private"},"jid":chat,"name":"Test chat"},
                "content":{"type":"text","body":body},"is_from_me":from_me,"is_forwarded":false
            })
        };
        crate::handle_web_event(
            serde_json::from_value(wire("message", "Get", 0)).unwrap(),
            &state,
            &state.store,
            None,
        )
        .await
        .unwrap();
        let before = state.store.get_message_by_id("original").unwrap().unwrap();
        let unread = state.store.get_contacts().unwrap()[0].unread_count;
        state
            .store
            .update_conversation_settings(
                chat,
                &ConversationSettings {
                    translation_enabled: true,
                    ..Default::default()
                },
            )
            .unwrap();
        state
            .store
            .finish_translation(
                "original",
                Some("Old translation"),
                "Hungarian",
                true,
                Some("Get"),
                0,
            )
            .unwrap();
        let mut events = state.broadcast_tx.subscribe();
        crate::handle_web_event(
            serde_json::from_value(wire("message_edit", "Grr", 1700000000123)).unwrap(),
            &state,
            &state.store,
            None,
        )
        .await
        .unwrap();
        let WebSocketEvent::MessageUpdated { message } = events.try_recv().unwrap() else {
            panic!("edit must update the original")
        };
        assert_eq!(message.id, "original");
        assert_eq!(message.timestamp, before.timestamp);
        assert_eq!(message.original_text.as_deref(), Some("Grr"));
        assert_eq!(message.translated_text, None);
        assert_eq!(message.edit_revision(), 1700000000123);
        assert_eq!(state.store.get_contacts().unwrap()[0].unread_count, unread);
        // A stale AI response cannot restore the translation of Get or remove the new job.
        assert!(!state
            .store
            .finish_translation("original", Some("Stale"), "Hungarian", true, Some("Get"), 0)
            .unwrap());
        assert_eq!(state.store.claim_translation().unwrap().is_some(), !from_me);
        for (text, revision) in [("Get", 1700000000122), ("Grr", 1700000000123)] {
            crate::handle_web_event(
                serde_json::from_value(wire("message_edit", text, revision)).unwrap(),
                &state,
                &state.store,
                None,
            )
            .await
            .unwrap();
            assert!(events.try_recv().is_err());
        }
        let router = create_router(state.clone());
        for route in [
            "/api/feed?limit=10".to_string(),
            format!("/api/messages/{}?limit=10", urlencoding::encode(chat)),
        ] {
            let response = router.clone().oneshot(empty_request(&route)).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body: serde_json::Value = serde_json::from_slice(
                &axum::body::to_bytes(response.into_body(), 100_000)
                    .await
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(body["messages"].as_array().unwrap().len(), 1);
            assert_eq!(body["messages"][0]["content"]["body"], "Grr");
            assert_eq!(body["messages"][0]["timestamp"], before.timestamp);
        }
        let reopened = MessageStore::new(&dir).unwrap();
        assert_eq!(
            reopened
                .get_message_by_id("original")
                .unwrap()
                .unwrap()
                .original_text
                .as_deref(),
            Some("Grr")
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[tokio::test]
async fn edits_arriving_before_history_survive_restart_and_caption_edits_keep_media() {
    let (state, dir) = test_state(None);
    let chat = "family@g.us";
    state
        .store
        .upsert_contact(chat, Some("Family"), None, Some("group"), 100)
        .unwrap();
    let mut original = feed_test_message("photo", chat, 100);
    original.content_type = "Image".into();
    original.content_json = serde_json::json!({"type":"image","caption":"Get","media_data":"photo-data","album_id":"album","reply_context":{"message_id":"quoted"}}).to_string();
    original.original_text = Some("Get".into());
    let mut edit = original.clone();
    edit.content_json = serde_json::json!({"type":"image","caption":"Grr"}).to_string();
    edit.original_text = Some("Grr".into());
    assert!(state
        .store
        .record_message_edit(&edit, 200)
        .unwrap()
        .is_none());
    let reopened = MessageStore::new(&dir).unwrap();
    reopened.add_message(&original).unwrap();
    let saved = reopened.get_message_by_id("photo").unwrap().unwrap();
    let content = saved.content.unwrap();
    assert_eq!(content["caption"], "Grr");
    assert_eq!(content["media_data"], "photo-data");
    assert_eq!(content["album_id"], "album");
    assert_eq!(content["reply_context"]["message_id"], "quoted");
    assert_eq!(saved.timestamp, 100);
    // Caption removal is a valid edit and must not remove the attachment.
    edit.content_json = serde_json::json!({"type":"image"}).to_string();
    edit.original_text = None;
    let removed = reopened.record_message_edit(&edit, 201).unwrap().unwrap();
    assert!(removed.content.as_ref().unwrap()["caption"].is_null());
    assert_eq!(
        removed.content.as_ref().unwrap()["media_data"],
        "photo-data"
    );
    edit.contact_id = "another@g.us".into();
    assert!(reopened.record_message_edit(&edit, 300).unwrap().is_none());
    assert_eq!(
        reopened
            .get_message_by_id("photo")
            .unwrap()
            .unwrap()
            .edit_revision(),
        201
    );
    reopened.clear_all().unwrap();
    reopened
        .upsert_contact(chat, Some("Family"), None, Some("group"), 100)
        .unwrap();
    reopened.add_message(&original).unwrap();
    assert_eq!(
        reopened
            .get_message_by_id("photo")
            .unwrap()
            .unwrap()
            .edit_revision(),
        0
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn confirmed_reaction_wire_events_reach_ack_live_stream_and_both_snapshots() {
    let (state, dir) = test_state(None);
    let chat = "family@g.us";
    state
        .store
        .upsert_contact(chat, Some("Family"), None, Some("group"), 100)
        .unwrap();
    state
        .store
        .add_message(&feed_test_message("target", chat, 100))
        .unwrap();
    state.set_connected(true, None, None).await;
    let (sender, mut commands) = mpsc::channel(8);
    state.set_command_tx(sender).await;
    let router = create_router(state.clone());
    let mut events = state.broadcast_tx.subscribe();
    for (index, emoji) in ["❤️", "👍", ""].into_iter().enumerate() {
        let request = HttpRequest::builder()
            .method("POST")
            .uri("/api/react")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"contactId":chat,"messageId":"target","emoji":emoji})
                    .to_string(),
            ))
            .unwrap();
        let sending = tokio::spawn(router.clone().oneshot(request));
        let BridgeCommand::SendReaction {
            request_id: Some(request_id),
            to,
            ..
        } = tokio::time::timeout(Duration::from_secs(2), commands.recv())
            .await
            .unwrap()
            .unwrap()
        else {
            panic!("expected reaction send")
        };
        assert_eq!(to, chat);
        let id = format!("reaction-{}", 3 - index);
        let timestamp = 1_700_000_000_100 + index as i64;
        let mut content = serde_json::json!({"type":"reaction","target_message_id":"target","sender_timestamp_ms":timestamp});
        // Go's omitempty removes the field entirely for a removal.
        if !emoji.is_empty() {
            content["emoji"] = emoji.into();
        }
        let wire = serde_json::json!({"type":"message","id":id,"timestamp":1700000000,
            "from":{"jid":"447700900123@s.whatsapp.net","phone":"447700900123"},
            "chat":{"type":"group","jid":chat,"name":"Family"},"content":content,"is_from_me":true,"is_forwarded":false});
        let event: crate::bridge::BridgeEvent = serde_json::from_value(wire).unwrap();
        crate::handle_web_event(event, &state, &state.store, None)
            .await
            .unwrap();
        let mut seen = false;
        while let Ok(event) = events.try_recv() {
            if let WebSocketEvent::Reaction { message } = event {
                assert_eq!(message.id, id);
                assert_eq!(message.timestamp, timestamp);
                assert_eq!(message.content.unwrap()["emoji"], emoji);
                seen = true;
            }
        }
        assert!(
            seen,
            "confirmed reaction must publish before the acknowledgement"
        );
        state
            .handle_send_result(BridgeSendResult {
                request_id,
                success: true,
                message_id: Some(id.clone()),
                timestamp: Some(1700000000),
                message_ids: vec![],
                timestamps: vec![],
                error: None,
            })
            .await;
        let response = sending.await.unwrap().unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), 100_000)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(body["reaction"]["id"], id);
        assert_eq!(body["reaction"]["timestamp"], timestamp);
        for route in [
            "/api/feed?limit=1".to_string(),
            format!(
                "/api/messages/{}?limit=1&before=200",
                urlencoding::encode(chat)
            ),
        ] {
            let response = router.clone().oneshot(empty_request(&route)).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body: serde_json::Value = serde_json::from_slice(
                &axum::body::to_bytes(response.into_body(), 100_000)
                    .await
                    .unwrap(),
            )
            .unwrap();
            let message = &body["messages"][0];
            assert_eq!(message["id"], "target");
            assert_eq!(message["reactionStates"]["me"]["emoji"], emoji);
            assert_eq!(message["reactionStates"]["me"]["timestamp"], timestamp);
            if emoji.is_empty() {
                assert_eq!(message["reactions"], serde_json::json!({}));
            } else {
                assert_eq!(message["reactions"][emoji], serde_json::json!(["me"]));
            }
        }
    }
    drop(router);
    drop(state);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn reaction_messages_use_a_dedicated_live_event() {
    let mut reaction = test_outgoing_message("reaction", 1_700_000_000_002);
    reaction.content_type = "Reaction".to_string();
    reaction.content = Some(serde_json::json!({
        "type": "reaction",
        "emoji": "❤️",
        "target_message_id": "outgoing",
    }));

    let event = live_event_for_message(reaction);
    assert!(matches!(event, WebSocketEvent::Reaction { .. }));
    assert_eq!(
        serde_json::to_value(event).expect("serialize event")["type"],
        "reaction"
    );
}

#[test]
fn outgoing_send_plan_puts_translation_before_original_follow_up() {
    assert_eq!(
        build_outgoing_send_plan("Hola a todos".to_string(), "Hello everyone", true, true,),
        vec!["Hola a todos", "Hello everyone"]
    );
    assert_eq!(
        build_outgoing_send_plan("Hello everyone".to_string(), "Hello everyone", false, true,),
        vec!["Hello everyone"]
    );
}

#[test]
fn clear_orphaned_session_sidecars_removes_sidecars_without_session_db() {
    let (state, data_dir) = test_state(None);
    std::fs::write(data_dir.join("session.db-wal"), b"wal").expect("write wal");
    std::fs::write(data_dir.join("session.db-shm"), b"shm").expect("write shm");

    state
        .clear_orphaned_session_sidecars()
        .expect("clear sidecars");

    assert!(!data_dir.join("session.db-wal").exists());
    assert!(!data_dir.join("session.db-shm").exists());

    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn clear_orphaned_session_sidecars_keeps_sidecars_with_session_db() {
    let (state, data_dir) = test_state(None);
    std::fs::write(data_dir.join("session.db"), b"session").expect("write session");
    std::fs::write(data_dir.join("session.db-wal"), b"wal").expect("write wal");
    std::fs::write(data_dir.join("session.db-shm"), b"shm").expect("write shm");

    state
        .clear_orphaned_session_sidecars()
        .expect("clear sidecars");

    assert!(data_dir.join("session.db").exists());
    assert!(data_dir.join("session.db-wal").exists());
    assert!(data_dir.join("session.db-shm").exists());

    let _ = std::fs::remove_dir_all(data_dir);
}
