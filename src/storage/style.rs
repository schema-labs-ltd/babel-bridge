//! Writing style profiles and the outgoing messages they are built from.

use super::*;

impl MessageStore {
    /// Get a style profile by contact ID (or "__global__" for global profile)
    pub fn get_style_profile(&self, contact_id: &str) -> Result<Option<StyleProfile>> {
        let contact_id = canonical_chat_id(contact_id);
        let contact_id = contact_id.as_ref();
        let conn = self.conn.lock().unwrap();

        let result = conn.query_row(
            "SELECT contact_id, profile_text, sample_messages, message_count, updated_at 
             FROM style_profiles WHERE contact_id = ?",
            params![contact_id],
            |row| {
                let sample_messages_json: String = row.get(2)?;
                let sample_messages: Vec<String> =
                    serde_json::from_str(&sample_messages_json).unwrap_or_default();

                Ok(StyleProfile {
                    contact_id: row.get(0)?,
                    profile_text: row.get(1)?,
                    sample_messages,
                    message_count: row.get(3)?,
                    updated_at: row.get(4)?,
                })
            },
        );

        match result {
            Ok(profile) => Ok(Some(profile)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Save or update a style profile
    pub fn save_style_profile(&self, profile: &StyleProfile) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        let sample_messages_json = serde_json::to_string(&profile.sample_messages)?;

        conn.execute(
            r#"
            INSERT INTO style_profiles (contact_id, profile_text, sample_messages, message_count, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(contact_id) DO UPDATE SET
                profile_text = excluded.profile_text,
                sample_messages = excluded.sample_messages,
                message_count = excluded.message_count,
                updated_at = excluded.updated_at
            "#,
            params![
                canonical_chat_id(&profile.contact_id),
                profile.profile_text,
                sample_messages_json,
                profile.message_count,
                profile.updated_at,
            ],
        )?;

        info!(messages = profile.message_count, "Saved style profile");

        Ok(())
    }

    /// Get count of outgoing messages (for determining if style profile needs refresh)
    pub fn get_outgoing_message_count(&self, contact_id: Option<&str>) -> Result<i32> {
        let contact_id = contact_id.map(canonical_chat_id);
        let contact_id = contact_id.as_deref();
        let conn = self.conn.lock().unwrap();

        let count: i32 = if let Some(cid) = contact_id {
            conn.query_row(
                "SELECT COUNT(*) FROM messages WHERE is_from_me = 1 AND contact_id = ?",
                params![cid],
                |row| row.get(0),
            )?
        } else {
            conn.query_row(
                "SELECT COUNT(*) FROM messages WHERE is_from_me = 1",
                [],
                |row| row.get(0),
            )?
        };

        Ok(count)
    }

    /// Get outgoing messages for style analysis
    /// If contact_id is None, gets messages across all contacts (for global profile)
    /// Returns messages with text content, ordered by timestamp descending
    pub fn get_outgoing_messages_for_style(
        &self,
        contact_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<StoredMessage>> {
        let contact_id = contact_id.map(canonical_chat_id);
        let contact_id = contact_id.as_deref();
        let conn = self.conn.lock().unwrap();

        let query = if contact_id.is_some() {
            r#"
            SELECT id, contact_id, timestamp, is_from_me, is_forwarded, sender_name,
                   sender_phone, chat_type, content_type, content_json, original_text,
                   translated_text, source_language, is_translated, delivery_status
            FROM messages
            WHERE is_from_me = 1 
              AND contact_id = ?
              AND content_type = 'Text'
              AND (original_text IS NOT NULL OR content_json LIKE '%"body"%')
            ORDER BY timestamp DESC
            LIMIT ?
            "#
        } else {
            r#"
            SELECT id, contact_id, timestamp, is_from_me, is_forwarded, sender_name,
                   sender_phone, chat_type, content_type, content_json, original_text,
                   translated_text, source_language, is_translated, delivery_status
            FROM messages
            WHERE is_from_me = 1 
              AND content_type = 'Text'
              AND (original_text IS NOT NULL OR content_json LIKE '%"body"%')
            ORDER BY timestamp DESC
            LIMIT ?
            "#
        };

        let mut stmt = conn.prepare(query)?;

        let messages: Vec<StoredMessage> = if let Some(cid) = contact_id {
            stmt.query_map(params![cid, limit as i64], |row| {
                Self::row_to_stored_message(row, None, None)
            })?
            .filter_map(|r| r.ok())
            .collect()
        } else {
            stmt.query_map(params![limit as i64], |row| {
                Self::row_to_stored_message(row, None, None)
            })?
            .filter_map(|r| r.ok())
            .collect()
        };

        Ok(messages)
    }

    /// Get message exchange pairs for style learning
    /// Returns pairs of (incoming message, my reply) to show conversation patterns
    /// This helps AI learn not just what you say, but how you respond to different inputs
    pub fn get_message_exchange_pairs(
        &self,
        contact_id: &str,
        limit: usize,
    ) -> Result<Vec<(StoredMessage, StoredMessage)>> {
        let contact_id = canonical_chat_id(contact_id);
        let contact_id = contact_id.as_ref();
        let conn = self.conn.lock().unwrap();

        // Get recent messages for this contact, ordered by timestamp
        let query = r#"
            SELECT id, contact_id, timestamp, is_from_me, is_forwarded, sender_name,
                   sender_phone, chat_type, content_type, content_json, original_text,
                   translated_text, source_language, is_translated, delivery_status
            FROM messages
            WHERE contact_id = ?
              AND content_type = 'Text'
              AND (original_text IS NOT NULL OR content_json LIKE '%"body"%')
            ORDER BY timestamp DESC
            LIMIT ?
        "#;

        let mut stmt = conn.prepare(query)?;
        let messages: Vec<StoredMessage> = stmt
            .query_map(params![contact_id, (limit * 3) as i64], |row| {
                Self::row_to_stored_message(row, None, None)
            })?
            .filter_map(|r| r.ok())
            .collect();

        // Find pairs: look for my messages that follow their messages
        let mut pairs = Vec::new();
        let messages_sorted: Vec<_> = messages.into_iter().rev().collect(); // chronological order

        for i in 1..messages_sorted.len() {
            let prev = &messages_sorted[i - 1];
            let curr = &messages_sorted[i];

            // If previous message is from them and current is from me, it's an exchange
            if !prev.is_from_me && curr.is_from_me {
                pairs.push((prev.clone(), curr.clone()));
                if pairs.len() >= limit {
                    break;
                }
            }
        }

        Ok(pairs)
    }
}
