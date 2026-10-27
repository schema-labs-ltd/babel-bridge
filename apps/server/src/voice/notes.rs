use super::*;

pub(super) async fn checked_bytes(response: reqwest::Response, limit: usize) -> Result<Vec<u8>> {
    use futures::StreamExt;
    if !response.status().is_success() {
        bail!(
            "Voice service returned {}. Please try again later.",
            response.status()
        );
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if bytes.len() + chunk.len() > limit {
            bail!("Voice service response is too large.");
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
pub(super) async fn build_note(
    state: &AppState,
    req: PrepareRequest,
    incoming: bool,
    id: String,
) -> Result<VoiceNote> {
    let epoch = state.voice_epoch.load(std::sync::atomic::Ordering::SeqCst);
    state
        .store
        .get_contact(&req.contact_id)?
        .context("Conversation not found")?;
    if let Some(reply_id) = &req.reply_to {
        let reply = state
            .store
            .get_message_by_id(reply_id)?
            .context("Reply message not found")?;
        if reply.contact_id != req.contact_id {
            bail!("Reply belongs to another conversation.");
        }
    }
    let settings = state.store.get_conversation_settings(&req.contact_id)?;
    if incoming && !settings.translation_enabled {
        bail!("Translation is off for this conversation. Enable it in Conversation settings.");
    }
    if !incoming {
        state
            .store
            .validate_reply_only(&req.contact_id, req.reply_to.as_deref())?;
    }
    let scratch = Scratch::new(&state.data_dir)?;
    tokio::fs::write(scratch.0.join("input"), decode_audio(&req.media_data)?).await?;
    let pcm = convert(
        &scratch.0,
        "input",
        "source.pcm",
        &["-ac", "1", "-ar", "16000", "-f", "s16le"],
    )
    .await?;
    if pcm.len() > MAX_SECONDS * RATE * 2 {
        bail!("Voice notes can be up to three minutes long.");
    }
    if pcm.len() < RATE / 2 {
        bail!("Record at least a quarter second of audio.");
    }
    if pcm
        .chunks_exact(2)
        .all(|s| i16::from_le_bytes([s[0], s[1]]).unsigned_abs() < 128)
    {
        bail!("No audible speech was detected. Please record again.");
    }
    let original_seconds = (pcm.len() as f64 / 2.0 / RATE as f64).ceil() as u32;
    if !settings.translation_enabled {
        // WhatsApp still needs its audio format, but this path never waits on
        // transcription, language detection, translation, or speech generation.
        let original = convert(
            &scratch.0,
            "input",
            "original.mp3",
            &["-ac", "1", "-c:a", "libmp3lame", "-b:a", "64k"],
        )
        .await?;
        let send_audio = convert(
            &scratch.0,
            "input",
            "original.ogg",
            &["-ac", "1", "-c:a", "libopus", "-b:a", "32k"],
        )
        .await?;
        if epoch != state.voice_epoch.load(std::sync::atomic::Ordering::SeqCst) {
            bail!("Session changed while preparing. Please reconnect.");
        }
        return Ok(VoiceNote {
            is_translated: false,
            id,
            contact_id: req.contact_id,
            transcript: String::new(),
            translation: String::new(),
            target_language: String::new(),
            source_language: String::new(),
            voice: "original".into(),
            audio_data: B64.encode(&original),
            original_data: B64.encode(original),
            duration_seconds: original_seconds,
            original_duration_seconds: original_seconds,
            send_audio: B64.encode(send_audio),
            send_original: String::new(),
            original_follow_up: false,
            reply_only_if_not_latest: req.reply_only_if_not_latest,
            reply_to: req.reply_to,
            reply_to_sender: req.reply_to_sender,
            reply_to_text: req.reply_to_text,
        });
    }
    let translator = state
        .translator()
        .context("Translation is unavailable. Configure the OpenAI API key on the server.")?;
    let target = if incoming {
        translator.default_language().to_string()
    } else {
        crate::incoming::outgoing_language(state, &req.contact_id, req.reply_to.as_deref())
            .await?
            .context("Translation settings changed. Please prepare the voice note again.")?
    };
    let preference = state.store.voice_setting(if incoming {
        &req.contact_id
    } else {
        "outgoing"
    })?;
    let voice = select_voice(&preference, &pcm).to_string();
    let wav = convert(
        &scratch.0,
        "input",
        "source.wav",
        &["-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le"],
    )
    .await?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(90))
        .build()?;
    let part = reqwest::multipart::Part::bytes(wav)
        .file_name("voice.wav")
        .mime_str("audio/wav")?;
    let response = client
        .post(translator.audio_api_url("transcriptions"))
        .bearer_auth(translator.audio_api_key()?)
        .multipart(
            reqwest::multipart::Form::new()
                .text("model", "gpt-4o-mini-transcribe")
                .part("file", part),
        )
        .send()
        .await?;
    let transcription: Value = serde_json::from_slice(&checked_bytes(response, 256 * 1024).await?)?;
    let transcript = transcription["text"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();
    if transcript.is_empty() {
        bail!("No speech was detected. Please record again.");
    }
    let translated = translator
        .translate_voice(&transcript, &target, settings.translation_style.as_deref())
        .await?;
    let translation = translated
        .translated_text
        .context("Voice translation returned no text")?;
    if translation.trim().is_empty() || translation.chars().count() > 4096 {
        bail!("The translated recording is empty or too long. Please use a shorter note.");
    }
    state.store.record_usage(
        Some(&req.contact_id),
        None,
        &translated.usage,
        "voice_translation",
    )?;
    let response = client.post(translator.audio_api_url("speech")).bearer_auth(translator.audio_api_key()?)
        .json(&json!({"model":"gpt-4o-mini-tts", "voice":voice, "input":translation, "response_format":"mp3", "instructions":format!("Speak naturally in {target}. Read the provided text faithfully without additions.")})).send().await?;
    let audio = checked_bytes(response, MAX_BYTES).await?;
    tokio::fs::write(scratch.0.join("translated.mp3"), &audio).await?;
    let translated_pcm = convert(
        &scratch.0,
        "translated.mp3",
        "translated.pcm",
        &["-ac", "1", "-ar", "16000", "-f", "s16le"],
    )
    .await?;
    let seconds = (translated_pcm.len() as f64 / 2.0 / RATE as f64).ceil() as u32;
    if seconds > 240 {
        bail!("The translated recording is too long. Please use a shorter note.");
    }
    let original = convert(
        &scratch.0,
        "input",
        "original.mp3",
        &["-ac", "1", "-c:a", "libmp3lame", "-b:a", "64k"],
    )
    .await?;
    let send_audio = if incoming {
        Vec::new()
    } else {
        convert(
            &scratch.0,
            "translated.mp3",
            "translated.ogg",
            &["-ac", "1", "-c:a", "libopus", "-b:a", "32k"],
        )
        .await?
    };
    let follow_up = !incoming && settings.send_original_follow_up && transcript != translation;
    let send_original = if follow_up {
        convert(
            &scratch.0,
            "input",
            "original.ogg",
            &["-ac", "1", "-c:a", "libopus", "-b:a", "32k"],
        )
        .await?
    } else {
        Vec::new()
    };
    if epoch != state.voice_epoch.load(std::sync::atomic::Ordering::SeqCst) {
        bail!("Session changed while translating. Please reconnect.");
    }
    Ok(VoiceNote {
        is_translated: true,
        id,
        contact_id: req.contact_id,
        source_language: translated.source_language,
        transcript,
        translation,
        target_language: target,
        voice,
        audio_data: B64.encode(audio),
        original_data: B64.encode(original),
        duration_seconds: seconds,
        original_duration_seconds: original_seconds,
        send_audio: B64.encode(send_audio),
        send_original: B64.encode(send_original),
        original_follow_up: follow_up,
        reply_only_if_not_latest: req.reply_only_if_not_latest,
        reply_to: req.reply_to,
        reply_to_sender: req.reply_to_sender,
        reply_to_text: req.reply_to_text,
    })
}

pub async fn prepare(
    State(state): State<Arc<AppState>>,
    Json(req): Json<PrepareRequest>,
) -> Response {
    result_response(
        async {
            let _guard = if state
                .store
                .get_conversation_settings(&req.contact_id)?
                .translation_enabled
            {
                Some(
                    tokio::time::timeout(Duration::from_secs(180), state.voice_lock.lock())
                        .await
                        .context("Voice translation is busy. Please try again shortly.")?,
                )
            } else {
                None
            };
            let note = build_note(&state, req, false, uuid::Uuid::new_v4().to_string()).await?;
            state
                .store
                .save_voice_note(&note.id, &serde_json::to_string(&note)?)?;
            Ok(note.public())
        }
        .await,
    )
}
pub async fn translate_received(
    State(state): State<Arc<AppState>>,
    Path(message_id): Path<String>,
) -> Response {
    result_response(
        async {
            if let Some(note) = cached_received(&state, &message_id)? {
                return Ok(note.public());
            }
            let _guard = tokio::time::timeout(Duration::from_secs(180), state.voice_lock.lock())
                .await
                .context("Voice translation is busy. Please try again shortly.")?;
            Ok(translate_received_inner(&state, &message_id)
                .await?
                .public())
        }
        .await,
    )
}
pub(super) fn received_key(state: &AppState, message_id: &str) -> Result<String> {
    let message = state
        .store
        .get_message_by_id(message_id)?
        .context("Message not found")?;
    if !message.is_audio() {
        bail!("This message is not audio.");
    }
    if message.is_from_me
        && state
            .store
            .voice_note(&format!("sent-{message_id}"))?
            .is_some()
    {
        return Ok(format!("sent-{message_id}"));
    }
    let translator = state
        .translator()
        .context("Voice translation is not configured on this server.")?;
    let preference = state.store.voice_setting(&message.contact_id)?;
    let settings = state.store.get_conversation_settings(&message.contact_id)?;
    Ok(format!(
        "received-{:x}",
        Sha256::digest(format!(
            "v1:{message_id}:{}:{preference}:{:?}",
            translator.default_language(),
            settings.translation_style
        ))
    ))
}
pub(super) fn cached_received(state: &AppState, message_id: &str) -> Result<Option<VoiceNote>> {
    let id = received_key(state, message_id)?;
    state
        .store
        .voice_note(&id)?
        .map(|(payload, _, _, _)| serde_json::from_str(&payload).map_err(Into::into))
        .transpose()
}
pub(super) async fn translate_received_inner(
    state: &AppState,
    message_id: &str,
) -> Result<VoiceNote> {
    if let Some(note) = cached_received(state, message_id)? {
        return Ok(note);
    }
    let id = received_key(state, message_id)?;
    let message = state
        .store
        .get_message_by_id(message_id)?
        .context("Message not found")?;
    if !state
        .store
        .get_conversation_settings(&message.contact_id)?
        .translation_enabled
    {
        bail!("Translation is off for this conversation. Enable it in Conversation settings.");
    }
    let (data, _) = state
        .store
        .get_message_media(message_id)?
        .context("Audio has not downloaded yet. Please try again.")?;
    let note = build_note(
        state,
        PrepareRequest {
            contact_id: message.contact_id,
            media_data: data,
            reply_only_if_not_latest: false,
            reply_to: None,
            reply_to_sender: None,
            reply_to_text: None,
        },
        true,
        id,
    )
    .await?;
    if !state
        .store
        .get_conversation_settings(&note.contact_id)?
        .translation_enabled
    {
        bail!("Translation was turned off while this voice note was processing.");
    }
    state
        .store
        .save_voice_note(&note.id, &serde_json::to_string(&note)?)?;
    if !message.is_from_me {
        state.store.update_voice_transcript(
            message_id,
            &note.transcript,
            &note.translation,
            &note.source_language,
        )?;
        // Signal cache readiness without rebroadcasting a message (which would increment unread counts).
        let _ = state
            .broadcast_tx
            .send(crate::web::WebSocketEvent::VoiceReady {
                message_id: message_id.to_string(),
            });
    }
    Ok(note)
}
