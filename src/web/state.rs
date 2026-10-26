//! Shared application state behaviour: bridge commands, send confirmations, live events and push notifications.

use super::*;

mod live_events;
mod sends;

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
