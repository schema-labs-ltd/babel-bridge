//! Voice transcripts, voice settings, message tones and voice note send state.

use super::*;

impl MessageStore {
    pub fn update_voice_transcript(
        &self,
        id: &str,
        original: &str,
        translated: &str,
        source: &str,
    ) -> Result<()> {
        self.conn.lock().unwrap().execute(
            "UPDATE messages SET original_text=? WHERE id=?",
            params![original, id],
        )?;
        self.update_message_translation(id, Some(translated), Some(source))
    }

    pub fn voice_setting(&self, scope: &str) -> Result<String> {
        let scope = canonical_chat_id(scope);
        let conn = self.conn.lock().unwrap();
        Ok(conn
            .query_row(
                "SELECT value FROM app_settings WHERE key = ?",
                params![format!("voice:{scope}")],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or_else(|| "auto".into()))
    }

    pub fn set_voice_setting(&self, scope: &str, voice: &str) -> Result<()> {
        let scope = canonical_chat_id(scope);
        self.conn.lock().unwrap().execute("INSERT INTO app_settings(key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![format!("voice:{scope}"), voice])?;
        Ok(())
    }

    pub fn message_tone_settings(
        &self,
        contact_id: Option<&str>,
    ) -> Result<crate::message_tones::MessageToneSettings> {
        let contact_id = contact_id.map(canonical_chat_id);
        let contact_id = contact_id.as_deref();
        use crate::message_tones::{MessageTone, MessageToneSettings};
        let conn = self.conn.lock().unwrap();
        let read = |key: &str| -> Result<Option<MessageTone>> {
            let stored: Option<String> = conn
                .query_row(
                    "SELECT value FROM app_settings WHERE key = ?",
                    [key],
                    |row| row.get(0),
                )
                .optional()?;
            stored
                .map(|value| {
                    serde_json::from_str(&value).context("Invalid stored message ringtone")
                })
                .transpose()
        };
        let global_tone = read("message-tone:global")?.unwrap_or_default();
        let tone = match contact_id {
            Some(id) => read(&format!("message-tone:conversation:{id}"))?,
            None => Some(global_tone),
        };
        Ok(MessageToneSettings {
            tone,
            global_tone,
            effective_tone: tone.unwrap_or(global_tone),
        })
    }

    pub fn set_message_tone(
        &self,
        contact_id: Option<&str>,
        tone: Option<crate::message_tones::MessageTone>,
    ) -> Result<()> {
        let contact_id = contact_id.map(canonical_chat_id);
        let contact_id = contact_id.as_deref();
        let key = match contact_id {
            Some(id) => format!("message-tone:conversation:{id}"),
            None => "message-tone:global".to_string(),
        };
        let conn = self.conn.lock().unwrap();
        if let Some(tone) = tone {
            conn.execute(
                "INSERT INTO app_settings(key,value) VALUES (?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![key, serde_json::to_string(&tone)?],
            )?;
        } else {
            anyhow::ensure!(
                contact_id.is_some(),
                "A global message ringtone is required"
            );
            conn.execute("DELETE FROM app_settings WHERE key = ?", [key])?;
        }
        Ok(())
    }

    /// Returns `(payload, status, result, created_at)`.
    pub fn voice_note(&self, id: &str) -> Result<Option<VoiceNoteRow>> {
        Ok(self
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT payload, status, result, created_at FROM voice_notes WHERE id = ?",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?)
    }

    pub fn save_voice_note(&self, id: &str, payload: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM voice_notes WHERE status = 'prepared' AND created_at < ?",
            params![chrono::Utc::now().timestamp() - 86400 * 7],
        )?;
        conn.execute("INSERT INTO voice_notes(id, payload, created_at) VALUES (?, ?, ?) ON CONFLICT(id) DO NOTHING", params![id, payload, chrono::Utc::now().timestamp()])?;
        Ok(())
    }

    pub fn claim_voice_send_with_payload(&self, id: &str, payload: &str) -> Result<bool> {
        Ok(self.conn.lock().unwrap().execute(
            "UPDATE voice_notes SET status='sending', payload=? WHERE id=? AND status='prepared'",
            params![payload, id],
        )? == 1)
    }

    #[cfg(test)]
    pub fn claim_voice_send(&self, id: &str) -> Result<bool> {
        Ok(self.conn.lock().unwrap().execute(
            "UPDATE voice_notes SET status='sending' WHERE id=? AND status='prepared'",
            params![id],
        )? == 1)
    }

    pub fn finish_voice_send(&self, id: &str, result: &str) -> Result<()> {
        self.conn.lock().unwrap().execute(
            "UPDATE voice_notes SET status='sent', result=?, payload='{}' WHERE id=?",
            params![result, id],
        )?;
        Ok(())
    }

    pub fn release_voice_send(&self, id: &str) -> Result<()> {
        self.conn.lock().unwrap().execute(
            "UPDATE voice_notes SET status='prepared' WHERE id=? AND status='sending'",
            params![id],
        )?;
        Ok(())
    }
}
