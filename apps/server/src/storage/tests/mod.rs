use super::{schema::table_column_names, *};
use crate::oauth::oauth_secret_hash;
use std::path::PathBuf;

pub(super) fn test_store() -> (MessageStore, PathBuf) {
    let data_dir = std::env::temp_dir().join(format!(
        "whatsapp-translator-storage-test-{}",
        uuid::Uuid::new_v4()
    ));
    let store = MessageStore::new(&data_dir).expect("test store");
    (store, data_dir)
}

pub(super) fn test_message(id: &str, timestamp: i64) -> StoredMessage {
    let content = serde_json::json!({
        "type": "text",
        "body": id,
    });

    StoredMessage {
        id: id.to_string(),
        contact_id: "chat@example.test".to_string(),
        timestamp,
        is_from_me: false,
        is_forwarded: false,
        sender_name: Some("Tester".to_string()),
        sender_phone: None,
        contact_name: Some("Test Chat".to_string()),
        contact_phone: None,
        chat_type: "private".to_string(),
        content_type: "Text".to_string(),
        content_json: content.to_string(),
        content: Some(content),
        original_text: Some(id.to_string()),
        translated_text: None,
        source_language: None,
        is_translated: false,
        delivery_status: None,
    }
}

fn message_ids(messages: &[StoredMessage]) -> Vec<String> {
    messages.iter().map(|message| message.id.clone()).collect()
}

fn test_oauth_client(client_id: &str) -> OAuthClientRegistration {
    OAuthClientRegistration {
        client_id: client_id.to_string(),
        client_name: Some("Test MCP Client".to_string()),
        redirect_uris: vec!["http://127.0.0.1:9000/callback".to_string()],
        scope: "mcp".to_string(),
        created_at: 1_700_000_000,
    }
}

fn insert_test_contact(store: &MessageStore) {
    store
        .upsert_contact(
            "chat@example.test",
            Some("Test Chat"),
            None,
            Some("private"),
            1_700_000_000,
        )
        .expect("insert contact");
}

fn linked_access_token(token: &str, client_id: &str, refresh_token: Option<&str>) -> AccessToken {
    AccessToken {
        token: token.to_string(),
        client_id: client_id.to_string(),
        scope: "whatsapp.read".to_string(),
        created_at: 1_700_000_000,
        expires_at: i64::MAX,
        refresh_token: refresh_token.map(str::to_string),
    }
}

fn stored_refresh_token(token: &str, client_id: &str) -> RefreshToken {
    RefreshToken {
        token: token.to_string(),
        client_id: client_id.to_string(),
        scope: "whatsapp.read".to_string(),
        created_at: 1_700_000_000,
        expires_at: i64::MAX,
    }
}

fn authorization_code(code: &str) -> AuthorizationCode {
    AuthorizationCode {
        code: code.to_string(),
        client_id: "client_test".to_string(),
        redirect_uri: "http://127.0.0.1:8787/callback".to_string(),
        code_challenge: "challenge".to_string(),
        code_challenge_method: "S256".to_string(),
        scope: "whatsapp.read".to_string(),
        created_at: 1_700_000_000,
        expires_at: i64::MAX,
        used: false,
    }
}

fn assert_oauth_secret_schema(conn: &Connection) {
    let access = table_column_names(conn, "oauth_access_tokens").expect("access columns");
    assert!(access.iter().any(|column| column == "token_hash"));
    assert!(access.iter().any(|column| column == "refresh_token_hash"));
    assert!(!access.iter().any(|column| column == "token"));
    assert!(!access.iter().any(|column| column == "refresh_token"));

    let refresh = table_column_names(conn, "oauth_refresh_tokens").expect("refresh columns");
    assert!(refresh.iter().any(|column| column == "token_hash"));
    assert!(!refresh.iter().any(|column| column == "token"));

    let codes = table_column_names(conn, "oauth_authorization_codes").expect("code columns");
    assert!(codes.iter().any(|column| column == "code_hash"));
    assert!(!codes.iter().any(|column| column == "code"));
}

fn oauth_row_count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |row| row.get(0)).expect("count")
}

mod messages;
mod oauth;
mod oauth_hash_migration;
mod oauth_hashes;
mod settings;
mod translation;
