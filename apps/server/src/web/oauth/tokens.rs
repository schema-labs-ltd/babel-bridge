use super::*;

/// OAuth Token endpoint - exchange code for tokens or refresh tokens
pub(in crate::web) async fn oauth_token(
    State(state): State<Arc<AppState>>,
    Host(host): Host,
    Form(req): Form<TokenRequest>,
) -> impl IntoResponse {
    if !oauth_resource_matches(req.resource.as_deref(), &host) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error":"invalid_target","error_description":"Invalid OAuth resource"})),
        )
            .into_response();
    }
    match req.grant_type.as_str() {
        "authorization_code" => handle_authorization_code_grant(state, req).await,
        "refresh_token" => handle_refresh_token_grant(state, req).await,
        _ => {
            let error = OAuthErrorResponse::from(OAuthError::UnsupportedGrantType);
            (StatusCode::BAD_REQUEST, Json(error)).into_response()
        }
    }
}

pub(super) async fn handle_authorization_code_grant(
    state: Arc<AppState>,
    req: TokenRequest,
) -> axum::response::Response {
    // Validate required parameters
    let code = match &req.code {
        Some(c) => c,
        None => {
            let error = OAuthErrorResponse::from(OAuthError::InvalidRequest);
            return (StatusCode::BAD_REQUEST, Json(error)).into_response();
        }
    };

    let code_verifier = match &req.code_verifier {
        Some(v) => v,
        None => {
            let error = OAuthErrorResponse::from(OAuthError::InvalidRequest);
            return (StatusCode::BAD_REQUEST, Json(error)).into_response();
        }
    };

    let redirect_uri = match &req.redirect_uri {
        Some(r) => r,
        None => {
            let error = OAuthErrorResponse::from(OAuthError::InvalidRequest);
            return (StatusCode::BAD_REQUEST, Json(error)).into_response();
        }
    };

    // Get and validate the authorization code
    let auth_code = match state.store.oauth_use_authorization_code(code) {
        Ok(Some(c)) => c,
        Ok(None) => {
            let error = OAuthErrorResponse::from(OAuthError::InvalidGrant);
            return (StatusCode::BAD_REQUEST, Json(error)).into_response();
        }
        Err(e) => {
            error!("Failed to get authorization code: {}", e);
            let error = OAuthErrorResponse::from(OAuthError::ServerError);
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response();
        }
    };

    // Verify redirect_uri matches
    if auth_code.redirect_uri != *redirect_uri {
        let error = OAuthErrorResponse::from(OAuthError::InvalidGrant);
        return (StatusCode::BAD_REQUEST, Json(error)).into_response();
    }

    if let Some(client_id) = &req.client_id {
        if client_id != &auth_code.client_id {
            let error = OAuthErrorResponse::from(OAuthError::InvalidClient);
            return (StatusCode::BAD_REQUEST, Json(error)).into_response();
        }
    }

    let client = match state.store.oauth_get_client(&auth_code.client_id) {
        Ok(Some(client)) => client,
        Ok(None) => {
            let error = OAuthErrorResponse::from(OAuthError::InvalidClient);
            return (StatusCode::BAD_REQUEST, Json(error)).into_response();
        }
        Err(e) => {
            error!("Failed to look up OAuth client: {}", e);
            let error = OAuthErrorResponse::from(OAuthError::ServerError);
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response();
        }
    };

    if !client
        .redirect_uris
        .iter()
        .any(|registered| registered == redirect_uri)
    {
        let error = OAuthErrorResponse::from(OAuthError::InvalidGrant);
        return (StatusCode::BAD_REQUEST, Json(error)).into_response();
    }

    // Verify PKCE
    if !auth_code.verify_pkce(code_verifier) {
        let error = OAuthErrorResponse::from(OAuthError::InvalidGrant);
        return (StatusCode::BAD_REQUEST, Json(error)).into_response();
    }

    // Generate tokens
    let now = now_unix_seconds();

    let access_token_str = generate_token();
    let refresh_token_str = generate_token();

    let access_token = AccessToken {
        token: access_token_str.clone(),
        client_id: auth_code.client_id.clone(),
        scope: auth_code.scope.clone(),
        created_at: now,
        expires_at: now + 3600, // 1 hour
        refresh_token: Some(refresh_token_str.clone()),
    };

    let refresh_token = RefreshToken {
        token: refresh_token_str.clone(),
        client_id: auth_code.client_id.clone(),
        scope: auth_code.scope.clone(),
        created_at: now,
        expires_at: now + 30 * 24 * 3600, // 30 days
    };

    // Store tokens
    if let Err(e) = state.store.oauth_store_access_token(&access_token) {
        error!("Failed to store access token: {}", e);
        let error = OAuthErrorResponse::from(OAuthError::ServerError);
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response();
    }

    if let Err(e) = state.store.oauth_store_refresh_token(&refresh_token) {
        error!("Failed to store refresh token: {}", e);
        let error = OAuthErrorResponse::from(OAuthError::ServerError);
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response();
    }

    info!("OAuth tokens issued");

    let response = TokenResponse {
        access_token: access_token_str,
        token_type: "Bearer".to_string(),
        expires_in: 3600,
        refresh_token: refresh_token_str,
        scope: auth_code.scope,
    };

    (
        StatusCode::OK,
        [(header::CACHE_CONTROL, "no-store")],
        Json(response),
    )
        .into_response()
}

pub(in crate::web) async fn handle_refresh_token_grant(
    state: Arc<AppState>,
    req: TokenRequest,
) -> axum::response::Response {
    let refresh_token_str = match &req.refresh_token {
        Some(r) => r,
        None => {
            let error = OAuthErrorResponse::from(OAuthError::InvalidRequest);
            return (StatusCode::BAD_REQUEST, Json(error)).into_response();
        }
    };

    // Validate refresh token
    let refresh_token = match state.store.oauth_get_refresh_token(refresh_token_str) {
        Ok(Some(t)) => t,
        Ok(None) => {
            let error = OAuthErrorResponse::from(OAuthError::InvalidGrant);
            return (StatusCode::BAD_REQUEST, Json(error)).into_response();
        }
        Err(e) => {
            error!("Failed to get refresh token: {}", e);
            let error = OAuthErrorResponse::from(OAuthError::ServerError);
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response();
        }
    };

    if let Some(client_id) = &req.client_id {
        if client_id != &refresh_token.client_id {
            let error = OAuthErrorResponse::from(OAuthError::InvalidClient);
            return (StatusCode::BAD_REQUEST, Json(error)).into_response();
        }
    }

    // Generate new access token
    let now = now_unix_seconds();

    let access_token_str = generate_token();

    let access_token = AccessToken {
        token: access_token_str.clone(),
        client_id: refresh_token.client_id.clone(),
        scope: refresh_token.scope.clone(),
        created_at: now,
        expires_at: now + 3600, // 1 hour
        refresh_token: Some(refresh_token_str.clone()),
    };

    // Store new access token
    if let Err(e) = state.store.oauth_store_access_token(&access_token) {
        error!("Failed to store access token: {}", e);
        let error = OAuthErrorResponse::from(OAuthError::ServerError);
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response();
    }

    info!("OAuth access token refreshed");

    let response = TokenResponse {
        access_token: access_token_str,
        token_type: "Bearer".to_string(),
        expires_in: 3600,
        refresh_token: refresh_token_str.clone(),
        scope: refresh_token.scope,
    };

    (
        StatusCode::OK,
        [(header::CACHE_CONTROL, "no-store")],
        Json(response),
    )
        .into_response()
}

/// OAuth Token revocation endpoint
pub(in crate::web) async fn oauth_revoke(
    State(state): State<Arc<AppState>>,
    Form(req): Form<RevokeRequest>,
) -> impl IntoResponse {
    // Revoke the token (we don't care if it exists or not per RFC 7009)
    if let Err(e) = state.store.oauth_revoke_token(&req.token) {
        error!("Failed to revoke token: {}", e);
        // Still return 200 per RFC 7009
    }

    StatusCode::OK
}

pub(in crate::web) async fn list_oauth_clients(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    match state.store.oauth_list_clients() {
        Ok(clients) => Json(serde_json::json!({ "clients": clients })).into_response(),
        Err(e) => {
            error!("Failed to list OAuth clients: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Failed to list OAuth clients: {}", e)
                })),
            )
                .into_response()
        }
    }
}

pub(in crate::web) async fn revoke_oauth_client(
    State(state): State<Arc<AppState>>,
    Path(client_id): Path<String>,
) -> impl IntoResponse {
    match state.store.oauth_revoke_client(&client_id) {
        Ok(revoked) => Json(serde_json::json!({
            "success": true,
            "clientId": client_id,
            "revoked": revoked
        }))
        .into_response(),
        Err(e) => {
            error!("Failed to revoke OAuth client: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Failed to revoke OAuth client: {}", e)
                })),
            )
                .into_response()
        }
    }
}

pub(in crate::web) async fn revoke_all_oauth_clients(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    match state.store.oauth_clear_all() {
        Ok(()) => Json(serde_json::json!({ "success": true })).into_response(),
        Err(e) => {
            error!("Failed to revoke all OAuth clients: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Failed to revoke OAuth clients: {}", e)
                })),
            )
                .into_response()
        }
    }
}
