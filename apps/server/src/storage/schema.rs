//! Schema creation and migrations.

use super::*;

mod column_migrations;
mod oauth_migrations;

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
}
