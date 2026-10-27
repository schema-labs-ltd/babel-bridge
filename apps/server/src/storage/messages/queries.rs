use super::*;

impl MessageStore {
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

    pub(super) fn get_messages_page(
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
