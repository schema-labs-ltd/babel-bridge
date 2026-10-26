use super::{schema::table_column_names, *};
use crate::oauth::oauth_secret_hash;
use std::path::PathBuf;

pub(super) fn test_store() -> (MessageStore, PathBuf) {
    let data_dir = std::env::temp_dir().join(format!(
        "whatsapp-translator-storage-test-{}",
        uuid::Uuid::new_v4()
    ));
    let store = MessageStore::new(&data_dir).expect("test store");
    (store, data_dir)
}

#[test]
fn close_checkpoints_wal_and_releases_the_database() {
    let (store, dir) = test_store();
    store
        .upsert_contact("family@g.us", Some("Family"), None, Some("group"), 1)
        .unwrap();
    store.close().unwrap();
    store.close().unwrap();

    let wal = dir.join("messages.db-wal");
    let wal_len = std::fs::metadata(&wal).map(|meta| meta.len()).unwrap_or(0);
    assert_eq!(wal_len, 0, "checkpoint should truncate the WAL");

    let conn = Connection::open(dir.join("messages.db")).unwrap();
    let name: String = conn
        .query_row(
            "SELECT name FROM contacts WHERE id = 'family@g.us'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(name, "Family");
}

#[test]
fn group_notification_recipient_count_survives_restart_and_clears_on_logout() {
    let (store, dir) = test_store();
    store
        .upsert_contact("family@g.us", Some("Family"), None, Some("group"), 1)
        .unwrap();
    assert_eq!(
        store.notification_recipient_count("family@g.us").unwrap(),
        None
    );
    store.set_group_participant_count("family@g.us", 4).unwrap();
    assert_eq!(
        store.notification_recipient_count("family@g.us").unwrap(),
        Some(3)
    );
    store.set_group_participant_count("family@g.us", 0).unwrap();
    assert_eq!(
        store.notification_recipient_count("family@g.us").unwrap(),
        Some(3)
    );
    drop(store);
    let store = MessageStore::new(&dir).unwrap();
    assert_eq!(
        store.notification_recipient_count("family@g.us").unwrap(),
        Some(3)
    );
    store.set_group_participant_count("family@g.us", 3).unwrap();
    assert_eq!(
        store.notification_recipient_count("family@g.us").unwrap(),
        Some(2)
    );
    assert_eq!(
        store
            .notification_recipient_count("friend@s.whatsapp.net")
            .unwrap(),
        None
    );
    store.clear_all().unwrap();
    assert_eq!(
        store.notification_recipient_count("family@g.us").unwrap(),
        None
    );
    drop(store);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn unified_feed_paginates_across_chats_and_quotes_only_older_context() {
    let (store, path) = test_store();
    store
        .upsert_contact("chat@example.test", None, None, None, 0)
        .unwrap();
    store
        .upsert_contact("other@example.test", None, None, None, 0)
        .unwrap();
    let first = test_message("a", 100);
    let second = test_message("b", 100);
    let mut other = test_message("c", 101);
    other.contact_id = "other@example.test".into();
    store.add_message(&first).unwrap();
    store.add_message(&second).unwrap();
    store.add_message(&other).unwrap();
    assert!(store.reply_is_latest(&first.contact_id, "b").unwrap());
    assert!(!store.reply_is_latest(&first.contact_id, "a").unwrap());
    assert!(store.reply_is_latest(&first.contact_id, "c").is_err());
    let page = store.get_unified_messages(2, None, None).unwrap();
    assert_eq!(
        page.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["b", "c"]
    );
    let older = store.get_unified_messages(2, Some(100), Some("b")).unwrap();
    assert_eq!(
        older.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["a"]
    );
    let mut reaction = test_message("reaction", 200);
    reaction.content_type = "Reaction".into();
    store.add_message(&reaction).unwrap();
    assert!(store.reply_is_latest(&first.contact_id, "b").unwrap());
    assert_eq!(store.get_unified_messages(10, None, None).unwrap().len(), 3);
    let later = test_message("later", 300);
    store.add_message(&later).unwrap();
    assert!(!store.reply_is_latest(&first.contact_id, "b").unwrap());
    store.delete_message("b").unwrap();
    assert!(store.reply_is_latest(&first.contact_id, "b").is_err());
    drop(store);
    let _ = std::fs::remove_dir_all(path);
}

pub(super) fn test_message(id: &str, timestamp: i64) -> StoredMessage {
    let content = serde_json::json!({
        "type": "text",
        "body": id,
    });

    StoredMessage {
        id: id.to_string(),
        contact_id: "chat@example.test".to_string(),
        timestamp,
        is_from_me: false,
        is_forwarded: false,
        sender_name: Some("Tester".to_string()),
        sender_phone: None,
        contact_name: Some("Test Chat".to_string()),
        contact_phone: None,
        chat_type: "private".to_string(),
        content_type: "Text".to_string(),
        content_json: content.to_string(),
        content: Some(content),
        original_text: Some(id.to_string()),
        translated_text: None,
        source_language: None,
        is_translated: false,
        delivery_status: None,
    }
}

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

fn message_ids(messages: &[StoredMessage]) -> Vec<String> {
    messages.iter().map(|message| message.id.clone()).collect()
}

fn test_oauth_client(client_id: &str) -> OAuthClientRegistration {
    OAuthClientRegistration {
        client_id: client_id.to_string(),
        client_name: Some("Test MCP Client".to_string()),
        redirect_uris: vec!["http://127.0.0.1:9000/callback".to_string()],
        scope: "mcp".to_string(),
        created_at: 1_700_000_000,
    }
}

fn insert_test_contact(store: &MessageStore) {
    store
        .upsert_contact(
            "chat@example.test",
            Some("Test Chat"),
            None,
            Some("private"),
            1_700_000_000,
        )
        .expect("insert contact");
}

#[test]
fn conversation_settings_persist_original_follow_up_preference() {
    let (store, data_dir) = test_store();
    insert_test_contact(&store);

    let settings = ConversationSettings {
        reply_only: false,
        translation_enabled: true,
        language_override: Some("Spanish".to_string()),
        translation_style: Some("friendly".to_string()),
        send_original_follow_up: true,
    };
    store
        .update_conversation_settings("chat@example.test", &settings)
        .expect("save settings");

    let loaded = store
        .get_conversation_settings("chat@example.test")
        .expect("load settings");
    assert!(loaded.send_original_follow_up);

    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn translation_is_opt_in_for_existing_people_and_new_groups_and_survives_restart() {
    let (store, dir) = test_store();
    for (id, kind) in [
        ("person@s.whatsapp.net", "private"),
        ("family@g.us", "group"),
    ] {
        store.upsert_contact(id, None, None, Some(kind), 0).unwrap();
        assert!(
            !store
                .get_conversation_settings(id)
                .unwrap()
                .translation_enabled
        );
    }
    let settings = ConversationSettings {
        reply_only: false,
        translation_enabled: true,
        language_override: Some("Hungarian".into()),
        translation_style: Some("friendly".into()),
        send_original_follow_up: true,
    };
    store
        .update_conversation_settings("person@s.whatsapp.net", &settings)
        .unwrap();
    // Simulate the pre-toggle database; adding the column must not opt an
    // existing foreign-language conversation in implicitly.
    store
        .conn
        .lock()
        .unwrap()
        .execute("ALTER TABLE contacts DROP COLUMN translation_enabled", [])
        .unwrap();
    drop(store);
    let store = MessageStore::new(&dir).unwrap();
    let migrated = store
        .get_conversation_settings("person@s.whatsapp.net")
        .unwrap();
    assert!(!migrated.translation_enabled);
    assert_eq!(migrated.language_override.as_deref(), Some("Hungarian"));
    assert!(migrated.send_original_follow_up);
    store
        .update_conversation_settings("family@g.us", &settings)
        .unwrap();
    store
        .upsert_contact(
            "family@g.us",
            Some("Refreshed family"),
            None,
            Some("group"),
            5,
        )
        .unwrap();
    drop(store);
    let store = MessageStore::new(&dir).unwrap();
    assert!(
        store
            .get_conversation_settings("family@g.us")
            .unwrap()
            .translation_enabled
    );
    assert!(
        !store
            .get_conversation_settings("person@s.whatsapp.net")
            .unwrap()
            .translation_enabled
    );
    let mut disabled = settings.clone();
    disabled.translation_enabled = false;
    store
        .update_conversation_settings("family@g.us", &disabled)
        .unwrap();
    let saved = store.get_conversation_settings("family@g.us").unwrap();
    assert!(!saved.translation_enabled);
    assert_eq!(saved.language_override, settings.language_override);
    drop(store);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn disabling_translation_cancels_queued_work_releases_alerts_and_discards_in_flight_results() {
    let (store, dir) = test_store();
    insert_test_contact(&store);
    let mut settings = ConversationSettings {
        translation_enabled: true,
        ..Default::default()
    };
    store
        .update_conversation_settings("chat@example.test", &settings)
        .unwrap();
    store
        .add_message_with_notification(&test_message("first", 1), Some(true))
        .unwrap();
    assert!(store.ready_notification_ids().unwrap().is_empty());
    assert_eq!(store.claim_translation().unwrap().as_deref(), Some("first"));
    settings.translation_enabled = false;
    store
        .update_conversation_settings("chat@example.test", &settings)
        .unwrap();
    store
        .finish_translation(
            "first",
            Some("Late translation"),
            "Hungarian",
            true,
            Some("first"),
            0,
        )
        .unwrap();
    let first = store.get_message_by_id("first").unwrap().unwrap();
    assert!(!first.is_translated);
    assert!(
        first.source_language.is_none(),
        "turning it back on must permit detection later"
    );
    assert_eq!(store.ready_notification_ids().unwrap(), vec!["first"]);
    store
        .add_message_with_notification(&test_message("second", 2), Some(true))
        .unwrap();
    store.enqueue_translation("first").unwrap();
    store.enqueue_translation("second").unwrap();
    assert!(store.claim_translation().unwrap().is_none());
    assert_eq!(
        store.ready_notification_ids().unwrap(),
        vec!["first", "second"]
    );
    settings.translation_enabled = true;
    store
        .update_conversation_settings("chat@example.test", &settings)
        .unwrap();
    store.enqueue_translation("second").unwrap();
    assert_eq!(
        store.claim_translation().unwrap().as_deref(),
        Some("second")
    );
    drop(store);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn openai_settings_round_trip_globally() {
    let (store, data_dir) = test_store();
    let settings = OpenAiSettings {
        model: Some("gpt-5.6-terra".to_string()),
        reasoning_effort: Some("high".to_string()),
    };

    store
        .update_openai_settings(&settings)
        .expect("save OpenAI settings");

    assert_eq!(
        store.get_openai_settings().expect("load OpenAI settings"),
        settings
    );

    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn push_device_registration_replaces_the_token_for_an_installation() {
    let (store, data_dir) = test_store();

    store
        .register_push_device("installation-1", "token-old", "sandbox")
        .expect("register first token");
    store
        .register_push_device("installation-1", "token-new", "production")
        .expect("replace token");

    let devices = store.list_push_devices().expect("list push devices");
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].installation_id, "installation-1");
    assert_eq!(devices[0].token, "token-new");
    assert_eq!(devices[0].environment, "production");

    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn paginated_messages_use_id_cursor_when_timestamps_match() {
    let (store, data_dir) = test_store();
    insert_test_contact(&store);

    for id in ["m01", "m02", "m03", "m04"] {
        store
            .add_message(&test_message(id, 1_700_000_000))
            .expect("insert message");
    }

    let first_page = store
        .get_messages_paginated("chat@example.test", Some(2), None, None, true)
        .expect("first page");
    assert_eq!(message_ids(&first_page), vec!["m03", "m04"]);

    let second_page = store
        .get_messages_paginated(
            "chat@example.test",
            Some(2),
            Some(first_page[0].timestamp),
            Some(&first_page[0].id),
            true,
        )
        .expect("second page");
    assert_eq!(message_ids(&second_page), vec!["m01", "m02"]);

    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn replace_message_id_reconciles_pending_outgoing_message() {
    let (store, data_dir) = test_store();
    insert_test_contact(&store);
    store
        .add_message(&test_message("pending_1", 1_700_000_000_000))
        .expect("insert pending");

    store
        .replace_message_id("pending_1", "real_1", Some(1_700_000_005_000))
        .expect("replace id");

    let messages = store
        .get_messages_paginated("chat@example.test", None, None, None, true)
        .expect("messages");
    assert_eq!(message_ids(&messages), vec!["real_1"]);
    assert_eq!(messages[0].timestamp, 1_700_000_005_000);

    let contact = store
        .get_contact("chat@example.test")
        .expect("contact query")
        .expect("contact");
    assert_eq!(contact.last_message_time, 1_700_000_005_000);
    assert_eq!(contact.last_message_preview.as_deref(), Some("pending_1"));

    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn delivery_status_advances_without_regressing() {
    let (store, data_dir) = test_store();
    insert_test_contact(&store);
    let mut message = test_message("outgoing_1", 1_700_000_000_000);
    message.is_from_me = true;
    message.delivery_status = Some("sent".to_string());
    store
        .add_message(&message)
        .expect("insert outgoing message");

    assert_eq!(
        store
            .update_delivery_status(&["outgoing_1".to_string()], "delivered")
            .expect("mark delivered"),
        1
    );
    assert_eq!(
        store
            .update_delivery_status(&["outgoing_1".to_string()], "read")
            .expect("mark read"),
        1
    );
    store
        .update_delivery_status(&["outgoing_1".to_string()], "delivered")
        .expect("ignore stale delivered receipt");

    let stored = store
        .get_message_by_id("outgoing_1")
        .expect("query message")
        .expect("stored message");
    assert_eq!(stored.delivery_status.as_deref(), Some("read"));

    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn delete_message_refreshes_contact_preview_to_previous_message() {
    let (store, data_dir) = test_store();
    insert_test_contact(&store);
    store
        .add_message(&test_message("older", 1_700_000_000_000))
        .expect("insert older");
    store
        .add_message(&test_message("pending_1", 1_700_000_005_000))
        .expect("insert pending");

    assert!(store.delete_message("pending_1").expect("delete"));

    let messages = store
        .get_messages_paginated("chat@example.test", None, None, None, true)
        .expect("messages");
    assert_eq!(message_ids(&messages), vec!["older"]);

    let contact = store
        .get_contact("chat@example.test")
        .expect("contact query")
        .expect("contact");
    assert_eq!(contact.last_message_time, 1_700_000_000_000);
    assert_eq!(contact.last_message_preview.as_deref(), Some("older"));

    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn incoming_translated_message_uses_translation_for_contact_preview() {
    let (store, data_dir) = test_store();
    insert_test_contact(&store);
    let mut message = test_message("Szia, hogy vagy?", 1_700_000_000_000);
    message.translated_text = Some("Hi, how are you?".to_string());
    message.source_language = Some("Hungarian".to_string());
    message.is_translated = true;

    store
        .add_message(&message)
        .expect("insert translated message");

    let contact = store
        .get_contact("chat@example.test")
        .expect("contact query")
        .expect("contact");
    assert_eq!(
        contact.last_message_preview.as_deref(),
        Some("Hi, how are you?")
    );

    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn translating_latest_message_refreshes_contact_preview() {
    let (store, data_dir) = test_store();
    insert_test_contact(&store);
    store
        .add_message(&test_message("Szia, hogy vagy?", 1_700_000_000_000))
        .expect("insert untranslated message");

    store
        .update_message_translation(
            "Szia, hogy vagy?",
            Some("Hi, how are you?"),
            Some("Hungarian"),
        )
        .expect("translate message");

    let contact = store
        .get_contact("chat@example.test")
        .expect("contact query")
        .expect("contact");
    assert_eq!(
        contact.last_message_preview.as_deref(),
        Some("Hi, how are you?")
    );

    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn startup_migration_refreshes_existing_translated_contact_previews() {
    let (store, data_dir) = test_store();
    insert_test_contact(&store);
    let mut message = test_message("Szia, hogy vagy?", 1_700_000_000_000);
    message.translated_text = Some("Hi, how are you?".to_string());
    message.source_language = Some("Hungarian".to_string());
    message.is_translated = true;
    store
        .add_message(&message)
        .expect("insert translated message");
    {
        let conn = store.conn.lock().expect("database lock");
        conn.execute(
            "UPDATE contacts SET last_message_preview = ? WHERE id = ?",
            params!["Szia, hogy vagy?", "chat@example.test"],
        )
        .expect("restore legacy preview");
        conn.execute(
            "DELETE FROM app_settings WHERE key = ?",
            params!["translated_contact_previews_version"],
        )
        .expect("reset migration marker");
    }
    drop(store);

    let reopened = MessageStore::new(&data_dir).expect("reopen store");
    let contact = reopened
        .get_contact("chat@example.test")
        .expect("contact query")
        .expect("contact");
    assert_eq!(
        contact.last_message_preview.as_deref(),
        Some("Hi, how are you?")
    );

    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn replace_message_id_removes_pending_duplicate_when_real_message_exists() {
    let (store, data_dir) = test_store();
    insert_test_contact(&store);
    store
        .add_message(&test_message("pending_1", 1_700_000_000_000))
        .expect("insert pending");
    store
        .add_message(&test_message("real_1", 1_700_000_006_000))
        .expect("insert real");

    store
        .replace_message_id("pending_1", "real_1", Some(1_700_000_005_000))
        .expect("replace duplicate");

    let messages = store
        .get_messages_paginated("chat@example.test", None, None, None, true)
        .expect("messages");
    assert_eq!(message_ids(&messages), vec!["real_1"]);
    assert_eq!(messages[0].timestamp, 1_700_000_006_000);

    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn oauth_tokens_require_persisted_client_and_revoke_with_client() {
    let (store, data_dir) = test_store();
    let access_token = AccessToken {
        token: "access-token".to_string(),
        client_id: "client_test".to_string(),
        scope: "mcp".to_string(),
        created_at: 1_700_000_000,
        expires_at: i64::MAX,
        refresh_token: None,
    };
    let refresh_token = RefreshToken {
        token: "refresh-token".to_string(),
        client_id: "client_test".to_string(),
        scope: "mcp".to_string(),
        created_at: 1_700_000_000,
        expires_at: i64::MAX,
    };

    store
        .oauth_store_access_token(&access_token)
        .expect("store access token");
    store
        .oauth_store_refresh_token(&refresh_token)
        .expect("store refresh token");
    assert!(store
        .oauth_validate_access_token("access-token")
        .expect("validate without client")
        .is_none());

    store
        .oauth_store_client(&test_oauth_client("client_test"))
        .expect("store client");
    assert!(store
        .oauth_validate_access_token("access-token")
        .expect("validate with client")
        .is_some());
    assert!(store
        .oauth_get_refresh_token("refresh-token")
        .expect("refresh with client")
        .is_some());

    assert!(store
        .oauth_revoke_client("client_test")
        .expect("revoke client"));
    assert!(store
        .oauth_validate_access_token("access-token")
        .expect("validate after revoke")
        .is_none());
    assert!(store
        .oauth_get_refresh_token("refresh-token")
        .expect("refresh after revoke")
        .is_none());

    let _ = std::fs::remove_dir_all(data_dir);
}

fn linked_access_token(token: &str, client_id: &str, refresh_token: Option<&str>) -> AccessToken {
    AccessToken {
        token: token.to_string(),
        client_id: client_id.to_string(),
        scope: "whatsapp.read".to_string(),
        created_at: 1_700_000_000,
        expires_at: i64::MAX,
        refresh_token: refresh_token.map(str::to_string),
    }
}

fn stored_refresh_token(token: &str, client_id: &str) -> RefreshToken {
    RefreshToken {
        token: token.to_string(),
        client_id: client_id.to_string(),
        scope: "whatsapp.read".to_string(),
        created_at: 1_700_000_000,
        expires_at: i64::MAX,
    }
}

#[test]
fn oauth_access_session_revoke_removes_only_the_linked_refresh_family() {
    let (store, data_dir) = test_store();
    store
        .oauth_store_client(&test_oauth_client("client-a"))
        .expect("client a");
    store
        .oauth_store_client(&test_oauth_client("client-b"))
        .expect("client b");
    store
        .oauth_store_refresh_token(&stored_refresh_token("refresh-a", "client-a"))
        .expect("refresh a");
    store
        .oauth_store_refresh_token(&stored_refresh_token("refresh-a-other", "client-a"))
        .expect("other refresh");
    store
        .oauth_store_refresh_token(&stored_refresh_token("refresh-b", "client-b"))
        .expect("refresh b");
    store
        .oauth_store_access_token(&linked_access_token(
            "access-a",
            "client-a",
            Some("refresh-a"),
        ))
        .expect("access a");
    store
        .oauth_store_access_token(&linked_access_token(
            "access-a-renewed",
            "client-a",
            Some("refresh-a"),
        ))
        .expect("renewed access");
    store
        .oauth_store_access_token(&linked_access_token(
            "access-a-other",
            "client-a",
            Some("refresh-a-other"),
        ))
        .expect("other access");
    store
        .oauth_store_access_token(&linked_access_token(
            "access-b",
            "client-b",
            Some("refresh-b"),
        ))
        .expect("access b");
    store
        .oauth_store_access_token(&linked_access_token("access-a-unlinked", "client-a", None))
        .expect("unlinked access on the same client");

    store
        .oauth_revoke_access_session("access-a")
        .expect("revoke calling grant");
    store
        .oauth_revoke_access_session("access-a")
        .expect("revoke again");
    store
        .oauth_revoke_access_session("missing-token")
        .expect("unknown token");

    assert!(store
        .oauth_validate_access_token("access-a")
        .expect("access a")
        .is_none());
    assert!(store
        .oauth_validate_access_token("access-a-renewed")
        .expect("renewed")
        .is_none());
    assert!(store
        .oauth_get_refresh_token("refresh-a")
        .expect("refresh a")
        .is_none());
    assert!(store
        .oauth_validate_access_token("access-a-other")
        .expect("other access")
        .is_some());
    assert!(store
        .oauth_get_refresh_token("refresh-a-other")
        .expect("other refresh")
        .is_some());
    assert!(store
        .oauth_validate_access_token("access-b")
        .expect("access b")
        .is_some());
    assert!(store
        .oauth_get_refresh_token("refresh-b")
        .expect("refresh b")
        .is_some());
    assert!(store
        .oauth_validate_access_token("access-a-unlinked")
        .expect("unlinked access")
        .is_some());
    assert_eq!(store.oauth_list_clients().expect("clients").len(), 2);

    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn oauth_access_session_revoke_of_unlinked_token_leaves_the_clients_only_refresh_grant() {
    let (store, data_dir) = test_store();
    store
        .oauth_store_client(&test_oauth_client("client-shared"))
        .expect("client");
    store
        .oauth_store_refresh_token(&stored_refresh_token("refresh-shared", "client-shared"))
        .expect("shared refresh");
    store
        .oauth_store_access_token(&linked_access_token(
            "access-unlinked",
            "client-shared",
            None,
        ))
        .expect("unlinked access");
    store
        .oauth_store_access_token(&linked_access_token(
            "access-other-device",
            "client-shared",
            Some("refresh-shared"),
        ))
        .expect("other device access");

    store
        .oauth_revoke_access_session("access-unlinked")
        .expect("revoke unlinked access token");
    store
        .oauth_revoke_access_session("access-unlinked")
        .expect("already revoked");

    assert!(store
        .oauth_validate_access_token("access-unlinked")
        .expect("unlinked")
        .is_none());
    assert!(store
        .oauth_validate_access_token("access-other-device")
        .expect("other device")
        .is_some());
    assert!(store
        .oauth_get_refresh_token("refresh-shared")
        .expect("shared refresh")
        .is_some());
    assert_eq!(store.oauth_list_clients().expect("clients").len(), 1);

    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn oauth_access_session_revoke_leaves_ambiguous_refresh_tokens_in_place() {
    let (store, data_dir) = test_store();
    store
        .oauth_store_client(&test_oauth_client("client-legacy"))
        .expect("client");
    store
        .oauth_store_refresh_token(&stored_refresh_token("refresh-one", "client-legacy"))
        .expect("refresh one");
    store
        .oauth_store_refresh_token(&stored_refresh_token("refresh-two", "client-legacy"))
        .expect("refresh two");
    store
        .oauth_store_access_token(&linked_access_token(
            "access-unlinked",
            "client-legacy",
            None,
        ))
        .expect("unlinked access");

    store
        .oauth_revoke_access_session("access-unlinked")
        .expect("revoke unlinked");
    store
        .oauth_revoke_access_session("access-unlinked")
        .expect("already revoked");

    assert!(store
        .oauth_validate_access_token("access-unlinked")
        .expect("access")
        .is_none());
    assert!(store
        .oauth_get_refresh_token("refresh-one")
        .expect("refresh one")
        .is_some());
    assert!(store
        .oauth_get_refresh_token("refresh-two")
        .expect("refresh two")
        .is_some());

    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn existing_oauth_access_tokens_gain_refresh_token_column() {
    let (store, data_dir) = test_store();
    store
        .oauth_store_client(&test_oauth_client("client_existing"))
        .expect("client");
    store.close().expect("close");

    {
        let conn = Connection::open(data_dir.join("messages.db")).expect("open database");
        conn.execute_batch(
            "
                DROP TABLE oauth_access_tokens;
                CREATE TABLE oauth_access_tokens (
                    token TEXT PRIMARY KEY,
                    client_id TEXT NOT NULL,
                    scope TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    expires_at INTEGER NOT NULL
                );
                INSERT INTO oauth_access_tokens (token, client_id, scope, created_at, expires_at)
                VALUES ('existing-access', 'client_existing', 'whatsapp.read', 1700000000, 9223372036854775807);
                ",
        )
        .expect("restore legacy access token table");
    }

    let store = MessageStore::new(&data_dir).expect("reopen");
    let token = store
        .oauth_validate_access_token("existing-access")
        .expect("validate")
        .expect("existing token");
    assert!(token.refresh_token.is_none());
    {
        let conn = store.conn.lock().expect("database lock");
        let link: Option<String> = conn
            .query_row(
                "SELECT refresh_token_hash FROM oauth_access_tokens WHERE token_hash = ?1",
                params![oauth_secret_hash("existing-access")],
                |row| row.get(0),
            )
            .expect("link");
        assert!(link.is_none());
        assert_oauth_secret_schema(&conn);
    }
    store
        .oauth_store_access_token(&linked_access_token(
            "linked-access",
            "client_existing",
            Some("refresh-existing"),
        ))
        .expect("store linked token");
    store
        .oauth_store_refresh_token(&stored_refresh_token("refresh-existing", "client_existing"))
        .expect("store refresh");
    let linked = store
        .oauth_validate_access_token("linked-access")
        .expect("validate linked")
        .expect("linked token");
    assert_eq!(linked.client_id, "client_existing");
    assert!(linked.refresh_token.is_none());
    {
        let conn = store.conn.lock().expect("database lock");
        let link: String = conn
            .query_row(
                "SELECT refresh_token_hash FROM oauth_access_tokens WHERE token_hash = ?1",
                params![oauth_secret_hash("linked-access")],
                |row| row.get(0),
            )
            .expect("hashed link");
        assert_eq!(link, oauth_secret_hash("refresh-existing"));
        assert_ne!(link, "refresh-existing");
    }
    store
        .oauth_revoke_access_session("linked-access")
        .expect("revoke linked grant");
    assert!(store
        .oauth_get_refresh_token("refresh-existing")
        .expect("refresh")
        .is_none());
    assert!(store
        .oauth_validate_access_token("existing-access")
        .expect("legacy access")
        .is_some());

    let _ = std::fs::remove_dir_all(data_dir);
}

fn authorization_code(code: &str) -> AuthorizationCode {
    AuthorizationCode {
        code: code.to_string(),
        client_id: "client_test".to_string(),
        redirect_uri: "http://127.0.0.1:8787/callback".to_string(),
        code_challenge: "challenge".to_string(),
        code_challenge_method: "S256".to_string(),
        scope: "whatsapp.read".to_string(),
        created_at: 1_700_000_000,
        expires_at: i64::MAX,
        used: false,
    }
}

fn assert_oauth_secret_schema(conn: &Connection) {
    let access = table_column_names(conn, "oauth_access_tokens").expect("access columns");
    assert!(access.iter().any(|column| column == "token_hash"));
    assert!(access.iter().any(|column| column == "refresh_token_hash"));
    assert!(!access.iter().any(|column| column == "token"));
    assert!(!access.iter().any(|column| column == "refresh_token"));

    let refresh = table_column_names(conn, "oauth_refresh_tokens").expect("refresh columns");
    assert!(refresh.iter().any(|column| column == "token_hash"));
    assert!(!refresh.iter().any(|column| column == "token"));

    let codes = table_column_names(conn, "oauth_authorization_codes").expect("code columns");
    assert!(codes.iter().any(|column| column == "code_hash"));
    assert!(!codes.iter().any(|column| column == "code"));
}

fn oauth_row_count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |row| row.get(0)).expect("count")
}

#[test]
fn issued_oauth_secrets_are_stored_as_hashes() {
    let (store, data_dir) = test_store();
    store
        .oauth_store_client(&test_oauth_client("client_test"))
        .expect("client");
    store
        .oauth_store_refresh_token(&stored_refresh_token("refresh-token", "client_test"))
        .expect("refresh");
    store
        .oauth_store_access_token(&linked_access_token(
            "access-token",
            "client_test",
            Some("refresh-token"),
        ))
        .expect("access");
    store
        .oauth_store_authorization_code(&authorization_code("auth-code"))
        .expect("code");

    {
        let conn = store.conn.lock().expect("database lock");
        assert_oauth_secret_schema(&conn);
        let stored_access: String = conn
            .query_row("SELECT token_hash FROM oauth_access_tokens", [], |row| {
                row.get(0)
            })
            .expect("access hash");
        assert_ne!(stored_access, "access-token");
        assert_eq!(stored_access, oauth_secret_hash("access-token"));
        let stored_link: String = conn
            .query_row(
                "SELECT refresh_token_hash FROM oauth_access_tokens",
                [],
                |row| row.get(0),
            )
            .expect("link hash");
        assert_ne!(stored_link, "refresh-token");
        assert_eq!(stored_link, oauth_secret_hash("refresh-token"));
        let stored_refresh: String = conn
            .query_row("SELECT token_hash FROM oauth_refresh_tokens", [], |row| {
                row.get(0)
            })
            .expect("refresh hash");
        assert_eq!(stored_refresh, stored_link);
        let stored_code: String = conn
            .query_row(
                "SELECT code_hash FROM oauth_authorization_codes",
                [],
                |row| row.get(0),
            )
            .expect("code hash");
        assert_ne!(stored_code, "auth-code");
        assert_eq!(stored_code, oauth_secret_hash("auth-code"));
    }

    assert!(store
        .oauth_validate_access_token("access-token")
        .expect("lookup access")
        .is_some());
    assert!(store
        .oauth_validate_access_token("wrong-access")
        .expect("reject access")
        .is_none());
    assert!(store
        .oauth_get_refresh_token("refresh-token")
        .expect("lookup refresh")
        .is_some());
    let used = store
        .oauth_use_authorization_code("auth-code")
        .expect("lookup code")
        .expect("code");
    assert_eq!(used.code, "auth-code");
    assert_eq!(used.client_id, "client_test");
    assert!(store
        .oauth_use_authorization_code("auth-code")
        .expect("second use")
        .is_none());

    drop(store);
    let reopened = MessageStore::new(&data_dir).expect("reopen hashed database");
    assert!(reopened
        .oauth_validate_access_token("access-token")
        .expect("lookup after second open")
        .is_some());
    {
        let conn = reopened.conn.lock().expect("database lock");
        assert_eq!(
            oauth_row_count(&conn, "SELECT COUNT(*) FROM oauth_access_tokens"),
            1
        );
        assert_oauth_secret_schema(&conn);
    }

    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn plaintext_oauth_secrets_work_after_hash_migration() {
    let (store, data_dir) = test_store();
    store
        .oauth_store_client(&test_oauth_client("client_existing"))
        .expect("client");
    store.close().expect("close");

    {
        let conn = Connection::open(data_dir.join("messages.db")).expect("open database");
        conn.execute_batch(
            "
                DROP TABLE oauth_access_tokens;
                DROP TABLE oauth_refresh_tokens;
                DROP TABLE oauth_authorization_codes;
                CREATE TABLE oauth_access_tokens (
                    token TEXT PRIMARY KEY,
                    client_id TEXT NOT NULL,
                    scope TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    expires_at INTEGER NOT NULL,
                    refresh_token TEXT
                );
                CREATE TABLE oauth_refresh_tokens (
                    token TEXT PRIMARY KEY,
                    client_id TEXT NOT NULL,
                    scope TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    expires_at INTEGER NOT NULL
                );
                CREATE TABLE oauth_authorization_codes (
                    code TEXT PRIMARY KEY,
                    client_id TEXT NOT NULL,
                    redirect_uri TEXT NOT NULL,
                    code_challenge TEXT NOT NULL,
                    code_challenge_method TEXT NOT NULL,
                    scope TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    expires_at INTEGER NOT NULL,
                    used INTEGER DEFAULT 0
                );
                INSERT INTO oauth_refresh_tokens (token, client_id, scope, created_at, expires_at)
                VALUES ('legacy-refresh', 'client_existing', 'whatsapp.read', 1700000000, 9223372036854775807);
                INSERT INTO oauth_access_tokens (token, client_id, scope, created_at, expires_at, refresh_token)
                VALUES ('legacy-access', 'client_existing', 'whatsapp.read', 1700000000, 9223372036854775807, 'legacy-refresh');
                INSERT INTO oauth_access_tokens (token, client_id, scope, created_at, expires_at, refresh_token)
                VALUES ('expired-access', 'client_existing', 'whatsapp.read', 1700000000, 1, NULL);
                INSERT INTO oauth_authorization_codes (
                    code, client_id, redirect_uri, code_challenge, code_challenge_method, scope, created_at, expires_at, used
                ) VALUES (
                    'legacy-code', 'client_existing', 'http://127.0.0.1:8787/callback', 'challenge', 'S256', 'whatsapp.read', 1700000000, 9223372036854775807, 0
                );
                ",
        )
        .expect("install plaintext oauth rows");
    }

    let store = MessageStore::new(&data_dir).expect("migrate");
    assert!(store
        .oauth_validate_access_token("legacy-access")
        .expect("legacy access")
        .is_some());
    assert!(store
        .oauth_validate_access_token("expired-access")
        .expect("expired access")
        .is_none());
    assert!(store
        .oauth_get_refresh_token("legacy-refresh")
        .expect("legacy refresh")
        .is_some());
    assert!(store
        .oauth_use_authorization_code("legacy-code")
        .expect("legacy code")
        .is_some());
    {
        let conn = store.conn.lock().expect("database lock");
        assert_oauth_secret_schema(&conn);
        let link: String = conn
            .query_row(
                "SELECT refresh_token_hash FROM oauth_access_tokens WHERE token_hash = ?1",
                params![oauth_secret_hash("legacy-access")],
                |row| row.get(0),
            )
            .expect("link");
        assert_eq!(link, oauth_secret_hash("legacy-refresh"));
        assert_ne!(link, "legacy-refresh");
        assert_eq!(
            oauth_row_count(&conn, "SELECT COUNT(*) FROM oauth_access_tokens"),
            2
        );
        assert_eq!(
            oauth_row_count(&conn, "SELECT COUNT(*) FROM oauth_refresh_tokens"),
            1
        );
        assert_eq!(
            oauth_row_count(&conn, "SELECT COUNT(*) FROM oauth_authorization_codes"),
            1
        );
    }

    store
        .oauth_revoke_access_session("legacy-access")
        .expect("revoke migrated grant");
    assert!(store
        .oauth_validate_access_token("legacy-access")
        .expect("revoked access")
        .is_none());
    assert!(store
        .oauth_get_refresh_token("legacy-refresh")
        .expect("revoked refresh")
        .is_none());
    assert!(store
        .oauth_validate_access_token("expired-access")
        .expect("expired row remains unvalidated")
        .is_none());

    drop(store);
    let reopened = MessageStore::new(&data_dir).expect("second migration");
    {
        let conn = reopened.conn.lock().expect("database lock");
        assert_oauth_secret_schema(&conn);
        assert_eq!(
            oauth_row_count(&conn, "SELECT COUNT(*) FROM oauth_access_tokens"),
            1
        );
        assert_eq!(
            oauth_row_count(&conn, "SELECT COUNT(*) FROM oauth_refresh_tokens"),
            0
        );
    }

    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn oauth_hash_migration_resumes_a_partial_backfill() {
    let (store, data_dir) = test_store();
    store
        .oauth_store_client(&test_oauth_client("client_existing"))
        .expect("client");
    store.close().expect("close");

    let access_done = oauth_secret_hash("access-done");
    let refresh_done = oauth_secret_hash("refresh-done");
    let refresh_link = oauth_secret_hash("refresh-link");
    {
        let conn = Connection::open(data_dir.join("messages.db")).expect("open database");
        conn.execute_batch(
            "
                DROP TABLE oauth_access_tokens;
                DROP TABLE oauth_refresh_tokens;
                DROP TABLE oauth_authorization_codes;
                CREATE TABLE oauth_access_tokens (
                    token TEXT PRIMARY KEY,
                    client_id TEXT NOT NULL,
                    scope TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    expires_at INTEGER NOT NULL,
                    refresh_token TEXT,
                    token_hash TEXT,
                    refresh_token_hash TEXT
                );
                CREATE TABLE oauth_refresh_tokens (
                    token TEXT PRIMARY KEY,
                    client_id TEXT NOT NULL,
                    scope TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    expires_at INTEGER NOT NULL,
                    token_hash TEXT
                );
                CREATE TABLE oauth_authorization_codes (
                    code TEXT PRIMARY KEY,
                    client_id TEXT NOT NULL,
                    redirect_uri TEXT NOT NULL,
                    code_challenge TEXT NOT NULL,
                    code_challenge_method TEXT NOT NULL,
                    scope TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    expires_at INTEGER NOT NULL,
                    used INTEGER DEFAULT 0,
                    code_hash TEXT
                );
                ",
        )
        .expect("partial schema");
        conn.execute(
            "
                INSERT INTO oauth_access_tokens (
                    token, client_id, scope, created_at, expires_at, refresh_token, token_hash, refresh_token_hash
                ) VALUES ('access-done', 'client_existing', 'whatsapp.read', 1700000000, 9223372036854775807, 'refresh-done', ?1, ?2)
                ",
            params![access_done, refresh_done],
        )
        .expect("finished access row");
        conn.execute(
            "
                INSERT INTO oauth_access_tokens (
                    token, client_id, scope, created_at, expires_at, refresh_token, token_hash, refresh_token_hash
                ) VALUES ('access-link-pending', 'client_existing', 'whatsapp.read', 1700000000, 9223372036854775807, 'refresh-link', ?1, NULL)
                ",
            params![oauth_secret_hash("access-link-pending")],
        )
        .expect("link pending row");
        conn.execute(
            "
                INSERT INTO oauth_access_tokens (
                    token, client_id, scope, created_at, expires_at, refresh_token, token_hash, refresh_token_hash
                ) VALUES ('access-pending', 'client_existing', 'whatsapp.read', 1700000000, 9223372036854775807, 'refresh-pending', NULL, NULL)
                ",
            [],
        )
        .expect("pending access row");
        conn.execute(
            "
                INSERT INTO oauth_refresh_tokens (token, client_id, scope, created_at, expires_at, token_hash)
                VALUES ('refresh-done', 'client_existing', 'whatsapp.read', 1700000000, 9223372036854775807, ?1)
                ",
            params![refresh_done],
        )
        .expect("finished refresh row");
        for token in ["refresh-link", "refresh-pending"] {
            conn.execute(
                "
                    INSERT INTO oauth_refresh_tokens (token, client_id, scope, created_at, expires_at, token_hash)
                    VALUES (?1, 'client_existing', 'whatsapp.read', 1700000000, 9223372036854775807, NULL)
                    ",
                params![token],
            )
            .expect("pending refresh row");
        }
        conn.execute(
            "
                INSERT INTO oauth_authorization_codes (
                    code, client_id, redirect_uri, code_challenge, code_challenge_method, scope, created_at, expires_at, used, code_hash
                ) VALUES (
                    'code-done', 'client_existing', 'http://127.0.0.1:8787/callback', 'challenge', 'S256', 'whatsapp.read', 1700000000, 9223372036854775807, 0, ?1
                )
                ",
            params![oauth_secret_hash("code-done")],
        )
        .expect("finished code");
        conn.execute(
            "
                INSERT INTO oauth_authorization_codes (
                    code, client_id, redirect_uri, code_challenge, code_challenge_method, scope, created_at, expires_at, used, code_hash
                ) VALUES (
                    'code-pending', 'client_existing', 'http://127.0.0.1:8787/callback', 'challenge', 'S256', 'whatsapp.read', 1700000000, 9223372036854775807, 0, NULL
                )
                ",
            [],
        )
        .expect("pending code");
    }

    let store = MessageStore::new(&data_dir).expect("resume migration");
    for token in ["access-done", "access-link-pending", "access-pending"] {
        assert!(
            store
                .oauth_validate_access_token(token)
                .expect("validate")
                .is_some(),
            "{token}"
        );
    }
    for token in ["refresh-done", "refresh-link", "refresh-pending"] {
        assert!(
            store
                .oauth_get_refresh_token(token)
                .expect("refresh")
                .is_some(),
            "{token}"
        );
    }
    assert!(store
        .oauth_use_authorization_code("code-pending")
        .expect("pending code")
        .is_some());
    {
        let conn = store.conn.lock().expect("database lock");
        assert_oauth_secret_schema(&conn);
        assert_eq!(
            oauth_row_count(&conn, "SELECT COUNT(*) FROM oauth_access_tokens"),
            3
        );
        assert_eq!(
            oauth_row_count(
                &conn,
                "SELECT COUNT(DISTINCT token_hash) FROM oauth_access_tokens"
            ),
            3
        );
        assert_eq!(
            oauth_row_count(&conn, "SELECT COUNT(*) FROM oauth_refresh_tokens"),
            3
        );
        assert_eq!(
            oauth_row_count(&conn, "SELECT COUNT(*) FROM oauth_authorization_codes"),
            2
        );
        let done_link: String = conn
            .query_row(
                "SELECT refresh_token_hash FROM oauth_access_tokens WHERE token_hash = ?1",
                params![access_done],
                |row| row.get(0),
            )
            .expect("done link");
        assert_eq!(done_link, refresh_done);
        let pending_link: String = conn
            .query_row(
                "SELECT refresh_token_hash FROM oauth_access_tokens WHERE token_hash = ?1",
                params![oauth_secret_hash("access-link-pending")],
                |row| row.get(0),
            )
            .expect("pending link");
        assert_eq!(pending_link, refresh_link);
        let pending_access: String = conn
            .query_row(
                "SELECT token_hash FROM oauth_access_tokens WHERE token_hash = ?1",
                params![oauth_secret_hash("access-pending")],
                |row| row.get(0),
            )
            .expect("pending access");
        assert_eq!(pending_access, oauth_secret_hash("access-pending"));
    }

    store
        .oauth_revoke_access_session("access-done")
        .expect("revoke finished grant");
    assert!(store
        .oauth_get_refresh_token("refresh-done")
        .expect("revoked refresh")
        .is_none());
    assert!(store
        .oauth_validate_access_token("access-pending")
        .expect("other access")
        .is_some());
    assert!(store
        .oauth_get_refresh_token("refresh-pending")
        .expect("other refresh")
        .is_some());

    drop(store);
    let reopened = MessageStore::new(&data_dir).expect("run migration again");
    assert!(reopened
        .oauth_validate_access_token("access-pending")
        .expect("still valid")
        .is_some());
    assert!(reopened
        .oauth_validate_access_token("access-done")
        .expect("stays revoked")
        .is_none());
    {
        let conn = reopened.conn.lock().expect("database lock");
        assert_oauth_secret_schema(&conn);
        assert_eq!(
            oauth_row_count(&conn, "SELECT COUNT(*) FROM oauth_access_tokens"),
            2
        );
        assert_eq!(
            oauth_row_count(&conn, "SELECT COUNT(*) FROM oauth_refresh_tokens"),
            2
        );
        assert_eq!(
            oauth_row_count(&conn, "SELECT COUNT(*) FROM oauth_authorization_codes"),
            2
        );
    }

    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn hashed_oauth_secrets_expire_and_cleanup() {
    let (store, data_dir) = test_store();
    store
        .oauth_store_client(&test_oauth_client("client_test"))
        .expect("client");
    let mut expired_access = linked_access_token("expired-access", "client_test", None);
    expired_access.expires_at = 1;
    store
        .oauth_store_access_token(&expired_access)
        .expect("expired access");
    let mut expired_refresh = stored_refresh_token("expired-refresh", "client_test");
    expired_refresh.expires_at = 1;
    store
        .oauth_store_refresh_token(&expired_refresh)
        .expect("expired refresh");
    let mut expired_code = authorization_code("expired-code");
    expired_code.expires_at = 1;
    store
        .oauth_store_authorization_code(&expired_code)
        .expect("expired code");
    store
        .oauth_store_access_token(&linked_access_token("live-access", "client_test", None))
        .expect("live access");

    assert!(store
        .oauth_validate_access_token("expired-access")
        .expect("expired access")
        .is_none());
    assert!(store
        .oauth_get_refresh_token("expired-refresh")
        .expect("expired refresh")
        .is_none());
    assert!(store
        .oauth_use_authorization_code("expired-code")
        .expect("expired code")
        .is_none());
    assert!(store
        .oauth_validate_access_token("live-access")
        .expect("live access")
        .is_some());

    store.oauth_cleanup_expired().expect("cleanup");
    {
        let conn = store.conn.lock().expect("database lock");
        assert_eq!(
            oauth_row_count(&conn, "SELECT COUNT(*) FROM oauth_access_tokens"),
            1
        );
        assert_eq!(
            oauth_row_count(&conn, "SELECT COUNT(*) FROM oauth_refresh_tokens"),
            0
        );
        assert_eq!(
            oauth_row_count(&conn, "SELECT COUNT(*) FROM oauth_authorization_codes"),
            0
        );
    }
    assert!(store
        .oauth_validate_access_token("live-access")
        .expect("live access after cleanup")
        .is_some());

    let _ = std::fs::remove_dir_all(data_dir);
}
