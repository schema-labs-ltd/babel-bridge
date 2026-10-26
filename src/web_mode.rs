//! Web mode: start the bridge, translator and web server, and shut them down.

use super::*;

/// Run in web server mode
pub(crate) async fn run_web_mode(
    config: BridgeConfig,
    args: Args,
    data_dir: std::path::PathBuf,
    translator: Option<Arc<TranslationService>>,
) -> Result<()> {
    // Initialize message store
    let store = MessageStore::new(&data_dir)?;
    if let Some(service) = &translator {
        service.set_runtime_settings(store.get_openai_settings()?);
    }

    let push_notifications = ApnsClient::from_env()
        .context("Invalid APNs configuration")?
        .map(Arc::new);
    if push_notifications.is_some() {
        info!("APNs notifications enabled");
    } else {
        info!("APNs notifications disabled (credentials not configured)");
    }

    // Find web directory (relative to executable or in project)
    let web_dir = find_web_dir()?;
    info!("Serving web files from: {:?}", web_dir);

    // Create app state with translator and password
    let state = AppState::new(
        store.clone(),
        web_dir,
        data_dir.clone(),
        translator.clone(),
        args.password.clone(),
        push_notifications,
    );

    incoming::start(state.clone())?;
    topics::start(state.clone());
    read_receipts::start(state.clone());
    // Spawn the web server (once, outside the bridge loop). HTTP, WebSocket,
    // and MCP all stop accepting when this shutdown signal fires.
    let server_state = state.clone();
    let host = args.host.clone();
    let port = args.port;
    let allow_local_no_auth = args.allow_local_no_auth;
    let (server_shutdown_tx, server_shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let server_task = tokio::spawn(async move {
        if let Err(e) =
            web::start_server(server_state, &host, port, allow_local_no_auth, async move {
                let _ = server_shutdown_rx.await;
            })
            .await
        {
            error!("Web server error: {}", e);
        }
    });
    let mut server_shutdown_tx = Some(server_shutdown_tx);
    let mut server_task = Some(server_task);
    let mut signals = shutdown::spawn_signal_listener();
    // The initial empty value is not a signal.
    let _ = signals.signals().borrow_and_update();

    // Bridge restart loop - restarts bridge after logout
    loop {
        if signals.signals().borrow().is_some() {
            let status = shutdown_web(
                &state,
                &store,
                server_shutdown_tx.take(),
                server_task.take(),
                None,
            )
            .await;
            shutdown::finish_process(status);
        }
        // Channel for receiving events from the bridge
        let (event_tx, mut event_rx) = mpsc::channel::<BridgeEvent>(100);

        if let Err(e) = state.clear_orphaned_session_sidecars() {
            warn!(
                "Failed to clear orphaned WhatsApp session sidecar files: {}",
                e
            );
        }

        // Spawn the bridge process
        print_info("Starting WhatsApp bridge...");
        let bridge = match BridgeProcess::spawn(config.clone(), event_tx).await {
            Ok(b) => b,
            Err(e) => {
                print_error(&format!("Failed to start bridge: {}", e));
                pause_for_shutdown(signals.signals(), tokio::time::Duration::from_secs(3)).await;
                continue;
            }
        };

        // Pass the bridge's command sender to the app state for sending messages
        state.set_command_tx(bridge.command_sender()).await;
        if let Some(pid) = bridge.pid() {
            shutdown::set_bridge_pid(pid);
        }

        // Event loop for this bridge instance
        let should_exit = loop {
            tokio::select! {
                changed = signals.signals().changed() => {
                    if changed.is_ok() && signals.signals().borrow().is_some() {
                        break true;
                    }
                }

                event = event_rx.recv() => {
                    match event {
                        Some(event) => {
                            if let Err(e) = handle_web_event(event, &state, &store, state.translator()).await {
                                error!("Error handling event: {}", e);
                            }
                        }
                        None => {
                            // Bridge terminated - check if it was a logout or unexpected
                            info!("Bridge process terminated, restarting...");
                            break false; // Restart bridge
                        }
                    }
                }
            }
        };

        // A crashed bridge must not leave clients believing WhatsApp is online.
        // Reap it before touching SQLite session files or starting a replacement.
        *state.command_tx.write().await = None;
        if should_exit {
            let status = shutdown_web(
                &state,
                &store,
                server_shutdown_tx.take(),
                server_task.take(),
                Some(bridge),
            )
            .await;
            shutdown::finish_process(status);
        }
        let _ = bridge.shutdown().await;
        state.set_connected(false, None, None).await;

        if state.take_session_reset_request() {
            info!("Clearing WhatsApp session files before bridge restart");
            if let Err(e) = state.clear_session_files() {
                error!(
                    "Failed to clear WhatsApp session files before restart: {}",
                    e
                );
            }
        }

        // Small delay before restarting
        pause_for_shutdown(signals.signals(), tokio::time::Duration::from_secs(1)).await;
    }
}

pub(crate) async fn pause_for_shutdown(
    signals: &mut tokio::sync::watch::Receiver<Option<shutdown::ShutdownSignal>>,
    delay: tokio::time::Duration,
) {
    tokio::select! {
        _ = signals.changed() => {}
        _ = tokio::time::sleep(delay) => {}
    }
}

pub(crate) async fn shutdown_web(
    state: &Arc<AppState>,
    store: &MessageStore,
    server_shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    server_task: Option<JoinHandle<()>>,
    bridge: Option<BridgeProcess>,
) -> shutdown::ShutdownStatus {
    // Background workers and open WebSockets stop before the server drains.
    let _ = state.shutdown.send_replace(true);
    let budgets = shutdown::ShutdownBudgets::production();
    let bridge_timeout = shutdown::BRIDGE_STOP_TIMEOUT;
    let drain: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> =
        match (server_shutdown_tx, server_task) {
            (Some(trigger), Some(task)) => Box::pin(shutdown::ServerDrain::new(trigger, task)),
            _ => Box::pin(std::future::ready(())),
        };
    let stop_bridge: std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send>> =
        match bridge {
            Some(bridge) => Box::pin(async move {
                matches!(
                    bridge.shutdown_within(bridge_timeout).await,
                    Ok(bridge::BridgeStop::Exited)
                )
            }),
            None => Box::pin(std::future::ready(true)),
        };
    let store = store.clone();
    let close_sqlite = async move {
        match tokio::task::spawn_blocking(move || store.close()).await {
            Ok(Ok(())) => true,
            Ok(Err(error)) => {
                error!("Failed to close message database: {error}");
                false
            }
            Err(error) => {
                error!("Failed to close message database: {error}");
                false
            }
        }
    };
    shutdown::run_shutdown_steps(budgets, drain, stop_bridge, close_sqlite).await
}
