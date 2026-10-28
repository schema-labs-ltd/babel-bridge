use super::*;

pub(in crate::web) async fn send_image(
    State(state): State<Arc<AppState>>,
    Json(mut req): Json<SendImageRequest>,
) -> impl IntoResponse {
    // Validate input
    if req.contact_id.is_empty() || req.media_data.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "contact_id and media_data are required"
            })),
        )
            .into_response();
    }

    let kind = req.media_kind.as_deref().unwrap_or("image");
    let content_type = match kind {
        "video" => "Video",
        "document" => "Document",
        _ => "Image",
    };
    let validated_image = match validate_attachment_payload(
        &req.media_data,
        &req.mime_type,
        kind,
        req.file_name.as_deref(),
    ) {
        Ok(payload) => payload,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": error
                })),
            )
                .into_response();
        }
    };
    let validated_mime_type = validated_image.mime_type.clone();
    let validated_decoded_size = validated_image.decoded_size;

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

    if let Err(error) = media_reply_needs_quote(
        &state,
        &req.contact_id,
        req.reply_to.as_deref(),
        req.reply_only_if_not_latest,
    ) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": error})),
        )
            .into_response();
    }

    let (translated_caption, caption_language) = match translate_caption(
        &state,
        &req.contact_id,
        req.reply_to.as_deref(),
        req.caption.as_deref(),
    )
    .await
    {
        Ok(result) => result,
        Err(error) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"error": error.to_string()})),
            )
                .into_response()
        }
    };
    let caption_changed = translated_caption != req.caption;
    match media_reply_needs_quote(
        &state,
        &req.contact_id,
        req.reply_to.as_deref(),
        req.reply_only_if_not_latest,
    ) {
        Ok(false) => {
            req.reply_to = None;
            req.reply_to_sender = None;
            req.reply_to_text = None;
            req.reply_to_sender_name = None;
        }
        Ok(true) => {}
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": error})),
            )
                .into_response()
        }
    }

    let request_id = state.next_request_id();
    let timestamp = chrono::Utc::now().timestamp_millis();
    let temp_message_id = format!("pending_img_{}_{}", request_id, timestamp);

    // Get contact info for the recipient
    let contact_info = state.store.get_contact(&req.contact_id).ok().flatten();
    let contact_name = contact_info.as_ref().and_then(|c| c.name.clone());
    let contact_phone = contact_info.as_ref().and_then(|c| c.phone.clone());
    let chat_type = contact_info
        .as_ref()
        .and_then(|c| c.contact_type.clone())
        .unwrap_or_else(|| "private".to_string());

    // Store the sent image message locally
    let stored_msg = crate::storage::StoredMessage {
        id: temp_message_id.clone(),
        contact_id: req.contact_id.clone(),
        timestamp,
        is_from_me: true,
        is_forwarded: false,
        sender_name: state.name.read().await.clone(),
        sender_phone: state.phone.read().await.clone(),
        contact_name,
        contact_phone,
        chat_type,
        content_type: content_type.to_string(),
        content_json: serde_json::json!({
            "type": kind,
            "file_name": req.file_name,
            "mime_type": validated_mime_type.clone(),
            "caption": translated_caption,
            "media_data": req.media_data,
            "file_size": validated_decoded_size,
            "reply_context": req.reply_to.as_ref().map(|reply_to| serde_json::json!({
                "messageId": reply_to,
                "senderName": req.reply_to_sender_name.clone().unwrap_or_else(|| {
                    req.reply_to_sender.clone().unwrap_or_else(|| "Unknown".to_string())
                }),
                "text": req.reply_to_text.clone().unwrap_or_default()
            }))
        })
        .to_string(),
        content: Some(serde_json::json!({
            "type": kind,
            "file_name": req.file_name,
            "mime_type": validated_mime_type.clone(),
            "caption": translated_caption,
            "media_data": req.media_data,
            "file_size": validated_decoded_size,
            "reply_context": req.reply_to.as_ref().map(|reply_to| serde_json::json!({
                "messageId": reply_to,
                "senderName": req.reply_to_sender_name.clone().unwrap_or_else(|| {
                    req.reply_to_sender.clone().unwrap_or_else(|| "Unknown".to_string())
                }),
                "text": req.reply_to_text.clone().unwrap_or_default()
            }))
        })),
        original_text: req.caption.clone(),
        translated_text: if caption_changed {
            translated_caption.clone()
        } else {
            None
        },
        source_language: caption_language.clone(),
        is_translated: caption_changed,
        delivery_status: Some("sending".to_string()),
    };

    // Store the message
    if let Err(e) = state.store.add_message(&stored_msg) {
        error!("Failed to store sent image: {}", e);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Failed to store pending attachment: {}", e)
            })),
        )
            .into_response();
    }

    // Update contact's last message time (preserve contact name/phone)
    if let Err(e) = state.store.upsert_contact(
        &stored_msg.contact_id,
        stored_msg.contact_name.as_deref(),
        stored_msg.contact_phone.as_deref(),
        Some(&stored_msg.chat_type),
        stored_msg.timestamp,
    ) {
        error!("Failed to update contact: {}", e);
        let _ = state.store.delete_message(&temp_message_id);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Failed to update contact: {}", e)
            })),
        )
            .into_response();
    }

    let rx = state
        .register_pending_send(request_id, Some(temp_message_id.clone()))
        .await;

    let cmd = if kind == "image" {
        BridgeCommand::SendImage {
            request_id: Some(request_id),
            to: req.contact_id.clone(),
            media_data: req.media_data.clone(),
            mime_type: validated_mime_type,
            caption: translated_caption.clone(),
            reply_to: req.reply_to.clone(),
            reply_to_sender: req.reply_to_sender.clone(),
            reply_to_text: req.reply_to_text.clone(),
        }
    } else {
        BridgeCommand::SendMedia {
            request_id: Some(request_id),
            to: req.contact_id.clone(),
            media_data: req.media_data.clone(),
            mime_type: validated_mime_type,
            media_kind: kind.to_string(),
            file_name: req.file_name.clone(),
            caption: translated_caption.clone(),
            reply_to: req.reply_to.clone(),
            reply_to_sender: req.reply_to_sender.clone(),
            reply_to_text: req.reply_to_text.clone(),
        }
    };

    if let Err(e) = state.send_bridge_command(cmd).await {
        error!("Failed to send attachment: {}", e);
        state.cancel_pending_send(request_id).await;
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Failed to send attachment: {}", e)
            })),
        )
            .into_response();
    }

    let send_result = match state
        .wait_for_send_result_with_timeout(request_id, rx, ALBUM_SEND_RESULT_TIMEOUT)
        .await
    {
        Ok(result) => result,
        Err(SendConfirmationError::Timeout) => {
            return (
                StatusCode::GATEWAY_TIMEOUT,
                Json(serde_json::json!({
                    "error": "Timed out waiting for WhatsApp attachment send confirmation"
                })),
            )
                .into_response();
        }
        Err(SendConfirmationError::ChannelClosed) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({
                    "error": "WhatsApp attachment send confirmation channel closed"
                })),
            )
                .into_response();
        }
    };

    if !send_result.success {
        return (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({
                "error": send_result.error.unwrap_or_else(|| "WhatsApp rejected the attachment".to_string())
            })),
        )
            .into_response();
    }

    let confirmed_message_id = send_result.message_id.unwrap_or(temp_message_id);
    let confirmed_timestamp = send_result.timestamp.unwrap_or(timestamp);

    Json(SendImageResponse {
        message_id: confirmed_message_id,
        timestamp: confirmed_timestamp,
    })
    .into_response()
}
