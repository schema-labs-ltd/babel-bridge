//! Bridge events in web mode: store them and broadcast them to clients.

use super::*;

/// Handle events in web mode
pub(crate) async fn handle_web_event(
    event: BridgeEvent,
    state: &Arc<AppState>,
    store: &MessageStore,
    translator: Option<&Arc<TranslationService>>,
) -> Result<()> {
    match event {
        BridgeEvent::Qr { data } => {
            debug!("Received QR code data");
            state.set_qr_code(data).await;
        }

        BridgeEvent::Connected { phone, name, .. } => {
            info!("WhatsApp session connected");
            state.set_connected(true, Some(phone), Some(name)).await;
        }

        BridgeEvent::ConnectionState { state: conn_state } => match conn_state {
            ConnectionState::Reconnecting | ConnectionState::Connecting => {
                state.set_connected(false, None, None).await;
            }
            ConnectionState::Disconnected => {
                state.mark_whatsapp_disconnected().await;
            }
            ConnectionState::LoggedOut => {
                state.require_whatsapp_link().await;
                state.request_session_reset_before_bridge_restart();
                if let Err(e) = state
                    .send_bridge_command(bridge::BridgeCommand::Disconnect)
                    .await
                {
                    warn!(
                        "Failed to request bridge restart after logged-out state: {}",
                        e
                    );
                }
            }
            _ => {}
        },

        BridgeEvent::MessageEdit {
            message,
            edited_at_ms,
        } => {
            let edit = process_message(message, None, Some(store)).await;
            if let Some(message) = store.record_message_edit(&edit, edited_at_ms)? {
                let _ = state
                    .broadcast_tx
                    .send(web::WebSocketEvent::MessageUpdated { message });
            }
        }

        BridgeEvent::Message(msg) => {
            // Extract unread count before moving msg
            let unread_count = msg.unread_count;
            let is_history = msg.is_history;
            let participant_count = match &msg.chat {
                bridge::Chat::Group {
                    participant_count, ..
                } => *participant_count,
                _ => None,
            };
            let already_stored = store.get_message_by_id(&msg.id)?.is_some();

            // Process and store the message
            let stored_msg = process_message(msg, None, Some(store)).await;

            // Update contact with contact_name (not sender_name!)
            // contact_name is the chat name (other person for DMs, group name for groups)
            // sender_name changes based on who sent the message
            store.upsert_contact(
                &stored_msg.contact_id,
                stored_msg.contact_name.as_deref(),
                stored_msg.contact_phone.as_deref(),
                Some(&stored_msg.chat_type),
                stored_msg.timestamp,
            )?;

            if !is_history {
                if let Some(count) = participant_count {
                    store.set_group_participant_count(&stored_msg.contact_id, count)?;
                }
            }

            // Store message
            if !already_stored {
                let notification = (!stored_msg.is_from_me && !is_history)
                    .then_some(translator.is_some() && stored_msg.original_text.is_some());
                store.add_message_with_notification(&stored_msg, notification)?;
            } else {
                store.recover_reply_context(&stored_msg)?;
            }

            // Handle unread counts
            if let Some(unread) = unread_count {
                // History sync message with unread count from WhatsApp - use it directly
                store.set_unread_count(&stored_msg.contact_id, unread)?;
            }

            if !stored_msg.is_from_me
                && !is_history
                && translator.is_some()
                && stored_msg.original_text.is_some()
            {
                store.enqueue_translation(&stored_msg.id)?;
            }

            if !stored_msg.is_from_me && !is_history && stored_msg.is_audio() {
                voice::queue_incoming(state.clone(), stored_msg.id.clone());
            }

            // Broadcast to WebSocket clients
            if already_stored {
                if let Some(message) = store.get_message_by_id(&stored_msg.id)? {
                    let _ = state
                        .broadcast_tx
                        .send(web::WebSocketEvent::MessageUpdated { message });
                }
            } else {
                state.broadcast_message(
                    store
                        .get_message_by_id(&stored_msg.id)?
                        .unwrap_or(stored_msg),
                );
            }
        }

        BridgeEvent::Error { code, message } => {
            error!(
                "Bridge error [{code}]: {}",
                log_privacy::redact_log_text(&message)
            );
            if code == "reconnect_required" {
                state.set_connected(false, None, None).await;
                let _ = state
                    .send_bridge_command(bridge::BridgeCommand::Disconnect)
                    .await;
            }
        }

        BridgeEvent::Log { level, message } => record_bridge_log(&level, &message),

        BridgeEvent::LoggedOut { reason } => {
            warn!("Logged out: {}", log_privacy::redact_log_text(&reason));
            state.require_whatsapp_link().await;
            state.request_session_reset_before_bridge_restart();
            if let Err(e) = state
                .send_bridge_command(bridge::BridgeCommand::Disconnect)
                .await
            {
                warn!("Failed to request bridge restart after logout event: {}", e);
            }
        }

        BridgeEvent::SendResult {
            request_id,
            success,
            message_id,
            timestamp,
            message_ids,
            timestamps,
            error,
        } => {
            state
                .handle_send_result(web::BridgeSendResult {
                    request_id,
                    success,
                    message_id: message_id.clone(),
                    timestamp,
                    message_ids,
                    timestamps,
                    error: error.clone(),
                })
                .await;

            if success {
                debug!(
                    "Message sent successfully: {:?} at {:?}",
                    message_id, timestamp
                );
            } else {
                error!(
                    "Failed to send message (request {request_id}): {}",
                    log_privacy::redact_log_text(&error.unwrap_or_default())
                );
            }
            // TODO: Could broadcast send result to WebSocket clients for UI updates
        }

        BridgeEvent::SendProgress {
            request_id: _,
            progress_id,
            stage,
            completed,
            total,
        } => {
            state.handle_send_progress(progress_id, stage, completed, total);
        }

        BridgeEvent::ProfilePicture {
            request_id,
            jid: _,
            url,
            id: _,
            error,
        } => {
            if let Some(err) = error {
                debug!(
                    "Profile picture error (request {request_id}): {}",
                    log_privacy::redact_log_text(&err)
                );
            }
            // Notify the waiting request
            state.handle_profile_picture_response(request_id, url).await;
        }

        BridgeEvent::ChatPresence {
            chat_id,
            user_id,
            state: presence_state,
        } => {
            let state_str = match presence_state {
                bridge::ChatPresenceState::Typing => "typing",
                bridge::ChatPresenceState::Paused => "paused",
                bridge::ChatPresenceState::Recording => "recording",
            };
            info!(state = state_str, "Chat presence");
            // Broadcast to WebSocket clients
            state.broadcast_typing(chat_id, user_id, state_str.to_string());
        }

        BridgeEvent::ReadReceiptResult {
            message_id,
            success,
            error,
        } => {
            if success {
                store.read_receipt_sent(&message_id, chrono::Utc::now().timestamp())?;
            } else {
                warn!(
                    "WhatsApp read receipt failed for {message_id}: {}",
                    log_privacy::redact_log_text(&error.unwrap_or_default())
                );
            }
        }

        BridgeEvent::MarkAsRead { chat_id } => {
            // Chat was marked as read from another device (e.g., user's phone)
            info!("Chat marked as read from another device");
            store.mark_as_read(&chat_id)?;
            // Broadcast to WebSocket clients so UI updates
            state.broadcast_mark_as_read(chat_id);
        }

        BridgeEvent::Receipt {
            message_ids,
            status,
        } => {
            store.update_delivery_status(&message_ids, &status)?;
            state.broadcast_receipt(message_ids, status);
        }
    }

    Ok(())
}

/// Process a message, translating if necessary
pub(crate) async fn process_message(
    msg: Message,
    translator: Option<&Arc<TranslationService>>,
    store: Option<&storage::MessageStore>,
) -> StoredMessage {
    let chat_type = match &msg.chat {
        bridge::Chat::Private { .. } => "private",
        bridge::Chat::Group { .. } => "group",
        bridge::Chat::Broadcast { .. } => "broadcast",
        bridge::Chat::Status { .. } => "status",
    };

    let canonical_private_phone = if chat_type == "private" {
        phone_from_chat_jid(msg.chat.jid())
    } else {
        None
    };

    let contact_id = if chat_type == "private" {
        canonical_private_phone
            .as_ref()
            .map(|phone| format!("{}@s.whatsapp.net", phone))
            .unwrap_or_else(|| identity::canonical_chat_id(msg.chat.jid()).into_owned())
    } else {
        msg.chat.jid().to_string()
    };

    // Get conversation settings if we have a store
    let settings = store
        .and_then(|s| s.get_conversation_settings(&contact_id).ok())
        .unwrap_or_default();

    // Extract text content for translation (skip history messages)
    let (original_text, translated_text, source_language, is_translated) =
        if let Some(translator) = translator {
            if let Some(text) = extract_text_content(&msg.content) {
                if !msg.is_from_me && !msg.is_history && settings.translation_enabled {
                    // Only translate incoming messages (not history sync)
                    match translator
                        .process_text(
                            &text,
                            settings.language_override.as_deref(),
                            settings.translation_style.as_deref(),
                        )
                        .await
                    {
                        Ok(result) => {
                            // Record usage if we have a store and there was actual API usage
                            if let Some(store) = store {
                                if result.usage.input_tokens > 0 {
                                    if let Err(e) = store.record_usage(
                                        Some(&contact_id),
                                        Some(&msg.id),
                                        &result.usage,
                                        if result.needs_translation {
                                            "translate_incoming"
                                        } else {
                                            "detect_language"
                                        },
                                    ) {
                                        tracing::warn!("Failed to record usage: {}", e);
                                    }
                                }
                            }

                            (
                                Some(result.original_text),
                                result.translated_text,
                                Some(result.source_language),
                                result.needs_translation,
                            )
                        }
                        Err(e) => {
                            tracing::warn!(
                                "Failed to translate incoming message {}: {}",
                                msg.id,
                                log_privacy::redact_log_text(&e.to_string())
                            );
                            (Some(text), None, None, false)
                        }
                    }
                } else {
                    (Some(text), None, None, false)
                }
            } else {
                (None, None, None, false)
            }
        } else {
            (extract_text_content(&msg.content), None, None, false)
        };

    // Serialize content to JSON
    let mut payload = serde_json::to_value(&msg.content).unwrap_or_default();
    if let Some(quote) = &msg.reply_context {
        payload["reply_context"] = serde_json::to_value(quote).unwrap_or_default();
    }
    // Keep the full sender identity, including LID addresses, for group read receipts.
    if !msg.is_from_me && !msg.from.jid.is_empty() {
        payload["receipt_sender_jid"] = serde_json::Value::String(msg.from.jid.clone());
    }
    let content_json = payload.to_string();
    let content = Some(payload);
    let content_type = msg.content.type_name().to_string();

    // Get contact name and phone from chat info
    // For private chats: this is the other person
    // For groups: this is the group name
    let (contact_name, contact_phone) = match &msg.chat {
        bridge::Chat::Private { name, jid } => {
            let phone = canonical_private_phone
                .clone()
                .or_else(|| phone_from_chat_jid(jid));
            (name.clone(), phone)
        }
        bridge::Chat::Group { name, .. } => (name.clone(), None),
        bridge::Chat::Broadcast { jid } => {
            let phone = phone_from_chat_jid(jid);
            (
                Some(format!(
                    "Broadcast: {}",
                    phone.as_deref().unwrap_or("Unknown")
                )),
                phone,
            )
        }
        bridge::Chat::Status { .. } => (Some("Status".to_string()), None),
    };

    StoredMessage {
        id: msg.id,
        contact_id,
        timestamp: match &msg.content {
            MessageContent::Reaction {
                sender_timestamp_ms: Some(timestamp),
                ..
            } if *timestamp > 0 => *timestamp,
            _ => msg.timestamp.timestamp_millis(),
        },
        is_from_me: msg.is_from_me,
        is_forwarded: msg.is_forwarded,
        sender_name: msg.push_name.or_else(|| msg.from.name.clone()),
        sender_phone: Some(msg.from.phone),
        contact_name,
        contact_phone,
        chat_type: chat_type.to_string(),
        content_type,
        content_json,
        content,
        original_text,
        translated_text,
        source_language,
        is_translated,
        delivery_status: msg.is_from_me.then(|| "sent".to_string()),
    }
}

pub(crate) fn phone_from_chat_jid(jid: &str) -> Option<String> {
    let (user, server) = jid.split_once('@')?;
    match server {
        "s.whatsapp.net" | "c.us" => identity::phone_number(jid),
        "broadcast" => Some(user.to_string()),
        _ => None,
    }
}

/// Extract text content from a message
pub(crate) fn extract_text_content(content: &MessageContent) -> Option<String> {
    match content {
        MessageContent::Text { body } => Some(body.clone()),
        MessageContent::Image {
            caption: Some(c), ..
        } => Some(c.clone()),
        MessageContent::Video {
            caption: Some(c), ..
        } => Some(c.clone()),
        MessageContent::Document {
            caption: Some(c), ..
        } => Some(c.clone()),
        _ => None,
    }
}
