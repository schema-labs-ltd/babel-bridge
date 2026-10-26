use super::*;

#[test]
fn chatgpt_body_streams_without_storage_or_unsupported_fields() {
    let body = chatgpt_request_body(
        serde_json::json!({
            "model": "gpt-6-astra",
            "instructions": "Translate",
            "input": "Hola",
            "max_output_tokens": 8192,
            "temperature": 0.2,
            "reasoning": {"effort": "low"},
            "text": {"format": {"type": "json_object"}},
        }),
        Some("gpt-6.1-sol"),
    );
    assert_eq!(body["model"], "gpt-6.1-sol");
    assert_eq!(body["store"], false);
    assert_eq!(body["stream"], true);
    assert!(body.get("max_output_tokens").is_none());
    assert!(body.get("temperature").is_none());
    assert_eq!(body["input"][0]["role"], "user");
    assert_eq!(body["input"][0]["content"], "Hola");
    assert_eq!(body["reasoning"]["effort"], "low");
    let body = chatgpt_request_body(
        serde_json::json!({"input": [{"role": "system", "content": "x"}]}),
        None,
    );
    assert_eq!(body["input"][0]["role"], "developer");
    assert!(body.get("model").is_none());
}

#[test]
fn stream_events_finish_only_on_a_terminal_event() {
    assert!(parse_stream_event(
        b"event: x\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"Hi\"}"
    )
    .unwrap()
    .is_none());
    let done = parse_stream_event(
        b"data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}",
    )
    .unwrap()
    .unwrap();
    assert_eq!(done["status"], "completed");
    let error = parse_stream_event(
        b"data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"subscription_sharing_usage_limit_exceeded\"}}}",
    )
    .unwrap_err();
    let failure = error.downcast_ref::<OpenAiApiFailure>().unwrap();
    assert_eq!(
        (failure.status, failure.reason),
        (429, "chatgpt_usage_limit")
    );
    assert_eq!(find_event_end(b"a\r\n\r\nb"), Some((1, 4)));
}

#[tokio::test]
async fn signed_in_chatgpt_uses_plan_token_model_and_costs_nothing() {
    let completed = serde_json::json!({
        "type": "response.completed",
        "response": {
            "status": "completed",
            "output": [{"type": "message", "content": [{"type": "output_text", "text": "Good morning"}]}],
            "usage": {"input_tokens": 1000, "output_tokens": 1000},
        },
    });
    let detect = serde_json::json!({
        "type": "response.completed",
        "response": {
            "status": "completed",
            "output": [{"type": "message", "content": [{"type": "output_text", "text": "{\"language\":\"Spanish\",\"isTargetLanguage\":false}"}]}],
            "usage": {"input_tokens": 1000, "output_tokens": 1000},
        },
    });
    let (url, requests, server) = spawn_capturing_openai_mock(vec![
        MockResponse {
            status: "200 OK",
            body: sse(&[serde_json::json!({"type":"response.created"}), detect]),
        },
        MockResponse {
            status: "200 OK",
            body: sse(&[completed]),
        },
    ]);
    let (service, dir) = chatgpt_service(url, &["gpt-6.1-sol"]);
    let result = service
        .process_text("Buenos días", None, None)
        .await
        .unwrap();
    server.join().unwrap();
    assert_eq!(result.translated_text.as_deref(), Some("Good morning"));
    assert_eq!(result.usage.cost_usd, 0.0);
    assert!(result.usage.input_tokens > 0);
    let requests = requests.lock().unwrap();
    for request in requests.iter() {
        assert!(request
            .to_lowercase()
            .contains("authorization: bearer plan-token"));
        let (_, body) = request.split_once("\r\n\r\n").unwrap();
        let body: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(body["model"], "gpt-6.1-sol");
        assert_eq!(body["stream"], true);
        assert_eq!(body["store"], false);
        assert!(body.get("max_output_tokens").is_none());
    }
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn chatgpt_usage_limit_is_reported_without_retrying_or_using_the_api_key() {
    let (url, requests, server) = spawn_capturing_openai_mock(vec![MockResponse {
        status: "429 Too Many Requests",
        body: serde_json::json!({"error": {"code": "subscription_sharing_usage_limit_exceeded"}})
            .to_string(),
    }]);
    let (service, dir) = chatgpt_service(url, &[]);
    let error = service.source_language("Hola").await.unwrap_err();
    server.join().unwrap();
    let failure = error.downcast_ref::<OpenAiApiFailure>().unwrap();
    assert_eq!(failure.reason, "chatgpt_usage_limit");
    assert_eq!(requests.lock().unwrap().len(), 1);
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn chatgpt_drops_a_rejected_field_and_resends_once() {
    let detect = serde_json::json!({
        "type": "response.completed",
        "response": {
            "status": "completed",
            "output": [{"type": "message", "content": [{"type": "output_text", "text": "{\"language\":\"Spanish\",\"isTargetLanguage\":false}"}]}],
        },
    });
    let (url, requests, server) = spawn_capturing_openai_mock(vec![
        MockResponse {
            status: "400 Bad Request",
            body: serde_json::json!({"error": {"code": "subscription_sharing_unsupported_capability", "param": "reasoning.effort"}}).to_string(),
        },
        MockResponse { status: "200 OK", body: sse(std::slice::from_ref(&detect)) },
        MockResponse { status: "200 OK", body: sse(&[detect]) },
    ]);
    let (service, dir) = chatgpt_service(url, &["gpt-6.1-sol"]);
    assert_eq!(service.source_language("Hola").await.unwrap().0, "Spanish");
    // Later requests leave the field out from the start.
    assert_eq!(service.source_language("Hola").await.unwrap().0, "Spanish");
    server.join().unwrap();
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    let body = |i: usize| -> serde_json::Value {
        serde_json::from_str(requests[i].split_once("\r\n\r\n").unwrap().1).unwrap()
    };
    assert!(body(0)["reasoning"]["effort"].is_string());
    assert!(body(1).get("reasoning").is_none());
    assert!(body(2).get("reasoning").is_none());
    assert_eq!(body(2)["text"]["format"]["type"], "json_object");
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn chatgpt_output_comes_from_stream_events_when_completed_omits_it() {
    let events = [
        serde_json::json!({"type": "response.output_text.delta", "delta": "{\"language\":\"French\","}),
        serde_json::json!({"type": "response.output_text.delta", "delta": "\"isTargetLanguage\":false}"}),
        serde_json::json!({"type": "response.completed", "response": {"status": "completed", "output": []}}),
    ];
    let (url, _requests, server) = spawn_capturing_openai_mock(vec![MockResponse {
        status: "200 OK",
        body: sse(&events),
    }]);
    let (service, dir) = chatgpt_service(url, &["gpt-6.1-sol"]);
    assert_eq!(
        service.source_language("Bonjour").await.unwrap().0,
        "French"
    );
    server.join().unwrap();
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn chatgpt_output_comes_from_stream_events_when_completed_has_only_reasoning() {
    let message = serde_json::json!({"type": "message", "content": [{"type": "output_text", "text": "{\"language\":\"German\",\"isTargetLanguage\":false}"}]});
    let events = [
        serde_json::json!({"type": "response.output_item.done", "item": {"type": "reasoning", "summary": []}}),
        serde_json::json!({"type": "response.output_item.done", "item": message}),
        serde_json::json!({"type": "response.completed", "response": {"status": "completed", "output": [{"type": "reasoning", "summary": []}]}}),
    ];
    let (url, _requests, server) = spawn_capturing_openai_mock(vec![MockResponse {
        status: "200 OK",
        body: sse(&events),
    }]);
    let (service, dir) = chatgpt_service(url, &["gpt-6.1-sol"]);
    assert_eq!(service.source_language("Hallo").await.unwrap().0, "German");
    server.join().unwrap();
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn chatgpt_output_text_done_is_used_when_nothing_else_carries_text() {
    let events = [
        serde_json::json!({"type": "response.output_text.done", "text": "{\"language\":\"Italian\",\"isTargetLanguage\":false}"}),
        serde_json::json!({"type": "response.completed", "response": {"status": "completed"}}),
    ];
    let (url, _requests, server) = spawn_capturing_openai_mock(vec![MockResponse {
        status: "200 OK",
        body: sse(&events),
    }]);
    let (service, dir) = chatgpt_service(url, &["gpt-6.1-sol"]);
    assert_eq!(service.source_language("Ciao").await.unwrap().0, "Italian");
    server.join().unwrap();
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn output_shape_names_types_without_text() {
    let output = serde_json::json!([{"type": "reasoning"}, {"type": "message", "content": [{"type": "refusal", "refusal": "secret"}]}]);
    assert_eq!(
        crate::translation::output_shape(&output),
        "reasoning,message[refusal]"
    );
    assert!(!crate::translation::has_output_text(&output));
}

#[test]
fn only_optional_fields_are_dropped_for_unsupported_capabilities() {
    assert_eq!(
        crate::translation::droppable_field("reasoning.effort"),
        Some("reasoning.effort")
    );
    assert_eq!(
        crate::translation::droppable_field("text.format.type"),
        Some("text.format")
    );
    assert_eq!(crate::translation::droppable_field("model"), None);
    assert_eq!(crate::translation::droppable_field("input"), None);
    let mut body = serde_json::json!({"text": {"format": {"type": "json_object"}}, "model": "m"});
    crate::translation::remove_field(&mut body, "text.format");
    assert_eq!(body, serde_json::json!({"model": "m"}));
}

#[test]
fn audio_needs_the_api_key_even_when_signed_in() {
    let dir = std::env::temp_dir().join(format!("wa-chatgpt-audio-{}", uuid::Uuid::new_v4()));
    let auth = crate::chatgpt::ChatGptAuth::signed_in_for_test(&dir, "t", &[]);
    let service =
        TranslationService::new(None, "a".into(), "b".into(), "c".into(), "English".into())
            .with_chatgpt(Arc::new(auth));
    assert!(service.is_configured());
    assert!(!service.has_audio());
    assert!(service.audio_api_key().is_err());
    std::fs::remove_dir_all(dir).ok();
}
