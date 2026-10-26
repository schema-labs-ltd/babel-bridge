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

    fn get_status_tool() -> Tool {
        Self::tool(
            "get_status",
            "Get WhatsApp status",
            "Check whether WhatsApp and translation are available before reading or sending.",
            json!({"type":"object","properties":{},"additionalProperties":false}),
            Self::object_output(
                json!({
                    "connected":{"type":"boolean"},
                    "connectionState":{"type":"string","enum":["connecting","connected","reconnecting","disconnected","linking_required"]},
                    "accountName":{"type":["string","null"]},
                    "accountPhone":{"type":["string","null"]},
                    "translationConfigured":{"type":"boolean"},
                    "contactCount":{"type":"integer"},
                    "messageCount":{"type":"integer"}
                }),
                &[
                    "connected",
                    "connectionState",
                    "translationConfigured",
                    "contactCount",
                    "messageCount",
                ],
            ),
            Self::read_annotations("Get WhatsApp status"),
        )
    }

    fn contacts_input_schema(require_query: bool) -> Value {
        let mut schema = json!({
            "type":"object",
            "properties":{
                "query":{"type":"string","description":"Case-insensitive name, phone, or JID search"},
                "contact_type":{"type":"string","enum":["all","private","group"],"default":"all"},
                "unread_only":{"type":"boolean","default":false},
                "cursor":{"type":"integer","minimum":0,"description":"Offset returned by the previous page"},
                "limit":{"type":"integer","minimum":1,"maximum":200,"default":50}
            },
            "additionalProperties":false
        });
        if require_query {
            schema["required"] = json!(["query"]);
        }
        schema
    }

    fn contacts_output_schema() -> Value {
        Self::object_output(
            json!({
                "contacts":{"type":"array","items":{"type":"object"}},
                "nextCursor":{"type":["integer","null"]},
                "totalMatched":{"type":"integer"}
            }),
            &["contacts", "totalMatched"],
        )
    }

    fn list_contacts_tool() -> Tool {
        Self::tool(
            "list_contacts",
            "List WhatsApp conversations",
            "List recent WhatsApp contacts and groups with last activity, preview, unread count, pin state, and pagination.",
            Self::contacts_input_schema(false),
            Self::contacts_output_schema(),
            Self::read_annotations("List WhatsApp conversations"),
        )
    }

    fn search_contacts_tool() -> Tool {
        Self::tool(
            "search_contacts",
            "Search WhatsApp contacts",
            "Resolve a person or group by case-insensitive name, phone number, or WhatsApp JID before reading or preparing a message.",
            Self::contacts_input_schema(true),
            Self::contacts_output_schema(),
            Self::read_annotations("Search WhatsApp contacts"),
        )
    }

    fn read_messages_tool() -> Tool {
        Self::tool(
            "read_messages",
            "Read WhatsApp messages",
            "Read a chronological page of messages with reply context, delivery state, translated text, and an older-page cursor.",
            json!({
                "type":"object",
                "properties":{
                    "contact_id":{"type":"string"},
                    "limit":{"type":"integer","minimum":1,"maximum":200,"default":50},
                    "before_timestamp":{"type":"integer","description":"Unix timestamp in milliseconds"},
                    "before_message_id":{"type":"string"},
                    "direction":{"type":"string","enum":["all","incoming","outgoing"],"default":"all"},
                    "content_type":{"type":"string","description":"Optional case-insensitive content type filter"}
                },
                "required":["contact_id"],
                "additionalProperties":false
            }),
            Self::object_output(
                json!({
                    "contact":{"type":"object"},
                    "messages":{"type":"array","items":{"type":"object"}},
                    "nextCursor":{"type":["object","null"]},
                    "hasMore":{"type":"boolean"}
                }),
                &["contact", "messages", "hasMore"],
            ),
            Self::read_annotations("Read WhatsApp messages"),
        )
    }

    fn feed_object_schema(properties: Value) -> Value {
        let required: Vec<&str> = properties
            .as_object()
            .expect("feed schema properties")
            .keys()
            .map(String::as_str)
            .collect();
        json!({"type":"object", "required":required,
            "properties":properties, "additionalProperties":false})
    }

    fn read_unified_messages_tool() -> Tool {
        let nullable_string = json!({"type":["string","null"]});
        let quote = Self::feed_object_schema(json!({
            "messageId":nullable_string, "senderName":nullable_string, "text":nullable_string
        }));
        let message = Self::feed_object_schema(json!({
            "id":{"type":"string"}, "contactId":{"type":"string"},
            "contactName":nullable_string, "contactPhone":nullable_string,
            "chatType":{"type":"string"}, "timestamp":{"type":"integer"},
            "timestampIso":nullable_string, "isFromMe":{"type":"boolean"},
            "senderName":nullable_string, "senderPhone":nullable_string,
            "text":nullable_string, "originalText":nullable_string,
            "translatedText":nullable_string, "isTranslated":{"type":"boolean"},
            "sourceLanguage":nullable_string, "contentType":{"type":"string"},
            "deliveryStatus":nullable_string,
            "replyContext":{"anyOf":[{"type":"null"},quote]}
        }));
        let cursor = Self::feed_object_schema(json!({
            "beforeTimestamp":{"type":"integer"}, "beforeMessageId":{"type":"string"}
        }));
        let mut tool = Self::tool(
            "read_unified_messages",
            "Read unified WhatsApp messages",
            "Read the latest stored messages across conversations, with source chat identity and stable older-page cursors. Returns saved translations only; never queues translation, marks messages read or sends anything.",
            json!({
                "type":"object",
                "properties":{
                    "limit":{"type":"integer","minimum":1,"maximum":FEED_MAX_LIMIT,"default":DEFAULT_LIMIT},
                    "before_timestamp":{"type":"integer","minimum":0,"maximum":JS_SAFE_INTEGER,"description":"Unix milliseconds; supply with before_message_id"},
                    "before_message_id":{"type":"string","minLength":1,"description":"Opaque oldest-retained message ID; supply with before_timestamp"}
                },
                "dependentRequired":{
                    "before_timestamp":["before_message_id"],
                    "before_message_id":["before_timestamp"]
                },
                "additionalProperties":false
            }),
            Self::feed_object_schema(json!({
                "messages":{"type":"array","items":message},
                "hasMore":{"type":"boolean"},
                "nextCursor":{"anyOf":[{"type":"null"},cursor]}
            })),
            Self::read_annotations("Read unified WhatsApp messages").open_world(false),
        );
        tool.meta = Some(
            serde_json::from_value(json!({"ui":{"visibility":["app"]}}))
                .expect("static feed metadata"),
        );
        tool
    }

    fn search_messages_tool() -> Tool {
        Self::tool(
            "search_messages",
            "Search WhatsApp messages",
            "Search stored message text across all conversations or within one contact, optionally constrained by time and direction.",
            json!({
                "type":"object",
                "properties":{
                    "query":{"type":"string"},
                    "contact_id":{"type":"string"},
                    "cursor":{"type":"integer","minimum":0},
                    "after_timestamp":{"type":"integer"},
                    "before_timestamp":{"type":"integer"},
                    "direction":{"type":"string","enum":["all","incoming","outgoing"],"default":"all"},
                    "limit":{"type":"integer","minimum":1,"maximum":200,"default":50}
                },
                "required":["query"],
                "additionalProperties":false
            }),
            Self::object_output(
                json!({
                    "query":{"type":"string"},
                    "messages":{"type":"array","items":{"type":"object"}},
                    "truncated":{"type":"boolean"}
                }),
                &["query", "messages", "truncated"],
            ),
            Self::read_annotations("Search WhatsApp messages"),
        )
    }

    fn prepare_message_tool() -> Tool {
        Self::tool(
            "prepare_message",
            "Prepare a WhatsApp message",
            "Resolve the recipient and final translated text without sending. Returns a short-lived token required by send_message or reply_to_message. Translation failures never fall back to English.",
            json!({
                "type":"object",
                "properties":{
                    "contact_id":{"type":"string"},
                    "text":{"type":"string","minLength":1},
                    "translation_mode":{"type":"string","enum":["auto","never","required"],"default":"auto"},
                    "target_language":{"type":"string"},
                    "reply_to_message_id":{"type":"string"}
                },
                "required":["contact_id","text"],
                "additionalProperties":false
            }),
            Self::object_output(
                json!({
                    "preparationToken":{"type":"string"},
                    "expiresAt":{"type":"integer"},
                    "recipient":{"type":"object"},
                    "originalText":{"type":"string"},
                    "finalText":{"type":"string"},
                    "translated":{"type":"boolean"},
                    "targetLanguage":{"type":["string","null"]},
                    "replyToMessageId":{"type":["string","null"]}
                }),
                &["preparationToken", "expiresAt", "recipient", "originalText", "finalText", "translated"],
            ),
            Self::write_annotations("Prepare a WhatsApp message", false),
        )
    }

    fn send_message_tool() -> Tool {
        Self::prepared_send_tool(
            "send_message",
            "Send prepared WhatsApp message",
            "Send an exact message previously returned by prepare_message. Retries with the same idempotency key never send twice.",
        )
    }

    fn reply_to_message_tool() -> Tool {
        Self::prepared_send_tool(
            "reply_to_message",
            "Send prepared WhatsApp reply",
            "Send a prepared reply. The preparation must include reply_to_message_id, fixing both reply target and final text before sending.",
        )
    }

    fn prepared_send_tool(
        name: &'static str,
        title: &'static str,
        description: &'static str,
    ) -> Tool {
        Self::tool(
            name,
            title,
            description,
            json!({
                "type":"object",
                "properties":{
                    "preparation_token":{"type":"string"},
                    "idempotency_key":{"type":"string","minLength":8,"maxLength":200}
                },
                "required":["preparation_token","idempotency_key"],
                "additionalProperties":false
            }),
            Self::object_output(
                json!({
                    "success":{"type":"boolean"},
                    "messageId":{"type":"string"},
                    "timestamp":{"type":"integer"},
                    "recipient":{"type":"object"},
                    "sentText":{"type":"string"},
                    "translated":{"type":"boolean"},
                    "targetLanguage":{"type":["string","null"]},
                    "replyToMessageId":{"type":["string","null"]}
                }),
                &[
                    "success",
                    "messageId",
                    "timestamp",
                    "recipient",
                    "sentText",
                    "translated",
                ],
            ),
            Self::write_annotations(title, false),
        )
    }

    fn react_to_message_tool() -> Tool {
        Self::tool(
            "react_to_message",
            "React to WhatsApp message",
            "Add, replace, or remove a reaction on a known message. Use an empty emoji to remove your reaction.",
            json!({
                "type":"object",
                "properties":{
                    "contact_id":{"type":"string"},
                    "message_id":{"type":"string"},
                    "emoji":{"type":"string","maxLength":16},
                    "sender_jid":{"type":"string"},
                    "idempotency_key":{"type":"string","minLength":8,"maxLength":200}
                },
                "required":["contact_id","message_id","emoji","idempotency_key"],
                "additionalProperties":false
            }),
            Self::object_output(
                json!({"success":{"type":"boolean"},"messageId":{"type":"string"},"emoji":{"type":"string"}}),
                &["success", "messageId", "emoji"],
            ),
            Self::write_annotations("React to WhatsApp message", false),
        )
    }

    fn mark_conversation_read_tool() -> Tool {
        Self::tool(
            "mark_conversation_read",
            "Mark WhatsApp conversation read",
            "Clear the local unread count and optionally send a WhatsApp read receipt for a specific stored message.",
            json!({
                "type":"object",
                "properties":{
                    "contact_id":{"type":"string"},
                    "message_id":{"type":"string"}
                },
                "required":["contact_id"],
                "additionalProperties":false
            }),
            Self::object_output(
                json!({"success":{"type":"boolean"},"contactId":{"type":"string"},"receiptSent":{"type":"boolean"},"receiptQueued":{"type":"boolean"}}),
                &["success", "contactId", "receiptSent", "receiptQueued"],
            ),
            Self::write_annotations("Mark WhatsApp conversation read", true),
        )
    }

    fn opener_tool(single_chat: bool) -> Tool {
        let mut tool = Self::tool(
            if single_chat { "open_chat" } else { "open_chats" },
            if single_chat { "Open WhatsApp chat" } else { "WhatsApp conversations" },
            "Open authorised WhatsApp conversations beside ChatGPT. This only reads stored data; opening never sends messages or read receipts.",
            if single_chat {
                json!({"type":"object","properties":{"contact_id":{"type":"string"}},"required":["contact_id"],"additionalProperties":false})
            } else { json!({"type":"object","properties":{},"additionalProperties":false}) },
            Self::object_output(json!({"contacts":{"type":"array"},"contact":{"type":"object"},"messages":{"type":"array"},"permissions":{"type":"object"}}), &["permissions"]),
            Self::read_annotations("Open WhatsApp conversations").open_world(false),
        );
        let mut metadata =
            json!({"ui":{"resourceUri":CHAT_RESOURCE_URI,"visibility":["model","app"]}});
        if !single_chat {
            metadata["openai/ui"] = json!({"entrypoints":[{"type":"global"},{"type":"thread"}]});
        }
        tool.meta = Some(serde_json::from_value(metadata).expect("static tool metadata"));
        tool.icons = Some(vec![Icon {
            src: format!("data:image/svg+xml,{}", urlencoding::encode("<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 20 20' fill='none' stroke='currentColor' stroke-width='1.33'><path d='M4 3h12a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H8l-4 3v-3a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2Z'/><path d='M6 7h8M6 11h5'/></svg>")),
            mime_type: Some("image/svg+xml".into()), sizes: Some(vec!["any".into()]),
        }]);
        tool
    }

    async fn handle_open(
        &self,
        args: Value,
        single_chat: bool,
    ) -> Result<CallToolResult, McpError> {
        self.require_read()?;
        let mut result = if single_chat {
            self.handle_read_messages(args).await?
        } else {
            self.handle_contacts(json!({}), false).await?
        };
        if let Some(payload) = result.structured_content.as_mut() {
            payload["permissions"] =
                json!({"read":self.permissions.read,"send":self.permissions.send});
        }
        Ok(result)
    }

    fn chat_resources(&self) -> ListResourcesResult {
        let resources = if self.permissions.read {
            let mut resource = RawResource::new(CHAT_RESOURCE_URI, "WhatsApp conversations");
            resource.mime_type = Some(CHAT_MIME_TYPE.into());
            resource.title = Some("WhatsApp conversations".into());
            vec![rmcp::model::Annotated::new(resource, None)]
        } else {
            vec![]
        };
        ListResourcesResult {
            resources,
            ..Default::default()
        }
    }

    fn chat_resource(&self, uri: &str) -> Result<ReadResourceResult, McpError> {
        self.require_read()?;
        if uri != CHAT_RESOURCE_URI {
            return Err(McpError::invalid_params("Unknown UI resource", None));
        }
        Ok(ReadResourceResult { contents: vec![ResourceContents::TextResourceContents {
            uri: CHAT_RESOURCE_URI.into(), mime_type: Some(CHAT_MIME_TYPE.into()),
            text: include_str!("../web/extensions/dist/chats.html").into(),
            meta: Some(serde_json::from_value(json!({
                "openai/ui":{"preferredDisplayMode":"fullscreen","availableDisplayModes":["inline","fullscreen"]},
                "ui":{"csp":{"connectDomains":[],"resourceDomains":[]}}
            })).expect("static resource metadata")),
        }] })
    }

    fn translate_message_tool() -> Tool {
        Self::tool("translate_message", "Translate incoming WhatsApp message",
            "Translate a stored incoming message using this conversation's opt-in settings. Saves the translation locally and may incur AI usage; sends nothing to WhatsApp. Requires read and send scopes.",
            json!({"type":"object","properties":{"contact_id":{"type":"string"},"message_id":{"type":"string"}},"required":["contact_id","message_id"],"additionalProperties":false}),
            Self::object_output(json!({"success":{"type":"boolean"},"translatedText":{"type":["string","null"]},"sourceLanguage":{"type":"string"}}), &["success"]),
            Self::write_annotations("Translate incoming WhatsApp message", false))
    }

    async fn handle_translate_message(&self, args: Value) -> Result<CallToolResult, McpError> {
        use axum::response::IntoResponse;
        self.require_read()?;
        self.require_send()?;
        let contact_id =
            crate::identity::canonical_chat_id(required_string(&args, "contact_id")?).into_owned();
        let message_id = required_string(&args, "message_id")?.to_string();
        let message = self
            .state
            .store
            .get_message_by_id(&message_id)
            .mcp()?
            .filter(|message| message.contact_id == contact_id && !message.is_from_me)
            .ok_or_else(|| {
                McpError::invalid_params("Incoming message not found in this conversation", None)
            })?;
        let response = crate::web::translate_message(
            axum::extract::State(self.state.clone()),
            axum::Json(crate::web::TranslateMessageRequest {
                text: message.original_text.unwrap_or_default(),
                message_id,
                contact_id,
            }),
        )
        .await
        .into_response();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .map_err(|error| McpError::internal_error(error.to_string(), None))?;
        let payload: Value = serde_json::from_slice(&bytes)
            .map_err(|error| McpError::internal_error(error.to_string(), None))?;
        if !status.is_success() || payload["success"] == false {
            return Err(McpError::internal_error(
                payload["error"]
                    .as_str()
                    .unwrap_or("Translation failed")
                    .to_string(),
                None,
            ));
        }
        Ok(CallToolResult::structured(payload))
    }

    fn require_read(&self) -> Result<(), McpError> {
        if self.permissions.read {
            Ok(())
        } else {
            Err(McpError::invalid_params(
                "This OAuth token does not include whatsapp.read",
                None,
            ))
        }
    }

    fn require_send(&self) -> Result<(), McpError> {
        if self.permissions.send {
            Ok(())
        } else {
            Err(McpError::invalid_params(
                "This OAuth token does not include whatsapp.send",
                None,
            ))
        }
    }

    async fn handle_get_status(&self) -> Result<CallToolResult, McpError> {
        self.require_read()?;
        let (message_count, contact_count) = self
            .state
            .store
            .get_stats()
            .map_err(|error| McpError::internal_error(error.to_string(), None))?;
        Ok(CallToolResult::structured(json!({
            "connected": *self.state.connected.read().await,
            "connectionState": self.state.connection_state.read().await.label(),
            "accountName": self.state.name.read().await.clone(),
            "accountPhone": self.state.phone.read().await.clone(),
            "translationConfigured": self.state.translator().is_some(),
            "contactCount": contact_count,
            "messageCount": message_count,
            "checkedAt": chrono::Utc::now().to_rfc3339(),
        })))
    }

    async fn handle_contacts(
        &self,
        args: Value,
        require_query: bool,
    ) -> Result<CallToolResult, McpError> {
        self.require_read()?;
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_lowercase();
        if require_query && query.is_empty() {
            return Err(McpError::invalid_params("query is required", None));
        }
        let contact_type = args
            .get("contact_type")
            .and_then(Value::as_str)
            .unwrap_or("all");
        let unread_only = args
            .get("unread_only")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let cursor = args.get("cursor").and_then(Value::as_u64).unwrap_or(0) as usize;
        let limit = argument_limit(&args);

        let contacts = self
            .state
            .store
            .get_contacts()
            .map_err(|error| McpError::internal_error(error.to_string(), None))?;
        let filtered: Vec<StoredContact> = contacts
            .into_iter()
            .filter(|contact| {
                (contact_type == "all" || contact.contact_type.as_deref() == Some(contact_type))
                    && (!unread_only || contact.unread_count > 0)
                    && (query.is_empty()
                        || contact.id.to_lowercase().contains(&query)
                        || contact
                            .name
                            .as_deref()
                            .is_some_and(|value| value.to_lowercase().contains(&query))
                        || contact
                            .phone
                            .as_deref()
                            .is_some_and(|value| value.to_lowercase().contains(&query)))
            })
            .collect();
        let total_matched = filtered.len();
        let contacts: Vec<ContactInfo> = filtered
            .into_iter()
            .skip(cursor)
            .take(limit)
            .map(ContactInfo::from)
            .collect();
        let next_cursor =
            (cursor + contacts.len() < total_matched).then_some(cursor + contacts.len());

        Ok(CallToolResult::structured(json!({
            "contacts": contacts,
            "nextCursor": next_cursor,
            "totalMatched": total_matched,
        })))
    }

    async fn handle_read_unified_messages(&self, args: Value) -> Result<CallToolResult, McpError> {
        self.require_read()?;
        let args = FeedArguments::parse(&args)?;
        // Call the read-only query directly: the REST feed handler queues AI
        // work. A feed view must not change messages, jobs or read state.
        let mut messages = self
            .state
            .store
            .get_unified_messages(
                args.limit + 1,
                args.before_timestamp,
                args.before_message_id.as_deref(),
            )
            .mcp()?;
        let has_more = messages.len() > args.limit as usize;
        if has_more {
            messages.remove(0);
        }
        let next_cursor = messages
            .first()
            .filter(|_| has_more)
            .map(|message| FeedCursor {
                before_timestamp: message.timestamp,
                before_message_id: message.id.clone(),
            });
        let page = FeedPage {
            messages: messages.into_iter().map(FeedMessage::from).collect(),
            has_more,
            next_cursor,
        };
        Ok(CallToolResult::structured(
            serde_json::to_value(page)
                .map_err(|error| McpError::internal_error(error.to_string(), None))?,
        ))
    }

    async fn handle_read_messages(&self, args: Value) -> Result<CallToolResult, McpError> {
        self.require_read()?;
        let contact_id = crate::identity::canonical_chat_id(required_string(&args, "contact_id")?);
        let contact_id = contact_id.as_ref();
        let contact = self
            .state
            .store
            .get_contact(contact_id)
            .mcp()?
            .ok_or_else(|| McpError::invalid_params("Unknown contact_id", None))?;
        let limit = argument_limit(&args);
        let before_timestamp = args.get("before_timestamp").and_then(Value::as_i64);
        let before_message_id = args.get("before_message_id").and_then(Value::as_str);
        let direction = args
            .get("direction")
            .and_then(Value::as_str)
            .unwrap_or("all");
        let content_type = args
            .get("content_type")
            .and_then(Value::as_str)
            .map(str::to_lowercase);
        let mut messages: Vec<StoredMessage> = Vec::new();
        let mut scan_timestamp = before_timestamp;
        let mut scan_id = before_message_id.map(str::to_string);
        loop {
            let batch = self
                .state
                .store
                .get_messages_paginated(
                    contact_id,
                    Some(MAX_LIMIT as u32),
                    scan_timestamp,
                    scan_id.as_deref(),
                    true,
                )
                .mcp()?;
            let exhausted = batch.len() < MAX_LIMIT;
            let cursor = batch
                .first()
                .map(|message| (message.timestamp, message.id.clone()));
            for message in batch.into_iter().rev() {
                let direction_matches = match direction {
                    "incoming" => !message.is_from_me,
                    "outgoing" => message.is_from_me,
                    _ => true,
                };
                if direction_matches
                    && content_type
                        .as_ref()
                        .is_none_or(|value| message.content_type.to_lowercase() == *value)
                {
                    messages.push(message);
                    if messages.len() > limit {
                        break;
                    }
                }
            }
            if exhausted || messages.len() > limit {
                break;
            }
            let Some((timestamp, id)) = cursor else {
                break;
            };
            scan_timestamp = Some(timestamp);
            scan_id = Some(id);
        }
        messages.reverse();
        let has_more = messages.len() > limit;
        if has_more {
            let remove_count = messages.len() - limit;
            messages.drain(0..remove_count);
        }
        let next_cursor = if has_more {
            messages.first().map(
                |message| json!({"beforeTimestamp":message.timestamp,"beforeMessageId":message.id}),
            )
        } else {
            None
        };
        let messages: Vec<MessageInfo> = messages.into_iter().map(MessageInfo::from).collect();

        Ok(CallToolResult::structured(json!({
            "contact": ContactInfo::from(contact),
            "messages": messages,
            "nextCursor": next_cursor,
            "hasMore": has_more,
        })))
    }

    async fn handle_search_messages(&self, args: Value) -> Result<CallToolResult, McpError> {
        self.require_read()?;
        let query = required_string(&args, "query")?.trim().to_lowercase();
        if query.is_empty() {
            return Err(McpError::invalid_params("query must not be empty", None));
        }
        let contact_filter = args.get("contact_id").and_then(Value::as_str);
        let after = args.get("after_timestamp").and_then(Value::as_i64);
        let before = args.get("before_timestamp").and_then(Value::as_i64);
        let direction = args
            .get("direction")
            .and_then(Value::as_str)
            .unwrap_or("all");
        let limit = argument_limit(&args);
        let contacts = if let Some(contact_id) = contact_filter {
            vec![self
                .state
                .store
                .get_contact(contact_id)
                .mcp()?
                .ok_or_else(|| McpError::invalid_params("Unknown contact_id", None))?]
        } else {
            self.state.store.get_contacts().mcp()?
        };

        let mut matches = Vec::new();
        for contact in contacts {
            for message in self.state.store.get_messages(&contact.id).mcp()? {
                if after.is_some_and(|value| message.timestamp < value)
                    || before.is_some_and(|value| message.timestamp >= value)
                    || (direction == "incoming" && message.is_from_me)
                    || (direction == "outgoing" && !message.is_from_me)
                {
                    continue;
                }
                let searchable = format!(
                    "{} {} {} {}",
                    message.original_text.as_deref().unwrap_or_default(),
                    message.translated_text.as_deref().unwrap_or_default(),
                    message
                        .content
                        .as_ref()
                        .and_then(|content| content.get("body"))
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                    message.sender_name.as_deref().unwrap_or_default()
                )
                .to_lowercase();
                if searchable.contains(&query) {
                    matches.push(message);
                }
            }
        }
        matches.sort_by(|left, right| {
            right
                .timestamp
                .cmp(&left.timestamp)
                .then_with(|| right.id.cmp(&left.id))
        });
        let cursor = args.get("cursor").and_then(Value::as_u64).unwrap_or(0) as usize;
        let total_matched = matches.len();
        let truncated = total_matched.saturating_sub(cursor) > limit;
        let matches = matches.into_iter().skip(cursor).take(limit);
        let messages: Vec<MessageInfo> = matches.into_iter().map(MessageInfo::from).collect();
        Ok(CallToolResult::structured(json!({
            "query": query,
            "messages": messages,
            "truncated": truncated,
            "nextCursor": truncated.then_some(cursor.saturating_add(limit)),
            "totalMatched": total_matched,
        })))
    }

    async fn handle_prepare_message(&self, args: Value) -> Result<CallToolResult, McpError> {
        self.require_read()?;
        let contact_id = crate::identity::canonical_chat_id(required_string(&args, "contact_id")?);
        let contact_id = contact_id.as_ref();
        let original_text = required_string(&args, "text")?.trim();
        if original_text.is_empty() {
            return Err(McpError::invalid_params("text must not be empty", None));
        }
        self.state
            .store
            .validate_reply_only(
                contact_id,
                args.get("reply_to_message_id").and_then(Value::as_str),
            )
            .mcp()?;
        let mode = TranslationMode::parse(args.get("translation_mode").and_then(Value::as_str))?;
        let contact = self
            .state
            .store
            .get_contact(contact_id)
            .mcp()?
            .ok_or_else(|| McpError::invalid_params("Unknown contact_id", None))?;
        let settings = self
            .state
            .store
            .get_conversation_settings(contact_id)
            .mcp()?;
        let target_language = if mode == TranslationMode::Never || !settings.translation_enabled {
            None
        } else {
            if let Some(target) = args
                .get("target_language")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                Some(target.to_string())
            } else {
                crate::incoming::outgoing_language(
                    &self.state,
                    contact_id,
                    args.get("reply_to_message_id").and_then(Value::as_str),
                )
                .await
                .mcp()?
            }
        };

        let translation_result = if let Some(target) = target_language.as_deref() {
            let translator = self.state.translator().ok_or_else(|| {
                McpError::internal_error(
                    format!(
                        "Translation to {target} is needed, but translation is not configured. No message was sent or prepared."
                    ),
                    None,
                )
            })?;
            match translator.translate_to(original_text, target).await {
                Ok((translated, usage)) => {
                    if usage.input_tokens > 0 {
                        if let Err(error) = self.state.store.record_usage(
                            Some(contact_id),
                            None,
                            &usage,
                            "prepare_outgoing_mcp",
                        ) {
                            warn!("Failed to record MCP preparation usage: {error}");
                        }
                    }
                    Ok(translated)
                }
                Err(error) => Err(error.to_string()),
            }
        } else {
            Ok(original_text.to_string())
        };
        let (final_text, translated, target_language) = resolve_prepared_translation(
            original_text,
            target_language.as_deref(),
            mode,
            translation_result,
        )
        .map_err(|error| McpError::internal_error(error, None))?;

        let reply_to_message_id = args
            .get("reply_to_message_id")
            .and_then(Value::as_str)
            .map(str::to_string);
        let (reply_to_sender, reply_to_text) = if let Some(message_id) = &reply_to_message_id {
            let message = self
                .state
                .store
                .get_message_by_id(message_id)
                .mcp()?
                .ok_or_else(|| McpError::invalid_params("Unknown reply_to_message_id", None))?;
            if message.contact_id != contact_id {
                return Err(McpError::invalid_params(
                    "reply_to_message_id belongs to another conversation",
                    None,
                ));
            }
            let info = MessageInfo::from(message.clone());
            (message.sender_phone, info.text)
        } else {
            (None, None)
        };
        let now = chrono::Utc::now().timestamp_millis();
        let token = uuid::Uuid::new_v4().to_string();
        let prepared = PreparedMcpMessage {
            token: token.clone(),
            contact_id: contact_id.to_string(),
            contact_name: contact.name.clone(),
            original_text: original_text.to_string(),
            final_text: final_text.clone(),
            translated,
            target_language: target_language.clone(),
            translation_mode: mode.as_str().to_string(),
            reply_to_message_id: reply_to_message_id.clone(),
            reply_to_sender,
            reply_to_text,
            created_at: now,
            expires_at: now + PREPARATION_TTL_MILLIS,
        };
        let prepared_value = serde_json::to_value(&prepared)
            .map_err(|error| McpError::internal_error(error.to_string(), None))?;
        let mut preparations = self.state.mcp_prepared_messages.write().await;
        preparations.retain(|_, value| {
            value
                .get("expiresAt")
                .and_then(Value::as_i64)
                .is_some_and(|expires_at| expires_at > now)
        });
        preparations.insert(token.clone(), prepared_value);

        Ok(CallToolResult::structured(json!({
            "preparationToken": token,
            "expiresAt": prepared.expires_at,
            "recipient": {"id":contact.id,"name":contact.name,"phone":contact.phone,"type":contact.contact_type},
            "originalText": prepared.original_text,
            "finalText": final_text,
            "translated": translated,
            "targetLanguage": target_language,
            "translationMode": prepared.translation_mode,
            "replyToMessageId": reply_to_message_id,
        })))
    }

    async fn handle_prepared_send(
        &self,
        args: Value,
        require_reply: bool,
    ) -> Result<CallToolResult, McpError> {
        self.require_send()?;
        let preparation_token = required_string(&args, "preparation_token")?;
        let idempotency_key = required_string(&args, "idempotency_key")?;
        if idempotency_key.len() < 8 || idempotency_key.len() > 200 {
            return Err(McpError::invalid_params(
                "idempotency_key must contain 8 to 200 characters",
                None,
            ));
        }
        let operation_key = format!("send:{preparation_token}");
        if let Some(existing) = self
            .idempotency_result(idempotency_key, &operation_key)
            .await?
        {
            return Ok(existing);
        }
        let prepared_value = self
            .state
            .mcp_prepared_messages
            .read()
            .await
            .get(preparation_token)
            .cloned()
            .ok_or_else(|| {
                McpError::invalid_params(
                    "Unknown, expired, or already-used preparation_token",
                    None,
                )
            })?;
        let prepared: PreparedMcpMessage = serde_json::from_value(prepared_value)
            .map_err(|error| McpError::internal_error(error.to_string(), None))?;
        if prepared.expires_at <= chrono::Utc::now().timestamp_millis() {
            return Err(McpError::invalid_params(
                "preparation_token has expired",
                None,
            ));
        }
        if require_reply && prepared.reply_to_message_id.is_none() {
            return Err(McpError::invalid_params(
                "reply_to_message requires a preparation created with reply_to_message_id",
                None,
            ));
        }
        self.state
            .store
            .validate_reply_only(
                &prepared.contact_id,
                prepared.reply_to_message_id.as_deref(),
            )
            .mcp()?;
        if prepared.translated
            && !self
                .state
                .store
                .get_conversation_settings(&prepared.contact_id)
                .mcp()?
                .translation_enabled
        {
            return Err(McpError::invalid_params(
                "Translation is now off for this conversation. Prepare the message again before sending.",
                None,
            ));
        }
        if !self
            .begin_idempotent_operation(idempotency_key, &operation_key)
            .await
        {
            return self
                .idempotency_result(idempotency_key, &operation_key)
                .await?
                .ok_or_else(|| McpError::internal_error("Idempotency state changed", None));
        }
        self.state
            .mcp_prepared_messages
            .write()
            .await
            .remove(preparation_token);
        match self.send_prepared_message(&prepared).await {
            Ok(result) => {
                self.finish_idempotent_operation(idempotency_key, "completed", result.clone())
                    .await;
                Ok(CallToolResult::structured(result))
            }
            Err(error) => {
                let result = json!({"success":false,"error":error.to_string()});
                self.finish_idempotent_operation(idempotency_key, "failed", result)
                    .await;
                Err(error)
            }
        }
    }

    async fn send_prepared_message(
        &self,
        prepared: &PreparedMcpMessage,
    ) -> Result<Value, McpError> {
        if !*self.state.connected.read().await {
            return Err(McpError::internal_error("Not connected to WhatsApp", None));
        }
        let contact = self
            .state
            .store
            .get_contact(&prepared.contact_id)
            .mcp()?
            .ok_or_else(|| McpError::invalid_params("Recipient no longer exists", None))?;
        let request_id = self.state.next_request_id();
        let timestamp = chrono::Utc::now().timestamp_millis();
        let temp_message_id = format!("mcp_pending_{request_id}_{timestamp}");
        let reply_context = prepared.reply_to_message_id.as_ref().map(|message_id| {
            json!({
                "messageId": message_id,
                "senderName": prepared.reply_to_sender.clone().unwrap_or_else(|| "Unknown".to_string()),
                "text": prepared.reply_to_text.clone().unwrap_or_default(),
            })
        });
        let content = json!({
            "type":"text",
            "body":prepared.original_text,
            "reply_context":reply_context,
        });
        let stored_message = StoredMessage {
            id: temp_message_id.clone(),
            contact_id: prepared.contact_id.clone(),
            timestamp,
            is_from_me: true,
            is_forwarded: false,
            sender_name: self.state.name.read().await.clone(),
            sender_phone: self.state.phone.read().await.clone(),
            contact_name: contact.name.clone(),
            contact_phone: contact.phone.clone(),
            chat_type: contact
                .contact_type
                .clone()
                .unwrap_or_else(|| "private".to_string()),
            content_type: "Text".to_string(),
            content_json: content.to_string(),
            content: Some(content),
            original_text: prepared.translated.then(|| prepared.original_text.clone()),
            translated_text: prepared.translated.then(|| prepared.final_text.clone()),
            source_language: prepared.target_language.clone(),
            is_translated: prepared.translated,
            delivery_status: Some("sent".to_string()),
        };
        self.state
            .store
            .add_message(&stored_message)
            .map_err(|error| McpError::internal_error(error.to_string(), None))?;
        if let Err(error) = self.state.store.upsert_contact(
            &prepared.contact_id,
            contact.name.as_deref(),
            contact.phone.as_deref(),
            contact.contact_type.as_deref(),
            timestamp,
        ) {
            let _ = self.state.store.delete_message(&temp_message_id);
            return Err(McpError::internal_error(error.to_string(), None));
        }
        let receiver = self
            .state
            .register_pending_send(request_id, Some(temp_message_id.clone()))
            .await;
        let command = BridgeCommand::Send {
            request_id: Some(request_id),
            to: prepared.contact_id.clone(),
            text: prepared.final_text.clone(),
            reply_to: prepared.reply_to_message_id.clone(),
            reply_to_sender: prepared.reply_to_sender.clone(),
            reply_to_text: prepared.reply_to_text.clone(),
        };
        if let Err(error) = self.state.send_bridge_command(command).await {
            self.state.cancel_pending_send(request_id).await;
            let _ = self.state.store.delete_message(&temp_message_id);
            return Err(McpError::internal_error(
                format!("Failed to send message: {error}"),
                None,
            ));
        }
        let send_result = self
            .state
            .wait_for_send_result(request_id, receiver)
            .await
            .map_err(|error| match error {
                SendConfirmationError::Timeout => McpError::internal_error(
                    "Timed out waiting for WhatsApp confirmation; the idempotency key is locked to prevent a duplicate send",
                    None,
                ),
                SendConfirmationError::ChannelClosed => McpError::internal_error(
                    "WhatsApp confirmation channel closed; the idempotency key is locked to prevent a duplicate send",
                    None,
                ),
            })?;
        if !send_result.success {
            return Err(McpError::internal_error(
                send_result
                    .error
                    .unwrap_or_else(|| "WhatsApp rejected the message".to_string()),
                None,
            ));
        }
        let message_id = send_result.message_id.unwrap_or(temp_message_id);
        let timestamp = send_result.timestamp.unwrap_or(timestamp);
        info!("MCP sent prepared message");
        Ok(json!({
            "success":true,
            "messageId":message_id,
            "timestamp":timestamp,
            "timestampIso":timestamp_millis_to_iso(timestamp),
            "recipient":{"id":prepared.contact_id,"name":prepared.contact_name},
            "sentText":prepared.final_text,
            "translated":prepared.translated,
            "targetLanguage":prepared.target_language,
            "replyToMessageId":prepared.reply_to_message_id,
        }))
    }

    async fn handle_reaction(&self, args: Value) -> Result<CallToolResult, McpError> {
        self.require_send()?;
        let contact_id = crate::identity::canonical_chat_id(required_string(&args, "contact_id")?);
        let contact_id = contact_id.as_ref();
        let message_id = required_string(&args, "message_id")?;
        let emoji = args
            .get("emoji")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if emoji.chars().count() > 16 {
            return Err(McpError::invalid_params("emoji is too long", None));
        }
        let idempotency_key = required_string(&args, "idempotency_key")?;
        let operation_key = format!("reaction:{contact_id}:{message_id}:{emoji}");
        if let Some(existing) = self
            .idempotency_result(idempotency_key, &operation_key)
            .await?
        {
            return Ok(existing);
        }
        let message = self
            .state
            .store
            .get_message_by_id(message_id)
            .mcp()?
            .ok_or_else(|| McpError::invalid_params("Unknown message_id", None))?;
        if message.contact_id != contact_id {
            return Err(McpError::invalid_params(
                "message_id belongs to another conversation",
                None,
            ));
        }
        if !*self.state.connected.read().await {
            return Err(McpError::internal_error("Not connected to WhatsApp", None));
        }
        if !self
            .begin_idempotent_operation(idempotency_key, &operation_key)
            .await
        {
            return self
                .idempotency_result(idempotency_key, &operation_key)
                .await?
                .ok_or_else(|| McpError::internal_error("Idempotency state changed", None));
        }
        let request_id = self.state.next_request_id();
        let receiver = self.state.register_pending_send(request_id, None).await;
        let command = BridgeCommand::SendReaction {
            request_id: Some(request_id),
            to: contact_id.to_string(),
            message_id: message_id.to_string(),
            sender_jid: args
                .get("sender_jid")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or(message.sender_phone),
            emoji: emoji.to_string(),
        };
        let outcome = async {
            if let Err(error) = self.state.send_bridge_command(command).await {
                self.state.cancel_pending_send(request_id).await;
                return Err(McpError::internal_error(error, None));
            }
            let result = self
                .state
                .wait_for_send_result(request_id, receiver)
                .await
                .map_err(|error| match error {
                    SendConfirmationError::Timeout => McpError::internal_error(
                        "Timed out waiting for reaction confirmation",
                        None,
                    ),
                    SendConfirmationError::ChannelClosed => {
                        McpError::internal_error("Reaction confirmation channel closed", None)
                    }
                })?;
            if result.success {
                Ok(json!({"success":true,"messageId":message_id,"emoji":emoji}))
            } else {
                Err(McpError::internal_error(
                    result
                        .error
                        .unwrap_or_else(|| "WhatsApp rejected the reaction".to_string()),
                    None,
                ))
            }
        }
        .await;
        match outcome {
            Ok(result) => {
                self.finish_idempotent_operation(idempotency_key, "completed", result.clone())
                    .await;
                Ok(CallToolResult::structured(result))
            }
            Err(error) => {
                self.finish_idempotent_operation(
                    idempotency_key,
                    "failed",
                    json!({"success":false,"error":error.to_string()}),
                )
                .await;
                Err(error)
            }
        }
    }

    async fn handle_mark_read(&self, args: Value) -> Result<CallToolResult, McpError> {
        self.require_send()?;
        let contact_id = crate::identity::canonical_chat_id(required_string(&args, "contact_id")?);
        let contact_id = contact_id.as_ref();
        self.state
            .store
            .get_contact(contact_id)
            .mcp()?
            .ok_or_else(|| McpError::invalid_params("Unknown contact_id", None))?;
        let mut receipt_sent = false;
        if let Some(id) = args.get("message_id").and_then(Value::as_str) {
            self.state
                .store
                .record_message_views(contact_id, &[id.to_owned()], chrono::Utc::now().timestamp())
                .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
            receipt_sent = true;
        } else {
            self.state.store.mark_as_read(contact_id).mcp()?;
        }
        Ok(CallToolResult::structured(json!({
            "success":true,
            "contactId":contact_id,
            "receiptSent":false,
            "receiptQueued":receipt_sent,
        })))
    }

    async fn idempotency_result(
        &self,
        idempotency_key: &str,
        operation_key: &str,
    ) -> Result<Option<CallToolResult>, McpError> {
        let Some(value) = self.state.store.mcp_send_record(idempotency_key).mcp()? else {
            return Ok(None);
        };
        let record: McpIdempotencyRecord = serde_json::from_value(value)
            .map_err(|error| McpError::internal_error(error.to_string(), None))?;
        if record.operation_key != operation_key {
            return Err(McpError::invalid_params(
                "idempotency_key was already used for a different operation",
                None,
            ));
        }
        match record.status.as_str() {
            "completed" => Ok(record.result.map(CallToolResult::structured)),
            "failed" => Err(McpError::internal_error(
                record
                    .result
                    .as_ref()
                    .and_then(|value| value.get("error"))
                    .and_then(Value::as_str)
                    .unwrap_or("The previous attempt failed; use a new preparation and idempotency key")
                    .to_string(),
                None,
            )),
            _ => Err(McpError::internal_error(
                "An operation with this idempotency_key is already in progress; it will not be sent again",
                None,
            )),
        }
    }

    async fn begin_idempotent_operation(&self, idempotency_key: &str, operation_key: &str) -> bool {
        let record = McpIdempotencyRecord {
            operation_key: operation_key.to_string(),
            status: "pending".to_string(),
            result: None,
            expires_at: chrono::Utc::now().timestamp_millis() + 24 * 60 * 60 * 1000,
        };
        self.state
            .store
            .claim_mcp_send(
                idempotency_key,
                &serde_json::to_value(record).expect("serialize idempotency record"),
            )
            .unwrap_or(false)
    }

    async fn finish_idempotent_operation(
        &self,
        idempotency_key: &str,
        status: &str,
        result: Value,
    ) {
        if let Ok(Some(value)) = self.state.store.mcp_send_record(idempotency_key) {
            if let Ok(mut record) = serde_json::from_value::<McpIdempotencyRecord>(value) {
                record.status = status.to_string();
                record.result = Some(result);
                if let Err(error) = self.state.store.update_mcp_send(
                    idempotency_key,
                    &serde_json::to_value(record).expect("serialize idempotency record"),
                ) {
                    tracing::error!("Cannot persist MCP send outcome: {error}");
                }
            }
        }
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

impl ServerHandler for WhatsAppMcpServer {
    async fn initialize(
        &self,
        request: InitializeRequestParam,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, McpError> {
        let started = Instant::now();
        if context.peer.peer_info().is_none() {
            context.peer.set_peer_info(request);
        }
        crate::log_privacy::log_mcp_call("initialize", None, "ok", None, 0, started.elapsed());
        Ok(self.get_info())
    }

    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: Default::default(),
            capabilities: ServerCapabilities::builder().enable_tools().enable_resources().build(),
            server_info: Implementation {
                name: "whatsapp-translator".to_string(),
                title: Some("WhatsApp Translator MCP Server".to_string()),
                version: env!("CARGO_PKG_VERSION").to_string(),
                icons: None,
                website_url: None,
            },
            instructions: Some(
                "Use search_contacts before selecting a recipient. Read context with read_messages. Before any text send, call prepare_message and show the resolved recipient and finalText to the user; only then call send_message or reply_to_message with a unique idempotency key. Translation failures never fall back to the English source. Treat react_to_message and mark_conversation_read as external writes."
                    .to_string(),
            ),
        }
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        let started = Instant::now();
        let result = self.chat_resources();
        crate::log_privacy::log_mcp_call(
            "resources/list",
            None,
            "ok",
            None,
            result.resources.len(),
            started.elapsed(),
        );
        Ok(result)
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        let started = Instant::now();
        let result = self.chat_resource(&request.uri);
        let (status, error_code, count) = match &result {
            Ok(value) => ("ok", None, value.contents.len()),
            Err(error) => ("error", Some(error.code.0), 0),
        };
        crate::log_privacy::log_mcp_call(
            "resources/read",
            None,
            status,
            error_code,
            count,
            started.elapsed(),
        );
        result
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let started = Instant::now();
        let result = ListToolsResult::with_all_items(self.advertised_tools());
        crate::log_privacy::log_mcp_call(
            "tools/list",
            None,
            "ok",
            None,
            result.tools.len(),
            started.elapsed(),
        );
        Ok(result)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParam,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let started = Instant::now();
        let tool = request.name.as_ref().to_string();
        let result = self.dispatch_tool(request, context).await;
        let (status, error_code, count) = match &result {
            Ok(value) if value.is_error == Some(true) => ("error", None, tool_result_count(value)),
            Ok(value) => ("ok", None, tool_result_count(value)),
            Err(error) => ("error", Some(error.code.0), 0),
        };
        crate::log_privacy::log_mcp_call(
            "tools/call",
            Some(&tool),
            status,
            error_code,
            count,
            started.elapsed(),
        );
        result
    }
}

impl WhatsAppMcpServer {
    async fn dispatch_tool(
        &self,
        request: CallToolRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let args = request
            .arguments
            .map(Value::Object)
            .unwrap_or(Value::Object(Default::default()));
        match request.name.as_ref() {
            "open_chats" => self.handle_open(args, false).await,
            "open_chat" => self.handle_open(args, true).await,
            "translate_message" => self.handle_translate_message(args).await,
            "get_status" => self.handle_get_status().await,
            "list_contacts" => self.handle_contacts(args, false).await,
            "search_contacts" => self.handle_contacts(args, true).await,
            "read_messages" => self.handle_read_messages(args).await,
            "read_unified_messages" => self.handle_read_unified_messages(args).await,
            "search_messages" => self.handle_search_messages(args).await,
            "prepare_message" => self.handle_prepare_message(args).await,
            "send_message" => self.handle_prepared_send(args, false).await,
            "reply_to_message" => self.handle_prepared_send(args, true).await,
            "react_to_message" => self.handle_reaction(args).await,
            "mark_conversation_read" => self.handle_mark_read(args).await,
            _ => Err(McpError::invalid_params(
                format!("Unknown tool: {}", request.name),
                None,
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{ConversationSettings, MessageStore};

    fn test_state() -> (Arc<AppState>, std::path::PathBuf) {
        let data_dir = std::env::temp_dir().join(format!(
            "whatsapp-translator-mcp-test-{}",
            uuid::Uuid::new_v4()
        ));
        let store = MessageStore::new(&data_dir).expect("test store");
        let state = AppState::new(
            store,
            std::env::current_dir().expect("cwd").join("web/public"),
            data_dir.clone(),
            None,
            None,
            None,
        );
        (state, data_dir)
    }

    #[tokio::test]
    async fn get_status_reports_reconnecting_and_a_stopped_connection() {
        let (state, data_dir) = test_state();
        let server = WhatsAppMcpServer::new(
            state.clone(),
            McpPermissions {
                read: true,
                send: false,
            },
        );
        state.set_connected(false, None, None).await;
        let retrying = server
            .handle_get_status()
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert_eq!(retrying["connected"], false);
        assert_eq!(retrying["connectionState"], "reconnecting");
        state.mark_whatsapp_disconnected().await;
        let stopped = server
            .handle_get_status()
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert_eq!(stopped["connected"], false);
        assert_eq!(stopped["connectionState"], "disconnected");
        let _ = std::fs::remove_dir_all(data_dir);
    }

    struct FeedFixture {
        state: Arc<AppState>,
        dir: std::path::PathBuf,
    }

    impl FeedFixture {
        fn new() -> Self {
            let (state, dir) = test_state();
            Self { state, dir }
        }
    }

    impl Drop for FeedFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn seed_feed(state: &AppState) {
        let private = "447700900123@s.whatsapp.net";
        let group = "synthetic-group@g.us";
        state
            .store
            .upsert_contact(
                private,
                Some("Fixture Friend"),
                Some("447700900123"),
                Some("private"),
                0,
            )
            .unwrap();
        state
            .store
            .upsert_contact(group, Some("Fixture Group"), None, Some("group"), 0)
            .unwrap();
        // The feed sources are beyond the initial contacts page.
        for index in 0..60 {
            state
                .store
                .upsert_contact(
                    &format!("other-{index}@example.test"),
                    Some("Other"),
                    None,
                    Some("private"),
                    1000 + index,
                )
                .unwrap();
        }
        let mut a = extension_message("a", "447700900123:22@s.whatsapp.net", 100, true);
        a.original_text = None;
        a.translated_text = None;
        a.source_language = None;
        a.is_translated = false;
        state.store.add_message(&a).unwrap();
        let mut b = extension_message("b", group, 100, true);
        b.chat_type = "group".into();
        b.sender_name = Some("Fixture Member".into());
        b.sender_phone = Some("900000000001@lid".into());
        b.content_type = "Image".into();
        b.content_json = json!({"type":"image", "caption":"Regarde", "media_data":"SYNTHETIC_MEDIA_BYTES",
            "reply_context":{"messageId":"outside-history", "senderName":"Quoted Member", "text":"Earlier quote", "unexpected":"must not leak"}}).to_string();
        b.original_text = Some("Regarde".into());
        b.translated_text = Some("Look".into());
        b.source_language = Some("French".into());
        state.store.add_message(&b).unwrap();
        let mut c = extension_message("c", private, 200, false);
        c.sender_name = Some("Me".into());
        c.sender_phone = Some("447700900000".into());
        c.original_text = Some("See you".into());
        c.translated_text = Some("À bientôt".into());
        c.source_language = Some("French".into());
        c.is_translated = true;
        c.delivery_status = Some("uncertain".into());
        state.store.add_message(&c).unwrap();
        state
            .store
            .upsert_contact("status@broadcast", None, None, Some("status"), 0)
            .unwrap();
        for (id, chat, content_type, content) in [
            (
                "status",
                "status@broadcast",
                "text",
                json!({"type":"text","body":"Status"}),
            ),
            (
                "reaction-label",
                private,
                "Reaction",
                json!({"type":"text","body":"Reaction label"}),
            ),
            (
                "reaction-payload",
                group,
                "text",
                json!({"type":"reaction","emoji":"👍"}),
            ),
        ] {
            let mut excluded = extension_message(id, chat, 300, true);
            excluded.content_type = content_type.into();
            excluded.content_json = content.to_string();
            state.store.add_message(&excluded).unwrap();
        }
    }

    #[test]
    fn unified_feed_schema_is_complete_app_only_and_read_scoped() {
        let fixture = FeedFixture::new();
        for scope in [
            "",
            "whatsapp.send",
            "whatsapp.read",
            "whatsapp.read whatsapp.send",
            "mcp",
        ] {
            let server =
                WhatsAppMcpServer::new(fixture.state.clone(), McpPermissions::from_scope(scope));
            let tools = server.advertised_tools();
            let feed = tools
                .iter()
                .find(|tool| tool.name == "read_unified_messages");
            assert_eq!(feed.is_some(), server.permissions.read, "{scope}");
        }
        let tool = serde_json::to_value(WhatsAppMcpServer::read_unified_messages_tool()).unwrap();
        assert_eq!(tool["_meta"]["ui"]["visibility"], json!(["app"]));
        assert_eq!(tool["annotations"]["readOnlyHint"], true);
        assert_eq!(tool["annotations"]["destructiveHint"], false);
        assert_eq!(tool["annotations"]["idempotentHint"], true);
        assert_eq!(tool["annotations"]["openWorldHint"], false);
        let input = &tool["inputSchema"];
        assert_eq!(input["additionalProperties"], false);
        assert_eq!(
            input["properties"]["limit"],
            json!({"type":"integer","minimum":1,"maximum":100,"default":50})
        );
        assert_eq!(
            input["properties"]["before_timestamp"]["maximum"],
            JS_SAFE_INTEGER
        );
        assert_eq!(
            input["dependentRequired"],
            json!({"before_timestamp":["before_message_id"], "before_message_id":["before_timestamp"]})
        );
        let output = &tool["outputSchema"];
        assert_eq!(output["additionalProperties"], false);
        assert_eq!(
            output["required"],
            json!(["hasMore", "messages", "nextCursor"])
        );
        let message = &output["properties"]["messages"]["items"];
        let mut required = vec![
            "id",
            "contactId",
            "contactName",
            "contactPhone",
            "chatType",
            "timestamp",
            "timestampIso",
            "isFromMe",
            "senderName",
            "senderPhone",
            "text",
            "originalText",
            "translatedText",
            "isTranslated",
            "sourceLanguage",
            "contentType",
            "deliveryStatus",
            "replyContext",
        ];
        required.sort_unstable();
        assert_eq!(message["required"], json!(required));
        assert_eq!(
            message["properties"].as_object().unwrap().len(),
            required.len()
        );
        assert_eq!(message["additionalProperties"], false);
        for name in [
            "contactName",
            "contactPhone",
            "timestampIso",
            "senderName",
            "senderPhone",
            "text",
            "originalText",
            "translatedText",
            "sourceLanguage",
            "deliveryStatus",
        ] {
            assert_eq!(
                message["properties"][name]["type"],
                json!(["string", "null"])
            );
        }
        let quote = &message["properties"]["replyContext"]["anyOf"][1];
        assert_eq!(
            quote["required"],
            json!(["messageId", "senderName", "text"])
        );
        assert_eq!(quote["additionalProperties"], false);
        assert_eq!(
            output["properties"]["nextCursor"]["anyOf"][1]["required"],
            json!(["beforeMessageId", "beforeTimestamp"])
        );
    }

    fn invalid_feed_arguments() -> Vec<Value> {
        vec![
            json!(null),
            json!([]),
            json!({"tenant":"other"}),
            json!({"contact_id":"ignored"}),
            json!({"limit":null}),
            json!({"limit":0}),
            json!({"limit":101}),
            json!({"limit":-1}),
            json!({"limit":1.5}),
            json!({"limit":"2"}),
            json!({"limit":true}),
            json!({"before_timestamp":100}),
            json!({"before_message_id":"a"}),
            json!({"before_timestamp":null,"before_message_id":"a"}),
            json!({"before_timestamp":100,"before_message_id":null}),
            json!({"before_timestamp":-1,"before_message_id":"a"}),
            json!({"before_timestamp":1.5,"before_message_id":"a"}),
            json!({"before_timestamp":JS_SAFE_INTEGER+1,"before_message_id":"a"}),
            json!({"before_timestamp":u64::MAX,"before_message_id":"a"}),
            json!({"before_timestamp":"100","before_message_id":"a"}),
            json!({"before_timestamp":true,"before_message_id":"a"}),
            json!({"before_timestamp":100,"before_message_id":""}),
            json!({"before_timestamp":100,"before_message_id":2}),
        ]
    }

    #[test]
    fn unified_feed_runtime_arguments_enforce_schema_and_preserve_opaque_ids() {
        for args in invalid_feed_arguments() {
            assert!(FeedArguments::parse(&args).is_err(), "accepted {args}");
        }
        assert_eq!(FeedArguments::parse(&json!({})).unwrap().limit, 50);
        let id = "  opaque:/%🧪  ";
        let args = FeedArguments::parse(
            &json!({"limit":100,"before_timestamp":JS_SAFE_INTEGER,"before_message_id":id}),
        )
        .unwrap();
        assert_eq!(args.limit, 100);
        assert_eq!(args.before_timestamp, Some(JS_SAFE_INTEGER));
        assert_eq!(args.before_message_id.as_deref(), Some(id));
        assert_eq!(
            FeedArguments::parse(
                &json!({"limit":2.0,"before_timestamp":0.0,"before_message_id":"a"})
            )
            .unwrap()
            .before_timestamp,
            Some(0)
        );
    }

    #[tokio::test]
    async fn unified_feed_exact_content_and_cursor_are_independent_of_contacts_page() {
        let fixture = FeedFixture::new();
        seed_feed(&fixture.state);
        let server = WhatsAppMcpServer::new(
            fixture.state.clone(),
            McpPermissions {
                read: true,
                send: true,
            },
        );
        let contacts = server
            .handle_contacts(json!({}), false)
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert_eq!(contacts["contacts"].as_array().unwrap().len(), 50);
        assert!(!contacts["contacts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|contact| contact["id"] == "synthetic-group@g.us"));
        let page = server
            .handle_read_unified_messages(json!({"limit":2}))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert_eq!(
            page,
            json!({"messages":[
            {"id":"b","contactId":"synthetic-group@g.us","contactName":"Fixture Group","contactPhone":null,"chatType":"group",
                "timestamp":100,"timestampIso":"1970-01-01T00:00:00.100+00:00","isFromMe":false,"senderName":"Fixture Member","senderPhone":"900000000001@lid",
                "text":"Look","originalText":"Regarde","translatedText":"Look","isTranslated":true,"sourceLanguage":"French","contentType":"Image","deliveryStatus":"delivered",
                "replyContext":{"messageId":"outside-history","senderName":"Quoted Member","text":"Earlier quote"}},
            {"id":"c","contactId":"447700900123@s.whatsapp.net","contactName":"Fixture Friend","contactPhone":"447700900123","chatType":"private",
                "timestamp":200,"timestampIso":"1970-01-01T00:00:00.200+00:00","isFromMe":true,"senderName":"Me","senderPhone":"447700900000",
                "text":"See you","originalText":"See you","translatedText":"À bientôt","isTranslated":true,"sourceLanguage":"French","contentType":"text","deliveryStatus":"uncertain","replyContext":null}
        ],"hasMore":true,"nextCursor":{"beforeTimestamp":100,"beforeMessageId":"b"}})
        );
        let older = server.handle_read_unified_messages(json!({"limit":2,"before_timestamp":page["nextCursor"]["beforeTimestamp"],"before_message_id":page["nextCursor"]["beforeMessageId"]})).await.unwrap().structured_content.unwrap();
        assert_eq!(older["messages"].as_array().unwrap().len(), 1);
        assert_eq!(older["messages"][0]["id"], "a");
        assert_eq!(
            older["messages"][0]["contactId"],
            "447700900123@s.whatsapp.net"
        );
        assert_eq!(older["messages"][0]["text"], "hello");
        assert_eq!(older["hasMore"], false);
        assert!(older["nextCursor"].is_null());
        let all = server
            .handle_read_unified_messages(json!({}))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert_eq!(
            all["messages"]
                .as_array()
                .unwrap()
                .iter()
                .map(|message| message["id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["a", "b", "c"]
        );
        assert!(!all.to_string().contains("SYNTHETIC_MEDIA_BYTES"));
        // Navigating by the source ID works without having loaded its contact row.
        let chat = server
            .handle_open(json!({"contact_id":page["messages"][0]["contactId"]}), true)
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert_eq!(chat["contact"]["id"], "synthetic-group@g.us");
        assert_eq!(chat["messages"][0]["id"], "b");
        assert!(chat["messages"][0].get("chatType").is_none());
        assert_eq!(
            chat["messages"][0]["replyContext"]["unexpected"],
            "must not leak"
        );
    }

    #[tokio::test]
    async fn unified_feed_empty_missing_metadata_media_and_partial_quotes_are_explicit() {
        let fixture = FeedFixture::new();
        let server = WhatsAppMcpServer::new(
            fixture.state.clone(),
            McpPermissions {
                read: true,
                send: false,
            },
        );
        assert_eq!(
            server
                .handle_read_unified_messages(json!({}))
                .await
                .unwrap()
                .structured_content
                .unwrap(),
            json!({"messages":[],"hasMore":false,"nextCursor":null})
        );
        fixture
            .state
            .store
            .upsert_contact("unresolved@lid", None, None, Some("private"), 0)
            .unwrap();
        for (id, kind, caption, quote) in [
            ("audio", "Audio", None, json!("invalid quote")),
            (
                "document",
                "Document",
                Some("Document caption"),
                json!({"messageId":7,"text":"Partial quote","extra":true}),
            ),
            ("video", "Video", Some("Video caption"), json!(null)),
        ] {
            let mut message = extension_message(id, "unresolved@lid", 100, true);
            message.content_type = kind.into();
            message.content_json = json!({"type":kind.to_lowercase(),"caption":caption,"mediaData":"SYNTHETIC_MEDIA_BYTES","reply_context":quote}).to_string();
            message.original_text = None;
            message.translated_text = None;
            message.is_translated = false;
            fixture.state.store.add_message(&message).unwrap();
        }
        let page = server
            .handle_read_unified_messages(json!({}))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert_eq!(page["messages"].as_array().unwrap().len(), 3);
        for message in page["messages"].as_array().unwrap() {
            assert_eq!(message["contactId"], "unresolved@lid");
            assert!(message["contactName"].is_null());
            assert!(message["contactPhone"].is_null());
        }
        assert!(page["messages"][0]["text"].is_null());
        assert!(page["messages"][0]["replyContext"].is_null());
        assert_eq!(page["messages"][1]["text"], "Document caption");
        assert_eq!(
            page["messages"][1]["replyContext"],
            json!({"messageId":null,"senderName":null,"text":"Partial quote"})
        );
        assert_eq!(page["messages"][2]["text"], "Video caption");
        assert!(!page.to_string().contains("SYNTHETIC_MEDIA_BYTES"));
    }

    // Read only the disposable test database, never the service's session data.
    // Snapshot every persisted table so unexpected writes outside the message
    // and receipt tables are caught as well.
    fn feed_database_snapshot(
        dir: &std::path::Path,
    ) -> std::collections::BTreeMap<String, Vec<String>> {
        use rusqlite::{types::ValueRef, Connection, OpenFlags};
        let connection =
            Connection::open_with_flags(dir.join("messages.db"), OpenFlags::SQLITE_OPEN_READ_ONLY)
                .unwrap();
        let names = connection.prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap()
            .query_map([], |row| row.get::<_, String>(0)).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
        let mut snapshot = std::collections::BTreeMap::new();
        for name in names {
            let mut statement = connection
                .prepare(&format!("SELECT * FROM \"{}\"", name.replace('"', "\"\"")))
                .unwrap();
            let columns = statement.column_count();
            let mut rows = statement
                .query_map([], |row| {
                    let values = (0..columns)
                        .map(|index| {
                            Ok(match row.get_ref(index)? {
                                ValueRef::Null => Value::Null,
                                ValueRef::Integer(value) => json!(value),
                                ValueRef::Real(value) => json!(value),
                                ValueRef::Text(value) => json!(std::str::from_utf8(value).unwrap()),
                                ValueRef::Blob(value) => json!({"blob":value}),
                            })
                        })
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    Ok(json!(values).to_string())
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            rows.sort();
            snapshot.insert(name, rows);
        }
        snapshot
    }

    fn feed_oauth_token(state: &AppState, token: &str, scope: &str, expires_at: i64) {
        state
            .store
            .oauth_store_client(&crate::oauth::OAuthClientRegistration {
                client_id: token.into(),
                client_name: Some("Synthetic feed client".into()),
                redirect_uris: vec!["http://127.0.0.1/callback".into()],
                scope: scope.into(),
                created_at: 0,
            })
            .unwrap();
        state
            .store
            .oauth_store_access_token(&crate::oauth::AccessToken {
                token: token.into(),
                client_id: token.into(),
                scope: scope.into(),
                created_at: 0,
                expires_at,
                refresh_token: None,
            })
            .unwrap();
    }

    async fn feed_rpc(
        state: Arc<AppState>,
        authorization: Option<&str>,
        method: &str,
        params: Value,
    ) -> axum::response::Response {
        use tower::ServiceExt;
        let mut request = axum::http::Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("host", "127.0.0.1:3000")
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .header("mcp-protocol-version", "2025-11-25");
        if let Some(authorization) = authorization {
            request = request.header("authorization", authorization);
        }
        crate::web::create_router(state)
            .oneshot(
                request
                    .body(axum::body::Body::from(
                        json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    async fn feed_rpc_body(response: axum::response::Response) -> Value {
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), 2 * 1024 * 1024)
            .await
            .unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        serde_json::from_str(text).unwrap_or_else(|_| {
            serde_json::from_str(
                text.lines()
                    .find_map(|line| line.strip_prefix("data: "))
                    .expect("SSE data"),
            )
            .unwrap()
        })
    }

    #[tokio::test]
    async fn unified_feed_http_auth_isolation_and_repeated_reads_have_no_side_effects() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let ai_requests = Arc::new(AtomicUsize::new(0));
        let observed = ai_requests.clone();
        let mock = axum::Router::new().route(
            "/responses",
            axum::routing::post(move || {
                let observed = observed.clone();
                async move {
                    observed.fetch_add(1, Ordering::SeqCst);
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api_url = format!("http://{}/responses", listener.local_addr().unwrap());
        let mock_task = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
        let mut first = FeedFixture::new();
        let mutable = Arc::get_mut(&mut first.state).unwrap();
        mutable.password = Some("synthetic-password".into());
        mutable.ai_service = Some(Arc::new(
            crate::translation::TranslationService::new_with_api_url(api_url),
        ));
        seed_feed(&first.state);
        let private = "447700900123@s.whatsapp.net";
        first
            .state
            .store
            .update_conversation_settings(
                private,
                &ConversationSettings {
                    translation_enabled: true,
                    reply_only: true,
                    ..Default::default()
                },
            )
            .unwrap();
        let mut untranslated = extension_message("needs-translation", private, 50, true);
        untranslated.original_text = Some("Bonjour, ceci attend une traduction".into());
        untranslated.translated_text = None;
        untranslated.source_language = None;
        untranslated.is_translated = false;
        first
            .state
            .store
            .add_message_with_notification(&untranslated, Some(false))
            .unwrap();
        let mut audio = extension_message("needs-voice", private, 40, true);
        audio.content_type = "Audio".into();
        audio.content_json =
            json!({"type":"audio","media_data":"SYNTHETIC_AUDIO_BYTES"}).to_string();
        audio.original_text = None;
        audio.translated_text = None;
        audio.source_language = None;
        audio.is_translated = false;
        first
            .state
            .store
            .add_message_with_notification(&audio, Some(false))
            .unwrap();
        for (token, scope, expiration) in [
            ("feed-read", "whatsapp.read", i64::MAX),
            ("feed-full", "whatsapp.read whatsapp.send", i64::MAX),
            ("feed-legacy", "mcp", i64::MAX),
            ("feed-send-only", "whatsapp.send", i64::MAX),
            ("feed-expired", "whatsapp.read", 1),
        ] {
            feed_oauth_token(&first.state, token, scope, expiration);
        }
        let (tx, mut commands) = tokio::sync::mpsc::channel(8);
        first.state.set_command_tx(tx).await;
        *first.state.connected.write().await = true;
        let mut events = first.state.broadcast_tx.subscribe();
        let request_id = first.state.request_id_counter.load(Ordering::SeqCst);
        let before = feed_database_snapshot(&first.dir);
        assert!(before["translation_jobs"].is_empty());
        assert!(before["viewed_messages"].is_empty());
        assert!(!before["unread_messages"].is_empty());
        assert!(!before["pending_notifications"].is_empty());
        let call = json!({"name":"read_unified_messages","arguments":{"limit":2}});
        for auth in [
            None,
            Some("Bearer invalid-feed-token"),
            Some("Basic feed-read"),
            Some("Bearer feed-expired"),
        ] {
            let response = feed_rpc(first.state.clone(), auth, "tools/call", call.clone()).await;
            assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
            assert!(response.headers().contains_key("www-authenticate"));
        }
        let tools = feed_rpc_body(
            feed_rpc(
                first.state.clone(),
                Some("Bearer feed-send-only"),
                "tools/list",
                json!({}),
            )
            .await,
        )
        .await;
        assert!(!tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["name"] == "read_unified_messages"));
        let denied = feed_rpc_body(
            feed_rpc(
                first.state.clone(),
                Some("Bearer feed-send-only"),
                "tools/call",
                call.clone(),
            )
            .await,
        )
        .await;
        assert!(denied.get("error").is_some());
        for args in invalid_feed_arguments()
            .into_iter()
            .filter(Value::is_object)
        {
            let rejected = feed_rpc_body(
                feed_rpc(
                    first.state.clone(),
                    Some("Bearer feed-read"),
                    "tools/call",
                    json!({"name":"read_unified_messages","arguments":args}),
                )
                .await,
            )
            .await;
            assert!(rejected.get("error").is_some(), "accepted {args}");
        }
        for token in ["feed-read", "feed-full", "feed-legacy"] {
            let authorization = format!("Bearer {token}");
            for _ in 0..3 {
                let page = feed_rpc_body(
                    feed_rpc(
                        first.state.clone(),
                        Some(&authorization),
                        "tools/call",
                        call.clone(),
                    )
                    .await,
                )
                .await;
                let page = &page["result"]["structuredContent"];
                assert_eq!(
                    page["messages"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|m| m["id"].as_str().unwrap())
                        .collect::<Vec<_>>(),
                    ["b", "c"]
                );
                assert_eq!(
                    page["nextCursor"],
                    json!({"beforeTimestamp":100,"beforeMessageId":"b"})
                );
                let older = feed_rpc_body(feed_rpc(first.state.clone(), Some(&authorization), "tools/call", json!({"name":"read_unified_messages","arguments":{"limit":100,"before_timestamp":100,"before_message_id":"b"}})).await).await;
                let older = &older["result"]["structuredContent"];
                assert_eq!(
                    older["messages"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|m| m["id"].as_str().unwrap())
                        .collect::<Vec<_>>(),
                    ["needs-voice", "needs-translation", "a"]
                );
                assert_eq!(older["hasMore"], false);
                assert!(older["nextCursor"].is_null());
            }
        }
        let mut second = FeedFixture::new();
        Arc::get_mut(&mut second.state).unwrap().password = Some("other-synthetic-password".into());
        second
            .state
            .store
            .upsert_contact(private, Some("Second instance"), None, Some("private"), 0)
            .unwrap();
        second
            .state
            .store
            .add_message(&extension_message("second-only", private, 100, true))
            .unwrap();
        feed_oauth_token(&second.state, "second-read", "whatsapp.read", i64::MAX);
        let second_before = feed_database_snapshot(&second.dir);
        assert_eq!(
            feed_rpc(
                second.state.clone(),
                Some("Bearer feed-read"),
                "tools/call",
                call.clone()
            )
            .await
            .status(),
            axum::http::StatusCode::UNAUTHORIZED
        );
        let isolated = feed_rpc_body(
            feed_rpc(
                second.state.clone(),
                Some("Bearer second-read"),
                "tools/call",
                call,
            )
            .await,
        )
        .await;
        assert_eq!(
            isolated["result"]["structuredContent"]["messages"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            isolated["result"]["structuredContent"]["messages"][0]["id"],
            "second-only"
        );
        assert_eq!(
            isolated["result"]["structuredContent"]["messages"][0]["contactName"],
            "Second instance"
        );
        assert_eq!(feed_database_snapshot(&first.dir), before);
        assert_eq!(feed_database_snapshot(&second.dir), second_before);
        assert!(matches!(
            commands.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
        assert!(matches!(
            events.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
        ));
        assert_eq!(ai_requests.load(Ordering::SeqCst), 0);
        assert_eq!(
            first.state.request_id_counter.load(Ordering::SeqCst),
            request_id
        );
        assert!(first.state.mcp_prepared_messages.read().await.is_empty());
        assert!(first.state.pending_send_requests.read().await.is_empty());
        assert_eq!(first.state.voice_queue.available_permits(), 4);
        mock_task.abort();
    }

    #[test]
    fn translation_failure_never_falls_back_to_english_when_translation_is_needed() {
        let result = resolve_prepared_translation(
            "Hello",
            Some("French"),
            TranslationMode::Auto,
            Err("translator unavailable".to_string()),
        );

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("No message was sent or prepared"));
    }

    #[test]
    fn no_translation_mode_can_intentionally_preserve_source_text() {
        assert_eq!(
            resolve_prepared_translation(
                "Hello",
                Some("French"),
                TranslationMode::Never,
                Err("translator unavailable".to_string()),
            )
            .expect("translation disabled"),
            ("Hello".to_string(), false, None)
        );
    }

    #[test]
    fn read_tools_are_annotated_read_only_and_send_is_not() {
        let list = WhatsAppMcpServer::list_contacts_tool();
        let read = WhatsAppMcpServer::read_messages_tool();
        let send = WhatsAppMcpServer::send_message_tool();

        assert_eq!(
            list.annotations
                .as_ref()
                .and_then(|value| value.read_only_hint),
            Some(true)
        );
        assert_eq!(
            read.annotations
                .as_ref()
                .and_then(|value| value.read_only_hint),
            Some(true)
        );
        assert_eq!(
            send.annotations
                .as_ref()
                .and_then(|value| value.read_only_hint),
            Some(false)
        );
        assert_eq!(
            send.annotations
                .as_ref()
                .and_then(|value| value.idempotent_hint),
            Some(false)
        );
    }

    #[test]
    fn scopes_control_advertised_tools() {
        assert_eq!(
            McpPermissions::from_scope("whatsapp.read"),
            McpPermissions {
                read: true,
                send: false
            }
        );
        assert_eq!(
            McpPermissions::from_scope("whatsapp.read whatsapp.send"),
            McpPermissions {
                read: true,
                send: true
            }
        );
        assert_eq!(
            McpPermissions::from_scope("mcp"),
            McpPermissions {
                read: true,
                send: true
            }
        );
    }

    #[test]
    fn read_only_scope_hides_external_write_tools() {
        let (state, data_dir) = test_state();
        let server = WhatsAppMcpServer::new(
            state,
            McpPermissions {
                read: true,
                send: false,
            },
        );
        let names: Vec<String> = server
            .advertised_tools()
            .into_iter()
            .map(|tool| tool.name.into_owned())
            .collect();

        assert!(names.iter().any(|name| name == "search_contacts"));
        assert!(names.iter().any(|name| name == "prepare_message"));
        assert!(!names.iter().any(|name| name == "send_message"));
        assert!(!names.iter().any(|name| name == "react_to_message"));
        std::fs::remove_dir_all(data_dir).expect("remove test data");
    }

    #[tokio::test]
    async fn search_contacts_returns_rich_structured_results() {
        let (state, data_dir) = test_state();
        state
            .store
            .upsert_contact(
                "447700900123@s.whatsapp.net",
                Some("Lee Example"),
                Some("447700900123"),
                Some("private"),
                1_700_000_000_000,
            )
            .expect("insert contact");
        let server = WhatsAppMcpServer::new(
            state,
            McpPermissions {
                read: true,
                send: false,
            },
        );

        let result = server
            .handle_contacts(json!({"query":"lee"}), true)
            .await
            .expect("search contacts");
        let payload = result.structured_content.expect("structured output");
        assert_eq!(payload["totalMatched"], 1);
        assert_eq!(payload["contacts"][0]["name"], "Lee Example");
        assert_eq!(
            payload["contacts"][0]["lastMessageTime"],
            1_700_000_000_000_i64
        );
        std::fs::remove_dir_all(data_dir).expect("remove test data");
    }

    #[tokio::test]
    async fn disabled_conversations_prepare_original_mcp_text_and_reject_stale_translated_preparations(
    ) {
        let (state, data_dir) = test_state();
        let server = WhatsAppMcpServer::new(
            state.clone(),
            McpPermissions {
                read: true,
                send: true,
            },
        );
        for contact_id in ["person@s.whatsapp.net", "family@g.us"] {
            state
                .store
                .upsert_contact(contact_id, Some("Synthetic chat"), None, None, 0)
                .unwrap();
            let result = server.handle_prepare_message(json!({
                "contact_id": contact_id, "text": "Hello", "translation_mode": "auto", "target_language": "Hungarian"
            })).await.unwrap().structured_content.unwrap();
            assert_eq!(result["finalText"], "Hello");
            assert_eq!(result["translated"], false);
            let preparations = state.mcp_prepared_messages.read().await;
            let mut stale: PreparedMcpMessage = serde_json::from_value(
                preparations
                    .get(result["preparationToken"].as_str().unwrap())
                    .unwrap()
                    .clone(),
            )
            .unwrap();
            drop(preparations);
            stale.translated = true;
            stale.final_text = "Szia".into();
            state
                .mcp_prepared_messages
                .write()
                .await
                .insert(stale.token.clone(), serde_json::to_value(&stale).unwrap());
            let error = server.handle_prepared_send(json!({"preparation_token": stale.token, "idempotency_key": format!("stale-{contact_id}")}), false).await.unwrap_err();
            assert!(error.to_string().contains("Translation is now off"));
            assert!(state
                .mcp_prepared_messages
                .read()
                .await
                .contains_key(&stale.token));
        }
        std::fs::remove_dir_all(data_dir).unwrap();
    }

    #[tokio::test]
    async fn reply_only_rejects_new_and_previously_prepared_unquoted_sends() {
        let (state, data_dir) = test_state();
        let contact = "family@g.us";
        state
            .store
            .upsert_contact(contact, None, None, Some("group"), 1)
            .unwrap();
        let server = WhatsAppMcpServer::new(
            state.clone(),
            McpPermissions {
                read: true,
                send: true,
            },
        );
        let prepared = server
            .handle_prepare_message(json!({"contact_id":contact,"text":"Hello"}))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        state
            .store
            .update_conversation_settings(
                contact,
                &ConversationSettings {
                    reply_only: true,
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(server
            .handle_prepare_message(json!({"contact_id":contact,"text":"Hello"}))
            .await
            .unwrap_err()
            .to_string()
            .contains("Reply only"));
        assert!(server.handle_prepared_send(json!({"preparation_token":prepared["preparationToken"],"idempotency_key":"reply-only-test"}),false).await.unwrap_err().to_string().contains("Reply only"));
        std::fs::remove_dir_all(data_dir).unwrap();
    }

    #[tokio::test]
    async fn required_conversation_translation_without_a_translator_prepares_nothing() {
        let (state, data_dir) = test_state();
        let contact_id = "33612345678@s.whatsapp.net";
        state
            .store
            .upsert_contact(
                contact_id,
                Some("French Contact"),
                Some("33612345678"),
                Some("private"),
                1_700_000_000_000,
            )
            .expect("insert contact");
        state
            .store
            .update_conversation_settings(
                contact_id,
                &ConversationSettings {
                    reply_only: false,
                    translation_enabled: true,
                    language_override: Some("French".to_string()),
                    translation_style: None,
                    send_original_follow_up: false,
                },
            )
            .expect("settings");
        let server = WhatsAppMcpServer::new(
            state.clone(),
            McpPermissions {
                read: true,
                send: true,
            },
        );

        let error = server
            .handle_prepare_message(json!({
                "contact_id":contact_id,
                "text":"Hello",
                "translation_mode":"auto"
            }))
            .await
            .expect_err("translation must fail closed");
        assert!(error
            .to_string()
            .contains("No message was sent or prepared"));
        assert!(state.mcp_prepared_messages.read().await.is_empty());
        std::fs::remove_dir_all(data_dir).expect("remove test data");
    }

    #[tokio::test]
    async fn choosing_reply_for_a_non_reply_preparation_does_not_consume_it() {
        let (state, data_dir) = test_state();
        let token = "prepared-token";
        let prepared = PreparedMcpMessage {
            token: token.to_string(),
            contact_id: "chat@example.test".to_string(),
            contact_name: Some("Test".to_string()),
            original_text: "Hello".to_string(),
            final_text: "Hello".to_string(),
            translated: false,
            target_language: None,
            translation_mode: "never".to_string(),
            reply_to_message_id: None,
            reply_to_sender: None,
            reply_to_text: None,
            created_at: chrono::Utc::now().timestamp_millis(),
            expires_at: chrono::Utc::now().timestamp_millis() + PREPARATION_TTL_MILLIS,
        };
        state.mcp_prepared_messages.write().await.insert(
            token.to_string(),
            serde_json::to_value(prepared).expect("serialize preparation"),
        );
        let server = WhatsAppMcpServer::new(
            state.clone(),
            McpPermissions {
                read: true,
                send: true,
            },
        );

        server
            .handle_prepared_send(
                json!({"preparation_token":token,"idempotency_key":"reply-test-key"}),
                true,
            )
            .await
            .expect_err("not a reply preparation");
        assert!(state.mcp_prepared_messages.read().await.contains_key(token));
        std::fs::remove_dir_all(data_dir).expect("remove test data");
    }

    #[tokio::test]
    async fn idempotency_claim_is_atomic() {
        let (state, data_dir) = test_state();
        let server = WhatsAppMcpServer::new(
            state,
            McpPermissions {
                read: true,
                send: true,
            },
        );
        assert!(
            server
                .begin_idempotent_operation("unique-key", "send:one")
                .await
        );
        assert!(
            !server
                .begin_idempotent_operation("unique-key", "send:one")
                .await
        );
        std::fs::remove_dir_all(data_dir).expect("remove test data");
    }

    fn extension_message(
        id: &str,
        contact_id: &str,
        timestamp: i64,
        incoming: bool,
    ) -> StoredMessage {
        StoredMessage {
            id: id.into(),
            contact_id: contact_id.into(),
            timestamp,
            is_from_me: !incoming,
            is_forwarded: false,
            sender_name: Some("Synthetic Sender".into()),
            sender_phone: None,
            contact_name: None,
            contact_phone: None,
            chat_type: "private".into(),
            content_type: "text".into(),
            content_json: r#"{"type":"text","body":"hello"}"#.into(),
            content: Some(json!({"type":"text","body":"hello"})),
            original_text: Some("hello".into()),
            translated_text: incoming.then(|| "bonjour".into()),
            source_language: Some("English".into()),
            is_translated: incoming,
            delivery_status: Some("delivered".into()),
        }
    }

    #[test]
    fn extension_resources_and_entrypoints_obey_read_scope_and_display_contract() {
        let (state, dir) = test_state();
        for read in [false, true] {
            let server =
                WhatsAppMcpServer::new(state.clone(), McpPermissions { read, send: false });
            assert_eq!(server.chat_resources().resources.len(), usize::from(read));
            assert_eq!(server.chat_resource(CHAT_RESOURCE_URI).is_ok(), read);
            assert!(server.chat_resource("file:///etc/passwd").is_err());
            let tools = server.advertised_tools();
            assert_eq!(tools.iter().any(|tool| tool.name == "open_chats"), read);
            if read {
                let opener = tools.iter().find(|tool| tool.name == "open_chats").unwrap();
                let metadata = serde_json::to_value(opener.meta.as_ref().unwrap()).unwrap();
                assert_eq!(metadata["ui"]["resourceUri"], CHAT_RESOURCE_URI);
                assert_eq!(
                    metadata["openai/ui"]["entrypoints"],
                    json!([{"type":"global"},{"type":"thread"}])
                );
                assert!(opener.input_schema.get("required").is_none());
                assert_eq!(
                    opener.annotations.as_ref().unwrap().read_only_hint,
                    Some(true)
                );
                let resource =
                    serde_json::to_value(server.chat_resource(CHAT_RESOURCE_URI).unwrap()).unwrap();
                assert_eq!(resource["contents"][0]["mimeType"], CHAT_MIME_TYPE);
                assert_eq!(
                    resource["contents"][0]["_meta"]["openai/ui"]["preferredDisplayMode"],
                    "fullscreen"
                );
                assert_eq!(
                    resource["contents"][0]["_meta"]["ui"]["csp"]["connectDomains"],
                    json!([])
                );
            }
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn extension_openers_keep_instance_data_and_permissions_isolated_without_writes() {
        let (first, first_dir) = test_state();
        let (second, second_dir) = test_state();
        first
            .store
            .upsert_contact(
                "alice@s.whatsapp.net",
                Some("Alice"),
                None,
                Some("private"),
                100,
            )
            .unwrap();
        first
            .store
            .add_message(&extension_message("a1", "alice@s.whatsapp.net", 100, true))
            .unwrap();
        second
            .store
            .upsert_contact(
                "bob@s.whatsapp.net",
                Some("Bob"),
                None,
                Some("private"),
                100,
            )
            .unwrap();
        let first_server = WhatsAppMcpServer::new(
            first.clone(),
            McpPermissions {
                read: true,
                send: false,
            },
        );
        let second_server = WhatsAppMcpServer::new(
            second,
            McpPermissions {
                read: true,
                send: true,
            },
        );
        let output = first_server
            .handle_open(json!({}), false)
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert_eq!(output["contacts"][0]["name"], "Alice");
        assert_eq!(output["permissions"]["send"], false);
        assert!(second_server
            .handle_open(json!({"contact_id":"alice@s.whatsapp.net"}), true)
            .await
            .is_err());
        assert!(first_server
            .handle_open(json!({"contact_id":"bob@s.whatsapp.net"}), true)
            .await
            .is_err());
        let unread_before = first
            .store
            .get_contact("alice@s.whatsapp.net")
            .unwrap()
            .unwrap()
            .unread_count;
        for _ in 0..3 {
            let output = first_server
                .handle_open(json!({"contact_id":"alice@s.whatsapp.net"}), true)
                .await
                .unwrap()
                .structured_content
                .unwrap();
            assert_eq!(output["messages"][0]["text"], "bonjour");
            assert_eq!(output["messages"][0]["originalText"], "hello");
        }
        assert_eq!(
            first
                .store
                .get_contact("alice@s.whatsapp.net")
                .unwrap()
                .unwrap()
                .unread_count,
            unread_before
        );
        assert!(first.mcp_prepared_messages.read().await.is_empty());
        assert!(first_server
            .handle_translate_message(
                json!({"contact_id":"alice@s.whatsapp.net","message_id":"a1"})
            )
            .await
            .is_err());
        for dir in [first_dir, second_dir] {
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    #[tokio::test]
    async fn extension_pagination_covers_all_chats_and_sparse_messages_with_equal_timestamps() {
        let (state, dir) = test_state();
        for id in [
            "first@s.whatsapp.net",
            "second@s.whatsapp.net",
            "third@g.us",
        ] {
            state
                .store
                .upsert_contact(id, None, None, None, 100)
                .unwrap();
        }
        let server = WhatsAppMcpServer::new(
            state.clone(),
            McpPermissions {
                read: true,
                send: false,
            },
        );
        let first = server
            .handle_contacts(json!({"limit":2}), false)
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert_eq!(first["nextCursor"], 2);
        let last = server
            .handle_contacts(json!({"limit":2,"cursor":2}), false)
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert_eq!(last["contacts"].as_array().unwrap().len(), 1);
        assert!(last["nextCursor"].is_null());
        for i in 0..650 {
            state
                .store
                .add_message(&extension_message(
                    &format!("m{i:04}"),
                    "first@s.whatsapp.net",
                    100,
                    i == 5 || i == 600,
                ))
                .unwrap();
        }
        let page = server
            .handle_read_messages(
                json!({"contact_id":"first@s.whatsapp.net","limit":1,"direction":"incoming"}),
            )
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert_eq!(page["messages"][0]["id"], "m0600");
        assert_eq!(page["hasMore"], true);
        let cursor = &page["nextCursor"];
        let older = server.handle_read_messages(json!({"contact_id":"first@s.whatsapp.net","limit":1,"direction":"incoming","before_timestamp":cursor["beforeTimestamp"],"before_message_id":cursor["beforeMessageId"]})).await.unwrap().structured_content.unwrap();
        assert_eq!(older["messages"][0]["id"], "m0005");
        assert_eq!(older["hasMore"], false);
        let search = server
            .handle_search_messages(json!({"query":"bonjour","limit":1}))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert_eq!(search["nextCursor"], 1);
        let next = server
            .handle_search_messages(json!({"query":"bonjour","limit":1,"cursor":1}))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert_eq!(next["messages"][0]["id"], "m0005");
        assert!(next["nextCursor"].is_null());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn verbose_mcp_read_logs_metadata_without_message_or_number() {
        const BODY: &str = "zz-unique-body-449d";
        const PHONE: &str = "15550001111";
        const NAME: &str = "ZzContactName449d";
        let jid = format!("{PHONE}@s.whatsapp.net");
        let (state, dir) = test_state();
        state
            .store
            .upsert_contact(&jid, Some(NAME), Some(PHONE), Some("private"), 10)
            .unwrap();
        let mut message = extension_message("privacy-1", &jid, 10, true);
        message.sender_name = Some(NAME.into());
        message.sender_phone = Some(PHONE.into());
        message.contact_name = Some(NAME.into());
        message.contact_phone = Some(PHONE.into());
        message.original_text = Some(BODY.into());
        message.translated_text = Some("zz-translated-body-449d".into());
        message.content_json = json!({"type":"text","body":BODY}).to_string();
        message.content = Some(json!({"type":"text","body":BODY}));
        state.store.add_message(&message).unwrap();
        feed_oauth_token(&state, "privacy-token", "whatsapp.read", i64::MAX);

        let buf = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
        let subscriber = tracing_subscriber::fmt()
            .with_env_filter(crate::log_privacy::logging_filter(true))
            .with_ansi(false)
            .without_time()
            .with_target(false)
            .with_writer({
                let buf = buf.clone();
                move || LogCapture(buf.clone())
            })
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);

        let read = feed_rpc(
            state.clone(),
            Some("Bearer privacy-token"),
            "tools/call",
            json!({"name":"read_messages","arguments":{"contact_id":jid,"limit":5}}),
        )
        .await;
        let read_body = feed_rpc_body(read).await;
        let read_text = read_body.to_string();
        assert!(
            read_text.contains(BODY),
            "api response must still return the message"
        );
        assert!(
            read_text.contains(PHONE),
            "api response must still return the number"
        );

        let failed = feed_rpc(
            state.clone(),
            Some("Bearer privacy-token"),
            "tools/call",
            json!({"name":"read_messages","arguments":{"contact_id":"missing-chat","note":BODY,"phone":PHONE}}),
        )
        .await;
        let failed_body = feed_rpc_body(failed).await;
        assert!(failed_body.get("error").is_some(), "{failed_body}");

        let logs = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        assert!(!logs.contains(BODY), "{logs}");
        assert!(!logs.contains(PHONE), "{logs}");
        assert!(!logs.contains(NAME), "{logs}");
        assert!(!logs.contains("zz-translated-body-449d"), "{logs}");
        assert!(logs.contains("method=\"tools/call\""), "{logs}");
        assert!(logs.contains("tool=\"read_messages\""), "{logs}");
        assert!(logs.contains("status=\"ok\""), "{logs}");
        assert!(logs.contains("status=\"error\""), "{logs}");
        assert!(logs.contains("count=1"), "{logs}");
        assert!(logs.contains("duration_ms="), "{logs}");
        assert!(logs.contains("error_code="), "{logs}");
        drop(_guard);
        let _ = std::fs::remove_dir_all(dir);
    }

    struct LogCapture(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for LogCapture {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("log buffer").extend_from_slice(data);
            Ok(data.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
}
