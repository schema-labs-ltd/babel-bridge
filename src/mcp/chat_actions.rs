use super::*;

impl WhatsAppMcpServer {
    pub(super) async fn handle_reaction(&self, args: Value) -> Result<CallToolResult, McpError> {
        self.require_send()?;
        let contact_id = crate::identity::canonical_chat_id(required_string(&args, "contact_id")?);
        let contact_id = contact_id.as_ref();
        let message_id = required_string(&args, "message_id")?;
        let emoji = args
            .get("emoji")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if emoji.chars().count() > 16 {
            return Err(McpError::invalid_params("emoji is too long", None));
        }
        let idempotency_key = required_string(&args, "idempotency_key")?;
        let operation_key = format!("reaction:{contact_id}:{message_id}:{emoji}");
        if let Some(existing) = self
            .idempotency_result(idempotency_key, &operation_key)
            .await?
        {
            return Ok(existing);
        }
        let message = self
            .state
            .store
            .get_message_by_id(message_id)
            .mcp()?
            .ok_or_else(|| McpError::invalid_params("Unknown message_id", None))?;
        if message.contact_id != contact_id {
            return Err(McpError::invalid_params(
                "message_id belongs to another conversation",
                None,
            ));
        }
        if !*self.state.connected.read().await {
            return Err(McpError::internal_error("Not connected to WhatsApp", None));
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
        let request_id = self.state.next_request_id();
        let receiver = self.state.register_pending_send(request_id, None).await;
        let command = BridgeCommand::SendReaction {
            request_id: Some(request_id),
            to: contact_id.to_string(),
            message_id: message_id.to_string(),
            sender_jid: args
                .get("sender_jid")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or(message.sender_phone),
            emoji: emoji.to_string(),
        };
        let outcome = async {
            if let Err(error) = self.state.send_bridge_command(command).await {
                self.state.cancel_pending_send(request_id).await;
                return Err(McpError::internal_error(error, None));
            }
            let result = self
                .state
                .wait_for_send_result(request_id, receiver)
                .await
                .map_err(|error| match error {
                    SendConfirmationError::Timeout => McpError::internal_error(
                        "Timed out waiting for reaction confirmation",
                        None,
                    ),
                    SendConfirmationError::ChannelClosed => {
                        McpError::internal_error("Reaction confirmation channel closed", None)
                    }
                })?;
            if result.success {
                Ok(json!({"success":true,"messageId":message_id,"emoji":emoji}))
            } else {
                Err(McpError::internal_error(
                    result
                        .error
                        .unwrap_or_else(|| "WhatsApp rejected the reaction".to_string()),
                    None,
                ))
            }
        }
        .await;
        match outcome {
            Ok(result) => {
                self.finish_idempotent_operation(idempotency_key, "completed", result.clone())
                    .await;
                Ok(CallToolResult::structured(result))
            }
            Err(error) => {
                self.finish_idempotent_operation(
                    idempotency_key,
                    "failed",
                    json!({"success":false,"error":error.to_string()}),
                )
                .await;
                Err(error)
            }
        }
    }

    pub(super) async fn handle_mark_read(&self, args: Value) -> Result<CallToolResult, McpError> {
        self.require_send()?;
        let contact_id = crate::identity::canonical_chat_id(required_string(&args, "contact_id")?);
        let contact_id = contact_id.as_ref();
        self.state
            .store
            .get_contact(contact_id)
            .mcp()?
            .ok_or_else(|| McpError::invalid_params("Unknown contact_id", None))?;
        let mut receipt_sent = false;
        if let Some(id) = args.get("message_id").and_then(Value::as_str) {
            self.state
                .store
                .record_message_views(contact_id, &[id.to_owned()], chrono::Utc::now().timestamp())
                .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
            receipt_sent = true;
        } else {
            self.state.store.mark_as_read(contact_id).mcp()?;
        }
        Ok(CallToolResult::structured(json!({
            "success":true,
            "contactId":contact_id,
            "receiptSent":false,
            "receiptQueued":receipt_sent,
        })))
    }

    pub(super) async fn idempotency_result(
        &self,
        idempotency_key: &str,
        operation_key: &str,
    ) -> Result<Option<CallToolResult>, McpError> {
        let Some(value) = self.state.store.mcp_send_record(idempotency_key).mcp()? else {
            return Ok(None);
        };
        let record: McpIdempotencyRecord = serde_json::from_value(value)
            .map_err(|error| McpError::internal_error(error.to_string(), None))?;
        if record.operation_key != operation_key {
            return Err(McpError::invalid_params(
                "idempotency_key was already used for a different operation",
                None,
            ));
        }
        match record.status.as_str() {
            "completed" => Ok(record.result.map(CallToolResult::structured)),
            "failed" => Err(McpError::internal_error(
                record
                    .result
                    .as_ref()
                    .and_then(|value| value.get("error"))
                    .and_then(Value::as_str)
                    .unwrap_or("The previous attempt failed; use a new preparation and idempotency key")
                    .to_string(),
                None,
            )),
            _ => Err(McpError::internal_error(
                "An operation with this idempotency_key is already in progress; it will not be sent again",
                None,
            )),
        }
    }

    pub(super) async fn begin_idempotent_operation(
        &self,
        idempotency_key: &str,
        operation_key: &str,
    ) -> bool {
        let record = McpIdempotencyRecord {
            operation_key: operation_key.to_string(),
            status: "pending".to_string(),
            result: None,
            expires_at: chrono::Utc::now().timestamp_millis() + 24 * 60 * 60 * 1000,
        };
        self.state
            .store
            .claim_mcp_send(
                idempotency_key,
                &serde_json::to_value(record).expect("serialize idempotency record"),
            )
            .unwrap_or(false)
    }

    pub(super) async fn finish_idempotent_operation(
        &self,
        idempotency_key: &str,
        status: &str,
        result: Value,
    ) {
        if let Ok(Some(value)) = self.state.store.mcp_send_record(idempotency_key) {
            if let Ok(mut record) = serde_json::from_value::<McpIdempotencyRecord>(value) {
                record.status = status.to_string();
                record.result = Some(result);
                if let Err(error) = self.state.store.update_mcp_send(
                    idempotency_key,
                    &serde_json::to_value(record).expect("serialize idempotency record"),
                ) {
                    tracing::error!("Cannot persist MCP send outcome: {error}");
                }
            }
        }
    }
}
