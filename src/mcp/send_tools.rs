use super::*;

impl WhatsAppMcpServer {
    pub(super) async fn handle_prepare_message(
        &self,
        args: Value,
    ) -> Result<CallToolResult, McpError> {
        self.require_read()?;
        let contact_id = crate::identity::canonical_chat_id(required_string(&args, "contact_id")?);
        let contact_id = contact_id.as_ref();
        let original_text = required_string(&args, "text")?.trim();
        if original_text.is_empty() {
            return Err(McpError::invalid_params("text must not be empty", None));
        }
        self.state
            .store
            .validate_reply_only(
                contact_id,
                args.get("reply_to_message_id").and_then(Value::as_str),
            )
            .mcp()?;
        let mode = TranslationMode::parse(args.get("translation_mode").and_then(Value::as_str))?;
        let contact = self
            .state
            .store
            .get_contact(contact_id)
            .mcp()?
            .ok_or_else(|| McpError::invalid_params("Unknown contact_id", None))?;
        let settings = self
            .state
            .store
            .get_conversation_settings(contact_id)
            .mcp()?;
        let target_language = if mode == TranslationMode::Never || !settings.translation_enabled {
            None
        } else if let Some(target) = args
            .get("target_language")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(target.to_string())
        } else {
            crate::incoming::outgoing_language(
                &self.state,
                contact_id,
                args.get("reply_to_message_id").and_then(Value::as_str),
            )
            .await
            .mcp()?
        };

        let translation_result = if let Some(target) = target_language.as_deref() {
            let translator = self.state.translator().ok_or_else(|| {
                McpError::internal_error(
                    format!(
                        "Translation to {target} is needed, but translation is not configured. No message was sent or prepared."
                    ),
                    None,
                )
            })?;
            match translator.translate_to(original_text, target).await {
                Ok((translated, usage)) => {
                    if usage.input_tokens > 0 {
                        if let Err(error) = self.state.store.record_usage(
                            Some(contact_id),
                            None,
                            &usage,
                            "prepare_outgoing_mcp",
                        ) {
                            warn!("Failed to record MCP preparation usage: {error}");
                        }
                    }
                    Ok(translated)
                }
                Err(error) => Err(error.to_string()),
            }
        } else {
            Ok(original_text.to_string())
        };
        let (final_text, translated, target_language) = resolve_prepared_translation(
            original_text,
            target_language.as_deref(),
            mode,
            translation_result,
        )
        .map_err(|error| McpError::internal_error(error, None))?;

        let reply_to_message_id = args
            .get("reply_to_message_id")
            .and_then(Value::as_str)
            .map(str::to_string);
        let (reply_to_sender, reply_to_text) = if let Some(message_id) = &reply_to_message_id {
            let message = self
                .state
                .store
                .get_message_by_id(message_id)
                .mcp()?
                .ok_or_else(|| McpError::invalid_params("Unknown reply_to_message_id", None))?;
            if message.contact_id != contact_id {
                return Err(McpError::invalid_params(
                    "reply_to_message_id belongs to another conversation",
                    None,
                ));
            }
            let info = MessageInfo::from(message.clone());
            (message.sender_phone, info.text)
        } else {
            (None, None)
        };
        let now = chrono::Utc::now().timestamp_millis();
        let token = uuid::Uuid::new_v4().to_string();
        let prepared = PreparedMcpMessage {
            token: token.clone(),
            contact_id: contact_id.to_string(),
            contact_name: contact.name.clone(),
            original_text: original_text.to_string(),
            final_text: final_text.clone(),
            translated,
            target_language: target_language.clone(),
            translation_mode: mode.as_str().to_string(),
            reply_to_message_id: reply_to_message_id.clone(),
            reply_to_sender,
            reply_to_text,
            created_at: now,
            expires_at: now + PREPARATION_TTL_MILLIS,
        };
        let prepared_value = serde_json::to_value(&prepared)
            .map_err(|error| McpError::internal_error(error.to_string(), None))?;
        let mut preparations = self.state.mcp_prepared_messages.write().await;
        preparations.retain(|_, value| {
            value
                .get("expiresAt")
                .and_then(Value::as_i64)
                .is_some_and(|expires_at| expires_at > now)
        });
        preparations.insert(token.clone(), prepared_value);

        Ok(CallToolResult::structured(json!({
            "preparationToken": token,
            "expiresAt": prepared.expires_at,
            "recipient": {"id":contact.id,"name":contact.name,"phone":contact.phone,"type":contact.contact_type},
            "originalText": prepared.original_text,
            "finalText": final_text,
            "translated": translated,
            "targetLanguage": target_language,
            "translationMode": prepared.translation_mode,
            "replyToMessageId": reply_to_message_id,
        })))
    }

    pub(super) async fn handle_prepared_send(
        &self,
        args: Value,
        require_reply: bool,
    ) -> Result<CallToolResult, McpError> {
        self.require_send()?;
        let preparation_token = required_string(&args, "preparation_token")?;
        let idempotency_key = required_string(&args, "idempotency_key")?;
        if idempotency_key.len() < 8 || idempotency_key.len() > 200 {
            return Err(McpError::invalid_params(
                "idempotency_key must contain 8 to 200 characters",
                None,
            ));
        }
        let operation_key = format!("send:{preparation_token}");
        if let Some(existing) = self
            .idempotency_result(idempotency_key, &operation_key)
            .await?
        {
            return Ok(existing);
        }
        let prepared_value = self
            .state
            .mcp_prepared_messages
            .read()
            .await
            .get(preparation_token)
            .cloned()
            .ok_or_else(|| {
                McpError::invalid_params(
                    "Unknown, expired, or already-used preparation_token",
                    None,
                )
            })?;
        let prepared: PreparedMcpMessage = serde_json::from_value(prepared_value)
            .map_err(|error| McpError::internal_error(error.to_string(), None))?;
        if prepared.expires_at <= chrono::Utc::now().timestamp_millis() {
            return Err(McpError::invalid_params(
                "preparation_token has expired",
                None,
            ));
        }
        if require_reply && prepared.reply_to_message_id.is_none() {
            return Err(McpError::invalid_params(
                "reply_to_message requires a preparation created with reply_to_message_id",
                None,
            ));
        }
        self.state
            .store
            .validate_reply_only(
                &prepared.contact_id,
                prepared.reply_to_message_id.as_deref(),
            )
            .mcp()?;
        if prepared.translated
            && !self
                .state
                .store
                .get_conversation_settings(&prepared.contact_id)
                .mcp()?
                .translation_enabled
        {
            return Err(McpError::invalid_params(
                "Translation is now off for this conversation. Prepare the message again before sending.",
                None,
            ));
        }
        if !self
            .begin_idempotent_operation(idempotency_key, &operation_key)
            .await
        {
            return self
                .idempotency_result(idempotency_key, &operation_key)
                .await?
                .ok_or_else(|| McpError::internal_error("Idempotency state changed", None));
        }
        self.state
            .mcp_prepared_messages
            .write()
            .await
            .remove(preparation_token);
        match self.send_prepared_message(&prepared).await {
            Ok(result) => {
                self.finish_idempotent_operation(idempotency_key, "completed", result.clone())
                    .await;
                Ok(CallToolResult::structured(result))
            }
            Err(error) => {
                let result = json!({"success":false,"error":error.to_string()});
                self.finish_idempotent_operation(idempotency_key, "failed", result)
                    .await;
                Err(error)
            }
        }
    }

    pub(super) async fn send_prepared_message(
        &self,
        prepared: &PreparedMcpMessage,
    ) -> Result<Value, McpError> {
        if !*self.state.connected.read().await {
            return Err(McpError::internal_error("Not connected to WhatsApp", None));
        }
        let contact = self
            .state
            .store
            .get_contact(&prepared.contact_id)
            .mcp()?
            .ok_or_else(|| McpError::invalid_params("Recipient no longer exists", None))?;
        let request_id = self.state.next_request_id();
        let timestamp = chrono::Utc::now().timestamp_millis();
        let temp_message_id = format!("mcp_pending_{request_id}_{timestamp}");
        let reply_context = prepared.reply_to_message_id.as_ref().map(|message_id| {
            json!({
                "messageId": message_id,
                "senderName": prepared.reply_to_sender.clone().unwrap_or_else(|| "Unknown".to_string()),
                "text": prepared.reply_to_text.clone().unwrap_or_default(),
            })
        });
        let content = json!({
            "type":"text",
            "body":prepared.original_text,
            "reply_context":reply_context,
        });
        let stored_message = StoredMessage {
            id: temp_message_id.clone(),
            contact_id: prepared.contact_id.clone(),
            timestamp,
            is_from_me: true,
            is_forwarded: false,
            sender_name: self.state.name.read().await.clone(),
            sender_phone: self.state.phone.read().await.clone(),
            contact_name: contact.name.clone(),
            contact_phone: contact.phone.clone(),
            chat_type: contact
                .contact_type
                .clone()
                .unwrap_or_else(|| "private".to_string()),
            content_type: "Text".to_string(),
            content_json: content.to_string(),
            content: Some(content),
            original_text: prepared.translated.then(|| prepared.original_text.clone()),
            translated_text: prepared.translated.then(|| prepared.final_text.clone()),
            source_language: prepared.target_language.clone(),
            is_translated: prepared.translated,
            delivery_status: Some("sent".to_string()),
        };
        self.state
            .store
            .add_message(&stored_message)
            .map_err(|error| McpError::internal_error(error.to_string(), None))?;
        if let Err(error) = self.state.store.upsert_contact(
            &prepared.contact_id,
            contact.name.as_deref(),
            contact.phone.as_deref(),
            contact.contact_type.as_deref(),
            timestamp,
        ) {
            let _ = self.state.store.delete_message(&temp_message_id);
            return Err(McpError::internal_error(error.to_string(), None));
        }
        let receiver = self
            .state
            .register_pending_send(request_id, Some(temp_message_id.clone()))
            .await;
        let command = BridgeCommand::Send {
            request_id: Some(request_id),
            to: prepared.contact_id.clone(),
            text: prepared.final_text.clone(),
            reply_to: prepared.reply_to_message_id.clone(),
            reply_to_sender: prepared.reply_to_sender.clone(),
            reply_to_text: prepared.reply_to_text.clone(),
        };
        if let Err(error) = self.state.send_bridge_command(command).await {
            self.state.cancel_pending_send(request_id).await;
            let _ = self.state.store.delete_message(&temp_message_id);
            return Err(McpError::internal_error(
                format!("Failed to send message: {error}"),
                None,
            ));
        }
        let send_result = self
            .state
            .wait_for_send_result(request_id, receiver)
            .await
            .map_err(|error| match error {
                SendConfirmationError::Timeout => McpError::internal_error(
                    "Timed out waiting for WhatsApp confirmation; the idempotency key is locked to prevent a duplicate send",
                    None,
                ),
                SendConfirmationError::ChannelClosed => McpError::internal_error(
                    "WhatsApp confirmation channel closed; the idempotency key is locked to prevent a duplicate send",
                    None,
                ),
            })?;
        if !send_result.success {
            return Err(McpError::internal_error(
                send_result
                    .error
                    .unwrap_or_else(|| "WhatsApp rejected the message".to_string()),
                None,
            ));
        }
        let message_id = send_result.message_id.unwrap_or(temp_message_id);
        let timestamp = send_result.timestamp.unwrap_or(timestamp);
        info!("MCP sent prepared message");
        Ok(json!({
            "success":true,
            "messageId":message_id,
            "timestamp":timestamp,
            "timestampIso":timestamp_millis_to_iso(timestamp),
            "recipient":{"id":prepared.contact_id,"name":prepared.contact_name},
            "sentText":prepared.final_text,
            "translated":prepared.translated,
            "targetLanguage":prepared.target_language,
            "replyToMessageId":prepared.reply_to_message_id,
        }))
    }
}
