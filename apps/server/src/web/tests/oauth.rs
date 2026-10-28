//! OAuth scope, redirect, registration and ChatGPT connector flow tests.

use super::*;

#[test]
fn oauth_scopes_default_to_read_only_and_are_canonicalized() {
    assert_eq!(
        normalize_oauth_scope(None).expect("default scope"),
        "whatsapp.read"
    );
    assert_eq!(
        normalize_oauth_scope(Some("whatsapp.send whatsapp.read")).expect("valid scopes"),
        "whatsapp.read whatsapp.send"
    );
    assert!(normalize_oauth_scope(Some("whatsapp.send")).is_err());
    assert!(normalize_oauth_scope(Some("contacts.admin")).is_err());
}

#[test]
fn oauth_dcr_omitted_scope_allows_supported_permissions_but_defaults_grants_to_read() {
    let mut request = ClientRegistrationRequest {
        redirect_uris: vec!["https://chatgpt.com/connector/oauth/fixture-callback".into()],
        client_name: None,
        client_uri: None,
        scope: None,
        grant_types: None,
        response_types: None,
        token_endpoint_auth_method: None,
    };
    let eligibility = validate_oauth_client_request(&request).unwrap();
    assert_eq!(eligibility, "whatsapp.read whatsapp.send");
    assert_eq!(
        resolve_oauth_authorization_scope(None, &eligibility).unwrap(),
        "whatsapp.read"
    );
    assert_eq!(
        resolve_oauth_authorization_scope(Some(""), &eligibility).unwrap(),
        "whatsapp.read"
    );
    assert_eq!(
        resolve_oauth_authorization_scope(Some("whatsapp.read whatsapp.send"), &eligibility)
            .unwrap(),
        eligibility
    );
    for scope in ["whatsapp.read", "", " "] {
        request.scope = Some(scope.into());
        let limited = validate_oauth_client_request(&request).unwrap();
        assert_eq!(limited, "whatsapp.read");
        assert!(
            resolve_oauth_authorization_scope(Some("whatsapp.read whatsapp.send"), &limited)
                .is_err()
        );
    }
    for invalid in ["whatsapp.send", "unknown"] {
        request.scope = Some(invalid.into());
        assert!(validate_oauth_client_request(&request).is_err());
    }
    assert_eq!(
        resolve_oauth_authorization_scope(None, "mcp").unwrap(),
        "mcp"
    );
}

#[test]
fn oauth_authorization_can_narrow_registered_permissions() {
    assert_eq!(
        resolve_oauth_authorization_scope(None, "whatsapp.read whatsapp.send")
            .expect("registered default"),
        "whatsapp.read"
    );
    assert_eq!(
        resolve_oauth_authorization_scope(Some("whatsapp.read"), "whatsapp.read whatsapp.send")
            .expect("narrow scope"),
        "whatsapp.read"
    );
    assert!(resolve_oauth_authorization_scope(
        Some("whatsapp.read whatsapp.send"),
        "whatsapp.read"
    )
    .is_err());
    assert_eq!(
        resolve_oauth_authorization_scope(Some("whatsapp.read whatsapp.send"), "mcp")
            .expect("legacy full access"),
        "whatsapp.read whatsapp.send"
    );
}

#[test]
fn oauth_redirect_validation_allows_loopback_and_exact_chatgpt_callback_paths() {
    assert!(validate_oauth_redirect_uri("http://127.0.0.1:8787/callback").is_ok());
    assert!(validate_oauth_redirect_uri("http://localhost/callback").is_ok());
    assert!(validate_oauth_redirect_uri("http://[::1]:8787/callback").is_ok());
    for allowed in [
        "https://chatgpt.com/connector/oauth/callback_fixture-123",
        "https://chatgpt.com/connector_platform_oauth_redirect",
    ] {
        assert!(validate_oauth_redirect_uri(allowed).is_ok(), "{allowed}");
    }
    for denied in [
        "https://example.com/callback",
        "file:///tmp/callback",
        "http://127.0.0.1/callback#fragment",
        "https://chatgpt.com.evil.test/connector/oauth/fixture",
        "https://evil.test@chatgpt.com/connector/oauth/fixture",
        "https://chatgpt.com@evil.test/connector/oauth/fixture",
        "http://chatgpt.com/connector/oauth/fixture",
        "https://chatgpt.com:8443/connector/oauth/fixture",
        "https://chatgpt.com/connector/oauth/fixture?next=https://evil.test",
        "https://chatgpt.com/connector/oauth/fixture#fragment",
        "https://chatgpt.com/connector/oauth/",
        "https://chatgpt.com/connector/oauth/fixture/extra",
        "https://chatgpt.com/connector/oauth/fixture%2Fextra",
        "https://chatgpt.com/other",
        "http://user@localhost/callback",
    ] {
        assert!(validate_oauth_redirect_uri(denied).is_err(), "{denied}");
    }
}

#[test]
fn oauth_resource_is_bound_to_the_instance_mcp_endpoint() {
    let host = "translator.example.test";
    assert!(oauth_resource_matches(None, host));
    assert!(oauth_resource_matches(
        Some("https://translator.example.test/mcp"),
        host
    ));
    for resource in [
        "",
        "https://other.example.test/mcp",
        "http://translator.example.test/mcp",
        "https://translator.example.test",
        "https://translator.example.test/mcp/",
    ] {
        assert!(!oauth_resource_matches(Some(resource), host));
    }
}

#[test]
fn request_host_loopback_detection_is_exact() {
    assert!(request_host_is_loopback("localhost:3000"));
    assert!(request_host_is_loopback("127.0.0.1:3000"));
    assert!(request_host_is_loopback("[::1]:3000"));
    assert!(!request_host_is_loopback("localhost.example.com"));
    assert!(!request_host_is_loopback("translator.example.com"));
}

#[tokio::test]
async fn oauth_registration_rejects_non_loopback_redirects() {
    let (state, data_dir) = test_state(None);
    let request = HttpRequest::builder()
        .method("POST")
        .uri("/oauth/register")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({
                "redirect_uris": ["https://example.com/callback"],
                "client_name": "Remote Client"
            })
            .to_string(),
        ))
        .expect("request");

    let response = create_router(state)
        .oneshot(request)
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let _ = std::fs::remove_dir_all(data_dir);
}

#[tokio::test]
async fn oauth_registration_persists_loopback_client() {
    let (state, data_dir) = test_state(None);
    let request = HttpRequest::builder()
        .method("POST")
        .uri("/oauth/register")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({
                "redirect_uris": ["http://127.0.0.1:8787/callback"],
                "client_name": "Local MCP Client"
            })
            .to_string(),
        ))
        .expect("request");

    let response = create_router(state.clone())
        .oneshot(request)
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::CREATED);
    let clients = state.store.oauth_list_clients().expect("clients");
    assert_eq!(clients.len(), 1);
    assert_eq!(clients[0].client_name.as_deref(), Some("Local MCP Client"));
    assert_eq!(
        clients[0].redirect_uris,
        vec!["http://127.0.0.1:8787/callback".to_string()]
    );

    let _ = std::fs::remove_dir_all(data_dir);
}

async fn oauth_fixture_request(
    state: Arc<AppState>,
    method: &str,
    uri: &str,
    content_type: &str,
    body: String,
) -> axum::response::Response {
    create_router(state)
        .oneshot(
            HttpRequest::builder()
                .method(method)
                .uri(uri)
                .header("host", "translator.example.test")
                .header("content-type", content_type)
                .header("accept", "application/json, text/event-stream")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap()
}

fn oauth_fixture_form(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(key, value)| format!("{}={}", key, urlencoding::encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}

async fn oauth_fixture_json(response: axum::response::Response) -> serde_json::Value {
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn oauth_chatgpt_dcr_discovery_approval_pkce_and_read_only_access() {
    oauth_chatgpt_fixture_flow(
        Some("whatsapp.read"),
        Some("whatsapp.read"),
        "whatsapp.read",
    )
    .await;
}

#[tokio::test]
async fn oauth_chatgpt_dcr_without_scope_allows_explicit_read_send_consent() {
    oauth_chatgpt_fixture_flow(
        None,
        Some("whatsapp.read whatsapp.send"),
        "whatsapp.read whatsapp.send",
    )
    .await;
}

#[tokio::test]
async fn oauth_chatgpt_dcr_without_scope_and_authorization_without_scope_grants_only_read() {
    oauth_chatgpt_fixture_flow(None, None, "whatsapp.read").await;
}

async fn oauth_chatgpt_fixture_flow(
    registration_scope: Option<&str>,
    requested_scope: Option<&str>,
    granted_scope: &str,
) {
    use sha2::Digest;
    let (state, dir) = test_state(Some("fixture-password"));
    let callback = "https://chatgpt.com/connector/oauth/fixture-callback";
    let resource = "https://translator.example.test/mcp";
    let issuer = "https://translator.example.test";
    let verifier = "synthetic-PKCE-verifier-with-more-than-43-characters-1234";
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(sha2::Sha256::digest(verifier.as_bytes()));

    let metadata = oauth_fixture_json(
        oauth_fixture_request(
            state.clone(),
            "GET",
            "/.well-known/oauth-protected-resource",
            "application/json",
            String::new(),
        )
        .await,
    )
    .await;
    assert_eq!(metadata["resource"], resource);
    assert_eq!(metadata["authorization_servers"][0], issuer);
    let auth_metadata = oauth_fixture_json(
        oauth_fixture_request(
            state.clone(),
            "GET",
            "/.well-known/oauth-authorization-server",
            "application/json",
            String::new(),
        )
        .await,
    )
    .await;
    assert_eq!(auth_metadata["issuer"], issuer);
    assert_eq!(
        auth_metadata["registration_endpoint"],
        format!("{issuer}/oauth/register")
    );
    assert_eq!(auth_metadata["code_challenge_methods_supported"][0], "S256");
    assert_eq!(
        auth_metadata["token_endpoint_auth_methods_supported"][0],
        "none"
    );
    assert_ne!(auth_metadata["client_id_metadata_document_supported"], true);
    let unauthenticated = oauth_fixture_request(state.clone(), "POST", "/mcp", "application/json", serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"fixture","version":"1"}}}).to_string()).await;
    assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(unauthenticated.headers()["www-authenticate"], format!("Bearer resource_metadata=\"{issuer}/.well-known/oauth-protected-resource\", scope=\"whatsapp.read\""));

    let mut registration_request = serde_json::json!({"redirect_uris":[callback],"client_name":"Synthetic ChatGPT","grant_types":["authorization_code","refresh_token"],"response_types":["code"],"token_endpoint_auth_method":"none"});
    if let Some(scope) = registration_scope {
        registration_request["scope"] = scope.into();
    }
    let registered = oauth_fixture_request(
        state.clone(),
        "POST",
        "/oauth/register",
        "application/json",
        registration_request.to_string(),
    )
    .await;
    assert_eq!(registered.status(), StatusCode::CREATED);
    let registration = oauth_fixture_json(registered).await;
    let client_id = registration["client_id"].as_str().unwrap();
    assert_eq!(
        registration["scope"],
        registration_scope.unwrap_or("whatsapp.read whatsapp.send")
    );
    assert!(registration.get("client_secret").is_none());
    let auth_pairs = [
        ("response_type", "code"),
        ("client_id", client_id),
        ("redirect_uri", callback),
        ("scope", requested_scope.unwrap_or("whatsapp.read")),
        ("state", "fixture state & value"),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
        ("resource", resource),
    ];
    for (index, bad_value) in [
        (2, "https://chatgpt.com/connector/oauth/different"),
        (
            3,
            if registration_scope == Some("whatsapp.read") {
                "whatsapp.read whatsapp.send"
            } else {
                "unsupported.scope"
            },
        ),
        (6, "plain"),
        (7, "https://other.example.test/mcp"),
    ] {
        let mut invalid = auth_pairs;
        invalid[index].1 = bad_value;
        let response = oauth_fixture_request(
            state.clone(),
            "GET",
            &format!("/oauth/authorize?{}", oauth_fixture_form(&invalid)),
            "application/json",
            String::new(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
    let authorization_pairs: Vec<_> = auth_pairs
        .iter()
        .copied()
        .filter(|(key, _)| *key != "scope" || requested_scope.is_some())
        .collect();
    let page = oauth_fixture_request(
        state.clone(),
        "GET",
        &format!(
            "/oauth/authorize?{}",
            oauth_fixture_form(&authorization_pairs)
        ),
        "application/json",
        String::new(),
    )
    .await;
    assert_eq!(page.status(), StatusCode::OK);
    let html = String::from_utf8(
        axum::body::to_bytes(page.into_body(), 1024 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(html.contains("name=\"password\""));
    assert!(html.contains(&format!(
        "<strong>Requested permissions:</strong> {granted_scope}<br>"
    )));
    let session_key = html
        .split("name=\"session_key\" value=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    let denied = oauth_fixture_request(
        state.clone(),
        "POST",
        "/oauth/approve",
        "application/x-www-form-urlencoded",
        oauth_fixture_form(&[
            ("session_key", session_key),
            ("approved", "true"),
            ("password", "wrong-password"),
        ]),
    )
    .await;
    assert!(denied.headers()["location"]
        .to_str()
        .unwrap()
        .contains("error=access_denied"));

    let page = oauth_fixture_request(
        state.clone(),
        "GET",
        &format!(
            "/oauth/authorize?{}",
            oauth_fixture_form(&authorization_pairs)
        ),
        "application/json",
        String::new(),
    )
    .await;
    let html = String::from_utf8(
        axum::body::to_bytes(page.into_body(), 1024 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    let session_key = html
        .split("name=\"session_key\" value=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    let approved = oauth_fixture_request(
        state.clone(),
        "POST",
        "/oauth/approve",
        "application/x-www-form-urlencoded",
        oauth_fixture_form(&[
            ("session_key", session_key),
            ("approved", "true"),
            ("password", "fixture-password"),
        ]),
    )
    .await;
    assert_eq!(approved.status(), StatusCode::SEE_OTHER);
    let redirect = reqwest::Url::parse(approved.headers()["location"].to_str().unwrap()).unwrap();
    assert_eq!(redirect.path(), "/connector/oauth/fixture-callback");
    let query: std::collections::HashMap<_, _> = redirect.query_pairs().collect();
    assert_eq!(query.get("state").unwrap(), "fixture state & value");
    let code = query.get("code").unwrap();
    let token_pairs = [
        ("grant_type", "authorization_code"),
        ("code", code.as_ref()),
        ("client_id", client_id),
        ("redirect_uri", callback),
        ("code_verifier", verifier),
        ("resource", resource),
    ];
    let mut invalid_resource = token_pairs;
    invalid_resource[5].1 = "https://other.example.test/mcp";
    let wrong_target = oauth_fixture_request(
        state.clone(),
        "POST",
        "/oauth/token",
        "application/x-www-form-urlencoded",
        oauth_fixture_form(&invalid_resource),
    )
    .await;
    assert_eq!(wrong_target.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        oauth_fixture_json(wrong_target).await["error"],
        "invalid_target"
    );
    let token = oauth_fixture_request(
        state.clone(),
        "POST",
        "/oauth/token",
        "application/x-www-form-urlencoded",
        oauth_fixture_form(&token_pairs),
    )
    .await;
    assert_eq!(token.status(), StatusCode::OK);
    let token = oauth_fixture_json(token).await;
    assert_eq!(token["scope"], granted_scope);
    let auth = format!("Bearer {}", token["access_token"].as_str().unwrap());
    let replay = oauth_fixture_request(
        state.clone(),
        "POST",
        "/oauth/token",
        "application/x-www-form-urlencoded",
        oauth_fixture_form(&token_pairs),
    )
    .await;
    assert_eq!(replay.status(), StatusCode::BAD_REQUEST);
    let opened = extension_rpc_body(
        extension_rpc(
            state.clone(),
            Some(&auth),
            "tools/call",
            serde_json::json!({"name":"open_chats","arguments":{}}),
        )
        .await,
    )
    .await;
    assert_ne!(opened["result"]["isError"], true);
    let tools = extension_rpc_body(
        extension_rpc(
            state.clone(),
            Some(&auth),
            "tools/list",
            serde_json::json!({}),
        )
        .await,
    )
    .await;
    assert_eq!(
        tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["name"] == "send_message"),
        granted_scope.contains("whatsapp.send")
    );
    if !granted_scope.contains("whatsapp.send") {
        let write = extension_rpc_body(extension_rpc(state.clone(), Some(&auth), "tools/call", serde_json::json!({"name":"mark_conversation_read","arguments":{"contact_id":"fixture"}})).await).await;
        assert!(write["error"]["message"]
            .as_str()
            .unwrap()
            .contains("whatsapp.send"));
    }
    let _ = std::fs::remove_dir_all(dir);
}
