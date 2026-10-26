use super::*;

pub(in crate::web) async fn send_reaction(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SendReactionRequest>,
) -> impl IntoResponse {
    // Validate input
    if req.contact_id.is_empty() || req.message_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "contact_id and message_id are required"
            })),
        )
            .into_response();
    }

    // Check if connected
    if !*state.connected.read().await {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "error": "Not connected to WhatsApp"
            })),
        )
            .into_response();
    }

    let request_id = state.next_request_id();
    let rx = state.register_pending_send(request_id, None).await;

    // Send the reaction via bridge
    let cmd = BridgeCommand::SendReaction {
        request_id: Some(request_id),
        to: req.contact_id.clone(),
        message_id: req.message_id.clone(),
        sender_jid: req.sender_jid.clone(),
        emoji: req.emoji.clone(),
    };

    if let Err(e) = state.send_bridge_command(cmd).await {
        error!("Failed to send reaction: {}", e);
        state.cancel_pending_send(request_id).await;
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Failed to send reaction: {}", e)
            })),
        )
            .into_response();
    }

    let send_result = match state.wait_for_send_result(request_id, rx).await {
        Ok(result) => result,
        Err(SendConfirmationError::Timeout) => {
            return (
                StatusCode::GATEWAY_TIMEOUT,
                Json(serde_json::json!({
                    "error": "Timed out waiting for WhatsApp reaction confirmation"
                })),
            )
                .into_response();
        }
        Err(SendConfirmationError::ChannelClosed) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({
                    "error": "WhatsApp reaction confirmation channel closed"
                })),
            )
                .into_response();
        }
    };

    if !send_result.success {
        return (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({
                "error": send_result.error.unwrap_or_else(|| "WhatsApp rejected the reaction".to_string())
            })),
        )
            .into_response();
    }

    let reaction = send_result
        .message_id
        .as_deref()
        .and_then(|id| state.store.get_message_by_id(id).ok().flatten());
    Json(SendReactionResponse {
        success: true,
        reaction,
    })
    .into_response()
}
