use super::*;

impl WhatsAppMcpServer {
    pub(super) async fn handle_open(
        &self,
        args: Value,
        single_chat: bool,
    ) -> Result<CallToolResult, McpError> {
        self.require_read()?;
        let mut result = if single_chat {
            self.handle_read_messages(args).await?
        } else {
            self.handle_contacts(json!({}), false).await?
        };
        if let Some(payload) = result.structured_content.as_mut() {
            payload["permissions"] =
                json!({"read":self.permissions.read,"send":self.permissions.send});
        }
        Ok(result)
    }

    pub(super) fn chat_resources(&self) -> ListResourcesResult {
        let resources = if self.permissions.read {
            let mut resource = RawResource::new(CHAT_RESOURCE_URI, "WhatsApp conversations");
            resource.mime_type = Some(CHAT_MIME_TYPE.into());
            resource.title = Some("WhatsApp conversations".into());
            vec![rmcp::model::Annotated::new(resource, None)]
        } else {
            vec![]
        };
        ListResourcesResult {
            resources,
            ..Default::default()
        }
    }

    pub(super) fn chat_resource(&self, uri: &str) -> Result<ReadResourceResult, McpError> {
        self.require_read()?;
        if uri != CHAT_RESOURCE_URI {
            return Err(McpError::invalid_params("Unknown UI resource", None));
        }
        Ok(ReadResourceResult { contents: vec![ResourceContents::TextResourceContents {
            uri: CHAT_RESOURCE_URI.into(), mime_type: Some(CHAT_MIME_TYPE.into()),
            text: include_str!("../../../../libs/web-extension/dist/chats.html").into(),
            meta: Some(serde_json::from_value(json!({
                "openai/ui":{"preferredDisplayMode":"fullscreen","availableDisplayModes":["inline","fullscreen"]},
                "ui":{"csp":{"connectDomains":[],"resourceDomains":[]}}
            })).expect("static resource metadata")),
        }] })
    }

    pub(super) fn translate_message_tool() -> Tool {
        Self::tool("translate_message", "Translate incoming WhatsApp message",
            "Translate a stored incoming message using this conversation's opt-in settings. Saves the translation locally and may incur AI usage; sends nothing to WhatsApp. Requires read and send scopes.",
            json!({"type":"object","properties":{"contact_id":{"type":"string"},"message_id":{"type":"string"}},"required":["contact_id","message_id"],"additionalProperties":false}),
            Self::object_output(json!({"success":{"type":"boolean"},"translatedText":{"type":["string","null"]},"sourceLanguage":{"type":"string"}}), &["success"]),
            Self::write_annotations("Translate incoming WhatsApp message", false))
    }

    pub(super) async fn handle_translate_message(
        &self,
        args: Value,
    ) -> Result<CallToolResult, McpError> {
        use axum::response::IntoResponse;
        self.require_read()?;
        self.require_send()?;
        let contact_id =
            crate::identity::canonical_chat_id(required_string(&args, "contact_id")?).into_owned();
        let message_id = required_string(&args, "message_id")?.to_string();
        let message = self
            .state
            .store
            .get_message_by_id(&message_id)
            .mcp()?
            .filter(|message| message.contact_id == contact_id && !message.is_from_me)
            .ok_or_else(|| {
                McpError::invalid_params("Incoming message not found in this conversation", None)
            })?;
        let response = crate::web::translate_message(
            axum::extract::State(self.state.clone()),
            axum::Json(crate::web::TranslateMessageRequest {
                text: message.original_text.unwrap_or_default(),
                message_id,
                contact_id,
            }),
        )
        .await
        .into_response();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .map_err(|error| McpError::internal_error(error.to_string(), None))?;
        let payload: Value = serde_json::from_slice(&bytes)
            .map_err(|error| McpError::internal_error(error.to_string(), None))?;
        if !status.is_success() || payload["success"] == false {
            return Err(McpError::internal_error(
                payload["error"]
                    .as_str()
                    .unwrap_or("Translation failed")
                    .to_string(),
                None,
            ));
        }
        Ok(CallToolResult::structured(payload))
    }

    pub(super) fn require_read(&self) -> Result<(), McpError> {
        if self.permissions.read {
            Ok(())
        } else {
            Err(McpError::invalid_params(
                "This OAuth token does not include whatsapp.read",
                None,
            ))
        }
    }

    pub(super) fn require_send(&self) -> Result<(), McpError> {
        if self.permissions.send {
            Ok(())
        } else {
            Err(McpError::invalid_params(
                "This OAuth token does not include whatsapp.send",
                None,
            ))
        }
    }

    pub(super) async fn handle_get_status(&self) -> Result<CallToolResult, McpError> {
        self.require_read()?;
        let (message_count, contact_count) = self
            .state
            .store
            .get_stats()
            .map_err(|error| McpError::internal_error(error.to_string(), None))?;
        Ok(CallToolResult::structured(json!({
            "connected": *self.state.connected.read().await,
            "connectionState": self.state.connection_state.read().await.label(),
            "accountName": self.state.name.read().await.clone(),
            "accountPhone": self.state.phone.read().await.clone(),
            "translationConfigured": self.state.translator().is_some(),
            "contactCount": contact_count,
            "messageCount": message_count,
            "checkedAt": chrono::Utc::now().to_rfc3339(),
        })))
    }

    pub(super) async fn handle_contacts(
        &self,
        args: Value,
        require_query: bool,
    ) -> Result<CallToolResult, McpError> {
        self.require_read()?;
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_lowercase();
        if require_query && query.is_empty() {
            return Err(McpError::invalid_params("query is required", None));
        }
        let contact_type = args
            .get("contact_type")
            .and_then(Value::as_str)
            .unwrap_or("all");
        let unread_only = args
            .get("unread_only")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let cursor = args.get("cursor").and_then(Value::as_u64).unwrap_or(0) as usize;
        let limit = argument_limit(&args);

        let contacts = self
            .state
            .store
            .get_contacts()
            .map_err(|error| McpError::internal_error(error.to_string(), None))?;
        let filtered: Vec<StoredContact> = contacts
            .into_iter()
            .filter(|contact| {
                (contact_type == "all" || contact.contact_type.as_deref() == Some(contact_type))
                    && (!unread_only || contact.unread_count > 0)
                    && (query.is_empty()
                        || contact.id.to_lowercase().contains(&query)
                        || contact
                            .name
                            .as_deref()
                            .is_some_and(|value| value.to_lowercase().contains(&query))
                        || contact
                            .phone
                            .as_deref()
                            .is_some_and(|value| value.to_lowercase().contains(&query)))
            })
            .collect();
        let total_matched = filtered.len();
        let contacts: Vec<ContactInfo> = filtered
            .into_iter()
            .skip(cursor)
            .take(limit)
            .map(ContactInfo::from)
            .collect();
        let next_cursor =
            (cursor + contacts.len() < total_matched).then_some(cursor + contacts.len());

        Ok(CallToolResult::structured(json!({
            "contacts": contacts,
            "nextCursor": next_cursor,
            "totalMatched": total_matched,
        })))
    }

    pub(super) async fn handle_read_unified_messages(
        &self,
        args: Value,
    ) -> Result<CallToolResult, McpError> {
        self.require_read()?;
        let args = FeedArguments::parse(&args)?;
        // Call the read-only query directly: the REST feed handler queues AI
        // work. A feed view must not change messages, jobs or read state.
        let mut messages = self
            .state
            .store
            .get_unified_messages(
                args.limit + 1,
                args.before_timestamp,
                args.before_message_id.as_deref(),
            )
            .mcp()?;
        let has_more = messages.len() > args.limit as usize;
        if has_more {
            messages.remove(0);
        }
        let next_cursor = messages
            .first()
            .filter(|_| has_more)
            .map(|message| FeedCursor {
                before_timestamp: message.timestamp,
                before_message_id: message.id.clone(),
            });
        let page = FeedPage {
            messages: messages.into_iter().map(FeedMessage::from).collect(),
            has_more,
            next_cursor,
        };
        Ok(CallToolResult::structured(
            serde_json::to_value(page)
                .map_err(|error| McpError::internal_error(error.to_string(), None))?,
        ))
    }

    pub(super) async fn handle_read_messages(
        &self,
        args: Value,
    ) -> Result<CallToolResult, McpError> {
        self.require_read()?;
        let contact_id = crate::identity::canonical_chat_id(required_string(&args, "contact_id")?);
        let contact_id = contact_id.as_ref();
        let contact = self
            .state
            .store
            .get_contact(contact_id)
            .mcp()?
            .ok_or_else(|| McpError::invalid_params("Unknown contact_id", None))?;
        let limit = argument_limit(&args);
        let before_timestamp = args.get("before_timestamp").and_then(Value::as_i64);
        let before_message_id = args.get("before_message_id").and_then(Value::as_str);
        let direction = args
            .get("direction")
            .and_then(Value::as_str)
            .unwrap_or("all");
        let content_type = args
            .get("content_type")
            .and_then(Value::as_str)
            .map(str::to_lowercase);
        let mut messages: Vec<StoredMessage> = Vec::new();
        let mut scan_timestamp = before_timestamp;
        let mut scan_id = before_message_id.map(str::to_string);
        loop {
            let batch = self
                .state
                .store
                .get_messages_paginated(
                    contact_id,
                    Some(MAX_LIMIT as u32),
                    scan_timestamp,
                    scan_id.as_deref(),
                    true,
                )
                .mcp()?;
            let exhausted = batch.len() < MAX_LIMIT;
            let cursor = batch
                .first()
                .map(|message| (message.timestamp, message.id.clone()));
            for message in batch.into_iter().rev() {
                let direction_matches = match direction {
                    "incoming" => !message.is_from_me,
                    "outgoing" => message.is_from_me,
                    _ => true,
                };
                if direction_matches
                    && content_type
                        .as_ref()
                        .is_none_or(|value| message.content_type.to_lowercase() == *value)
                {
                    messages.push(message);
                    if messages.len() > limit {
                        break;
                    }
                }
            }
            if exhausted || messages.len() > limit {
                break;
            }
            let Some((timestamp, id)) = cursor else {
                break;
            };
            scan_timestamp = Some(timestamp);
            scan_id = Some(id);
        }
        messages.reverse();
        let has_more = messages.len() > limit;
        if has_more {
            let remove_count = messages.len() - limit;
            messages.drain(0..remove_count);
        }
        let next_cursor = if has_more {
            messages.first().map(
                |message| json!({"beforeTimestamp":message.timestamp,"beforeMessageId":message.id}),
            )
        } else {
            None
        };
        let messages: Vec<MessageInfo> = messages.into_iter().map(MessageInfo::from).collect();

        Ok(CallToolResult::structured(json!({
            "contact": ContactInfo::from(contact),
            "messages": messages,
            "nextCursor": next_cursor,
            "hasMore": has_more,
        })))
    }

    pub(super) async fn handle_search_messages(
        &self,
        args: Value,
    ) -> Result<CallToolResult, McpError> {
        self.require_read()?;
        let query = required_string(&args, "query")?.trim().to_lowercase();
        if query.is_empty() {
            return Err(McpError::invalid_params("query must not be empty", None));
        }
        let contact_filter = args.get("contact_id").and_then(Value::as_str);
        let after = args.get("after_timestamp").and_then(Value::as_i64);
        let before = args.get("before_timestamp").and_then(Value::as_i64);
        let direction = args
            .get("direction")
            .and_then(Value::as_str)
            .unwrap_or("all");
        let limit = argument_limit(&args);
        let contacts = if let Some(contact_id) = contact_filter {
            vec![self
                .state
                .store
                .get_contact(contact_id)
                .mcp()?
                .ok_or_else(|| McpError::invalid_params("Unknown contact_id", None))?]
        } else {
            self.state.store.get_contacts().mcp()?
        };

        let mut matches = Vec::new();
        for contact in contacts {
            for message in self.state.store.get_messages(&contact.id).mcp()? {
                if after.is_some_and(|value| message.timestamp < value)
                    || before.is_some_and(|value| message.timestamp >= value)
                    || (direction == "incoming" && message.is_from_me)
                    || (direction == "outgoing" && !message.is_from_me)
                {
                    continue;
                }
                let searchable = format!(
                    "{} {} {} {}",
                    message.original_text.as_deref().unwrap_or_default(),
                    message.translated_text.as_deref().unwrap_or_default(),
                    message
                        .content
                        .as_ref()
                        .and_then(|content| content.get("body"))
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                    message.sender_name.as_deref().unwrap_or_default()
                )
                .to_lowercase();
                if searchable.contains(&query) {
                    matches.push(message);
                }
            }
        }
        matches.sort_by(|left, right| {
            right
                .timestamp
                .cmp(&left.timestamp)
                .then_with(|| right.id.cmp(&left.id))
        });
        let cursor = args.get("cursor").and_then(Value::as_u64).unwrap_or(0) as usize;
        let total_matched = matches.len();
        let truncated = total_matched.saturating_sub(cursor) > limit;
        let matches = matches.into_iter().skip(cursor).take(limit);
        let messages: Vec<MessageInfo> = matches.into_iter().map(MessageInfo::from).collect();
        Ok(CallToolResult::structured(json!({
            "query": query,
            "messages": messages,
            "truncated": truncated,
            "nextCursor": truncated.then_some(cursor.saturating_add(limit)),
            "totalMatched": total_matched,
        })))
    }
}
