//! Web sign-in, session tokens, push device registration and the auth middleware.

use super::*;

/// Check if authentication is required
pub(super) async fn auth_check(State(state): State<Arc<AppState>>) -> Json<AuthCheckResponse> {
    Json(AuthCheckResponse {
        required: state.password.is_some(),
    })
}

/// Handle login attempt
pub(super) async fn auth_login(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AuthRequest>,
) -> impl IntoResponse {
    // If no password is set, auth is not required
    let Some(expected_password) = &state.password else {
        return Json(AuthResponse {
            success: true,
            token: None,
            error: None,
        })
        .into_response();
    };

    // Check password
    if !state.login_budget.lock().await.take() {
        return (StatusCode::TOO_MANY_REQUESTS, [("Retry-After", "60")], Json(serde_json::json!({"success":false,"error":"Too many sign-in attempts. Try again in one minute."}))).into_response();
    }
    if req.password == *expected_password {
        let token = generate_token();
        let expires_at = chrono::Utc::now().timestamp() + WEB_AUTH_TOKEN_TTL_SECONDS;

        let mut tokens = state.auth_tokens.write().await;
        tokens.retain(|_, expiry| *expiry > chrono::Utc::now().timestamp());
        if tokens.len() >= 128 {
            if let Some(oldest) = tokens
                .iter()
                .min_by_key(|(_, expiry)| *expiry)
                .map(|(key, _)| key.clone())
            {
                tokens.remove(&oldest);
            }
        }
        tokens.insert(token.clone(), expires_at);

        info!("User authenticated successfully");
        Json(AuthResponse {
            success: true,
            token: Some(token),
            error: None,
        })
        .into_response()
    } else {
        warn!("Failed authentication attempt");
        Json(AuthResponse {
            success: false,
            token: None,
            error: Some("Invalid password".to_string()),
        })
        .into_response()
    }
}

pub(super) async fn register_push_device(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RegisterPushDeviceRequest>,
) -> Response {
    let installation_id = req.installation_id.trim();
    let token = req.token.trim().to_ascii_lowercase();
    let environment = req.environment.trim().to_ascii_lowercase();

    if installation_id.is_empty() || installation_id.len() > 128 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid installation ID"})),
        )
            .into_response();
    }
    if token.len() < 32 || token.len() > 512 || !token.chars().all(|c| c.is_ascii_hexdigit()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid APNs device token"})),
        )
            .into_response();
    }
    if environment != "sandbox" && environment != "production" {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid APNs environment"})),
        )
            .into_response();
    }

    match state
        .store
        .register_push_device(installation_id, &token, &environment)
    {
        Ok(()) => Json(serde_json::json!({"success": true})).into_response(),
        Err(error) => {
            error!("Failed to register APNs device: {}", error);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Failed to register device"})),
            )
                .into_response()
        }
    }
}

pub(super) async fn unregister_push_device(
    State(state): State<Arc<AppState>>,
    Path(installation_id): Path<String>,
) -> Response {
    match state.store.delete_push_installation(&installation_id) {
        Ok(_) => Json(serde_json::json!({"success": true})).into_response(),
        Err(error) => {
            error!("Failed to unregister APNs device: {}", error);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Failed to unregister device"})),
            )
                .into_response()
        }
    }
}

pub(super) async fn send_test_push_notification(State(state): State<Arc<AppState>>) -> Response {
    let Some(client) = &state.push_notifications else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "Push notifications are not configured"})),
        )
            .into_response();
    };
    let devices = match state.store.list_push_devices() {
        Ok(devices) => devices,
        Err(error) => {
            error!("Failed to load APNs devices: {}", error);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Failed to load registered devices"})),
            )
                .into_response();
        }
    };
    if devices.is_empty() {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": "No iPhones are registered for notifications"})),
        )
            .into_response();
    }

    let notification = PushNotification::test();
    let mut delivered = 0;
    let mut removed = 0;
    let mut failed = 0;
    for device in devices {
        match client.send(&device, &notification).await {
            Ok(ApnsSendOutcome::Delivered) => delivered += 1,
            Ok(ApnsSendOutcome::RemoveDevice) => {
                removed += 1;
                if let Err(error) = state.store.delete_push_device(&device.token) {
                    warn!("Failed to remove stale APNs token: {}", error);
                }
            }
            Err(error) => {
                failed += 1;
                warn!(
                    "Failed to send APNs test notification to installation {}: {}",
                    device.installation_id, error
                );
            }
        }
    }

    let status = if delivered > 0 {
        StatusCode::OK
    } else {
        StatusCode::BAD_GATEWAY
    };
    (
        status,
        Json(serde_json::json!({
            "success": delivered > 0,
            "delivered": delivered,
            "removed": removed,
            "failed": failed
        })),
    )
        .into_response()
}

pub(super) async fn require_web_auth(state: Arc<AppState>, req: Request, next: Next) -> Response {
    let auth_header = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .map(str::to_string);
    let websocket_token = if req.uri().path() == "/ws" {
        token_query_param(req.uri().query())
    } else {
        None
    };

    if verify_auth_values(&state, auth_header.as_deref(), websocket_token.as_deref()).await {
        crate::outbox::dispatch(state, req, next).await
    } else {
        StatusCode::UNAUTHORIZED.into_response()
    }
}

async fn verify_auth_values(
    state: &Arc<AppState>,
    auth_header: Option<&str>,
    websocket_token: Option<&str>,
) -> bool {
    if verify_auth_header(state, auth_header).await {
        return true;
    }

    if let Some(token) = websocket_token {
        return verify_auth_token(state, token).await;
    }

    false
}

fn token_query_param(query: Option<&str>) -> Option<String> {
    let query = query?;
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        if key == "token" {
            urlencoding::decode(value)
                .ok()
                .map(|token| token.into_owned())
        } else {
            None
        }
    })
}

async fn verify_auth_header(state: &Arc<AppState>, auth_header: Option<&str>) -> bool {
    if state.password.is_none() {
        return true;
    }

    if let Some(header) = auth_header {
        if let Some(token) = header.strip_prefix("Bearer ") {
            return verify_auth_token(state, token).await;
        }
    }

    false
}

async fn verify_auth_token(state: &Arc<AppState>, token: &str) -> bool {
    if state.password.is_none() {
        return true;
    }

    let now = chrono::Utc::now().timestamp();
    let mut tokens = state.auth_tokens.write().await;
    tokens.retain(|_, expires_at| *expires_at > now);

    tokens
        .get(token)
        .map(|expires_at| *expires_at > now)
        .unwrap_or(false)
}

/// Logout - clear all data and session
pub(super) async fn logout(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    info!("Logout requested - clearing all data");
    state.voice_epoch.fetch_add(1, Ordering::SeqCst);

    // 1. Clear the message store (contacts, messages, usage)
    if let Err(e) = state.store.clear_all() {
        error!("Failed to clear message store: {}", e);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "success": false,
                "error": format!("Failed to clear data: {}", e)
            })),
        )
            .into_response();
    }

    if let Err(e) = state.store.oauth_clear_all() {
        error!("Failed to clear OAuth tokens: {}", e);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "success": false,
                "error": format!("Failed to clear OAuth tokens: {}", e)
            })),
        )
            .into_response();
    }

    // 2. Send logout command to bridge (this will notify WhatsApp and stop the bridge).
    // The bridge restart loop clears session files after the process exits and before respawn.
    state.request_session_reset_before_bridge_restart();
    let mut sent_logout_to_bridge = false;
    if let Some(tx) = state.command_tx.read().await.as_ref() {
        if let Err(e) = tx.send(BridgeCommand::Logout).await {
            warn!("Failed to send logout command to bridge: {}", e);
        } else {
            sent_logout_to_bridge = true;
        }
    }

    if !sent_logout_to_bridge {
        if let Err(e) = state.clear_session_files() {
            warn!("Failed to remove session files: {}", e);
        }
        state.take_session_reset_request();
    }

    // 4. Clear auth tokens
    state.auth_tokens.write().await.clear();
    state.mcp_prepared_messages.write().await.clear();

    // 5. Reset connection state
    state.require_whatsapp_link().await;

    // 6. Clear avatar cache
    state.avatar_cache.write().await.clear();

    info!("Logout complete - all data cleared");

    Json(serde_json::json!({
        "success": true,
        "message": "Logged out successfully. Please refresh the page."
    }))
    .into_response()
}

fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let header = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let token = header.strip_prefix("Bearer ")?;
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

async fn live_web_auth_token(state: &AppState, token: &str) -> bool {
    let now = chrono::Utc::now().timestamp();
    let tokens = state.auth_tokens.read().await;
    tokens
        .get(token)
        .is_some_and(|expires_at| *expires_at > now)
}

fn session_revoke_failure() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({
            "success": false,
            "error": "Failed to revoke session"
        })),
    )
        .into_response()
}

/// Revoke only the bearer token that called this route.
///
/// Web and iOS sessions are removed from the in-memory token map. An OAuth
/// access token also drops the refresh token stored on that same grant, and
/// access tokens minted from that refresh token. An access token with no grant
/// link drops only itself. Other devices, stored messages, OAuth client
/// registrations, and the WhatsApp session stay in place. `/api/logout` is
/// unchanged.
///
/// When no password is configured, the route is reachable without a bearer
/// token, matching the rest of the API. That call revokes nothing. A presented
/// bearer still revokes only that token.
pub(super) async fn delete_auth_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let Some(token) = bearer_token(&headers) else {
        return if state.password.is_none() {
            Json(serde_json::json!({"success": true})).into_response()
        } else {
            StatusCode::UNAUTHORIZED.into_response()
        };
    };

    let web_session = live_web_auth_token(&state, &token).await;
    let oauth_session = match state.store.oauth_validate_access_token(&token) {
        Ok(access_token) => access_token.is_some(),
        Err(error) => {
            error!("Failed to validate calling session: {error}");
            return session_revoke_failure();
        }
    };

    if !web_session && !oauth_session {
        return if state.password.is_none() {
            Json(serde_json::json!({"success": true})).into_response()
        } else {
            StatusCode::UNAUTHORIZED.into_response()
        };
    }

    if web_session {
        state.auth_tokens.write().await.remove(&token);
    }
    if oauth_session {
        if let Err(error) = state.store.oauth_revoke_access_session(&token) {
            error!("Failed to revoke calling session: {error}");
            return session_revoke_failure();
        }
    }

    info!("Calling device session revoked");
    Json(serde_json::json!({"success": true})).into_response()
}
