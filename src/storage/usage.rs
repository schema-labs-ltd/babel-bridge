//! AI token usage accounting.

use super::*;

impl MessageStore {
    /// Record AI usage for a message
    pub fn record_usage(
        &self,
        contact_id: Option<&str>,
        message_id: Option<&str>,
        usage: &UsageInfo,
        operation: &str,
    ) -> Result<()> {
        let contact_id = contact_id.map(canonical_chat_id);
        let contact_id = contact_id.as_deref();
        let conn = self.conn.lock().unwrap();
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        conn.execute(
            r#"
            INSERT INTO translation_usage 
            (contact_id, message_id, timestamp, input_tokens, cached_input_tokens, output_tokens, cost_usd, operation)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            params![
                contact_id,
                message_id,
                timestamp,
                usage.input_tokens,
                usage.cached_input_tokens,
                usage.output_tokens,
                usage.cost_usd,
                operation,
            ],
        )?;

        Ok(())
    }

    /// Get total usage across all conversations
    pub fn get_global_usage(&self) -> Result<UsageInfo> {
        let conn = self.conn.lock().unwrap();

        let result = conn.query_row(
            r#"
            SELECT COALESCE(SUM(input_tokens), 0),
                   COALESCE(SUM(cached_input_tokens), 0),
                   COALESCE(SUM(output_tokens), 0), 
                   COALESCE(SUM(cost_usd), 0.0)
            FROM translation_usage
            "#,
            [],
            |row| {
                Ok(UsageInfo {
                    input_tokens: row.get::<_, i64>(0)? as u32,
                    cached_input_tokens: row.get::<_, i64>(1)? as u32,
                    output_tokens: row.get::<_, i64>(2)? as u32,
                    cost_usd: row.get(3)?,
                })
            },
        )?;

        Ok(result)
    }

    /// Get usage for a specific conversation
    pub fn get_conversation_usage(&self, contact_id: &str) -> Result<UsageInfo> {
        let contact_id = canonical_chat_id(contact_id);
        let contact_id = contact_id.as_ref();
        let conn = self.conn.lock().unwrap();

        let result = conn.query_row(
            r#"
            SELECT COALESCE(SUM(input_tokens), 0),
                   COALESCE(SUM(cached_input_tokens), 0),
                   COALESCE(SUM(output_tokens), 0), 
                   COALESCE(SUM(cost_usd), 0.0)
            FROM translation_usage
            WHERE contact_id = ?
            "#,
            params![contact_id],
            |row| {
                Ok(UsageInfo {
                    input_tokens: row.get::<_, i64>(0)? as u32,
                    cached_input_tokens: row.get::<_, i64>(1)? as u32,
                    output_tokens: row.get::<_, i64>(2)? as u32,
                    cost_usd: row.get(3)?,
                })
            },
        )?;

        Ok(result)
    }
}
