//! Read receipt and mark-as-read tests.

use super::*;
use crate::bridge::BridgeEvent;

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

#[tokio::test]
async fn reading_on_the_phone_or_replying_clears_the_unread_badge() {
    let (state, dir) = test_state(None);
    let chat = "447700900123@s.whatsapp.net";
    let raw = |id: &str, timestamp: i64, from_me: bool| {
        serde_json::json!({
            "type":"message","id":id,"timestamp":timestamp,
            "from":{"jid":chat,"phone":"447700900123"},
            "chat":{"type":"private","jid":chat,"name":"Virág"},
            "content":{"type":"text","body":id},"is_from_me":from_me,"is_forwarded":false
        })
    };
    let wire = |id: &str, timestamp: i64, from_me: bool| {
        serde_json::from_value::<BridgeEvent>(raw(id, timestamp, from_me)).unwrap()
    };
    let unread =
        |state: &Arc<AppState>| state.store.get_contact(chat).unwrap().unwrap().unread_count;
    for (id, timestamp) in [("in-1", 100), ("in-2", 101), ("in-3", 102), ("in-4", 103)] {
        crate::handle_web_event(wire(id, timestamp, false), &state, &state.store, None)
            .await
            .unwrap();
    }
    assert_eq!(unread(&state), 4);

    // The phone reports reading in-2 (a LID chat ID, as WhatsApp may send it):
    // everything up to it is read, later messages stay unread.
    let mut events = state.broadcast_tx.subscribe();
    let read: BridgeEvent = serde_json::from_value(serde_json::json!({
        "type":"messages_read","chat_id":"123456789@lid","message_ids":["in-2","unknown"]
    }))
    .unwrap();
    crate::handle_web_event(read, &state, &state.store, None)
        .await
        .unwrap();
    assert_eq!(unread(&state), 2);
    let WebSocketEvent::MessagesViewed {
        chat_id,
        mut message_ids,
        unread_count,
    } = events.try_recv().unwrap()
    else {
        panic!("clients must update the badge")
    };
    message_ids.sort();
    assert_eq!((chat_id.as_str(), unread_count), (chat, 2));
    assert_eq!(message_ids, ["in-1", "in-2"]);
    assert!(
        state
            .store
            .pending_read_receipts(i64::MAX)
            .unwrap()
            .is_empty(),
        "Reads elsewhere never send receipts from here"
    );

    // Reacting to an old message is not reading the chat.
    let mut reaction = raw("react-1", 110, true);
    reaction["content"] = serde_json::json!({
        "type":"reaction","emoji":"👍","target_message_id":"in-1","sender_timestamp_ms":110000
    });
    crate::handle_web_event(
        serde_json::from_value(reaction).unwrap(),
        &state,
        &state.store,
        None,
    )
    .await
    .unwrap();
    assert_eq!(unread(&state), 2);

    // A message in the same second as the reply may have arrived after it.
    crate::handle_web_event(wire("in-same", 104, false), &state, &state.store, None)
        .await
        .unwrap();
    // Sending a reply from the phone reads the chat before it.
    crate::handle_web_event(wire("out-1", 104, true), &state, &state.store, None)
        .await
        .unwrap();
    assert_eq!(unread(&state), 1);
    let unread_ids: Vec<String> = state
        .store
        .record_read_elsewhere(&["in-same".into()], true)
        .unwrap()
        .remove(0)
        .message_ids;
    assert_eq!(unread_ids, ["in-same"]);
    assert_eq!(unread(&state), 0);

    // A message after the reply is unread again.
    crate::handle_web_event(wire("in-5", 105, false), &state, &state.store, None)
        .await
        .unwrap();
    assert_eq!(unread(&state), 1);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_count_with_no_unread_messages_behind_it_clears_once_viewed() {
    let (state, dir) = test_state(None);
    let chat = "family@g.us";
    state
        .store
        .upsert_contact(chat, None, None, Some("group"), 0)
        .unwrap();
    let mut message = feed_test_message("only", chat, 100);
    message.content_json =
        serde_json::json!({"type":"text","body":"only","receipt_sender_jid":"111@lid"}).to_string();
    state.store.add_message(&message).unwrap();
    // WhatsApp's history sync said 10 unread, but only one message was stored.
    state.store.set_unread_count(chat, 10).unwrap();
    assert_eq!(
        state
            .store
            .record_message_views(chat, &["only".into()], 1000)
            .unwrap(),
        0
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn history_unread_counts_cover_every_synced_message_and_receipts_keep_same_second_arrivals() {
    let (state, dir) = test_state(None);
    let chat = "447700900123@s.whatsapp.net";
    let wire = |id: &str, timestamp: i64, history: bool, unread: Option<u32>| {
        serde_json::from_value::<BridgeEvent>(serde_json::json!({
            "type":"message","id":id,"timestamp":timestamp,
            "from":{"jid":chat,"phone":"447700900123"},
            "chat":{"type":"private","jid":chat,"name":"Geckos"},
            "content":{"type":"text","body":id},"is_from_me":false,"is_forwarded":false,
            "is_history":history,"unread_count":unread
        }))
        .unwrap()
    };
    let unread =
        |state: &Arc<AppState>| state.store.get_contact(chat).unwrap().unwrap().unread_count;
    // History sync sends the newest message first, carrying the chat's count.
    for (id, timestamp, count) in [
        ("h-3", 103, Some(3)),
        ("h-2", 102, None),
        ("h-1", 101, None),
        ("h-0", 100, None),
    ] {
        crate::handle_web_event(wire(id, timestamp, true, count), &state, &state.store, None)
            .await
            .unwrap();
    }
    assert_eq!(unread(&state), 3);
    // Viewing the oldest unread message leaves the other two unread.
    assert_eq!(
        state
            .store
            .record_message_views(chat, &["h-1".into()], 1000)
            .unwrap(),
        2
    );

    // A live message arrives in the same second as h-3, after the phone read h-3.
    crate::handle_web_event(wire("live", 103, false, None), &state, &state.store, None)
        .await
        .unwrap();
    assert_eq!(unread(&state), 3);
    let read: BridgeEvent = serde_json::from_value(serde_json::json!({
        "type":"messages_read","chat_id":chat,"message_ids":["h-3"]
    }))
    .unwrap();
    crate::handle_web_event(read, &state, &state.store, None)
        .await
        .unwrap();
    assert_eq!(
        unread(&state),
        1,
        "only the message the receipt names, and those before it"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn deleted_messages_are_never_unread_and_reads_cancel_reaction_notifications() {
    let (state, dir) = test_state(None);
    let chat = "447700900123@s.whatsapp.net";
    let wire = |id: &str,
                timestamp: i64,
                history: bool,
                unread: Option<u32>,
                content: serde_json::Value| {
        serde_json::from_value::<BridgeEvent>(serde_json::json!({
            "type":"message","id":id,"timestamp":timestamp,
            "from":{"jid":chat,"phone":"447700900123"},
            "chat":{"type":"private","jid":chat,"name":"Geckos"},
            "content":content,"is_from_me":false,"is_forwarded":false,
            "is_history":history,"unread_count":unread
        }))
        .unwrap()
    };
    let text = |id: &str| serde_json::json!({"type":"text","body":id});
    let unread =
        |state: &Arc<AppState>| state.store.get_contact(chat).unwrap().unwrap().unread_count;
    // Newest first: a text, a deleted message, then an older text, two unread.
    for (id, timestamp, count, content) in [
        ("t-2", 102, Some(2), text("t-2")),
        ("gone", 101, None, serde_json::json!({"type":"revoked"})),
        ("t-1", 100, None, text("t-1")),
    ] {
        crate::handle_web_event(
            wire(id, timestamp, true, count, content),
            &state,
            &state.store,
            None,
        )
        .await
        .unwrap();
    }
    assert_eq!(
        state
            .store
            .record_message_views(chat, &["t-1".into(), "t-2".into()], 1000)
            .unwrap(),
        0
    );

    // A live reaction is notified but not counted; reading past it cancels it.
    crate::handle_web_event(
        wire(
            "like",
            200,
            false,
            None,
            serde_json::json!({"type":"reaction","emoji":"👍","target_message_id":"t-2","sender_timestamp_ms":200000}),
        ),
        &state,
        &state.store,
        None,
    )
    .await
    .unwrap();
    crate::handle_web_event(
        wire("t-3", 201, false, None, text("t-3")),
        &state,
        &state.store,
        None,
    )
    .await
    .unwrap();
    assert_eq!(unread(&state), 1);
    assert!(state
        .store
        .ready_notification_ids()
        .unwrap()
        .contains(&"like".to_string()));
    let read: BridgeEvent = serde_json::from_value(serde_json::json!({
        "type":"messages_read","chat_id":chat,"message_ids":["t-3"]
    }))
    .unwrap();
    crate::handle_web_event(read, &state, &state.store, None)
        .await
        .unwrap();
    assert_eq!(unread(&state), 0);
    assert!(state.store.ready_notification_ids().unwrap().is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}
