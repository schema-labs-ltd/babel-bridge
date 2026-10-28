use super::*;

#[tokio::test]
async fn whatsapp_session_loss_relinks_without_erasing_saved_chats() {
    let path = std::env::temp_dir().join(format!("connection-recovery-{}", uuid::Uuid::new_v4()));
    let store = MessageStore::new(&path).unwrap();
    store
        .upsert_contact(
            "test@s.whatsapp.net",
            Some("Preview"),
            None,
            Some("private"),
            100,
        )
        .unwrap();
    let state = AppState::new(store.clone(), path.clone(), path.clone(), None, None, None);
    let (tx, mut rx) = mpsc::channel(10);
    state.set_command_tx(tx).await;
    state
        .set_connected(true, Some("447700900123".into()), Some("Preview".into()))
        .await;
    handle_web_event(
        BridgeEvent::ConnectionState {
            state: ConnectionState::Reconnecting,
        },
        &state,
        &store,
        None,
    )
    .await
    .unwrap();
    assert!(!*state.connected.read().await);
    assert_eq!(
        *state.connection_state.read().await,
        web::WhatsAppConnectionState::Reconnecting
    );
    assert!(!state.take_session_reset_request());
    assert!(rx.try_recv().is_err());

    handle_web_event(
        BridgeEvent::LoggedOut {
            reason: "device removed".into(),
        },
        &state,
        &store,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        *state.connection_state.read().await,
        web::WhatsAppConnectionState::LinkingRequired
    );
    assert!(state.take_session_reset_request());
    assert!(matches!(
        rx.recv().await.unwrap(),
        bridge::BridgeCommand::Disconnect
    ));
    assert!(store.get_contact("test@s.whatsapp.net").unwrap().is_some());
    handle_web_event(
        BridgeEvent::Qr {
            data: "fresh-test-code".into(),
        },
        &state,
        &store,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        state.qr_code.read().await.as_deref(),
        Some("fresh-test-code")
    );
    handle_web_event(
        BridgeEvent::Connected {
            phone: "447700900123".into(),
            name: "Preview".into(),
            platform: None,
        },
        &state,
        &store,
        None,
    )
    .await
    .unwrap();
    assert!(*state.connected.read().await);
    assert!(state.qr_code.read().await.is_none());
    assert_eq!(
        *state.connection_state.read().await,
        web::WhatsAppConnectionState::Connected
    );
    let _ = std::fs::remove_dir_all(path);
}

#[tokio::test]
async fn reconnecting_status_lasts_until_a_permanent_disconnect() {
    let path = std::env::temp_dir().join(format!("connection-backoff-{}", uuid::Uuid::new_v4()));
    let store = MessageStore::new(&path).unwrap();
    let state = AppState::new(store.clone(), path.clone(), path.clone(), None, None, None);
    handle_web_event(
        BridgeEvent::ConnectionState {
            state: ConnectionState::Reconnecting,
        },
        &state,
        &store,
        None,
    )
    .await
    .unwrap();
    assert!(!*state.connected.read().await);
    assert_eq!(state.connection_state.read().await.label(), "reconnecting");
    handle_web_event(
        BridgeEvent::ConnectionState {
            state: ConnectionState::Disconnected,
        },
        &state,
        &store,
        None,
    )
    .await
    .unwrap();
    assert!(!*state.connected.read().await);
    assert_eq!(state.connection_state.read().await.label(), "disconnected");
    assert!(!state.take_session_reset_request());
    let _ = std::fs::remove_dir_all(path);
}

#[tokio::test]
async fn replaced_whatsapp_connection_restarts_without_resetting_session() {
    let path = std::env::temp_dir().join(format!("connection-replaced-{}", uuid::Uuid::new_v4()));
    let store = MessageStore::new(&path).unwrap();
    let state = AppState::new(store.clone(), path.clone(), path.clone(), None, None, None);
    let (tx, mut rx) = mpsc::channel(10);
    state.set_command_tx(tx).await;
    handle_web_event(
        BridgeEvent::Error {
            code: "reconnect_required".into(),
            message: "replaced".into(),
        },
        &state,
        &store,
        None,
    )
    .await
    .unwrap();
    assert!(matches!(
        rx.recv().await.unwrap(),
        bridge::BridgeCommand::Disconnect
    ));
    assert!(!state.take_session_reset_request());
    let _ = std::fs::remove_dir_all(path);
}

#[test]
fn image_and_sticker_dimensions_reach_clients() {
    for (wire, kind) in [
        (
            serde_json::json!({"type":"image","mime_type":"image/jpeg","file_size":10,"width":1179,"height":2556}),
            "image",
        ),
        (
            serde_json::json!({"type":"sticker","mime_type":"image/webp","is_animated":false,"width":512,"height":512}),
            "sticker",
        ),
    ] {
        let content: MessageContent = serde_json::from_value(wire.clone()).unwrap();
        let payload = serde_json::to_value(&content).unwrap();
        assert_eq!(payload["type"], kind);
        assert_eq!(payload["width"], wire["width"]);
        assert_eq!(payload["height"], wire["height"]);
    }
    let zero_sized: MessageContent = serde_json::from_value(
        serde_json::json!({"type":"image","mime_type":"image/jpeg","file_size":10,"width":0,"height":0}),
    )
    .unwrap();
    let payload = serde_json::to_value(&zero_sized).unwrap();
    assert!(payload.get("width").is_none() && payload.get("height").is_none());
}

#[tokio::test]
async fn incoming_embedded_quotes_survive_storage_and_restore_missing_metadata_only() {
    let path = std::env::temp_dir().join(format!("reply-quotes-{}", uuid::Uuid::new_v4()));
    let store = MessageStore::new(&path).unwrap();
    for chat in ["family@g.us", "friends@g.us"] {
        store
            .upsert_contact(chat, Some("Test chat"), None, Some("group"), 100)
            .unwrap();
    }
    for history in [false, true] {
        for content in [
            serde_json::json!({"type":"text","body":"Thanks!"}),
            serde_json::json!({"type":"image","mime_type":"image/jpeg","file_size":10}),
        ] {
            let id = format!("reply-{history}-{}", content["type"]);
            let wire = serde_json::json!({"id":id,"timestamp":1_700_000_000,
                "from":{"jid":"447700900123@s.whatsapp.net","phone":"447700900123"},
                "chat":{"type":"group","jid":"family@g.us","name":"Family"},
                "content":content,"is_from_me":false,"is_forwarded":false,"is_history":history,
                "reply_context":{"messageId":"outside-history","senderName":"Alex","text":"Yep I can"}});
            let stored = process_message(serde_json::from_value(wire).unwrap(), None, None).await;
            assert_eq!(
                stored.content.as_ref().unwrap()["reply_context"]["text"],
                "Yep I can"
            );
            let mut old = stored.clone();
            old.content_json = r#"{"type":"text","body":"Edited thanks","edited_at_ms":200,"media_data":"retained"}"#.into();
            old.translated_text = Some("Translation retained".into());
            store.add_message(&old).unwrap();
            store.recover_reply_context(&stored).unwrap();
            let recovered = store.get_message_by_id(&id).unwrap().unwrap();
            let payload: serde_json::Value = serde_json::from_str(&recovered.content_json).unwrap();
            assert_eq!(payload["reply_context"]["messageId"], "outside-history");
            assert_eq!(payload["body"], "Edited thanks");
            assert_eq!(payload["edited_at_ms"], 200);
            assert_eq!(payload["media_data"], "retained");
            assert_eq!(
                recovered.translated_text.as_deref(),
                Some("Translation retained")
            );
            let mut other = stored.clone();
            other.content_json = r#"{"reply_context":{"text":"Must not replace"}}"#.into();
            store.recover_reply_context(&other).unwrap();
            assert_eq!(
                store
                    .get_message_by_id(&id)
                    .unwrap()
                    .unwrap()
                    .content
                    .unwrap()["reply_context"]["text"],
                "Yep I can"
            );
        }
    }
    assert!(store
        .get_message_by_id("outside-history")
        .unwrap()
        .is_none());
    let mut original = store
        .get_message_by_id("reply-false-\"text\"")
        .unwrap()
        .unwrap();
    original.id = "different-chat".into();
    original.contact_id = "friends@g.us".into();
    original.content_json = r#"{"type":"text","body":"Hello"}"#.into();
    store.add_message(&original).unwrap();
    let mut spoof = original.clone();
    spoof.contact_id = "family@g.us".into();
    spoof.content_json = r#"{"reply_context":{"text":"Wrong chat"}}"#.into();
    store.recover_reply_context(&spoof).unwrap();
    spoof.contact_id = original.contact_id.clone();
    spoof.sender_phone = Some("447700900456".into());
    store.recover_reply_context(&spoof).unwrap();
    assert!(store
        .get_message_by_id("different-chat")
        .unwrap()
        .unwrap()
        .content
        .unwrap()
        .get("reply_context")
        .is_none());
    let mut edit = original.clone();
    edit.content_json = r#"{"type":"text","body":"Edited hello","reply_context":{"messageId":"outside-history","senderName":"Alex","text":"Yep I can"}}"#.into();
    let updated = store.record_message_edit(&edit, 500).unwrap().unwrap();
    assert_eq!(
        updated.content.unwrap()["reply_context"]["text"],
        "Yep I can"
    );
    drop(store);
    std::fs::remove_dir_all(path).unwrap();
}

#[tokio::test]
async fn device_identity_live_and_history_messages_use_account_contact_and_phone() {
    for device in [17, 22] {
        for is_from_me in [true, false] {
            for is_history in [true, false] {
                let msg: Message = serde_json::from_value(serde_json::json!({
                    "id":"fixture", "timestamp":1_700_000_000,
                    "from":{"jid":"447700900123@s.whatsapp.net","phone":"447700900123"},
                    "chat":{"type":"private","jid":format!("447700900123:{device}@s.whatsapp.net"),"name":"Friend"},
                    "content":{"type":"text","body":"Hello"},
                    "is_from_me":is_from_me,"is_history":is_history,"is_forwarded":false
                })).unwrap();
                let stored = process_message(msg, None, None).await;
                assert_eq!(stored.contact_id, "447700900123@s.whatsapp.net");
                assert_eq!(stored.contact_phone.as_deref(), Some("447700900123"));
            }
        }
    }
}
