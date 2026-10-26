use super::*;

impl WhatsAppMcpServer {
    pub(super) fn get_status_tool() -> Tool {
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

    pub(super) fn contacts_input_schema(require_query: bool) -> Value {
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

    pub(super) fn contacts_output_schema() -> Value {
        Self::object_output(
            json!({
                "contacts":{"type":"array","items":{"type":"object"}},
                "nextCursor":{"type":["integer","null"]},
                "totalMatched":{"type":"integer"}
            }),
            &["contacts", "totalMatched"],
        )
    }

    pub(super) fn list_contacts_tool() -> Tool {
        Self::tool(
            "list_contacts",
            "List WhatsApp conversations",
            "List recent WhatsApp contacts and groups with last activity, preview, unread count, pin state, and pagination.",
            Self::contacts_input_schema(false),
            Self::contacts_output_schema(),
            Self::read_annotations("List WhatsApp conversations"),
        )
    }

    pub(super) fn search_contacts_tool() -> Tool {
        Self::tool(
            "search_contacts",
            "Search WhatsApp contacts",
            "Resolve a person or group by case-insensitive name, phone number, or WhatsApp JID before reading or preparing a message.",
            Self::contacts_input_schema(true),
            Self::contacts_output_schema(),
            Self::read_annotations("Search WhatsApp contacts"),
        )
    }

    pub(super) fn read_messages_tool() -> Tool {
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

    pub(super) fn feed_object_schema(properties: Value) -> Value {
        let required: Vec<&str> = properties
            .as_object()
            .expect("feed schema properties")
            .keys()
            .map(String::as_str)
            .collect();
        json!({"type":"object", "required":required,
            "properties":properties, "additionalProperties":false})
    }

    pub(super) fn read_unified_messages_tool() -> Tool {
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

    pub(super) fn search_messages_tool() -> Tool {
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

    pub(super) fn prepare_message_tool() -> Tool {
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

    pub(super) fn send_message_tool() -> Tool {
        Self::prepared_send_tool(
            "send_message",
            "Send prepared WhatsApp message",
            "Send an exact message previously returned by prepare_message. Retries with the same idempotency key never send twice.",
        )
    }

    pub(super) fn reply_to_message_tool() -> Tool {
        Self::prepared_send_tool(
            "reply_to_message",
            "Send prepared WhatsApp reply",
            "Send a prepared reply. The preparation must include reply_to_message_id, fixing both reply target and final text before sending.",
        )
    }

    pub(super) fn prepared_send_tool(
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

    pub(super) fn react_to_message_tool() -> Tool {
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

    pub(super) fn mark_conversation_read_tool() -> Tool {
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

    pub(super) fn opener_tool(single_chat: bool) -> Tool {
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
}
