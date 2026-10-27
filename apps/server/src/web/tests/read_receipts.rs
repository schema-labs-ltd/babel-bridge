//! Read receipt and mark-as-read tests.

use super::*;

#[tokio::test]
async fn viewed_read_receipts_validate_exact_ids_preserve_unseen_and_retry_after_restart() {
    let (state, dir) = test_state(None);
    for chat in ["family@g.us", "other@g.us"] {
        state
            .store
            .upsert_contact(chat, None, None, Some("group"), 0)
            .unwrap();
    }
    for (id, chat, sender) in [
        ("visible", "family@g.us", "111@lid"),
        ("offscreen", "family@g.us", "222@s.whatsapp.net"),
        ("elsewhere", "other@g.us", "333@lid"),
    ] {
        let mut msg = feed_test_message(id, chat, 100);
        msg.content_json =
            serde_json::json!({"type":"text", "body":id, "receipt_sender_jid":sender}).to_string();
        state
            .store
            .add_message_with_notification(&msg, Some(false))
            .unwrap();
    }
    let mut own = feed_test_message("own", "family@g.us", 100);
    own.is_from_me = true;
    state.store.add_message(&own).unwrap();
    state
        .store
        .update_message_translation("visible", Some("Translated"), Some("French"))
        .unwrap();
    assert!(
        state.store.pending_read_receipts(1000).unwrap().is_empty(),
        "Translation and receipt of a message never mark it seen"
    );
    let app = create_router(state.clone());
    for ids in [
        serde_json::json!(["visible", "elsewhere"]),
        serde_json::json!(["own"]),
        serde_json::json!(["missing"]),
    ] {
        let request = HttpRequest::builder()
            .method("POST")
            .uri("/api/contacts/family%40g.us/read")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"messageIds":ids}).to_string(),
            ))
            .unwrap();
        assert_eq!(
            app.clone().oneshot(request).await.unwrap().status(),
            StatusCode::BAD_REQUEST
        );
        assert!(state
            .store
            .pending_read_receipts(chrono::Utc::now().timestamp())
            .unwrap()
            .is_empty());
    }
    let viewed = || {
        HttpRequest::builder().method("POST").uri("/api/contacts/family%40g.us/read")
        .header("content-type", "application/json").body(Body::from(r#"{"messageIds":["visible","visible"],"timestamp":100,"senderJid":"spoofed@s.whatsapp.net"}"#)).unwrap()
    };
    assert_eq!(
        app.clone().oneshot(viewed()).await.unwrap().status(),
        StatusCode::OK
    );
    assert_eq!(
        app.oneshot(viewed()).await.unwrap().status(),
        StatusCode::OK
    );
    assert_eq!(
        state
            .store
            .get_contact("family@g.us")
            .unwrap()
            .unwrap()
            .unread_count,
        1
    );
    let (tx, mut rx) = mpsc::channel(10);
    state.set_command_tx(tx).await;
    crate::read_receipts::dispatch(&state).await.unwrap();
    assert!(rx.try_recv().is_err(), "Offline views remain queued");
    *state.connected.write().await = true;
    crate::read_receipts::dispatch(&state).await.unwrap();
    let BridgeCommand::MarkRead {
        to,
        message_id,
        sender_jid,
        timestamp,
    } = rx.try_recv().unwrap()
    else {
        panic!("Expected read receipt")
    };
    assert_eq!(to, "family@g.us");
    assert_eq!(message_id, "visible");
    assert_eq!(sender_jid.as_deref(), Some("111@lid"));
    assert!(
        timestamp > 100,
        "Receipt uses the view time, never the message's sent time or client supplied time"
    );
    let restarted = MessageStore::new(&dir).unwrap();
    assert!(restarted
        .pending_read_receipts(timestamp + 14)
        .unwrap()
        .is_empty());
    assert_eq!(
        restarted
            .pending_read_receipts(timestamp + 16)
            .unwrap()
            .len(),
        1,
        "No success acknowledgement means retry after restart"
    );
    restarted
        .read_receipt_sent("visible", timestamp + 17)
        .unwrap();
    assert!(restarted
        .pending_read_receipts(timestamp + 60)
        .unwrap()
        .is_empty());
    assert_eq!(
        restarted
            .get_contact("family@g.us")
            .unwrap()
            .unwrap()
            .unread_count,
        1
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn replay_restores_missing_group_receipt_sender_without_marking_new_messages_seen() {
    let (state, dir) = test_state(None);
    state
        .store
        .upsert_contact("family@g.us", None, None, Some("group"), 0)
        .unwrap();
    let mut message = feed_test_message("old-group-message", "family@g.us", 100);
    message.sender_phone = None;
    state
        .store
        .add_message_with_notification(&message, Some(false))
        .unwrap();
    state
        .store
        .record_message_views("family@g.us", &[message.id.clone()], 1000)
        .unwrap();
    assert!(state.store.pending_read_receipts(1020).unwrap().is_empty());
    message.content_json =
        serde_json::json!({"type":"text","body":"Recovered", "receipt_sender_jid":"111@lid"})
            .to_string();
    state.store.recover_reply_context(&message).unwrap();
    let pending = state.store.pending_read_receipts(1020).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].sender_jid.as_deref(), Some("111@lid"));
    assert_eq!(pending[0].viewed_at, 1000);
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn legacy_empty_read_requests_never_infer_receipts_and_auth_is_required() {
    let (state, dir) = test_state(Some("secret"));
    let request = HttpRequest::builder()
        .method("POST")
        .uri("/api/contacts/family%40g.us/read")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"messageIds":["x"]}"#))
        .unwrap();
    assert_eq!(
        create_router(state)
            .oneshot(request)
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    std::fs::remove_dir_all(dir).unwrap();
    let (state, dir) = test_state(None);
    state
        .store
        .upsert_contact("friend@s.whatsapp.net", None, None, Some("private"), 0)
        .unwrap();
    let msg = feed_test_message("latest", "friend@s.whatsapp.net", 100);
    state
        .store
        .add_message_with_notification(&msg, Some(false))
        .unwrap();
    let request = HttpRequest::builder()
        .method("POST")
        .uri("/api/contacts/friend%40s.whatsapp.net/read")
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    assert_eq!(
        create_router(state.clone())
            .oneshot(request)
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert!(state
        .store
        .pending_read_receipts(chrono::Utc::now().timestamp())
        .unwrap()
        .is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}
