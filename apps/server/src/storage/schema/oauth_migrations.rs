use super::*;

impl MessageStore {
    /// Link each access token to the refresh token from the same grant.
    /// Legacy databases keep their rows and gain a nullable plaintext column.
    /// Hashed databases already store that link as `refresh_token_hash`.
    pub(super) fn migrate_oauth_access_refresh_link(&self, conn: &Connection) -> Result<()> {
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
    pub(super) fn migrate_oauth_secret_hashes(&self, conn: &Connection) -> Result<()> {
        self.migrate_refresh_token_hashes(conn)?;
        self.migrate_access_token_hashes(conn)?;
        self.migrate_authorization_code_hashes(conn)?;
        Ok(())
    }

    pub(super) fn migrate_refresh_token_hashes(&self, conn: &Connection) -> Result<()> {
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

    pub(super) fn migrate_access_token_hashes(&self, conn: &Connection) -> Result<()> {
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

    pub(super) fn migrate_authorization_code_hashes(&self, conn: &Connection) -> Result<()> {
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
}
