use super::*;

#[test]
fn unified_feed_schema_is_complete_app_only_and_read_scoped() {
    let fixture = FeedFixture::new();
    for scope in [
        "",
        "whatsapp.send",
        "whatsapp.read",
        "whatsapp.read whatsapp.send",
        "mcp",
    ] {
        let server =
            WhatsAppMcpServer::new(fixture.state.clone(), McpPermissions::from_scope(scope));
        let tools = server.advertised_tools();
        let feed = tools
            .iter()
            .find(|tool| tool.name == "read_unified_messages");
        assert_eq!(feed.is_some(), server.permissions.read, "{scope}");
    }
    let tool = serde_json::to_value(WhatsAppMcpServer::read_unified_messages_tool()).unwrap();
    assert_eq!(tool["_meta"]["ui"]["visibility"], json!(["app"]));
    assert_eq!(tool["annotations"]["readOnlyHint"], true);
    assert_eq!(tool["annotations"]["destructiveHint"], false);
    assert_eq!(tool["annotations"]["idempotentHint"], true);
    assert_eq!(tool["annotations"]["openWorldHint"], false);
    let input = &tool["inputSchema"];
    assert_eq!(input["additionalProperties"], false);
    assert_eq!(
        input["properties"]["limit"],
        json!({"type":"integer","minimum":1,"maximum":100,"default":50})
    );
    assert_eq!(
        input["properties"]["before_timestamp"]["maximum"],
        JS_SAFE_INTEGER
    );
    assert_eq!(
        input["dependentRequired"],
        json!({"before_timestamp":["before_message_id"], "before_message_id":["before_timestamp"]})
    );
    let output = &tool["outputSchema"];
    assert_eq!(output["additionalProperties"], false);
    assert_eq!(
        output["required"],
        json!(["hasMore", "messages", "nextCursor"])
    );
    let message = &output["properties"]["messages"]["items"];
    let mut required = vec![
        "id",
        "contactId",
        "contactName",
        "contactPhone",
        "chatType",
        "timestamp",
        "timestampIso",
        "isFromMe",
        "senderName",
        "senderPhone",
        "text",
        "originalText",
        "translatedText",
        "isTranslated",
        "sourceLanguage",
        "contentType",
        "deliveryStatus",
        "replyContext",
    ];
    required.sort_unstable();
    assert_eq!(message["required"], json!(required));
    assert_eq!(
        message["properties"].as_object().unwrap().len(),
        required.len()
    );
    assert_eq!(message["additionalProperties"], false);
    for name in [
        "contactName",
        "contactPhone",
        "timestampIso",
        "senderName",
        "senderPhone",
        "text",
        "originalText",
        "translatedText",
        "sourceLanguage",
        "deliveryStatus",
    ] {
        assert_eq!(
            message["properties"][name]["type"],
            json!(["string", "null"])
        );
    }
    let quote = &message["properties"]["replyContext"]["anyOf"][1];
    assert_eq!(
        quote["required"],
        json!(["messageId", "senderName", "text"])
    );
    assert_eq!(quote["additionalProperties"], false);
    assert_eq!(
        output["properties"]["nextCursor"]["anyOf"][1]["required"],
        json!(["beforeMessageId", "beforeTimestamp"])
    );
}

#[test]
fn unified_feed_runtime_arguments_enforce_schema_and_preserve_opaque_ids() {
    for args in invalid_feed_arguments() {
        assert!(FeedArguments::parse(&args).is_err(), "accepted {args}");
    }
    assert_eq!(FeedArguments::parse(&json!({})).unwrap().limit, 50);
    let id = "  opaque:/%🧪  ";
    let args = FeedArguments::parse(
        &json!({"limit":100,"before_timestamp":JS_SAFE_INTEGER,"before_message_id":id}),
    )
    .unwrap();
    assert_eq!(args.limit, 100);
    assert_eq!(args.before_timestamp, Some(JS_SAFE_INTEGER));
    assert_eq!(args.before_message_id.as_deref(), Some(id));
    assert_eq!(
        FeedArguments::parse(&json!({"limit":2.0,"before_timestamp":0.0,"before_message_id":"a"}))
            .unwrap()
            .before_timestamp,
        Some(0)
    );
}

#[tokio::test]
async fn unified_feed_exact_content_and_cursor_are_independent_of_contacts_page() {
    let fixture = FeedFixture::new();
    seed_feed(&fixture.state);
    let server = WhatsAppMcpServer::new(
        fixture.state.clone(),
        McpPermissions {
            read: true,
            send: true,
        },
    );
    let contacts = server
        .handle_contacts(json!({}), false)
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_eq!(contacts["contacts"].as_array().unwrap().len(), 50);
    assert!(!contacts["contacts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|contact| contact["id"] == "synthetic-group@g.us"));
    let page = server
        .handle_read_unified_messages(json!({"limit":2}))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_eq!(
        page,
        json!({"messages":[
        {"id":"b","contactId":"synthetic-group@g.us","contactName":"Fixture Group","contactPhone":null,"chatType":"group",
            "timestamp":100,"timestampIso":"1970-01-01T00:00:00.100+00:00","isFromMe":false,"senderName":"Fixture Member","senderPhone":"900000000001@lid",
            "text":"Look","originalText":"Regarde","translatedText":"Look","isTranslated":true,"sourceLanguage":"French","contentType":"Image","deliveryStatus":"delivered",
            "replyContext":{"messageId":"outside-history","senderName":"Quoted Member","text":"Earlier quote"}},
        {"id":"c","contactId":"447700900123@s.whatsapp.net","contactName":"Fixture Friend","contactPhone":"447700900123","chatType":"private",
            "timestamp":200,"timestampIso":"1970-01-01T00:00:00.200+00:00","isFromMe":true,"senderName":"Me","senderPhone":"447700900000",
            "text":"See you","originalText":"See you","translatedText":"À bientôt","isTranslated":true,"sourceLanguage":"French","contentType":"text","deliveryStatus":"uncertain","replyContext":null}
    ],"hasMore":true,"nextCursor":{"beforeTimestamp":100,"beforeMessageId":"b"}})
    );
    let older = server.handle_read_unified_messages(json!({"limit":2,"before_timestamp":page["nextCursor"]["beforeTimestamp"],"before_message_id":page["nextCursor"]["beforeMessageId"]})).await.unwrap().structured_content.unwrap();
    assert_eq!(older["messages"].as_array().unwrap().len(), 1);
    assert_eq!(older["messages"][0]["id"], "a");
    assert_eq!(
        older["messages"][0]["contactId"],
        "447700900123@s.whatsapp.net"
    );
    assert_eq!(older["messages"][0]["text"], "hello");
    assert_eq!(older["hasMore"], false);
    assert!(older["nextCursor"].is_null());
    let all = server
        .handle_read_unified_messages(json!({}))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_eq!(
        all["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|message| message["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["a", "b", "c"]
    );
    assert!(!all.to_string().contains("SYNTHETIC_MEDIA_BYTES"));
    // Navigating by the source ID works without having loaded its contact row.
    let chat = server
        .handle_open(json!({"contact_id":page["messages"][0]["contactId"]}), true)
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_eq!(chat["contact"]["id"], "synthetic-group@g.us");
    assert_eq!(chat["messages"][0]["id"], "b");
    assert!(chat["messages"][0].get("chatType").is_none());
    assert_eq!(
        chat["messages"][0]["replyContext"]["unexpected"],
        "must not leak"
    );
}

#[tokio::test]
async fn unified_feed_empty_missing_metadata_media_and_partial_quotes_are_explicit() {
    let fixture = FeedFixture::new();
    let server = WhatsAppMcpServer::new(
        fixture.state.clone(),
        McpPermissions {
            read: true,
            send: false,
        },
    );
    assert_eq!(
        server
            .handle_read_unified_messages(json!({}))
            .await
            .unwrap()
            .structured_content
            .unwrap(),
        json!({"messages":[],"hasMore":false,"nextCursor":null})
    );
    fixture
        .state
        .store
        .upsert_contact("unresolved@lid", None, None, Some("private"), 0)
        .unwrap();
    for (id, kind, caption, quote) in [
        ("audio", "Audio", None, json!("invalid quote")),
        (
            "document",
            "Document",
            Some("Document caption"),
            json!({"messageId":7,"text":"Partial quote","extra":true}),
        ),
        ("video", "Video", Some("Video caption"), json!(null)),
    ] {
        let mut message = extension_message(id, "unresolved@lid", 100, true);
        message.content_type = kind.into();
        message.content_json = json!({"type":kind.to_lowercase(),"caption":caption,"mediaData":"SYNTHETIC_MEDIA_BYTES","reply_context":quote}).to_string();
        message.original_text = None;
        message.translated_text = None;
        message.is_translated = false;
        fixture.state.store.add_message(&message).unwrap();
    }
    let page = server
        .handle_read_unified_messages(json!({}))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_eq!(page["messages"].as_array().unwrap().len(), 3);
    for message in page["messages"].as_array().unwrap() {
        assert_eq!(message["contactId"], "unresolved@lid");
        assert!(message["contactName"].is_null());
        assert!(message["contactPhone"].is_null());
    }
    assert!(page["messages"][0]["text"].is_null());
    assert!(page["messages"][0]["replyContext"].is_null());
    assert_eq!(page["messages"][1]["text"], "Document caption");
    assert_eq!(
        page["messages"][1]["replyContext"],
        json!({"messageId":null,"senderName":null,"text":"Partial quote"})
    );
    assert_eq!(page["messages"][2]["text"], "Video caption");
    assert!(!page.to_string().contains("SYNTHETIC_MEDIA_BYTES"));
}
