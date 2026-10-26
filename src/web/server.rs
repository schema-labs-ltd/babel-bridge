//! Router construction and server startup.

use super::*;

/// Create the web server router
pub fn create_router(state: Arc<AppState>) -> Router {
    // Serve static files from the web directory
    let serve_dir = ServeDir::new(&state.web_dir);
    let auth_state = state.clone();

    let protected_api = Router::new()
        .route("/api/logout", post(logout))
        .route("/api/status", get(get_status))
        .route("/api/diagnostics", get(get_diagnostics))
        .route("/api/outbox/:id", get(crate::outbox::status))
        .route(
            "/api/settings/openai",
            get(get_openai_settings).put(update_openai_settings),
        )
        .route("/api/chatgpt", get(crate::chatgpt::routes::status))
        .route("/api/chatgpt/sign-in", post(crate::chatgpt::routes::start))
        .route(
            "/api/chatgpt/callback",
            post(crate::chatgpt::routes::callback),
        )
        .route("/api/chatgpt/model", put(crate::chatgpt::routes::set_model))
        .route(
            "/api/chatgpt/models/refresh",
            post(crate::chatgpt::routes::refresh_models),
        )
        .route(
            "/api/chatgpt/sign-out",
            post(crate::chatgpt::routes::sign_out),
        )
        .route("/api/contacts", get(get_contacts))
        .route(
            "/api/settings/message-tone",
            get(crate::message_tones::get_settings).put(crate::message_tones::put_settings),
        )
        .route("/api/push/devices", put(register_push_device))
        .route("/api/push/test", post(send_test_push_notification))
        .route(
            "/api/push/devices/:installation_id",
            delete(unregister_push_device),
        )
        .route("/api/contacts/:contact_id/read", post(mark_contact_as_read))
        .route("/api/contacts/:contact_id/pin", post(toggle_pin))
        .route(
            "/api/contacts/:contact_id/settings",
            get(get_conversation_settings).put(update_conversation_settings),
        )
        .route("/api/messages/:contact_id", get(get_messages))
        .route("/api/gallery/:contact_id", get(crate::gallery::messages))
        .route("/api/feed", get(get_unified_messages))
        .route("/api/topics", get(crate::topics::catalog))
        .route("/api/topics/messages", post(crate::topics::message_topics))
        .route(
            "/api/topics/import",
            get(crate::topics::import_preview).post(crate::topics::import_recent),
        )
        .route("/api/topics/:id/messages", get(crate::topics::messages))
        .route(
            "/api/contacts/:id/topics",
            put(crate::topics::update_setting),
        )
        .route("/api/media/:message_id", get(get_media))
        .route(
            "/api/media/:message_id/thumbnail",
            get(crate::gallery::thumbnail),
        )
        .route("/api/avatar/:jid", get(get_avatar))
        .route("/api/qr", get(get_qr))
        .route("/api/send", post(send_message))
        .route(
            "/api/voice/settings/:scope",
            get(crate::voice::get_settings).put(crate::voice::put_settings),
        )
        .route(
            "/api/voice/translate/:message_id",
            post(crate::voice::translate_received),
        )
        .route(
            "/api/voice/prepare",
            post(crate::voice::prepare).layer(DefaultBodyLimit::max(22 * 1024 * 1024)),
        )
        .route("/api/voice/send", post(crate::voice::send))
        .route("/api/voice/sample/:voice", post(crate::voice::sample))
        .route(
            "/api/send-media",
            post(send_image).layer(DefaultBodyLimit::max(MAX_ALBUM_REQUEST_BYTES)),
        )
        .route(
            "/api/send-image",
            post(send_image).layer(DefaultBodyLimit::max(MAX_IMAGE_REQUEST_BYTES)),
        )
        .route(
            "/api/send-images",
            post(send_images).layer(DefaultBodyLimit::max(MAX_ALBUM_REQUEST_BYTES)),
        )
        .route("/api/photo-albums", post(create_photo_album))
        .route(
            "/api/photo-albums/:job_id/images/:index",
            put(stage_photo_album_image).layer(DefaultBodyLimit::max(MAX_IMAGE_REQUEST_BYTES)),
        )
        .route(
            "/api/photo-albums/:job_id/send",
            post(send_staged_photo_album),
        )
        .route("/api/react", post(send_reaction))
        .route("/api/ai-compose", post(ai_compose))
        .route("/api/ai-reply", post(ai_reply))
        .route("/api/translate", post(translate_message))
        .route("/api/stats", get(get_stats))
        .route("/api/usage", get(get_global_usage))
        .route("/api/usage/:contact_id", get(get_conversation_usage))
        .route("/api/link-preview", get(get_link_preview))
        .route("/api/oauth/clients", get(list_oauth_clients))
        .route("/api/oauth/clients/:client_id", delete(revoke_oauth_client))
        .route("/api/oauth/revoke-all", post(revoke_all_oauth_clients))
        .route("/ws", get(websocket_handler))
        .route_layer(middleware::from_fn(move |req: Request, next: Next| {
            let state = auth_state.clone();
            async move { require_web_auth(state, req, next).await }
        }));

    Router::new()
        .route("/", get(serve_index))
        .route("/index.html", get(serve_index))
        .route("/api/health", get(get_health))
        // OAuth 2.0 routes for MCP authentication
        .route(
            "/.well-known/oauth-authorization-server",
            get(oauth_metadata),
        )
        .route(
            "/.well-known/oauth-protected-resource",
            get(oauth_protected_resource_metadata),
        )
        .route("/oauth/register", post(oauth_register))
        .route("/oauth/authorize", get(oauth_authorize))
        .route("/oauth/approve", post(oauth_approve))
        .route("/oauth/token", post(oauth_token))
        .route("/oauth/revoke", post(oauth_revoke))
        // Auth routes (no auth required)
        .route("/api/auth/check", get(auth_check))
        .route("/api/auth", post(auth_login))
        .route("/api/auth/session", delete(delete_auth_session))
        .merge(protected_api)
        // MCP (Model Context Protocol) endpoint - HTTP transport
        .route("/mcp", post(mcp_handler))
        // Serve static files
        .fallback_service(serve_dir)
        .with_state(state)
}

/// Start the web server
pub async fn start_server(
    state: Arc<AppState>,
    host: &str,
    port: u16,
    allow_local_no_auth: bool,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    crate::access::validate(host, state.password.as_deref(), allow_local_no_auth)?;
    let addr = SocketAddr::new(host.parse()?, port);
    cleanup_expired_oauth(&state, "startup");
    spawn_oauth_cleanup_task(state.clone());
    let router = create_router(state);

    info!("Web server running at http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    // HTTP, WebSocket, and MCP share this server. Graceful shutdown stops new
    // connections and waits for the ones already accepted.
    crate::shutdown::serve_until_shutdown(listener, router, shutdown).await?;

    Ok(())
}

fn cleanup_expired_oauth(state: &Arc<AppState>, reason: &str) {
    if let Err(e) = state.store.oauth_cleanup_expired() {
        warn!(
            "Failed to clean expired OAuth records during {}: {}",
            reason, e
        );
    }
}

fn spawn_oauth_cleanup_task(state: Arc<AppState>) {
    tokio::spawn(async move {
        loop {
            if state.wait_for_shutdown(OAUTH_CLEANUP_INTERVAL).await {
                break;
            }
            cleanup_expired_oauth(&state, "periodic cleanup");
        }
    });
}
