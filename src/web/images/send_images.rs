use super::*;

pub(in crate::web) async fn send_images(
    State(state): State<Arc<AppState>>,
    Json(mut req): Json<SendImagesRequest>,
) -> impl IntoResponse {
    if req.contact_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "contact_id is required" })),
        )
            .into_response();
    }

    let validated = match validate_album_payloads(&req.images) {
        Ok(images) => images,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": error })),
            )
                .into_response();
        }
    };

    if !*state.connected.read().await {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "Not connected to WhatsApp" })),
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
    let rx = state.register_pending_send(request_id, None).await;
    let command = BridgeCommand::SendImages {
        request_id: Some(request_id),
        progress_id: req.progress_id.clone(),
        to: req.contact_id.clone(),
        images: req
            .images
            .iter()
            .zip(validated.iter())
            .map(|(image, validated)| BridgeImage {
                media_data: image.media_data.clone(),
                mime_type: validated.mime_type.clone(),
            })
            .collect(),
        caption: translated_caption.clone(),
        reply_to: req.reply_to.clone(),
        reply_to_sender: req.reply_to_sender.clone(),
        reply_to_text: req.reply_to_text.clone(),
    };

    if let Err(error) = state.send_bridge_command(command).await {
        state.cancel_pending_send(request_id).await;
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Failed to send album: {}", error) })),
        )
            .into_response();
    }

    let send_result = match state
        .wait_for_send_result_with_timeout(
            request_id,
            rx,
            album_send_result_timeout(req.images.len()),
        )
        .await
    {
        Ok(result) => result,
        Err(SendConfirmationError::Timeout) => {
            return (
                StatusCode::GATEWAY_TIMEOUT,
                Json(serde_json::json!({ "error": "Timed out waiting for WhatsApp album confirmation" })),
            )
                .into_response();
        }
        Err(SendConfirmationError::ChannelClosed) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": "WhatsApp album confirmation channel closed" })),
            )
                .into_response();
        }
    };

    if !send_result.success {
        return (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({
                "error": send_result.error.unwrap_or_else(|| "WhatsApp rejected the album".to_string())
            })),
        )
            .into_response();
    }
    if send_result.message_ids.len() != req.images.len() {
        return (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": "WhatsApp returned an incomplete album confirmation" })),
        )
            .into_response();
    }

    let now = chrono::Utc::now().timestamp_millis();
    let contact_info = state.store.get_contact(&req.contact_id).ok().flatten();
    let contact_name = contact_info
        .as_ref()
        .and_then(|contact| contact.name.clone());
    let contact_phone = contact_info
        .as_ref()
        .and_then(|contact| contact.phone.clone());
    let chat_type = contact_info
        .as_ref()
        .and_then(|contact| contact.contact_type.clone())
        .unwrap_or_else(|| "private".to_string());
    let sender_name = state.name.read().await.clone();
    let sender_phone = state.phone.read().await.clone();
    let mut stored_ids: Vec<String> = Vec::new();
    let album_id = send_result
        .message_id
        .clone()
        .unwrap_or_else(|| send_result.message_ids[0].clone());

    for (index, ((image, validated), message_id)) in req
        .images
        .iter()
        .zip(validated.iter())
        .zip(send_result.message_ids.iter())
        .enumerate()
    {
        let timestamp = send_result
            .timestamps
            .get(index)
            .copied()
            .unwrap_or(now + index as i64);
        let caption = if index == 0 {
            translated_caption.clone()
        } else {
            None
        };
        let reply_context = if index == 0 {
            req.reply_to.as_ref().map(|reply_to| {
                serde_json::json!({
                    "messageId": reply_to,
                    "senderName": req.reply_to_sender_name.clone().unwrap_or_else(|| {
                        req.reply_to_sender.clone().unwrap_or_else(|| "Unknown".to_string())
                    }),
                    "text": req.reply_to_text.clone().unwrap_or_default()
                })
            })
        } else {
            None
        };
        let content = serde_json::json!({
            "type": "image",
            "mime_type": validated.mime_type,
            "caption": caption,
            "media_data": image.media_data,
            "file_size": validated.decoded_size,
            "reply_context": reply_context,
            "album_id": album_id,
            "album_index": index
        });
        let stored_message = StoredMessage {
            id: message_id.clone(),
            contact_id: req.contact_id.clone(),
            timestamp,
            is_from_me: true,
            is_forwarded: false,
            sender_name: sender_name.clone(),
            sender_phone: sender_phone.clone(),
            contact_name: contact_name.clone(),
            contact_phone: contact_phone.clone(),
            chat_type: chat_type.clone(),
            content_type: "Image".to_string(),
            content_json: content.to_string(),
            content: Some(content),
            original_text: if index == 0 {
                req.caption.clone()
            } else {
                None
            },
            translated_text: if index == 0 && caption_changed {
                translated_caption.clone()
            } else {
                None
            },
            source_language: if index == 0 {
                caption_language.clone()
            } else {
                None
            },
            is_translated: index == 0 && caption_changed,
            delivery_status: Some("sent".to_string()),
        };
        if let Err(error) = state.store.add_message(&stored_message) {
            for stored_id in &stored_ids {
                let _ = state.store.delete_message(stored_id);
            }
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Failed to store sent album: {}", error) })),
            )
                .into_response();
        }
        stored_ids.push(message_id.clone());
    }

    let last_timestamp = send_result.timestamps.last().copied().unwrap_or(now);
    if let Err(error) = state.store.upsert_contact(
        &req.contact_id,
        contact_name.as_deref(),
        contact_phone.as_deref(),
        Some(&chat_type),
        last_timestamp,
    ) {
        warn!("Failed to update contact after album send: {}", error);
    }

    Json(SendImageResponse {
        message_id: album_id,
        timestamp: send_result.timestamp.unwrap_or(now),
    })
    .into_response()
}
