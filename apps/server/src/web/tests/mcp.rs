//! MCP HTTP endpoint tests.

use super::*;

#[tokio::test]
async fn extension_mcp_http_requires_valid_oauth_and_enforces_scope_for_tools_and_resources() {
    let (state, dir) = test_state(Some("secret"));
    for auth in [
        None,
        Some("Bearer invalid-fixture"),
        Some("Basic invalid-fixture"),
    ] {
        for (method, params) in [
            (
                "resources/read",
                serde_json::json!({"uri":"ui://whatsapp-translator/chats"}),
            ),
            (
                "tools/call",
                serde_json::json!({"name":"open_chats","arguments":{}}),
            ),
        ] {
            let response = extension_rpc(state.clone(), auth, method, params).await;
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            assert!(response.headers().contains_key("www-authenticate"));
        }
    }
    state
        .store
        .oauth_store_client(&OAuthClientRegistration {
            client_id: "synthetic-client".into(),
            client_name: None,
            redirect_uris: vec!["http://127.0.0.1/callback".into()],
            scope: "whatsapp.read whatsapp.send".into(),
            created_at: 0,
        })
        .unwrap();
    for (token, scope, expires) in [
        ("expired-fixture", "whatsapp.read", 1),
        ("read-fixture", "whatsapp.read", i64::MAX),
        ("send-fixture", "whatsapp.send", i64::MAX),
    ] {
        state
            .store
            .oauth_store_access_token(&AccessToken {
                token: token.into(),
                client_id: "synthetic-client".into(),
                scope: scope.into(),
                created_at: 0,
                expires_at: expires,
                refresh_token: None,
            })
            .unwrap();
    }
    assert_eq!(
        extension_rpc(
            state.clone(),
            Some("Bearer expired-fixture"),
            "tools/list",
            serde_json::json!({})
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    let (other_state, other_dir) = test_state(Some("other-secret"));
    assert_eq!(
        extension_rpc(
            other_state,
            Some("Bearer read-fixture"),
            "tools/list",
            serde_json::json!({})
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    std::fs::remove_dir_all(other_dir).unwrap();
    let read_tools = extension_rpc_body(
        extension_rpc(
            state.clone(),
            Some("Bearer read-fixture"),
            "tools/list",
            serde_json::json!({}),
        )
        .await,
    )
    .await;
    let tools = read_tools["result"]["tools"].as_array().unwrap();
    assert!(tools.iter().any(|tool| tool["name"] == "open_chats"));
    assert!(!tools.iter().any(|tool| tool["name"] == "send_message"));
    let resource = extension_rpc_body(
        extension_rpc(
            state.clone(),
            Some("Bearer read-fixture"),
            "resources/read",
            serde_json::json!({"uri":"ui://whatsapp-translator/chats"}),
        )
        .await,
    )
    .await;
    assert_eq!(
        resource["result"]["contents"][0]["mimeType"],
        "text/html;profile=mcp-app"
    );
    let forbidden = extension_rpc_body(
        extension_rpc(
            state.clone(),
            Some("Bearer send-fixture"),
            "resources/read",
            serde_json::json!({"uri":"ui://whatsapp-translator/chats"}),
        )
        .await,
    )
    .await;
    assert!(forbidden.get("error").is_some());
    let send = extension_rpc_body(extension_rpc(state.clone(), Some("Bearer read-fixture"), "tools/call", serde_json::json!({"name":"send_message","arguments":{"preparation_token":"fake","idempotency_key":"synthetic-key"}})).await).await;
    assert!(send.get("error").is_some());
    assert!(state.mcp_prepared_messages.read().await.is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}
