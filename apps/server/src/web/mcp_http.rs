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
    let service = create_mcp_service(state, permissions);
    // StreamableHttpService has an async handle method we can call directly
    service.handle(request).await.into_response()
}
