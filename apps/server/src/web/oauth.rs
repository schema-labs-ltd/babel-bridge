//! OAuth 2.0 authorization server handlers for MCP clients.

use super::*;

mod authorize;
mod tokens;

pub(super) use authorize::*;
pub(super) use tokens::*;

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
    #[allow(dead_code)] // RFC 7591 metadata, accepted but not stored.
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
