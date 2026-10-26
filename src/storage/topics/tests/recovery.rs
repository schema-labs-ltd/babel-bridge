use super::*;

#[test]
fn compact_batch_fix_recovers_only_enabled_label_jobs_once() {
    let (store, path) = test_store();
    legacy_topic(&store, "family@g.us", "a", "Hospital support");
    legacy_topic(&store, "disabled@g.us", "b", "Hospital recovery");
    seed_legacy_migration(&store);
    {
        let conn = store.conn.lock().unwrap();
        conn.execute(
            "UPDATE topic_label_jobs SET attempts=3,retry_at=9999999999",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE topic_settings SET enabled=0 WHERE contact_id='disabled@g.us'",
            [],
        )
        .unwrap();
        conn.execute(
            "DELETE FROM app_settings WHERE key='topic_compact_batches_v1'",
            [],
        )
        .unwrap();
    }
    drop(store);
    let store = MessageStore::new(&path).unwrap();
    let batch = store.next_topic_labels().unwrap();
    assert_eq!(batch.len(), 1);
    assert_eq!(batch[0].contact_id, "family@g.us");
    assert_eq!(
        store
            .topic_settings()
            .unwrap()
            .iter()
            .find(|s| s.contact_id == "disabled@g.us")
            .unwrap()
            .failed_count,
        0
    );
    assert_eq!(store.conn.lock().unwrap().query_row("SELECT attempts FROM topic_label_jobs j JOIN chat_topics t ON t.id=j.topic_id WHERE t.contact_id='disabled@g.us'", [], |r| r.get::<_,i64>(0)).unwrap(), 3);
    assert_eq!(store.list_topics().unwrap()[0].title, "Hospital support");
    for _ in 0..3 {
        store.retry_topic_labels(&batch).unwrap();
    }
    drop(store);
    let store = MessageStore::new(&path).unwrap();
    assert!(store.next_topic_labels().unwrap().is_empty());
    assert_eq!(
        store
            .topic_settings()
            .unwrap()
            .iter()
            .find(|s| s.contact_id == "family@g.us")
            .unwrap()
            .failed_count,
        1
    );
    drop(store);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn compact_batch_recovery_completes_the_queue_in_bounded_chunks() {
    let (store, path) = test_store();
    for n in 0..19 {
        add(&store, "family@g.us", &format!("m{n:02}"), n);
    }
    store.set_topics_enabled("family@g.us", true).unwrap();
    {
        let conn = store.conn.lock().unwrap();
        conn.execute("UPDATE topic_jobs SET attempts=3,retry_at=9999999999", [])
            .unwrap();
        conn.execute(
            "DELETE FROM app_settings WHERE key='topic_compact_batches_v1'",
            [],
        )
        .unwrap();
    }
    drop(store);
    let store = MessageStore::new(&path).unwrap();
    let mut sizes = Vec::new();
    let mut ids = std::collections::HashSet::new();
    while let Some(batch) = store.next_topic_batch().unwrap() {
        sizes.push(batch.messages.len());
        for message in &batch.messages {
            assert!(ids.insert(message.id.clone()));
        }
        finish(&store, &batch, "Hospital");
    }
    assert_eq!(sizes, [8, 8, 3]);
    assert_eq!(ids.len(), 19);
    assert_eq!(store.list_topics().unwrap()[0].message_count, 19);
    assert_eq!(store.topic_settings().unwrap()[0].pending_count, 0);
    add(&store, "family@g.us", "later", 100);
    let later = store.next_topic_batch().unwrap().unwrap();
    for _ in 0..3 {
        store.retry_topic_batch(&later).unwrap();
    }
    drop(store);
    let store = MessageStore::new(&path).unwrap();
    assert!(store.next_topic_batch().unwrap().is_none());
    assert_eq!(store.topic_settings().unwrap()[0].failed_count, 1);
    drop(store);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn timeout_fix_recovers_exhausted_jobs_once_without_enabling_other_chats() {
    let (store, path) = test_store();
    add(&store, "family@g.us", "pending", 100);
    add(&store, "friends@g.us", "disabled", 100);
    store.set_topics_enabled("family@g.us", true).unwrap();
    let batch = store.next_topic_batch().unwrap().unwrap();
    for _ in 0..3 {
        store.retry_topic_batch(&batch).unwrap();
    }
    assert!(store.next_topic_batch().unwrap().is_none());
    store
        .conn
        .lock()
        .unwrap()
        .execute(
            "DELETE FROM app_settings WHERE key='topic_background_timeout_v1'",
            [],
        )
        .unwrap();
    drop(store);
    let store = MessageStore::new(&path).unwrap();
    let batch = store.next_topic_batch().unwrap().unwrap();
    assert_eq!(batch.messages.len(), 1);
    assert_eq!(batch.messages[0].id, "pending");
    assert_eq!(store.topic_settings().unwrap().len(), 1);
    for _ in 0..3 {
        store.retry_topic_batch(&batch).unwrap();
    }
    drop(store);
    let store = MessageStore::new(&path).unwrap();
    assert!(store.next_topic_batch().unwrap().is_none());
    drop(store);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn topic_json_fix_recovers_only_existing_enabled_jobs_once_across_restarts() {
    let (store, path) = test_store();
    add(&store, "enabled@g.us", "assigned", 100);
    store.set_topics_enabled("enabled@g.us", true).unwrap();
    finish(
        &store,
        &store.next_topic_batch().unwrap().unwrap(),
        "Saved topic",
    );
    add(&store, "enabled@g.us", "failed", 200);
    add(&store, "enabled@g.us", "waiting", 300);
    add(&store, "disabled@g.us", "disabled", 400);
    store.set_topics_enabled("disabled@g.us", true).unwrap();
    add(&store, "never-enabled@g.us", "untouched", 500);
    {
        let conn = store.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM app_settings WHERE key='topic_json_input_v1'",
            [],
        )
        .unwrap();
        conn.execute("UPDATE topic_jobs SET attempts=3,retry_at=9999999999", [])
            .unwrap();
        conn.execute(
            "UPDATE topic_jobs SET attempts=1 WHERE message_id='waiting'",
            [],
        )
        .unwrap();
        // A stale disabled-chat job is deliberately retained to verify the recovery scope.
        conn.execute(
            "UPDATE topic_settings SET enabled=0 WHERE contact_id='disabled@g.us'",
            [],
        )
        .unwrap();
    }
    drop(store);
    let store = MessageStore::new(&path).unwrap();
    let enabled = store
        .topic_settings()
        .unwrap()
        .into_iter()
        .find(|s| s.contact_id == "enabled@g.us")
        .unwrap();
    assert_eq!((enabled.pending_count, enabled.failed_count), (2, 0));
    assert_eq!(store.list_topics().unwrap()[0].message_count, 1);
    let batch = store.next_topic_batch().unwrap().unwrap();
    assert_eq!(
        batch
            .messages
            .iter()
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>(),
        vec!["failed", "waiting"]
    );
    {
        let conn = store.conn.lock().unwrap();
        assert_eq!(
            conn.query_row(
                "SELECT attempts FROM topic_jobs WHERE message_id='disabled'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            3
        );
        assert!(!conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM topic_jobs WHERE message_id='untouched')",
                [],
                |r| r.get::<_, bool>(0)
            )
            .unwrap());
    }
    for _ in 0..3 {
        store.retry_topic_batch(&batch).unwrap();
    }
    drop(store);
    let store = MessageStore::new(&path).unwrap();
    assert!(store.next_topic_batch().unwrap().is_none()); // Restart must not create endless retries.
    let enabled = store
        .topic_settings()
        .unwrap()
        .into_iter()
        .find(|s| s.contact_id == "enabled@g.us")
        .unwrap();
    assert_eq!((enabled.pending_count, enabled.failed_count), (0, 2));
    assert_eq!(store.list_topics().unwrap()[0].title, "Saved topic");
    drop(store);
    std::fs::remove_dir_all(path).unwrap();
}
