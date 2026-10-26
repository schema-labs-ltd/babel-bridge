//! OAuth 2.0 authorization server handlers for MCP clients.

use super::*;

/// Get base URL from request (for OAuth metadata)
pub(super) fn get_base_url(host: &str, is_https: bool) -> String {
    let scheme = if is_https { "https" } else { "http" };
    format!("{}://{}", scheme, host)
}

pub(super) fn request_host_is_loopback(host: &str) -> bool {
    let normalized = if let Some(rest) = host.strip_prefix('[') {
        rest.split(']').next().unwrap_or(host)
    } else {
        host.split(':').next().unwrap_or(host)
    };

    matches!(normalized, "localhost" | "127.0.0.1" | "::1")
}

fn now_unix_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

pub(super) fn oauth_resource_matches(resource: Option<&str>, host: &str) -> bool {
    // Opaque tokens are stored only in this instance's database and accepted
    // only by its single MCP resource. Keep existing local clients compatible,
    // but reject an explicit resource targeting any other server.
    resource.is_none_or(|resource| {
        resource
            == format!(
                "{}/mcp",
                get_base_url(host, !request_host_is_loopback(host))
            )
    })
}

pub(super) fn normalize_oauth_scope(scope: Option<&str>) -> Result<String, String> {
    let scope = scope.unwrap_or("whatsapp.read").trim();
    if scope.is_empty() {
        return Ok("whatsapp.read".to_string());
    }

    let scopes: Vec<&str> = scope.split_whitespace().collect();
    if scopes == ["mcp"] {
        // Backwards compatibility for clients registered before split scopes existed.
        return Ok("mcp".to_string());
    }
    if scopes
        .iter()
        .any(|entry| !matches!(*entry, "whatsapp.read" | "whatsapp.send"))
    {
        return Err("Only whatsapp.read and whatsapp.send OAuth scopes are supported".to_string());
    }
    if scopes.contains(&"whatsapp.send") && !scopes.contains(&"whatsapp.read") {
        return Err("whatsapp.send requires whatsapp.read so recipients can be resolved and messages can be prepared safely".to_string());
    }
    let mut canonical = Vec::new();
    if scopes.contains(&"whatsapp.read") {
        canonical.push("whatsapp.read");
    }
    if scopes.contains(&"whatsapp.send") {
        canonical.push("whatsapp.send");
    }
    Ok(canonical.join(" "))
}

pub(super) fn resolve_oauth_authorization_scope(
    requested_scope: Option<&str>,
    registered_scope: &str,
) -> Result<String, String> {
    let requested_scope = match requested_scope.map(str::trim) {
        None | Some("") if registered_scope == "mcp" => "mcp".to_string(),
        None | Some("") => "whatsapp.read".to_string(),
        Some(scope) => normalize_oauth_scope(Some(scope))?,
    };
    if registered_scope == "mcp" {
        // Legacy registrations represented full read/send access.
        return Ok(requested_scope);
    }
    if requested_scope == "mcp" {
        return Err("The legacy mcp scope is only available to legacy registrations".to_string());
    }
    let registered: Vec<&str> = registered_scope.split_whitespace().collect();
    if requested_scope
        .split_whitespace()
        .all(|scope| registered.contains(&scope))
    {
        Ok(requested_scope)
    } else {
        Err("Requested scope is not registered for this OAuth client".to_string())
    }
}

pub(super) fn validate_oauth_redirect_uri(redirect_uri: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(redirect_uri)
        .map_err(|_| "redirect_uri must be an absolute URL".to_string())?;

    if parsed.fragment().is_some() {
        return Err("redirect_uri must not contain a fragment".to_string());
    }

    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("redirect_uri must not contain credentials".to_string());
    }

    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("redirect_uri must use http or https".to_string());
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| "redirect_uri must include a host".to_string())?;
    if matches!(host, "localhost" | "127.0.0.1" | "[::1]") {
        return Ok(());
    }

    // Hosted ChatGPT clients cannot receive loopback callbacks. Limit DCR to
    // OpenAI's documented callback paths; authorization still requires the
    // exact registered URI, password approval and S256 PKCE.
    let callback_id = parsed.path().strip_prefix("/connector/oauth/");
    let chatgpt_path = parsed.path() == "/connector_platform_oauth_redirect"
        || callback_id.is_some_and(|id| {
            !id.is_empty()
                && id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        });
    if parsed.scheme() == "https"
        && host == "chatgpt.com"
        && parsed.port().is_none()
        && parsed.query().is_none()
        && chatgpt_path
    {
        return Ok(());
    }
    Err(
        "redirect_uri must be a loopback URL or a supported https://chatgpt.com OAuth callback"
            .to_string(),
    )
}

pub(super) fn validate_oauth_client_request(
    req: &ClientRegistrationRequest,
) -> Result<String, String> {
    if req.redirect_uris.is_empty() {
        return Err("redirect_uris must include at least one supported callback URI".to_string());
    }

    for redirect_uri in &req.redirect_uris {
        validate_oauth_redirect_uri(redirect_uri)?;
    }

    if let Some(grant_types) = &req.grant_types {
        let allowed = ["authorization_code", "refresh_token"];
        if grant_types.is_empty()
            || grant_types
                .iter()
                .any(|grant| !allowed.contains(&grant.as_str()))
            || !grant_types
                .iter()
                .any(|grant| grant == "authorization_code")
        {
            return Err(
                "grant_types must contain authorization_code and may contain refresh_token"
                    .to_string(),
            );
        }
    }

    if let Some(response_types) = &req.response_types {
        if response_types.is_empty() || response_types.iter().any(|response| response != "code") {
            return Err("Only the code response type is supported".to_string());
        }
    }

    if let Some(method) = &req.token_endpoint_auth_method {
        if method != "none" {
            return Err(
                "Only public OAuth clients with token_endpoint_auth_method=none are supported"
                    .to_string(),
            );
        }
    }

    // Registration declares the scopes a client may later request, not a user
    // grant. Hosted clients may omit scope here and select it at authorization.
    // Keep explicit registration limits; authorization defaults to read-only.
    match req.scope.as_deref() {
        None => Ok("whatsapp.read whatsapp.send".to_string()),
        Some(scope) => normalize_oauth_scope(Some(scope)),
    }
}

/// OAuth 2.0 Authorization Server Metadata (RFC 8414)
pub(super) async fn oauth_metadata(Host(host): Host) -> impl IntoResponse {
    // Assume HTTPS in production (Railway sets this)
    let is_https = !host.contains("localhost") && !host.contains("127.0.0.1");
    let base_url = get_base_url(&host, is_https);

    // Extended metadata with Dynamic Client Registration support
    Json(serde_json::json!({
        "issuer": base_url,
        "authorization_endpoint": format!("{}/oauth/authorize", base_url),
        "token_endpoint": format!("{}/oauth/token", base_url),
        "registration_endpoint": format!("{}/oauth/register", base_url),
        "revocation_endpoint": format!("{}/oauth/revoke", base_url),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
        "scopes_supported": ["whatsapp.read", "whatsapp.send"],
        // MCP-specific fields
        "service_documentation": format!("{}/docs", base_url),
    }))
}

/// OAuth 2.0 Protected Resource Metadata (RFC 9728)
/// This tells MCP clients which authorization server to use
pub(super) async fn oauth_protected_resource_metadata(Host(host): Host) -> impl IntoResponse {
    let is_https = !host.contains("localhost") && !host.contains("127.0.0.1");
    let base_url = get_base_url(&host, is_https);

    Json(serde_json::json!({
        "resource": format!("{}/mcp", base_url),
        "authorization_servers": [base_url],
        "scopes_supported": ["whatsapp.read", "whatsapp.send"],
        "bearer_methods_supported": ["header"]
    }))
}

/// Dynamic Client Registration request (RFC 7591)
#[derive(Debug, Deserialize)]
pub(super) struct ClientRegistrationRequest {
    pub(super) redirect_uris: Vec<String>,
    pub(super) client_name: Option<String>,
    pub(super) client_uri: Option<String>,
    pub(super) scope: Option<String>,
    pub(super) grant_types: Option<Vec<String>>,
    pub(super) response_types: Option<Vec<String>>,
    pub(super) token_endpoint_auth_method: Option<String>,
}

/// Dynamic Client Registration endpoint (RFC 7591)
/// Allows public MCP clients to register before starting OAuth flow
pub(super) async fn oauth_register(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ClientRegistrationRequest>,
) -> impl IntoResponse {
    let scope = match validate_oauth_client_request(&req) {
        Ok(scope) => scope,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "invalid_client_metadata",
                    "error_description": error
                })),
            )
                .into_response();
        }
    };

    // Generate a client_id for this registration
    let client_id = format!("client_{}", &generate_token()[..16]);
    let now = now_unix_seconds();

    // For public clients, we don't issue a client_secret
    // The client will use PKCE for security instead
    let client = OAuthClientRegistration {
        client_id: client_id.clone(),
        client_name: req.client_name.clone(),
        redirect_uris: req.redirect_uris.clone(),
        scope: scope.clone(),
        created_at: now,
    };

    if let Err(e) = state.store.oauth_store_client(&client) {
        error!("Failed to store OAuth client registration: {}", e);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": "server_error",
                "error_description": "Failed to store OAuth client registration"
            })),
        )
            .into_response();
    }

    info!("OAuth client registered");

    // Return the registration response
    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "client_id": client_id,
            "client_name": req.client_name,
            "redirect_uris": req.redirect_uris,
            "grant_types": req.grant_types.unwrap_or_else(|| vec!["authorization_code".to_string(), "refresh_token".to_string()]),
            "response_types": req.response_types.unwrap_or_else(|| vec!["code".to_string()]),
            "token_endpoint_auth_method": "none",
            "scope": scope,
        })),
    )
        .into_response()
}

/// OAuth Authorization endpoint - shows approval page
pub(super) async fn oauth_authorize(
    State(state): State<Arc<AppState>>,
    Host(host): Host,
    Query(params): Query<AuthorizeRequest>,
) -> impl IntoResponse {
    if !oauth_resource_matches(params.resource.as_deref(), &host) {
        return (
            StatusCode::BAD_REQUEST,
            Html(
                "<html><body><h1>Error</h1><p>Invalid OAuth resource.</p></body></html>"
                    .to_string(),
            ),
        )
            .into_response();
    }
    if state.password.is_none() && !request_host_is_loopback(&host) {
        return (
            StatusCode::FORBIDDEN,
            Html("<html><body><h1>Error</h1><p>WA_PASSWORD is required before OAuth clients can be approved on a non-loopback host.</p></body></html>".to_string()),
        )
            .into_response();
    }

    // Validate request
    if params.response_type != "code" {
        return (
            StatusCode::BAD_REQUEST,
            Html(format!(
                r#"<html><body><h1>Error</h1><p>Unsupported response_type: {}</p></body></html>"#,
                params.response_type
            )),
        )
            .into_response();
    }

    if params.code_challenge_method != "S256" {
        return (
            StatusCode::BAD_REQUEST,
            Html(r#"<html><body><h1>Error</h1><p>Only S256 code_challenge_method is supported (PKCE required)</p></body></html>"#.to_string()),
        )
            .into_response();
    }

    let client = match state.store.oauth_get_client(&params.client_id) {
        Ok(Some(client)) => client,
        Ok(None) => {
            return (
                StatusCode::BAD_REQUEST,
                Html(
                    "<html><body><h1>Error</h1><p>Unknown OAuth client. Register the client before authorizing.</p></body></html>"
                        .to_string(),
                ),
            )
                .into_response();
        }
        Err(e) => {
            error!("Failed to look up OAuth client: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Html(
                    "<html><body><h1>Error</h1><p>Internal server error</p></body></html>"
                        .to_string(),
                ),
            )
                .into_response();
        }
    };

    if !client
        .redirect_uris
        .iter()
        .any(|redirect_uri| redirect_uri == &params.redirect_uri)
    {
        return (
            StatusCode::BAD_REQUEST,
            Html("<html><body><h1>Error</h1><p>redirect_uri is not registered for this OAuth client.</p></body></html>".to_string()),
        )
            .into_response();
    }

    let requested_scope =
        match resolve_oauth_authorization_scope(params.scope.as_deref(), &client.scope) {
            Ok(scope) => scope,
            Err(error) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Html(format!(
                        "<html><body><h1>Error</h1><p>{}</p></body></html>",
                        html_escape(&error)
                    )),
                )
                    .into_response();
            }
        };

    // Generate a session key for this authorization request
    let session_key = generate_token();
    let now = now_unix_seconds();

    let pending = PendingAuthorization {
        session_key: session_key.clone(),
        client_id: params.client_id.clone(),
        redirect_uri: params.redirect_uri.clone(),
        code_challenge: params.code_challenge.clone(),
        code_challenge_method: params.code_challenge_method.clone(),
        scope: requested_scope.clone(),
        state: params.state.clone(),
        created_at: now,
        expires_at: now + 600, // 10 minutes
    };

    // Store pending authorization
    if let Err(e) = state.store.oauth_store_pending_auth(&pending) {
        error!("Failed to store pending auth: {}", e);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Html(
                "<html><body><h1>Error</h1><p>Internal server error</p></body></html>".to_string(),
            ),
        )
            .into_response();
    }

    // Check if password auth is required
    let requires_password = state.password.is_some();

    // Show approval page
    let is_https = !host.contains("localhost") && !host.contains("127.0.0.1");
    let base_url = get_base_url(&host, is_https);

    let html = format!(
        r#"<!DOCTYPE html>
<html>
<head>
    <title>Authorize MCP Client</title>
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <style>
        body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; 
               max-width: 500px; margin: 50px auto; padding: 20px; background: #f5f5f5; }}
        .card {{ background: white; border-radius: 12px; padding: 30px; box-shadow: 0 2px 10px rgba(0,0,0,0.1); }}
        h1 {{ color: #333; margin-top: 0; }}
        .client-info {{ background: #f0f0f0; padding: 15px; border-radius: 8px; margin: 20px 0; }}
        .scope {{ color: #666; font-size: 0.9em; }}
        .warning {{ color: #e67e22; font-size: 0.9em; margin: 15px 0; }}
        .buttons {{ display: flex; gap: 10px; margin-top: 20px; }}
        button {{ flex: 1; padding: 12px 24px; border: none; border-radius: 8px; font-size: 16px; cursor: pointer; }}
        .approve {{ background: #25D366; color: white; }}
        .approve:hover {{ background: #1da851; }}
        .deny {{ background: #e74c3c; color: white; }}
        .deny:hover {{ background: #c0392b; }}
        input {{ width: 100%; padding: 12px; margin: 10px 0; border: 1px solid #ddd; border-radius: 8px; box-sizing: border-box; }}
        label {{ display: block; margin-top: 15px; color: #666; }}
    </style>
</head>
<body>
    <div class="card">
        <h1>🔐 Authorize MCP Client</h1>
        <p>An application is requesting access to your WhatsApp Translator:</p>
        
        <div class="client-info">
            <strong>Client:</strong> {client_name}<br>
            <strong>Client ID:</strong> {client_id}<br>
            <strong>Redirect URI:</strong> {redirect_uri}
        </div>
        
        <div class="scope">
            <strong>Requested permissions:</strong> {scope}<br>
            {scope_description}
        </div>
        
        <p class="warning">⚠️ Only authorize applications you trust!</p>
        
        <form method="POST" action="{base_url}/oauth/approve">
            <input type="hidden" name="session_key" value="{session_key}">
            {password_field}
            <div class="buttons">
                <button type="submit" name="approved" value="true" class="approve">✓ Authorize</button>
                <button type="submit" name="approved" value="false" class="deny">✗ Deny</button>
            </div>
        </form>
    </div>
</body>
</html>"#,
        client_name = html_escape(
            client
                .client_name
                .as_deref()
                .unwrap_or("Unnamed MCP client")
        ),
        client_id = html_escape(&params.client_id),
        redirect_uri = html_escape(&params.redirect_uri),
        scope = html_escape(&requested_scope),
        scope_description = if requested_scope == "whatsapp.read" {
            "This allows the application to read your contacts and stored messages, but not send or modify WhatsApp state."
        } else if requested_scope == "whatsapp.send" {
            "This allows the application to send messages, replies, reactions, and read receipts on your behalf."
        } else {
            "This allows the application to read your contacts and messages and to send messages, replies, reactions, and read receipts on your behalf."
        },
        base_url = base_url,
        session_key = session_key,
        password_field = if requires_password {
            r#"<label for="password">Enter your password to authorize:</label>
            <input type="password" name="password" id="password" placeholder="Password" required>"#
        } else {
            ""
        }
    );

    Html(html).into_response()
}

/// Simple HTML escaping
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

/// OAuth approval form data
#[derive(Debug, Deserialize)]
pub(super) struct OAuthApprovalForm {
    session_key: String,
    approved: String,
    password: Option<String>,
}

/// Handle OAuth approval form submission
pub(super) async fn oauth_approve(
    State(state): State<Arc<AppState>>,
    Form(form): Form<OAuthApprovalForm>,
) -> impl IntoResponse {
    // Get the pending authorization
    let pending = match state.store.oauth_take_pending_auth(&form.session_key) {
        Ok(Some(p)) => p,
        Ok(None) => {
            return (
                StatusCode::BAD_REQUEST,
                Html("<html><body><h1>Error</h1><p>Invalid or expired authorization request</p></body></html>".to_string()),
            )
                .into_response();
        }
        Err(e) => {
            error!("Failed to get pending auth: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Html(
                    "<html><body><h1>Error</h1><p>Internal server error</p></body></html>"
                        .to_string(),
                ),
            )
                .into_response();
        }
    };

    // Check if user denied
    if form.approved != "true" {
        let redirect_url = build_error_redirect(
            &pending.redirect_uri,
            OAuthError::AccessDenied,
            pending.state.as_deref(),
        );
        return Redirect::to(&redirect_url).into_response();
    }

    // Verify password if required
    if let Some(expected_password) = &state.password {
        if !state.login_budget.lock().await.take() {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                [("Retry-After", "60")],
                "Too many sign-in attempts. Try again in one minute.",
            )
                .into_response();
        }
        match &form.password {
            Some(password) if password == expected_password => {}
            _ => {
                let redirect_url = build_error_redirect(
                    &pending.redirect_uri,
                    OAuthError::AccessDenied,
                    pending.state.as_deref(),
                );
                return Redirect::to(&redirect_url).into_response();
            }
        }
    }

    // Generate authorization code
    let code = generate_token();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    let auth_code = AuthorizationCode {
        code: code.clone(),
        client_id: pending.client_id,
        redirect_uri: pending.redirect_uri.clone(),
        code_challenge: pending.code_challenge,
        code_challenge_method: pending.code_challenge_method,
        scope: pending.scope,
        created_at: now,
        expires_at: now + 300, // 5 minutes
        used: false,
    };

    // Store the authorization code
    if let Err(e) = state.store.oauth_store_authorization_code(&auth_code) {
        error!("Failed to store authorization code: {}", e);
        let redirect_url = build_error_redirect(
            &pending.redirect_uri,
            OAuthError::ServerError,
            pending.state.as_deref(),
        );
        return Redirect::to(&redirect_url).into_response();
    }

    info!("OAuth authorization granted");

    // Redirect back to client with authorization code
    let mut redirect_url = format!("{}?code={}", pending.redirect_uri, code);
    if let Some(state_param) = pending.state {
        redirect_url.push_str(&format!("&state={}", urlencoding::encode(&state_param)));
    }

    Redirect::to(&redirect_url).into_response()
}

/// Build error redirect URL
fn build_error_redirect(redirect_uri: &str, error: OAuthError, state: Option<&str>) -> String {
    let mut url = format!(
        "{}?error={}&error_description={}",
        redirect_uri,
        error.as_str(),
        urlencoding::encode(error.description())
    );
    if let Some(s) = state {
        url.push_str(&format!("&state={}", urlencoding::encode(s)));
    }
    url
}

/// OAuth Token endpoint - exchange code for tokens or refresh tokens
pub(super) async fn oauth_token(
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

async fn handle_authorization_code_grant(
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

pub(super) async fn handle_refresh_token_grant(
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
pub(super) async fn oauth_revoke(
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

pub(super) async fn list_oauth_clients(State(state): State<Arc<AppState>>) -> impl IntoResponse {
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

pub(super) async fn revoke_oauth_client(
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

pub(super) async fn revoke_all_oauth_clients(
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
