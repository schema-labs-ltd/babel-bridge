use super::*;

pub(super) async fn send_original_follow_up(
    state: &Arc<AppState>,
    req: &SendMessageRequest,
    primary_message: &StoredMessage,
    original_text: &str,
) -> Result<(String, i64), String> {
    let request_id = state.next_request_id();
    let timestamp = chrono::Utc::now()
        .timestamp_millis()
        .max(primary_message.timestamp.saturating_add(1));
    let temp_message_id = format!("pending_original_{}_{}", request_id, timestamp);
    let content = serde_json::json!({
        "type": "text",
        "body": original_text,
        "reply_context": req.reply_to.as_ref().map(|id| serde_json::json!({"messageId":id,"senderName":req.reply_to_sender_name,"text":req.reply_to_text})),
    });
    let stored_msg = StoredMessage {
        id: temp_message_id.clone(),
        contact_id: req.contact_id.clone(),
        timestamp,
        is_from_me: true,
        is_forwarded: false,
        sender_name: primary_message.sender_name.clone(),
        sender_phone: primary_message.sender_phone.clone(),
        contact_name: primary_message.contact_name.clone(),
        contact_phone: primary_message.contact_phone.clone(),
        chat_type: primary_message.chat_type.clone(),
        content_type: "Text".to_string(),
        content_json: content.to_string(),
        content: Some(content),
        original_text: None,
        translated_text: None,
        source_language: None,
        is_translated: false,
        delivery_status: Some("sending".to_string()),
    };

    state
        .store
        .add_message(&stored_msg)
        .map_err(|error| format!("Failed to store original follow-up: {}", error))?;

    if let Err(error) = state.store.upsert_contact(
        &stored_msg.contact_id,
        stored_msg.contact_name.as_deref(),
        stored_msg.contact_phone.as_deref(),
        Some(&stored_msg.chat_type),
        stored_msg.timestamp,
    ) {
        let _ = state.store.delete_message(&temp_message_id);
        return Err(format!(
            "Failed to update contact for original follow-up: {}",
            error
        ));
    }

    let rx = state
        .register_pending_send(request_id, Some(temp_message_id.clone()))
        .await;
    let command = BridgeCommand::Send {
        request_id: Some(request_id),
        to: req.contact_id.clone(),
        text: original_text.to_string(),
        reply_to: req.reply_to.clone(),
        reply_to_sender: req.reply_to_sender.clone(),
        reply_to_text: req.reply_to_text.clone(),
    };

    if let Err(error) = state.send_bridge_command(command).await {
        state.cancel_pending_send(request_id).await;
        return Err(format!("Failed to send original follow-up: {}", error));
    }

    let send_result =
        state
            .wait_for_send_result(request_id, rx)
            .await
            .map_err(|error| match error {
                SendConfirmationError::Timeout => {
                    "Timed out waiting for original follow-up confirmation".to_string()
                }
                SendConfirmationError::ChannelClosed => {
                    "Original follow-up confirmation channel closed".to_string()
                }
            })?;

    if !send_result.success {
        return Err(send_result
            .error
            .unwrap_or_else(|| "WhatsApp rejected the original follow-up".to_string()));
    }

    Ok((
        send_result.message_id.unwrap_or(temp_message_id),
        send_result.timestamp.unwrap_or(timestamp),
    ))
}
