//! Photo and album sending: payload validation, single images, albums and staged album uploads.

use super::*;

pub(super) fn validate_image_payload(
    media_data: &str,
    mime_type: &str,
) -> Result<ValidatedImagePayload, String> {
    if media_data.starts_with("data:") {
        return Err("media_data must be raw base64 without a data URL prefix".to_string());
    }

    let mime_type = normalize_image_mime_type(mime_type)?;
    if media_data.len() > MAX_IMAGE_BASE64_BYTES {
        return Err("Image is too large. Maximum decoded size is 16MB.".to_string());
    }

    let decoded = BASE64_STANDARD
        .decode(media_data)
        .map_err(|_| "media_data must be valid base64".to_string())?;
    if decoded.is_empty() {
        return Err("media_data must not decode to an empty image".to_string());
    }
    if decoded.len() > MAX_IMAGE_BYTES {
        return Err("Image is too large. Maximum decoded size is 16MB.".to_string());
    }
    if !image_signature_matches(&decoded, &mime_type) {
        return Err(format!("media_data does not match MIME type {}", mime_type));
    }

    Ok(ValidatedImagePayload {
        mime_type,
        decoded_size: decoded.len(),
    })
}

pub(super) fn validate_album_payloads(
    images: &[SendImageItemRequest],
) -> Result<Vec<ValidatedImagePayload>, String> {
    if !(2..=MAX_ALBUM_IMAGES).contains(&images.len()) {
        return Err(format!(
            "Choose between 2 and {} images for an album.",
            MAX_ALBUM_IMAGES
        ));
    }

    let validated = images
        .iter()
        .map(|image| validate_image_payload(&image.media_data, &image.mime_type))
        .collect::<Result<Vec<_>, _>>()?;
    let total_size: usize = validated.iter().map(|image| image.decoded_size).sum();
    if total_size > MAX_ALBUM_BYTES {
        return Err("The selected album is too large. Maximum combined size is 64MB.".to_string());
    }
    Ok(validated)
}

fn normalize_image_mime_type(mime_type: &str) -> Result<String, String> {
    let normalized = mime_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();

    match normalized.as_str() {
        "image/jpeg" | "image/png" | "image/gif" | "image/webp" => Ok(normalized),
        _ => Err("Unsupported image MIME type".to_string()),
    }
}

fn image_signature_matches(decoded: &[u8], mime_type: &str) -> bool {
    match mime_type {
        "image/jpeg" => decoded.starts_with(&[0xff, 0xd8, 0xff]),
        "image/png" => decoded.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/gif" => decoded.starts_with(b"GIF87a") || decoded.starts_with(b"GIF89a"),
        "image/webp" => {
            decoded.len() >= 12 && decoded.starts_with(b"RIFF") && &decoded[8..12] == b"WEBP"
        }
        _ => false,
    }
}

pub(super) async fn send_image(
    State(state): State<Arc<AppState>>,
    Json(mut req): Json<SendImageRequest>,
) -> impl IntoResponse {
    // Validate input
    if req.contact_id.is_empty() || req.media_data.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "contact_id and media_data are required"
            })),
        )
            .into_response();
    }

    let kind = req.media_kind.as_deref().unwrap_or("image");
    let content_type = match kind {
        "video" => "Video",
        "document" => "Document",
        _ => "Image",
    };
    let validated_image = match validate_attachment_payload(
        &req.media_data,
        &req.mime_type,
        kind,
        req.file_name.as_deref(),
    ) {
        Ok(payload) => payload,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": error
                })),
            )
                .into_response();
        }
    };
    let validated_mime_type = validated_image.mime_type.clone();
    let validated_decoded_size = validated_image.decoded_size;

    // Check if connected
    if !*state.connected.read().await {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "error": "Not connected to WhatsApp"
            })),
        )
            .into_response();
    }

    if let Err(error) = media_reply_needs_quote(
        &state,
        &req.contact_id,
        req.reply_to.as_deref(),
        req.reply_only_if_not_latest,
    ) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": error})),
        )
            .into_response();
    }

    let (translated_caption, caption_language) = match translate_caption(
        &state,
        &req.contact_id,
        req.reply_to.as_deref(),
        req.caption.as_deref(),
    )
    .await
    {
        Ok(result) => result,
        Err(error) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"error": error.to_string()})),
            )
                .into_response()
        }
    };
    let caption_changed = translated_caption != req.caption;
    match media_reply_needs_quote(
        &state,
        &req.contact_id,
        req.reply_to.as_deref(),
        req.reply_only_if_not_latest,
    ) {
        Ok(false) => {
            req.reply_to = None;
            req.reply_to_sender = None;
            req.reply_to_text = None;
            req.reply_to_sender_name = None;
        }
        Ok(true) => {}
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": error})),
            )
                .into_response()
        }
    }

    let request_id = state.next_request_id();
    let timestamp = chrono::Utc::now().timestamp_millis();
    let temp_message_id = format!("pending_img_{}_{}", request_id, timestamp);

    // Get contact info for the recipient
    let contact_info = state.store.get_contact(&req.contact_id).ok().flatten();
    let contact_name = contact_info.as_ref().and_then(|c| c.name.clone());
    let contact_phone = contact_info.as_ref().and_then(|c| c.phone.clone());
    let chat_type = contact_info
        .as_ref()
        .and_then(|c| c.contact_type.clone())
        .unwrap_or_else(|| "private".to_string());

    // Store the sent image message locally
    let stored_msg = crate::storage::StoredMessage {
        id: temp_message_id.clone(),
        contact_id: req.contact_id.clone(),
        timestamp,
        is_from_me: true,
        is_forwarded: false,
        sender_name: state.name.read().await.clone(),
        sender_phone: state.phone.read().await.clone(),
        contact_name,
        contact_phone,
        chat_type,
        content_type: content_type.to_string(),
        content_json: serde_json::json!({
            "type": kind,
            "file_name": req.file_name,
            "mime_type": validated_mime_type.clone(),
            "caption": translated_caption,
            "media_data": req.media_data,
            "file_size": validated_decoded_size,
            "reply_context": req.reply_to.as_ref().map(|reply_to| serde_json::json!({
                "messageId": reply_to,
                "senderName": req.reply_to_sender_name.clone().unwrap_or_else(|| {
                    req.reply_to_sender.clone().unwrap_or_else(|| "Unknown".to_string())
                }),
                "text": req.reply_to_text.clone().unwrap_or_default()
            }))
        })
        .to_string(),
        content: Some(serde_json::json!({
            "type": kind,
            "file_name": req.file_name,
            "mime_type": validated_mime_type.clone(),
            "caption": translated_caption,
            "media_data": req.media_data,
            "file_size": validated_decoded_size,
            "reply_context": req.reply_to.as_ref().map(|reply_to| serde_json::json!({
                "messageId": reply_to,
                "senderName": req.reply_to_sender_name.clone().unwrap_or_else(|| {
                    req.reply_to_sender.clone().unwrap_or_else(|| "Unknown".to_string())
                }),
                "text": req.reply_to_text.clone().unwrap_or_default()
            }))
        })),
        original_text: req.caption.clone(),
        translated_text: if caption_changed {
            translated_caption.clone()
        } else {
            None
        },
        source_language: caption_language.clone(),
        is_translated: caption_changed,
        delivery_status: Some("sending".to_string()),
    };

    // Store the message
    if let Err(e) = state.store.add_message(&stored_msg) {
        error!("Failed to store sent image: {}", e);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Failed to store pending attachment: {}", e)
            })),
        )
            .into_response();
    }

    // Update contact's last message time (preserve contact name/phone)
    if let Err(e) = state.store.upsert_contact(
        &stored_msg.contact_id,
        stored_msg.contact_name.as_deref(),
        stored_msg.contact_phone.as_deref(),
        Some(&stored_msg.chat_type),
        stored_msg.timestamp,
    ) {
        error!("Failed to update contact: {}", e);
        let _ = state.store.delete_message(&temp_message_id);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Failed to update contact: {}", e)
            })),
        )
            .into_response();
    }

    let rx = state
        .register_pending_send(request_id, Some(temp_message_id.clone()))
        .await;

    let cmd = if kind == "image" {
        BridgeCommand::SendImage {
            request_id: Some(request_id),
            to: req.contact_id.clone(),
            media_data: req.media_data.clone(),
            mime_type: validated_mime_type,
            caption: translated_caption.clone(),
            reply_to: req.reply_to.clone(),
            reply_to_sender: req.reply_to_sender.clone(),
            reply_to_text: req.reply_to_text.clone(),
        }
    } else {
        BridgeCommand::SendMedia {
            request_id: Some(request_id),
            to: req.contact_id.clone(),
            media_data: req.media_data.clone(),
            mime_type: validated_mime_type,
            media_kind: kind.to_string(),
            file_name: req.file_name.clone(),
            caption: translated_caption.clone(),
            reply_to: req.reply_to.clone(),
            reply_to_sender: req.reply_to_sender.clone(),
            reply_to_text: req.reply_to_text.clone(),
        }
    };

    if let Err(e) = state.send_bridge_command(cmd).await {
        error!("Failed to send attachment: {}", e);
        state.cancel_pending_send(request_id).await;
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Failed to send attachment: {}", e)
            })),
        )
            .into_response();
    }

    let send_result = match state
        .wait_for_send_result_with_timeout(request_id, rx, ALBUM_SEND_RESULT_TIMEOUT)
        .await
    {
        Ok(result) => result,
        Err(SendConfirmationError::Timeout) => {
            return (
                StatusCode::GATEWAY_TIMEOUT,
                Json(serde_json::json!({
                    "error": "Timed out waiting for WhatsApp attachment send confirmation"
                })),
            )
                .into_response();
        }
        Err(SendConfirmationError::ChannelClosed) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({
                    "error": "WhatsApp attachment send confirmation channel closed"
                })),
            )
                .into_response();
        }
    };

    if !send_result.success {
        return (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({
                "error": send_result.error.unwrap_or_else(|| "WhatsApp rejected the attachment".to_string())
            })),
        )
            .into_response();
    }

    let confirmed_message_id = send_result.message_id.unwrap_or(temp_message_id);
    let confirmed_timestamp = send_result.timestamp.unwrap_or(timestamp);

    Json(SendImageResponse {
        message_id: confirmed_message_id,
        timestamp: confirmed_timestamp,
    })
    .into_response()
}

pub(super) async fn send_images(
    State(state): State<Arc<AppState>>,
    Json(mut req): Json<SendImagesRequest>,
) -> impl IntoResponse {
    if req.contact_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "contact_id is required" })),
        )
            .into_response();
    }

    let validated = match validate_album_payloads(&req.images) {
        Ok(images) => images,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": error })),
            )
                .into_response();
        }
    };

    if !*state.connected.read().await {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "Not connected to WhatsApp" })),
        )
            .into_response();
    }

    if let Err(error) = media_reply_needs_quote(
        &state,
        &req.contact_id,
        req.reply_to.as_deref(),
        req.reply_only_if_not_latest,
    ) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": error})),
        )
            .into_response();
    }

    let (translated_caption, caption_language) = match translate_caption(
        &state,
        &req.contact_id,
        req.reply_to.as_deref(),
        req.caption.as_deref(),
    )
    .await
    {
        Ok(result) => result,
        Err(error) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"error": error.to_string()})),
            )
                .into_response()
        }
    };
    let caption_changed = translated_caption != req.caption;
    match media_reply_needs_quote(
        &state,
        &req.contact_id,
        req.reply_to.as_deref(),
        req.reply_only_if_not_latest,
    ) {
        Ok(false) => {
            req.reply_to = None;
            req.reply_to_sender = None;
            req.reply_to_text = None;
            req.reply_to_sender_name = None;
        }
        Ok(true) => {}
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": error})),
            )
                .into_response()
        }
    }

    let request_id = state.next_request_id();
    let rx = state.register_pending_send(request_id, None).await;
    let command = BridgeCommand::SendImages {
        request_id: Some(request_id),
        progress_id: req.progress_id.clone(),
        to: req.contact_id.clone(),
        images: req
            .images
            .iter()
            .zip(validated.iter())
            .map(|(image, validated)| BridgeImage {
                media_data: image.media_data.clone(),
                mime_type: validated.mime_type.clone(),
            })
            .collect(),
        caption: translated_caption.clone(),
        reply_to: req.reply_to.clone(),
        reply_to_sender: req.reply_to_sender.clone(),
        reply_to_text: req.reply_to_text.clone(),
    };

    if let Err(error) = state.send_bridge_command(command).await {
        state.cancel_pending_send(request_id).await;
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Failed to send album: {}", error) })),
        )
            .into_response();
    }

    let send_result = match state
        .wait_for_send_result_with_timeout(
            request_id,
            rx,
            album_send_result_timeout(req.images.len()),
        )
        .await
    {
        Ok(result) => result,
        Err(SendConfirmationError::Timeout) => {
            return (
                StatusCode::GATEWAY_TIMEOUT,
                Json(serde_json::json!({ "error": "Timed out waiting for WhatsApp album confirmation" })),
            )
                .into_response();
        }
        Err(SendConfirmationError::ChannelClosed) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": "WhatsApp album confirmation channel closed" })),
            )
                .into_response();
        }
    };

    if !send_result.success {
        return (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({
                "error": send_result.error.unwrap_or_else(|| "WhatsApp rejected the album".to_string())
            })),
        )
            .into_response();
    }
    if send_result.message_ids.len() != req.images.len() {
        return (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": "WhatsApp returned an incomplete album confirmation" })),
        )
            .into_response();
    }

    let now = chrono::Utc::now().timestamp_millis();
    let contact_info = state.store.get_contact(&req.contact_id).ok().flatten();
    let contact_name = contact_info
        .as_ref()
        .and_then(|contact| contact.name.clone());
    let contact_phone = contact_info
        .as_ref()
        .and_then(|contact| contact.phone.clone());
    let chat_type = contact_info
        .as_ref()
        .and_then(|contact| contact.contact_type.clone())
        .unwrap_or_else(|| "private".to_string());
    let sender_name = state.name.read().await.clone();
    let sender_phone = state.phone.read().await.clone();
    let mut stored_ids: Vec<String> = Vec::new();
    let album_id = send_result
        .message_id
        .clone()
        .unwrap_or_else(|| send_result.message_ids[0].clone());

    for (index, ((image, validated), message_id)) in req
        .images
        .iter()
        .zip(validated.iter())
        .zip(send_result.message_ids.iter())
        .enumerate()
    {
        let timestamp = send_result
            .timestamps
            .get(index)
            .copied()
            .unwrap_or(now + index as i64);
        let caption = if index == 0 {
            translated_caption.clone()
        } else {
            None
        };
        let reply_context = if index == 0 {
            req.reply_to.as_ref().map(|reply_to| {
                serde_json::json!({
                    "messageId": reply_to,
                    "senderName": req.reply_to_sender_name.clone().unwrap_or_else(|| {
                        req.reply_to_sender.clone().unwrap_or_else(|| "Unknown".to_string())
                    }),
                    "text": req.reply_to_text.clone().unwrap_or_default()
                })
            })
        } else {
            None
        };
        let content = serde_json::json!({
            "type": "image",
            "mime_type": validated.mime_type,
            "caption": caption,
            "media_data": image.media_data,
            "file_size": validated.decoded_size,
            "reply_context": reply_context,
            "album_id": album_id,
            "album_index": index
        });
        let stored_message = StoredMessage {
            id: message_id.clone(),
            contact_id: req.contact_id.clone(),
            timestamp,
            is_from_me: true,
            is_forwarded: false,
            sender_name: sender_name.clone(),
            sender_phone: sender_phone.clone(),
            contact_name: contact_name.clone(),
            contact_phone: contact_phone.clone(),
            chat_type: chat_type.clone(),
            content_type: "Image".to_string(),
            content_json: content.to_string(),
            content: Some(content),
            original_text: if index == 0 {
                req.caption.clone()
            } else {
                None
            },
            translated_text: if index == 0 && caption_changed {
                translated_caption.clone()
            } else {
                None
            },
            source_language: if index == 0 {
                caption_language.clone()
            } else {
                None
            },
            is_translated: index == 0 && caption_changed,
            delivery_status: Some("sent".to_string()),
        };
        if let Err(error) = state.store.add_message(&stored_message) {
            for stored_id in &stored_ids {
                let _ = state.store.delete_message(stored_id);
            }
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Failed to store sent album: {}", error) })),
            )
                .into_response();
        }
        stored_ids.push(message_id.clone());
    }

    let last_timestamp = send_result.timestamps.last().copied().unwrap_or(now);
    if let Err(error) = state.store.upsert_contact(
        &req.contact_id,
        contact_name.as_deref(),
        contact_phone.as_deref(),
        Some(&chat_type),
        last_timestamp,
    ) {
        warn!("Failed to update contact after album send: {}", error);
    }

    Json(SendImageResponse {
        message_id: album_id,
        timestamp: send_result.timestamp.unwrap_or(now),
    })
    .into_response()
}

pub(super) async fn create_photo_album(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreatePhotoAlbumRequest>,
) -> Response {
    let valid_job_id = (8..=128).contains(&req.job_id.len())
        && req
            .job_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
    if !valid_job_id
        || req.contact_id.is_empty()
        || !(2..=MAX_ALBUM_IMAGES).contains(&req.photo_count)
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Invalid album job details" })),
        )
            .into_response();
    }

    if let Err(error) = media_reply_needs_quote(
        &state,
        &req.contact_id,
        req.reply_to.as_deref(),
        req.reply_only_if_not_latest,
    ) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error":error})),
        )
            .into_response();
    }

    let now = chrono::Utc::now().timestamp();
    let mut albums = state.pending_photo_albums.write().await;
    albums.retain(|_, album| now - album.created_at < 60 * 60);
    if albums.len() >= 4 && !albums.contains_key(&req.job_id) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(serde_json::json!({ "error": "Too many photo albums are waiting to send" })),
        )
            .into_response();
    }
    albums.insert(
        req.job_id,
        PendingPhotoAlbum {
            reply_only_if_not_latest: req.reply_only_if_not_latest,
            contact_id: req.contact_id,
            caption: req.caption,
            reply_to: req.reply_to,
            reply_to_sender: req.reply_to_sender,
            reply_to_text: req.reply_to_text,
            reply_to_sender_name: req.reply_to_sender_name,
            images: vec![None; req.photo_count],
            decoded_bytes: 0,
            created_at: now,
        },
    );
    Json(serde_json::json!({ "success": true })).into_response()
}

pub(super) async fn stage_photo_album_image(
    State(state): State<Arc<AppState>>,
    Path((job_id, index)): Path<(String, usize)>,
    Json(image): Json<SendImageItemRequest>,
) -> Response {
    let validated = match validate_image_payload(&image.media_data, &image.mime_type) {
        Ok(value) => value,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": error })),
            )
                .into_response();
        }
    };
    let mut albums = state.pending_photo_albums.write().await;
    let all_pending_bytes: usize = albums.values().map(|album| album.decoded_bytes).sum();
    let Some(album) = albums.get_mut(&job_id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Photo album job expired" })),
        )
            .into_response();
    };
    if index >= album.images.len() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Photo index is outside this album" })),
        )
            .into_response();
    }
    let previous_size = album.images[index]
        .as_ref()
        .and_then(|previous| validate_image_payload(&previous.media_data, &previous.mime_type).ok())
        .map_or(0, |value| value.decoded_size);
    let next_total = album.decoded_bytes - previous_size + validated.decoded_size;
    let next_pending_total = all_pending_bytes - previous_size + validated.decoded_size;
    if next_total > MAX_ALBUM_BYTES || next_pending_total > MAX_PENDING_ALBUM_BYTES {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(
                serde_json::json!({ "error": "The album exceeds WhatsApp's combined photo limit" }),
            ),
        )
            .into_response();
    }
    album.decoded_bytes = next_total;
    album.images[index] = Some(SendImageItemRequest {
        media_data: image.media_data,
        mime_type: validated.mime_type,
    });
    Json(
        serde_json::json!({ "success": true, "completed": index + 1, "total": album.images.len() }),
    )
    .into_response()
}

pub(super) async fn send_staged_photo_album(
    State(state): State<Arc<AppState>>,
    Path(job_id): Path<String>,
) -> Response {
    let album = state.pending_photo_albums.write().await.remove(&job_id);
    let Some(album) = album else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Photo album job expired" })),
        )
            .into_response();
    };
    if album.images.iter().any(Option::is_none) {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": "Not all photos have transferred" })),
        )
            .into_response();
    }
    let req = SendImagesRequest {
        reply_only_if_not_latest: album.reply_only_if_not_latest,
        contact_id: album.contact_id,
        progress_id: Some(job_id),
        images: album.images.into_iter().flatten().collect(),
        caption: album.caption,
        reply_to: album.reply_to,
        reply_to_sender: album.reply_to_sender,
        reply_to_text: album.reply_to_text,
        reply_to_sender_name: album.reply_to_sender_name,
    };
    send_images(State(state), Json(req)).await.into_response()
}
