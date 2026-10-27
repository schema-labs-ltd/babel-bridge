use super::*;

pub(super) fn should_retry_reqwest_error(error: &reqwest::Error) -> bool {
    error.is_timeout() || error.is_connect()
}

pub(super) fn should_retry_status(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::REQUEST_TIMEOUT
        || status == reqwest::StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
}

pub(super) fn openai_retry_delay(attempt: usize) -> Duration {
    Duration::from_millis(200 * (attempt as u64 + 1))
}

/// Request fields the ChatGPT plan route rejects.
pub(super) const CHATGPT_UNSUPPORTED_FIELDS: &[&str] = &[
    "background",
    "conversation",
    "max_output_tokens",
    "max_tool_calls",
    "metadata",
    "moderation",
    "multi_agent",
    "previous_response_id",
    "prompt",
    "prompt_cache_retention",
    "safety_identifier",
    "temperature",
    "top_logprobs",
    "top_p",
    "truncation",
    "user",
];

/// Shape a Responses body for ChatGPT plan usage: no storage, streamed output,
/// an input array, developer instead of system messages, and the account's model.
pub(super) fn chatgpt_request_body(mut body: Value, model: Option<&str>) -> Value {
    if let Some(object) = body.as_object_mut() {
        for field in CHATGPT_UNSUPPORTED_FIELDS {
            object.remove(*field);
        }
        object.insert("store".into(), json!(false));
        object.insert("stream".into(), json!(true));
        if let Some(model) = model {
            object.insert("model".into(), json!(model));
        }
        match object.get_mut("input") {
            Some(Value::String(text)) => {
                let text = std::mem::take(text);
                object.insert("input".into(), json!([{ "role": "user", "content": text }]));
            }
            Some(Value::Array(items)) => {
                for item in items {
                    if item["role"] == "system" {
                        item["role"] = json!("developer");
                    }
                }
            }
            _ => {}
        }
    }
    body
}

/// Map ChatGPT plan errors to safe diagnostics and say whether a retry can help.
pub(super) struct ChatGptFailure {
    pub(super) failure: OpenAiApiFailure,
    pub(super) retryable: bool,
    pub(super) code: Option<String>,
    /// The request field OpenAI named, such as `reasoning.effort`.
    pub(super) param: Option<String>,
}

pub(super) fn chatgpt_failure(status: u16, code: Option<&str>) -> ChatGptFailure {
    let (reason, retryable) = match code {
        Some("subscription_sharing_usage_limit_exceeded") => ("chatgpt_usage_limit", false),
        Some("subscription_sharing_user_not_eligible") => ("chatgpt_not_eligible", false),
        Some(
            "subscription_sharing_usage_unavailable" | "subscription_sharing_user_unavailable",
        ) => ("chatgpt_unavailable", true),
        Some("subscription_sharing_unsupported_capability") => {
            ("chatgpt_unsupported_request", false)
        }
        Some("subscription_sharing_route_not_supported") => ("chatgpt_route_not_supported", false),
        Some("subscription_sharing_invalid_user") => ("chatgpt_sign_in_rejected", false),
        Some("chatpass_v2_scope_not_authorized" | "chatpass_v2_invalid_authorization_context") => {
            ("chatgpt_permission_missing", false)
        }
        _ => match status {
            401 => ("chatgpt_sign_in_rejected", false),
            403 => ("permission_denied", false),
            404 => ("model_or_endpoint_unavailable", false),
            429 => ("chatgpt_usage_limit", false),
            408 | 500..=599 => ("upstream_unavailable", true),
            _ => ("invalid_request", false),
        },
    };
    ChatGptFailure {
        failure: OpenAiApiFailure { status, reason },
        retryable,
        code: code.map(str::to_string),
        param: None,
    }
}

impl OpenAiApiFailure {
    pub(super) fn from_chatgpt_response(status: reqwest::StatusCode, body: &str) -> ChatGptFailure {
        let body: Value = serde_json::from_str(body).unwrap_or(Value::Null);
        let mut failure = chatgpt_failure(status.as_u16(), body["error"]["code"].as_str());
        failure.param = body["error"]["param"].as_str().map(str::to_string);
        // These name request fields and policy reasons, never message text.
        warn!(
            status = status.as_u16(),
            code = failure.code.as_deref().unwrap_or("-"),
            param = failure.param.as_deref().unwrap_or("-"),
            detail = body["detail"].as_str().unwrap_or("-"),
            "ChatGPT plan request rejected"
        );
        failure
    }
}

/// Fields the plan route may reject for some models and that requests work
/// without. Anything else, such as `model` or `input`, is never dropped.
pub(super) const CHATGPT_DROPPABLE_FIELDS: &[&str] = &[
    "reasoning",
    "reasoning.effort",
    "reasoning.summary",
    "text.verbosity",
    "text.format",
    "service_tier",
    "include",
    "tools",
    "tool_choice",
    "parallel_tool_calls",
];

/// The droppable field an `error.param` such as `text.format.type` refers to.
pub(super) fn droppable_field(param: &str) -> Option<&'static str> {
    let param = param.strip_prefix("body.").unwrap_or(param);
    CHATGPT_DROPPABLE_FIELDS
        .iter()
        .filter(|field| param == **field || param.starts_with(&format!("{field}.")))
        .max_by_key(|field| field.len())
        .copied()
}

pub(super) fn remove_field(body: &mut Value, field: &str) {
    match field.split_once('.') {
        Some((parent, child)) => {
            if let Some(object) = body.get_mut(parent).and_then(Value::as_object_mut) {
                object.remove(child);
                if object.is_empty() {
                    body.as_object_mut().map(|body| body.remove(parent));
                }
            }
        }
        None => {
            body.as_object_mut().map(|body| body.remove(field));
        }
    }
}

/// Read Responses server-sent events until the response finishes. Only
/// `response.completed` counts as success; `response.incomplete` is returned so
/// the caller reports why output stopped.
pub(super) async fn read_response_stream(
    response: reqwest::Response,
    idle_timeout: Duration,
) -> Result<std::result::Result<Value, ChatGptFailure>> {
    use futures::StreamExt;

    let started = std::time::Instant::now();
    let mut stream = response.bytes_stream();
    let mut buffer: Vec<u8> = Vec::new();
    let mut collected = StreamOutput::default();
    loop {
        while let Some(end) = find_event_end(&buffer) {
            let event: Vec<u8> = buffer.drain(..end.0 + end.1).collect();
            match collected.accept(&event[..end.0]) {
                Ok(Some(result)) => return Ok(Ok(result)),
                Ok(None) => {}
                Err(failure) => return Ok(Err(failure)),
            }
        }
        let next = match tokio::time::timeout(idle_timeout, stream.next()).await {
            Ok(next) => next,
            Err(_) => {
                collected.log_unfinished("idle timeout", started.elapsed());
                return Err(OpenAiOutputFailure("stream_stalled").into());
            }
        };
        match next {
            Some(Ok(chunk)) => buffer.extend_from_slice(&chunk),
            Some(Err(error)) => {
                collected.log_unfinished("read error", started.elapsed());
                return Err(error).context("ChatGPT stream interrupted");
            }
            None => {
                return match collected.accept(&buffer) {
                    Ok(Some(result)) => Ok(Ok(result)),
                    Ok(None) => {
                        collected.log_unfinished("closed early", started.elapsed());
                        Err(OpenAiOutputFailure("stream_interrupted").into())
                    }
                    Err(failure) => Ok(Err(failure)),
                };
            }
        }
    }
}

/// Output gathered from stream events, used when the terminal event's
/// response object leaves `output` empty.
#[derive(Default)]
pub(super) struct StreamOutput {
    items: Vec<Value>,
    text: String,
    done_text: String,
    /// Event types in arrival order, collapsing repeats, for diagnostics.
    event_types: Vec<String>,
}

impl StreamOutput {
    /// Log how far a stream got before it stopped: event types only, never text.
    fn log_unfinished(&self, reason: &str, elapsed: Duration) {
        warn!(
            reason,
            elapsed_ms = elapsed.as_millis() as u64,
            events = %self.event_types.join(","),
            "ChatGPT plan stream ended before response.completed"
        );
    }

    fn accept(&mut self, event: &[u8]) -> std::result::Result<Option<Value>, ChatGptFailure> {
        let Some(payload) = stream_event_payload(event) else {
            return Ok(None);
        };
        let kind = payload["type"].as_str().unwrap_or("?");
        if self.event_types.last().map(String::as_str) != Some(kind) && self.event_types.len() < 40
        {
            self.event_types.push(kind.to_string());
        }
        match payload["type"].as_str() {
            Some("response.output_item.done") => {
                self.items.push(payload["item"].clone());
                Ok(None)
            }
            Some("response.output_text.delta") => {
                self.text
                    .push_str(payload["delta"].as_str().unwrap_or_default());
                Ok(None)
            }
            Some("response.output_text.done") => {
                // The full text of one part; it supersedes that part's deltas.
                if let Some(text) = payload["text"].as_str() {
                    self.done_text.push_str(text);
                }
                Ok(None)
            }
            _ => {
                let Some(mut response) = parse_stream_payload(&payload)? else {
                    return Ok(None);
                };
                if !has_output_text(&response["output"]) {
                    let items = Value::Array(std::mem::take(&mut self.items));
                    let text = if self.done_text.is_empty() {
                        &self.text
                    } else {
                        &self.done_text
                    };
                    if has_output_text(&items) {
                        response["output"] = items;
                    } else if !text.is_empty() {
                        response["output"] = json!([{
                            "type": "message",
                            "content": [{ "type": "output_text", "text": text }],
                        }]);
                    } else {
                        // Shapes only, never text, so a new format can be diagnosed.
                        warn!(
                            output = %output_shape(&response["output"]),
                            items = %output_shape(&items),
                            "ChatGPT plan response finished without output text"
                        );
                    }
                }
                Ok(Some(response))
            }
        }
    }
}

/// Whether a Responses `output` array holds message text the parsers can read.
pub(super) fn has_output_text(output: &Value) -> bool {
    output.as_array().is_some_and(|items| {
        items.iter().any(|item| {
            item["type"] == "message"
                && item["content"].as_array().is_some_and(|parts| {
                    parts.iter().any(|part| {
                        part["type"] == "output_text"
                            && part["text"].as_str().is_some_and(|text| !text.is_empty())
                    })
                })
        })
    })
}

/// Item and content part types, for example `message[output_text],reasoning`.
pub(super) fn output_shape(output: &Value) -> String {
    output
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    let kind = item["type"].as_str().unwrap_or("?");
                    match item["content"].as_array() {
                        Some(parts) => format!(
                            "{kind}[{}]",
                            parts
                                .iter()
                                .map(|part| part["type"].as_str().unwrap_or("?"))
                                .collect::<Vec<_>>()
                                .join("|")
                        ),
                        None => kind.to_string(),
                    }
                })
                .collect::<Vec<_>>()
                .join(",")
        })
        .unwrap_or_else(|| "none".to_string())
}

/// Position of the blank line ending the next event, and that separator's length.
pub(super) fn find_event_end(buffer: &[u8]) -> Option<(usize, usize)> {
    let lf = buffer.windows(2).position(|w| w == b"\n\n").map(|i| (i, 2));
    let crlf = buffer
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| (i, 4));
    match (lf, crlf) {
        (Some(a), Some(b)) => Some(if a.0 <= b.0 { a } else { b }),
        (a, b) => a.or(b),
    }
}

#[cfg(test)]
pub(super) fn parse_stream_event(event: &[u8]) -> Result<Option<Value>> {
    match stream_event_payload(event) {
        Some(payload) => parse_stream_payload(&payload).map_err(|failure| failure.failure.into()),
        None => Ok(None),
    }
}

pub(super) fn stream_event_payload(event: &[u8]) -> Option<Value> {
    let text = String::from_utf8_lossy(event);
    let data = text
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(|line| line.strip_prefix(' ').unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n");
    if data.is_empty() || data == "[DONE]" {
        return None;
    }
    serde_json::from_str::<Value>(&data).ok()
}

/// The finished response for a terminal event, `None` for other events, or the
/// plan error a failed stream reports.
pub(super) fn parse_stream_payload(
    payload: &Value,
) -> std::result::Result<Option<Value>, ChatGptFailure> {
    let error = match payload["type"].as_str() {
        Some("response.completed" | "response.incomplete") => {
            return Ok(Some(payload["response"].clone()))
        }
        Some("response.failed") => &payload["response"]["error"],
        Some("error") if payload["error"].is_object() => &payload["error"],
        Some("error") => payload,
        _ => return Ok(None),
    };
    let code = error["code"].as_str();
    let mut failure = chatgpt_failure(stream_error_status(code), code);
    failure.param = error["param"].as_str().map(str::to_string);
    warn!(
        code = code.unwrap_or("-"),
        param = failure.param.as_deref().unwrap_or("-"),
        "ChatGPT plan stream failed"
    );
    Err(failure)
}

/// Errors after the stream opened arrive with HTTP 200; report their usual status.
pub(super) fn stream_error_status(code: Option<&str>) -> u16 {
    match code {
        Some("subscription_sharing_usage_limit_exceeded") => 429,
        Some(
            "subscription_sharing_usage_unavailable" | "subscription_sharing_user_unavailable",
        ) => 503,
        Some("subscription_sharing_unsupported_capability") => 400,
        Some("subscription_sharing_invalid_user") => 401,
        Some(_) => 403,
        None => 502,
    }
}
