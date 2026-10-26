use super::*;

impl AppState {
    /// Register a bridge send request so send_result can reconcile it.
    pub async fn register_pending_send(
        &self,
        request_id: i32,
        temp_message_id: Option<String>,
    ) -> oneshot::Receiver<BridgeSendResult> {
        let (tx, rx) = oneshot::channel();
        let operation_id = crate::outbox::OPERATION_ID.try_with(Clone::clone).ok();
        if let Err(error) = self.store.register_outbox_attempt(
            request_id,
            operation_id.as_deref(),
            temp_message_id.as_deref(),
        ) {
            error!("Cannot persist send attempt: {error}");
            return rx;
        }
        let mut pending = self.pending_send_requests.write().await;
        pending.insert(
            request_id,
            PendingSendRequest {
                temp_message_id,
                tx,
            },
        );
        rx
    }

    /// Cancel a pending send and remove any temporary optimistic row.
    pub async fn cancel_pending_send(&self, request_id: i32) {
        let _ = self.store.cancel_outbox_attempt(request_id);
        let pending = self.pending_send_requests.write().await.remove(&request_id);
        if let Some(pending) = pending {
            if let Some(temp_message_id) = pending.temp_message_id {
                if let Err(e) = self.store.delete_message(&temp_message_id) {
                    warn!(
                        "Failed to delete pending message {} after send cancellation: {}",
                        temp_message_id, e
                    );
                }
            }
        }
    }

    /// Wait for a bridge send result with a bounded timeout.
    pub async fn wait_for_send_result(
        &self,
        request_id: i32,
        rx: oneshot::Receiver<BridgeSendResult>,
    ) -> Result<BridgeSendResult, SendConfirmationError> {
        self.wait_for_send_result_with_timeout(request_id, rx, SEND_RESULT_TIMEOUT)
            .await
    }

    pub(in crate::web) async fn wait_for_send_result_with_timeout(
        &self,
        request_id: i32,
        rx: oneshot::Receiver<BridgeSendResult>,
        timeout: Duration,
    ) -> Result<BridgeSendResult, SendConfirmationError> {
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(_)) => {
                self.mark_send_uncertain(request_id).await;
                Err(SendConfirmationError::ChannelClosed)
            }
            Err(_) => {
                self.mark_send_uncertain(request_id).await;
                Err(SendConfirmationError::Timeout)
            }
        }
    }

    pub(super) async fn mark_send_uncertain(&self, request_id: i32) {
        // Release the in-memory waiter, retaining durable metadata for late acknowledgements.
        self.pending_send_requests.write().await.remove(&request_id);
        if let Ok(Some(id)) = self.store.uncertain_attempt(request_id) {
            if let Ok(Some(message)) = self.store.get_message_by_id(&id) {
                let _ = self
                    .broadcast_tx
                    .send(WebSocketEvent::MessageUpdated { message });
            }
        }
    }

    /// Reconcile a bridge send_result with a temporary stored row and release waiters.
    pub async fn handle_send_result(&self, mut result: BridgeSendResult) {
        if result.success && result.message_id.as_deref().is_none_or(str::is_empty) {
            result.success = false;
            result.error =
                Some("WhatsApp confirmation did not include a message identifier.".into());
        }
        let expected = self
            .store
            .expected_send_count(result.request_id)
            .unwrap_or(usize::MAX);
        if result.success && expected > 1 && result.message_ids.len() != expected {
            result.success = false;
            result.error=Some("WhatsApp returned an incomplete album confirmation. Check the conversation before sending again.".into());
        }
        result.timestamp = normalize_bridge_timestamp_millis(result.timestamp);
        result.timestamps = result
            .timestamps
            .into_iter()
            .map(|timestamp| {
                normalize_bridge_timestamp_millis(Some(timestamp)).unwrap_or(timestamp)
            })
            .collect();
        let metadata = self
            .store
            .complete_attempt(
                result.request_id,
                result.success,
                &serde_json::to_string(&result).unwrap_or_default(),
            )
            .ok()
            .flatten();
        let pending = self
            .pending_send_requests
            .write()
            .await
            .remove(&result.request_id);

        let late = pending
            .as_ref()
            .is_none_or(|pending| pending.tx.is_closed());
        let temp_id = pending
            .as_ref()
            .and_then(|pending| pending.temp_message_id.clone())
            .or_else(|| metadata.as_ref().and_then(|(_, temp)| temp.clone()));
        if pending.is_some() || metadata.is_some() {
            if result.success {
                if let Some(temp_message_id) = temp_id.as_deref() {
                    let confirmed_message_id =
                        result.message_id.as_deref().unwrap_or(temp_message_id);
                    if let Err(e) = self.store.replace_message_id(
                        temp_message_id,
                        confirmed_message_id,
                        result.timestamp,
                    ) {
                        warn!(
                            "Failed to reconcile pending message {} to {}: {}",
                            temp_message_id, confirmed_message_id, e
                        );
                    }
                    let _ = self.store.set_delivery_status(confirmed_message_id, "sent");
                }
            } else if let Some(temp_message_id) = temp_id.as_deref() {
                let _ = self.store.set_delivery_status(temp_message_id, "uncertain");
            }

            let _ = self.broadcast_tx.send(WebSocketEvent::SendResult {
                request_id: result.request_id,
                success: result.success,
                temp_message_id: temp_id,
                message_id: result.message_id.clone(),
                timestamp: result.timestamp,
                error: result.error.clone(),
            });

            if late && result.success {
                if let Err(error) = crate::voice::recover_confirmation(self, result.clone()) {
                    warn!("Late voice confirmation failed: {error}");
                }
                if let Some((Some(operation), _)) = metadata {
                    if let Err(error) = crate::outbox::confirm_late(self, &operation) {
                        warn!("Late delivery reconciliation failed: {error}");
                    }
                }
            }
            if let Some(pending) = pending {
                let _ = pending.tx.send(result);
            }
        } else if result.request_id != 0 {
            warn!(
                "Received send_result for unknown request_id {}",
                result.request_id
            );
        }
    }

    pub fn handle_send_progress(
        &self,
        progress_id: String,
        stage: String,
        completed: usize,
        total: usize,
    ) {
        let _ = self.broadcast_tx.send(WebSocketEvent::SendProgress {
            progress_id,
            stage,
            completed,
            total,
        });
    }
}
