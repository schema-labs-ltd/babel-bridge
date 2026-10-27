//! Contacts, unread counts, previews, pinning and per-conversation settings.

use super::*;

impl MessageStore {
    /// WhatsApp supplies the full group membership count with live messages.
    pub fn set_group_participant_count(&self, contact_id: &str, count: u32) -> Result<()> {
        let contact_id = canonical_chat_id(contact_id);
        let contact_id = contact_id.as_ref();
        if contact_id.ends_with("@g.us") && count > 0 {
            self.conn.lock().unwrap().execute(
                "UPDATE contacts SET participant_count = ? WHERE id = ?",
                params![count, contact_id],
            )?;
        }
        Ok(())
    }

    /// Incoming recipients include this account and exclude the message's sender.
    pub fn notification_recipient_count(&self, contact_id: &str) -> Result<Option<u32>> {
        let contact_id = canonical_chat_id(contact_id);
        let contact_id = contact_id.as_ref();
        if !contact_id.ends_with("@g.us") {
            return Ok(None);
        }
        let count: Option<u32> = self
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT participant_count FROM contacts WHERE id = ?",
                [contact_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        Ok(count
            .and_then(|value| value.checked_sub(1))
            .filter(|value| *value > 0))
    }

    /// Add or update a contact
    pub fn upsert_contact(
        &self,
        id: &str,
        name: Option<&str>,
        phone: Option<&str>,
        contact_type: Option<&str>,
        last_message_time: i64,
    ) -> Result<()> {
        let id = canonical_chat_id(id);
        let normalized_phone = crate::identity::phone_number(&id);
        let phone = normalized_phone.as_deref().or(phone);
        let conn = self.conn.lock().unwrap();

        conn.execute(
            r#"
            INSERT INTO contacts (id, name, phone, type, last_message_time, unread_count)
            VALUES (?1, ?2, ?3, ?4, ?5, 0)
            ON CONFLICT(id) DO UPDATE SET
                name = COALESCE(
                    CASE WHEN excluded.name IS NOT NULL AND excluded.name != excluded.phone 
                         THEN excluded.name ELSE NULL END,
                    contacts.name
                ),
                phone = COALESCE(excluded.phone, contacts.phone),
                type = COALESCE(excluded.type, contacts.type),
                last_message_time = MAX(contacts.last_message_time, excluded.last_message_time)
            "#,
            params![id, name, phone, contact_type, last_message_time],
        )?;

        Ok(())
    }

    /// Increment unread count for a contact
    #[cfg(test)]
    pub fn increment_unread(&self, contact_id: &str) -> Result<()> {
        let contact_id = canonical_chat_id(contact_id);
        let contact_id = contact_id.as_ref();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE contacts SET unread_count = unread_count + 1 WHERE id = ?",
            params![contact_id],
        )?;
        Ok(())
    }

    /// Reset unread count for a contact
    pub fn mark_as_read(&self, contact_id: &str) -> Result<()> {
        let contact_id = canonical_chat_id(contact_id);
        let contact_id = contact_id.as_ref();
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM unread_messages WHERE message_id IN (SELECT id FROM messages WHERE contact_id=?)", [contact_id])?;
        conn.execute(
            "UPDATE contacts SET unread_count = 0 WHERE id = ?",
            params![contact_id],
        )?;
        Ok(())
    }

    /// Set unread count for a contact (used for history sync)
    pub fn set_unread_count(&self, contact_id: &str, count: u32) -> Result<()> {
        let contact_id = canonical_chat_id(contact_id);
        let contact_id = contact_id.as_ref();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE contacts SET unread_count = ? WHERE id = ?",
            params![count as i32, contact_id],
        )?;
        conn.execute("DELETE FROM unread_messages WHERE message_id IN (SELECT id FROM messages WHERE contact_id=?)", [contact_id])?;
        conn.execute("INSERT OR IGNORE INTO unread_messages(message_id) SELECT id FROM messages WHERE contact_id=?1 AND is_from_me=0 AND lower(content_type) NOT IN ('reaction','revoked','protocol','unknown') AND id NOT IN (SELECT message_id FROM viewed_messages) ORDER BY timestamp DESC,id DESC LIMIT ?2", params![contact_id,count])?;
        Ok(())
    }

    pub fn total_unread_count(&self) -> Result<i32> {
        let conn = self.conn.lock().unwrap();
        let count = conn.query_row(
            "SELECT COALESCE(SUM(unread_count), 0) FROM contacts",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Get all contacts sorted by pinned status first, then last message time
    pub fn get_contacts(&self) -> Result<Vec<StoredContact>> {
        let conn = self.conn.lock().unwrap();

        let mut stmt = conn.prepare(
            r#"
            SELECT
                id, name, phone, type, last_message_time, unread_count, pinned_at, last_message_preview
            FROM contacts
            ORDER BY 
                CASE WHEN pinned_at IS NOT NULL THEN 0 ELSE 1 END,
                pinned_at ASC,
                last_message_time DESC
            "#,
        )?;

        let contacts = stmt
            .query_map([], |row| {
                Ok(StoredContact {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    phone: row.get(2)?,
                    contact_type: row.get(3)?,
                    last_message_time: row.get(4)?,
                    unread_count: row.get(5)?,
                    pinned_at: row.get(6)?,
                    last_message_preview: row.get(7)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(contacts)
    }

    /// Generate a preview string for a message (matching frontend logic)
    pub(super) fn generate_message_preview(
        content_json: Option<&str>,
        content_type: Option<&str>,
        is_from_me: bool,
        translated_text: Option<&str>,
    ) -> Option<String> {
        let prefix = if is_from_me { "You: " } else { "" };
        let translated_text = (!is_from_me)
            .then_some(translated_text)
            .flatten()
            .map(str::trim)
            .filter(|text| !text.is_empty());

        let content_type = content_type?;
        let content_json = content_json?;

        let content: serde_json::Value = serde_json::from_str(content_json).ok()?;

        // Normalize content_type to lowercase for matching
        let content_type_lower = content_type.to_lowercase();
        let preview = match content_type_lower.as_str() {
            "text" => {
                let body = translated_text.unwrap_or_else(|| {
                    content
                        .get("body")
                        .or_else(|| content.get("text"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                });
                let truncated: String = body.chars().take(50).collect();
                format!("{}{}", prefix, truncated)
            }
            "image" => {
                let caption = translated_text
                    .or_else(|| content.get("caption").and_then(|v| v.as_str()))
                    .map(|c| {
                        let truncated: String = c.chars().take(30).collect();
                        format!(" {}", truncated)
                    })
                    .unwrap_or_default();
                format!("{}[ Image ]{}", prefix, caption)
            }
            "video" => {
                let caption = translated_text
                    .or_else(|| content.get("caption").and_then(|v| v.as_str()))
                    .map(|c| {
                        let truncated: String = c.chars().take(30).collect();
                        format!(" {}", truncated)
                    })
                    .unwrap_or_default();
                format!("{}[ Video ]{}", prefix, caption)
            }
            "audio" => {
                let is_voice = content
                    .get("is_voice_note")
                    .or_else(|| content.get("isVoiceNote"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if is_voice {
                    format!("{}[ Voice Note ]", prefix)
                } else {
                    format!("{}[ Audio ]", prefix)
                }
            }
            "voice note" => format!("{}[ Voice Note ]", prefix),
            "document" => {
                let file_name = content
                    .get("file_name")
                    .or_else(|| content.get("fileName"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("file");
                format!("{}[ Document: {} ]", prefix, file_name)
            }
            "sticker" => format!("{}[ Sticker ]", prefix),
            "location" => format!("{}[ Location ]", prefix),
            "contact" => {
                let name = content.get("name").and_then(|v| v.as_str()).unwrap_or("");
                format!("{}[ Contact: {} ]", prefix, name)
            }
            "reaction" => {
                let emoji = content.get("emoji").and_then(|v| v.as_str()).unwrap_or("");
                format!("{}{}", prefix, emoji)
            }
            "revoked" => "[ Message deleted ]".to_string(),
            "poll" => {
                let question = content
                    .get("question")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                format!("{}[ Poll: {} ]", prefix, question)
            }
            _ => format!("{}[ Message ]", prefix),
        };

        Some(preview)
    }

    /// Pin or unpin a contact
    pub fn toggle_pin(&self, contact_id: &str) -> Result<bool> {
        let contact_id = canonical_chat_id(contact_id);
        let contact_id = contact_id.as_ref();
        let conn = self.conn.lock().unwrap();

        // Check if currently pinned
        let currently_pinned: Option<i64> = conn
            .query_row(
                "SELECT pinned_at FROM contacts WHERE id = ?",
                params![contact_id],
                |row| row.get(0),
            )
            .ok()
            .flatten();

        if currently_pinned.is_some() {
            // Unpin
            conn.execute(
                "UPDATE contacts SET pinned_at = NULL WHERE id = ?",
                params![contact_id],
            )?;
            Ok(false) // Now unpinned
        } else {
            // Pin with current timestamp
            let now = chrono::Utc::now().timestamp_millis();
            conn.execute(
                "UPDATE contacts SET pinned_at = ? WHERE id = ?",
                params![now, contact_id],
            )?;
            Ok(true) // Now pinned
        }
    }

    /// Get conversation settings for a contact
    pub fn get_conversation_settings(&self, contact_id: &str) -> Result<ConversationSettings> {
        let contact_id = canonical_chat_id(contact_id);
        let contact_id = contact_id.as_ref();
        let conn = self.conn.lock().unwrap();

        let result = conn.query_row(
            "SELECT language_override, translation_style, send_original_follow_up, translation_enabled, reply_only FROM contacts WHERE id = ?",
            params![contact_id],
            |row| {
                Ok(ConversationSettings {
                    language_override: row.get(0)?,
                    translation_style: row.get(1)?,
                    send_original_follow_up: row.get(2)?,
                    translation_enabled: row.get(3)?,
                    reply_only: row.get(4)?,
                })
            },
        );

        match result {
            Ok(settings) => Ok(settings),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(ConversationSettings::default()),
            Err(e) => Err(e.into()),
        }
    }

    /// Update conversation settings for a contact
    pub fn update_conversation_settings(
        &self,
        contact_id: &str,
        settings: &ConversationSettings,
    ) -> Result<()> {
        let contact_id = canonical_chat_id(contact_id);
        let contact_id = contact_id.as_ref();
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let changed = tx.execute(
            "UPDATE contacts SET language_override = ?, translation_style = ?, send_original_follow_up = ?, translation_enabled = ?, reply_only = ? WHERE id = ?",
            params![
                settings.language_override,
                settings.translation_style,
                settings.send_original_follow_up,
                settings.translation_enabled,
                settings.reply_only,
                contact_id
            ],
        )?;
        anyhow::ensure!(changed == 1, "Conversation not found");
        if !settings.translation_enabled {
            tx.execute("DELETE FROM translation_jobs WHERE message_id IN (SELECT id FROM messages WHERE contact_id=?)", params![contact_id])?;
            tx.execute("UPDATE pending_notifications SET requires_translation=0 WHERE message_id IN (SELECT id FROM messages WHERE contact_id=?)", params![contact_id])?;
        }
        tx.commit()?;

        info!(
            language = ?settings.language_override,
            style = ?settings.translation_style,
            original_follow_up = settings.send_original_follow_up,
            "Updated conversation settings"
        );

        Ok(())
    }

    pub fn get_openai_settings(&self) -> Result<OpenAiSettings> {
        let conn = self.conn.lock().unwrap();
        let value: Option<String> = conn
            .query_row(
                "SELECT value FROM app_settings WHERE key = 'openai'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        value
            .map(|json| serde_json::from_str(&json).context("Invalid stored OpenAI settings"))
            .unwrap_or_else(|| Ok(OpenAiSettings::default()))
    }

    pub fn update_openai_settings(&self, settings: &OpenAiSettings) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let value = serde_json::to_string(settings)?;
        conn.execute(
            "INSERT INTO app_settings (key, value) VALUES ('openai', ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![value],
        )?;
        Ok(())
    }

    /// Get a contact by ID
    pub fn get_contact(&self, contact_id: &str) -> Result<Option<StoredContact>> {
        let contact_id = canonical_chat_id(contact_id);
        let contact_id = contact_id.as_ref();
        let conn = self.conn.lock().unwrap();

        let mut stmt = conn.prepare(
            r#"
            SELECT
                id, name, phone, type, last_message_time, unread_count, pinned_at, last_message_preview
            FROM contacts
            WHERE id = ?
            "#,
        )?;

        let contact = stmt
            .query_row(params![contact_id], |row| {
                Ok(StoredContact {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    phone: row.get(2)?,
                    contact_type: row.get(3)?,
                    last_message_time: row.get(4)?,
                    unread_count: row.get(5)?,
                    pinned_at: row.get(6)?,
                    last_message_preview: row.get(7)?,
                })
            })
            .ok();

        Ok(contact)
    }

    /// Get the predominant language used by a contact in recent messages.
    /// Returns the most common source_language from the last N incoming messages.
    pub fn get_conversation_language(
        &self,
        contact_id: &str,
        limit: usize,
    ) -> Result<Option<String>> {
        let contact_id = canonical_chat_id(contact_id);
        let contact_id = contact_id.as_ref();
        let conn = self.conn.lock().unwrap();

        // Get the most common source language from recent incoming (not from me) messages
        let mut stmt = conn.prepare(
            r#"
            SELECT source_language, COUNT(*) as cnt
            FROM (
                SELECT source_language, timestamp FROM messages
                WHERE contact_id = ? AND is_from_me = 0
                  AND source_language IS NOT NULL AND source_language != ''
                  AND lower(source_language) NOT IN ('unknown', 'undetermined', 'neutral', 'mixed')
                ORDER BY timestamp DESC, id DESC LIMIT ?
            )
            GROUP BY source_language COLLATE NOCASE
            ORDER BY cnt DESC, MAX(timestamp) DESC
            LIMIT 1
            "#,
        )?;

        let language: Option<String> = stmt
            .query_row(params![contact_id, limit.clamp(1, 100) as i64], |row| {
                row.get(0)
            })
            .optional()?;

        Ok(language)
    }
}
