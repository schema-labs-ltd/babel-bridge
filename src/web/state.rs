//! Shared application state behaviour: bridge commands, send confirmations, live events and push notifications.

use super::*;

impl AppState {
    /// The AI service when it has a usable credential: API key or ChatGPT.
    pub fn translator(&self) -> Option<&Arc<TranslationService>> {
        self.ai_service
            .as_ref()
            .filter(|service| service.is_configured())
    }

    pub fn new(
        store: MessageStore,
        web_dir: PathBuf,
        data_dir: PathBuf,
        translator: Option<Arc<TranslationService>>,
        password: Option<String>,
        push_notifications: Option<Arc<ApnsClient>>,
    ) -> Arc<Self> {
        let (broadcast_tx, _) = broadcast::channel(100);
        let next_request_id = store
            .recover_outbox()
            .expect("Cannot recover the durable outbox");

        let state = Arc::new(Self {
            store,
            connected: RwLock::new(false),
            phone: RwLock::new(None),
            name: RwLock::new(None),
            qr_code: RwLock::new(None),
            connection_state: RwLock::new(WhatsAppConnectionState::Connecting),
            broadcast_tx,
            web_dir,
            data_dir,
            command_tx: RwLock::new(None),
            ai_service: translator,
            avatar_cache: RwLock::new(HashMap::new()),
            pending_avatar_requests: RwLock::new(HashMap::new()),
            pending_send_requests: RwLock::new(HashMap::new()),
            pending_photo_albums: RwLock::new(HashMap::new()),
            mcp_prepared_messages: RwLock::new(HashMap::new()),
            voice_lock: tokio::sync::Mutex::new(()),
            gallery_thumbnails: crate::gallery::ThumbnailCache::default(),
            voice_queue: Arc::new(tokio::sync::Semaphore::new(4)),
            voice_epoch: std::sync::atomic::AtomicU64::new(0),
            login_budget: tokio::sync::Mutex::new(crate::access::LoginBudget::default()),
            request_id_counter: AtomicI32::new(next_request_id),
            password,
            auth_tokens: RwLock::new(HashMap::new()),
            session_reset_requested: AtomicBool::new(false),
            push_notifications,
            shutdown: watch::channel(false).0,
        });
        crate::outbox::recover_receipts(&state).expect("Cannot reconcile durable send receipts");
        state
    }

    /// Wait for `delay` or for shutdown, whichever comes first.
    /// Returns true when shutdown has been requested.
    pub async fn wait_for_shutdown(&self, delay: Duration) -> bool {
        let mut watch = self.shutdown.subscribe();
        if *watch.borrow() {
            return true;
        }
        tokio::select! {
            changed = watch.changed() => changed.is_ok() && *watch.borrow(),
            _ = tokio::time::sleep(delay) => false,
        }
    }

    pub fn request_session_reset_before_bridge_restart(&self) {
        self.session_reset_requested.store(true, Ordering::SeqCst);
    }

    pub fn take_session_reset_request(&self) -> bool {
        self.session_reset_requested.swap(false, Ordering::SeqCst)
    }

    pub fn clear_session_files(&self) -> std::io::Result<()> {
        for filename in ["session.db", "session.db-wal", "session.db-shm"] {
            let path = self.data_dir.join(filename);
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(err),
            }
        }

        Ok(())
    }

    pub fn clear_orphaned_session_sidecars(&self) -> std::io::Result<()> {
        if self.data_dir.join("session.db").try_exists()? {
            return Ok(());
        }

        for filename in ["session.db-wal", "session.db-shm"] {
            let path = self.data_dir.join(filename);
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(err),
            }
        }

        Ok(())
    }

    /// Set the bridge command sender
    pub async fn set_command_tx(&self, tx: mpsc::Sender<BridgeCommand>) {
        *self.command_tx.write().await = Some(tx);
    }

    /// Send a command to the bridge
    pub async fn send_bridge_command(&self, cmd: BridgeCommand) -> Result<(), String> {
        match &cmd {
            BridgeCommand::Send { to, reply_to, .. }
            | BridgeCommand::SendImage { to, reply_to, .. }
            | BridgeCommand::SendMedia { to, reply_to, .. }
            | BridgeCommand::SendImages { to, reply_to, .. }
            | BridgeCommand::SendAudio { to, reply_to, .. } => {
                self.store
                    .validate_reply_only(to, reply_to.as_deref())
                    .map_err(|e| e.to_string())?;
            }
            _ => {}
        }
        let send_id = match &cmd {
            BridgeCommand::Send { request_id, .. }
            | BridgeCommand::SendImage { request_id, .. }
            | BridgeCommand::SendMedia { request_id, .. }
            | BridgeCommand::SendImages { request_id, .. }
            | BridgeCommand::SendAudio { request_id, .. }
            | BridgeCommand::SendReaction { request_id, .. } => *request_id,
            _ => None,
        };
        if let Some(id) = send_id {
            if !self
                .store
                .has_outbox_attempt(id)
                .map_err(|error| error.to_string())?
            {
                return Err("Unable to persist the send; nothing was sent.".into());
            }
            if let BridgeCommand::SendImages { images, .. } = &cmd {
                self.store
                    .set_expected_send_count(id, images.len())
                    .map_err(|error| error.to_string())?;
            }
        }
        let tx = self.command_tx.read().await;
        if let Some(tx) = tx.as_ref() {
            tx.send(cmd).await.map_err(|e| e.to_string())
        } else {
            Err("Bridge not connected".to_string())
        }
    }

    /// Update connection status
    pub async fn set_connected(
        &self,
        connected: bool,
        phone: Option<String>,
        name: Option<String>,
    ) {
        *self.connected.write().await = connected;
        *self.phone.write().await = phone.clone();
        *self.name.write().await = name.clone();
        *self.qr_code.write().await = None;
        let mut state = self.connection_state.write().await;
        if connected {
            *state = WhatsAppConnectionState::Connected;
        } else if *state != WhatsAppConnectionState::LinkingRequired {
            *state = WhatsAppConnectionState::Reconnecting;
        }
        drop(state);
        self.broadcast_connection_status().await;

        if connected {
            *self.qr_code.write().await = None;
            let _ = self.broadcast_tx.send(WebSocketEvent::Connected {
                phone: phone.unwrap_or_default(),
                name: name.unwrap_or_default(),
            });
        } else {
            let _ = self.broadcast_tx.send(WebSocketEvent::Disconnected);
        }
    }

    /// Set QR code
    pub async fn set_qr_code(&self, qr: String) {
        *self.connected.write().await = false;
        *self.connection_state.write().await = WhatsAppConnectionState::LinkingRequired;
        *self.qr_code.write().await = Some(qr.clone());
        self.broadcast_connection_status().await;
        let _ = self.broadcast_tx.send(WebSocketEvent::Qr { data: qr });
    }

    /// Record a connection that will not be retried. Linking stays as it is.
    pub async fn mark_whatsapp_disconnected(&self) {
        if *self.connection_state.read().await == WhatsAppConnectionState::LinkingRequired {
            return;
        }
        *self.connected.write().await = false;
        *self.phone.write().await = None;
        *self.name.write().await = None;
        let mut connection_state = self.connection_state.write().await;
        if *connection_state == WhatsAppConnectionState::LinkingRequired {
            return;
        }
        *connection_state = WhatsAppConnectionState::Disconnected;
        drop(connection_state);
        self.broadcast_connection_status().await;
        let _ = self.broadcast_tx.send(WebSocketEvent::Disconnected);
    }

    pub async fn require_whatsapp_link(&self) {
        *self.connection_state.write().await = WhatsAppConnectionState::LinkingRequired;
        self.set_connected(false, None, None).await;
    }

    async fn broadcast_connection_status(&self) {
        let _ = self.broadcast_tx.send(WebSocketEvent::Status {
            connected: *self.connected.read().await,
            phone: self.phone.read().await.clone(),
            name: self.name.read().await.clone(),
            connection_state: *self.connection_state.read().await,
            qr: self.qr_code.read().await.clone(),
        });
    }

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

    /// Request a profile picture and wait for the response
    pub async fn get_profile_picture(&self, jid: &str) -> Option<String> {
        // Check cache first (valid for 1 hour)
        let now = chrono::Utc::now().timestamp();
        {
            let cache = self.avatar_cache.read().await;
            if let Some(cached) = cache.get(jid) {
                if now - cached.fetched_at < 3600 {
                    return cached.url.clone();
                }
            }
        }

        // Not in cache or expired, request from bridge
        let request_id = self.next_request_id();
        let (tx, rx) = oneshot::channel();

        // Register pending request
        {
            let mut pending = self.pending_avatar_requests.write().await;
            pending.insert(request_id, tx);
        }

        // Send command to bridge
        let cmd = BridgeCommand::GetProfilePicture {
            request_id,
            to: jid.to_string(),
        };

        if let Err(e) = self.send_bridge_command(cmd).await {
            error!("Failed to request profile picture: {}", e);
            // Clean up pending request
            let mut pending = self.pending_avatar_requests.write().await;
            pending.remove(&request_id);
            return None;
        }

        // Wait for response with timeout
        match tokio::time::timeout(std::time::Duration::from_secs(10), rx).await {
            Ok(Ok(url)) => {
                // Cache the result
                let mut cache = self.avatar_cache.write().await;
                cache.insert(
                    jid.to_string(),
                    ProfilePicture {
                        url: url.clone(),
                        fetched_at: now,
                    },
                );
                url
            }
            _ => {
                // Timeout or error, clean up
                let mut pending = self.pending_avatar_requests.write().await;
                pending.remove(&request_id);
                None
            }
        }
    }

    /// Handle profile picture response from bridge
    pub async fn handle_profile_picture_response(&self, request_id: i32, url: Option<String>) {
        let mut pending = self.pending_avatar_requests.write().await;
        if let Some(tx) = pending.remove(&request_id) {
            let _ = tx.send(url);
        }
    }

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

    pub(super) async fn wait_for_send_result_with_timeout(
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

    async fn mark_send_uncertain(&self, request_id: i32) {
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

pub(super) fn should_send_push_notification(
    message: &StoredMessage,
    reaction_target: Option<&StoredMessage>,
) -> bool {
    !message.content_type.eq_ignore_ascii_case("reaction")
        || reaction_target.is_some_and(|target| target.is_from_me)
}

fn normalize_bridge_timestamp_millis(timestamp: Option<i64>) -> Option<i64> {
    timestamp.filter(|value| *value > 0).map(|value| {
        if value < 10_000_000_000 {
            value * 1000
        } else {
            value
        }
    })
}
