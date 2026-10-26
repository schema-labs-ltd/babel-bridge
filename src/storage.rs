//! SQLite storage for messages and contacts.

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Arc, Mutex};
use tracing::info;

use crate::link_preview::LinkPreview;
use crate::oauth::{
    oauth_secret_hash, oauth_secret_hashes_equal, AccessToken, AuthorizationCode,
    OAuthClientRegistration, PendingAuthorization, RefreshToken,
};
use crate::translation::UsageInfo;
mod identity;
use crate::identity::canonical_chat_id;
mod reliability;
pub use reliability::OutboxEntry;
mod edits;
mod reactions;
mod read_receipts;
mod topics;

mod contacts;
mod link_previews;
mod messages;
mod oauth;
mod push_devices;
mod schema;
mod style;
mod usage;
mod voice;
pub use reactions::PresentedMessage;

/// Stored message with translation info
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredMessage {
    pub id: String,
    #[serde(rename = "contactId")]
    #[serde(deserialize_with = "crate::identity::deserialize_contact_id")]
    pub contact_id: String,
    pub timestamp: i64,
    #[serde(rename = "isFromMe")]
    pub is_from_me: bool,
    #[serde(rename = "isForwarded")]
    pub is_forwarded: bool,
    #[serde(rename = "senderName")]
    pub sender_name: Option<String>,
    #[serde(rename = "senderPhone")]
    pub sender_phone: Option<String>,
    /// Contact name (other person for private chats, group name for groups)
    #[serde(rename = "contactName")]
    pub contact_name: Option<String>,
    /// Contact phone (for private chats)
    #[serde(rename = "contactPhone")]
    pub contact_phone: Option<String>,
    #[serde(rename = "chatType")]
    pub chat_type: String,
    #[serde(rename = "contentType")]
    pub content_type: String,
    /// Raw JSON string stored in database
    #[serde(skip_serializing)]
    pub content_json: String,
    /// Parsed content for API responses
    #[serde(skip_deserializing)]
    pub content: Option<serde_json::Value>,
    // Translation fields
    #[serde(rename = "originalText")]
    pub original_text: Option<String>,
    #[serde(rename = "translatedText")]
    pub translated_text: Option<String>,
    #[serde(rename = "sourceLanguage")]
    pub source_language: Option<String>,
    #[serde(rename = "isTranslated")]
    pub is_translated: bool,
    #[serde(rename = "deliveryStatus", default)]
    pub delivery_status: Option<String>,
}

impl StoredMessage {
    /// The bridge stores display labels in content_type, while the payload uses wire types.
    pub fn is_audio(&self) -> bool {
        if self.content_type.eq_ignore_ascii_case("audio")
            || self.content_type.eq_ignore_ascii_case("voice note")
        {
            return true;
        }
        serde_json::from_str::<serde_json::Value>(&self.content_json)
            .ok()
            .and_then(|content| {
                content
                    .get("type")
                    .and_then(|kind| kind.as_str())
                    .map(|kind| kind.eq_ignore_ascii_case("audio"))
            })
            .unwrap_or(false)
    }
}

/// Stored contact
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredContact {
    pub id: String,
    pub name: Option<String>,
    pub phone: Option<String>,
    #[serde(rename = "type")]
    pub contact_type: Option<String>,
    #[serde(rename = "lastMessageTime")]
    pub last_message_time: i64,
    #[serde(rename = "unreadCount")]
    pub unread_count: i32,
    /// Timestamp when pinned (None = not pinned)
    #[serde(rename = "pinnedAt")]
    pub pinned_at: Option<i64>,
    /// Preview of the last message (truncated)
    #[serde(rename = "lastMessagePreview")]
    pub last_message_preview: Option<String>,
}

/// Conversation settings for per-contact customization
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSettings {
    /// Require a quoted incoming message for every outgoing send.
    #[serde(default)]
    pub reply_only: bool,
    /// Translation is opt-in for each direct chat or group.
    #[serde(default)]
    pub translation_enabled: bool,
    /// Override the target language for translations in this conversation
    /// e.g., "Spanish", "French", "Japanese"
    pub language_override: Option<String>,
    /// Style instruction for translations in this conversation
    /// e.g., "formal", "informal", "family", "robotic", "geek"
    pub translation_style: Option<String>,
    /// Send the untranslated draft as a second WhatsApp message after a translated send.
    pub send_original_follow_up: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OpenAiSettings {
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushDevice {
    pub installation_id: String,
    pub token: String,
    pub environment: String,
}

/// Style profile for AI reply generation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StyleProfile {
    /// Contact ID or "__global__" for overall style
    pub contact_id: String,
    /// AI-generated style description
    pub profile_text: String,
    /// Example messages used for analysis (JSON array)
    pub sample_messages: Vec<String>,
    /// Number of messages analyzed
    pub message_count: i32,
    /// Timestamp of last update
    pub updated_at: i64,
}

impl StyleProfile {
    /// The special contact ID used for the global style profile
    pub const GLOBAL_ID: &'static str = "__global__";

    /// Check if this is the global profile
    pub fn is_global(&self) -> bool {
        self.contact_id == Self::GLOBAL_ID
    }
}

/// Thread-safe message store backed by SQLite
pub struct MessageStore {
    conn: Arc<Mutex<Connection>>,
}

impl MessageStore {
    /// Create a new message store
    pub fn new(data_dir: &Path) -> Result<Self> {
        // Ensure data directory exists with proper permissions
        std::fs::create_dir_all(data_dir)
            .with_context(|| format!("Failed to create data directory: {:?}", data_dir))?;

        let db_path = data_dir.join("messages.db");

        info!("Opening database at {:?}", db_path);

        // Open with explicit create flag
        let conn = Connection::open_with_flags(
            &db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
                | rusqlite::OpenFlags::SQLITE_OPEN_CREATE
                | rusqlite::OpenFlags::SQLITE_OPEN_FULL_MUTEX,
        )
        .with_context(|| format!("unable to open database file: {:?}", db_path))?;

        // Tune SQLite for a read-heavy chat workload.
        conn.execute_batch(
            r#"
            PRAGMA journal_mode=WAL;
            PRAGMA synchronous=NORMAL;
            PRAGMA temp_store=MEMORY;
            PRAGMA cache_size=-20000;
            CREATE TABLE IF NOT EXISTS translation_jobs (
                message_id TEXT PRIMARY KEY,
                status TEXT NOT NULL DEFAULT 'pending',
                attempts INTEGER NOT NULL DEFAULT 0,
                retry_at INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS message_edits (
                message_id TEXT PRIMARY KEY,
                contact_id TEXT NOT NULL,
                is_from_me INTEGER NOT NULL,
                sender_phone TEXT,
                edited_at_ms INTEGER NOT NULL,
                content_json TEXT NOT NULL,
                original_text TEXT
            );
            CREATE TABLE IF NOT EXISTS pending_notifications (
                message_id TEXT PRIMARY KEY,
                requires_translation INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS outbox (
                id TEXT PRIMARY KEY,
                fingerprint TEXT NOT NULL,
                path TEXT NOT NULL,
                state TEXT NOT NULL DEFAULT 'sending',
                status_code INTEGER,
                response TEXT,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS mcp_send_records (
                id TEXT PRIMARY KEY,
                payload TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS outbox_attempts (
                request_id INTEGER PRIMARY KEY,
                outbox_id TEXT,
                temp_message_id TEXT,
                voice_note_id TEXT,
                voice_original INTEGER NOT NULL DEFAULT 0,
                expected_count INTEGER NOT NULL DEFAULT 1,
                state TEXT NOT NULL DEFAULT 'sending',
                result TEXT
            );
            CREATE TABLE IF NOT EXISTS voice_notes (
                id TEXT PRIMARY KEY,
                payload TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'prepared',
                result TEXT,
                created_at INTEGER NOT NULL
            );
            "#,
        )?;

        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
        };

        store.init_schema()?;
        store.init_topics()?;
        store.init_read_receipts()?;

        info!("Message store initialized at {:?}", db_path);

        Ok(store)
    }

    /// Flush the WAL into the database file and close the connection.
    pub fn close(&self) -> Result<()> {
        let mut guard = self
            .conn
            .lock()
            .expect("sqlite mutex poisoned during shutdown");
        guard.busy_timeout(std::time::Duration::from_millis(500))?;
        let (busy, _, _) = guard.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;
        let placeholder = Connection::open_in_memory().context("open placeholder database")?;
        let live = std::mem::replace(&mut *guard, placeholder);
        drop(guard);
        if let Err((_conn, error)) = Connection::close(live) {
            return Err(error).context("failed to close sqlite");
        }
        if busy != 0 {
            anyhow::bail!("sqlite checkpoint was busy");
        }
        Ok(())
    }

    /// Clear all data from the database (for logout)
    pub fn clear_all(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        conn.execute_batch(
            r#"
            DELETE FROM message_edits;
            DELETE FROM topic_jobs;
            DELETE FROM topic_label_jobs;
            DELETE FROM topic_assignments;
            DELETE FROM chat_topics;
            DELETE FROM topic_settings;
            DELETE FROM messages;
            DELETE FROM contacts;
            DELETE FROM translation_usage;
            DELETE FROM link_previews;
            DELETE FROM voice_notes;
            DELETE FROM translation_jobs;
            DELETE FROM pending_notifications;
            DELETE FROM outbox_attempts;
            DELETE FROM outbox;
            DELETE FROM mcp_send_records;
            DELETE FROM app_settings WHERE key LIKE 'voice:%';
            DELETE FROM app_settings WHERE key LIKE 'message-tone:%';
            "#,
        )?;

        info!("All data cleared from database");
        Ok(())
    }
}

impl Clone for MessageStore {
    fn clone(&self) -> Self {
        Self {
            conn: Arc::clone(&self.conn),
        }
    }
}

#[cfg(test)]
mod tests;
