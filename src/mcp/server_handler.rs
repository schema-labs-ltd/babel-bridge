use super::*;

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
