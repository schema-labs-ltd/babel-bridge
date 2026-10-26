use super::*;
use crate::storage::tests::{test_message, test_store};

fn add(store: &MessageStore, chat: &str, id: &str, time: i64) {
    store
        .upsert_contact(chat, Some(chat), None, Some("group"), time)
        .unwrap();
    let mut m = test_message(id, time);
    m.contact_id = chat.into();
    m.chat_type = "group".into();
    store.add_message(&m).unwrap();
}
fn finish(store: &MessageStore, batch: &TopicBatch, title: &str) {
    let assignments = batch
        .messages
        .iter()
        .map(|m| TopicAssignment {
            message_id: m.id.clone(),
            topic: title.into(),
        })
        .collect::<Vec<_>>();
    store.finish_topic_batch(batch, &assignments).unwrap();
}

fn legacy_topic(store: &MessageStore, chat: &str, id: &str, title: &str) {
    add(store, chat, id, 100);
    store.set_topics_enabled(chat, true).unwrap();
    finish(
        store,
        &store.next_topic_batch().unwrap().unwrap(),
        "Temporary",
    );
    store.conn.lock().unwrap().execute("UPDATE chat_topics SET title=?1,name_key=?2 WHERE contact_id=?3 AND name_key='temporary'",params![title,title.to_lowercase(),chat]).unwrap();
}

fn seed_legacy_migration(store: &MessageStore) {
    store
        .conn
        .lock()
        .unwrap()
        .execute(
            "DELETE FROM app_settings WHERE key='topic_broad_labels_v1'",
            [],
        )
        .unwrap();
    store.queue_legacy_topic_labels().unwrap();
}

mod labels;
mod membership;
mod queue;
mod recovery;
