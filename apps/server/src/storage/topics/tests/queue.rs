use super::*;

#[test]
fn outgoing_topic_text_and_jobs_survive_send_confirmation_and_echo() {
    let (store, path) = test_store();
    store
        .upsert_contact("chat@g.us", None, None, Some("group"), 1)
        .unwrap();
    store.set_topics_enabled("chat@g.us", true).unwrap();
    let mut message = test_message("pending", 100);
    message.contact_id = "chat@g.us".into();
    message.is_from_me = true;
    message.original_text = None;
    message.content_json =
        json!({"type":"text","body":"Yes, the hospital appointment works."}).to_string();
    store.add_message(&message).unwrap();
    let pending = store.next_topic_batch().unwrap().unwrap();
    assert_eq!(
        pending.messages[0].original_text.as_deref(),
        Some("Yes, the hospital appointment works.")
    );
    store
        .replace_message_id("pending", "confirmed", Some(101))
        .unwrap();
    // A response for the temporary ID cannot attach to the wrong message.
    let assignment = TopicAssignment {
        message_id: "pending".into(),
        topic: "Hospital".into(),
    };
    assert_eq!(
        store.finish_topic_batch(&pending, &[assignment]).unwrap(),
        0
    );
    let confirmed = store.next_topic_batch().unwrap().unwrap();
    assert_eq!(confirmed.messages[0].id, "confirmed");
    finish(&store, &confirmed, "Hospital");
    store
        .replace_message_id("confirmed", "final-id", None)
        .unwrap();
    assert_eq!(
        store.message_topics(&["final-id".into()]).unwrap()[0]
            .title
            .as_deref(),
        Some("Hospital")
    );
    assert!(store.next_topic_batch().unwrap().is_none());
    // WhatsApp can echo the real message before send confirmation arrives.
    message.id = "pending-echo".into();
    store.add_message(&message).unwrap();
    message.id = "real-echo".into();
    store.add_message(&message).unwrap();
    store
        .replace_message_id("pending-echo", "real-echo", None)
        .unwrap();
    let echo = store.next_topic_batch().unwrap().unwrap();
    assert_eq!(echo.messages.len(), 1);
    assert_eq!(echo.messages[0].id, "real-echo");
    finish(&store, &echo, "Hospital");
    let conn = store.conn.lock().unwrap();
    assert_eq!(
        conn.query_row("SELECT count(*) FROM topic_jobs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(conn);
    drop(store);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn topic_context_resolves_quotes_and_only_earlier_same_chat_messages() {
    let (store, path) = test_store();
    legacy_topic(&store, "chat@g.us", "hospital", "Hospital");
    // The quote remains useful beyond the six-message neighbourhood.
    for i in 0..8 {
        add(&store, "chat@g.us", &format!("earlier-{i}"), 110 + i);
    }
    add(&store, "other@g.us", "private-other-chat", 119);
    add(&store, "chat@g.us", "future", 200);
    let mut target = test_message("outgoing", 120);
    target.contact_id = "chat@g.us".into();
    target.is_from_me = true;
    target.original_text = Some("Yes, I can take you.".into());
    target.content_json = json!({"type":"text","body":"Yes, I can take you.","reply_context":{"message_id":"hospital","text":"old quote","media_data":"must-not-send"}}).to_string();
    let batch = TopicBatch {
        contact_id: target.contact_id.clone(),
        epoch: "test".into(),
        messages: vec![target.clone()],
    };
    let input = store.topic_message_input(&batch).unwrap();
    assert_eq!(input[0]["isFromMe"], true);
    assert_eq!(input[0]["replyTo"]["topic"], "Hospital");
    assert_eq!(input[0]["replyTo"]["messageId"], "hospital");
    let context = input[0]["previousMessages"].as_array().unwrap();
    assert_eq!(context.len(), 6);
    assert_eq!(context[0]["messageId"], "earlier-2");
    assert_eq!(context[5]["messageId"], "earlier-7");
    assert!(context.iter().all(|m| m["topic"].is_null())); // Unclassified neighbours are included.
    let serialized = serde_json::to_string(&input).unwrap();
    assert!(!serialized.contains("private-other-chat"));
    assert!(!serialized.contains("future"));
    assert!(!serialized.contains("must-not-send"));
    // Embedded quote text still works if the quoted message is not stored;
    // an ID from another chat cannot resolve to that chat's text or topic.
    target.content_json = json!({"type":"text","reply_context":{"messageId":"private-other-chat","text":"x".repeat(2000),"media_data":"must-not-send"}}).to_string();
    let fallback = store
        .topic_message_input(&TopicBatch {
            messages: vec![target],
            ..batch
        })
        .unwrap();
    assert_eq!(fallback[0]["replyTo"]["text"].as_str().unwrap().len(), 600);
    assert!(fallback[0]["replyTo"]["topic"].is_null());
    assert!(!fallback[0]["replyTo"].to_string().contains("must-not-send"));
    drop(store);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn outgoing_topic_recovery_is_recent_opted_in_and_once_only() {
    let (store, path) = test_store();
    let now = 1_800_000_000_000;
    for chat in ["enabled@g.us", "paused@g.us"] {
        store
            .upsert_contact(chat, None, None, Some("group"), now)
            .unwrap();
        store
            .set_topics_enabled(chat, chat.starts_with("enabled"))
            .unwrap();
    }
    for (id, chat, time) in [
        ("recent", "enabled@g.us", now),
        ("lost-id", "enabled@g.us", now),
        ("old", "enabled@g.us", now - 8 * 86_400_000),
        ("future", "enabled@g.us", now + 1),
        ("paused", "paused@g.us", now),
        ("saved", "enabled@g.us", now),
    ] {
        let mut m = test_message(id, time);
        m.contact_id = chat.into();
        m.is_from_me = true;
        m.content_json = json!({"type":"text","body":"See you at the hospital."}).to_string();
        store.add_message(&m).unwrap();
    }
    while let Some(batch) = store.next_topic_batch().unwrap() {
        finish(&store, &batch, "Hospital");
    }
    {
        let conn = store.conn.lock().unwrap();
        conn.execute(
            "UPDATE messages SET original_text=NULL WHERE id IN ('recent','old','future','paused')",
            [],
        )
        .unwrap();
        conn.execute(
            "DELETE FROM topic_assignments WHERE message_id='lost-id'",
            [],
        )
        .unwrap();
        conn.execute("DELETE FROM topic_jobs", []).unwrap();
        conn.execute(
            "DELETE FROM app_settings WHERE key='topic_outgoing_context_v1'",
            [],
        )
        .unwrap();
    }
    store.recover_outgoing_topics(now).unwrap();
    let batch = store.next_topic_batch().unwrap().unwrap();
    let ids: Vec<_> = batch.messages.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, vec!["lost-id", "recent"]);
    assert_eq!(
        batch.messages[1].original_text.as_deref(),
        Some("See you at the hospital.")
    );
    assert_eq!(
        store.message_topics(&["saved".into()]).unwrap()[0]
            .title
            .as_deref(),
        Some("Hospital")
    );
    finish(&store, &batch, "Hospital");
    store
        .recover_outgoing_topics(now + 10 * 86_400_000)
        .unwrap();
    assert!(store.next_topic_batch().unwrap().is_none());
    drop(store);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn topic_retries_isolate_messages_and_recover_exhausted_work_only_once() {
    let (store, path) = test_store();
    for i in 0..8 {
        add(&store, "family@g.us", &format!("message-{i}"), i);
    }
    store.set_topics_enabled("family@g.us", true).unwrap();
    let initial = store.next_topic_batch().unwrap().unwrap();
    assert_eq!(initial.messages.len(), 8);
    store.retry_topic_batch(&initial).unwrap();
    store
        .conn
        .lock()
        .unwrap()
        .execute("UPDATE topic_jobs SET retry_at=0", [])
        .unwrap();
    let retry = store.next_topic_batch().unwrap().unwrap();
    assert_eq!(retry.messages.len(), 1);
    finish(&store, &retry, "Hospital");
    assert_eq!(store.topic_settings().unwrap()[0].pending_count, 7);
    store.conn.lock().unwrap().execute_batch("UPDATE topic_jobs SET attempts=3; DELETE FROM app_settings WHERE key='topic_isolated_retries_v1';").unwrap();
    drop(store);
    let store = MessageStore::new(&path).unwrap();
    assert_eq!(store.topic_settings().unwrap()[0].pending_count, 7);
    assert_eq!(store.topic_settings().unwrap()[0].failed_count, 0);
    let recovered = store.next_topic_batch().unwrap().unwrap();
    assert_eq!(recovered.messages.len(), 1);
    for _ in 0..2 {
        store.retry_topic_batch(&recovered).unwrap();
    }
    drop(store);
    let store = MessageStore::new(&path).unwrap();
    assert_eq!(store.topic_settings().unwrap()[0].failed_count, 1); // restart cannot give unlimited retries
    while let Some(batch) = store.next_topic_batch().unwrap() {
        assert_eq!(batch.messages.len(), 1);
        finish(&store, &batch, "Hospital");
    }
    assert_eq!(store.topic_settings().unwrap()[0].pending_count, 0);
    assert_eq!(store.list_topics().unwrap()[0].message_count, 7);
    drop(store);
    std::fs::remove_dir_all(path).unwrap();
}
