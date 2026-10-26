use super::*;

#[test]
fn extension_resources_and_entrypoints_obey_read_scope_and_display_contract() {
    let (state, dir) = test_state();
    for read in [false, true] {
        let server = WhatsAppMcpServer::new(state.clone(), McpPermissions { read, send: false });
        assert_eq!(server.chat_resources().resources.len(), usize::from(read));
        assert_eq!(server.chat_resource(CHAT_RESOURCE_URI).is_ok(), read);
        assert!(server.chat_resource("file:///etc/passwd").is_err());
        let tools = server.advertised_tools();
        assert_eq!(tools.iter().any(|tool| tool.name == "open_chats"), read);
        if read {
            let opener = tools.iter().find(|tool| tool.name == "open_chats").unwrap();
            let metadata = serde_json::to_value(opener.meta.as_ref().unwrap()).unwrap();
            assert_eq!(metadata["ui"]["resourceUri"], CHAT_RESOURCE_URI);
            assert_eq!(
                metadata["openai/ui"]["entrypoints"],
                json!([{"type":"global"},{"type":"thread"}])
            );
            assert!(opener.input_schema.get("required").is_none());
            assert_eq!(
                opener.annotations.as_ref().unwrap().read_only_hint,
                Some(true)
            );
            let resource =
                serde_json::to_value(server.chat_resource(CHAT_RESOURCE_URI).unwrap()).unwrap();
            assert_eq!(resource["contents"][0]["mimeType"], CHAT_MIME_TYPE);
            assert_eq!(
                resource["contents"][0]["_meta"]["openai/ui"]["preferredDisplayMode"],
                "fullscreen"
            );
            assert_eq!(
                resource["contents"][0]["_meta"]["ui"]["csp"]["connectDomains"],
                json!([])
            );
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn extension_openers_keep_instance_data_and_permissions_isolated_without_writes() {
    let (first, first_dir) = test_state();
    let (second, second_dir) = test_state();
    first
        .store
        .upsert_contact(
            "alice@s.whatsapp.net",
            Some("Alice"),
            None,
            Some("private"),
            100,
        )
        .unwrap();
    first
        .store
        .add_message(&extension_message("a1", "alice@s.whatsapp.net", 100, true))
        .unwrap();
    second
        .store
        .upsert_contact(
            "bob@s.whatsapp.net",
            Some("Bob"),
            None,
            Some("private"),
            100,
        )
        .unwrap();
    let first_server = WhatsAppMcpServer::new(
        first.clone(),
        McpPermissions {
            read: true,
            send: false,
        },
    );
    let second_server = WhatsAppMcpServer::new(
        second,
        McpPermissions {
            read: true,
            send: true,
        },
    );
    let output = first_server
        .handle_open(json!({}), false)
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_eq!(output["contacts"][0]["name"], "Alice");
    assert_eq!(output["permissions"]["send"], false);
    assert!(second_server
        .handle_open(json!({"contact_id":"alice@s.whatsapp.net"}), true)
        .await
        .is_err());
    assert!(first_server
        .handle_open(json!({"contact_id":"bob@s.whatsapp.net"}), true)
        .await
        .is_err());
    let unread_before = first
        .store
        .get_contact("alice@s.whatsapp.net")
        .unwrap()
        .unwrap()
        .unread_count;
    for _ in 0..3 {
        let output = first_server
            .handle_open(json!({"contact_id":"alice@s.whatsapp.net"}), true)
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert_eq!(output["messages"][0]["text"], "bonjour");
        assert_eq!(output["messages"][0]["originalText"], "hello");
    }
    assert_eq!(
        first
            .store
            .get_contact("alice@s.whatsapp.net")
            .unwrap()
            .unwrap()
            .unread_count,
        unread_before
    );
    assert!(first.mcp_prepared_messages.read().await.is_empty());
    assert!(first_server
        .handle_translate_message(json!({"contact_id":"alice@s.whatsapp.net","message_id":"a1"}))
        .await
        .is_err());
    for dir in [first_dir, second_dir] {
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[tokio::test]
async fn extension_pagination_covers_all_chats_and_sparse_messages_with_equal_timestamps() {
    let (state, dir) = test_state();
    for id in [
        "first@s.whatsapp.net",
        "second@s.whatsapp.net",
        "third@g.us",
    ] {
        state
            .store
            .upsert_contact(id, None, None, None, 100)
            .unwrap();
    }
    let server = WhatsAppMcpServer::new(
        state.clone(),
        McpPermissions {
            read: true,
            send: false,
        },
    );
    let first = server
        .handle_contacts(json!({"limit":2}), false)
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_eq!(first["nextCursor"], 2);
    let last = server
        .handle_contacts(json!({"limit":2,"cursor":2}), false)
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_eq!(last["contacts"].as_array().unwrap().len(), 1);
    assert!(last["nextCursor"].is_null());
    for i in 0..650 {
        state
            .store
            .add_message(&extension_message(
                &format!("m{i:04}"),
                "first@s.whatsapp.net",
                100,
                i == 5 || i == 600,
            ))
            .unwrap();
    }
    let page = server
        .handle_read_messages(
            json!({"contact_id":"first@s.whatsapp.net","limit":1,"direction":"incoming"}),
        )
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_eq!(page["messages"][0]["id"], "m0600");
    assert_eq!(page["hasMore"], true);
    let cursor = &page["nextCursor"];
    let older = server.handle_read_messages(json!({"contact_id":"first@s.whatsapp.net","limit":1,"direction":"incoming","before_timestamp":cursor["beforeTimestamp"],"before_message_id":cursor["beforeMessageId"]})).await.unwrap().structured_content.unwrap();
    assert_eq!(older["messages"][0]["id"], "m0005");
    assert_eq!(older["hasMore"], false);
    let search = server
        .handle_search_messages(json!({"query":"bonjour","limit":1}))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_eq!(search["nextCursor"], 1);
    let next = server
        .handle_search_messages(json!({"query":"bonjour","limit":1,"cursor":1}))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_eq!(next["messages"][0]["id"], "m0005");
    assert!(next["nextCursor"].is_null());
    std::fs::remove_dir_all(dir).unwrap();
}
