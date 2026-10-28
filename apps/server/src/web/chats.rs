//! Read endpoints for status, contacts, conversation settings, messages, media, avatars and link previews.

use super::*;

pub(super) async fn get_status(State(state): State<Arc<AppState>>) -> Json<StatusResponse> {
    Json(StatusResponse {
        connected: *state.connected.read().await,
        phone: state.phone.read().await.clone(),
        name: state.name.read().await.clone(),
        connection_state: *state.connection_state.read().await,
        qr: state.qr_code.read().await.clone(),
    })
}

/// Recent warnings and errors (already redacted by the log writer) and
/// connection state, so an agent can diagnose production without log access.
/// Never includes the account's phone, name, QR code or any message data.
pub(super) async fn get_diagnostics(
    State(state): State<Arc<AppState>>,
) -> Json<DiagnosticsResponse> {
    Json(DiagnosticsResponse {
        version: env!("CARGO_PKG_VERSION"),
        commit: std::env::var("RAILWAY_GIT_COMMIT_SHA").ok(),
        connected: *state.connected.read().await,
        connection_state: *state.connection_state.read().await,
        ai_configured: state
            .ai_service
            .as_ref()
            .is_some_and(|service| service.is_configured()),
        recent_problems: crate::log_privacy::recent_problems(),
    })
}

pub(super) async fn get_health() -> Json<HealthResponse> {
    Json(HealthResponse {
        ok: true,
        service: "whatsapp-translator",
    })
}

pub(super) async fn serve_index(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let index_path = state.web_dir.join("index.html");

    match tokio::fs::read_to_string(&index_path).await {
        Ok(contents) => Html(contents).into_response(),
        Err(err) => {
            error!("Failed to read index.html from {:?}: {}", index_path, err);
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to load web UI").into_response()
        }
    }
}

pub(super) async fn get_contacts(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match state.store.get_contacts() {
        Ok(contacts) => Json(contacts).into_response(),
        Err(e) => {
            error!("Failed to get contacts: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to get contacts").into_response()
        }
    }
}

pub(super) async fn mark_contact_as_read(
    State(state): State<Arc<AppState>>,
    Path(contact_id): Path<String>,
    Json(req): Json<MarkReadRequest>,
) -> impl IntoResponse {
    let contact_id = urlencoding::decode(&contact_id)
        .map(|s| s.into_owned())
        .unwrap_or(contact_id);
    let contact_id = crate::identity::canonical_chat_id(&contact_id).into_owned();

    let mut ids = req.message_ids;
    if let Some(id) = req.message_id {
        ids.push(id);
    }
    ids.sort();
    ids.dedup();
    if ids.is_empty() {
        // Compatibility: older clients only clear their local badge. Never infer viewed IDs.
        return match state.store.mark_as_read(&contact_id) {
            Ok(()) => Json(serde_json::json!({"success":true,"unreadCount":0})).into_response(),
            Err(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to clear unread count",
            )
                .into_response(),
        };
    }
    match state
        .store
        .record_message_views(&contact_id, &ids, chrono::Utc::now().timestamp())
    {
        Ok(unread_count) => {
            let _ = state.broadcast_tx.send(WebSocketEvent::MessagesViewed {
                chat_id: contact_id,
                message_ids: ids,
                unread_count,
            });
            Json(serde_json::json!({"success":true,"unreadCount":unread_count})).into_response()
        }
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"success":false,"error":error.to_string()})),
        )
            .into_response(),
    }
}

/// Toggle pin status for a contact
pub(super) async fn toggle_pin(
    State(state): State<Arc<AppState>>,
    Path(contact_id): Path<String>,
) -> impl IntoResponse {
    let contact_id = urlencoding::decode(&contact_id)
        .map(|s| s.into_owned())
        .unwrap_or(contact_id);
    let contact_id = crate::identity::canonical_chat_id(&contact_id).into_owned();

    match state.store.toggle_pin(&contact_id) {
        Ok(is_pinned) => Json(serde_json::json!({
            "success": true,
            "pinned": is_pinned
        }))
        .into_response(),
        Err(e) => {
            error!("Failed to toggle pin: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to toggle pin").into_response()
        }
    }
}

/// Get conversation settings for a contact
pub(super) async fn get_conversation_settings(
    State(state): State<Arc<AppState>>,
    Path(contact_id): Path<String>,
) -> impl IntoResponse {
    let contact_id = crate::identity::canonical_chat_id(&contact_id).into_owned();
    match state.store.get_conversation_settings(&contact_id) {
        Ok(settings) => Json(ConversationSettingsResponse {
            reply_only: settings.reply_only,
            translation_enabled: settings.translation_enabled,
            language_override: settings.language_override,
            translation_style: settings.translation_style,
            send_original_follow_up: settings.send_original_follow_up,
        })
        .into_response(),
        Err(e) => {
            error!("Failed to get conversation settings: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to get conversation settings",
            )
                .into_response()
        }
    }
}

/// Update conversation settings for a contact
pub(super) async fn update_conversation_settings(
    State(state): State<Arc<AppState>>,
    Path(contact_id): Path<String>,
    Json(req): Json<UpdateConversationSettingsRequest>,
) -> impl IntoResponse {
    let contact_id = crate::identity::canonical_chat_id(&contact_id).into_owned();
    match state.store.get_contact(&contact_id) {
        Ok(Some(_)) => {}
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error":"Conversation not found"})),
            )
                .into_response()
        }
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error":"Could not load conversation"})),
            )
                .into_response()
        }
    }
    let previous = match state.store.get_conversation_settings(&contact_id) {
        Ok(settings) => settings,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error":"Could not load conversation settings"})),
            )
                .into_response()
        }
    };

    // Convert empty strings to None
    let settings = crate::storage::ConversationSettings {
        reply_only: req.reply_only.unwrap_or(previous.reply_only),
        translation_enabled: req
            .translation_enabled
            .unwrap_or(previous.translation_enabled),
        language_override: req.language_override.filter(|s| !s.trim().is_empty()),
        translation_style: req.translation_style.filter(|s| !s.trim().is_empty()),
        send_original_follow_up: req.send_original_follow_up,
    };

    match state
        .store
        .update_conversation_settings(&contact_id, &settings)
    {
        Ok(()) => {
            let _ = state
                .broadcast_tx
                .send(WebSocketEvent::ConversationSettingsUpdated {
                    chat_id: contact_id,
                    settings: settings.clone(),
                });
            Json(serde_json::json!({
                "success": true,
                "replyOnly": settings.reply_only,
                "translationEnabled": settings.translation_enabled,
                "languageOverride": settings.language_override,
                "translationStyle": settings.translation_style,
                "sendOriginalFollowUp": settings.send_original_follow_up
            }))
            .into_response()
        }
        Err(e) => {
            error!("Failed to update conversation settings: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "success": false,
                    "error": format!("Failed to update settings: {}", e)
                })),
            )
                .into_response()
        }
    }
}

pub(super) async fn get_openai_settings(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match state.store.get_openai_settings() {
        Ok(settings) => Json(serde_json::json!({
            "model": settings.model,
            "reasoningEffort": settings.reasoning_effort,
            "available": state.translator().is_some()
        }))
        .into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

pub(super) async fn update_openai_settings(
    State(state): State<Arc<AppState>>,
    Json(req): Json<UpdateOpenAiSettingsRequest>,
) -> impl IntoResponse {
    let model = req.model.filter(|value| !value.is_empty());
    let reasoning_effort = req.reasoning_effort.filter(|value| !value.is_empty());
    let reasoning_effort = if model.as_deref() == Some("gpt-6-astra")
        && matches!(reasoning_effort.as_deref(), None | Some("none"))
    {
        Some("low".to_string())
    } else {
        reasoning_effort
    };
    if model
        .as_deref()
        .is_some_and(|value| !SUPPORTED_OPENAI_MODELS.contains(&value))
        || reasoning_effort
            .as_deref()
            .is_some_and(|value| !SUPPORTED_REASONING_EFFORTS.contains(&value))
    {
        return (
            StatusCode::BAD_REQUEST,
            "Unsupported model or reasoning level",
        )
            .into_response();
    }
    let settings = OpenAiSettings {
        model,
        reasoning_effort,
    };
    if let Err(error) = state.store.update_openai_settings(&settings) {
        return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
    }
    if let Some(translator) = &state.ai_service {
        translator.set_runtime_settings(settings.clone());
    }
    Json(serde_json::json!({ "success": true, "model": settings.model, "reasoningEffort": settings.reasoning_effort })).into_response()
}

/// Query parameters for messages pagination
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct MessagesQuery {
    /// Maximum number of messages to return (default: 30 for initial load)
    limit: Option<u32>,
    /// Only get messages before this timestamp (for loading older messages)
    before: Option<i64>,
    /// Tie-breaker message ID for stable pagination when timestamps match
    #[serde(alias = "before_id")]
    before_id: Option<String>,
}

/// Response for paginated messages
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MessagesResponse {
    messages: Vec<crate::storage::PresentedMessage>,
    has_more: bool,
}

pub(crate) fn present_message_page(
    store: &MessageStore,
    messages: Vec<StoredMessage>,
    has_more: bool,
) -> Response {
    match store.present_messages(messages) {
        Ok(messages) => Json(MessagesResponse { messages, has_more }).into_response(),
        Err(error) => {
            error!("Could not load message reaction state: {error}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not load message reactions",
            )
                .into_response()
        }
    }
}

pub(super) async fn get_unified_messages(
    State(state): State<Arc<AppState>>,
    Query(params): Query<MessagesQuery>,
) -> impl IntoResponse {
    let limit = params.limit.unwrap_or(50).clamp(1, 100);
    match state
        .store
        .get_unified_messages(limit + 1, params.before, params.before_id.as_deref())
    {
        Ok(mut messages) => {
            let has_more = messages.len() > limit as usize;
            if has_more {
                messages.remove(0);
            }
            if state.translator().is_some() {
                for message in &messages {
                    if !message.is_from_me
                        && !message.is_audio()
                        && message.source_language.is_none()
                        && message
                            .original_text
                            .as_deref()
                            .is_some_and(|text| !text.trim().is_empty())
                    {
                        if let Err(error) = state.store.enqueue_translation(&message.id) {
                            warn!("Could not queue feed translation: {error}");
                        }
                    }
                }
            }
            present_message_page(&state.store, messages, has_more)
        }
        Err(error) => {
            error!("Failed to load unified feed: {error}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Could not load messages").into_response()
        }
    }
}

pub(super) async fn get_messages(
    State(state): State<Arc<AppState>>,
    Path(contact_id): Path<String>,
    Query(params): Query<MessagesQuery>,
) -> impl IntoResponse {
    let contact_id = urlencoding::decode(&contact_id)
        .map(|s| s.into_owned())
        .unwrap_or(contact_id);
    let contact_id = crate::identity::canonical_chat_id(&contact_id).into_owned();

    // Default to 30 messages for initial load, unless explicitly requesting all (limit=0)
    let limit = match params.limit {
        Some(0) => None, // 0 means all messages (for backwards compatibility / MCP)
        Some(n) => Some(n),
        None => Some(30), // Default to 30 for lazy loading
    };
    let storage_limit = limit.map(|l| l.saturating_add(1));

    // Strip media_data from messages to reduce payload (media loaded on demand via /api/media)
    match state.store.get_messages_paginated(
        &contact_id,
        storage_limit,
        params.before,
        params.before_id.as_deref(),
        true,
    ) {
        Ok(mut messages) => {
            let has_more = limit.map(|l| messages.len() > l as usize).unwrap_or(false);
            if has_more {
                messages.remove(0);
            }
            if state.translator().is_some() {
                for message in &messages {
                    if !message.is_from_me
                        && !message.is_audio()
                        && message.source_language.is_none()
                        && message
                            .original_text
                            .as_deref()
                            .is_some_and(|text| !text.trim().is_empty())
                    {
                        if let Err(error) = state.store.enqueue_translation(&message.id) {
                            warn!("Could not queue conversation translation: {error}");
                        }
                    }
                }
            }
            present_message_page(&state.store, messages, has_more)
        }
        Err(e) => {
            error!("Failed to get messages: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to get messages").into_response()
        }
    }
}

/// Get media data for a specific message (lazy loaded)
pub(super) async fn get_media(
    State(state): State<Arc<AppState>>,
    Path(message_id): Path<String>,
) -> impl IntoResponse {
    let message_id = urlencoding::decode(&message_id)
        .map(|s| s.into_owned())
        .unwrap_or(message_id);

    match state.store.get_message_media(&message_id) {
        Ok(Some((media_data, mime_type))) => {
            let is_audio = mime_type
                .as_deref()
                .is_some_and(|mime| mime.starts_with("audio/"))
                || state
                    .store
                    .get_message_by_id(&message_id)
                    .ok()
                    .flatten()
                    .is_some_and(|message| message.is_audio());
            let (media_data, mime_type) = if is_audio {
                match crate::voice::playable_audio(&media_data).await {
                    Ok(data) => (data, Some("audio/mpeg".to_string())),
                    Err(error) => {
                        return (
                            StatusCode::UNPROCESSABLE_ENTITY,
                            Json(serde_json::json!({"error": error.to_string()})),
                        )
                            .into_response()
                    }
                }
            } else {
                (media_data, mime_type)
            };
            Json(serde_json::json!({
                "media_data": media_data,
                "mime_type": mime_type
            }))
            .into_response()
        }
        Ok(None) => (StatusCode::NOT_FOUND, "Media not found").into_response(),
        Err(e) => {
            error!("Failed to get media: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to get media").into_response()
        }
    }
}

pub(super) async fn get_qr(State(state): State<Arc<AppState>>) -> Json<QrResponse> {
    Json(QrResponse {
        qr: state.qr_code.read().await.clone(),
    })
}

/// Avatar response
#[derive(Serialize)]
struct AvatarResponse {
    url: Option<String>,
}

pub(super) async fn get_avatar(
    State(state): State<Arc<AppState>>,
    Path(jid): Path<String>,
) -> impl IntoResponse {
    let jid = urlencoding::decode(&jid)
        .map(|s| s.into_owned())
        .unwrap_or(jid);
    let jid = crate::identity::canonical_chat_id(&jid).into_owned();

    // Check if connected
    if !*state.connected.read().await {
        return Json(AvatarResponse { url: None }).into_response();
    }

    let url = state.get_profile_picture(&jid).await;
    Json(AvatarResponse { url }).into_response()
}

/// Query parameters for link preview
#[derive(Deserialize)]
pub(super) struct LinkPreviewQuery {
    url: String,
}

/// Get link preview for a URL (with caching)
pub(super) async fn get_link_preview(
    State(state): State<Arc<AppState>>,
    Query(query): Query<LinkPreviewQuery>,
) -> impl IntoResponse {
    use crate::link_preview;

    let url = query.url;

    if let Err(e) = link_preview::validate_link_preview_url(&url).await {
        return Json(link_preview::LinkPreview::error(
            url,
            format!("Blocked link preview URL: {}", e),
        ))
        .into_response();
    }

    // Cache duration: 24 hours for successful fetches, 1 hour for errors
    let cache_duration = 24 * 60 * 60; // 24 hours

    // Check cache first
    match state.store.get_link_preview(&url, cache_duration) {
        Ok(Some(preview)) => {
            return Json(preview).into_response();
        }
        Ok(None) => {
            // Not in cache, need to fetch
        }
        Err(e) => {
            warn!("Failed to check link preview cache: {}", e);
            // Continue to fetch
        }
    }

    // Fetch the preview
    match link_preview::fetch_link_preview(&url).await {
        Ok(preview) => {
            // Cache the result
            if let Err(e) = state.store.save_link_preview(&preview) {
                warn!("Failed to cache link preview: {}", e);
            }
            Json(preview).into_response()
        }
        Err(e) => {
            error!("Failed to fetch link preview for {}: {}", url, e);
            // Return error preview
            let error_preview =
                link_preview::LinkPreview::error(url, format!("Failed to fetch: {}", e));
            // Cache the error for a shorter duration (by saving it)
            let _ = state.store.save_link_preview(&error_preview);
            Json(error_preview).into_response()
        }
    }
}
