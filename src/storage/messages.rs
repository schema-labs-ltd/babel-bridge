//! Message storage, lookup and pagination.

use super::*;
mod queries;

impl MessageStore {
    /// Replays restore metadata older bridges omitted, without replacing edits or media.
    pub fn recover_reply_context(&self, message: &StoredMessage) -> Result<()> {
        let incoming: serde_json::Value = serde_json::from_str(&message.content_json)?;
        let conn = self.conn.lock().unwrap();
        if let Some(sender) = incoming.get("receipt_sender_jid").and_then(|v| v.as_str()) {
            conn.execute(
                "UPDATE messages SET content_json=json_set(content_json,'$.receipt_sender_jid',?1) WHERE id=?2 AND contact_id=?3 AND is_from_me=0 AND sender_phone IS ?4 AND json_extract(content_json,'$.receipt_sender_jid') IS NULL",
                params![sender, message.id, canonical_chat_id(&message.contact_id).as_ref(), message.sender_phone],
            )?;
            conn.execute(
                "UPDATE viewed_messages SET sender_jid=(SELECT json_extract(content_json,'$.receipt_sender_jid') FROM messages WHERE id=?1) WHERE message_id=?1 AND sender_jid IS NULL",
                [&message.id],
            )?;
        }
        if let Some(quote) = incoming.get("reply_context").filter(|q| q.is_object()) {
            conn.execute(
                "UPDATE messages SET content_json=json_set(content_json,'$.reply_context',json(?1)) WHERE id=?2 AND contact_id=?3 AND is_from_me=?4 AND sender_phone IS ?5 AND json_extract(content_json,'$.reply_context') IS NULL",
                params![quote.to_string(), message.id, canonical_chat_id(&message.contact_id).as_ref(), message.is_from_me, message.sender_phone],
            )?;
        }
        Ok(())
    }

    /// Add a message to the store
    pub fn add_message(&self, msg: &StoredMessage) -> Result<()> {
        self.add_message_with_notification(msg, None)
    }

    /// Persist live notification eligibility with the message so a restart cannot
    /// lose an alert while translation is pending. History never opts in.
    pub fn add_message_with_notification(
        &self,
        msg: &StoredMessage,
        notification_requires_translation: Option<bool>,
    ) -> Result<()> {
        let contact_id = canonical_chat_id(&msg.contact_id);
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let preview = Self::generate_message_preview(
            Some(&msg.content_json),
            Some(&msg.content_type),
            msg.is_from_me,
            msg.translated_text.as_deref(),
        );
        // Every send path (native/web, MCP and original-language follow-ups)
        // must retain its text for search and topic classification.
        let outgoing_text = if msg.is_from_me
            && msg
                .original_text
                .as_deref()
                .is_none_or(|s| s.trim().is_empty())
        {
            serde_json::from_str::<serde_json::Value>(&msg.content_json)
                .ok()
                .and_then(|content| {
                    match content.get("type").and_then(serde_json::Value::as_str) {
                        Some("text") => content.get("body"),
                        Some("image" | "video" | "document") => content.get("caption"),
                        _ => None,
                    }
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
                })
        } else {
            None
        };
        let original_text = outgoing_text.as_ref().or(msg.original_text.as_ref());

        let inserted = tx.execute(
            r#"
            INSERT OR IGNORE INTO messages 
            (id, contact_id, timestamp, is_from_me, is_forwarded, sender_name, sender_phone, 
             chat_type, content_type, content_json, original_text, translated_text, 
             source_language, is_translated, delivery_status)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
            "#,
            params![
                msg.id,
                contact_id,
                msg.timestamp,
                msg.is_from_me,
                msg.is_forwarded,
                msg.sender_name,
                msg.sender_phone,
                msg.chat_type,
                msg.content_type,
                msg.content_json,
                original_text,
                msg.translated_text,
                msg.source_language,
                msg.is_translated,
                msg.delivery_status,
            ],
        )?;

        tx.execute(
            r#"
            UPDATE contacts
            SET last_message_time = MAX(last_message_time, ?2),
                last_message_preview = ?3
            WHERE id = ?1
            "#,
            params![contact_id, msg.timestamp, preview],
        )?;
        if inserted > 0 && !msg.is_from_me {
            if notification_requires_translation.is_some()
                && !matches!(
                    msg.content_type.to_lowercase().as_str(),
                    "reaction" | "revoked" | "protocol" | "unknown"
                )
            {
                tx.execute(
                    "INSERT OR IGNORE INTO unread_messages(message_id) VALUES (?)",
                    [&msg.id],
                )?;
                tx.execute(
                    "UPDATE contacts SET unread_count=unread_count+1 WHERE id=?",
                    [contact_id.as_ref()],
                )?;
            }
            if let Some(requires_translation) = notification_requires_translation {
                let requires_translation = requires_translation && tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM contacts WHERE id=? AND translation_enabled=1)",
                    params![contact_id],
                    |row| row.get::<_, bool>(0),
                )?;
                tx.execute(
                    "INSERT INTO pending_notifications(message_id, requires_translation) VALUES (?, ?)",
                    params![msg.id, requires_translation],
                )?;
                if requires_translation && msg.source_language.is_none() {
                    tx.execute(
                        "INSERT OR IGNORE INTO translation_jobs(message_id) VALUES (?)",
                        params![msg.id],
                    )?;
                }
            }
        }
        Self::apply_pending_edit(&tx, &msg.id)?;
        tx.commit()?;
        Ok(())
    }

    pub fn update_delivery_status(&self, message_ids: &[String], status: &str) -> Result<usize> {
        if message_ids.is_empty() || !matches!(status, "delivered" | "read") {
            return Ok(0);
        }
        let conn = self.conn.lock().unwrap();
        let mut updated = 0;
        for message_id in message_ids {
            updated += conn.execute(
                r#"
                UPDATE messages
                SET delivery_status = CASE
                    WHEN delivery_status = 'read' THEN 'read'
                    WHEN ?2 = 'read' THEN 'read'
                    ELSE 'delivered'
                END
                WHERE id = ?1 AND is_from_me = 1
                "#,
                params![message_id, status],
            )?;
        }
        Ok(updated)
    }

    /// Update the translation for an existing message
    pub fn update_message_translation(
        &self,
        message_id: &str,
        translated_text: Option<&str>,
        source_language: Option<&str>,
    ) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;

        tx.execute(
            r#"
            UPDATE messages 
            SET translated_text = ?1, source_language = ?2, is_translated = 1
            WHERE id = ?3
            "#,
            params![translated_text, source_language, message_id],
        )?;

        let contact_id: Option<String> = tx
            .query_row(
                "SELECT contact_id FROM messages WHERE id = ?",
                params![message_id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(contact_id) = contact_id {
            Self::refresh_contact_last_message(&tx, &contact_id)?;
        }

        tx.commit()?;

        Ok(())
    }

    /// Replace a temporary outgoing message ID with the bridge-confirmed WhatsApp ID.
    pub fn replace_message_id(
        &self,
        temp_message_id: &str,
        confirmed_message_id: &str,
        confirmed_timestamp: Option<i64>,
    ) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;

        let temp_contact_id: Option<String> = tx
            .query_row(
                "SELECT contact_id FROM messages WHERE id = ?",
                params![temp_message_id],
                |row| row.get(0),
            )
            .optional()?;

        let Some(contact_id) = temp_contact_id else {
            return Ok(());
        };

        let confirmed_exists: bool = tx
            .query_row(
                "SELECT 1 FROM messages WHERE id = ?",
                params![confirmed_message_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();

        if confirmed_exists && temp_message_id != confirmed_message_id {
            tx.execute(
                "DELETE FROM messages WHERE id = ?",
                params![temp_message_id],
            )?;
        } else {
            tx.execute(
                r#"
                UPDATE messages
                SET id = ?1,
                    timestamp = COALESCE(?2, timestamp)
                WHERE id = ?3
                "#,
                params![confirmed_message_id, confirmed_timestamp, temp_message_id],
            )?;
        }

        Self::refresh_contact_last_message(&tx, &contact_id)?;
        tx.commit()?;
        Ok(())
    }

    /// Delete a message and refresh the owning contact's last-message summary.
    pub fn delete_message(&self, message_id: &str) -> Result<bool> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;

        let contact_id: Option<String> = tx
            .query_row(
                "SELECT contact_id FROM messages WHERE id = ?",
                params![message_id],
                |row| row.get(0),
            )
            .optional()?;

        let Some(contact_id) = contact_id else {
            return Ok(false);
        };

        let deleted = tx.execute("DELETE FROM messages WHERE id = ?", params![message_id])?;
        Self::refresh_contact_last_message(&tx, &contact_id)?;
        tx.commit()?;
        Ok(deleted > 0)
    }

    pub(super) fn refresh_contact_last_message(
        conn: &rusqlite::Transaction<'_>,
        contact_id: &str,
    ) -> Result<()> {
        let contact_id = canonical_chat_id(contact_id);
        let contact_id = contact_id.as_ref();
        let latest: Option<(i64, String, String, bool, Option<String>)> = conn
            .query_row(
                r#"
                SELECT timestamp, content_json, content_type, is_from_me, translated_text
                FROM messages
                WHERE contact_id = ?
                ORDER BY timestamp DESC, id DESC
                LIMIT 1
                "#,
                params![contact_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()?;

        if let Some((timestamp, content_json, content_type, is_from_me, translated_text)) = latest {
            let preview = Self::generate_message_preview(
                Some(&content_json),
                Some(&content_type),
                is_from_me,
                translated_text.as_deref(),
            );
            conn.execute(
                r#"
                UPDATE contacts
                SET last_message_time = ?2,
                    last_message_preview = ?3
                WHERE id = ?1
                "#,
                params![contact_id, timestamp, preview],
            )?;
        } else {
            conn.execute(
                r#"
                UPDATE contacts
                SET last_message_time = 0,
                    last_message_preview = NULL
                WHERE id = ?1
                "#,
                params![contact_id],
            )?;
        }

        Ok(())
    }

    /// Get messages for a specific contact (all messages - for MCP/internal use)
    pub fn get_messages(&self, contact_id: &str) -> Result<Vec<StoredMessage>> {
        let contact_id = canonical_chat_id(contact_id);
        let contact_id = contact_id.as_ref();
        self.get_messages_paginated(contact_id, None, None, None, false)
    }

    /// Strip media_data from content JSON to reduce payload size
    pub(super) fn strip_media_from_content(
        content_json: &str,
    ) -> (String, Option<serde_json::Value>) {
        if let Ok(mut content) = serde_json::from_str::<serde_json::Value>(content_json) {
            // Check if this content has media_data
            let has_media =
                content.get("media_data").is_some() || content.get("mediaData").is_some();

            if has_media {
                // Remove media_data from content
                if let Some(obj) = content.as_object_mut() {
                    obj.remove("media_data");
                    obj.remove("mediaData");
                    // Add a flag to indicate media is available
                    obj.insert("has_media".to_string(), serde_json::Value::Bool(true));
                }
                let stripped_json =
                    serde_json::to_string(&content).unwrap_or_else(|_| content_json.to_string());
                return (stripped_json, Some(content));
            }

            (content_json.to_string(), Some(content))
        } else {
            (content_json.to_string(), None)
        }
    }

    /// Helper to convert a row to StoredMessage (used by multiple methods)
    pub(super) fn row_to_stored_message(
        row: &rusqlite::Row,
        contact_name: Option<String>,
        contact_phone: Option<String>,
    ) -> rusqlite::Result<StoredMessage> {
        let content_json: String = row.get(9)?;
        let content = serde_json::from_str(&content_json).ok();

        Ok(StoredMessage {
            id: row.get(0)?,
            contact_id: row.get(1)?,
            timestamp: row.get(2)?,
            is_from_me: row.get::<_, i32>(3)? != 0,
            is_forwarded: row.get::<_, i32>(4).unwrap_or(0) != 0,
            sender_name: row.get(5)?,
            sender_phone: row.get(6)?,
            contact_name,
            contact_phone,
            chat_type: row
                .get::<_, Option<String>>(7)?
                .unwrap_or_else(|| "private".to_string()),
            content_type: row.get(8)?,
            content_json,
            content,
            original_text: row.get(10)?,
            translated_text: row.get(11)?,
            source_language: row.get(12)?,
            is_translated: row.get::<_, i32>(13).unwrap_or(0) != 0,
            delivery_status: row.get(14)?,
        })
    }
}
