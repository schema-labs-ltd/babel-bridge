use super::*;

#[tokio::test]
async fn topic_classification_reuses_configured_model_and_json_response_api() {
    use serde_json::{json, Value};
    let expected = r#"{"assignments":[{"messageId":"a","topic":"Weekend plans"}]}"#;
    let (url, requests, server) = spawn_capturing_openai_mock(vec![mock_text(expected)]);
    let service = TranslationService::new_with_api_url(url);
    service.set_runtime_settings(OpenAiSettings {
        model: Some("gpt-6-astra".into()),
        reasoning_effort: Some("low".into()),
    });
    let (result,_)=service.classify_topics(json!({"labelLanguage":"English","existingTopics":[],"messages":[{"messageId":"a","text":"Saturday picnic?"}]})).await.unwrap();
    assert_eq!(result, expected);
    server.join().unwrap();
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let body: Value = serde_json::from_str(requests[0].split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(body["model"], "gpt-6-astra");
    assert_eq!(body["reasoning"]["effort"], "low");
    assert_eq!(body["max_output_tokens"], 16384);
    assert_eq!(body["text"]["format"]["type"], "json_object");
    // Responses validates JSON mode against input messages, not the top-level instructions.
    assert!(
        body["input"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("json"),
        "JSON mode requires an explicit JSON instruction in the input"
    );
    assert!(body["instructions"]
        .as_str()
        .unwrap()
        .contains("untrusted data"));
    let instructions = body["instructions"].as_str().unwrap();
    assert!(instructions.contains("1-3 words"));
    assert!(instructions.contains("Hospital covers"));
    assert!(instructions.contains("broaden them"));
}

#[tokio::test]
async fn legacy_topic_simplification_uses_labels_only_and_validates_complete_mapping() {
    let output = r#"{"assignments":[{"messageId":"first","topic":"Hospital"},{"messageId":"second","topic":"Hospital"},{"messageId":"third","topic":"Shopping"}]}"#;
    let (url, requests, server) = spawn_capturing_openai_mock(vec![mock_text(output)]);
    let service = TranslationService::new_with_api_url(url);
    let (text,_)=service.simplify_topic_labels(serde_json::json!({"labelLanguage":"English","labels":[{"messageId":"first","title":"Hospital support"},{"messageId":"second","title":"Procedure delay and discharge plans"},{"messageId":"third","title":"Grocery shopping and meal requests"}]})).await.unwrap();
    let assignments =
        crate::topics::parse_title_assignments(&text, ["first", "second", "third"].into_iter())
            .unwrap();
    assert_eq!(
        assignments
            .iter()
            .map(|a| a.topic.as_str())
            .collect::<Vec<_>>(),
        ["Hospital", "Hospital", "Shopping"]
    );
    assert!(
        crate::topics::parse_title_assignments(&text, ["first", "second"].into_iter()).is_err()
    );
    server.join().unwrap();
    let requests = requests.lock().unwrap();
    let body: serde_json::Value =
        serde_json::from_str(requests[0].split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(body["text"]["format"]["type"], "json_object");
    assert!(body["input"].as_str().unwrap().contains("JSON"));
    assert_eq!(body["max_output_tokens"], 16384);
    let instructions = body["instructions"].as_str().unwrap();
    assert!(instructions.contains("untrusted data"));
    assert!(instructions.contains("1-3 words"));
    assert!(instructions.contains("to Hospital"));
    assert!(!body["input"].as_str().unwrap().contains("\"messages\""));
    assert!(!crate::topics::valid_title("A topic with too many words"));
    assert!(!crate::topics::valid_title(&"x".repeat(33)));
    assert!(!crate::topics::valid_title("Hospital\n"));
}

#[tokio::test]
async fn topic_batches_override_short_client_timeouts_and_leave_retries_to_the_queue() {
    use crate::translation::RequestPolicy;
    use reqwest::Client;
    use serde_json::json;
    use std::io::Write;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        read_http_request(&mut stream);
        thread::sleep(Duration::from_millis(40));
        let body = mock_text(r#"{"assignments":[]}"#).body;
        write!(stream, "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}", body.len(), body).unwrap();
    });
    let mut service = TranslationService::new_with_api_url(url);
    service.client = Client::builder()
        .timeout(Duration::from_millis(5))
        .build()
        .unwrap();
    assert_eq!(
        service
            .classify_topics(json!({"messages":[]}))
            .await
            .unwrap()
            .0,
        r#"{"assignments":[]}"#
    );
    server.join().unwrap();
    assert_eq!(
        RequestPolicy::Interactive.timeout(),
        Duration::from_secs(30)
    );
    assert_eq!(RequestPolicy::Interactive.attempts(), 3);
    assert_eq!(
        RequestPolicy::TopicBatch.timeout(),
        Duration::from_secs(110)
    );
    assert_eq!(RequestPolicy::TopicBatch.attempts(), 1);
    let (url, requests, server) = spawn_capturing_openai_mock(vec![MockResponse {
        status: "503 Unavailable",
        body: "{}".into(),
    }]);
    let service = TranslationService::new_with_api_url(url);
    assert!(service
        .classify_topics(json!({"messages":[]}))
        .await
        .is_err());
    server.join().unwrap();
    assert_eq!(requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn topic_api_rejections_keep_safe_diagnostics_without_private_response_text() {
    let (url, requests, server) = spawn_capturing_openai_mock(vec![MockResponse {
        status: "400 Bad Request",
        body: serde_json::json!({"error": {"message": "Response input messages must contain the word json. Private echoed message: secret picnic address", "type": "invalid_request_error"}}).to_string(),
    }]);
    let service = TranslationService::new_with_api_url(url);
    let error = service
        .classify_topics(serde_json::json!({"messages": []}))
        .await
        .unwrap_err();
    server.join().unwrap();
    assert_eq!(requests.lock().unwrap().len(), 1); // A rejected request must not be retried inside the AI client.
    let failure = error
        .downcast_ref::<crate::translation::OpenAiApiFailure>()
        .unwrap();
    assert_eq!(
        (failure.status, failure.reason),
        (400, "json_input_required")
    );
    assert!(!format!("{error:#?}").contains("secret picnic address"));
}

#[tokio::test]
async fn fake_ai_translates_locally_without_calling_openai() {
    // An unroutable URL: any real request would fail the test.
    let service = TranslationService::new_with_api_url("http://127.0.0.1:9/unused".to_string())
        .with_fake_ai();
    assert!(service.is_configured());
    let (translated, usage) = service
        .translate_to("¿Quedamos el sábado?", "English")
        .await
        .unwrap();
    assert_eq!(translated, "[Spanish → English] ¿Quedamos el sábado?");
    assert_eq!(usage.cost_usd, 0.0);
    let (unchanged, _) = service
        .translate_to("See you then", "English")
        .await
        .unwrap();
    assert_eq!(unchanged, "See you then");
    assert_eq!(
        service.source_language("Bom dia a todos").await.unwrap().0,
        "Portuguese"
    );
    assert!(service
        .classify_topics(serde_json::json!([]))
        .await
        .is_err());
    // Callers that build their own request, like style analysis, get the
    // placeholder from the shared transport instead of a network call.
    let (response, plan) = service
        .post_responses(
            serde_json::json!({}),
            crate::translation::RequestPolicy::Interactive,
        )
        .await
        .unwrap();
    assert!(!plan);
    assert_eq!(
        response["output"][0]["content"][0]["text"],
        crate::translation::FAKE_AI_TEXT
    );
}

#[tokio::test]
async fn astra_uses_low_reasoning_with_enough_output_budget() {
    let (url, requests, server) = spawn_capturing_openai_mock(vec![mock_text(
        r#"{"language":"English","isTargetLanguage":true}"#,
    )]);
    let service = TranslationService::new_with_api_url(url);
    service.set_runtime_settings(OpenAiSettings {
        model: Some("gpt-6-astra".into()),
        reasoning_effort: Some("none".into()),
    });
    service
        .process_text("Good morning", None, None)
        .await
        .unwrap();
    server.join().unwrap();
    let requests = requests.lock().unwrap();
    let (_, body) = requests[0].split_once("\r\n\r\n").unwrap();
    let body: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(body["model"], "gpt-6-astra");
    assert_eq!(body["reasoning"]["effort"], "low");
    assert!(body["max_output_tokens"].as_u64().unwrap() >= 8192);
}
