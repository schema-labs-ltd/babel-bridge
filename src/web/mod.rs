//! Web server for the WhatsApp Translator frontend.
//!
//! Provides REST API endpoints and WebSocket support for real-time updates.

use axum::{
    extract::Request,
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        DefaultBodyLimit, Host, Path, Query, State,
    },
    http::{header, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Json, Redirect, Response},
    routing::{delete, get, post, put},
    Form, Router,
};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, oneshot, watch, RwLock};
use tower_http::services::ServeDir;
use tracing::{error, info, warn};

use crate::bridge::{BridgeCommand, BridgeImage};
use crate::mcp::{McpPermissions, WhatsAppMcpServer};
use crate::oauth::{
    generate_token, AccessToken, AuthorizationCode, AuthorizeRequest, OAuthClientRegistration,
    OAuthError, OAuthErrorResponse, PendingAuthorization, RefreshToken, RevokeRequest,
    TokenRequest, TokenResponse,
};
use crate::push::{ApnsClient, ApnsSendOutcome, PushNotification};
use crate::storage::{MessageStore, OpenAiSettings, StoredMessage};
use crate::translation::TranslationService;
use tokio::sync::mpsc;

mod ai;
mod auth;
mod chats;
mod images;
mod mcp_http;
mod oauth;
mod send;
mod server;
mod state;
#[cfg(test)]
mod tests;
mod usage;
mod websocket;

use ai::*;
use auth::*;
use chats::*;
use images::*;
use mcp_http::*;
use oauth::*;
use send::*;
#[cfg(test)]
use state::*;
use usage::*;
use websocket::*;

pub(crate) use ai::translate_message;
pub(crate) use chats::present_message_page;
#[cfg(test)]
pub use server::create_router;
pub use server::start_server;

const WEB_AUTH_TOKEN_TTL_SECONDS: i64 = 12 * 60 * 60;
const MAX_IMAGE_BYTES: usize = 16 * 1024 * 1024;
const MAX_IMAGE_BASE64_BYTES: usize = MAX_IMAGE_BYTES.div_ceil(3) * 4;
const MAX_IMAGE_REQUEST_BYTES: usize = MAX_IMAGE_BASE64_BYTES + 1024 * 1024;
const MAX_ALBUM_IMAGES: usize = 30;
const MAX_ALBUM_BYTES: usize = 64 * 1024 * 1024;
const MAX_PENDING_ALBUM_BYTES: usize = 128 * 1024 * 1024;
const MAX_ALBUM_BASE64_BYTES: usize = MAX_ALBUM_BYTES.div_ceil(3) * 4;
const MAX_ALBUM_REQUEST_BYTES: usize = MAX_ALBUM_BASE64_BYTES + 2 * 1024 * 1024;
const SEND_RESULT_TIMEOUT: Duration = Duration::from_secs(30);
const ALBUM_SEND_RESULT_TIMEOUT: Duration = Duration::from_secs(180);
const OAUTH_CLEANUP_INTERVAL: Duration = Duration::from_secs(60 * 60);

fn album_send_result_timeout(photo_count: usize) -> Duration {
    Duration::from_secs((photo_count as u64 * 60).clamp(180, 15 * 60))
}

#[derive(Debug, Clone)]
struct ValidatedImagePayload {
    mime_type: String,
    decoded_size: usize,
}

#[derive(Debug)]
pub struct PendingPhotoAlbum {
    reply_only_if_not_latest: bool,
    contact_id: String,
    caption: Option<String>,
    reply_to: Option<String>,
    reply_to_sender: Option<String>,
    reply_to_text: Option<String>,
    reply_to_sender_name: Option<String>,
    images: Vec<Option<SendImageItemRequest>>,
    decoded_bytes: usize,
    created_at: i64,
}

/// Profile picture cache entry
#[derive(Debug, Clone)]
pub struct ProfilePicture {
    pub url: Option<String>,
    pub fetched_at: i64,
}

/// Shared application state
pub struct AppState {
    pub store: MessageStore,
    pub connected: RwLock<bool>,
    pub phone: RwLock<Option<String>>,
    pub name: RwLock<Option<String>>,
    pub qr_code: RwLock<Option<String>>,
    pub connection_state: RwLock<WhatsAppConnectionState>,
    pub broadcast_tx: broadcast::Sender<WebSocketEvent>,
    pub web_dir: PathBuf,
    pub data_dir: PathBuf,
    pub command_tx: RwLock<Option<mpsc::Sender<BridgeCommand>>>,
    /// The AI service, present even before any credential is configured so
    /// ChatGPT sign-in can complete at runtime. Use `translator()` for AI work.
    pub ai_service: Option<Arc<TranslationService>>,
    pub voice_lock: tokio::sync::Mutex<()>,
    pub gallery_thumbnails: crate::gallery::ThumbnailCache,
    pub voice_queue: Arc<tokio::sync::Semaphore>,
    pub voice_epoch: std::sync::atomic::AtomicU64,
    pub login_budget: tokio::sync::Mutex<crate::access::LoginBudget>,
    /// Cache of profile pictures (JID -> ProfilePicture)
    pub avatar_cache: RwLock<HashMap<String, ProfilePicture>>,
    /// Pending profile picture requests (request_id -> sender)
    pub pending_avatar_requests: RwLock<HashMap<i32, oneshot::Sender<Option<String>>>>,
    /// Pending send requests (request_id -> pending reconciliation metadata)
    pub pending_send_requests: RwLock<HashMap<i32, PendingSendRequest>>,
    pub pending_photo_albums: RwLock<HashMap<String, PendingPhotoAlbum>>,
    /// Short-lived MCP message preparations keyed by opaque preparation token.
    pub mcp_prepared_messages: RwLock<HashMap<String, serde_json::Value>>,
    /// Request ID counter
    pub request_id_counter: AtomicI32,
    /// Password for web interface (None = no password required)
    pub password: Option<String>,
    /// Valid web auth tokens and their expiration timestamps.
    pub auth_tokens: RwLock<HashMap<String, i64>>,
    /// Whether the bridge restart loop should clear WhatsApp session files before respawning.
    pub session_reset_requested: AtomicBool,
    pub push_notifications: Option<Arc<ApnsClient>>,
    /// Set to true once, when the process should stop background work and sockets.
    pub shutdown: watch::Sender<bool>,
}

/// Bridge send result normalized for Rust/web consumers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeSendResult {
    pub request_id: i32,
    pub success: bool,
    pub message_id: Option<String>,
    pub timestamp: Option<i64>,
    pub message_ids: Vec<String>,
    pub timestamps: Vec<i64>,
    pub error: Option<String>,
}

/// Error while waiting for the bridge to confirm a send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendConfirmationError {
    Timeout,
    ChannelClosed,
}

pub struct PendingSendRequest {
    temp_message_id: Option<String>,
    tx: oneshot::Sender<BridgeSendResult>,
}

/// Events sent to WebSocket clients
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WebSocketEvent {
    Status {
        connected: bool,
        phone: Option<String>,
        name: Option<String>,
        connection_state: WhatsAppConnectionState,
        qr: Option<String>,
    },
    Qr {
        data: String,
    },
    Connected {
        phone: String,
        name: String,
    },
    Disconnected,
    Message {
        message: StoredMessage,
    },
    Reaction {
        message: StoredMessage,
    },
    VoiceReady {
        message_id: String,
    },
    MessageUpdated {
        message: StoredMessage,
    },
    ConversationSettingsUpdated {
        chat_id: String,
        settings: crate::storage::ConversationSettings,
    },
    Resync,
    TopicsUpdated,
    // Part of the pinned client contract (fixtures/api); clients handle it
    // even though the server does not send it today.
    #[allow(dead_code)]
    Heartbeat,
    Typing {
        chat_id: String,
        user_id: String,
        state: String, // "typing", "paused", or "recording"
    },
    MarkAsRead {
        chat_id: String,
    },
    MessagesViewed {
        chat_id: String,
        message_ids: Vec<String>,
        unread_count: i32,
    },
    Receipt {
        message_ids: Vec<String>,
        status: String,
    },
    SendProgress {
        progress_id: String,
        stage: String,
        completed: usize,
        total: usize,
    },
    SendResult {
        request_id: i32,
        success: bool,
        temp_message_id: Option<String>,
        message_id: Option<String>,
        timestamp: Option<i64>,
        error: Option<String>,
    },
    // Part of the pinned client contract (fixtures/api).
    #[allow(dead_code)]
    Error {
        error: String,
    },
}

fn live_event_for_message(message: StoredMessage) -> WebSocketEvent {
    if message.content_type.eq_ignore_ascii_case("reaction") {
        WebSocketEvent::Reaction { message }
    } else {
        WebSocketEvent::Message { message }
    }
}

/// API status response
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WhatsAppConnectionState {
    Connecting,
    Connected,
    Reconnecting,
    LinkingRequired,
    /// The bridge stopped retrying. Another dial will not succeed until the
    /// reported cause is fixed.
    Disconnected,
}

impl WhatsAppConnectionState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Reconnecting => "reconnecting",
            Self::LinkingRequired => "linking_required",
            Self::Disconnected => "disconnected",
        }
    }
}

#[derive(Serialize)]
struct StatusResponse {
    connected: bool,
    phone: Option<String>,
    name: Option<String>,
    connection_state: WhatsAppConnectionState,
    qr: Option<String>,
}

#[derive(Serialize)]
struct HealthResponse {
    ok: bool,
    service: &'static str,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegisterPushDeviceRequest {
    installation_id: String,
    token: String,
    environment: String,
}

/// API QR response
#[derive(Serialize)]
struct QrResponse {
    qr: Option<String>,
}

/// Send message request
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendMessageRequest {
    #[serde(deserialize_with = "crate::identity::deserialize_contact_id")]
    pub contact_id: String,
    pub text: String,
    /// Message ID to reply to (optional)
    pub reply_to: Option<String>,
    /// Sender JID of the replied message (optional)
    pub reply_to_sender: Option<String>,
    /// Text preview of the replied message (for storage)
    pub reply_to_text: Option<String>,
    /// Sender display name of the replied message (for storage)
    pub reply_to_sender_name: Option<String>,
    /// Unified feed: quote only when another message has since arrived in this chat.
    #[serde(default)]
    pub reply_only_if_not_latest: bool,
}

/// Send message response
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendMessageResponse {
    pub message_id: String,
    pub timestamp: i64,
    /// Whether the message was translated before sending
    pub is_translated: bool,
    /// The translated text that was actually sent (if translated)
    pub translated_text: Option<String>,
    /// The target language (if translated)
    pub source_language: Option<String>,
    /// Whether the original draft was confirmed as a second WhatsApp message.
    pub original_follow_up_sent: bool,
    pub original_message_id: Option<String>,
    pub original_timestamp: Option<i64>,
    pub original_follow_up_error: Option<String>,
}

/// Send image request
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendImageRequest {
    pub media_kind: Option<String>,
    pub file_name: Option<String>,
    #[serde(default)]
    pub reply_only_if_not_latest: bool,
    #[serde(deserialize_with = "crate::identity::deserialize_contact_id")]
    pub contact_id: String,
    /// Base64 encoded image data
    pub media_data: String,
    pub mime_type: String,
    pub caption: Option<String>,
    /// Message ID to reply to (optional)
    pub reply_to: Option<String>,
    /// Sender JID of the replied message (optional)
    pub reply_to_sender: Option<String>,
    /// Text preview of the replied message (for storage)
    pub reply_to_text: Option<String>,
    /// Sender display name of the replied message (for storage)
    pub reply_to_sender_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendImageItemRequest {
    pub media_data: String,
    pub mime_type: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatePhotoAlbumRequest {
    #[serde(default)]
    pub reply_only_if_not_latest: bool,
    pub job_id: String,
    #[serde(deserialize_with = "crate::identity::deserialize_contact_id")]
    pub contact_id: String,
    pub photo_count: usize,
    pub caption: Option<String>,
    pub reply_to: Option<String>,
    pub reply_to_sender: Option<String>,
    pub reply_to_text: Option<String>,
    pub reply_to_sender_name: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendImagesRequest {
    #[serde(default)]
    pub reply_only_if_not_latest: bool,
    #[serde(deserialize_with = "crate::identity::deserialize_contact_id")]
    pub contact_id: String,
    pub progress_id: Option<String>,
    pub images: Vec<SendImageItemRequest>,
    pub caption: Option<String>,
    pub reply_to: Option<String>,
    pub reply_to_sender: Option<String>,
    pub reply_to_text: Option<String>,
    pub reply_to_sender_name: Option<String>,
}

/// Send image response
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendImageResponse {
    pub message_id: String,
    pub timestamp: i64,
}

/// Send reaction request
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendReactionRequest {
    #[serde(deserialize_with = "crate::identity::deserialize_contact_id")]
    pub contact_id: String,
    pub message_id: String,
    pub sender_jid: Option<String>,
    pub emoji: String,
}

/// Send reaction response
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendReactionResponse {
    pub success: bool,
    pub reaction: Option<StoredMessage>,
}

/// Mark-read request
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkReadRequest {
    pub message_id: Option<String>,
    #[serde(default)]
    pub message_ids: Vec<String>,
}

/// Translate message request
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslateMessageRequest {
    #[allow(dead_code)] // Sent by clients; the server translates the stored text.
    pub text: String,
    pub message_id: String,
    #[serde(deserialize_with = "crate::identity::deserialize_contact_id")]
    pub contact_id: String,
}

/// Translate message response
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslateMessageResponse {
    pub success: bool,
    pub translated_text: Option<String>,
    pub source_language: Option<String>,
    pub error: Option<String>,
}

/// AI compose request
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiComposeRequest {
    pub prompt: String,
    /// Optional: the message being replied to (for context)
    pub reply_to_text: Option<String>,
    /// Optional: who sent the message being replied to
    pub reply_to_sender: Option<String>,
    /// Optional: base64 image data if replying to an image
    pub reply_to_image: Option<String>,
    /// Optional: mime type of the image (e.g., "image/jpeg")
    pub reply_to_image_type: Option<String>,
}

/// AI compose response
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiComposeResponse {
    pub success: bool,
    pub message: Option<String>,
    pub error: Option<String>,
    pub cost_usd: Option<f64>,
}

/// AI styled reply request - generates a reply that sounds like the user
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiReplyRequest {
    #[serde(deserialize_with = "crate::identity::deserialize_contact_id")]
    pub contact_id: String,
    pub message_id: String,
}

/// AI styled reply response
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiReplyResponse {
    pub success: bool,
    pub reply_text: Option<String>,
    pub error: Option<String>,
    pub cost_usd: Option<f64>,
}

/// Auth check response
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthCheckResponse {
    pub required: bool,
}

/// Auth request
#[derive(Deserialize)]
pub struct AuthRequest {
    pub password: String,
}

/// Auth response
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthResponse {
    pub success: bool,
    pub token: Option<String>,
    pub error: Option<String>,
}

/// Conversation settings request/response
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateConversationSettingsRequest {
    pub reply_only: Option<bool>,
    // Older native builds omit this field when saving their existing settings.
    // Preserve an explicit choice rather than silently resetting it.
    pub translation_enabled: Option<bool>,
    /// Override the target language for translations (plain text, e.g., "Spanish")
    pub language_override: Option<String>,
    /// Style instruction for translations (plain text, e.g., "formal", "casual")
    pub translation_style: Option<String>,
    #[serde(default)]
    pub send_original_follow_up: bool,
}

/// Conversation settings response
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSettingsResponse {
    pub reply_only: bool,
    pub translation_enabled: bool,
    pub language_override: Option<String>,
    pub translation_style: Option<String>,
    pub send_original_follow_up: bool,
}

const SUPPORTED_OPENAI_MODELS: &[&str] = &[
    "gpt-6-astra",
    "gpt-5.6-sol",
    "gpt-5.6-terra",
    "gpt-5.6-luna",
];
const SUPPORTED_REASONING_EFFORTS: &[&str] = &["none", "low", "medium", "high", "xhigh", "max"];

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateOpenAiSettingsRequest {
    model: Option<String>,
    reasoning_effort: Option<String>,
}
