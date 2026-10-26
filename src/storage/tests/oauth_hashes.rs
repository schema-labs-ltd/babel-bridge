use super::*;

#[test]
fn issued_oauth_secrets_are_stored_as_hashes() {
    let (store, data_dir) = test_store();
    store
        .oauth_store_client(&test_oauth_client("client_test"))
        .expect("client");
    store
        .oauth_store_refresh_token(&stored_refresh_token("refresh-token", "client_test"))
        .expect("refresh");
    store
        .oauth_store_access_token(&linked_access_token(
            "access-token",
            "client_test",
            Some("refresh-token"),
        ))
        .expect("access");
    store
        .oauth_store_authorization_code(&authorization_code("auth-code"))
        .expect("code");

    {
        let conn = store.conn.lock().expect("database lock");
        assert_oauth_secret_schema(&conn);
        let stored_access: String = conn
            .query_row("SELECT token_hash FROM oauth_access_tokens", [], |row| {
                row.get(0)
            })
            .expect("access hash");
        assert_ne!(stored_access, "access-token");
        assert_eq!(stored_access, oauth_secret_hash("access-token"));
        let stored_link: String = conn
            .query_row(
                "SELECT refresh_token_hash FROM oauth_access_tokens",
                [],
                |row| row.get(0),
            )
            .expect("link hash");
        assert_ne!(stored_link, "refresh-token");
        assert_eq!(stored_link, oauth_secret_hash("refresh-token"));
        let stored_refresh: String = conn
            .query_row("SELECT token_hash FROM oauth_refresh_tokens", [], |row| {
                row.get(0)
            })
            .expect("refresh hash");
        assert_eq!(stored_refresh, stored_link);
        let stored_code: String = conn
            .query_row(
                "SELECT code_hash FROM oauth_authorization_codes",
                [],
                |row| row.get(0),
            )
            .expect("code hash");
        assert_ne!(stored_code, "auth-code");
        assert_eq!(stored_code, oauth_secret_hash("auth-code"));
    }

    assert!(store
        .oauth_validate_access_token("access-token")
        .expect("lookup access")
        .is_some());
    assert!(store
        .oauth_validate_access_token("wrong-access")
        .expect("reject access")
        .is_none());
    assert!(store
        .oauth_get_refresh_token("refresh-token")
        .expect("lookup refresh")
        .is_some());
    let used = store
        .oauth_use_authorization_code("auth-code")
        .expect("lookup code")
        .expect("code");
    assert_eq!(used.code, "auth-code");
    assert_eq!(used.client_id, "client_test");
    assert!(store
        .oauth_use_authorization_code("auth-code")
        .expect("second use")
        .is_none());

    drop(store);
    let reopened = MessageStore::new(&data_dir).expect("reopen hashed database");
    assert!(reopened
        .oauth_validate_access_token("access-token")
        .expect("lookup after second open")
        .is_some());
    {
        let conn = reopened.conn.lock().expect("database lock");
        assert_eq!(
            oauth_row_count(&conn, "SELECT COUNT(*) FROM oauth_access_tokens"),
            1
        );
        assert_oauth_secret_schema(&conn);
    }

    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn plaintext_oauth_secrets_work_after_hash_migration() {
    let (store, data_dir) = test_store();
    store
        .oauth_store_client(&test_oauth_client("client_existing"))
        .expect("client");
    store.close().expect("close");

    {
        let conn = Connection::open(data_dir.join("messages.db")).expect("open database");
        conn.execute_batch(
            "
                DROP TABLE oauth_access_tokens;
                DROP TABLE oauth_refresh_tokens;
                DROP TABLE oauth_authorization_codes;
                CREATE TABLE oauth_access_tokens (
                    token TEXT PRIMARY KEY,
                    client_id TEXT NOT NULL,
                    scope TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    expires_at INTEGER NOT NULL,
                    refresh_token TEXT
                );
                CREATE TABLE oauth_refresh_tokens (
                    token TEXT PRIMARY KEY,
                    client_id TEXT NOT NULL,
                    scope TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    expires_at INTEGER NOT NULL
                );
                CREATE TABLE oauth_authorization_codes (
                    code TEXT PRIMARY KEY,
                    client_id TEXT NOT NULL,
                    redirect_uri TEXT NOT NULL,
                    code_challenge TEXT NOT NULL,
                    code_challenge_method TEXT NOT NULL,
                    scope TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    expires_at INTEGER NOT NULL,
                    used INTEGER DEFAULT 0
                );
                INSERT INTO oauth_refresh_tokens (token, client_id, scope, created_at, expires_at)
                VALUES ('legacy-refresh', 'client_existing', 'whatsapp.read', 1700000000, 9223372036854775807);
                INSERT INTO oauth_access_tokens (token, client_id, scope, created_at, expires_at, refresh_token)
                VALUES ('legacy-access', 'client_existing', 'whatsapp.read', 1700000000, 9223372036854775807, 'legacy-refresh');
                INSERT INTO oauth_access_tokens (token, client_id, scope, created_at, expires_at, refresh_token)
                VALUES ('expired-access', 'client_existing', 'whatsapp.read', 1700000000, 1, NULL);
                INSERT INTO oauth_authorization_codes (
                    code, client_id, redirect_uri, code_challenge, code_challenge_method, scope, created_at, expires_at, used
                ) VALUES (
                    'legacy-code', 'client_existing', 'http://127.0.0.1:8787/callback', 'challenge', 'S256', 'whatsapp.read', 1700000000, 9223372036854775807, 0
                );
                ",
        )
        .expect("install plaintext oauth rows");
    }

    let store = MessageStore::new(&data_dir).expect("migrate");
    assert!(store
        .oauth_validate_access_token("legacy-access")
        .expect("legacy access")
        .is_some());
    assert!(store
        .oauth_validate_access_token("expired-access")
        .expect("expired access")
        .is_none());
    assert!(store
        .oauth_get_refresh_token("legacy-refresh")
        .expect("legacy refresh")
        .is_some());
    assert!(store
        .oauth_use_authorization_code("legacy-code")
        .expect("legacy code")
        .is_some());
    {
        let conn = store.conn.lock().expect("database lock");
        assert_oauth_secret_schema(&conn);
        let link: String = conn
            .query_row(
                "SELECT refresh_token_hash FROM oauth_access_tokens WHERE token_hash = ?1",
                params![oauth_secret_hash("legacy-access")],
                |row| row.get(0),
            )
            .expect("link");
        assert_eq!(link, oauth_secret_hash("legacy-refresh"));
        assert_ne!(link, "legacy-refresh");
        assert_eq!(
            oauth_row_count(&conn, "SELECT COUNT(*) FROM oauth_access_tokens"),
            2
        );
        assert_eq!(
            oauth_row_count(&conn, "SELECT COUNT(*) FROM oauth_refresh_tokens"),
            1
        );
        assert_eq!(
            oauth_row_count(&conn, "SELECT COUNT(*) FROM oauth_authorization_codes"),
            1
        );
    }

    store
        .oauth_revoke_access_session("legacy-access")
        .expect("revoke migrated grant");
    assert!(store
        .oauth_validate_access_token("legacy-access")
        .expect("revoked access")
        .is_none());
    assert!(store
        .oauth_get_refresh_token("legacy-refresh")
        .expect("revoked refresh")
        .is_none());
    assert!(store
        .oauth_validate_access_token("expired-access")
        .expect("expired row remains unvalidated")
        .is_none());

    drop(store);
    let reopened = MessageStore::new(&data_dir).expect("second migration");
    {
        let conn = reopened.conn.lock().expect("database lock");
        assert_oauth_secret_schema(&conn);
        assert_eq!(
            oauth_row_count(&conn, "SELECT COUNT(*) FROM oauth_access_tokens"),
            1
        );
        assert_eq!(
            oauth_row_count(&conn, "SELECT COUNT(*) FROM oauth_refresh_tokens"),
            0
        );
    }

    let _ = std::fs::remove_dir_all(data_dir);
}
