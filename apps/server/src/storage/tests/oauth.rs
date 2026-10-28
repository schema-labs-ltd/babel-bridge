use super::*;

#[test]
fn oauth_tokens_require_persisted_client_and_revoke_with_client() {
    let (store, data_dir) = test_store();
    let access_token = AccessToken {
        token: "access-token".to_string(),
        client_id: "client_test".to_string(),
        scope: "mcp".to_string(),
        created_at: 1_700_000_000,
        expires_at: i64::MAX,
        refresh_token: None,
    };
    let refresh_token = RefreshToken {
        token: "refresh-token".to_string(),
        client_id: "client_test".to_string(),
        scope: "mcp".to_string(),
        created_at: 1_700_000_000,
        expires_at: i64::MAX,
    };

    store
        .oauth_store_access_token(&access_token)
        .expect("store access token");
    store
        .oauth_store_refresh_token(&refresh_token)
        .expect("store refresh token");
    assert!(store
        .oauth_validate_access_token("access-token")
        .expect("validate without client")
        .is_none());

    store
        .oauth_store_client(&test_oauth_client("client_test"))
        .expect("store client");
    assert!(store
        .oauth_validate_access_token("access-token")
        .expect("validate with client")
        .is_some());
    assert!(store
        .oauth_get_refresh_token("refresh-token")
        .expect("refresh with client")
        .is_some());

    assert!(store
        .oauth_revoke_client("client_test")
        .expect("revoke client"));
    assert!(store
        .oauth_validate_access_token("access-token")
        .expect("validate after revoke")
        .is_none());
    assert!(store
        .oauth_get_refresh_token("refresh-token")
        .expect("refresh after revoke")
        .is_none());

    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn oauth_access_session_revoke_removes_only_the_linked_refresh_family() {
    let (store, data_dir) = test_store();
    store
        .oauth_store_client(&test_oauth_client("client-a"))
        .expect("client a");
    store
        .oauth_store_client(&test_oauth_client("client-b"))
        .expect("client b");
    store
        .oauth_store_refresh_token(&stored_refresh_token("refresh-a", "client-a"))
        .expect("refresh a");
    store
        .oauth_store_refresh_token(&stored_refresh_token("refresh-a-other", "client-a"))
        .expect("other refresh");
    store
        .oauth_store_refresh_token(&stored_refresh_token("refresh-b", "client-b"))
        .expect("refresh b");
    store
        .oauth_store_access_token(&linked_access_token(
            "access-a",
            "client-a",
            Some("refresh-a"),
        ))
        .expect("access a");
    store
        .oauth_store_access_token(&linked_access_token(
            "access-a-renewed",
            "client-a",
            Some("refresh-a"),
        ))
        .expect("renewed access");
    store
        .oauth_store_access_token(&linked_access_token(
            "access-a-other",
            "client-a",
            Some("refresh-a-other"),
        ))
        .expect("other access");
    store
        .oauth_store_access_token(&linked_access_token(
            "access-b",
            "client-b",
            Some("refresh-b"),
        ))
        .expect("access b");
    store
        .oauth_store_access_token(&linked_access_token("access-a-unlinked", "client-a", None))
        .expect("unlinked access on the same client");

    store
        .oauth_revoke_access_session("access-a")
        .expect("revoke calling grant");
    store
        .oauth_revoke_access_session("access-a")
        .expect("revoke again");
    store
        .oauth_revoke_access_session("missing-token")
        .expect("unknown token");

    assert!(store
        .oauth_validate_access_token("access-a")
        .expect("access a")
        .is_none());
    assert!(store
        .oauth_validate_access_token("access-a-renewed")
        .expect("renewed")
        .is_none());
    assert!(store
        .oauth_get_refresh_token("refresh-a")
        .expect("refresh a")
        .is_none());
    assert!(store
        .oauth_validate_access_token("access-a-other")
        .expect("other access")
        .is_some());
    assert!(store
        .oauth_get_refresh_token("refresh-a-other")
        .expect("other refresh")
        .is_some());
    assert!(store
        .oauth_validate_access_token("access-b")
        .expect("access b")
        .is_some());
    assert!(store
        .oauth_get_refresh_token("refresh-b")
        .expect("refresh b")
        .is_some());
    assert!(store
        .oauth_validate_access_token("access-a-unlinked")
        .expect("unlinked access")
        .is_some());
    assert_eq!(store.oauth_list_clients().expect("clients").len(), 2);

    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn oauth_access_session_revoke_of_unlinked_token_leaves_the_clients_only_refresh_grant() {
    let (store, data_dir) = test_store();
    store
        .oauth_store_client(&test_oauth_client("client-shared"))
        .expect("client");
    store
        .oauth_store_refresh_token(&stored_refresh_token("refresh-shared", "client-shared"))
        .expect("shared refresh");
    store
        .oauth_store_access_token(&linked_access_token(
            "access-unlinked",
            "client-shared",
            None,
        ))
        .expect("unlinked access");
    store
        .oauth_store_access_token(&linked_access_token(
            "access-other-device",
            "client-shared",
            Some("refresh-shared"),
        ))
        .expect("other device access");

    store
        .oauth_revoke_access_session("access-unlinked")
        .expect("revoke unlinked access token");
    store
        .oauth_revoke_access_session("access-unlinked")
        .expect("already revoked");

    assert!(store
        .oauth_validate_access_token("access-unlinked")
        .expect("unlinked")
        .is_none());
    assert!(store
        .oauth_validate_access_token("access-other-device")
        .expect("other device")
        .is_some());
    assert!(store
        .oauth_get_refresh_token("refresh-shared")
        .expect("shared refresh")
        .is_some());
    assert_eq!(store.oauth_list_clients().expect("clients").len(), 1);

    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn oauth_access_session_revoke_leaves_ambiguous_refresh_tokens_in_place() {
    let (store, data_dir) = test_store();
    store
        .oauth_store_client(&test_oauth_client("client-legacy"))
        .expect("client");
    store
        .oauth_store_refresh_token(&stored_refresh_token("refresh-one", "client-legacy"))
        .expect("refresh one");
    store
        .oauth_store_refresh_token(&stored_refresh_token("refresh-two", "client-legacy"))
        .expect("refresh two");
    store
        .oauth_store_access_token(&linked_access_token(
            "access-unlinked",
            "client-legacy",
            None,
        ))
        .expect("unlinked access");

    store
        .oauth_revoke_access_session("access-unlinked")
        .expect("revoke unlinked");
    store
        .oauth_revoke_access_session("access-unlinked")
        .expect("already revoked");

    assert!(store
        .oauth_validate_access_token("access-unlinked")
        .expect("access")
        .is_none());
    assert!(store
        .oauth_get_refresh_token("refresh-one")
        .expect("refresh one")
        .is_some());
    assert!(store
        .oauth_get_refresh_token("refresh-two")
        .expect("refresh two")
        .is_some());

    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn existing_oauth_access_tokens_gain_refresh_token_column() {
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
                CREATE TABLE oauth_access_tokens (
                    token TEXT PRIMARY KEY,
                    client_id TEXT NOT NULL,
                    scope TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    expires_at INTEGER NOT NULL
                );
                INSERT INTO oauth_access_tokens (token, client_id, scope, created_at, expires_at)
                VALUES ('existing-access', 'client_existing', 'whatsapp.read', 1700000000, 9223372036854775807);
                ",
        )
        .expect("restore legacy access token table");
    }

    let store = MessageStore::new(&data_dir).expect("reopen");
    let token = store
        .oauth_validate_access_token("existing-access")
        .expect("validate")
        .expect("existing token");
    assert!(token.refresh_token.is_none());
    {
        let conn = store.conn.lock().expect("database lock");
        let link: Option<String> = conn
            .query_row(
                "SELECT refresh_token_hash FROM oauth_access_tokens WHERE token_hash = ?1",
                params![oauth_secret_hash("existing-access")],
                |row| row.get(0),
            )
            .expect("link");
        assert!(link.is_none());
        assert_oauth_secret_schema(&conn);
    }
    store
        .oauth_store_access_token(&linked_access_token(
            "linked-access",
            "client_existing",
            Some("refresh-existing"),
        ))
        .expect("store linked token");
    store
        .oauth_store_refresh_token(&stored_refresh_token("refresh-existing", "client_existing"))
        .expect("store refresh");
    let linked = store
        .oauth_validate_access_token("linked-access")
        .expect("validate linked")
        .expect("linked token");
    assert_eq!(linked.client_id, "client_existing");
    assert!(linked.refresh_token.is_none());
    {
        let conn = store.conn.lock().expect("database lock");
        let link: String = conn
            .query_row(
                "SELECT refresh_token_hash FROM oauth_access_tokens WHERE token_hash = ?1",
                params![oauth_secret_hash("linked-access")],
                |row| row.get(0),
            )
            .expect("hashed link");
        assert_eq!(link, oauth_secret_hash("refresh-existing"));
        assert_ne!(link, "refresh-existing");
    }
    store
        .oauth_revoke_access_session("linked-access")
        .expect("revoke linked grant");
    assert!(store
        .oauth_get_refresh_token("refresh-existing")
        .expect("refresh")
        .is_none());
    assert!(store
        .oauth_validate_access_token("existing-access")
        .expect("legacy access")
        .is_some());

    let _ = std::fs::remove_dir_all(data_dir);
}
