//! MCP (Model Context Protocol) HTTP handler.
//!
//! Uses Streamable HTTP transport (POST for requests, SSE for responses).

use super::*;

use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};

fn create_mcp_service(
    state: Arc<AppState>,
    permissions: McpPermissions,
) -> StreamableHttpService<WhatsAppMcpServer, LocalSessionManager> {
    let session_manager = Arc::new(LocalSessionManager::default());
    let config = StreamableHttpServerConfig {
        stateful_mode: false, // Stateless mode - simpler, no session management needed
        ..Default::default()
    };

    StreamableHttpService::new(
        move || {
            // Create a new MCP server instance for each request
            Ok(WhatsAppMcpServer::new(state.clone(), permissions))
        },
        session_manager,
        config,
    )
}

pub(super) async fn mcp_handler(
    State(state): State<Arc<AppState>>,
    Host(host): Host,
    request: axum::http::Request<axum::body::Body>,
) -> impl IntoResponse {
    // Check OAuth Bearer token authentication
    let auth_header = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok());

    let bearer = auth_header
        .and_then(|header| header.strip_prefix("Bearer "))
        .map(str::to_owned);
    let access_token = if let Some(header) = auth_header {
        if let Some(token) = header.strip_prefix("Bearer ") {
            // Validate the OAuth access token
            match state.store.oauth_validate_access_token(token) {
                Ok(Some(access_token)) => {
                    info!("MCP authenticated via OAuth token");
                    Some(access_token)
                }
                Ok(None) => {
                    info!("MCP request with invalid/expired OAuth token");
                    None
                }
                Err(e) => {
                    error!("Failed to validate OAuth token: {}", e);
                    None
                }
            }
        } else {
            None
        }
    } else {
        None
    };

    let Some(access_token) = access_token else {
        // Build the resource_metadata URL for the WWW-Authenticate header
        let is_https = !host.contains("localhost") && !host.contains("127.0.0.1");
        let base_url = get_base_url(&host, is_https);
        let resource_metadata_url = format!("{}/.well-known/oauth-protected-resource", base_url);

        // Return 401 with WWW-Authenticate header per RFC 6750 and RFC 9728
        // The resource_metadata parameter tells MCP clients where to find OAuth config
        let www_authenticate = format!(
            "Bearer resource_metadata=\"{}\", scope=\"whatsapp.read\"",
            resource_metadata_url
        );

        return (
            StatusCode::UNAUTHORIZED,
            [
                (header::WWW_AUTHENTICATE.as_str(), www_authenticate.as_str()),
                (header::CONTENT_TYPE.as_str(), "application/json"),
            ],
            Json(serde_json::json!({
                "error": "unauthorized",
                "error_description": "OAuth Bearer token required. Complete the OAuth flow to get a token."
            })),
        )
            .into_response();
    };

    let permissions = McpPermissions::from_scope(&access_token.scope);
    // Read the body once to route Events extension requests, which rmcp does
    // not know, and to recognise `initialize`.
    let (parts, body) = request.into_parts();
    let bytes = match axum::body::to_bytes(body, MAX_MCP_REQUEST_BYTES).await {
        Ok(bytes) => bytes,
        Err(error) => {
            let too_large = std::iter::successors(
                Some(&error as &(dyn std::error::Error + 'static)),
                |error| error.source(),
            )
            .any(|error| error.to_string().contains("length limit exceeded"));
            return if too_large {
                (StatusCode::PAYLOAD_TOO_LARGE, "MCP request too large").into_response()
            } else {
                (StatusCode::BAD_REQUEST, "Could not read the MCP request").into_response()
            };
        }
    };
    let message = serde_json::from_slice::<serde_json::Value>(&bytes).ok();
    if let (Some(message), Some(token)) = (&message, bearer) {
        if crate::mcp::events::events_method(message).is_some() {
            let caller = crate::mcp::events::Caller::new(
                token,
                access_token.client_id.clone(),
                permissions,
                access_token.expires_at,
            );
            return crate::mcp::events::handle(state, caller, &parts.headers, message).await;
        }
    }
    let initialize = message
        .as_ref()
        .and_then(|message| message.get("method"))
        .is_some_and(|method| method == "initialize");
    let request = axum::http::Request::from_parts(parts, axum::body::Body::from(bytes));
    let service = create_mcp_service(state, permissions);
    // StreamableHttpService has an async handle method we can call directly
    let response = service.handle(request).await.into_response();
    if initialize {
        crate::mcp::events::advertise(response).await
    } else {
        response
    }
}

/// The largest MCP JSON-RPC request body accepted. MCP tools take text only.
const MAX_MCP_REQUEST_BYTES: usize = 4 * 1024 * 1024;
