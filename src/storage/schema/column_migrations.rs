use super::*;

impl MessageStore {
    pub(super) fn migrate_invalidate_unverified_language_labels(
        &self,
        conn: &Connection,
    ) -> Result<()> {
        let applied: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM app_settings WHERE key='language_detection_contract_v2')",
            [],
            |row| row.get(0),
        )?;
        if applied {
            return Ok(());
        }
        // Older detection treated malformed model output as a successful
        // owner-language result. These derived labels cannot be trusted. Keep
        // message contents and successful translations; re-detect on chat open.
        let tx = conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE messages SET source_language=NULL WHERE is_from_me=0 AND is_translated=0 AND COALESCE(translated_text,'')='' AND original_text IS NOT NULL AND source_language IS NOT NULL",
            [],
        )?;
        tx.execute(
            "INSERT INTO app_settings(key,value) VALUES('language_detection_contract_v2','1')",
            [],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub(super) fn migrate_add_delivery_status_column(&self, conn: &Connection) -> Result<()> {
        let exists: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('messages') WHERE name = 'delivery_status'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0)
            > 0;
        if !exists {
            conn.execute("ALTER TABLE messages ADD COLUMN delivery_status TEXT", [])?;
        }
        conn.execute(
            "UPDATE messages SET delivery_status = 'sent' WHERE is_from_me = 1 AND delivery_status IS NULL",
            [],
        )?;
        Ok(())
    }

    /// Add composite indexes for the most common read paths on existing databases
    pub(super) fn migrate_add_query_indexes(&self, conn: &Connection) -> Result<()> {
        conn.execute_batch(
            r#"
            CREATE INDEX IF NOT EXISTS idx_messages_contact_timestamp_desc
                ON messages(contact_id, timestamp DESC);
            CREATE INDEX IF NOT EXISTS idx_messages_contact_timestamp_id_desc
                ON messages(contact_id, timestamp DESC, id DESC);
            CREATE INDEX IF NOT EXISTS idx_contacts_pinned_sort
                ON contacts(pinned_at, last_message_time DESC);
            CREATE INDEX IF NOT EXISTS idx_usage_contact_timestamp
                ON translation_usage(contact_id, timestamp DESC);
            CREATE INDEX IF NOT EXISTS idx_messages_contact_language
                ON messages(contact_id, is_from_me, source_language);
            "#,
        )?;

        conn.execute_batch("PRAGMA optimize;")?;

        Ok(())
    }

    /// Add style_profiles table for AI reply generation
    pub(super) fn migrate_add_style_profiles_table(&self, conn: &Connection) -> Result<()> {
        // Check if table exists
        let table_exists: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='style_profiles'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0)
            > 0;

        if !table_exists {
            info!("Migrating database: creating style_profiles table...");
            conn.execute_batch(
                r#"
                CREATE TABLE style_profiles (
                    contact_id TEXT PRIMARY KEY,
                    profile_text TEXT NOT NULL,
                    sample_messages TEXT NOT NULL,
                    message_count INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL
                );

                CREATE INDEX idx_style_profiles_updated ON style_profiles(updated_at);
                "#,
            )?;
            info!("Database migration complete: created style_profiles table");
        }

        Ok(())
    }

    /// Add conversation settings columns to contacts table
    pub(super) fn migrate_add_conversation_settings_columns(
        &self,
        conn: &Connection,
    ) -> Result<()> {
        let has_reply_only = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('contacts') WHERE name = 'reply_only'",
            [],
            |row| row.get::<_, i64>(0),
        )? > 0;
        if !has_reply_only {
            conn.execute(
                "ALTER TABLE contacts ADD COLUMN reply_only INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        let has_translation_enabled = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('contacts') WHERE name = 'translation_enabled'",
            [],
            |row| row.get::<_, i64>(0),
        )? > 0;
        if !has_translation_enabled {
            conn.execute(
                "ALTER TABLE contacts ADD COLUMN translation_enabled INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        // Check if language_override column exists
        let has_language_override: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('contacts') WHERE name = 'language_override'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0)
            > 0;

        if !has_language_override {
            info!("Migrating database: adding conversation settings columns...");
            conn.execute("ALTER TABLE contacts ADD COLUMN language_override TEXT", [])?;
        }

        let has_translation_style: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('contacts') WHERE name = 'translation_style'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0)
            > 0;
        if !has_translation_style {
            conn.execute("ALTER TABLE contacts ADD COLUMN translation_style TEXT", [])?;
        }

        let has_send_original_follow_up: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('contacts') WHERE name = 'send_original_follow_up'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0)
            > 0;
        if !has_send_original_follow_up {
            info!("Migrating database: adding original follow-up setting...");
            conn.execute(
                "ALTER TABLE contacts ADD COLUMN send_original_follow_up INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }

        if !has_language_override || !has_translation_style || !has_send_original_follow_up {
            info!("Database migration complete: added conversation settings columns");
        }

        Ok(())
    }

    /// Add pinned_at column to contacts table
    pub(super) fn migrate_add_pinned_column(&self, conn: &Connection) -> Result<()> {
        // Check if column exists
        let has_pinned_at: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('contacts') WHERE name = 'pinned_at'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0)
            > 0;

        if !has_pinned_at {
            info!("Migrating database: adding pinned_at column...");
            conn.execute("ALTER TABLE contacts ADD COLUMN pinned_at INTEGER", [])?;
            info!("Database migration complete: added pinned_at column");
        }

        Ok(())
    }

    /// Add last_message_preview column and backfill previews for existing contacts
    pub(super) fn migrate_add_last_message_preview_column(&self, conn: &Connection) -> Result<()> {
        let has_last_message_preview: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('contacts') WHERE name = 'last_message_preview'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0)
            > 0;

        if !has_last_message_preview {
            info!("Migrating database: adding last_message_preview column...");
            conn.execute(
                "ALTER TABLE contacts ADD COLUMN last_message_preview TEXT",
                [],
            )?;
            Self::backfill_contact_previews(conn)?;
            info!("Database migration complete: added last_message_preview");
        }

        Ok(())
    }

    pub(super) fn migrate_refresh_translated_contact_previews(
        &self,
        conn: &Connection,
    ) -> Result<()> {
        let version: Option<String> = conn
            .query_row(
                "SELECT value FROM app_settings WHERE key = ?",
                params!["translated_contact_previews_version"],
                |row| row.get(0),
            )
            .optional()?;
        if version.as_deref() == Some("1") {
            return Ok(());
        }

        info!("Migrating database: refreshing translated contact previews...");
        Self::backfill_contact_previews(conn)?;
        conn.execute(
            r#"
            INSERT INTO app_settings (key, value) VALUES (?1, ?2)
            ON CONFLICT(key) DO UPDATE SET value = excluded.value
            "#,
            params!["translated_contact_previews_version", "1"],
        )?;
        info!("Database migration complete: refreshed translated contact previews");
        Ok(())
    }

    pub(super) fn backfill_contact_previews(conn: &Connection) -> Result<()> {
        let mut stmt = conn.prepare(
            r#"
            SELECT
                c.id,
                m.content_json,
                m.content_type,
                m.is_from_me,
                m.translated_text
            FROM contacts c
            LEFT JOIN (
                SELECT contact_id, content_json, content_type, is_from_me, translated_text,
                       ROW_NUMBER() OVER (PARTITION BY contact_id ORDER BY timestamp DESC, rowid DESC) as rn
                FROM messages
            ) m ON m.contact_id = c.id AND m.rn = 1
            "#,
        )?;

        // (contact_id, content_json, content_type, is_from_me, translated_text)
        type PreviewRow = (
            String,
            Option<String>,
            Option<String>,
            Option<bool>,
            Option<String>,
        );
        let rows: Vec<PreviewRow> = stmt
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })?
            .filter_map(|row| row.ok())
            .collect();
        drop(stmt);

        for (contact_id, content_json, content_type, is_from_me, translated_text) in rows {
            let preview = Self::generate_message_preview(
                content_json.as_deref(),
                content_type.as_deref(),
                is_from_me.unwrap_or(false),
                translated_text.as_deref(),
            );
            conn.execute(
                "UPDATE contacts SET last_message_preview = ? WHERE id = ?",
                params![preview, contact_id],
            )?;
        }

        Ok(())
    }

    /// Fix contact types based on JID suffix (groups end with @g.us)
    pub(super) fn migrate_fix_contact_types(&self, conn: &Connection) -> Result<()> {
        // Update contacts where type doesn't match the JID suffix
        // This fixes both NULL types and incorrectly set types
        let updated = conn.execute(
            r#"
            UPDATE contacts 
            SET type = CASE 
                WHEN id LIKE '%@g.us' THEN 'group'
                WHEN id LIKE '%@s.whatsapp.net' THEN 'private'
                WHEN id LIKE '%@broadcast' THEN 'broadcast'
                ELSE 'private'
            END
            WHERE type IS NULL 
               OR (id LIKE '%@g.us' AND type != 'group')
               OR (id LIKE '%@s.whatsapp.net' AND type != 'private')
               OR (id LIKE '%@broadcast' AND type != 'broadcast')
            "#,
            [],
        )?;

        if updated > 0 {
            info!(
                "Fixed contact types for {} contacts based on JID suffix",
                updated
            );
        }

        Ok(())
    }

    /// Add translation columns to existing database
    pub(super) fn migrate_add_translation_columns(&self, conn: &Connection) -> Result<()> {
        // Check if columns exist by querying table info
        let has_original_text: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('messages') WHERE name = 'original_text'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0)
            > 0;

        if !has_original_text {
            tracing::info!("Migrating database: adding translation columns...");
            conn.execute_batch(
                r#"
                ALTER TABLE messages ADD COLUMN original_text TEXT;
                ALTER TABLE messages ADD COLUMN translated_text TEXT;
                ALTER TABLE messages ADD COLUMN source_language TEXT;
                ALTER TABLE messages ADD COLUMN is_translated INTEGER DEFAULT 0;
                "#,
            )?;
            tracing::info!("Database migration complete");
        }

        Ok(())
    }

    /// Add cached_input_tokens column to usage tracking
    pub(super) fn migrate_add_cached_usage_column(&self, conn: &Connection) -> Result<()> {
        let has_cached_input_tokens: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('translation_usage') WHERE name = 'cached_input_tokens'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0)
            > 0;

        if !has_cached_input_tokens {
            info!("Migrating database: adding cached_input_tokens to translation_usage...");
            conn.execute(
                "ALTER TABLE translation_usage ADD COLUMN cached_input_tokens INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
            info!("Database migration complete: added cached_input_tokens");
        }

        Ok(())
    }
}
