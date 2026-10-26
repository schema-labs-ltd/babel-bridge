//! Web auth, logout and sign-out revocation tests.

use super::*;

mod sign_out;

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
