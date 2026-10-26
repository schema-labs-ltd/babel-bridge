//! OAuth 2.0 clients, authorization codes and tokens.

use super::*;

impl MessageStore {
    /// Store a registered OAuth client.
    pub fn oauth_store_client(&self, client: &OAuthClientRegistration) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let redirect_uris_json = serde_json::to_string(&client.redirect_uris)?;

        conn.execute(
            r#"
            INSERT OR REPLACE INTO oauth_clients
            (client_id, client_name, redirect_uris_json, scope, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
            params![
                client.client_id,
                client.client_name,
                redirect_uris_json,
                client.scope,
                client.created_at,
            ],
        )?;

        Ok(())
    }

    /// Get a registered OAuth client.
    pub fn oauth_get_client(&self, client_id: &str) -> Result<Option<OAuthClientRegistration>> {
        let conn = self.conn.lock().unwrap();

        let result = conn.query_row(
            r#"
            SELECT client_id, client_name, redirect_uris_json, scope, created_at
            FROM oauth_clients
            WHERE client_id = ?
            "#,
            params![client_id],
            |row| {
                let redirect_uris_json: String = row.get(2)?;
                let redirect_uris =
                    serde_json::from_str(&redirect_uris_json).unwrap_or_else(|_| Vec::new());
                Ok(OAuthClientRegistration {
                    client_id: row.get(0)?,
                    client_name: row.get(1)?,
                    redirect_uris,
                    scope: row.get(3)?,
                    created_at: row.get(4)?,
                })
            },
        );

        match result {
            Ok(client) => Ok(Some(client)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// List registered OAuth clients.
    pub fn oauth_list_clients(&self) -> Result<Vec<OAuthClientRegistration>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            r#"
            SELECT client_id, client_name, redirect_uris_json, scope, created_at
            FROM oauth_clients
            ORDER BY created_at DESC, client_id ASC
            "#,
        )?;

        let rows = stmt.query_map([], |row| {
            let redirect_uris_json: String = row.get(2)?;
            let redirect_uris =
                serde_json::from_str(&redirect_uris_json).unwrap_or_else(|_| Vec::new());
            Ok(OAuthClientRegistration {
                client_id: row.get(0)?,
                client_name: row.get(1)?,
                redirect_uris,
                scope: row.get(3)?,
                created_at: row.get(4)?,
            })
        })?;

        Ok(rows.filter_map(|row| row.ok()).collect())
    }

    /// Revoke a registered OAuth client and all tokens/codes issued to it.
    pub fn oauth_revoke_client(&self, client_id: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();

        conn.execute(
            "DELETE FROM oauth_pending_auth WHERE client_id = ?",
            params![client_id],
        )?;
        conn.execute(
            "DELETE FROM oauth_authorization_codes WHERE client_id = ?",
            params![client_id],
        )?;
        conn.execute(
            "DELETE FROM oauth_access_tokens WHERE client_id = ?",
            params![client_id],
        )?;
        conn.execute(
            "DELETE FROM oauth_refresh_tokens WHERE client_id = ?",
            params![client_id],
        )?;
        let deleted = conn.execute(
            "DELETE FROM oauth_clients WHERE client_id = ?",
            params![client_id],
        )?;

        Ok(deleted > 0)
    }

    /// Clean up expired OAuth entries (call periodically)
    pub fn oauth_cleanup_expired(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        conn.execute(
            "DELETE FROM oauth_pending_auth WHERE expires_at < ?",
            params![now],
        )?;
        conn.execute(
            "DELETE FROM oauth_authorization_codes WHERE expires_at < ? OR used = 1",
            params![now],
        )?;
        conn.execute(
            "DELETE FROM oauth_access_tokens WHERE expires_at < ?",
            params![now],
        )?;
        conn.execute(
            "DELETE FROM oauth_refresh_tokens WHERE expires_at < ?",
            params![now],
        )?;

        Ok(())
    }

    /// Store a pending authorization request
    pub fn oauth_store_pending_auth(&self, pending: &PendingAuthorization) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        conn.execute(
            r#"
            INSERT OR REPLACE INTO oauth_pending_auth 
            (session_key, client_id, redirect_uri, code_challenge, code_challenge_method, scope, state, created_at, expires_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
            params![
                pending.session_key,
                pending.client_id,
                pending.redirect_uri,
                pending.code_challenge,
                pending.code_challenge_method,
                pending.scope,
                pending.state,
                pending.created_at,
                pending.expires_at,
            ],
        )?;

        Ok(())
    }

    /// Get and remove a pending authorization
    pub fn oauth_take_pending_auth(
        &self,
        session_key: &str,
    ) -> Result<Option<PendingAuthorization>> {
        let conn = self.conn.lock().unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let result = conn.query_row(
            r#"
            SELECT session_key, client_id, redirect_uri, code_challenge, code_challenge_method, 
                   scope, state, created_at, expires_at
            FROM oauth_pending_auth 
            WHERE session_key = ? AND expires_at > ?
            "#,
            params![session_key, now],
            |row| {
                Ok(PendingAuthorization {
                    session_key: row.get(0)?,
                    client_id: row.get(1)?,
                    redirect_uri: row.get(2)?,
                    code_challenge: row.get(3)?,
                    code_challenge_method: row.get(4)?,
                    scope: row.get(5)?,
                    state: row.get(6)?,
                    created_at: row.get(7)?,
                    expires_at: row.get(8)?,
                })
            },
        );

        match result {
            Ok(pending) => {
                // Delete the pending auth after retrieval
                conn.execute(
                    "DELETE FROM oauth_pending_auth WHERE session_key = ?",
                    params![session_key],
                )?;
                Ok(Some(pending))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Store an authorization code as a SHA-256 hash.
    pub fn oauth_store_authorization_code(&self, code: &AuthorizationCode) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let code_hash = oauth_secret_hash(&code.code);

        conn.execute(
            r#"
            INSERT INTO oauth_authorization_codes 
            (code_hash, client_id, redirect_uri, code_challenge, code_challenge_method, scope, created_at, expires_at, used)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
            params![
                code_hash,
                code.client_id,
                code.redirect_uri,
                code.code_challenge,
                code.code_challenge_method,
                code.scope,
                code.created_at,
                code.expires_at,
                code.used,
            ],
        )?;

        Ok(())
    }

    /// Get an authorization code by the hash of the presented code, and mark it as used.
    pub fn oauth_use_authorization_code(&self, code: &str) -> Result<Option<AuthorizationCode>> {
        let conn = self.conn.lock().unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let code_hash = oauth_secret_hash(code);

        let result = conn.query_row(
            r#"
            SELECT code_hash, client_id, redirect_uri, code_challenge, code_challenge_method, 
                   scope, created_at, expires_at, used
            FROM oauth_authorization_codes 
            WHERE code_hash = ? AND expires_at > ? AND used = 0
            "#,
            params![code_hash, now],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    AuthorizationCode {
                        code: code.to_string(),
                        client_id: row.get(1)?,
                        redirect_uri: row.get(2)?,
                        code_challenge: row.get(3)?,
                        code_challenge_method: row.get(4)?,
                        scope: row.get(5)?,
                        created_at: row.get(6)?,
                        expires_at: row.get(7)?,
                        used: row.get::<_, i32>(8)? != 0,
                    },
                ))
            },
        );

        match result {
            Ok((stored_hash, auth_code)) => {
                if !oauth_secret_hashes_equal(&stored_hash, &code_hash) {
                    return Ok(None);
                }
                conn.execute(
                    "UPDATE oauth_authorization_codes SET used = 1 WHERE code_hash = ?",
                    params![code_hash],
                )?;
                Ok(Some(auth_code))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Store an access token as a SHA-256 hash. The refresh-token link is hashed too.
    pub fn oauth_store_access_token(&self, token: &AccessToken) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let token_hash = oauth_secret_hash(&token.token);
        let refresh_token_hash = token
            .refresh_token
            .as_deref()
            .filter(|value| !value.is_empty())
            .map(oauth_secret_hash);

        conn.execute(
            r#"
            INSERT INTO oauth_access_tokens (token_hash, client_id, scope, created_at, expires_at, refresh_token_hash)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
            params![
                token_hash,
                token.client_id,
                token.scope,
                token.created_at,
                token.expires_at,
                refresh_token_hash,
            ],
        )?;

        Ok(())
    }

    /// Validate an access token by the hash of the presented token.
    pub fn oauth_validate_access_token(&self, token: &str) -> Result<Option<AccessToken>> {
        let conn = self.conn.lock().unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let token_hash = oauth_secret_hash(token);

        let result = conn.query_row(
            r#"
            SELECT token_hash, client_id, scope, created_at, expires_at
            FROM oauth_access_tokens 
            WHERE token_hash = ? AND expires_at > ?
              AND EXISTS (
                SELECT 1 FROM oauth_clients
                WHERE oauth_clients.client_id = oauth_access_tokens.client_id
              )
            "#,
            params![token_hash, now],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    AccessToken {
                        token: token.to_string(),
                        client_id: row.get(1)?,
                        scope: row.get(2)?,
                        created_at: row.get(3)?,
                        expires_at: row.get(4)?,
                        refresh_token: None,
                    },
                ))
            },
        );

        match result {
            Ok((stored_hash, access_token)) => {
                if !oauth_secret_hashes_equal(&stored_hash, &token_hash) {
                    return Ok(None);
                }
                Ok(Some(access_token))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Store a refresh token as a SHA-256 hash.
    pub fn oauth_store_refresh_token(&self, token: &RefreshToken) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let token_hash = oauth_secret_hash(&token.token);

        conn.execute(
            r#"
            INSERT INTO oauth_refresh_tokens (token_hash, client_id, scope, created_at, expires_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
            params![
                token_hash,
                token.client_id,
                token.scope,
                token.created_at,
                token.expires_at,
            ],
        )?;

        Ok(())
    }

    /// Validate and get a refresh token by the hash of the presented token.
    pub fn oauth_get_refresh_token(&self, token: &str) -> Result<Option<RefreshToken>> {
        let conn = self.conn.lock().unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let token_hash = oauth_secret_hash(token);

        let result = conn.query_row(
            r#"
            SELECT token_hash, client_id, scope, created_at, expires_at
            FROM oauth_refresh_tokens 
            WHERE token_hash = ? AND expires_at > ?
              AND EXISTS (
                SELECT 1 FROM oauth_clients
                WHERE oauth_clients.client_id = oauth_refresh_tokens.client_id
              )
            "#,
            params![token_hash, now],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    RefreshToken {
                        token: token.to_string(),
                        client_id: row.get(1)?,
                        scope: row.get(2)?,
                        created_at: row.get(3)?,
                        expires_at: row.get(4)?,
                    },
                ))
            },
        );

        match result {
            Ok((stored_hash, refresh_token)) => {
                if !oauth_secret_hashes_equal(&stored_hash, &token_hash) {
                    return Ok(None);
                }
                Ok(Some(refresh_token))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Revoke the access token a caller presented, plus the refresh token stored on that same row.
    ///
    /// The stored link is the hash of the refresh token. An access token with no grant link
    /// drops only that row. OAuth client ids are one registration, not one device, so this
    /// never selects a refresh token by client id. An unknown or already-removed token is a no-op.
    pub fn oauth_revoke_access_session(&self, token: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let token_hash = oauth_secret_hash(token);
        let linked_refresh = conn
            .query_row(
                "SELECT refresh_token_hash FROM oauth_access_tokens WHERE token_hash = ?1",
                params![token_hash],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?;
        let Some(linked_refresh) = linked_refresh else {
            return Ok(());
        };

        if let Some(refresh_hash) = linked_refresh.filter(|value| !value.is_empty()) {
            conn.execute(
                "DELETE FROM oauth_refresh_tokens WHERE token_hash = ?1",
                params![refresh_hash],
            )?;
            conn.execute(
                "DELETE FROM oauth_access_tokens WHERE token_hash = ?1 OR refresh_token_hash = ?2",
                params![token_hash, refresh_hash],
            )?;
        } else {
            conn.execute(
                "DELETE FROM oauth_access_tokens WHERE token_hash = ?1",
                params![token_hash],
            )?;
        }

        Ok(())
    }

    /// Revoke a token (access or refresh) by the hash of the presented token.
    pub fn oauth_revoke_token(&self, token: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let token_hash = oauth_secret_hash(token);

        conn.execute(
            "DELETE FROM oauth_access_tokens WHERE token_hash = ?",
            params![token_hash],
        )?;
        conn.execute(
            "DELETE FROM oauth_refresh_tokens WHERE token_hash = ?",
            params![token_hash],
        )?;

        Ok(())
    }

    /// Clear all OAuth tokens (for complete logout)
    pub fn oauth_clear_all(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        conn.execute_batch(
            r#"
            DELETE FROM oauth_pending_auth;
            DELETE FROM oauth_authorization_codes;
            DELETE FROM oauth_access_tokens;
            DELETE FROM oauth_refresh_tokens;
            DELETE FROM oauth_clients;
            "#,
        )?;

        info!("All OAuth tokens cleared from database");
        Ok(())
    }
}
