use crate::message_tones::MessageTone;
#[test]
fn notification_payload_uses_bundled_tones_and_silent_keeps_the_alert() {
    let original = PushNotification::test();
    let audible = PushNotification::test().with_message_tone(MessageTone::Orbit);
    assert_eq!(audible.payload["aps"]["sound"], "bb-orbit.wav");
    let silent = audible.with_message_tone(MessageTone::Silent);
    assert!(silent.payload["aps"].get("sound").is_none());
    assert_eq!(
        silent.payload["aps"]["alert"],
        original.payload["aps"]["alert"]
    );
    let standard = silent.with_message_tone(MessageTone::Default);
    assert_eq!(standard.payload["aps"]["sound"], "default");
}

use super::*;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

#[tokio::test]
async fn provider_token_cache_reuses_a_token_within_the_refresh_window() {
    let cache = ProviderTokenCache::default();
    let sign_count = Arc::new(AtomicUsize::new(0));

    let first_count = Arc::clone(&sign_count);
    let first = cache
        .get_or_create(1_000, move |issued_at| {
            first_count.fetch_add(1, Ordering::SeqCst);
            Ok(format!("token-{issued_at}"))
        })
        .await
        .expect("create provider token");
    let second_count = Arc::clone(&sign_count);
    let second = cache
        .get_or_create(1_001, move |issued_at| {
            second_count.fetch_add(1, Ordering::SeqCst);
            Ok(format!("token-{issued_at}"))
        })
        .await
        .expect("reuse provider token");

    assert_eq!(first, second);
    assert_eq!(sign_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn provider_token_cache_refreshes_after_fifty_minutes() {
    let cache = ProviderTokenCache::default();
    let sign_count = Arc::new(AtomicUsize::new(0));

    let first_count = Arc::clone(&sign_count);
    let first = cache
        .get_or_create(1_000, move |issued_at| {
            first_count.fetch_add(1, Ordering::SeqCst);
            Ok(format!("token-{issued_at}"))
        })
        .await
        .expect("create provider token");
    let refreshed_count = Arc::clone(&sign_count);
    let refreshed = cache
        .get_or_create(1_000 + PROVIDER_TOKEN_REFRESH_SECONDS, move |issued_at| {
            refreshed_count.fetch_add(1, Ordering::SeqCst);
            Ok(format!("token-{issued_at}"))
        })
        .await
        .expect("refresh provider token");

    assert_ne!(first, refreshed);
    assert_eq!(sign_count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn concurrent_provider_token_requests_share_one_signed_token() {
    let cache = Arc::new(ProviderTokenCache::default());
    let sign_count = Arc::new(AtomicUsize::new(0));
    let request_token = |cache: Arc<ProviderTokenCache>, sign_count: Arc<AtomicUsize>| async move {
        cache
            .get_or_create(1_000, move |issued_at| {
                sign_count.fetch_add(1, Ordering::SeqCst);
                Ok(format!("token-{issued_at}"))
            })
            .await
            .expect("get provider token")
    };

    let (first, second, third) = tokio::join!(
        request_token(Arc::clone(&cache), Arc::clone(&sign_count)),
        request_token(Arc::clone(&cache), Arc::clone(&sign_count)),
        request_token(Arc::clone(&cache), Arc::clone(&sign_count)),
    );

    assert_eq!(first, second);
    assert_eq!(second, third);
    assert_eq!(sign_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn provider_token_cache_invalidates_only_the_rejected_token() {
    let cache = ProviderTokenCache::default();
    let first = cache
        .get_or_create(1_000, |issued_at| Ok(format!("token-{issued_at}")))
        .await
        .expect("create provider token");

    cache.invalidate_if_matches("a-different-token").await;
    let retained = cache
        .get_or_create(1_001, |issued_at| Ok(format!("token-{issued_at}")))
        .await
        .expect("retain provider token");
    assert_eq!(retained, first);

    cache.invalidate_if_matches(&first).await;
    let replaced = cache
        .get_or_create(1_002, |issued_at| Ok(format!("token-{issued_at}")))
        .await
        .expect("replace rejected provider token");
    assert_ne!(replaced, first);
}

#[test]
fn translated_message_payload_contains_chat_navigation_and_badge() {
    let content = json!({"type": "text", "body": "Szia"});
    let message = StoredMessage {
        id: "message-1".to_string(),
        contact_id: "family@g.us".to_string(),
        timestamp: 1_700_000_000_000,
        is_from_me: false,
        is_forwarded: false,
        sender_name: Some("Virág".to_string()),
        sender_phone: None,
        contact_name: Some("The Skinners".to_string()),
        contact_phone: None,
        chat_type: "group".to_string(),
        content_type: "Text".to_string(),
        content_json: content.to_string(),
        content: Some(content),
        original_text: Some("Szia".to_string()),
        translated_text: Some("Hello".to_string()),
        source_language: Some("Hungarian".to_string()),
        is_translated: true,
        delivery_status: None,
    };

    let notification = PushNotification::from_message_with_avatar(
        &message,
        7,
        Some("https://cdn.example.com/avatar.jpg"),
    )
    .with_recipient_count(Some(3));

    assert_eq!(notification.payload["aps"]["alert"]["title"], "Virág");
    assert_eq!(
        notification.payload["aps"]["alert"]["subtitle"],
        "The Skinners"
    );
    assert_eq!(notification.payload["aps"]["alert"]["body"], "Hello");
    assert_eq!(notification.payload["aps"]["badge"], 7);
    assert_eq!(notification.payload["aps"]["content-available"], 1);
    assert_eq!(notification.payload["aps"]["mutable-content"], 1);
    assert_eq!(notification.payload["aps"]["category"], "MESSAGE_CATEGORY");
    assert_eq!(notification.payload["aps"]["thread-id"], "family@g.us");
    assert_eq!(notification.payload["contactId"], "family@g.us");
    assert_eq!(notification.payload["messageId"], "message-1");
    assert_eq!(notification.payload["senderName"], "Virág");
    assert_eq!(notification.payload["conversationName"], "The Skinners");
    assert_eq!(notification.payload["chatType"], "group");
    assert_eq!(notification.payload["messageBody"], "Hello");
    assert_eq!(notification.payload["recipientCount"], 3);
    assert_eq!(
        notification.payload["avatarUrl"],
        "https://cdn.example.com/avatar.jpg"
    );
    let mut direct = message;
    direct.chat_type = "private".into();
    direct.contact_id = "friend@s.whatsapp.net".into();
    let direct =
        PushNotification::from_message_with_avatar(&direct, 0, None).with_recipient_count(Some(3));
    assert!(direct.payload.get("recipientCount").is_none());
    assert!(direct.payload["aps"]["alert"].get("subtitle").is_none());
}

#[test]
fn reaction_payload_describes_the_reaction() {
    let content = json!({
        "type": "reaction",
        "emoji": "😂",
        "target_message_id": "message-1"
    });
    let message = StoredMessage {
        id: "reaction-1".to_string(),
        contact_id: "family@g.us".to_string(),
        timestamp: 1_700_000_000_000,
        is_from_me: false,
        is_forwarded: false,
        sender_name: Some("Eileen".to_string()),
        sender_phone: Some("447700900123".to_string()),
        contact_name: Some("The Skinners".to_string()),
        contact_phone: None,
        chat_type: "group".to_string(),
        content_type: "Reaction".to_string(),
        content_json: content.to_string(),
        content: Some(content),
        original_text: None,
        translated_text: None,
        source_language: None,
        is_translated: false,
        delivery_status: None,
    };

    let target_content = json!({"type": "text", "body": "Mother... slow down!"});
    let target = StoredMessage {
        id: "message-1".to_string(),
        content_type: "Text".to_string(),
        content_json: target_content.to_string(),
        content: Some(target_content),
        ..message.clone()
    };
    let notification =
        PushNotification::from_message_with_context(&message, 1, None, Some(&target));

    assert_eq!(
        notification.payload["aps"]["alert"]["body"],
        "Reacted 😂 to “Mother... slow down!”"
    );
}

#[test]
fn test_payload_is_visible_and_does_not_open_a_chat() {
    let notification = PushNotification::test();

    assert_eq!(
        notification.payload["aps"]["alert"]["title"],
        "WhatsApp Translator"
    );
    assert_eq!(notification.payload["aps"]["sound"], "default");
    assert_eq!(notification.payload["notificationTest"], true);
    assert!(notification.payload.get("contactId").is_none());
}
