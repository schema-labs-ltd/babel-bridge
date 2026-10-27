use super::*;

#[test]
fn initial_topic_import_covers_seven_days_across_chats_without_reclassifying_or_expanding_history()
{
    let (store, path) = test_store();
    let now = 1_800_000_000_000_i64;
    let since = now - 7 * 24 * 60 * 60 * 1000;
    for i in 0..205 {
        add(&store, "one@g.us", &format!("recent-{i:03}"), since + i);
    }
    add(&store, "two@s.whatsapp.net", "direct", now);
    add(&store, "old@g.us", "too-old", since - 1);
    add(&store, "future@g.us", "future", now + 1);
    add(&store, "status@broadcast", "status", now);
    add(&store, "empty@g.us", "empty", now);
    add(&store, "one@g.us", "reaction", now);
    store
        .conn
        .lock()
        .unwrap()
        .execute("UPDATE messages SET original_text=' ' WHERE id='empty'", [])
        .unwrap();
    store
        .conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE messages SET content_type='reaction' WHERE id='reaction'",
            [],
        )
        .unwrap();
    let preview = store.topic_import_preview(now).unwrap();
    assert_eq!(
        (preview.days, preview.chat_count, preview.message_count),
        (7, 2, 206)
    );
    assert!(store.topic_settings().unwrap().is_empty()); // A preview does not opt in or enqueue anything.
    let imported = store.import_recent_topics(now).unwrap();
    assert_eq!((imported.chat_count, imported.message_count), (2, 206));
    assert_eq!(
        store
            .topic_settings()
            .unwrap()
            .iter()
            .map(|s| s.pending_count)
            .sum::<i64>(),
        206
    );
    let in_flight = store.next_topic_batch().unwrap().unwrap();
    store.import_recent_topics(now).unwrap();
    finish(&store, &in_flight, "Plans"); // A repeated import preserves a running batch's epoch.
    while let Some(batch) = store.next_topic_batch().unwrap() {
        finish(&store, &batch, "Plans");
    }
    assert_eq!(
        store
            .list_topics()
            .unwrap()
            .iter()
            .map(|t| t.message_count)
            .sum::<i64>(),
        206
    );
    assert_eq!(store.topic_import_preview(now).unwrap().message_count, 0);
    assert_eq!(store.import_recent_topics(now).unwrap().message_count, 0);
    add(&store, "two@s.whatsapp.net", "new-after-import", now + 1000);
    assert_eq!(
        store.next_topic_batch().unwrap().unwrap().messages[0].id,
        "new-after-import"
    );
    assert_eq!(store.topic_settings().unwrap().len(), 2);
    drop(store);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn topics_are_opt_in_bounded_persistent_and_separate_even_when_titles_match() {
    let (store, path) = test_store();
    for i in 0..205 {
        add(&store, "one@g.us", &format!("a{i:03}"), i);
    }
    add(&store, "two@g.us", "second-chat", 300);
    assert!(store.next_topic_batch().unwrap().is_none());
    store.set_topics_enabled("one@g.us", true).unwrap();
    assert_eq!(store.topic_settings().unwrap()[0].pending_count, 200);
    while let Some(batch) = store.next_topic_batch().unwrap() {
        finish(&store, &batch, "Weekend plans");
    }
    let first = store.list_topics().unwrap().remove(0);
    assert_eq!(first.message_count, 200);
    store.set_topics_enabled("two@g.us", true).unwrap();
    finish(
        &store,
        &store.next_topic_batch().unwrap().unwrap(),
        "Weekend plans",
    );
    let topics = store.list_topics().unwrap();
    assert_eq!(topics.len(), 2);
    assert_ne!(topics[0].id, topics[1].id);
    assert_eq!(
        store
            .topic_messages(&first.id, 250, None, None)
            .unwrap()
            .unwrap()
            .iter()
            .map(|m| m.contact_id.as_str())
            .collect::<std::collections::HashSet<_>>(),
        std::collections::HashSet::from(["one@g.us"])
    );
    drop(store);
    let store = MessageStore::new(&path).unwrap();
    assert_eq!(store.list_topics().unwrap().len(), 2);
    assert!(store.next_topic_batch().unwrap().is_none());
    store.clear_all().unwrap();
    assert!(store.list_topics().unwrap().is_empty());
    assert!(store.topic_settings().unwrap().is_empty());
    drop(store);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn topic_edits_requeue_and_stale_results_cannot_restore_membership() {
    let (store, path) = test_store();
    add(&store, "one@g.us", "a", 100);
    store.set_topics_enabled("one@g.us", true).unwrap();
    let original = store.next_topic_batch().unwrap().unwrap();
    finish(&store, &original, "Food");
    assert_eq!(store.list_topics().unwrap().len(), 1);
    store
        .conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE messages SET original_text='Football',content_json=? WHERE id='a'",
            [r#"{"type":"text","body":"Football","edited_at_ms":200}"#],
        )
        .unwrap();
    assert!(store.list_topics().unwrap().is_empty());
    finish(&store, &original, "Food");
    assert!(store.list_topics().unwrap().is_empty());
    let edited = store.next_topic_batch().unwrap().unwrap();
    assert_eq!(
        edited.messages[0].original_text.as_deref(),
        Some("Football")
    );
    finish(&store, &edited, "Sport");
    assert_eq!(store.list_topics().unwrap()[0].title, "Sport");
    // A reaction/translation-only update does not incur another classification.
    store.conn.lock().unwrap().execute("UPDATE messages SET translated_text='Sport',content_json=json_set(content_json,'$.reactions',json('{}')) WHERE id='a'",[]).unwrap();
    assert!(store.next_topic_batch().unwrap().is_none());
    drop(store);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn disabling_or_reenabling_a_chat_invalidates_inflight_results_and_retries_are_bounded() {
    let (store, path) = test_store();
    add(&store, "one@g.us", "a", 100);
    store.set_topics_enabled("one@g.us", true).unwrap();
    let old = store.next_topic_batch().unwrap().unwrap();
    store.set_topics_enabled("one@g.us", false).unwrap();
    finish(&store, &old, "Plans");
    assert!(store.list_topics().unwrap().is_empty());
    assert!(store.next_topic_batch().unwrap().is_none());
    store.set_topics_enabled("one@g.us", true).unwrap();
    finish(&store, &old, "Plans");
    assert!(store.list_topics().unwrap().is_empty());
    let current = store.next_topic_batch().unwrap().unwrap();
    for _ in 0..3 {
        store.retry_topic_batch(&current).unwrap();
    }
    assert!(store.next_topic_batch().unwrap().is_none());
    assert_eq!(store.topic_settings().unwrap()[0].failed_count, 1);
    store.set_topics_enabled("one@g.us", true).unwrap();
    let retried = store.next_topic_batch().unwrap().unwrap();
    finish(&store, &retried, "Plans");
    let id = store.list_topics().unwrap()[0].id.clone();
    store.set_topics_enabled("one@g.us", false).unwrap();
    assert!(store.topic_messages(&id, 50, None, None).unwrap().is_none());
    drop(store);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn topic_pagination_preserves_ties_and_rejects_cross_chat_assignments() {
    let (store, path) = test_store();
    for id in ["a", "b", "c"] {
        add(&store, "one@g.us", id, 100);
    }
    add(&store, "two@g.us", "foreign", 100);
    store.set_topics_enabled("one@g.us", true).unwrap();
    let batch = store.next_topic_batch().unwrap().unwrap();
    for invalid in [
        r#"{"assignments":[{"messageId":"foreign","topic":"Oops"}]}"#,
        r#"{"assignments":[{"messageId":"a","topic":"X"},{"messageId":"a","topic":"X"},{"messageId":"c","topic":"X"}]}"#,
    ] {
        assert!(crate::topics::parse_assignments(invalid, &batch).is_err());
    }
    finish(&store, &batch, "Plans");
    let topic = &store.list_topics().unwrap()[0];
    let page = store
        .topic_messages(&topic.id, 2, None, None)
        .unwrap()
        .unwrap();
    assert_eq!(
        page.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        vec!["b", "c"]
    );
    let older = store
        .topic_messages(&topic.id, 2, Some(100), Some("b"))
        .unwrap()
        .unwrap();
    assert_eq!(older[0].id, "a");
    assert!(store.reply_is_latest("one@g.us", "c").unwrap());
    assert!(!store.reply_is_latest("one@g.us", "a").unwrap());
    store.delete_message("b").ok();
    drop(store);
    std::fs::remove_dir_all(path).unwrap();
}
