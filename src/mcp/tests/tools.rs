use super::*;

#[tokio::test]
async fn get_status_reports_reconnecting_and_a_stopped_connection() {
    let (state, data_dir) = test_state();
    let server = WhatsAppMcpServer::new(
        state.clone(),
        McpPermissions {
            read: true,
            send: false,
        },
    );
    state.set_connected(false, None, None).await;
    let retrying = server
        .handle_get_status()
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_eq!(retrying["connected"], false);
    assert_eq!(retrying["connectionState"], "reconnecting");
    state.mark_whatsapp_disconnected().await;
    let stopped = server
        .handle_get_status()
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_eq!(stopped["connected"], false);
    assert_eq!(stopped["connectionState"], "disconnected");
    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn translation_failure_never_falls_back_to_english_when_translation_is_needed() {
    let result = resolve_prepared_translation(
        "Hello",
        Some("French"),
        TranslationMode::Auto,
        Err("translator unavailable".to_string()),
    );

    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .contains("No message was sent or prepared"));
}

#[test]
fn no_translation_mode_can_intentionally_preserve_source_text() {
    assert_eq!(
        resolve_prepared_translation(
            "Hello",
            Some("French"),
            TranslationMode::Never,
            Err("translator unavailable".to_string()),
        )
        .expect("translation disabled"),
        ("Hello".to_string(), false, None)
    );
}

#[test]
fn read_tools_are_annotated_read_only_and_send_is_not() {
    let list = WhatsAppMcpServer::list_contacts_tool();
    let read = WhatsAppMcpServer::read_messages_tool();
    let send = WhatsAppMcpServer::send_message_tool();

    assert_eq!(
        list.annotations
            .as_ref()
            .and_then(|value| value.read_only_hint),
        Some(true)
    );
    assert_eq!(
        read.annotations
            .as_ref()
            .and_then(|value| value.read_only_hint),
        Some(true)
    );
    assert_eq!(
        send.annotations
            .as_ref()
            .and_then(|value| value.read_only_hint),
        Some(false)
    );
    assert_eq!(
        send.annotations
            .as_ref()
            .and_then(|value| value.idempotent_hint),
        Some(false)
    );
}

#[test]
fn scopes_control_advertised_tools() {
    assert_eq!(
        McpPermissions::from_scope("whatsapp.read"),
        McpPermissions {
            read: true,
            send: false
        }
    );
    assert_eq!(
        McpPermissions::from_scope("whatsapp.read whatsapp.send"),
        McpPermissions {
            read: true,
            send: true
        }
    );
    assert_eq!(
        McpPermissions::from_scope("mcp"),
        McpPermissions {
            read: true,
            send: true
        }
    );
}

#[test]
fn read_only_scope_hides_external_write_tools() {
    let (state, data_dir) = test_state();
    let server = WhatsAppMcpServer::new(
        state,
        McpPermissions {
            read: true,
            send: false,
        },
    );
    let names: Vec<String> = server
        .advertised_tools()
        .into_iter()
        .map(|tool| tool.name.into_owned())
        .collect();

    assert!(names.iter().any(|name| name == "search_contacts"));
    assert!(names.iter().any(|name| name == "prepare_message"));
    assert!(!names.iter().any(|name| name == "send_message"));
    assert!(!names.iter().any(|name| name == "react_to_message"));
    std::fs::remove_dir_all(data_dir).expect("remove test data");
}

#[tokio::test]
async fn search_contacts_returns_rich_structured_results() {
    let (state, data_dir) = test_state();
    state
        .store
        .upsert_contact(
            "447700900123@s.whatsapp.net",
            Some("Lee Example"),
            Some("447700900123"),
            Some("private"),
            1_700_000_000_000,
        )
        .expect("insert contact");
    let server = WhatsAppMcpServer::new(
        state,
        McpPermissions {
            read: true,
            send: false,
        },
    );

    let result = server
        .handle_contacts(json!({"query":"lee"}), true)
        .await
        .expect("search contacts");
    let payload = result.structured_content.expect("structured output");
    assert_eq!(payload["totalMatched"], 1);
    assert_eq!(payload["contacts"][0]["name"], "Lee Example");
    assert_eq!(
        payload["contacts"][0]["lastMessageTime"],
        1_700_000_000_000_i64
    );
    std::fs::remove_dir_all(data_dir).expect("remove test data");
}
