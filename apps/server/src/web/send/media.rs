use super::*;

pub(in crate::web) async fn translate_caption(
    state: &AppState,
    contact_id: &str,
    reply_id: Option<&str>,
    caption: Option<&str>,
) -> anyhow::Result<(Option<String>, Option<String>)> {
    let Some(text) = caption.filter(|text| !text.trim().is_empty()) else {
        return Ok((caption.map(str::to_string), None));
    };
    let language = crate::incoming::outgoing_language(state, contact_id, reply_id).await?;
    let Some(target) = language.as_deref() else {
        return Ok((Some(text.to_string()), None));
    };
    let translator = state
        .translator()
        .ok_or_else(|| anyhow::anyhow!("Translation is unavailable; image was not sent"))?;
    let (translated, usage) = translator.translate_outgoing(text, target).await?;
    state
        .store
        .record_usage(Some(contact_id), None, &usage, "translate_outgoing_caption")?;
    Ok((Some(translated), language))
}

pub(in crate::web) fn media_reply_needs_quote(
    state: &AppState,
    contact: &str,
    reply: Option<&str>,
    conditional: bool,
) -> Result<bool, String> {
    if state
        .store
        .validate_reply_only(contact, reply)
        .map_err(|e| e.to_string())?
    {
        return Ok(true);
    }
    if !conditional {
        return Ok(reply.is_some());
    }
    let id = reply.ok_or("Select a message in the destination conversation before replying")?;
    state
        .store
        .reply_is_latest(contact, id)
        .map(|latest| !latest)
        .map_err(|e| e.to_string())
}

pub(in crate::web) fn validate_attachment_payload(
    data: &str,
    mime: &str,
    kind: &str,
    filename: Option<&str>,
) -> Result<ValidatedImagePayload, String> {
    if kind == "image" {
        return validate_image_payload(data, mime);
    }
    if !matches!(kind, "video" | "document") {
        return Err("Unsupported attachment type".into());
    }
    let mime = mime.trim().to_ascii_lowercase();
    if mime.len() > 128
        || !mime.contains('/')
        || !mime
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/.-+".contains(&b))
    {
        return Err("Invalid attachment MIME type".into());
    }
    if kind == "video" && mime != "video/mp4" {
        return Err("Choose an MP4 video".into());
    }
    if kind == "document"
        && !filename.is_some_and(|name| {
            !name.trim().is_empty()
                && name.len() <= 255
                && !name
                    .chars()
                    .any(|c| c.is_control() || c == '/' || c == '\\')
        })
    {
        return Err("A valid file name is required".into());
    }
    if data.len() > MAX_ALBUM_BASE64_BYTES {
        return Err("Attachments must be smaller than 64 MB".into());
    }
    let decoded = BASE64_STANDARD
        .decode(data)
        .map_err(|_| "Invalid attachment data")?;
    if decoded.is_empty() || decoded.len() > MAX_ALBUM_BYTES {
        return Err("Choose a nonempty attachment smaller than 64 MB".into());
    }
    if kind == "video" && decoded.get(4..8) != Some(b"ftyp".as_slice()) {
        return Err("The video is not an MP4 file".into());
    }
    Ok(ValidatedImagePayload {
        mime_type: mime,
        decoded_size: decoded.len(),
    })
}
