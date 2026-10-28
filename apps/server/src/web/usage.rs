//! Message statistics and AI usage handlers.

use super::*;

pub(super) async fn get_stats(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match state.store.get_stats() {
        Ok((messages, contacts)) => Json(serde_json::json!({
            "messageCount": messages,
            "contactCount": contacts,
        }))
        .into_response(),
        Err(e) => {
            error!("Failed to get stats: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to get stats").into_response()
        }
    }
}

/// Get global translation usage/cost
pub(super) async fn get_global_usage(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match state.store.get_global_usage() {
        Ok(usage) => Json(serde_json::json!({
            "inputTokens": usage.input_tokens,
            "cachedInputTokens": usage.cached_input_tokens,
            "outputTokens": usage.output_tokens,
            "costUsd": usage.cost_usd,
        }))
        .into_response(),
        Err(e) => {
            error!("Failed to get global usage: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to get usage").into_response()
        }
    }
}

/// Get translation usage/cost for a specific conversation
pub(super) async fn get_conversation_usage(
    State(state): State<Arc<AppState>>,
    Path(contact_id): Path<String>,
) -> impl IntoResponse {
    let contact_id = urlencoding::decode(&contact_id)
        .map(|s| s.into_owned())
        .unwrap_or(contact_id);
    let contact_id = crate::identity::canonical_chat_id(&contact_id).into_owned();

    match state.store.get_conversation_usage(&contact_id) {
        Ok(usage) => Json(serde_json::json!({
            "inputTokens": usage.input_tokens,
            "cachedInputTokens": usage.cached_input_tokens,
            "outputTokens": usage.output_tokens,
            "costUsd": usage.cost_usd,
        }))
        .into_response(),
        Err(e) => {
            error!("Failed to get conversation usage: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to get usage").into_response()
        }
    }
}
