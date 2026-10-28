use super::*;

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
