use super::*;

/// Bound automatic work independently of the bridge event loop. Older notes remain available on demand.
pub fn queue_incoming(state: Arc<AppState>, message_id: String) {
    if !state
        .translator()
        .is_some_and(|translator| translator.has_audio())
    {
        return;
    }
    let enabled = state
        .store
        .get_message_by_id(&message_id)
        .ok()
        .flatten()
        .and_then(|message| {
            state
                .store
                .get_conversation_settings(&message.contact_id)
                .ok()
        })
        .is_some_and(|settings| settings.translation_enabled);
    if !enabled {
        return;
    }
    let Ok(permit) = state.voice_queue.clone().try_acquire_owned() else {
        return;
    };
    let epoch = state.voice_epoch.load(std::sync::atomic::Ordering::SeqCst);
    tokio::spawn(async move {
        let _permit = permit;
        let _guard = state.voice_lock.lock().await;
        if epoch != state.voice_epoch.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        if let Err(error) = translate_received_inner(&state, &message_id).await {
            tracing::warn!("Incoming voice translation deferred: {error}");
        }
    });
}

pub async fn send(State(state): State<Arc<AppState>>, Json(req): Json<SendRequest>) -> Response {
    result_response(async {
        if uuid::Uuid::parse_str(&req.preparation_id).is_err() { bail!("Incoming translations cannot be sent as prepared recordings."); }
        let (payload, status, result, created) = state.store.voice_note(&req.preparation_id)?.context("Recording preview expired. Please prepare it again.")?;
        if let Some(result) = result { return Ok(serde_json::from_str(&result)?); }
        if status != "prepared" { bail!("Delivery is awaiting confirmation. Check the conversation before recording again; this note will not be sent twice."); }
        if chrono::Utc::now().timestamp() - created > 15 * 60 { bail!("Recording preview expired. Please prepare it again."); }
        if !*state.connected.read().await { bail!("WhatsApp is disconnected. Reconnect before sending."); }
        let mut note: VoiceNote = serde_json::from_str(&payload)?;
        if note.is_translated != state.store.get_conversation_settings(&note.contact_id)?.translation_enabled {
            bail!("Translation settings changed. Please record or prepare this voice note again.");
        }
        let requires_quote = state.store.validate_reply_only(&note.contact_id, note.reply_to.as_deref())?;
        if note.reply_only_if_not_latest && !requires_quote {
            let id = note.reply_to.as_deref().context("Select a message before recording a reply")?;
            if state.store.reply_is_latest(&note.contact_id, id)? {
                note.reply_to = None; note.reply_to_sender = None; note.reply_to_text = None;
            }
        }
        note.reply_only_if_not_latest = false;
        if !state.store.claim_voice_send_with_payload(&note.id, &serde_json::to_string(&note)?)? { bail!("This recording is already sending."); }
        // Sending belongs to the server, not the HTTP connection: losing a client must not cancel delivery.
        let send_state = state.clone();
        let operation = crate::outbox::OPERATION_ID.try_with(Clone::clone).unwrap_or_default();
        let outcome = tokio::spawn(crate::outbox::OPERATION_ID.scope(operation,async move {
            let primary = send_one(&send_state, &note, false).await?;
            let mut result = json!({"success":true,"messageId":primary.0,"timestamp":primary.1,"originalFollowUpSent":false});
            if note.original_follow_up {
                match send_one(&send_state, &note, true).await {
                    Ok(_) => result["originalFollowUpSent"] = json!(true),
                    Err(_) => result["warning"] = json!("Translated voice note sent, but the original follow-up could not be confirmed. Check the conversation before sending it again."),
                }
            }
            send_state.store.finish_voice_send(&note.id, &result.to_string())?;
            Ok::<_, anyhow::Error>(result)
        })).await??;
        Ok(outcome)
    }.await)
}
pub(super) async fn send_one(
    state: &AppState,
    note: &VoiceNote,
    original: bool,
) -> Result<(String, i64)> {
    let request_id = state.next_request_id();
    let pending_id = format!("pending_voice_{request_id}");
    store_audio_message(
        state,
        note,
        original,
        crate::web::BridgeSendResult {
            request_id,
            success: false,
            message_id: Some(pending_id.clone()),
            timestamp: Some(chrono::Utc::now().timestamp_millis()),
            message_ids: vec![],
            timestamps: vec![],
            error: None,
        },
        "sending",
    )?;
    let rx = state
        .register_pending_send(request_id, Some(pending_id))
        .await;
    state
        .store
        .attach_voice_attempt(request_id, &note.id, original)?;
    let duration = if original {
        note.original_duration_seconds
    } else {
        note.duration_seconds
    };
    let handed_off = state
        .send_bridge_command(BridgeCommand::SendAudio {
            request_id: Some(request_id),
            to: note.contact_id.clone(),
            media_data: if original {
                note.send_original.clone()
            } else {
                note.send_audio.clone()
            },
            duration_seconds: duration,
            reply_to: note.reply_to.clone(),
            reply_to_sender: note.reply_to_sender.clone(),
            reply_to_text: note.reply_to_text.clone(),
        })
        .await;
    if let Err(error) = handed_off {
        state.cancel_pending_send(request_id).await;
        if !original {
            state.store.release_voice_send(&note.id)?;
        }
        bail!(error);
    }
    let result = state.wait_for_send_result(request_id, rx).await.map_err(|_| anyhow::anyhow!("Voice-note delivery could not be confirmed. Check the conversation; this recording is locked against duplicate sending."))?;
    if !result.success {
        bail!("WhatsApp could not confirm the voice note. Check the conversation before trying a new recording.");
    }
    store_confirmed(state, note, original, result)
}

pub fn recover_confirmation(state: &AppState, result: crate::web::BridgeSendResult) -> Result<()> {
    let Some((id, original)) = state.store.voice_attempt(result.request_id)? else {
        return Ok(());
    };
    let Some((payload, _, _, _)) = state.store.voice_note(&id)? else {
        return Ok(());
    };
    if payload == "{}" {
        return Ok(());
    } // Its durable pending message has already been reconciled.
    let note: VoiceNote = serde_json::from_str(&payload)?;
    let (message_id, timestamp) = store_confirmed(state, &note, original, result)?;
    // Preserve the uncertain preparation against resending, but make its late confirmation reviewable.
    if !original {
        state.store.finish_voice_send(&id,&json!({"success":true,"messageId":message_id,"timestamp":timestamp,"originalFollowUpSent":false,"warning":"Delivery confirmed after a delay. The original follow-up was not sent automatically."}).to_string())?;
    }
    Ok(())
}

pub(super) fn store_confirmed(
    state: &AppState,
    note: &VoiceNote,
    original: bool,
    result: crate::web::BridgeSendResult,
) -> Result<(String, i64)> {
    store_audio_message(state, note, original, result, "sent")
}

pub(super) fn store_audio_message(
    state: &AppState,
    note: &VoiceNote,
    original: bool,
    result: crate::web::BridgeSendResult,
    status: &str,
) -> Result<(String, i64)> {
    let duration = if original {
        note.original_duration_seconds
    } else {
        note.duration_seconds
    };
    let message_id = result.message_id.context("Missing WhatsApp confirmation")?;
    let timestamp = result
        .timestamp
        .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());
    let contact = state
        .store
        .get_contact(&note.contact_id)?
        .context("Conversation not found")?;
    let is_translated = note.is_translated && !original;
    let content = json!({"type":"audio","isVoiceNote":true,"durationSeconds":duration,"mime_type":"audio/mpeg", "media_data":if original {&note.original_data} else {&note.audio_data},"aiGenerated":is_translated,
        "reply_context": note.reply_to.as_ref().map(|id| json!({"messageId":id,"senderName":note.reply_to_sender,"text":note.reply_to_text}))});
    let stored = StoredMessage {
        id: message_id.clone(),
        contact_id: note.contact_id.clone(),
        contact_name: contact.name.clone(),
        contact_phone: contact.phone.clone(),
        sender_name: None,
        sender_phone: None,
        timestamp,
        is_from_me: true,
        is_forwarded: false,
        chat_type: contact
            .contact_type
            .clone()
            .unwrap_or_else(|| "private".into()),
        content_type: "audio".into(),
        content_json: content.to_string(),
        content: Some(content),
        original_text: (!note.transcript.is_empty()).then(|| note.transcript.clone()),
        translated_text: is_translated.then(|| note.translation.clone()),
        source_language: note.is_translated.then(|| {
            if original {
                note.source_language.clone()
            } else {
                note.target_language.clone()
            }
        }),
        is_translated,
        delivery_status: Some(status.into()),
    };
    state.store.add_message(&stored)?;
    if !original {
        let mut cached = note.clone();
        cached.id = format!("sent-{message_id}");
        cached.send_audio.clear();
        cached.send_original.clear();
        state
            .store
            .save_voice_note(&cached.id, &serde_json::to_string(&cached)?)?;
    }
    state.store.upsert_contact(
        &note.contact_id,
        contact.name.as_deref(),
        contact.phone.as_deref(),
        contact.contact_type.as_deref(),
        timestamp,
    )?;
    state.broadcast_message(stored);
    Ok((message_id, timestamp))
}
