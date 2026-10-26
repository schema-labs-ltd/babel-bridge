//! MCP (Model Context Protocol) server implementation for WhatsApp.
//!
//! The MCP surface is deliberately split into read-only discovery/context tools and
//! explicitly mutating tools. Text sends use a two-step prepare/send flow so the
//! exact recipient and translated text are fixed before WhatsApp is touched.

use crate::bridge::BridgeCommand;
use crate::storage::{StoredContact, StoredMessage};
use crate::web::{AppState, SendConfirmationError};
use rmcp::{
    model::{
        CallToolRequestParam, CallToolResult, Icon, Implementation, InitializeRequestParam,
        InitializeResult, ListResourcesResult, ListToolsResult, PaginatedRequestParam, RawResource,
        ReadResourceRequestParam, ReadResourceResult, ResourceContents, ServerCapabilities,
        ServerInfo, Tool, ToolAnnotations,
    },
    service::RequestContext,
    ErrorData as McpError, RoleServer, ServerHandler,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Instant;
use tracing::{info, warn};

mod chat_actions;
mod read_tools;
mod send_tools;
mod server_handler;
mod tool_definitions;

const CHAT_RESOURCE_URI: &str = "ui://whatsapp-translator/chats";
const CHAT_MIME_TYPE: &str = "text/html;profile=mcp-app";
const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 200;
const FEED_MAX_LIMIT: u32 = 100;
const JS_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
const PREPARATION_TTL_MILLIS: i64 = 10 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct McpPermissions {
    pub read: bool,
    pub send: bool,
}

impl McpPermissions {
    pub fn from_scope(scope: &str) -> Self {
        let scopes: Vec<&str> = scope.split_whitespace().collect();
        let legacy_full_access = scopes.contains(&"mcp");
        Self {
            read: legacy_full_access || scopes.contains(&"whatsapp.read"),
            send: legacy_full_access || scopes.contains(&"whatsapp.send"),
        }
    }
}

#[derive(Clone)]
pub struct WhatsAppMcpServer {
    state: Arc<AppState>,
    permissions: McpPermissions,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ContactInfo {
    id: String,
    name: Option<String>,
    phone: Option<String>,
    #[serde(rename = "type")]
    contact_type: Option<String>,
    unread_count: i32,
    is_pinned: bool,
    last_message_time: i64,
    last_message_time_iso: Option<String>,
    last_message_preview: Option<String>,
}

impl From<StoredContact> for ContactInfo {
    fn from(contact: StoredContact) -> Self {
        Self {
            id: contact.id,
            name: contact.name,
            phone: contact.phone,
            contact_type: contact.contact_type,
            unread_count: contact.unread_count,
            is_pinned: contact.pinned_at.is_some(),
            last_message_time: contact.last_message_time,
            last_message_time_iso: timestamp_millis_to_iso(contact.last_message_time),
            last_message_preview: contact.last_message_preview,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct MessageInfo {
    id: String,
    contact_id: String,
    timestamp: i64,
    timestamp_iso: Option<String>,
    is_from_me: bool,
    sender_name: Option<String>,
    sender_phone: Option<String>,
    text: Option<String>,
    original_text: Option<String>,
    translated_text: Option<String>,
    content_type: String,
    delivery_status: Option<String>,
    reply_context: Option<Value>,
}

impl From<StoredMessage> for MessageInfo {
    fn from(message: StoredMessage) -> Self {
        let original_text = message.original_text.clone().or_else(|| {
            message.content.as_ref().and_then(|content| {
                content
                    .get("body")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| {
                        content
                            .get("caption")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
            })
        });
        let text = if message.is_translated && !message.is_from_me {
            message.translated_text.clone()
        } else {
            original_text.clone()
        };
        let reply_context = message
            .content
            .as_ref()
            .and_then(|content| content.get("reply_context"))
            .filter(|value| !value.is_null())
            .cloned();

        Self {
            id: message.id,
            contact_id: message.contact_id,
            timestamp: message.timestamp,
            timestamp_iso: timestamp_millis_to_iso(message.timestamp),
            is_from_me: message.is_from_me,
            sender_name: message.sender_name,
            sender_phone: message.sender_phone,
            text,
            original_text,
            translated_text: message.translated_text,
            content_type: message.content_type,
            delivery_status: message.delivery_status,
            reply_context,
        }
    }
}

/// WA-FEED-CONTRACT-v1: carry source identity independently of loaded contacts.
/// The existing chat DTO and its text selection semantics remain unchanged.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FeedMessage {
    id: String,
    contact_id: String,
    contact_name: Option<String>,
    contact_phone: Option<String>,
    chat_type: String,
    timestamp: i64,
    timestamp_iso: Option<String>,
    is_from_me: bool,
    sender_name: Option<String>,
    sender_phone: Option<String>,
    text: Option<String>,
    original_text: Option<String>,
    translated_text: Option<String>,
    is_translated: bool,
    source_language: Option<String>,
    content_type: String,
    delivery_status: Option<String>,
    reply_context: Option<FeedQuote>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FeedQuote {
    message_id: Option<String>,
    sender_name: Option<String>,
    text: Option<String>,
}

impl From<StoredMessage> for FeedMessage {
    fn from(message: StoredMessage) -> Self {
        let contact_name = message.contact_name.clone();
        let contact_phone = message.contact_phone.clone();
        let chat_type = message.chat_type.clone();
        let is_translated = message.is_translated;
        let source_language = message.source_language.clone();
        let info = MessageInfo::from(message);
        let reply_context = info
            .reply_context
            .as_ref()
            .filter(|quote| quote.is_object())
            .map(|quote| FeedQuote {
                message_id: quote["messageId"].as_str().map(str::to_owned),
                sender_name: quote["senderName"].as_str().map(str::to_owned),
                text: quote["text"].as_str().map(str::to_owned),
            });
        Self {
            id: info.id,
            contact_id: info.contact_id,
            contact_name,
            contact_phone,
            chat_type,
            timestamp: info.timestamp,
            timestamp_iso: info.timestamp_iso,
            is_from_me: info.is_from_me,
            sender_name: info.sender_name,
            sender_phone: info.sender_phone,
            text: info.text,
            original_text: info.original_text,
            translated_text: info.translated_text,
            is_translated,
            source_language,
            content_type: info.content_type,
            delivery_status: info.delivery_status,
            reply_context,
        }
    }
}

struct FeedArguments {
    limit: u32,
    before_timestamp: Option<i64>,
    before_message_id: Option<String>,
}

impl FeedArguments {
    fn parse(args: &Value) -> Result<Self, McpError> {
        let object = args
            .as_object()
            .ok_or_else(|| McpError::invalid_params("Feed arguments must be an object", None))?;
        if object.keys().any(|key| {
            !matches!(
                key.as_str(),
                "limit" | "before_timestamp" | "before_message_id"
            )
        }) {
            return Err(McpError::invalid_params("Unknown feed argument", None));
        }
        let integer = |value: &Value, maximum: i64, name: &str| {
            // JSON Schema integers also include numbers such as 2.0. Check
            // bounds before conversion, including the JavaScript-safe cursor.
            value
                .as_f64()
                .filter(|number| {
                    number.is_finite()
                        && *number >= 0.0
                        && *number <= maximum as f64
                        && number.fract() == 0.0
                })
                .map(|number| number as i64)
                .ok_or_else(|| {
                    McpError::invalid_params(
                        format!("{name} must be an integer from 0 to {maximum}"),
                        None,
                    )
                })
        };
        let limit = match object.get("limit") {
            Some(value) => {
                let limit = integer(value, i64::from(FEED_MAX_LIMIT), "limit")?;
                if limit == 0 {
                    return Err(McpError::invalid_params(
                        "limit must be from 1 to 100",
                        None,
                    ));
                }
                limit as u32
            }
            None => DEFAULT_LIMIT as u32,
        };
        let (before_timestamp, before_message_id) = match (
            object.get("before_timestamp"),
            object.get("before_message_id"),
        ) {
            (None, None) => (None, None),
            (Some(timestamp), Some(id)) => {
                let timestamp = integer(timestamp, JS_SAFE_INTEGER, "before_timestamp")?;
                let id = id.as_str().filter(|id| !id.is_empty()).ok_or_else(|| {
                    McpError::invalid_params("before_message_id must be a nonempty string", None)
                })?;
                (Some(timestamp), Some(id.to_owned()))
            }
            _ => {
                return Err(McpError::invalid_params(
                    "Feed cursor fields must be supplied together",
                    None,
                ))
            }
        };
        Ok(Self {
            limit,
            before_timestamp,
            before_message_id,
        })
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FeedCursor {
    before_timestamp: i64,
    before_message_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FeedPage {
    messages: Vec<FeedMessage>,
    has_more: bool,
    next_cursor: Option<FeedCursor>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TranslationMode {
    Auto,
    Never,
    Required,
}

impl TranslationMode {
    fn parse(value: Option<&str>) -> Result<Self, McpError> {
        match value.unwrap_or("auto") {
            "auto" => Ok(Self::Auto),
            "never" => Ok(Self::Never),
            "required" => Ok(Self::Required),
            _ => Err(McpError::invalid_params(
                "translation_mode must be auto, never, or required",
                None,
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Never => "never",
            Self::Required => "required",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedMcpMessage {
    pub token: String,
    #[serde(deserialize_with = "crate::identity::deserialize_contact_id")]
    pub contact_id: String,
    pub contact_name: Option<String>,
    pub original_text: String,
    pub final_text: String,
    pub translated: bool,
    pub target_language: Option<String>,
    pub translation_mode: String,
    pub reply_to_message_id: Option<String>,
    pub reply_to_sender: Option<String>,
    pub reply_to_text: Option<String>,
    pub created_at: i64,
    pub expires_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpIdempotencyRecord {
    pub operation_key: String,
    pub status: String,
    pub result: Option<Value>,
    pub expires_at: i64,
}

fn timestamp_millis_to_iso(timestamp: i64) -> Option<String> {
    chrono::DateTime::from_timestamp_millis(timestamp).map(|value| value.to_rfc3339())
}

fn argument_limit(args: &Value) -> usize {
    args.get("limit")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(DEFAULT_LIMIT)
        .clamp(1, MAX_LIMIT)
}

fn resolve_prepared_translation(
    original_text: &str,
    target_language: Option<&str>,
    mode: TranslationMode,
    translation_result: Result<String, String>,
) -> Result<(String, bool, Option<String>), String> {
    if mode == TranslationMode::Never {
        return Ok((original_text.to_string(), false, None));
    }

    let Some(target_language) = target_language.filter(|value| !value.trim().is_empty()) else {
        return if mode == TranslationMode::Required {
            Err("Translation was required but no target language could be determined. No message was sent or prepared.".to_string())
        } else {
            Ok((original_text.to_string(), false, None))
        };
    };

    match translation_result {
        Ok(translated_text) if !translated_text.trim().is_empty() => Ok((
            translated_text.clone(),
            translated_text != original_text,
            Some(target_language.to_string()),
        )),
        Ok(_) => Err(format!(
            "Translation to {target_language} returned no text. No message was sent or prepared."
        )),
        Err(error) => Err(format!(
            "Translation to {target_language} failed: {error}. No message was sent or prepared."
        )),
    }
}

impl WhatsAppMcpServer {
    pub fn new(state: Arc<AppState>, permissions: McpPermissions) -> Self {
        Self { state, permissions }
    }

    fn tool(
        name: &'static str,
        title: &'static str,
        description: &'static str,
        input_schema: Value,
        output_schema: Value,
        annotations: ToolAnnotations,
    ) -> Tool {
        let mut tool = Tool::new(
            name,
            description,
            input_schema.as_object().expect("tool input schema").clone(),
        );
        tool.title = Some(title.to_string());
        tool.output_schema = Some(Arc::new(
            output_schema
                .as_object()
                .expect("tool output schema")
                .clone(),
        ));
        tool.annotate(annotations)
    }

    fn read_annotations(title: &str) -> ToolAnnotations {
        ToolAnnotations::with_title(title)
            .read_only(true)
            .destructive(false)
            .idempotent(true)
            .open_world(true)
    }

    fn write_annotations(title: &str, idempotent: bool) -> ToolAnnotations {
        ToolAnnotations::with_title(title)
            .read_only(false)
            .destructive(false)
            .idempotent(idempotent)
            .open_world(true)
    }

    fn object_output(properties: Value, required: &[&str]) -> Value {
        json!({
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": true
        })
    }

    fn advertised_tools(&self) -> Vec<Tool> {
        let mut tools = Vec::new();
        if self.permissions.read {
            tools.extend([
                Self::opener_tool(false),
                Self::opener_tool(true),
                Self::get_status_tool(),
                Self::list_contacts_tool(),
                Self::search_contacts_tool(),
                Self::read_messages_tool(),
                Self::read_unified_messages_tool(),
                Self::search_messages_tool(),
                Self::prepare_message_tool(),
            ]);
        }
        if self.permissions.send {
            tools.extend([
                Self::send_message_tool(),
                Self::reply_to_message_tool(),
                Self::react_to_message_tool(),
                Self::mark_conversation_read_tool(),
            ]);
        }
        if self.permissions.read && self.permissions.send {
            tools.push(Self::translate_message_tool());
        }
        tools
    }
}

fn required_string<'a>(args: &'a Value, name: &str) -> Result<&'a str, McpError> {
    args.get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| McpError::invalid_params(format!("{name} is required"), None))
}

trait McpResultExt<T> {
    fn mcp(self) -> Result<T, McpError>;
}

impl<T> McpResultExt<T> for anyhow::Result<T> {
    fn mcp(self) -> Result<T, McpError> {
        self.map_err(|error| McpError::internal_error(error.to_string(), None))
    }
}

fn tool_result_count(result: &CallToolResult) -> usize {
    let Some(structured) = result.structured_content.as_ref() else {
        return 0;
    };
    for key in ["messages", "contacts", "chats", "tools", "resources"] {
        if let Some(items) = structured.get(key).and_then(Value::as_array) {
            return items.len();
        }
    }
    0
}

#[cfg(test)]
mod tests;
