//! Terminal mode: show bridge events in the terminal.

use super::*;

/// Main event loop for terminal mode
pub(crate) async fn run_terminal_mode(
    config: BridgeConfig,
    json_output: bool,
    translator: Option<Arc<TranslationService>>,
) -> Result<()> {
    // Channel for receiving events from the bridge
    let (event_tx, mut event_rx) = mpsc::channel::<BridgeEvent>(100);

    // Spawn the bridge process
    print_info("Starting WhatsApp bridge...");
    let bridge = BridgeProcess::spawn(config, event_tx)
        .await
        .context("Failed to start bridge process")?;
    if let Some(pid) = bridge.pid() {
        shutdown::set_bridge_pid(pid);
    }

    let message_display = MessageDisplay::new();
    let mut connected = false;
    let mut qr_displayed = false;

    let mut signals = shutdown::spawn_signal_listener();
    let _ = signals.signals().borrow_and_update();

    loop {
        tokio::select! {
            changed = signals.signals().changed() => {
                if changed.is_err() || signals.signals().borrow().is_some() {
                    let budgets = shutdown::ShutdownBudgets::production();
                    let bridge_timeout = shutdown::BRIDGE_STOP_TIMEOUT;
                    let status = shutdown::run_shutdown_steps(
                        budgets,
                        std::future::ready(()),
                        async move {
                            matches!(
                                bridge.shutdown_within(bridge_timeout).await,
                                Ok(bridge::BridgeStop::Exited)
                            )
                        },
                        std::future::ready(true),
                    )
                    .await;
                    shutdown::finish_process(status);
                }
            }

            // Process events from the bridge
            event = event_rx.recv() => {
                match event {
                    Some(event) => {
                        if json_output {
                            // Raw JSON output mode
                            if let Ok(json) = serde_json::to_string(&event) {
                                println!("{}", json);
                            }
                        } else {
                            handle_terminal_event(
                                event,
                                &message_display,
                                &mut connected,
                                &mut qr_displayed,
                                translator.as_ref(),
                            ).await?;
                        }
                    }
                    None => {
                        // Bridge closed the channel
                        if !connected {
                            print_error("Bridge process terminated unexpectedly");
                        }
                        break;
                    }
                }
            }
        }
    }

    Ok(())
}

/// Handle a single bridge event in terminal mode
pub(crate) async fn handle_terminal_event(
    event: BridgeEvent,
    message_display: &MessageDisplay,
    connected: &mut bool,
    qr_displayed: &mut bool,
    translator: Option<&Arc<TranslationService>>,
) -> Result<()> {
    match event {
        BridgeEvent::Qr { data } => {
            debug!("Received QR code data");
            render_qr_code(&data)?;
            *qr_displayed = true;
        }

        BridgeEvent::Connected { phone, name, .. } => {
            if *qr_displayed {
                clear_qr_display()?;
                *qr_displayed = false;
            }
            print_connected(&phone, &name);
            *connected = true;
        }

        BridgeEvent::ConnectionState { state } => {
            debug!("Connection state: {:?}", state);
            match state {
                ConnectionState::Connecting => {
                    print_info("Connecting to WhatsApp...");
                }
                ConnectionState::Connected => {
                    if !*connected {
                        print_info("Connected!");
                    }
                }
                ConnectionState::Disconnected => {
                    print_warning("Disconnected from WhatsApp");
                    *connected = false;
                }
                ConnectionState::Reconnecting => {
                    print_info("Reconnecting...");
                }
                ConnectionState::LoggedOut => {
                    print_warning("Logged out from WhatsApp");
                    *connected = false;
                }
            }
        }

        BridgeEvent::Message(msg) => {
            // Translate if needed
            if let Some(translator) = translator {
                if !msg.is_from_me {
                    if let Some(text) = extract_text_content(&msg.content) {
                        // CLI mode doesn't have per-conversation settings
                        match translator.process_text(&text, None, None).await {
                            Ok(result) if result.needs_translation => {
                                // Display with translation
                                message_display.display_with_translation(
                                    &msg,
                                    &result.translated_text.unwrap_or(text),
                                    &result.source_language,
                                )?;
                                return Ok(());
                            }
                            Ok(_) => {}
                            Err(e) => {
                                warn!("Failed to translate CLI message {}: {}", msg.id, e);
                            }
                        }
                    }
                }
            }
            message_display.display(&msg)?;
        }

        BridgeEvent::MessageEdit { message, .. } => {
            print_info("Message edited:");
            message_display.display(&message)?;
        }

        BridgeEvent::Error { code, message } => {
            let message = log_privacy::redact_log_text(&message);
            error!("Bridge error [{code}]: {message}");
            print_error(&format!("[{code}] {message}"));
        }

        BridgeEvent::Log { level, message } => record_bridge_log(&level, &message),

        BridgeEvent::LoggedOut { reason } => {
            print_warning(&format!("Logged out: {}", reason));
            print_info("Please restart the application to scan a new QR code.");
            *connected = false;
        }

        BridgeEvent::SendResult {
            request_id,
            success,
            message_id,
            timestamp,
            error,
            ..
        } => {
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
        }

        BridgeEvent::SendProgress {
            progress_id,
            stage,
            completed,
            total,
            ..
        } => {
            debug!("Album {} {}: {}/{}", progress_id, stage, completed, total);
        }

        BridgeEvent::ProfilePicture { .. } => {
            // Profile pictures are only used in web mode
            debug!("Ignoring profile picture event in terminal mode");
        }

        BridgeEvent::ChatPresence { .. } => {
            // Typing indicators are only used in web mode
            debug!("Ignoring chat presence event in terminal mode");
        }

        BridgeEvent::ReadReceiptResult { .. } => {
            debug!("Read receipt result");
        }
        BridgeEvent::MarkAsRead { .. } => {
            // Mark-as-read events are only used in web mode
            debug!("Ignoring mark-as-read event in terminal mode");
        }

        BridgeEvent::Receipt {
            message_ids,
            status,
        } => {
            debug!("Receipt {:?}: {}", message_ids, status);
        }
    }

    Ok(())
}
