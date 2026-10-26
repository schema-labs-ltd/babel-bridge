//! Text message, reaction and caption sending handlers.

use super::*;

mod follow_up;
mod media;
mod reactions;

use follow_up::*;
pub(super) use media::*;
pub(super) use reactions::*;

pub(super) fn build_outgoing_send_plan(
    translated_or_original: String,
    original_text: &str,
    was_translated: bool,
    send_original_follow_up: bool,
) -> Vec<String> {
    let mut messages = vec![translated_or_original];
    if was_translated && send_original_follow_up {
        messages.push(original_text.to_string());
    }
    messages
}

pub(super) async fn send_message(
    State(state): State<Arc<AppState>>,
    Json(mut req): Json<SendMessageRequest>,
) -> impl IntoResponse {
    // Validate input
    if req.contact_id.is_empty() || req.text.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "contact_id and text are required"
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
            Json(serde_json::json!({"error":error})),
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

    let settings = state
        .store
        .get_conversation_settings(&req.contact_id)
        .unwrap_or_default();

    // Determine the target before checking translator availability. If a target exists,
    // translation is necessary and every failure must stop before WhatsApp is touched.
    let target_lang =
        match crate::incoming::outgoing_language(&state, &req.contact_id, req.reply_to.as_deref())
            .await
        {
            Ok(language) => language,
            Err(error) => {
                return (
                    StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({"error": error.to_string()})),
                )
                    .into_response()
            }
        };
    let (text_to_send, _original_text, was_translated, target_language) = if let Some(conv_lang) =
        target_lang
    {
        let Some(translator) = state.translator() else {
            error!("Translation to {conv_lang} is needed, but no translator is configured");
            return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(serde_json::json!({
                        "error": format!(
                            "Translation to {} is required, but translation is unavailable. The English message was not sent.",
                            conv_lang
                        )
                    })),
                )
                    .into_response();
        };
        info!(
            target_language = %conv_lang,
            has_override = settings.language_override.is_some(),
            "Resolved outgoing target language"
        );

        match translator.translate_outgoing(&req.text, &conv_lang).await {
            Ok((translated, usage)) if !translated.trim().is_empty() => {
                if usage.input_tokens > 0 {
                    if let Err(e) = state.store.record_usage(
                        Some(&req.contact_id),
                        None,
                        &usage,
                        "translate_outgoing",
                    ) {
                        warn!("Failed to record usage: {}", e);
                    }
                }

                if translated != req.text {
                    info!(
                        "Translated outgoing message to {} (cost: ${:.6})",
                        conv_lang, usage.cost_usd
                    );
                    (translated, Some(req.text.clone()), true, Some(conv_lang))
                } else {
                    (req.text.clone(), None, false, None)
                }
            }
            Ok((_translated, _usage)) => {
                error!("Translation to {} returned empty text", conv_lang);
                return (
                    StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({
                        "error": format!(
                            "Translation to {} returned no text. The English message was not sent.",
                            conv_lang
                        )
                    })),
                )
                    .into_response();
            }
            Err(e) => {
                error!("Failed to translate outgoing message: {}", e);
                return (
                    StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({
                        "error": format!(
                            "Translation to {} failed. The English message was not sent.",
                            conv_lang
                        )
                    })),
                )
                    .into_response();
            }
        }
    } else {
        (req.text.clone(), None, false, None)
    };

    let send_plan = build_outgoing_send_plan(
        text_to_send,
        &req.text,
        was_translated,
        settings.send_original_follow_up,
    );
    let text_to_send = send_plan[0].clone();

    // Preserve the quote in reply-only chats, including replies to the latest message.
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
                Json(serde_json::json!({"error":error})),
            )
                .into_response()
        }
    }

    let request_id = state.next_request_id();
    let timestamp = chrono::Utc::now().timestamp_millis();
    let temp_message_id = format!("pending_{}_{}", request_id, timestamp);

    // Store the sent message locally
    // For outgoing translated messages:
    // - content.body = what user typed (English) - THIS IS DISPLAYED
    // - original_text = what user typed (English) - same as content for consistency
    // - translated_text = what was actually sent (foreign language) - SHOWN IN TOOLTIP
    // - source_language = the language we translated TO (e.g., "French")
    // Get contact info for the recipient
    let contact_info = state.store.get_contact(&req.contact_id).ok().flatten();
    let contact_name = contact_info.as_ref().and_then(|c| c.name.clone());
    let contact_phone = contact_info.as_ref().and_then(|c| c.phone.clone());
    let chat_type = contact_info
        .as_ref()
        .and_then(|c| c.contact_type.clone())
        .unwrap_or_else(|| "private".to_string());

    let stored_msg = StoredMessage {
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
        content_type: "Text".to_string(),
        // Store English (what user typed) as the content for display
        content_json: serde_json::json!({
            "type": "text",
            "body": req.text.clone(),
            "showTranslatedPrimary": was_translated && settings.send_original_follow_up,
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
            "type": "text",
            "body": req.text.clone(),
            "showTranslatedPrimary": was_translated && settings.send_original_follow_up,
            "reply_context": req.reply_to.as_ref().map(|reply_to| serde_json::json!({
                "messageId": reply_to,
                "senderName": req.reply_to_sender_name.clone().unwrap_or_else(|| {
                    req.reply_to_sender.clone().unwrap_or_else(|| "Unknown".to_string())
                }),
                "text": req.reply_to_text.clone().unwrap_or_default()
            }))
        })),
        original_text: if was_translated {
            Some(req.text.clone())
        } else {
            None
        },
        // Store the translated text (what was actually sent) for the tooltip
        translated_text: if was_translated {
            Some(text_to_send.clone())
        } else {
            None
        },
        source_language: target_language.clone(), // The language we translated TO
        is_translated: was_translated,
        delivery_status: Some("sending".to_string()),
    };

    if let Err(e) = state.store.add_message(&stored_msg) {
        error!("Failed to store sent message: {}", e);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Failed to store pending message: {}", e)
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

    let cmd = BridgeCommand::Send {
        request_id: Some(request_id),
        to: req.contact_id.clone(),
        text: text_to_send.clone(),
        reply_to: req.reply_to.clone(),
        reply_to_sender: req.reply_to_sender.clone(),
        reply_to_text: req.reply_to_text.clone(),
    };

    if let Err(e) = state.send_bridge_command(cmd).await {
        error!("Failed to send message: {}", e);
        state.cancel_pending_send(request_id).await;
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Failed to send message: {}", e)
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
                    "error": "Timed out waiting for WhatsApp send confirmation"
                })),
            )
                .into_response();
        }
        Err(SendConfirmationError::ChannelClosed) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({
                    "error": "WhatsApp send confirmation channel closed"
                })),
            )
                .into_response();
        }
    };

    if !send_result.success {
        return (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({
                "error": send_result.error.unwrap_or_else(|| "WhatsApp rejected the message".to_string())
            })),
        )
            .into_response();
    }

    let confirmed_message_id = send_result.message_id.unwrap_or(temp_message_id);
    let confirmed_timestamp = send_result.timestamp.unwrap_or(timestamp);

    let (
        original_follow_up_sent,
        original_message_id,
        original_timestamp,
        original_follow_up_error,
    ) = if let Some(original_text) = send_plan.get(1) {
        match send_original_follow_up(&state, &req, &stored_msg, original_text).await {
            Ok((message_id, timestamp)) => (true, Some(message_id), Some(timestamp), None),
            Err(error) => {
                warn!(
                    "Translated message was sent, but the original follow-up failed: {}",
                    crate::log_privacy::redact_log_text(&error.to_string())
                );
                (false, None, None, Some(error))
            }
        }
    } else {
        (false, None, None, None)
    };

    Json(SendMessageResponse {
        message_id: confirmed_message_id,
        timestamp: confirmed_timestamp,
        is_translated: was_translated,
        translated_text: if was_translated {
            Some(text_to_send)
        } else {
            None
        },
        source_language: target_language,
        original_follow_up_sent,
        original_message_id,
        original_timestamp,
        original_follow_up_error,
    })
    .into_response()
}
