//! JSON serialization of bridge events for web clients.

use super::*;

// Implement Serialize for BridgeEvent for JSON output mode
impl serde::Serialize for BridgeEvent {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;

        let mut map = serializer.serialize_map(None)?;

        match self {
            BridgeEvent::Qr { data } => {
                map.serialize_entry("type", "qr")?;
                map.serialize_entry("data", data)?;
            }
            BridgeEvent::Connected {
                phone,
                name,
                platform,
            } => {
                map.serialize_entry("type", "connected")?;
                map.serialize_entry("phone", phone)?;
                map.serialize_entry("name", name)?;
                if let Some(p) = platform {
                    map.serialize_entry("platform", p)?;
                }
            }
            BridgeEvent::ConnectionState { state } => {
                map.serialize_entry("type", "connection_state")?;
                map.serialize_entry("state", state)?;
            }
            BridgeEvent::Message(msg) => {
                map.serialize_entry("type", "message")?;
                map.serialize_entry("message", msg)?;
            }
            BridgeEvent::MessageEdit {
                message,
                edited_at_ms,
            } => {
                map.serialize_entry("type", "message_edit")?;
                map.serialize_entry("message", message)?;
                map.serialize_entry("edited_at_ms", edited_at_ms)?;
            }
            BridgeEvent::Error { code, message } => {
                map.serialize_entry("type", "error")?;
                map.serialize_entry("code", code)?;
                map.serialize_entry("message", message)?;
            }
            BridgeEvent::Log { level, message } => {
                map.serialize_entry("type", "log")?;
                map.serialize_entry("level", level)?;
                map.serialize_entry("message", message)?;
            }
            BridgeEvent::LoggedOut { reason } => {
                map.serialize_entry("type", "logged_out")?;
                map.serialize_entry("reason", reason)?;
            }
            BridgeEvent::SendResult {
                request_id,
                success,
                message_id,
                timestamp,
                message_ids,
                timestamps,
                error,
            } => {
                map.serialize_entry("type", "send_result")?;
                map.serialize_entry("request_id", request_id)?;
                map.serialize_entry("success", success)?;
                if let Some(id) = message_id {
                    map.serialize_entry("message_id", id)?;
                }
                if let Some(ts) = timestamp {
                    map.serialize_entry("timestamp", ts)?;
                }
                if !message_ids.is_empty() {
                    map.serialize_entry("message_ids", message_ids)?;
                }
                if !timestamps.is_empty() {
                    map.serialize_entry("timestamps", timestamps)?;
                }
                if let Some(err) = error {
                    map.serialize_entry("error", err)?;
                }
            }
            BridgeEvent::SendProgress {
                request_id,
                progress_id,
                stage,
                completed,
                total,
            } => {
                map.serialize_entry("type", "send_progress")?;
                map.serialize_entry("request_id", request_id)?;
                map.serialize_entry("progress_id", progress_id)?;
                map.serialize_entry("stage", stage)?;
                map.serialize_entry("completed", completed)?;
                map.serialize_entry("total", total)?;
            }
            BridgeEvent::ProfilePicture {
                request_id,
                jid,
                url,
                id,
                error,
            } => {
                map.serialize_entry("type", "profile_picture")?;
                map.serialize_entry("request_id", request_id)?;
                map.serialize_entry("jid", jid)?;
                if let Some(u) = url {
                    map.serialize_entry("url", u)?;
                }
                if let Some(i) = id {
                    map.serialize_entry("id", i)?;
                }
                if let Some(err) = error {
                    map.serialize_entry("error", err)?;
                }
            }
            BridgeEvent::ChatPresence {
                chat_id,
                user_id,
                state,
            } => {
                map.serialize_entry("type", "chat_presence")?;
                map.serialize_entry("chat_id", chat_id)?;
                map.serialize_entry("user_id", user_id)?;
                let state_str = match state {
                    bridge::ChatPresenceState::Typing => "typing",
                    bridge::ChatPresenceState::Paused => "paused",
                    bridge::ChatPresenceState::Recording => "recording",
                };
                map.serialize_entry("state", state_str)?;
            }
            BridgeEvent::ReadReceiptResult {
                message_id,
                success,
                error,
            } => {
                map.serialize_entry("type", "read_receipt_result")?;
                map.serialize_entry("message_id", message_id)?;
                map.serialize_entry("success", success)?;
                map.serialize_entry("error", error)?;
            }
            BridgeEvent::MarkAsRead { chat_id } => {
                map.serialize_entry("type", "mark_as_read")?;
                map.serialize_entry("chat_id", chat_id)?;
            }
            BridgeEvent::Receipt {
                message_ids,
                status,
            } => {
                map.serialize_entry("type", "receipt")?;
                map.serialize_entry("message_ids", message_ids)?;
                map.serialize_entry("status", status)?;
            }
        }

        map.end()
    }
}

// Implement Serialize for Message
impl serde::Serialize for bridge::Message {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;

        let mut s = serializer.serialize_struct("Message", 9)?;
        s.serialize_field("id", &self.id)?;
        s.serialize_field("timestamp", &self.timestamp.timestamp())?;
        s.serialize_field("from", &self.from)?;
        s.serialize_field("chat", &self.chat)?;
        s.serialize_field("content", &self.content)?;
        s.serialize_field("reply_context", &self.reply_context)?;
        s.serialize_field("is_from_me", &self.is_from_me)?;
        s.serialize_field("is_forwarded", &self.is_forwarded)?;
        s.serialize_field("push_name", &self.push_name)?;
        s.end()
    }
}

// Implement Serialize for Contact
impl serde::Serialize for bridge::Contact {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;

        let mut s = serializer.serialize_struct("Contact", 3)?;
        s.serialize_field("jid", &self.jid)?;
        s.serialize_field("phone", &self.phone)?;
        s.serialize_field("name", &self.name)?;
        s.end()
    }
}

// Implement Serialize for Chat
impl serde::Serialize for bridge::Chat {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;

        let mut map = serializer.serialize_map(None)?;

        match self {
            bridge::Chat::Private { jid, name } => {
                map.serialize_entry("type", "private")?;
                map.serialize_entry("jid", jid)?;
                if let Some(n) = name {
                    map.serialize_entry("name", n)?;
                }
            }
            bridge::Chat::Group {
                jid,
                name,
                participant_count,
            } => {
                map.serialize_entry("type", "group")?;
                map.serialize_entry("jid", jid)?;
                if let Some(n) = name {
                    map.serialize_entry("name", n)?;
                }
                if let Some(c) = participant_count {
                    map.serialize_entry("participant_count", c)?;
                }
            }
            bridge::Chat::Broadcast { jid } => {
                map.serialize_entry("type", "broadcast")?;
                map.serialize_entry("jid", jid)?;
            }
            bridge::Chat::Status { jid } => {
                map.serialize_entry("type", "status")?;
                map.serialize_entry("jid", jid)?;
            }
        }

        map.end()
    }
}

/// Write pixel dimensions only when both are known and non-zero, so clients never
/// reserve a degenerate frame for a photo or sticker that has not loaded yet.
pub(crate) fn serialize_dimensions<M: serde::ser::SerializeMap>(
    map: &mut M,
    width: Option<u32>,
    height: Option<u32>,
) -> std::result::Result<(), M::Error> {
    if let (Some(width), Some(height)) = (width, height) {
        if width > 0 && height > 0 {
            map.serialize_entry("width", &width)?;
            map.serialize_entry("height", &height)?;
        }
    }
    Ok(())
}

// Implement Serialize for MessageContent
impl serde::Serialize for bridge::MessageContent {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;

        let mut map = serializer.serialize_map(None)?;

        match self {
            bridge::MessageContent::Text { body } => {
                map.serialize_entry("type", "text")?;
                map.serialize_entry("body", body)?;
            }
            bridge::MessageContent::Image {
                caption,
                mime_type,
                file_size,
                file_hash,
                media_data,
                album_id,
                album_index,
                width,
                height,
            } => {
                map.serialize_entry("type", "image")?;
                map.serialize_entry("mime_type", mime_type)?;
                map.serialize_entry("file_size", file_size)?;
                if let Some(c) = caption {
                    map.serialize_entry("caption", c)?;
                }
                if let Some(h) = file_hash {
                    map.serialize_entry("file_hash", h)?;
                }
                if let Some(data) = media_data {
                    map.serialize_entry("media_data", data)?;
                }
                if let Some(id) = album_id {
                    map.serialize_entry("album_id", id)?;
                }
                if let Some(index) = album_index {
                    map.serialize_entry("album_index", index)?;
                }
                serialize_dimensions(&mut map, *width, *height)?;
            }
            bridge::MessageContent::Video {
                caption,
                mime_type,
                file_size,
                duration_seconds,
                media_data,
            } => {
                map.serialize_entry("type", "video")?;
                map.serialize_entry("mime_type", mime_type)?;
                map.serialize_entry("file_size", file_size)?;
                if let Some(c) = caption {
                    map.serialize_entry("caption", c)?;
                }
                if let Some(d) = duration_seconds {
                    map.serialize_entry("duration_seconds", d)?;
                }
                if let Some(data) = media_data {
                    map.serialize_entry("media_data", data)?;
                }
            }
            bridge::MessageContent::Audio {
                mime_type,
                file_size,
                duration_seconds,
                is_voice_note,
                media_data,
            } => {
                map.serialize_entry("type", "audio")?;
                map.serialize_entry("mime_type", mime_type)?;
                map.serialize_entry("file_size", file_size)?;
                map.serialize_entry("is_voice_note", is_voice_note)?;
                if let Some(d) = duration_seconds {
                    map.serialize_entry("duration_seconds", d)?;
                }
                if let Some(data) = media_data {
                    map.serialize_entry("media_data", data)?;
                }
            }
            bridge::MessageContent::Document {
                caption,
                mime_type,
                file_name,
                file_size,
                media_data,
            } => {
                map.serialize_entry("type", "document")?;
                map.serialize_entry("mime_type", mime_type)?;
                map.serialize_entry("file_size", file_size)?;
                if let Some(c) = caption {
                    map.serialize_entry("caption", c)?;
                }
                if let Some(n) = file_name {
                    map.serialize_entry("file_name", n)?;
                }
                if let Some(data) = media_data {
                    map.serialize_entry("media_data", data)?;
                }
            }
            bridge::MessageContent::Sticker {
                mime_type,
                is_animated,
                media_data,
                width,
                height,
            } => {
                map.serialize_entry("type", "sticker")?;
                map.serialize_entry("mime_type", mime_type)?;
                map.serialize_entry("is_animated", is_animated)?;
                if let Some(data) = media_data {
                    map.serialize_entry("media_data", data)?;
                }
                serialize_dimensions(&mut map, *width, *height)?;
            }
            bridge::MessageContent::Location {
                latitude,
                longitude,
                name,
                address,
            } => {
                map.serialize_entry("type", "location")?;
                map.serialize_entry("latitude", latitude)?;
                map.serialize_entry("longitude", longitude)?;
                if let Some(n) = name {
                    map.serialize_entry("name", n)?;
                }
                if let Some(a) = address {
                    map.serialize_entry("address", a)?;
                }
            }
            bridge::MessageContent::Contact {
                display_name,
                vcard,
            } => {
                map.serialize_entry("type", "contact")?;
                map.serialize_entry("display_name", display_name)?;
                map.serialize_entry("vcard", vcard)?;
            }
            bridge::MessageContent::Reaction {
                emoji,
                target_message_id,
                sender_timestamp_ms,
            } => {
                map.serialize_entry("type", "reaction")?;
                map.serialize_entry("emoji", emoji)?;
                map.serialize_entry("target_message_id", target_message_id)?;
                if let Some(timestamp) = sender_timestamp_ms {
                    map.serialize_entry("sender_timestamp_ms", timestamp)?;
                }
            }
            bridge::MessageContent::Revoked => {
                map.serialize_entry("type", "revoked")?;
            }
            bridge::MessageContent::Poll { question, options } => {
                map.serialize_entry("type", "poll")?;
                map.serialize_entry("question", question)?;
                map.serialize_entry("options", options)?;
            }
            bridge::MessageContent::Unknown { raw_type } => {
                map.serialize_entry("type", "unknown")?;
                map.serialize_entry("raw_type", raw_type)?;
            }
        }

        map.end()
    }
}
