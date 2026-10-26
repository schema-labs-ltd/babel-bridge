use super::*;

#[tokio::test]
async fn disabled_conversations_prepare_original_mcp_text_and_reject_stale_translated_preparations()
{
    let (state, data_dir) = test_state();
    let server = WhatsAppMcpServer::new(
        state.clone(),
        McpPermissions {
            read: true,
            send: true,
        },
    );
    for contact_id in ["person@s.whatsapp.net", "family@g.us"] {
        state
            .store
            .upsert_contact(contact_id, Some("Synthetic chat"), None, None, 0)
            .unwrap();
        let result = server.handle_prepare_message(json!({
            "contact_id": contact_id, "text": "Hello", "translation_mode": "auto", "target_language": "Hungarian"
        })).await.unwrap().structured_content.unwrap();
        assert_eq!(result["finalText"], "Hello");
        assert_eq!(result["translated"], false);
        let preparations = state.mcp_prepared_messages.read().await;
        let mut stale: PreparedMcpMessage = serde_json::from_value(
            preparations
                .get(result["preparationToken"].as_str().unwrap())
                .unwrap()
                .clone(),
        )
        .unwrap();
        drop(preparations);
        stale.translated = true;
        stale.final_text = "Szia".into();
        state
            .mcp_prepared_messages
            .write()
            .await
            .insert(stale.token.clone(), serde_json::to_value(&stale).unwrap());
        let error = server.handle_prepared_send(json!({"preparation_token": stale.token, "idempotency_key": format!("stale-{contact_id}")}), false).await.unwrap_err();
        assert!(error.to_string().contains("Translation is now off"));
        assert!(state
            .mcp_prepared_messages
            .read()
            .await
            .contains_key(&stale.token));
    }
    std::fs::remove_dir_all(data_dir).unwrap();
}

#[tokio::test]
async fn reply_only_rejects_new_and_previously_prepared_unquoted_sends() {
    let (state, data_dir) = test_state();
    let contact = "family@g.us";
    state
        .store
        .upsert_contact(contact, None, None, Some("group"), 1)
        .unwrap();
    let server = WhatsAppMcpServer::new(
        state.clone(),
        McpPermissions {
            read: true,
            send: true,
        },
    );
    let prepared = server
        .handle_prepare_message(json!({"contact_id":contact,"text":"Hello"}))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    state
        .store
        .update_conversation_settings(
            contact,
            &ConversationSettings {
                reply_only: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(server
        .handle_prepare_message(json!({"contact_id":contact,"text":"Hello"}))
        .await
        .unwrap_err()
        .to_string()
        .contains("Reply only"));
    assert!(server.handle_prepared_send(json!({"preparation_token":prepared["preparationToken"],"idempotency_key":"reply-only-test"}),false).await.unwrap_err().to_string().contains("Reply only"));
    std::fs::remove_dir_all(data_dir).unwrap();
}

#[tokio::test]
async fn required_conversation_translation_without_a_translator_prepares_nothing() {
    let (state, data_dir) = test_state();
    let contact_id = "33612345678@s.whatsapp.net";
    state
        .store
        .upsert_contact(
            contact_id,
            Some("French Contact"),
            Some("33612345678"),
            Some("private"),
            1_700_000_000_000,
        )
        .expect("insert contact");
    state
        .store
        .update_conversation_settings(
            contact_id,
            &ConversationSettings {
                reply_only: false,
                translation_enabled: true,
                language_override: Some("French".to_string()),
                translation_style: None,
                send_original_follow_up: false,
            },
        )
        .expect("settings");
    let server = WhatsAppMcpServer::new(
        state.clone(),
        McpPermissions {
            read: true,
            send: true,
        },
    );

    let error = server
        .handle_prepare_message(json!({
            "contact_id":contact_id,
            "text":"Hello",
            "translation_mode":"auto"
        }))
        .await
        .expect_err("translation must fail closed");
    assert!(error
        .to_string()
        .contains("No message was sent or prepared"));
    assert!(state.mcp_prepared_messages.read().await.is_empty());
    std::fs::remove_dir_all(data_dir).expect("remove test data");
}

#[tokio::test]
async fn choosing_reply_for_a_non_reply_preparation_does_not_consume_it() {
    let (state, data_dir) = test_state();
    let token = "prepared-token";
    let prepared = PreparedMcpMessage {
        token: token.to_string(),
        contact_id: "chat@example.test".to_string(),
        contact_name: Some("Test".to_string()),
        original_text: "Hello".to_string(),
        final_text: "Hello".to_string(),
        translated: false,
        target_language: None,
        translation_mode: "never".to_string(),
        reply_to_message_id: None,
        reply_to_sender: None,
        reply_to_text: None,
        created_at: chrono::Utc::now().timestamp_millis(),
        expires_at: chrono::Utc::now().timestamp_millis() + PREPARATION_TTL_MILLIS,
    };
    state.mcp_prepared_messages.write().await.insert(
        token.to_string(),
        serde_json::to_value(prepared).expect("serialize preparation"),
    );
    let server = WhatsAppMcpServer::new(
        state.clone(),
        McpPermissions {
            read: true,
            send: true,
        },
    );

    server
        .handle_prepared_send(
            json!({"preparation_token":token,"idempotency_key":"reply-test-key"}),
            true,
        )
        .await
        .expect_err("not a reply preparation");
    assert!(state.mcp_prepared_messages.read().await.contains_key(token));
    std::fs::remove_dir_all(data_dir).expect("remove test data");
}

#[tokio::test]
async fn idempotency_claim_is_atomic() {
    let (state, data_dir) = test_state();
    let server = WhatsAppMcpServer::new(
        state,
        McpPermissions {
            read: true,
            send: true,
        },
    );
    assert!(
        server
            .begin_idempotent_operation("unique-key", "send:one")
            .await
    );
    assert!(
        !server
            .begin_idempotent_operation("unique-key", "send:one")
            .await
    );
    std::fs::remove_dir_all(data_dir).expect("remove test data");
}
