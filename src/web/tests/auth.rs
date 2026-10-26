//! Web auth, logout and sign-out revocation tests.

use super::*;

#[tokio::test]
async fn protected_api_allows_requests_when_password_is_not_configured() {
    let (state, data_dir) = test_state(None);
    let response = create_router(state)
        .oneshot(empty_request("/api/status"))
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let _ = std::fs::remove_dir_all(data_dir);
}

#[tokio::test]
async fn protected_api_rejects_missing_token_when_password_is_configured() {
    let (state, data_dir) = test_state(Some("secret"));
    let response = create_router(state)
        .oneshot(empty_request("/api/status"))
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let _ = std::fs::remove_dir_all(data_dir);
}

#[tokio::test]
async fn health_check_stays_public_when_password_is_configured() {
    let (state, data_dir) = test_state(Some("secret"));
    let response = create_router(state)
        .oneshot(empty_request("/api/health"))
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let payload: serde_json::Value = serde_json::from_slice(&body).expect("health json");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["service"], "whatsapp-translator");

    let _ = std::fs::remove_dir_all(data_dir);
}

#[tokio::test]
async fn protected_api_accepts_valid_bearer_token() {
    let (state, data_dir) = test_state(Some("secret"));
    let token = generate_token();
    state
        .auth_tokens
        .write()
        .await
        .insert(token.clone(), chrono::Utc::now().timestamp() + 60);

    let request = HttpRequest::builder()
        .uri("/api/status")
        .header(header::AUTHORIZATION, format!("Bearer {}", token))
        .body(Body::empty())
        .expect("request");

    let response = create_router(state)
        .oneshot(request)
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let _ = std::fs::remove_dir_all(data_dir);
}

#[tokio::test]
async fn authenticated_push_device_registration_is_persisted() {
    let (state, data_dir) = test_state(None);
    let request = HttpRequest::builder()
        .method("PUT")
        .uri("/api/push/devices")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            r#"{"installationId":"installation-1","token":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef","environment":"sandbox"}"#,
        ))
        .expect("request");

    let response = create_router(state.clone())
        .oneshot(request)
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let devices = state.store.list_push_devices().expect("push devices");
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].installation_id, "installation-1");
    assert_eq!(devices[0].environment, "sandbox");

    let _ = std::fs::remove_dir_all(data_dir);
}

#[tokio::test]
async fn websocket_route_rejects_missing_token_before_upgrade() {
    let (state, data_dir) = test_state(Some("secret"));
    let response = create_router(state)
        .oneshot(empty_request("/ws"))
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let _ = std::fs::remove_dir_all(data_dir);
}

#[tokio::test]
async fn logout_route_rejects_missing_token_when_password_is_configured() {
    let (state, data_dir) = test_state(Some("secret"));
    let request = HttpRequest::builder()
        .method("POST")
        .uri("/api/logout")
        .body(Body::empty())
        .expect("request");

    let response = create_router(state)
        .oneshot(request)
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let _ = std::fs::remove_dir_all(data_dir);
}

#[tokio::test]
async fn logout_clears_messages_oauth_clients_and_ui_tokens() {
    let (state, data_dir) = test_state(Some("secret"));
    for filename in ["session.db", "session.db-wal", "session.db-shm"] {
        std::fs::write(data_dir.join(filename), b"session").expect("write session file");
    }
    state
        .store
        .upsert_contact(
            "chat@example.test",
            Some("Test Chat"),
            None,
            Some("private"),
            1_700_000_000_000,
        )
        .expect("insert contact");
    state
        .store
        .add_message(&test_outgoing_message("msg_1", 1_700_000_000_000))
        .expect("insert message");

    let client = OAuthClientRegistration {
        client_id: "client_test".to_string(),
        client_name: Some("Local MCP Client".to_string()),
        redirect_uris: vec!["http://127.0.0.1:8787/callback".to_string()],
        scope: "mcp".to_string(),
        created_at: 1_700_000_000,
    };
    state
        .store
        .oauth_store_client(&client)
        .expect("store oauth client");
    state
        .store
        .oauth_store_access_token(&AccessToken {
            token: "access-token".to_string(),
            client_id: client.client_id.clone(),
            scope: "mcp".to_string(),
            created_at: 1_700_000_000,
            expires_at: i64::MAX,
            refresh_token: None,
        })
        .expect("store access token");
    state
        .store
        .oauth_store_refresh_token(&RefreshToken {
            token: "refresh-token".to_string(),
            client_id: client.client_id.clone(),
            scope: "mcp".to_string(),
            created_at: 1_700_000_000,
            expires_at: i64::MAX,
        })
        .expect("store refresh token");

    let ui_token = generate_token();
    state
        .auth_tokens
        .write()
        .await
        .insert(ui_token.clone(), chrono::Utc::now().timestamp() + 60);
    state.mcp_prepared_messages.write().await.insert(
        "prepared".to_string(),
        serde_json::json!({"expiresAt":i64::MAX}),
    );
    state
        .store
        .claim_mcp_send("idempotent", &serde_json::json!({"expiresAt":i64::MAX}))
        .unwrap();

    let request = HttpRequest::builder()
        .method("POST")
        .uri("/api/logout")
        .header(header::AUTHORIZATION, format!("Bearer {}", ui_token))
        .body(Body::empty())
        .expect("request");

    let response = create_router(state.clone())
        .oneshot(request)
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    assert!(state.store.get_contacts().expect("contacts").is_empty());
    assert!(state
        .store
        .get_messages_paginated("chat@example.test", None, None, None, true)
        .expect("messages")
        .is_empty());
    assert!(state
        .store
        .oauth_list_clients()
        .expect("oauth clients")
        .is_empty());
    assert!(state
        .store
        .oauth_validate_access_token("access-token")
        .expect("access token")
        .is_none());
    assert!(state
        .store
        .oauth_get_refresh_token("refresh-token")
        .expect("refresh token")
        .is_none());
    assert!(state.auth_tokens.read().await.is_empty());
    assert!(state.mcp_prepared_messages.read().await.is_empty());
    assert!(state.store.mcp_send_record("idempotent").unwrap().is_none());
    for filename in ["session.db", "session.db-wal", "session.db-shm"] {
        assert!(
            !data_dir.join(filename).exists(),
            "{filename} should be removed"
        );
    }
    assert!(!state.take_session_reset_request());

    let _ = std::fs::remove_dir_all(data_dir);
}

#[tokio::test]
async fn logout_defers_session_file_cleanup_until_bridge_restart_when_bridge_is_running() {
    let (state, data_dir) = test_state(Some("secret"));
    let (command_tx, mut command_rx) = mpsc::channel(1);
    state.set_command_tx(command_tx).await;

    let ui_token = generate_token();
    state
        .auth_tokens
        .write()
        .await
        .insert(ui_token.clone(), chrono::Utc::now().timestamp() + 60);

    let request = HttpRequest::builder()
        .method("POST")
        .uri("/api/logout")
        .header(header::AUTHORIZATION, format!("Bearer {}", ui_token))
        .body(Body::empty())
        .expect("request");

    let response = create_router(state.clone())
        .oneshot(request)
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    assert!(matches!(command_rx.try_recv(), Ok(BridgeCommand::Logout)));
    assert!(state.take_session_reset_request());

    let _ = std::fs::remove_dir_all(data_dir);
}

fn store_linked_oauth_grant(
    state: &AppState,
    client_id: &str,
    access_token: &str,
    refresh_token: &str,
) {
    state
        .store
        .oauth_store_client(&OAuthClientRegistration {
            client_id: client_id.to_string(),
            client_name: Some("Synthetic client".to_string()),
            redirect_uris: vec!["http://127.0.0.1:8787/callback".to_string()],
            scope: "whatsapp.read".to_string(),
            created_at: 1_700_000_000,
        })
        .expect("store oauth client");
    state
        .store
        .oauth_store_refresh_token(&RefreshToken {
            token: refresh_token.to_string(),
            client_id: client_id.to_string(),
            scope: "whatsapp.read".to_string(),
            created_at: 1_700_000_000,
            expires_at: i64::MAX,
        })
        .expect("store refresh token");
    state
        .store
        .oauth_store_access_token(&AccessToken {
            token: access_token.to_string(),
            client_id: client_id.to_string(),
            scope: "whatsapp.read".to_string(),
            created_at: 1_700_000_000,
            expires_at: i64::MAX,
            refresh_token: Some(refresh_token.to_string()),
        })
        .expect("store access token");
}

async fn delete_session(state: Arc<AppState>, token: Option<&str>) -> Response {
    let mut request = HttpRequest::builder()
        .method("DELETE")
        .uri("/api/auth/session");
    if let Some(token) = token {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    create_router(state)
        .oneshot(request.body(Body::empty()).expect("request"))
        .await
        .expect("response")
}

async fn status_code_for(state: Arc<AppState>, token: &str) -> StatusCode {
    let request = HttpRequest::builder()
        .uri("/api/status")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .expect("request");
    create_router(state)
        .oneshot(request)
        .await
        .expect("response")
        .status()
}

fn remembered_message_count(state: &AppState) -> usize {
    state
        .store
        .get_messages_paginated("chat@example.test", None, None, None, true)
        .expect("messages")
        .len()
}

#[tokio::test]
async fn sign_out_revokes_only_the_calling_web_session() {
    let (state, data_dir) = test_state(Some("secret"));
    std::fs::write(data_dir.join("session.db"), b"session").expect("session file");
    let (command_tx, mut command_rx) = mpsc::channel(1);
    state.set_command_tx(command_tx).await;
    *state.connected.write().await = true;
    state
        .store
        .upsert_contact(
            "chat@example.test",
            Some("Test Chat"),
            None,
            Some("private"),
            1_700_000_000_000,
        )
        .expect("insert contact");
    state
        .store
        .add_message(&test_outgoing_message("msg_1", 1_700_000_000_000))
        .expect("insert message");
    store_linked_oauth_grant(&state, "client-kept", "access-kept", "refresh-kept");
    let device_a = generate_token();
    let device_b = generate_token();
    let expires_at = chrono::Utc::now().timestamp() + 60;
    {
        let mut tokens = state.auth_tokens.write().await;
        tokens.insert(device_a.clone(), expires_at);
        tokens.insert(device_b.clone(), expires_at);
    }
    let epoch = state.voice_epoch.load(Ordering::SeqCst);

    let response = delete_session(state.clone(), Some(&device_a)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(payload["success"], true);
    assert!(!String::from_utf8(body.to_vec())
        .expect("text")
        .contains(&device_a));

    assert_eq!(
        status_code_for(state.clone(), &device_a).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        status_code_for(state.clone(), &device_b).await,
        StatusCode::OK
    );
    assert_eq!(remembered_message_count(&state), 1);
    assert!(*state.connected.read().await);
    assert!(data_dir.join("session.db").exists());
    assert!(command_rx.try_recv().is_err());
    assert!(!state.take_session_reset_request());
    assert_eq!(state.voice_epoch.load(Ordering::SeqCst), epoch);
    assert!(state
        .store
        .oauth_validate_access_token("access-kept")
        .expect("kept access")
        .is_some());
    assert!(state
        .store
        .oauth_get_refresh_token("refresh-kept")
        .expect("kept refresh")
        .is_some());
    assert_eq!(state.store.oauth_list_clients().expect("clients").len(), 1);
    assert!(state.auth_tokens.read().await.contains_key(&device_b));
    assert!(!state.auth_tokens.read().await.contains_key(&device_a));

    let _ = std::fs::remove_dir_all(data_dir);
}

#[tokio::test]
async fn sign_out_revokes_oauth_access_token_and_its_refresh_token_only() {
    let (state, data_dir) = test_state(Some("secret"));
    std::fs::write(data_dir.join("session.db"), b"session").expect("session file");
    let (command_tx, mut command_rx) = mpsc::channel(1);
    state.set_command_tx(command_tx).await;
    *state.connected.write().await = true;
    state
        .store
        .upsert_contact(
            "chat@example.test",
            Some("Test Chat"),
            None,
            Some("private"),
            1_700_000_000_000,
        )
        .expect("insert contact");
    state
        .store
        .add_message(&test_outgoing_message("msg_1", 1_700_000_000_000))
        .expect("insert message");
    store_linked_oauth_grant(
        &state,
        "client-device-a",
        "access-device-a",
        "refresh-device-a",
    );
    state
        .store
        .oauth_store_access_token(&AccessToken {
            token: "access-device-a-renewed".to_string(),
            client_id: "client-device-a".to_string(),
            scope: "whatsapp.read".to_string(),
            created_at: 1_700_000_100,
            expires_at: i64::MAX,
            refresh_token: Some("refresh-device-a".to_string()),
        })
        .expect("store renewed access token");
    store_linked_oauth_grant(
        &state,
        "client-device-b",
        "access-device-b",
        "refresh-device-b",
    );
    let ios_token = generate_token();
    state
        .auth_tokens
        .write()
        .await
        .insert(ios_token.clone(), chrono::Utc::now().timestamp() + 60);

    assert_eq!(
        extension_rpc(
            state.clone(),
            Some("Bearer access-device-a"),
            "tools/list",
            serde_json::json!({})
        )
        .await
        .status(),
        StatusCode::OK
    );

    let mut request = HttpRequest::builder()
        .method("DELETE")
        .uri("/api/auth/session")
        .header(header::AUTHORIZATION, "Bearer access-device-a");
    request = request.header(header::CONTENT_TYPE, "application/json");
    let response = create_router(state.clone())
        .oneshot(
            request
                .body(Body::from(
                    r#"{"token":"access-device-b","refresh_token":"refresh-device-b"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);

    assert_eq!(
        extension_rpc(
            state.clone(),
            Some("Bearer access-device-a"),
            "tools/list",
            serde_json::json!({})
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        extension_rpc(
            state.clone(),
            Some("Bearer access-device-a-renewed"),
            "tools/list",
            serde_json::json!({})
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        extension_rpc(
            state.clone(),
            Some("Bearer access-device-b"),
            "tools/list",
            serde_json::json!({})
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert!(state
        .store
        .oauth_get_refresh_token("refresh-device-a")
        .expect("revoked refresh")
        .is_none());
    assert!(state
        .store
        .oauth_get_refresh_token("refresh-device-b")
        .expect("kept refresh")
        .is_some());
    assert_eq!(
        status_code_for(state.clone(), &ios_token).await,
        StatusCode::OK
    );
    assert_eq!(state.store.oauth_list_clients().expect("clients").len(), 2);
    assert_eq!(remembered_message_count(&state), 1);
    assert!(*state.connected.read().await);
    assert!(data_dir.join("session.db").exists());
    assert!(command_rx.try_recv().is_err());
    assert!(!state.take_session_reset_request());

    let repeat = delete_session(state.clone(), Some("access-device-a")).await;
    assert_eq!(repeat.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        extension_rpc(
            state.clone(),
            Some("Bearer access-device-b"),
            "tools/list",
            serde_json::json!({})
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert!(state
        .store
        .oauth_get_refresh_token("refresh-device-b")
        .expect("refresh after repeat")
        .is_some());

    let _ = std::fs::remove_dir_all(data_dir);
}

#[tokio::test]
async fn sign_out_without_or_with_unknown_token_revokes_nothing() {
    let (state, data_dir) = test_state(Some("secret"));
    *state.connected.write().await = true;
    state
        .store
        .upsert_contact(
            "chat@example.test",
            Some("Test Chat"),
            None,
            Some("private"),
            1_700_000_000_000,
        )
        .expect("insert contact");
    state
        .store
        .add_message(&test_outgoing_message("msg_1", 1_700_000_000_000))
        .expect("insert message");
    store_linked_oauth_grant(
        &state,
        "client-device-b",
        "access-device-b",
        "refresh-device-b",
    );
    let device_b = generate_token();
    state
        .auth_tokens
        .write()
        .await
        .insert(device_b.clone(), chrono::Utc::now().timestamp() + 60);

    for token in [None, Some("unknown-token")] {
        let response = delete_session(state.clone(), token).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    assert_eq!(
        status_code_for(state.clone(), &device_b).await,
        StatusCode::OK
    );
    assert_eq!(
        extension_rpc(
            state.clone(),
            Some("Bearer access-device-b"),
            "tools/list",
            serde_json::json!({})
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert!(state
        .store
        .oauth_get_refresh_token("refresh-device-b")
        .expect("refresh")
        .is_some());
    assert_eq!(remembered_message_count(&state), 1);
    assert!(*state.connected.read().await);
    assert_eq!(state.store.oauth_list_clients().expect("clients").len(), 1);

    let _ = std::fs::remove_dir_all(data_dir);
}

#[tokio::test]
async fn sign_out_without_a_password_revokes_only_a_presented_token() {
    let (state, data_dir) = test_state(None);
    *state.connected.write().await = true;
    state
        .store
        .upsert_contact(
            "chat@example.test",
            Some("Test Chat"),
            None,
            Some("private"),
            1_700_000_000_000,
        )
        .expect("insert contact");
    state
        .store
        .add_message(&test_outgoing_message("msg_1", 1_700_000_000_000))
        .expect("insert message");
    store_linked_oauth_grant(&state, "client-shared", "access-linked", "refresh-shared");
    state
        .store
        .oauth_store_access_token(&AccessToken {
            token: "access-unlinked".to_string(),
            client_id: "client-shared".to_string(),
            scope: "whatsapp.read".to_string(),
            created_at: 1_700_000_000,
            expires_at: i64::MAX,
            refresh_token: None,
        })
        .expect("store unlinked access token");
    let device_a = generate_token();
    let device_b = generate_token();
    let expired = generate_token();
    {
        let mut tokens = state.auth_tokens.write().await;
        let now = chrono::Utc::now().timestamp();
        tokens.insert(device_a.clone(), now + 60);
        tokens.insert(device_b.clone(), now + 60);
        tokens.insert(expired.clone(), now - 60);
    }

    for token in [None, Some("unknown-token")] {
        let response = delete_session(state.clone(), token).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(payload["success"], true);
    }
    let tokens = state.auth_tokens.read().await;
    assert!(tokens.contains_key(&device_a));
    assert!(tokens.contains_key(&device_b));
    assert!(tokens.contains_key(&expired));
    drop(tokens);
    assert!(state
        .store
        .oauth_validate_access_token("access-linked")
        .expect("linked")
        .is_some());
    assert!(state
        .store
        .oauth_validate_access_token("access-unlinked")
        .expect("unlinked")
        .is_some());
    assert!(state
        .store
        .oauth_get_refresh_token("refresh-shared")
        .expect("refresh")
        .is_some());

    let request = HttpRequest::builder()
        .method("DELETE")
        .uri("/api/auth/session")
        .header(header::AUTHORIZATION, format!("Bearer {device_a}"))
        .header(header::CONTENT_TYPE, "application/json");
    let response = create_router(state.clone())
        .oneshot(
            request
                .body(Body::from(
                    r#"{"token":"access-linked","refresh_token":"refresh-shared"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let tokens = state.auth_tokens.read().await;
    assert!(!tokens.contains_key(&device_a));
    assert!(tokens.contains_key(&device_b));
    assert!(tokens.contains_key(&expired));
    drop(tokens);
    assert!(state
        .store
        .oauth_get_refresh_token("refresh-shared")
        .expect("refresh after web sign-out")
        .is_some());

    let oauth_response = delete_session(state.clone(), Some("access-unlinked")).await;
    assert_eq!(oauth_response.status(), StatusCode::OK);
    assert!(state
        .store
        .oauth_validate_access_token("access-unlinked")
        .expect("unlinked after sign-out")
        .is_none());
    assert!(state
        .store
        .oauth_validate_access_token("access-linked")
        .expect("linked after unlinked sign-out")
        .is_some());
    assert!(state
        .store
        .oauth_get_refresh_token("refresh-shared")
        .expect("refresh after unlinked sign-out")
        .is_some());
    assert!(state.auth_tokens.read().await.contains_key(&device_b));
    assert_eq!(remembered_message_count(&state), 1);
    assert!(*state.connected.read().await);
    assert_eq!(state.store.oauth_list_clients().expect("clients").len(), 1);

    let _ = std::fs::remove_dir_all(data_dir);
}

#[tokio::test]
async fn refresh_log_omits_client_id_token_and_hash() {
    let (state, data_dir) = test_state(None);
    let client_id = "client-log-check";
    let refresh = "refresh-log-check";
    let invalid_refresh = "not-a-real-refresh-token";
    state
        .store
        .oauth_store_client(&OAuthClientRegistration {
            client_id: client_id.to_string(),
            client_name: Some("Synthetic client".to_string()),
            redirect_uris: vec!["http://127.0.0.1:8787/callback".to_string()],
            scope: "whatsapp.read".to_string(),
            created_at: 1_700_000_000,
        })
        .expect("client");
    state
        .store
        .oauth_store_refresh_token(&RefreshToken {
            token: refresh.to_string(),
            client_id: client_id.to_string(),
            scope: "whatsapp.read".to_string(),
            created_at: 1_700_000_000,
            expires_at: i64::MAX,
        })
        .expect("refresh");

    let buf = Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(crate::log_privacy::logging_filter(false))
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_writer({
            let buf = buf.clone();
            move || LogCapture(buf.clone())
        })
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let response = handle_refresh_token_grant(
        state.clone(),
        TokenRequest {
            grant_type: "refresh_token".to_string(),
            code: None,
            redirect_uri: None,
            code_verifier: None,
            refresh_token: Some(refresh.to_string()),
            client_id: Some(client_id.to_string()),
            resource: None,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");
    let access = payload["access_token"]
        .as_str()
        .expect("issued access token")
        .to_string();
    assert_ne!(access, refresh);
    assert!(state
        .store
        .oauth_validate_access_token(&access)
        .expect("issued token")
        .is_some());

    let invalid = handle_refresh_token_grant(
        state.clone(),
        TokenRequest {
            grant_type: "refresh_token".to_string(),
            code: None,
            redirect_uri: None,
            code_verifier: None,
            refresh_token: Some(invalid_refresh.to_string()),
            client_id: Some(client_id.to_string()),
            resource: None,
        },
    )
    .await;
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

    let logs = String::from_utf8(buf.lock().expect("log buffer").clone()).expect("logs");
    assert!(logs.contains("OAuth access token refreshed"));
    let refresh_hash = crate::oauth::oauth_secret_hash(refresh);
    let access_hash = crate::oauth::oauth_secret_hash(&access);
    let invalid_hash = crate::oauth::oauth_secret_hash(invalid_refresh);
    for secret in [
        client_id,
        refresh,
        access.as_str(),
        invalid_refresh,
        refresh_hash.as_str(),
        access_hash.as_str(),
        invalid_hash.as_str(),
    ] {
        assert!(!logs.contains(secret), "log contained a secret");
    }
    drop(_guard);

    let _ = std::fs::remove_dir_all(data_dir);
}

struct LogCapture(Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for LogCapture {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("log buffer").extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn diagnostics_need_auth_and_leave_out_account_details() {
    let (state, data_dir) = test_state(Some("secret"));
    state
        .set_connected(true, Some("447700900123".into()), Some("Preview".into()))
        .await;
    let response = create_router(state.clone())
        .oneshot(empty_request("/api/diagnostics"))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let token = generate_token();
    state
        .auth_tokens
        .write()
        .await
        .insert(token.clone(), chrono::Utc::now().timestamp() + 60);
    let request = HttpRequest::builder()
        .uri("/api/diagnostics")
        .header(header::AUTHORIZATION, format!("Bearer {}", token))
        .body(Body::empty())
        .expect("request");
    let response = create_router(state)
        .oneshot(request)
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let text = String::from_utf8(body.to_vec()).expect("utf8");
    let payload: serde_json::Value = serde_json::from_str(&text).expect("json");
    assert_eq!(payload["connected"], true);
    assert!(payload["recentProblems"].is_array());
    assert!(!text.contains("447700900123") && !text.contains("Preview"));
    let _ = std::fs::remove_dir_all(data_dir);
}
