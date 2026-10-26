//! OpenAI, ringtone and conversation translation settings tests.

use super::*;

#[tokio::test]
async fn global_openai_settings_are_validated_and_persisted() {
    let (state, data_dir) = test_state(None);
    let app = create_router(state.clone());
    let request = HttpRequest::builder()
        .method("PUT")
        .uri("/api/settings/openai")
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"model":"gpt-5.6-luna","reasoningEffort":"medium"}"#,
        ))
        .expect("request");
    assert_eq!(
        app.oneshot(request).await.expect("response").status(),
        StatusCode::OK
    );
    assert_eq!(
        state.store.get_openai_settings().expect("settings"),
        OpenAiSettings {
            model: Some("gpt-5.6-luna".to_string()),
            reasoning_effort: Some("medium".to_string()),
        }
    );
    let _ = std::fs::remove_dir_all(data_dir);
}

#[tokio::test]
async fn ringtone_api_validates_and_isolates_global_and_conversation_changes() {
    let (state, dir) = test_state(None);
    let contact_id = "family+one&two%three@g.us";
    state
        .store
        .upsert_contact(contact_id, Some("Family"), None, Some("group"), 1)
        .unwrap();
    let app = create_router(state.clone());
    let global = "/api/settings/message-tone";
    let conversation = format!("{global}?contactId={}", urlencoding::encode(contact_id));
    let put = |path: &str, body: &str| {
        HttpRequest::builder()
            .method("PUT")
            .uri(path)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    };
    for (path, body) in [
        (global, r#"{"tone":"aurora"}"#),
        (conversation.as_str(), r#"{"tone":"silent"}"#),
        (global, r#"{"tone":"glass"}"#),
    ] {
        assert_eq!(
            app.clone().oneshot(put(path, body)).await.unwrap().status(),
            StatusCode::OK
        );
    }
    let response = app
        .clone()
        .oneshot(empty_request(&conversation))
        .await
        .unwrap();
    let settings: crate::message_tones::MessageToneSettings = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 8192)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        settings.effective_tone,
        crate::message_tones::MessageTone::Silent
    );
    assert_eq!(
        settings.global_tone,
        crate::message_tones::MessageTone::Glass
    );
    assert_eq!(
        app.clone()
            .oneshot(put(&conversation, r#"{"tone":null}"#))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        state
            .store
            .message_tone_settings(Some(contact_id))
            .unwrap()
            .effective_tone,
        crate::message_tones::MessageTone::Glass
    );
    for body in [
        r#"{"tone":"../private.wav"}"#,
        r#"{"tone":"unknown"}"#,
        r#"{}"#,
        r#"{"tone":null}"#,
        r#"{"tone":"silent","extra":true}"#,
    ] {
        assert!(app
            .clone()
            .oneshot(put(global, body))
            .await
            .unwrap()
            .status()
            .is_client_error());
    }
    assert_eq!(
        app.clone()
            .oneshot(put(
                &format!("{global}?contactId=missing"),
                r#"{"tone":"silent"}"#
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        state
            .store
            .message_tone_settings(None)
            .unwrap()
            .effective_tone,
        crate::message_tones::MessageTone::Glass
    );
    let (locked, locked_dir) = test_state(Some("secret"));
    assert_eq!(
        create_router(locked)
            .oneshot(put(global, r#"{"tone":"silent"}"#))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(locked_dir);
}

#[tokio::test]
async fn conversation_translation_settings_default_off_and_preserve_choices_for_older_clients() {
    let (state, dir) = test_state(None);
    let id = "family+one%2Ftwo@g.us";
    state
        .store
        .upsert_contact(id, None, None, Some("group"), 0)
        .unwrap();
    let app = create_router(state.clone());
    let path = format!("/api/contacts/{}/settings", urlencoding::encode(id));
    let response = app.clone().oneshot(empty_request(&path)).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["translationEnabled"], false);
    let mut events = state.broadcast_tx.subscribe();
    for (body, expected) in [
        (
            serde_json::json!({"translationEnabled":true,"languageOverride":"Hungarian"}),
            true,
        ),
        (
            serde_json::json!({"languageOverride":"Hungarian","translationStyle":"friendly"}),
            true,
        ),
        (
            serde_json::json!({"translationEnabled":false,"languageOverride":"Hungarian"}),
            false,
        ),
    ] {
        let request = HttpRequest::builder()
            .method("PUT")
            .uri(&path)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            state
                .store
                .get_conversation_settings(id)
                .unwrap()
                .translation_enabled,
            expected
        );
        let WebSocketEvent::ConversationSettingsUpdated { chat_id, settings } =
            events.recv().await.unwrap()
        else {
            panic!("Expected settings event")
        };
        assert_eq!(chat_id, id);
        assert_eq!(settings.translation_enabled, expected);
    }
    let unknown = HttpRequest::builder()
        .method("PUT")
        .uri("/api/contacts/missing/settings")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"translationEnabled":true}"#))
        .unwrap();
    assert_eq!(
        app.oneshot(unknown).await.unwrap().status(),
        StatusCode::NOT_FOUND
    );
    let (locked, locked_dir) = test_state(Some("test-secret"));
    assert_eq!(
        create_router(locked)
            .oneshot(empty_request(&path))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(locked_dir);
}
