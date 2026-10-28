use super::*;

fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("wa-chatgpt-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn redirect_uri_must_be_ipv4_loopback_callback() {
    assert!(validate_redirect_uri("http://127.0.0.1:1455/auth/callback").is_ok());
    assert!(validate_redirect_uri("http://127.0.0.1:53111/auth/callback").is_ok());
    for bad in [
        "http://localhost:1455/auth/callback",
        "https://127.0.0.1:1455/auth/callback",
        "http://127.0.0.1/auth/callback",
        "http://127.0.0.1:1455/other",
        "http://127.0.0.1:1455/auth/callback?x=1",
        "http://example.com:1455/auth/callback",
    ] {
        assert!(validate_redirect_uri(bad).is_err(), "{bad}");
    }
}

#[test]
fn host_id_is_created_once_and_kept() {
    let dir = temp_dir();
    let first = ChatGptAuth::load(&dir).unwrap().host_id;
    assert!(first.starts_with("urn:uuid:"));
    assert_eq!(ChatGptAuth::load(&dir).unwrap().host_id, first);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.join(HOST_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn authorize_url_registers_dynamically_with_pkce_and_plan_scopes() {
    let dir = temp_dir();
    let auth = ChatGptAuth::load(&dir).unwrap();
    let url = Url::parse(&auth.start_sign_in(DEFAULT_REDIRECT_URI, false).unwrap()).unwrap();
    let params: HashMap<String, String> = url.query_pairs().into_owned().collect();
    assert_eq!(url.as_str().split('?').next(), Some(AUTHORIZE_URL));
    assert_eq!(params["client_id"], DYNAMIC_CLIENT_ID);
    assert_eq!(params["agent_name_hint"], AGENT_NAME);
    assert_eq!(params["ext_agent_host_id"], auth.host_id);
    assert_eq!(params["resource"], RESOURCE);
    assert_eq!(params["code_challenge_method"], "S256");
    assert!(params["scope"].contains(PLAN_SCOPE));
    assert!(params["scope"].contains("offline_access"));
    let pending = auth.pending.read().unwrap();
    let attempt = &pending[&params["state"]];
    assert_eq!(params["code_challenge"], pkce_challenge(&attempt.verifier));
    assert_eq!(params["nonce"], attempt.nonce);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn saved_client_is_reused_without_a_name_hint() {
    let dir = temp_dir();
    let auth = ChatGptAuth::load(&dir).unwrap();
    auth.save(&Registration {
        client_id: "issued-client".into(),
        ..Default::default()
    })
    .unwrap();
    let url = Url::parse(&auth.start_sign_in(DEFAULT_REDIRECT_URI, false).unwrap()).unwrap();
    let params: HashMap<String, String> = url.query_pairs().into_owned().collect();
    assert_eq!(params["client_id"], "issued-client");
    assert!(!params.contains_key("agent_name_hint"));
    let url = Url::parse(&auth.start_sign_in(DEFAULT_REDIRECT_URI, true).unwrap()).unwrap();
    assert!(url
        .query_pairs()
        .any(|(k, v)| k == "client_id" && v == DYNAMIC_CLIENT_ID));
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn callback_rejects_unknown_state_and_denied_consent() {
    let dir = temp_dir();
    let auth = ChatGptAuth::load(&dir).unwrap();
    assert!(auth
        .complete_sign_in("http://127.0.0.1:1455/auth/callback?code=x&state=nope")
        .await
        .is_err());
    let url = Url::parse(&auth.start_sign_in(DEFAULT_REDIRECT_URI, false).unwrap()).unwrap();
    let state = url
        .query_pairs()
        .find(|(k, _)| k == "state")
        .unwrap()
        .1
        .into_owned();
    let error = auth
        .complete_sign_in(&format!(
            "http://127.0.0.1:1455/auth/callback?error=access_denied&state={state}"
        ))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("cancelled"));
    // A state is single use.
    assert!(auth
        .complete_sign_in(&format!(
            "http://127.0.0.1:1455/auth/callback?code=x&state={state}"
        ))
        .await
        .is_err());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn signed_in_requires_the_plan_scope() {
    let mut registration = Registration {
        access_token: Some("token".into()),
        scopes: vec!["openid".into()],
        ..Default::default()
    };
    assert!(!registration.signed_in());
    registration.scopes.push(PLAN_SCOPE.into());
    assert!(registration.signed_in());
    registration.clear_tokens();
    assert!(!registration.signed_in());
}

#[test]
fn models_keep_listed_entries_in_server_order() {
    let body = serde_json::json!({"models": [
        {"slug": "b", "display_name": "B", "visibility": "list"},
        {"slug": "hidden", "display_name": "H", "visibility": "hide"},
        {"slug": "a", "visibility": "list"},
    ]});
    let models = parse_models(&body);
    assert_eq!(
        models,
        vec![
            ChatGptModel {
                slug: "b".into(),
                display_name: "B".into()
            },
            ChatGptModel {
                slug: "a".into(),
                display_name: "a".into()
            },
        ]
    );
}

#[tokio::test]
async fn fresh_tokens_are_returned_without_refreshing() {
    let dir = temp_dir();
    let auth = ChatGptAuth::load(&dir).unwrap();
    assert_eq!(auth.access_token().await.unwrap(), None);
    auth.save(&Registration {
        client_id: "c".into(),
        access_token: Some("live".into()),
        scopes: vec![PLAN_SCOPE.into()],
        expires_at: now() + 3600,
        ..Default::default()
    })
    .unwrap();
    assert_eq!(auth.access_token().await.unwrap().as_deref(), Some("live"));
    auth.mark_access_token_rejected("live");
    assert!(auth.fresh_token().is_none());
    std::fs::remove_dir_all(dir).ok();
}
