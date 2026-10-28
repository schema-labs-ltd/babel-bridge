use super::*;

impl AppState {
    /// Broadcast a new message
    pub fn broadcast_message(&self, message: StoredMessage) {
        let _ = self.broadcast_tx.send(live_event_for_message(message));
    }

    pub fn broadcast_receipt(&self, message_ids: Vec<String>, status: String) {
        let _ = self.broadcast_tx.send(WebSocketEvent::Receipt {
            message_ids,
            status,
        });
    }

    pub async fn send_push_notification(&self, message: &StoredMessage) {
        let Some(client) = &self.push_notifications else {
            return;
        };
        let reaction_target = if message.content_type.eq_ignore_ascii_case("reaction") {
            message
                .content
                .as_ref()
                .and_then(|content| content.get("target_message_id"))
                .and_then(serde_json::Value::as_str)
                .and_then(
                    |message_id| match self.store.get_message_by_id(message_id) {
                        Ok(target) => target,
                        Err(error) => {
                            warn!("Failed to load reaction target for push notification: {error}");
                            None
                        }
                    },
                )
        } else {
            None
        };
        if !should_send_push_notification(message, reaction_target.as_ref()) {
            tracing::debug!(
                "Skipping reaction push because its target was not sent by the user: {}",
                message.id
            );
            return;
        }
        let devices = match self.store.list_push_devices() {
            Ok(devices) => devices,
            Err(error) => {
                error!("Failed to load APNs devices: {}", error);
                return;
            }
        };
        if devices.is_empty() {
            return;
        }

        let badge = self.store.total_unread_count().unwrap_or_default().max(0);
        let avatar_jid = if message.chat_type == "group" {
            message.sender_phone.as_deref().map(|value| {
                if value.contains('@') {
                    value.to_string()
                } else {
                    format!("{value}@s.whatsapp.net")
                }
            })
        } else {
            Some(message.contact_id.clone())
        };
        let avatar_url = match avatar_jid {
            Some(jid) => tokio::time::timeout(
                std::time::Duration::from_secs(2),
                self.get_profile_picture(&jid),
            )
            .await
            .ok()
            .flatten(),
            None => None,
        };
        let notification = PushNotification::from_message_with_context(
            message,
            badge,
            avatar_url.as_deref(),
            reaction_target.as_ref(),
        )
        .with_recipient_count(
            self.store
                .notification_recipient_count(&message.contact_id)
                .unwrap_or_else(|error| {
                    warn!("Unable to load group notification context: {error}");
                    None
                }),
        )
        .with_message_tone(
            self.store
                .message_tone_settings(Some(&message.contact_id))
                .map(|settings| settings.effective_tone)
                .unwrap_or_else(|error| {
                    warn!("Unable to load message ringtone: {error}");
                    // A storage failure must not unexpectedly make a silenced chat audible.
                    crate::message_tones::MessageTone::Silent
                }),
        );
        for device in devices {
            match client.send(&device, &notification).await {
                Ok(ApnsSendOutcome::Delivered) => {}
                Ok(ApnsSendOutcome::RemoveDevice) => {
                    if let Err(error) = self.store.delete_push_device(&device.token) {
                        warn!("Failed to remove stale APNs token: {}", error);
                    }
                }
                Err(error) => warn!(
                    "Failed to send APNs notification to installation {}: {}",
                    device.installation_id, error
                ),
            }
        }
    }

    /// Broadcast a typing indicator
    pub fn broadcast_typing(&self, chat_id: String, user_id: String, state: String) {
        tracing::info!(state = %state, "Broadcasting typing event");
        let result = self.broadcast_tx.send(WebSocketEvent::Typing {
            chat_id,
            user_id,
            state,
        });
        if let Err(e) = result {
            tracing::warn!("Failed to broadcast typing event: {}", e);
        }
    }

    /// Broadcast a mark-as-read event (chat was read from another device)
    pub fn broadcast_mark_as_read(&self, chat_id: String) {
        let _ = self
            .broadcast_tx
            .send(WebSocketEvent::MarkAsRead { chat_id });
    }

    /// Get next request ID
    pub fn next_request_id(&self) -> i32 {
        self.request_id_counter.fetch_add(1, Ordering::SeqCst)
    }
}
