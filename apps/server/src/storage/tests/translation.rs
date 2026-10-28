use super::*;

#[test]
fn live_notifications_wait_for_translation_through_retry_and_restart() {
    let (store, dir) = test_store();
    store
        .upsert_contact("chat@example.test", Some("Test Chat"), None, None, 1)
        .unwrap();
    store
        .update_conversation_settings(
            "chat@example.test",
            &ConversationSettings {
                translation_enabled: true,
                ..Default::default()
            },
        )
        .unwrap();
    store
        .add_message_with_notification(&test_message("Szia", 1), Some(true))
        .unwrap();
    assert!(store.ready_notification_ids().unwrap().is_empty());
    assert_eq!(store.claim_translation().unwrap().as_deref(), Some("Szia"));
    store.retry_translation("Szia").unwrap();
    assert!(store.ready_notification_ids().unwrap().is_empty());
    drop(store);

    let store = MessageStore::new(&dir).unwrap();
    assert!(store.ready_notification_ids().unwrap().is_empty());
    store
        .finish_translation("Szia", Some("Hello"), "Hungarian", true, Some("Szia"), 0)
        .unwrap();
    assert_eq!(store.ready_notification_ids().unwrap(), vec!["Szia"]);
    let saved = store.get_message_by_id("Szia").unwrap().unwrap();
    assert_eq!(saved.translated_text.as_deref(), Some("Hello"));
    store.finish_notification("Szia").unwrap();
    // Replayed bridge events must not create a second alert.
    store
        .add_message_with_notification(&test_message("Szia", 1), Some(true))
        .unwrap();
    assert!(store.ready_notification_ids().unwrap().is_empty());
    drop(store);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn notification_queue_excludes_history_and_outgoing_and_allows_detected_target_language() {
    let (store, dir) = test_store();
    store
        .upsert_contact("chat@example.test", None, None, None, 1)
        .unwrap();
    store
        .update_conversation_settings(
            "chat@example.test",
            &ConversationSettings {
                translation_enabled: true,
                ..Default::default()
            },
        )
        .unwrap();
    store.add_message(&test_message("history", 1)).unwrap();
    store.enqueue_translation("history").unwrap();
    store
        .finish_translation(
            "history",
            Some("Old message"),
            "Hungarian",
            true,
            Some("history"),
            0,
        )
        .unwrap();
    let mut outgoing = test_message("outgoing", 2);
    outgoing.is_from_me = true;
    store
        .add_message_with_notification(&outgoing, Some(false))
        .unwrap();
    store
        .add_message_with_notification(&test_message("English", 3), Some(true))
        .unwrap();
    store
        .add_message_with_notification(&test_message("empty-result", 4), Some(true))
        .unwrap();
    store
        .finish_translation(
            "empty-result",
            Some(" "),
            "Hungarian",
            true,
            Some("empty-result"),
            0,
        )
        .unwrap();
    assert!(store.ready_notification_ids().unwrap().is_empty());
    store
        .finish_translation("English", None, "English", false, Some("English"), 0)
        .unwrap();
    assert_eq!(store.ready_notification_ids().unwrap(), vec!["English"]);
    store.finish_notification("English").unwrap();
    let mut photo = test_message("photo", 5);
    photo.original_text = None;
    photo.content_type = "Image".into();
    store
        .add_message_with_notification(&photo, Some(false))
        .unwrap();
    assert_eq!(store.ready_notification_ids().unwrap(), vec!["photo"]);
    store.delete_message("photo").unwrap();
    assert!(store.ready_notification_ids().unwrap().is_empty());
    drop(store);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn legacy_language_labels_are_rechecked_once_without_losing_translations() {
    let (store, dir) = test_store();
    store
        .upsert_contact("chat@example.test", None, None, None, 0)
        .unwrap();
    store
        .update_conversation_settings(
            "chat@example.test",
            &ConversationSettings {
                translation_enabled: true,
                ..Default::default()
            },
        )
        .unwrap();
    let mut message = test_message("legacy", 1);
    message.source_language = Some("English".into());
    store.add_message(&message).unwrap();
    message.id = "translated".into();
    message.source_language = Some("Hungarian".into());
    message.translated_text = Some("Good morning".into());
    message.is_translated = true;
    store.add_message(&message).unwrap();
    store
        .conn
        .lock()
        .unwrap()
        .execute(
            "DELETE FROM app_settings WHERE key='language_detection_contract_v2'",
            [],
        )
        .unwrap();
    drop(store);
    let store = MessageStore::new(&dir).unwrap();
    let legacy = store.get_message_by_id("legacy").unwrap().unwrap();
    assert!(legacy.source_language.is_none());
    assert_eq!(legacy.original_text.as_deref(), Some("legacy"));
    let translated = store.get_message_by_id("translated").unwrap().unwrap();
    assert_eq!(translated.source_language.as_deref(), Some("Hungarian"));
    assert_eq!(translated.translated_text.as_deref(), Some("Good morning"));
    store
        .finish_translation("legacy", None, "English", false, Some("legacy"), 0)
        .unwrap();
    drop(store);
    let store = MessageStore::new(&dir).unwrap();
    assert_eq!(
        store
            .get_message_by_id("legacy")
            .unwrap()
            .unwrap()
            .source_language
            .as_deref(),
        Some("English")
    );
    drop(store);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn conversation_language_uses_recent_incoming_messages_and_latest_tie() {
    let (store, dir) = test_store();
    store
        .upsert_contact("chat@example.test", None, None, None, 0)
        .unwrap();
    for n in 0..25 {
        let mut message = test_message(&format!("old-{n}"), n);
        message.source_language = Some("English".into());
        store.add_message(&message).unwrap();
    }
    for n in 25..29 {
        let mut message = test_message(&format!("recent-{n}"), n);
        message.source_language = Some("Hungarian".into());
        store.add_message(&message).unwrap();
    }
    assert_eq!(
        store
            .get_conversation_language("chat@example.test", 4)
            .unwrap()
            .as_deref(),
        Some("Hungarian")
    );
    let mut reply = test_message("latest", 30);
    reply.source_language = Some("French".into());
    store.add_message(&reply).unwrap();
    assert_eq!(
        store
            .get_conversation_language("chat@example.test", 2)
            .unwrap()
            .as_deref(),
        Some("French")
    );
    reply.id = "outgoing".into();
    reply.timestamp = 31;
    reply.is_from_me = true;
    reply.source_language = Some("English".into());
    store.add_message(&reply).unwrap();
    assert_eq!(
        store
            .get_conversation_language("chat@example.test", 2)
            .unwrap()
            .as_deref(),
        Some("French")
    );
    drop(store);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn fresh_translations_take_priority_over_recovered_failures() {
    let (store, dir) = test_store();
    store
        .upsert_contact("chat@example.test", None, None, None, 0)
        .unwrap();
    store
        .update_conversation_settings(
            "chat@example.test",
            &ConversationSettings {
                translation_enabled: true,
                ..Default::default()
            },
        )
        .unwrap();
    for id in ["old-failure", "new-message"] {
        store.add_message(&test_message(id, 1)).unwrap();
        store.enqueue_translation(id).unwrap();
    }
    store
        .conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE translation_jobs SET attempts=3 WHERE message_id='old-failure'",
            [],
        )
        .unwrap();
    assert_eq!(
        store.claim_translation().unwrap().as_deref(),
        Some("new-message")
    );
    assert_eq!(
        store.claim_translation().unwrap().as_deref(),
        Some("old-failure")
    );
    drop(store);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn translation_queue_survives_restart_and_retries_transient_failures() {
    let (store, dir) = test_store();
    store
        .upsert_contact("chat@example.test", None, None, None, 1)
        .unwrap();
    store
        .update_conversation_settings(
            "chat@example.test",
            &ConversationSettings {
                translation_enabled: true,
                ..Default::default()
            },
        )
        .unwrap();
    store.add_message(&test_message("queued", 1)).unwrap();
    store.enqueue_translation("queued").unwrap();
    assert_eq!(
        store.claim_translation().unwrap().as_deref(),
        Some("queued")
    );
    drop(store);
    let store = MessageStore::new(&dir).unwrap();
    store.recover_translations().unwrap();
    assert_eq!(
        store.claim_translation().unwrap().as_deref(),
        Some("queued")
    );
    store.retry_translation("queued").unwrap();
    assert!(store.claim_translation().unwrap().is_none());
    store
        .conn
        .lock()
        .unwrap()
        .execute("UPDATE translation_jobs SET retry_at=0", [])
        .unwrap();
    assert_eq!(
        store.claim_translation().unwrap().as_deref(),
        Some("queued")
    );
    store.retry_translation("queued").unwrap();
    store
        .conn
        .lock()
        .unwrap()
        .execute("UPDATE translation_jobs SET retry_at=0", [])
        .unwrap();
    assert_eq!(
        store.claim_translation().unwrap().as_deref(),
        Some("queued")
    );
    assert_eq!(
        store
            .get_message_by_id("queued")
            .unwrap()
            .unwrap()
            .original_text
            .as_deref(),
        Some("queued")
    );
    drop(store);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn mcp_send_claim_and_outcome_survive_restart() {
    let (store, dir) = test_store();
    let pending = serde_json::json!({"state":"pending"});
    assert!(store.claim_mcp_send("operation", &pending).unwrap());
    drop(store);
    let store = MessageStore::new(&dir).unwrap();
    assert!(!store.claim_mcp_send("operation", &pending).unwrap());
    assert_eq!(store.mcp_send_record("operation").unwrap(), Some(pending));
    let outcome = serde_json::json!({"state":"completed", "messageId":"confirmed"});
    store.update_mcp_send("operation", &outcome).unwrap();
    drop(store);
    let store = MessageStore::new(&dir).unwrap();
    assert!(!store
        .claim_mcp_send("operation", &serde_json::json!({}))
        .unwrap());
    assert_eq!(store.mcp_send_record("operation").unwrap(), Some(outcome));
    drop(store);
    std::fs::remove_dir_all(dir).unwrap();
}
