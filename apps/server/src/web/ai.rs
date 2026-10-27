//! Manual translation, AI compose and AI reply handlers.

use super::*;

/// Translate a message manually
pub(crate) async fn translate_message(
    State(state): State<Arc<AppState>>,
    Json(req): Json<TranslateMessageRequest>,
) -> impl IntoResponse {
    // Check if translation service is available
    let translator = match state.translator() {
        Some(t) => t,
        None => {
            return Json(TranslateMessageResponse {
                success: false,
                translated_text: None,
                source_language: None,
                error: Some("Translation service not configured".to_string()),
            })
            .into_response();
        }
    };
    let message = match state.store.get_message_by_id(&req.message_id) {
        Ok(Some(message)) if message.contact_id == req.contact_id && !message.is_from_me => message,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({"error": "Incoming message not found in this conversation"}),
                ),
            )
                .into_response()
        }
    };
    let Some(text) = message.original_text.as_deref() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Message has no text or caption"})),
        )
            .into_response();
    };
    // Get conversation settings for this contact
    let settings = state
        .store
        .get_conversation_settings(&req.contact_id)
        .unwrap_or_default();

    // Call the translation service with conversation settings
    if !settings.translation_enabled {
        return (StatusCode::CONFLICT, Json(serde_json::json!({"error":"Translation is off for this conversation. Enable it in Conversation settings."}))).into_response();
    }
    let result = match translator
        .process_text(
            text,
            settings.language_override.as_deref(),
            settings.translation_style.as_deref(),
        )
        .await
    {
        Ok(result) => result,
        Err(e) => {
            warn!("Manual translation failed for {}: {}", req.message_id, e);
            return (
                StatusCode::BAD_GATEWAY,
                Json(TranslateMessageResponse {
                    success: false,
                    translated_text: None,
                    source_language: None,
                    error: Some(format!("Translation failed: {:#}", e)),
                }),
            )
                .into_response();
        }
    };

    // Record usage if there was API usage
    if result.usage.input_tokens > 0 {
        if let Err(e) = state.store.record_usage(
            Some(&req.contact_id),
            Some(&req.message_id),
            &result.usage,
            if result.needs_translation {
                "manual_translate"
            } else {
                "detect_language"
            },
        ) {
            warn!("Failed to record translation usage: {}", e);
        }
    }

    match state.store.finish_translation(
        &req.message_id,
        result.translated_text.as_deref(),
        &result.source_language,
        result.needs_translation,
        message.original_text.as_deref(),
        message.edit_revision(),
    ) {
        Ok(true) => {},
        Ok(false) => return (StatusCode::CONFLICT, Json(serde_json::json!({"error":"The message changed during translation. Please try again."}))).into_response(),
        Err(error) => {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Could not save translation: {error}")})),
        )
            .into_response();
        }
    }
    if let Ok(Some(message)) = state.store.get_message_by_id(&req.message_id) {
        let _ = state
            .broadcast_tx
            .send(WebSocketEvent::MessageUpdated { message });
    }
    Json(serde_json::json!({
        "success": true, "translatedText": result.translated_text,
        "sourceLanguage": result.source_language, "needsTranslation": result.needs_translation,
        "error": null,
    }))
    .into_response()
}

/// AI compose endpoint - generates a message using OpenAI
pub(super) async fn ai_compose(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AiComposeRequest>,
) -> impl IntoResponse {
    // Check if translation service is available (it has the API key)
    let translator = match state.translator() {
        Some(t) => t,
        None => {
            return Json(AiComposeResponse {
                success: false,
                message: None,
                error: Some("AI service not configured (missing API key)".to_string()),
                cost_usd: None,
            })
            .into_response();
        }
    };

    // Build reply context if provided
    let reply_context = match (&req.reply_to_sender, &req.reply_to_text) {
        (Some(sender), Some(text)) => Some((sender.as_str(), text.as_str())),
        (None, Some(text)) => Some(("Someone", text.as_str())),
        _ => None,
    };

    // Build image context if provided
    let reply_image = match (&req.reply_to_image_type, &req.reply_to_image) {
        (Some(mime_type), Some(data)) => Some((mime_type.as_str(), data.as_str())),
        _ => None,
    };

    // Call the AI compose method
    match translator
        .compose_ai_message(&req.prompt, reply_context, reply_image)
        .await
    {
        Ok((message, usage)) => {
            info!(
                "AI composed message ({} chars), cost: ${:.6}",
                message.len(),
                usage.cost_usd
            );

            // Record usage
            if let Err(e) = state.store.record_usage(
                None,
                None,
                &crate::translation::UsageInfo {
                    input_tokens: usage.input_tokens,
                    cached_input_tokens: usage.cached_input_tokens,
                    output_tokens: usage.output_tokens,
                    cost_usd: usage.cost_usd,
                },
                "ai_compose",
            ) {
                warn!("Failed to record AI compose usage: {}", e);
            }

            Json(AiComposeResponse {
                success: true,
                message: Some(message),
                error: None,
                cost_usd: Some(usage.cost_usd),
            })
            .into_response()
        }
        Err(e) => {
            error!("AI compose failed: {}", e);
            Json(AiComposeResponse {
                success: false,
                message: None,
                error: Some(format!("Failed to compose message: {}", e)),
                cost_usd: None,
            })
            .into_response()
        }
    }
}

/// AI styled reply endpoint - generates a reply that sounds like the user
pub(super) async fn ai_reply(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AiReplyRequest>,
) -> impl IntoResponse {
    // Check if translation service is available (it has the API key)
    let translator = match state.translator() {
        Some(t) => t,
        None => {
            return Json(AiReplyResponse {
                success: false,
                reply_text: None,
                error: Some("AI service not configured (missing API key)".to_string()),
                cost_usd: None,
            })
            .into_response();
        }
    };

    // Get the message being replied to
    let message = match state.store.get_message_by_id(&req.message_id) {
        Ok(Some(m)) => m,
        Ok(None) => {
            return Json(AiReplyResponse {
                success: false,
                reply_text: None,
                error: Some("Message not found".to_string()),
                cost_usd: None,
            })
            .into_response();
        }
        Err(e) => {
            error!("Failed to get message: {}", e);
            return Json(AiReplyResponse {
                success: false,
                reply_text: None,
                error: Some(format!("Failed to get message: {}", e)),
                cost_usd: None,
            })
            .into_response();
        }
    };

    // Get recent conversation for context (last 20 messages)
    let recent_conversation = match state.store.get_recent_messages(&req.contact_id, 20) {
        Ok(msgs) => msgs,
        Err(e) => {
            warn!("Failed to get recent messages: {}", e);
            vec![]
        }
    };

    // Create style analyzer
    let detection_model = translator.get_detection_model();
    let style_analyzer = crate::style_analyzer::StyleAnalyzer::new(
        translator.clone(),
        detection_model,
        translator.get_reasoning_effort(),
    );

    // Get or create global style profile
    let (global_style, global_usage) = match style_analyzer
        .get_or_create_profile(&state.store, None)
        .await
    {
        Ok(result) => result,
        Err(e) => {
            error!("Failed to get global style profile: {}", e);
            return Json(AiReplyResponse {
                success: false,
                reply_text: None,
                error: Some(format!("Failed to analyze writing style: {}", e)),
                cost_usd: None,
            })
            .into_response();
        }
    };

    // Get or create per-contact style profile
    let (contact_style, contact_usage) = match style_analyzer
        .get_or_create_profile(&state.store, Some(&req.contact_id))
        .await
    {
        Ok(result) => result,
        Err(e) => {
            warn!("Failed to get contact style profile: {}", e);
            // Continue without contact-specific style
            (
                crate::storage::StyleProfile {
                    contact_id: req.contact_id.clone(),
                    profile_text: "No contact-specific style data available.".to_string(),
                    sample_messages: vec![],
                    message_count: 0,
                    updated_at: 0,
                },
                None,
            )
        }
    };

    // Get examples of user's messages to this contact (more examples = better style matching)
    let my_examples = match state
        .store
        .get_outgoing_messages_for_style(Some(&req.contact_id), 20)
    {
        Ok(msgs) => msgs,
        Err(e) => {
            warn!("Failed to get example messages: {}", e);
            vec![]
        }
    };

    // Generate the styled reply
    match translator
        .compose_styled_reply(
            &message,
            &recent_conversation,
            &global_style,
            Some(&contact_style),
            &my_examples,
        )
        .await
    {
        Ok((reply_text, usage)) => {
            // Calculate total cost (including any style analysis)
            let mut total_input_tokens = usage.input_tokens as i64;
            let mut total_cached_input_tokens = usage.cached_input_tokens as i64;
            let mut total_output_tokens = usage.output_tokens as i64;
            let mut total_cost = usage.cost_usd;
            if let Some(gu) = &global_usage {
                total_input_tokens += gu.input_tokens as i64;
                total_cached_input_tokens += gu.cached_input_tokens as i64;
                total_output_tokens += gu.output_tokens as i64;
                total_cost += gu.cost_usd;
            }
            if let Some(cu) = &contact_usage {
                total_input_tokens += cu.input_tokens as i64;
                total_cached_input_tokens += cu.cached_input_tokens as i64;
                total_output_tokens += cu.output_tokens as i64;
                total_cost += cu.cost_usd;
            }

            info!(
                "AI reply generated ({} chars), cost: ${:.6}",
                reply_text.len(),
                total_cost
            );

            // Record usage
            if let Err(e) = state.store.record_usage(
                Some(&req.contact_id),
                Some(&req.message_id),
                &crate::translation::UsageInfo {
                    input_tokens: total_input_tokens as u32,
                    cached_input_tokens: total_cached_input_tokens as u32,
                    output_tokens: total_output_tokens as u32,
                    cost_usd: total_cost,
                },
                "ai_styled_reply",
            ) {
                warn!("Failed to record AI reply usage: {}", e);
            }

            Json(AiReplyResponse {
                success: true,
                reply_text: Some(reply_text),
                error: None,
                cost_usd: Some(total_cost),
            })
            .into_response()
        }
        Err(e) => {
            error!("AI reply generation failed: {}", e);
            Json(AiReplyResponse {
                success: false,
                reply_text: None,
                error: Some(format!("Failed to generate reply: {}", e)),
                cost_usd: None,
            })
            .into_response()
        }
    }
}
