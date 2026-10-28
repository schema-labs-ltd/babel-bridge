use super::*;

#[test]
fn organising_counts_exclude_disabled_and_invalid_legacy_assignments() {
    let (store, path) = test_store();
    for id in ["a", "b", "c"] {
        add(&store, "disabled@g.us", id, 100);
    }
    store.set_topics_enabled("disabled@g.us", true).unwrap();
    finish(
        &store,
        &store.next_topic_batch().unwrap().unwrap(),
        "Hospital",
    );
    seed_legacy_migration(&store);
    assert_eq!(store.topic_settings().unwrap()[0].pending_count, 3);
    store.set_topics_enabled("disabled@g.us", false).unwrap();
    assert!(store.next_topic_labels().unwrap().is_empty());
    assert_eq!(store.topic_settings().unwrap()[0].pending_count, 0);
    // Retaining paused work is intentional: opting back in resumes it.
    store.set_topics_enabled("disabled@g.us", true).unwrap();
    assert_eq!(store.topic_settings().unwrap()[0].pending_count, 3);
    store
        .upsert_contact("wrong@g.us", None, None, Some("group"), 100)
        .unwrap();
    store
        .conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE messages SET contact_id='wrong@g.us' WHERE id='a'",
            [],
        )
        .unwrap();
    assert_eq!(store.topic_settings().unwrap()[0].pending_count, 2);
    store
        .conn
        .lock()
        .unwrap()
        .execute("UPDATE topic_label_jobs SET attempts=3", [])
        .unwrap();
    assert_eq!(store.topic_settings().unwrap()[0].failed_count, 2);
    store.set_topics_enabled("disabled@g.us", false).unwrap();
    assert_eq!(store.topic_settings().unwrap()[0].failed_count, 0);
    store.log_topic_queue_health().unwrap();
    drop(store);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn message_topic_inspection_tracks_saved_assignment_edit_retry_and_opt_out() {
    let (store, path) = test_store();
    add(&store, "family@g.us", "a", 100);
    let ids = vec!["a".into(), "missing".into()];
    let status = || store.message_topics(&ids).unwrap().remove(0);
    assert_eq!(store.message_topics(&ids).unwrap().len(), 1);
    assert_eq!(status().state, "off");
    store.set_topics_enabled("family@g.us", true).unwrap();
    assert_eq!(status().state, "pending");
    finish(
        &store,
        &store.next_topic_batch().unwrap().unwrap(),
        "Hospital",
    );
    assert_eq!(status().title.as_deref(), Some("Hospital"));
    store.set_topics_enabled("family@g.us", false).unwrap();
    assert_eq!(status().title.as_deref(), Some("Hospital"));
    store.set_topics_enabled("family@g.us", true).unwrap();
    store.conn.lock().unwrap().execute("UPDATE messages SET original_text='Shopping now',content_json=json_set(content_json,'$.edited_at_ms',200) WHERE id='a'", []).unwrap();
    assert_eq!(status().state, "pending");
    assert!(status().title.is_none());
    assert_eq!(status().revision, 200);
    let batch = store.next_topic_batch().unwrap().unwrap();
    for _ in 0..3 {
        store.retry_topic_batch(&batch).unwrap();
    }
    assert_eq!(status().state, "failed");
    store.set_topics_enabled("family@g.us", true).unwrap();
    finish(
        &store,
        &store.next_topic_batch().unwrap().unwrap(),
        "Shopping",
    );
    assert_eq!(status().title.as_deref(), Some("Shopping"));
    // A corrupt cross-chat assignment must never leak a misleading title.
    store
        .conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE chat_topics SET contact_id='other@g.us' WHERE title='Shopping'",
            [],
        )
        .unwrap();
    assert!(status().title.is_none());
    assert_eq!(status().state, "unassigned");
    drop(store);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn broad_labels_merge_existing_topics_and_preserve_messages_across_restart() {
    let (store, path) = test_store();
    legacy_topic(
        &store,
        "family@g.us",
        "a",
        "Hospital support during recovery",
    );
    legacy_topic(
        &store,
        "family@g.us",
        "b",
        "Procedure delay and discharge plans",
    );
    legacy_topic(
        &store,
        "friend@s.whatsapp.net",
        "c",
        "Hospital recovery and surgery",
    );
    legacy_topic(
        &store,
        "family@g.us",
        "d",
        "Grocery shopping and meal requests",
    );
    seed_legacy_migration(&store);
    let batch = store.next_topic_labels().unwrap();
    assert_eq!(batch.len(), 4);
    let before = store.get_message_by_id("b").unwrap().unwrap().content_json;
    let assignments: Vec<_> = batch
        .iter()
        .map(|label| TopicAssignment {
            message_id: label.id.clone(),
            topic: if label.title.starts_with("Grocery") {
                "Shopping"
            } else {
                "Hospital"
            }
            .into(),
        })
        .collect();
    assert_eq!(store.finish_topic_labels(&batch, &assignments).unwrap(), 4);
    assert!(store.next_topic_labels().unwrap().is_empty());
    assert_eq!(
        store.get_message_by_id("b").unwrap().unwrap().content_json,
        before
    );
    let topics = store.list_topics().unwrap();
    assert_eq!(topics.len(), 3);
    let category = crate::topics::category_id("hospital");
    let messages = store
        .topic_messages(&category, 10, None, None)
        .unwrap()
        .unwrap();
    assert_eq!(
        messages.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["a", "b", "c"]
    );
    assert_eq!(messages[2].contact_id, "friend@s.whatsapp.net");
    assert!(store
        .topic_settings()
        .unwrap()
        .iter()
        .all(|s| s.pending_count == 0));
    drop(store);
    let store = MessageStore::new(&path).unwrap();
    assert!(store.next_topic_labels().unwrap().is_empty());
    assert_eq!(
        store
            .topic_messages(&category, 10, None, None)
            .unwrap()
            .unwrap()
            .len(),
        3
    );
    drop(store);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn broad_label_migration_rejects_invalid_output_and_in_flight_opt_out() {
    let (store, path) = test_store();
    legacy_topic(&store, "family@g.us", "a", "Hospital support");
    legacy_topic(&store, "disabled@g.us", "b", "Hospital recovery");
    store.set_topics_enabled("disabled@g.us", false).unwrap();
    seed_legacy_migration(&store);
    let batch = store.next_topic_labels().unwrap();
    assert_eq!(batch.len(), 1);
    let bad = [TopicAssignment {
        message_id: batch[0].id.clone(),
        topic: "A label with too many words".into(),
    }];
    assert!(store.finish_topic_labels(&batch, &bad).is_err());
    assert_eq!(store.list_topics().unwrap()[0].title, "Hospital support");
    let valid = [TopicAssignment {
        message_id: batch[0].id.clone(),
        topic: "Hospital".into(),
    }];
    store.set_topics_enabled("family@g.us", false).unwrap();
    assert_eq!(store.finish_topic_labels(&batch, &valid).unwrap(), 0);
    store.set_topics_enabled("family@g.us", true).unwrap();
    assert_eq!(store.finish_topic_labels(&batch, &valid).unwrap(), 0); // epoch changed
    let current = store.next_topic_labels().unwrap();
    assert_eq!(store.finish_topic_labels(&current, &valid).unwrap(), 1);
    store.set_topics_enabled("disabled@g.us", true).unwrap();
    let retry = store.next_topic_labels().unwrap();
    assert_eq!(retry.len(), 1);
    for _ in 0..3 {
        store.retry_topic_labels(&retry).unwrap();
        store
            .conn
            .lock()
            .unwrap()
            .execute("UPDATE topic_label_jobs SET retry_at=0", [])
            .unwrap();
    }
    assert!(store.next_topic_labels().unwrap().is_empty());
    assert_eq!(
        store
            .topic_settings()
            .unwrap()
            .iter()
            .find(|s| s.contact_id == "disabled@g.us")
            .unwrap()
            .failed_count,
        1
    );
    store.clear_all().unwrap();
    assert!(store.next_topic_labels().unwrap().is_empty());
    let count: i64 = store
        .conn
        .lock()
        .unwrap()
        .query_row("SELECT count(*) FROM topic_label_jobs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
    drop(store);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn label_mappings_apply_simultaneously_without_cascading_between_sources() {
    let (store, path) = test_store();
    legacy_topic(&store, "family@g.us", "a", "Travel");
    legacy_topic(&store, "family@g.us", "b", "Holidays");
    seed_legacy_migration(&store);
    let batch = store.next_topic_labels().unwrap();
    let assignments: Vec<_> = batch
        .iter()
        .map(|label| TopicAssignment {
            message_id: label.id.clone(),
            topic: if label.title == "Travel" {
                "Holidays"
            } else {
                "Weekend plans"
            }
            .into(),
        })
        .collect();
    store.finish_topic_labels(&batch, &assignments).unwrap();
    for (key, id) in [("holidays", "a"), ("weekend plans", "b")] {
        let messages = store
            .topic_messages(&crate::topics::category_id(key), 10, None, None)
            .unwrap()
            .unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].id, id);
    }
    drop(store);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn unified_categories_merge_people_and_groups_with_stable_pagination_and_opt_out() {
    let (store, path) = test_store();
    for (chat, id, title) in [
        ("family@g.us", "a", "Birthday wishes"),
        ("447700900123@s.whatsapp.net", "b", "birthday wishes"),
        ("friends@g.us", "c", "Birthday wishes"),
    ] {
        add(&store, chat, id, 100);
        store.set_topics_enabled(chat, true).unwrap();
        finish(&store, &store.next_topic_batch().unwrap().unwrap(), title);
    }
    let topics = store.list_topics().unwrap();
    let category = crate::topics::category_id("birthday wishes");
    assert!(topics.iter().all(|t| t.category_id == category));
    assert_eq!(store.topic_names("new@g.us").unwrap().len(), 1);
    let recent = store
        .topic_messages(&category, 2, None, None)
        .unwrap()
        .unwrap();
    assert_eq!(
        recent.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["b", "c"]
    );
    let earlier = store
        .topic_messages(&category, 2, Some(100), Some("b"))
        .unwrap()
        .unwrap();
    assert_eq!(earlier[0].id, "a");
    assert_eq!(earlier[0].contact_id, "family@g.us");
    let per_chat = topics
        .iter()
        .find(|t| t.contact_id == "family@g.us")
        .unwrap();
    assert_eq!(
        store
            .topic_messages(&per_chat.id, 10, None, None)
            .unwrap()
            .unwrap()
            .len(),
        1
    );
    store.set_topics_enabled("family@g.us", false).unwrap();
    assert_eq!(
        store
            .topic_messages(&category, 10, None, None)
            .unwrap()
            .unwrap()
            .len(),
        2
    );
    assert!(store
        .topic_messages(&per_chat.id, 10, None, None)
        .unwrap()
        .is_none());
    assert!(store
        .topic_messages("category:not-valid!", 10, None, None)
        .unwrap()
        .is_none());
    drop(store);
    let store = MessageStore::new(&path).unwrap();
    assert_eq!(
        store
            .topic_messages(&category, 10, None, None)
            .unwrap()
            .unwrap()
            .len(),
        2
    );
    drop(store);
    std::fs::remove_dir_all(path).unwrap();
}
