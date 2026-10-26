//! Gallery, identity, topics and unified feed read tests.

use super::*;

#[tokio::test]
async fn gallery_paginates_only_visual_media_in_one_chat_and_strips_originals() {
    let (state, dir) = test_state(None);
    for chat in ["gallery@g.us", "other@g.us"] {
        state
            .store
            .upsert_contact(chat, None, None, Some("group"), 1)
            .unwrap();
    }
    for (id, kind, timestamp, chat) in [
        ("a", "image", 50, "gallery@g.us"),
        ("b", "image", 100, "gallery@g.us"),
        ("c", "video", 100, "gallery@g.us"),
        ("text", "text", 200, "gallery@g.us"),
        ("audio", "audio", 300, "gallery@g.us"),
        ("sticker", "sticker", 400, "gallery@g.us"),
        ("other", "image", 500, "other@g.us"),
        ("deleted", "revoked", 600, "gallery@g.us"),
    ] {
        let mut message = feed_test_message(id, chat, timestamp);
        message.content_type = if kind == "revoked" {
            "image".into()
        } else {
            kind.into()
        };
        message.is_from_me = id == "c";
        message.content_json =
            serde_json::json!({"type":kind, "media_data":"cGl4ZWxz", "caption":"Caption"})
                .to_string();
        state.store.add_message(&message).unwrap();
    }
    let router = create_router(state.clone());
    let response = router
        .clone()
        .oneshot(empty_request("/api/gallery/gallery%40g.us?limit=2"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let first: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 1_000_000)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(first["hasMore"], true);
    assert_eq!(first["messages"][0]["id"], "b");
    assert_eq!(first["messages"][1]["id"], "c");
    assert_eq!(first["messages"][1]["isFromMe"], true);
    assert!(first["messages"][0]["content"].get("media_data").is_none());
    assert_eq!(first["messages"][0]["content"]["has_media"], true);
    let response = router
        .clone()
        .oneshot(empty_request(
            "/api/gallery/gallery%40g.us?limit=2&before=100&before_id=b",
        ))
        .await
        .unwrap();
    let older: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 1_000_000)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(older["hasMore"], false);
    assert_eq!(older["messages"].as_array().unwrap().len(), 1);
    assert_eq!(older["messages"][0]["id"], "a");
    assert_eq!(
        router
            .oneshot(empty_request("/api/media/deleted/thumbnail"))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    drop(state);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn gallery_and_previews_require_the_existing_server_authentication() {
    let (state, dir) = test_state(Some("test-password"));
    let router = create_router(state);
    for path in ["/api/gallery/gallery%40g.us", "/api/media/photo/thumbnail"] {
        assert_eq!(
            router
                .clone()
                .oneshot(empty_request(path))
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn device_identity_cached_link_and_reply_route_to_account_and_replay_once() {
    let (state, dir) = test_state(None);
    let account = "447700900123@s.whatsapp.net";
    let device = "447700900123:22@s.whatsapp.net";
    state
        .store
        .upsert_contact(account, Some("Friend"), None, Some("private"), 1)
        .unwrap();
    state
        .store
        .add_message(&feed_test_message("a", device, 100))
        .unwrap();
    state
        .store
        .add_message(&feed_test_message("b", account, 200))
        .unwrap();
    let router = create_router(state.clone());
    let response = router
        .clone()
        .oneshot(empty_request(&format!(
            "/api/messages/{}",
            urlencoding::encode(device)
        )))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let payload: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 1_000_000)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(payload["messages"].as_array().unwrap().len(), 2);
    assert_eq!(payload["messages"][0]["contactId"], account);
    state.set_connected(true, None, None).await;
    let (sender, mut receiver) = mpsc::channel(8);
    state.set_command_tx(sender).await;
    let request = || {
        HttpRequest::builder().method("POST").uri("/api/send")
        .header("content-type", "application/json").header("idempotency-key", "device-reply-once")
        .body(Body::from(serde_json::json!({"contactId": device, "text":"Reply", "replyTo":"a", "replyToText":"a", "replyOnlyIfNotLatest":true}).to_string())).unwrap()
    };
    let sending = tokio::spawn(router.clone().oneshot(request()));
    let BridgeCommand::Send {
        request_id: Some(request_id),
        to,
        reply_to,
        ..
    } = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
        .await
        .unwrap()
        .unwrap()
    else {
        panic!("expected send")
    };
    assert_eq!(to, account);
    assert_eq!(reply_to.as_deref(), Some("a"));
    state
        .handle_send_result(BridgeSendResult {
            request_id,
            success: true,
            message_id: Some("sent-device-reply".into()),
            timestamp: Some(1_700_000_000),
            message_ids: vec![],
            timestamps: vec![],
            error: None,
        })
        .await;
    assert_eq!(sending.await.unwrap().unwrap().status(), StatusCode::OK);
    assert_eq!(
        router.oneshot(request()).await.unwrap().status(),
        StatusCode::OK
    );
    assert!(receiver.try_recv().is_err());
    assert_eq!(state.store.get_contacts().unwrap().len(), 1);
    assert_eq!(
        state
            .store
            .get_message_by_id("sent-device-reply")
            .unwrap()
            .unwrap()
            .contact_id,
        account
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn message_topic_inspection_is_authenticated_bounded_and_read_only() {
    let (state, dir) = test_state(Some("private"));
    let request = |count: usize| {
        HttpRequest::builder()
            .method("POST")
            .uri("/api/topics/messages")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"messageIds":vec!["missing"; count]}).to_string(),
            ))
            .unwrap()
    };
    assert_eq!(
        create_router(state)
            .oneshot(request(1))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let (state, other_dir) = test_state(None);
    let app = create_router(state);
    assert_eq!(
        app.clone().oneshot(request(201)).await.unwrap().status(),
        StatusCode::BAD_REQUEST
    );
    let response = app.oneshot(request(1)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
        serde_json::json!({"topics":[]})
    );
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(other_dir);
}

#[tokio::test]
async fn topics_require_auth_and_paginate_without_read_receipts_or_cross_chat_results() {
    let (locked, locked_dir) = test_state(Some("private"));
    let locked_router = create_router(locked);
    for path in [
        "/api/topics",
        "/api/topics/import",
        "/api/topics/example/messages",
        "/api/topics/category:cGxhbnM/messages",
    ] {
        assert_eq!(
            locked_router
                .clone()
                .oneshot(empty_request(path))
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        locked_router
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api/topics/import")
                    .body(Body::empty())
                    .unwrap()
            )
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let protected = HttpRequest::builder()
        .method("PUT")
        .uri("/api/contacts/family%40g.us/topics")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"enabled":true}"#))
        .unwrap();
    assert_eq!(
        locked_router.oneshot(protected).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    let (state, dir) = test_state(None);
    for (id, chat, time) in [
        ("a", "family@g.us", 100),
        ("b", "family@g.us", 100),
        ("c", "friends@g.us", 200),
    ] {
        state
            .store
            .upsert_contact(chat, Some(chat), None, Some("group"), time)
            .unwrap();
        let mut message = feed_test_message(id, chat, time);
        message.original_text = Some(id.into());
        state.store.add_message(&message).unwrap();
    }
    let app = create_router(state.clone());
    let enable = HttpRequest::builder()
        .method("PUT")
        .uri("/api/contacts/family%40g.us/topics")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"enabled":true}"#))
        .unwrap();
    assert_eq!(
        app.clone().oneshot(enable).await.unwrap().status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert!(state.store.topic_settings().unwrap().is_empty());
    assert_eq!(
        app.clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api/topics/import")
                    .body(Body::empty())
                    .unwrap()
            )
            .await
            .unwrap()
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    state.store.set_topics_enabled("family@g.us", true).unwrap();
    let batch = state.store.next_topic_batch().unwrap().unwrap();
    let assignments = batch
        .messages
        .iter()
        .map(|m| crate::topics::TopicAssignment {
            message_id: m.id.clone(),
            topic: "Plans".into(),
        })
        .collect::<Vec<_>>();
    state
        .store
        .finish_topic_batch(&batch, &assignments)
        .unwrap();
    let topic = state.store.list_topics().unwrap().remove(0);
    state.store.set_unread_count("family@g.us", 3).unwrap();
    let response = app
        .clone()
        .oneshot(empty_request(&format!(
            "/api/topics/{}/messages?limit=1",
            topic.id
        )))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 1_000_000)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["hasMore"], true);
    assert_eq!(body["messages"][0]["id"], "b");
    assert_eq!(body["messages"][0]["contactId"], "family@g.us");
    assert_eq!(
        state
            .store
            .get_contact("family@g.us")
            .unwrap()
            .unwrap()
            .unread_count,
        3
    );
    let response = app
        .clone()
        .oneshot(empty_request(&format!(
            "/api/topics/{}/messages?limit=1&before=100&before_id=b",
            topic.id
        )))
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 1_000_000)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["messages"][0]["id"], "a");
    assert_eq!(body["hasMore"], false);
    state
        .store
        .set_topics_enabled("friends@g.us", true)
        .unwrap();
    let batch = state.store.next_topic_batch().unwrap().unwrap();
    state
        .store
        .finish_topic_batch(
            &batch,
            &[crate::topics::TopicAssignment {
                message_id: "c".into(),
                topic: "Plans".into(),
            }],
        )
        .unwrap();
    let response = app
        .clone()
        .oneshot(empty_request(&format!(
            "/api/topics/{}/messages?limit=3",
            urlencoding::encode(&topic.category_id)
        )))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 1_000_000)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["messages"].as_array().unwrap().len(), 3);
    assert_eq!(body["messages"][0]["contactId"], "family@g.us");
    assert_eq!(body["messages"][2]["contactId"], "friends@g.us");
    state
        .store
        .set_topics_enabled("family@g.us", false)
        .unwrap();
    assert_eq!(
        app.oneshot(empty_request(&format!("/api/topics/{}/messages", topic.id)))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(locked_dir);
}

#[tokio::test]
async fn unified_feed_requires_auth_and_pages_without_marking_chats_read() {
    let (locked, locked_dir) = test_state(Some("private"));
    assert_eq!(
        create_router(locked)
            .oneshot(empty_request("/api/feed"))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let (state, dir) = test_state(None);
    for contact in ["one@g.us", "two@g.us"] {
        state
            .store
            .upsert_contact(contact, None, None, Some("group"), 1)
            .unwrap();
    }
    for (id, contact, time) in [
        ("a", "one@g.us", 100),
        ("b", "two@g.us", 100),
        ("c", "one@g.us", 200),
    ] {
        state
            .store
            .add_message(&feed_test_message(id, contact, time))
            .unwrap();
    }
    state.store.set_unread_count("one@g.us", 3).unwrap();
    let before = state
        .store
        .get_contact("one@g.us")
        .unwrap()
        .unwrap()
        .unread_count;
    let app = create_router(state.clone());
    let response = app
        .clone()
        .oneshot(empty_request("/api/feed?limit=2"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let payload: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 1_000_000)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(payload["hasMore"], true);
    assert_eq!(payload["messages"][0]["id"], "b");
    assert_eq!(payload["messages"][1]["id"], "c");
    let response = app
        .oneshot(empty_request("/api/feed?limit=2&before=100&before_id=b"))
        .await
        .unwrap();
    let payload: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 1_000_000)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(payload["messages"][0]["id"], "a");
    assert_eq!(payload["hasMore"], false);
    assert_eq!(
        state
            .store
            .get_contact("one@g.us")
            .unwrap()
            .unwrap()
            .unread_count,
        before
    );
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(locked_dir);
}
