use super::*;

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
