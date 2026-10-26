//! Message storage, lookup and pagination.

use super::*;

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

    /// Bounded, stable timeline across conversations. Media remains lazy-loaded.
    pub fn get_unified_messages(
        &self,
        limit: u32,
        before: Option<i64>,
        before_id: Option<&str>,
    ) -> Result<Vec<StoredMessage>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            r#"SELECT m.id, m.contact_id, m.timestamp, m.is_from_me, m.is_forwarded,
                      m.sender_name, m.sender_phone, m.chat_type, m.content_type,
                      m.content_json, m.original_text, m.translated_text, m.source_language,
                      m.is_translated, m.delivery_status, c.name, c.phone
               FROM messages m LEFT JOIN contacts c ON c.id = m.contact_id
               WHERE m.contact_id != 'status@broadcast'
                 AND lower(m.content_type) != 'reaction'
                 AND lower(COALESCE(json_extract(m.content_json, '$.type'), '')) != 'reaction'
                 AND (?1 IS NULL OR m.timestamp < ?1 OR
                     (m.timestamp = ?1 AND ?2 IS NOT NULL AND m.id < ?2))
               ORDER BY m.timestamp DESC, m.id DESC LIMIT ?3"#,
        )?;
        let mut messages = stmt
            .query_map(params![before, before_id, limit], |row| {
                Self::row_to_stored_message(row, row.get(15)?, row.get(16)?)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for message in &mut messages {
            let (json, content) = Self::strip_media_from_content(&message.content_json);
            message.content_json = json;
            message.content = content;
        }
        messages.reverse();
        Ok(messages)
    }

    /// Returns whether the conversation requires a quote. Fail closed on storage errors.
    pub fn validate_reply_only(&self, contact_id: &str, reply: Option<&str>) -> Result<bool> {
        if !self.get_conversation_settings(contact_id)?.reply_only {
            return Ok(false);
        }
        let error = "Reply only is on. Select an incoming message in this conversation and choose Reply before sending.";
        let id = reply.context(error)?;
        let message = self.get_message_by_id(id)?.context(error)?;
        anyhow::ensure!(
            message.contact_id == canonical_chat_id(contact_id)
                && !message.is_from_me
                && !["reaction", "revoked"].contains(&message.content_type.to_lowercase().as_str())
                && !["reaction", "revoked"].contains(
                    &message
                        .content
                        .as_ref()
                        .and_then(|c| c.get("type"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_lowercase()
                        .as_str()
                ),
            error
        );
        Ok(true)
    }

    /// The target must belong to this conversation. Reactions do not change reply context.
    pub fn reply_is_latest(&self, contact_id: &str, message_id: &str) -> Result<bool> {
        let contact_id = canonical_chat_id(contact_id);
        let contact_id = contact_id.as_ref();
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            r#"SELECT id = (
                  SELECT id FROM messages WHERE contact_id = ?1
                    AND lower(content_type) != 'reaction'
                    AND lower(COALESCE(json_extract(content_json, '$.type'), '')) != 'reaction'
                  ORDER BY timestamp DESC, id DESC LIMIT 1)
               FROM messages WHERE contact_id = ?1 AND id = ?2
                 AND lower(content_type) != 'reaction'
                 AND lower(COALESCE(json_extract(content_json, '$.type'), '')) != 'reaction'"#,
            params![contact_id, message_id],
            |row| row.get(0),
        )
        .optional()?
        .context("The selected message is no longer in this conversation")
    }

    /// Get media data for a specific message
    /// Returns the media_data and mime_type for a message
    pub fn get_message_media(&self, message_id: &str) -> Result<Option<(String, Option<String>)>> {
        let conn = self.conn.lock().unwrap();

        let result = conn.query_row(
            "SELECT content_json FROM messages WHERE id = ?",
            params![message_id],
            |row| row.get::<_, String>(0),
        );

        match result {
            Ok(content_json) => {
                // Parse and extract media_data and mime_type
                if let Ok(content) = serde_json::from_str::<serde_json::Value>(&content_json) {
                    let media_data = content
                        .get("media_data")
                        .or_else(|| content.get("mediaData"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());

                    let mime_type = content
                        .get("mime_type")
                        .or_else(|| content.get("mimeType"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());

                    if let Some(media) = media_data {
                        return Ok(Some((media, mime_type)));
                    }
                }
                Ok(None)
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
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

    /// Get messages for a specific contact with pagination
    /// - limit: max number of messages to return (default: all)
    /// - before_timestamp: only get messages before this timestamp (for loading older messages)
    /// - before_message_id: tie-breaker cursor for messages with the same timestamp
    /// - strip_media: if true, remove media_data from content to reduce payload size
    ///
    /// Returns messages in ascending order by timestamp and ID (oldest first)
    pub fn get_messages_paginated(
        &self,
        contact_id: &str,
        limit: Option<u32>,
        before_timestamp: Option<i64>,
        before_message_id: Option<&str>,
        strip_media: bool,
    ) -> Result<Vec<StoredMessage>> {
        self.get_messages_page(
            contact_id,
            limit,
            before_timestamp,
            before_message_id,
            strip_media,
            false,
        )
    }

    pub fn get_gallery_messages(
        &self,
        contact_id: &str,
        limit: u32,
        before_timestamp: Option<i64>,
        before_message_id: Option<&str>,
    ) -> Result<Vec<StoredMessage>> {
        self.get_messages_page(
            contact_id,
            Some(limit),
            before_timestamp,
            before_message_id,
            true,
            true,
        )
    }

    pub fn message_is_visual_media(&self, message_id: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        Ok(conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM messages WHERE id=? AND lower(content_type) IN ('image','video') AND lower(COALESCE(json_extract(content_json,'$.type'),content_type)) IN ('image','video'))",
            params![message_id], |row| row.get(0),
        )?)
    }

    fn get_messages_page(
        &self,
        contact_id: &str,
        limit: Option<u32>,
        before_timestamp: Option<i64>,
        before_message_id: Option<&str>,
        strip_media: bool,
        visual_only: bool,
    ) -> Result<Vec<StoredMessage>> {
        let contact_id = canonical_chat_id(contact_id);
        let contact_id = contact_id.as_ref();
        let conn = self.conn.lock().unwrap();

        // First get the contact info to populate contact_name and contact_phone
        let contact_info: Option<(Option<String>, Option<String>)> = conn
            .query_row(
                "SELECT name, phone FROM contacts WHERE id = ?",
                params![contact_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();

        let (contact_name, contact_phone) = contact_info.unwrap_or((None, None));

        let cursor_clause = if before_timestamp.is_some() && before_message_id.is_some() {
            "AND (timestamp < ? OR (timestamp = ? AND id < ?))"
        } else if before_timestamp.is_some() {
            "AND timestamp < ?"
        } else {
            ""
        };
        let order_clause = if limit.is_some() {
            "ORDER BY timestamp DESC, id DESC"
        } else {
            "ORDER BY timestamp ASC, id ASC"
        };
        let limit_clause = limit
            .map(|lim| format!("LIMIT {}", lim))
            .unwrap_or_default();
        let media_clause = if visual_only {
            "AND lower(content_type) IN ('image','video') AND lower(COALESCE(json_extract(content_json,'$.type'),content_type)) IN ('image','video')"
        } else {
            ""
        };
        let query = format!(
            r#"
            SELECT id, contact_id, timestamp, is_from_me, is_forwarded, sender_name,
                   sender_phone, chat_type, content_type, content_json, original_text,
                   translated_text, source_language, is_translated, delivery_status
            FROM messages
            WHERE contact_id = ? {cursor_clause} {media_clause}
            {order_clause}
            {limit_clause}
            "#
        );

        let mut stmt = conn.prepare(&query)?;

        // Helper to build StoredMessage from row
        let build_message = |row: &rusqlite::Row,
                             contact_name: &Option<String>,
                             contact_phone: &Option<String>,
                             strip: bool|
         -> rusqlite::Result<StoredMessage> {
            let raw_content_json: String = row.get(9)?;
            let (content_json, content) = if strip {
                Self::strip_media_from_content(&raw_content_json)
            } else {
                (
                    raw_content_json.clone(),
                    serde_json::from_str(&raw_content_json).ok(),
                )
            };

            Ok(StoredMessage {
                id: row.get(0)?,
                contact_id: row.get(1)?,
                timestamp: row.get(2)?,
                is_from_me: row.get(3)?,
                is_forwarded: row.get(4)?,
                sender_name: row.get(5)?,
                sender_phone: row.get(6)?,
                contact_name: contact_name.clone(),
                contact_phone: contact_phone.clone(),
                chat_type: row.get(7)?,
                content_type: row.get(8)?,
                content_json,
                content,
                original_text: row.get(10)?,
                translated_text: row.get(11)?,
                source_language: row.get(12)?,
                is_translated: row.get(13)?,
                delivery_status: row.get(14)?,
            })
        };

        let messages: Vec<StoredMessage> =
            if let (Some(before), Some(before_id)) = (before_timestamp, before_message_id) {
                stmt.query_map(params![contact_id, before, before, before_id], |row| {
                    build_message(row, &contact_name, &contact_phone, strip_media)
                })?
                .filter_map(|r| r.ok())
                .collect()
            } else if let Some(before) = before_timestamp {
                stmt.query_map(params![contact_id, before], |row| {
                    build_message(row, &contact_name, &contact_phone, strip_media)
                })?
                .filter_map(|r| r.ok())
                .collect()
            } else {
                stmt.query_map(params![contact_id], |row| {
                    build_message(row, &contact_name, &contact_phone, strip_media)
                })?
                .filter_map(|r| r.ok())
                .collect()
            };

        // If we used DESC order with limit, reverse to get chronological order
        if limit.is_some() {
            let mut messages = messages;
            messages.reverse();
            Ok(messages)
        } else {
            Ok(messages)
        }
    }

    /// Get database statistics
    pub fn get_stats(&self) -> Result<(i64, i64)> {
        let conn = self.conn.lock().unwrap();

        let message_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))?;

        let contact_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM contacts", [], |row| row.get(0))?;

        Ok((message_count, contact_count))
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

    /// Get a specific message by ID
    pub fn get_message_by_id(&self, message_id: &str) -> Result<Option<StoredMessage>> {
        let conn = self.conn.lock().unwrap();

        let result = conn.query_row(
            r#"
            SELECT m.id, m.contact_id, m.timestamp, m.is_from_me, m.is_forwarded, m.sender_name,
                   m.sender_phone, m.chat_type, m.content_type, m.content_json, m.original_text,
                   m.translated_text, m.source_language, m.is_translated, m.delivery_status,
                   c.name as contact_name, c.phone as contact_phone
            FROM messages m
            LEFT JOIN contacts c ON m.contact_id = c.id
            WHERE m.id = ?
            "#,
            params![message_id],
            |row| {
                let contact_name: Option<String> = row.get(15)?;
                let contact_phone: Option<String> = row.get(16)?;
                Self::row_to_stored_message(row, contact_name, contact_phone)
            },
        );

        match result {
            Ok(msg) => Ok(Some(msg)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Get recent messages for a contact (for conversation context)
    pub fn get_recent_messages(
        &self,
        contact_id: &str,
        limit: usize,
    ) -> Result<Vec<StoredMessage>> {
        let contact_id = canonical_chat_id(contact_id);
        let contact_id = contact_id.as_ref();
        let conn = self.conn.lock().unwrap();

        // Get contact info
        let contact_info: Option<(Option<String>, Option<String>)> = conn
            .query_row(
                "SELECT name, phone FROM contacts WHERE id = ?",
                params![contact_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();

        let (contact_name, contact_phone) = contact_info.unwrap_or((None, None));

        let mut stmt = conn.prepare(
            r#"
            SELECT id, contact_id, timestamp, is_from_me, is_forwarded, sender_name,
                   sender_phone, chat_type, content_type, content_json, original_text,
                   translated_text, source_language, is_translated, delivery_status
            FROM messages
            WHERE contact_id = ?
            ORDER BY timestamp DESC, id DESC
            LIMIT ?
            "#,
        )?;

        let rows = stmt.query_map(params![contact_id, limit as i64], |row| {
            Self::row_to_stored_message(row, contact_name.clone(), contact_phone.clone())
        })?;

        // Collect and reverse to get chronological order
        let mut messages: Vec<StoredMessage> = rows.filter_map(|r| r.ok()).collect();
        messages.reverse();
        Ok(messages)
    }
}
