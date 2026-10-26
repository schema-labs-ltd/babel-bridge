//! Schema creation and migrations.

use super::*;

pub(super) fn table_column_names(conn: &Connection, table: &str) -> Result<Vec<String>> {
    let pragma = match table {
        "oauth_access_tokens" => "PRAGMA table_info(oauth_access_tokens)",
        "oauth_refresh_tokens" => "PRAGMA table_info(oauth_refresh_tokens)",
        "oauth_authorization_codes" => "PRAGMA table_info(oauth_authorization_codes)",
        _ => anyhow::bail!("unknown oauth table"),
    };
    let mut stmt = conn.prepare(pragma)?;
    let names = stmt
        .query_map([], |row| row.get(1))?
        .collect::<std::result::Result<Vec<String>, _>>()?;
    Ok(names)
}

fn column_exists(columns: &[String], name: &str) -> bool {
    columns.iter().any(|column| column == name)
}

fn table_count(conn: &Connection, sql: &str) -> Result<i64> {
    Ok(conn.query_row(sql, [], |row| row.get(0))?)
}

fn ensure_no_null_hashes(conn: &Connection, sql: &str, message: &str) -> Result<()> {
    let missing: i64 = conn.query_row(sql, [], |row| row.get(0))?;
    if missing != 0 {
        anyhow::bail!("{message}");
    }
    Ok(())
}

fn ensure_row_count(before: i64, inserted: usize, migration: &str) -> Result<()> {
    let inserted = i64::try_from(inserted).context("oauth hash migration row count")?;
    if inserted != before {
        anyhow::bail!("{migration} changed the row count");
    }
    Ok(())
}

impl MessageStore {
    /// Initialize database schema
    pub(super) fn init_schema(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        conn.execute_batch(
            r#"
            -- Contacts table
            CREATE TABLE IF NOT EXISTS contacts (
                id TEXT PRIMARY KEY,
                name TEXT,
                phone TEXT,
                type TEXT,
                last_message_time INTEGER DEFAULT 0,
                unread_count INTEGER DEFAULT 0,
                last_message_preview TEXT
            );

            -- Messages table
            CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY,
                contact_id TEXT NOT NULL,
                timestamp INTEGER NOT NULL,
                is_from_me INTEGER NOT NULL,
                is_forwarded INTEGER DEFAULT 0,
                sender_name TEXT,
                sender_phone TEXT,
                chat_type TEXT,
                content_type TEXT NOT NULL,
                content_json TEXT NOT NULL,
                delivery_status TEXT,
                FOREIGN KEY (contact_id) REFERENCES contacts(id)
            );

            -- Indexes
            CREATE INDEX IF NOT EXISTS idx_messages_contact_id ON messages(contact_id);
            CREATE INDEX IF NOT EXISTS idx_messages_timestamp ON messages(timestamp);
            CREATE INDEX IF NOT EXISTS idx_messages_feed_cursor ON messages(timestamp DESC, id DESC);
            CREATE INDEX IF NOT EXISTS idx_messages_contact_timestamp_desc ON messages(contact_id, timestamp DESC);
            CREATE INDEX IF NOT EXISTS idx_messages_contact_timestamp_id_desc ON messages(contact_id, timestamp DESC, id DESC);
            CREATE INDEX IF NOT EXISTS idx_messages_visual_media ON messages(contact_id, timestamp DESC, id DESC) WHERE lower(content_type) IN ('image', 'video');
            CREATE INDEX IF NOT EXISTS idx_message_reactions_target ON messages(contact_id,json_extract(content_json,'$.target_message_id'),timestamp DESC,id DESC) WHERE lower(content_type)='reaction';
            CREATE INDEX IF NOT EXISTS idx_contacts_last_message ON contacts(last_message_time DESC);

            -- AI usage tracking
            CREATE TABLE IF NOT EXISTS translation_usage (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                contact_id TEXT,
                message_id TEXT,
                timestamp INTEGER NOT NULL,
                input_tokens INTEGER NOT NULL,
                cached_input_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL,
                cost_usd REAL NOT NULL,
                operation TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS app_settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS push_devices (
                token TEXT PRIMARY KEY,
                installation_id TEXT NOT NULL UNIQUE,
                environment TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_usage_contact_id ON translation_usage(contact_id);
            CREATE INDEX IF NOT EXISTS idx_usage_timestamp ON translation_usage(timestamp);

            -- Link preview cache
            CREATE TABLE IF NOT EXISTS link_previews (
                url TEXT PRIMARY KEY,
                title TEXT,
                description TEXT,
                image_url TEXT,
                site_name TEXT,
                error TEXT,
                fetched_at INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_link_previews_fetched ON link_previews(fetched_at);

            -- OAuth 2.0 tables for MCP authentication
            CREATE TABLE IF NOT EXISTS oauth_clients (
                client_id TEXT PRIMARY KEY,
                client_name TEXT,
                redirect_uris_json TEXT NOT NULL,
                scope TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            
            -- Pending authorization requests (before user approves)
            CREATE TABLE IF NOT EXISTS oauth_pending_auth (
                session_key TEXT PRIMARY KEY,
                client_id TEXT NOT NULL,
                redirect_uri TEXT NOT NULL,
                code_challenge TEXT NOT NULL,
                code_challenge_method TEXT NOT NULL,
                scope TEXT NOT NULL,
                state TEXT,
                created_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL
            );

            -- Authorization codes are stored as SHA-256 hashes.
            CREATE TABLE IF NOT EXISTS oauth_authorization_codes (
                code_hash TEXT PRIMARY KEY,
                client_id TEXT NOT NULL,
                redirect_uri TEXT NOT NULL,
                code_challenge TEXT NOT NULL,
                code_challenge_method TEXT NOT NULL,
                scope TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL,
                used INTEGER DEFAULT 0
            );

            -- Access tokens are stored as SHA-256 hashes.
            -- refresh_token_hash links the grant that minted this access token.
            CREATE TABLE IF NOT EXISTS oauth_access_tokens (
                token_hash TEXT PRIMARY KEY,
                client_id TEXT NOT NULL,
                scope TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL,
                refresh_token_hash TEXT
            );

            -- Refresh tokens are stored as SHA-256 hashes.
            CREATE TABLE IF NOT EXISTS oauth_refresh_tokens (
                token_hash TEXT PRIMARY KEY,
                client_id TEXT NOT NULL,
                scope TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_oauth_pending_expires ON oauth_pending_auth(expires_at);
            CREATE INDEX IF NOT EXISTS idx_oauth_codes_expires ON oauth_authorization_codes(expires_at);
            CREATE INDEX IF NOT EXISTS idx_oauth_access_expires ON oauth_access_tokens(expires_at);
            CREATE INDEX IF NOT EXISTS idx_oauth_refresh_expires ON oauth_refresh_tokens(expires_at);
            "#,
        )?;

        // Add translation columns if they don't exist (migration for existing databases)
        self.migrate_add_translation_columns(&conn)?;
        self.migrate_add_delivery_status_column(&conn)?;
        self.migrate_add_cached_usage_column(&conn)?;

        // Fix contact types based on JID suffix
        self.migrate_fix_contact_types(&conn)?;

        // Add pinned_at column for pinning contacts
        self.migrate_add_pinned_column(&conn)?;
        if !conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('contacts') WHERE name = 'participant_count')",
            [], |row| row.get::<_, bool>(0),
        )? {
            conn.execute("ALTER TABLE contacts ADD COLUMN participant_count INTEGER", [])?;
        }

        self.migrate_add_query_indexes(&conn)?;

        // Add last_message_preview column and backfill it for faster contact list loads
        self.migrate_add_last_message_preview_column(&conn)?;
        self.migrate_refresh_translated_contact_previews(&conn)?;

        // Add style_profiles table for AI reply generation
        self.migrate_add_style_profiles_table(&conn)?;

        // Add conversation settings columns.
        self.migrate_add_conversation_settings_columns(&conn)?;
        self.migrate_invalidate_unverified_language_labels(&conn)?;
        self.migrate_device_jids(&conn)?;
        self.migrate_oauth_access_refresh_link(&conn)?;
        self.migrate_oauth_secret_hashes(&conn)?;

        Ok(())
    }

    /// Link each access token to the refresh token from the same grant.
    /// Legacy databases keep their rows and gain a nullable plaintext column.
    /// Hashed databases already store that link as `refresh_token_hash`.
    fn migrate_oauth_access_refresh_link(&self, conn: &Connection) -> Result<()> {
        let columns = table_column_names(conn, "oauth_access_tokens")?;
        if !column_exists(&columns, "token") {
            return Ok(());
        }
        if !column_exists(&columns, "refresh_token") {
            conn.execute(
                "ALTER TABLE oauth_access_tokens ADD COLUMN refresh_token TEXT",
                [],
            )?;
        }
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_oauth_access_refresh ON oauth_access_tokens(refresh_token)",
            [],
        )?;
        Ok(())
    }

    /// Store only SHA-256 hashes of OAuth secrets.
    ///
    /// Each table is migrated in stages so a restart can resume: add hash
    /// columns, fill only missing hashes, then rebuild the table without the
    /// plaintext primary key. SQLite cannot drop a primary key column in place.
    /// A later run sees hash columns only and leaves the rows unchanged.
    fn migrate_oauth_secret_hashes(&self, conn: &Connection) -> Result<()> {
        self.migrate_refresh_token_hashes(conn)?;
        self.migrate_access_token_hashes(conn)?;
        self.migrate_authorization_code_hashes(conn)?;
        Ok(())
    }

    fn migrate_refresh_token_hashes(&self, conn: &Connection) -> Result<()> {
        let columns = table_column_names(conn, "oauth_refresh_tokens")?;
        if !column_exists(&columns, "token") {
            conn.execute(
                "CREATE INDEX IF NOT EXISTS idx_oauth_refresh_expires ON oauth_refresh_tokens(expires_at)",
                [],
            )?;
            return Ok(());
        }
        if !column_exists(&columns, "token_hash") {
            conn.execute(
                "ALTER TABLE oauth_refresh_tokens ADD COLUMN token_hash TEXT",
                [],
            )?;
        }

        let pending = {
            let mut stmt =
                conn.prepare("SELECT token FROM oauth_refresh_tokens WHERE token_hash IS NULL")?;
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            rows
        };
        for token in pending {
            conn.execute(
                "UPDATE oauth_refresh_tokens SET token_hash = ?1 WHERE token = ?2 AND token_hash IS NULL",
                params![oauth_secret_hash(&token), token],
            )?;
        }
        ensure_no_null_hashes(
            conn,
            "SELECT COUNT(*) FROM oauth_refresh_tokens WHERE token_hash IS NULL",
            "oauth refresh token hash backfill left rows incomplete",
        )?;

        let tx = conn.unchecked_transaction()?;
        let before = table_count(&tx, "SELECT COUNT(*) FROM oauth_refresh_tokens")?;
        tx.execute_batch(
            "
            DROP TABLE IF EXISTS oauth_refresh_tokens__hashed;
            CREATE TABLE oauth_refresh_tokens__hashed (
                token_hash TEXT PRIMARY KEY,
                client_id TEXT NOT NULL,
                scope TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL
            );
            ",
        )?;
        let inserted = tx.execute(
            "
            INSERT INTO oauth_refresh_tokens__hashed (token_hash, client_id, scope, created_at, expires_at)
            SELECT token_hash, client_id, scope, created_at, expires_at
            FROM oauth_refresh_tokens
            ",
            [],
        )?;
        ensure_row_count(before, inserted, "oauth refresh token hash migration")?;
        tx.execute_batch(
            "
            DROP TABLE oauth_refresh_tokens;
            ALTER TABLE oauth_refresh_tokens__hashed RENAME TO oauth_refresh_tokens;
            CREATE INDEX IF NOT EXISTS idx_oauth_refresh_expires ON oauth_refresh_tokens(expires_at);
            ",
        )?;
        let after = table_count(&tx, "SELECT COUNT(*) FROM oauth_refresh_tokens")?;
        if after != before {
            anyhow::bail!("oauth refresh token hash migration changed the row count");
        }
        tx.commit()?;
        info!("OAuth refresh tokens are stored as hashes");
        Ok(())
    }

    fn migrate_access_token_hashes(&self, conn: &Connection) -> Result<()> {
        let columns = table_column_names(conn, "oauth_access_tokens")?;
        if !column_exists(&columns, "token") {
            conn.execute(
                "CREATE INDEX IF NOT EXISTS idx_oauth_access_expires ON oauth_access_tokens(expires_at)",
                [],
            )?;
            conn.execute(
                "CREATE INDEX IF NOT EXISTS idx_oauth_access_refresh ON oauth_access_tokens(refresh_token_hash)",
                [],
            )?;
            return Ok(());
        }
        if !column_exists(&columns, "refresh_token") {
            conn.execute(
                "ALTER TABLE oauth_access_tokens ADD COLUMN refresh_token TEXT",
                [],
            )?;
        }
        if !column_exists(&columns, "token_hash") {
            conn.execute(
                "ALTER TABLE oauth_access_tokens ADD COLUMN token_hash TEXT",
                [],
            )?;
        }
        if !column_exists(&columns, "refresh_token_hash") {
            conn.execute(
                "ALTER TABLE oauth_access_tokens ADD COLUMN refresh_token_hash TEXT",
                [],
            )?;
        }

        let pending = {
            let mut stmt = conn.prepare(
                "
                SELECT token, token_hash, refresh_token, refresh_token_hash
                FROM oauth_access_tokens
                WHERE token_hash IS NULL
                   OR (
                        refresh_token IS NOT NULL
                        AND refresh_token != ''
                        AND refresh_token_hash IS NULL
                   )
                ",
            )?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            rows
        };
        for (token, existing_hash, refresh_token, existing_link) in pending {
            let token_hash = existing_hash.unwrap_or_else(|| oauth_secret_hash(&token));
            let refresh_token_hash = match existing_link {
                Some(link) => Some(link),
                None => refresh_token
                    .as_deref()
                    .filter(|value| !value.is_empty())
                    .map(oauth_secret_hash),
            };
            conn.execute(
                "
                UPDATE oauth_access_tokens
                SET token_hash = ?1, refresh_token_hash = ?2
                WHERE token = ?3
                ",
                params![token_hash, refresh_token_hash, token],
            )?;
        }
        ensure_no_null_hashes(
            conn,
            "SELECT COUNT(*) FROM oauth_access_tokens WHERE token_hash IS NULL",
            "oauth access token hash backfill left rows incomplete",
        )?;

        let tx = conn.unchecked_transaction()?;
        let before = table_count(&tx, "SELECT COUNT(*) FROM oauth_access_tokens")?;
        tx.execute_batch(
            "
            DROP TABLE IF EXISTS oauth_access_tokens__hashed;
            CREATE TABLE oauth_access_tokens__hashed (
                token_hash TEXT PRIMARY KEY,
                client_id TEXT NOT NULL,
                scope TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL,
                refresh_token_hash TEXT
            );
            ",
        )?;
        let inserted = tx.execute(
            "
            INSERT INTO oauth_access_tokens__hashed (
                token_hash, client_id, scope, created_at, expires_at, refresh_token_hash
            )
            SELECT token_hash, client_id, scope, created_at, expires_at, refresh_token_hash
            FROM oauth_access_tokens
            ",
            [],
        )?;
        ensure_row_count(before, inserted, "oauth access token hash migration")?;
        tx.execute_batch(
            "
            DROP TABLE oauth_access_tokens;
            ALTER TABLE oauth_access_tokens__hashed RENAME TO oauth_access_tokens;
            CREATE INDEX IF NOT EXISTS idx_oauth_access_expires ON oauth_access_tokens(expires_at);
            CREATE INDEX IF NOT EXISTS idx_oauth_access_refresh ON oauth_access_tokens(refresh_token_hash);
            ",
        )?;
        let after = table_count(&tx, "SELECT COUNT(*) FROM oauth_access_tokens")?;
        if after != before {
            anyhow::bail!("oauth access token hash migration changed the row count");
        }
        tx.commit()?;
        info!("OAuth access tokens are stored as hashes");
        Ok(())
    }

    fn migrate_authorization_code_hashes(&self, conn: &Connection) -> Result<()> {
        let columns = table_column_names(conn, "oauth_authorization_codes")?;
        if !column_exists(&columns, "code") {
            conn.execute(
                "CREATE INDEX IF NOT EXISTS idx_oauth_codes_expires ON oauth_authorization_codes(expires_at)",
                [],
            )?;
            return Ok(());
        }
        if !column_exists(&columns, "code_hash") {
            conn.execute(
                "ALTER TABLE oauth_authorization_codes ADD COLUMN code_hash TEXT",
                [],
            )?;
        }

        let pending = {
            let mut stmt =
                conn.prepare("SELECT code FROM oauth_authorization_codes WHERE code_hash IS NULL")?;
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            rows
        };
        for code in pending {
            conn.execute(
                "UPDATE oauth_authorization_codes SET code_hash = ?1 WHERE code = ?2 AND code_hash IS NULL",
                params![oauth_secret_hash(&code), code],
            )?;
        }
        ensure_no_null_hashes(
            conn,
            "SELECT COUNT(*) FROM oauth_authorization_codes WHERE code_hash IS NULL",
            "oauth authorization code hash backfill left rows incomplete",
        )?;

        let tx = conn.unchecked_transaction()?;
        let before = table_count(&tx, "SELECT COUNT(*) FROM oauth_authorization_codes")?;
        tx.execute_batch(
            "
            DROP TABLE IF EXISTS oauth_authorization_codes__hashed;
            CREATE TABLE oauth_authorization_codes__hashed (
                code_hash TEXT PRIMARY KEY,
                client_id TEXT NOT NULL,
                redirect_uri TEXT NOT NULL,
                code_challenge TEXT NOT NULL,
                code_challenge_method TEXT NOT NULL,
                scope TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL,
                used INTEGER DEFAULT 0
            );
            ",
        )?;
        let inserted = tx.execute(
            "
            INSERT INTO oauth_authorization_codes__hashed (
                code_hash, client_id, redirect_uri, code_challenge, code_challenge_method,
                scope, created_at, expires_at, used
            )
            SELECT code_hash, client_id, redirect_uri, code_challenge, code_challenge_method,
                   scope, created_at, expires_at, used
            FROM oauth_authorization_codes
            ",
            [],
        )?;
        ensure_row_count(before, inserted, "oauth authorization code hash migration")?;
        tx.execute_batch(
            "
            DROP TABLE oauth_authorization_codes;
            ALTER TABLE oauth_authorization_codes__hashed RENAME TO oauth_authorization_codes;
            CREATE INDEX IF NOT EXISTS idx_oauth_codes_expires ON oauth_authorization_codes(expires_at);
            ",
        )?;
        let after = table_count(&tx, "SELECT COUNT(*) FROM oauth_authorization_codes")?;
        if after != before {
            anyhow::bail!("oauth authorization code hash migration changed the row count");
        }
        tx.commit()?;
        info!("OAuth authorization codes are stored as hashes");
        Ok(())
    }

    fn migrate_invalidate_unverified_language_labels(&self, conn: &Connection) -> Result<()> {
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

    fn migrate_add_delivery_status_column(&self, conn: &Connection) -> Result<()> {
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
    fn migrate_add_query_indexes(&self, conn: &Connection) -> Result<()> {
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
    fn migrate_add_style_profiles_table(&self, conn: &Connection) -> Result<()> {
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
    fn migrate_add_conversation_settings_columns(&self, conn: &Connection) -> Result<()> {
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
    fn migrate_add_pinned_column(&self, conn: &Connection) -> Result<()> {
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
    fn migrate_add_last_message_preview_column(&self, conn: &Connection) -> Result<()> {
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

    fn migrate_refresh_translated_contact_previews(&self, conn: &Connection) -> Result<()> {
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

    fn backfill_contact_previews(conn: &Connection) -> Result<()> {
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

        let rows: Vec<(
            String,
            Option<String>,
            Option<String>,
            Option<bool>,
            Option<String>,
        )> = stmt
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
    fn migrate_fix_contact_types(&self, conn: &Connection) -> Result<()> {
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
    fn migrate_add_translation_columns(&self, conn: &Connection) -> Result<()> {
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
    fn migrate_add_cached_usage_column(&self, conn: &Connection) -> Result<()> {
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
