use super::*;

/// OAuth Authorization endpoint - shows approval page
pub(in crate::web) async fn oauth_authorize(
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
pub(super) fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

/// OAuth approval form data
#[derive(Debug, Deserialize)]
pub(in crate::web) struct OAuthApprovalForm {
    session_key: String,
    approved: String,
    password: Option<String>,
}

/// Handle OAuth approval form submission
pub(in crate::web) async fn oauth_approve(
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
pub(super) fn build_error_redirect(
    redirect_uri: &str,
    error: OAuthError,
    state: Option<&str>,
) -> String {
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
