//! Photo and album sending: payload validation, single images, albums and staged album uploads.

use super::*;

mod send_image;
mod send_images;

pub(super) use send_image::*;
pub(super) use send_images::*;

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
