use super::*;

#[test]
fn oauth_hash_migration_resumes_a_partial_backfill() {
    let (store, data_dir) = test_store();
    store
        .oauth_store_client(&test_oauth_client("client_existing"))
        .expect("client");
    store.close().expect("close");

    let access_done = oauth_secret_hash("access-done");
    let refresh_done = oauth_secret_hash("refresh-done");
    let refresh_link = oauth_secret_hash("refresh-link");
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
                    refresh_token TEXT,
                    token_hash TEXT,
                    refresh_token_hash TEXT
                );
                CREATE TABLE oauth_refresh_tokens (
                    token TEXT PRIMARY KEY,
                    client_id TEXT NOT NULL,
                    scope TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    expires_at INTEGER NOT NULL,
                    token_hash TEXT
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
                    used INTEGER DEFAULT 0,
                    code_hash TEXT
                );
                ",
        )
        .expect("partial schema");
        conn.execute(
            "
                INSERT INTO oauth_access_tokens (
                    token, client_id, scope, created_at, expires_at, refresh_token, token_hash, refresh_token_hash
                ) VALUES ('access-done', 'client_existing', 'whatsapp.read', 1700000000, 9223372036854775807, 'refresh-done', ?1, ?2)
                ",
            params![access_done, refresh_done],
        )
        .expect("finished access row");
        conn.execute(
            "
                INSERT INTO oauth_access_tokens (
                    token, client_id, scope, created_at, expires_at, refresh_token, token_hash, refresh_token_hash
                ) VALUES ('access-link-pending', 'client_existing', 'whatsapp.read', 1700000000, 9223372036854775807, 'refresh-link', ?1, NULL)
                ",
            params![oauth_secret_hash("access-link-pending")],
        )
        .expect("link pending row");
        conn.execute(
            "
                INSERT INTO oauth_access_tokens (
                    token, client_id, scope, created_at, expires_at, refresh_token, token_hash, refresh_token_hash
                ) VALUES ('access-pending', 'client_existing', 'whatsapp.read', 1700000000, 9223372036854775807, 'refresh-pending', NULL, NULL)
                ",
            [],
        )
        .expect("pending access row");
        conn.execute(
            "
                INSERT INTO oauth_refresh_tokens (token, client_id, scope, created_at, expires_at, token_hash)
                VALUES ('refresh-done', 'client_existing', 'whatsapp.read', 1700000000, 9223372036854775807, ?1)
                ",
            params![refresh_done],
        )
        .expect("finished refresh row");
        for token in ["refresh-link", "refresh-pending"] {
            conn.execute(
                "
                    INSERT INTO oauth_refresh_tokens (token, client_id, scope, created_at, expires_at, token_hash)
                    VALUES (?1, 'client_existing', 'whatsapp.read', 1700000000, 9223372036854775807, NULL)
                    ",
                params![token],
            )
            .expect("pending refresh row");
        }
        conn.execute(
            "
                INSERT INTO oauth_authorization_codes (
                    code, client_id, redirect_uri, code_challenge, code_challenge_method, scope, created_at, expires_at, used, code_hash
                ) VALUES (
                    'code-done', 'client_existing', 'http://127.0.0.1:8787/callback', 'challenge', 'S256', 'whatsapp.read', 1700000000, 9223372036854775807, 0, ?1
                )
                ",
            params![oauth_secret_hash("code-done")],
        )
        .expect("finished code");
        conn.execute(
            "
                INSERT INTO oauth_authorization_codes (
                    code, client_id, redirect_uri, code_challenge, code_challenge_method, scope, created_at, expires_at, used, code_hash
                ) VALUES (
                    'code-pending', 'client_existing', 'http://127.0.0.1:8787/callback', 'challenge', 'S256', 'whatsapp.read', 1700000000, 9223372036854775807, 0, NULL
                )
                ",
            [],
        )
        .expect("pending code");
    }

    let store = MessageStore::new(&data_dir).expect("resume migration");
    for token in ["access-done", "access-link-pending", "access-pending"] {
        assert!(
            store
                .oauth_validate_access_token(token)
                .expect("validate")
                .is_some(),
            "{token}"
        );
    }
    for token in ["refresh-done", "refresh-link", "refresh-pending"] {
        assert!(
            store
                .oauth_get_refresh_token(token)
                .expect("refresh")
                .is_some(),
            "{token}"
        );
    }
    assert!(store
        .oauth_use_authorization_code("code-pending")
        .expect("pending code")
        .is_some());
    {
        let conn = store.conn.lock().expect("database lock");
        assert_oauth_secret_schema(&conn);
        assert_eq!(
            oauth_row_count(&conn, "SELECT COUNT(*) FROM oauth_access_tokens"),
            3
        );
        assert_eq!(
            oauth_row_count(
                &conn,
                "SELECT COUNT(DISTINCT token_hash) FROM oauth_access_tokens"
            ),
            3
        );
        assert_eq!(
            oauth_row_count(&conn, "SELECT COUNT(*) FROM oauth_refresh_tokens"),
            3
        );
        assert_eq!(
            oauth_row_count(&conn, "SELECT COUNT(*) FROM oauth_authorization_codes"),
            2
        );
        let done_link: String = conn
            .query_row(
                "SELECT refresh_token_hash FROM oauth_access_tokens WHERE token_hash = ?1",
                params![access_done],
                |row| row.get(0),
            )
            .expect("done link");
        assert_eq!(done_link, refresh_done);
        let pending_link: String = conn
            .query_row(
                "SELECT refresh_token_hash FROM oauth_access_tokens WHERE token_hash = ?1",
                params![oauth_secret_hash("access-link-pending")],
                |row| row.get(0),
            )
            .expect("pending link");
        assert_eq!(pending_link, refresh_link);
        let pending_access: String = conn
            .query_row(
                "SELECT token_hash FROM oauth_access_tokens WHERE token_hash = ?1",
                params![oauth_secret_hash("access-pending")],
                |row| row.get(0),
            )
            .expect("pending access");
        assert_eq!(pending_access, oauth_secret_hash("access-pending"));
    }

    store
        .oauth_revoke_access_session("access-done")
        .expect("revoke finished grant");
    assert!(store
        .oauth_get_refresh_token("refresh-done")
        .expect("revoked refresh")
        .is_none());
    assert!(store
        .oauth_validate_access_token("access-pending")
        .expect("other access")
        .is_some());
    assert!(store
        .oauth_get_refresh_token("refresh-pending")
        .expect("other refresh")
        .is_some());

    drop(store);
    let reopened = MessageStore::new(&data_dir).expect("run migration again");
    assert!(reopened
        .oauth_validate_access_token("access-pending")
        .expect("still valid")
        .is_some());
    assert!(reopened
        .oauth_validate_access_token("access-done")
        .expect("stays revoked")
        .is_none());
    {
        let conn = reopened.conn.lock().expect("database lock");
        assert_oauth_secret_schema(&conn);
        assert_eq!(
            oauth_row_count(&conn, "SELECT COUNT(*) FROM oauth_access_tokens"),
            2
        );
        assert_eq!(
            oauth_row_count(&conn, "SELECT COUNT(*) FROM oauth_refresh_tokens"),
            2
        );
        assert_eq!(
            oauth_row_count(&conn, "SELECT COUNT(*) FROM oauth_authorization_codes"),
            2
        );
    }

    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn hashed_oauth_secrets_expire_and_cleanup() {
    let (store, data_dir) = test_store();
    store
        .oauth_store_client(&test_oauth_client("client_test"))
        .expect("client");
    let mut expired_access = linked_access_token("expired-access", "client_test", None);
    expired_access.expires_at = 1;
    store
        .oauth_store_access_token(&expired_access)
        .expect("expired access");
    let mut expired_refresh = stored_refresh_token("expired-refresh", "client_test");
    expired_refresh.expires_at = 1;
    store
        .oauth_store_refresh_token(&expired_refresh)
        .expect("expired refresh");
    let mut expired_code = authorization_code("expired-code");
    expired_code.expires_at = 1;
    store
        .oauth_store_authorization_code(&expired_code)
        .expect("expired code");
    store
        .oauth_store_access_token(&linked_access_token("live-access", "client_test", None))
        .expect("live access");

    assert!(store
        .oauth_validate_access_token("expired-access")
        .expect("expired access")
        .is_none());
    assert!(store
        .oauth_get_refresh_token("expired-refresh")
        .expect("expired refresh")
        .is_none());
    assert!(store
        .oauth_use_authorization_code("expired-code")
        .expect("expired code")
        .is_none());
    assert!(store
        .oauth_validate_access_token("live-access")
        .expect("live access")
        .is_some());

    store.oauth_cleanup_expired().expect("cleanup");
    {
        let conn = store.conn.lock().expect("database lock");
        assert_eq!(
            oauth_row_count(&conn, "SELECT COUNT(*) FROM oauth_access_tokens"),
            1
        );
        assert_eq!(
            oauth_row_count(&conn, "SELECT COUNT(*) FROM oauth_refresh_tokens"),
            0
        );
        assert_eq!(
            oauth_row_count(&conn, "SELECT COUNT(*) FROM oauth_authorization_codes"),
            0
        );
    }
    assert!(store
        .oauth_validate_access_token("live-access")
        .expect("live access after cleanup")
        .is_some());

    let _ = std::fs::remove_dir_all(data_dir);
}
