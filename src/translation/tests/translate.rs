use super::*;

#[test]
fn runtime_settings_override_model_and_reasoning() {
    let service = TranslationService::new_with_api_url("http://example.test".to_string());
    service.set_runtime_settings(OpenAiSettings {
        model: Some("gpt-5.6-sol".to_string()),
        reasoning_effort: Some("xhigh".to_string()),
    });

    assert_eq!(service.get_detection_model(), "gpt-5.6-sol");
    assert_eq!(service.get_reasoning_effort().as_deref(), Some("xhigh"));
}

#[tokio::test]
async fn outgoing_foreign_text_translates_even_when_target_is_owner_language() {
    let (url, server) = spawn_openai_mock(vec![
        mock_text(r#"{"language":"Hungarian","isTargetLanguage":false}"#),
        mock_text("Good morning"),
    ]);
    let service = TranslationService::new_with_api_url(url);
    assert_eq!(
        service
            .translate_outgoing("Jó reggelt", "English")
            .await
            .unwrap()
            .0,
        "Good morning"
    );
    server.join().unwrap();
}

#[tokio::test]
async fn invalid_detection_is_an_error_and_already_english_is_unchanged() {
    let (url, server) = spawn_openai_mock(vec![
        mock_text("invalid JSON"),
        mock_text(r#"{"language":"English","isTargetLanguage":true}"#),
    ]);
    let service = TranslationService::new_with_api_url(url);
    assert!(service
        .process_text("Jó reggelt", None, None)
        .await
        .is_err());
    let result = service
        .process_text("Good morning", None, None)
        .await
        .unwrap();
    assert!(!result.needs_translation);
    assert!(result.translated_text.is_none());
    server.join().unwrap();
}

#[tokio::test]
async fn incomplete_response_cannot_be_saved_as_a_translation() {
    let mut response = mock_text(r#"{"language":"English","isTargetLanguage":true}"#);
    response.body = response.body.replace("completed", "incomplete");
    let (url, server) = spawn_openai_mock(vec![response]);
    assert!(TranslationService::new_with_api_url(url)
        .process_text("Hello", None, None)
        .await
        .is_err());
    server.join().unwrap();
}

#[test]
fn calculates_cost_with_cached_input_tokens() {
    let usage = crate::translation::ApiUsage {
        input_tokens: 1_000_000,
        output_tokens: 1_000_000,
        input_tokens_details: Some(crate::translation::InputTokensDetails {
            cached_tokens: 250_000,
        }),
    };

    let priced = TranslationService::usage_from_api(Some(usage), CHEAP_PRICING);

    assert_eq!(priced.input_tokens, 1_000_000);
    assert_eq!(priced.cached_input_tokens, 250_000);
    assert_eq!(priced.output_tokens, 1_000_000);
    assert!((priced.cost_usd - 0.7025).abs() < 0.000001);
}

#[test]
fn extracts_json_object_from_wrapped_text() {
    let wrapped = "```json\n{\"language\":\"Spanish\",\"isTargetLanguage\":false}\n```";
    assert_eq!(
        TranslationService::extract_json_object(wrapped),
        Some("{\"language\":\"Spanish\",\"isTargetLanguage\":false}")
    );
}

#[test]
fn builds_data_url_from_raw_base64() {
    assert_eq!(
        TranslationService::build_data_url("image/jpeg", "abc123"),
        "data:image/jpeg;base64,abc123"
    );
}

#[test]
fn preserves_existing_data_url() {
    let data_url = "data:image/png;base64,abc123";
    assert_eq!(
        TranslationService::build_data_url("image/png", data_url),
        data_url
    );
}

#[test]
fn combines_cached_usage() {
    let combined = TranslationService::combine_usage(
        &UsageInfo {
            input_tokens: 10,
            cached_input_tokens: 3,
            output_tokens: 4,
            cost_usd: 1.0,
        },
        &UsageInfo {
            input_tokens: 8,
            cached_input_tokens: 2,
            output_tokens: 5,
            cost_usd: 2.0,
        },
    );

    assert_eq!(combined.input_tokens, 18);
    assert_eq!(combined.cached_input_tokens, 5);
    assert_eq!(combined.output_tokens, 9);
    assert_eq!(combined.cost_usd, 3.0);
}

#[tokio::test]
async fn process_text_returns_error_when_translation_request_fails() {
    let detection_response = r#"{
        "output": [{
            "type": "message",
            "content": [{
                "type": "output_text",
                "text": "{\"language\":\"Spanish\",\"isTargetLanguage\":false}"
            }]
        }],
        "usage": {
            "input_tokens": 12,
            "output_tokens": 4,
            "input_tokens_details": { "cached_tokens": 2 }
        }
    }"#;
    let (api_url, server) = spawn_openai_mock(vec![
        MockResponse {
            status: "200 OK",
            body: detection_response.to_string(),
        },
        MockResponse {
            status: "500 Internal Server Error",
            body: r#"{"error":"translation unavailable"}"#.to_string(),
        },
        MockResponse {
            status: "500 Internal Server Error",
            body: r#"{"error":"translation still unavailable"}"#.to_string(),
        },
        MockResponse {
            status: "500 Internal Server Error",
            body: r#"{"error":"translation failed"}"#.to_string(),
        },
    ]);
    let service = TranslationService::new_with_api_url(api_url);

    let error = service
        .process_text("hola, podemos hablar mañana?", None, None)
        .await
        .expect_err("translation failure should be returned");

    assert!(
        error.to_string().contains("Translation failed"),
        "unexpected error: {error:#}"
    );
    server.join().expect("mock server should finish");
}

#[tokio::test]
async fn configured_contact_language_translates_incoming_text_back_to_default_language() {
    let detection_response = r#"{
        "output": [{
            "type": "message",
            "content": [{
                "type": "output_text",
                "text": "{\"language\":\"Hungarian\",\"isTargetLanguage\":false}"
            }]
        }],
        "usage": { "input_tokens": 8, "output_tokens": 3 }
    }"#;
    let translation_response = r#"{
        "output": [{
            "type": "message",
            "content": [{ "type": "output_text", "text": "Good morning" }]
        }],
        "usage": { "input_tokens": 10, "output_tokens": 2 }
    }"#;
    let (api_url, requests, server) = spawn_capturing_openai_mock(vec![
        MockResponse {
            status: "200 OK",
            body: detection_response.to_string(),
        },
        MockResponse {
            status: "200 OK",
            body: translation_response.to_string(),
        },
    ]);
    let service = TranslationService::new_with_api_url(api_url);

    let result = service
        .process_text("Jó reggelt", Some("Hungarian"), None)
        .await
        .expect("incoming Hungarian message should translate");

    assert!(result.needs_translation);
    assert_eq!(result.translated_text.as_deref(), Some("Good morning"));
    let requests = requests.lock().expect("lock captured requests");
    assert!(requests[0].contains("primarily in English"));
    assert!(requests[1].contains("Hungarian to English"));
    server.join().expect("mock server should finish");
}

#[tokio::test]
async fn short_outgoing_text_is_still_translated_to_configured_contact_language() {
    let detection_response = r#"{
        "output": [{
            "type": "message",
            "content": [{
                "type": "output_text",
                "text": "{\"language\":\"English\",\"isTargetLanguage\":false}"
            }]
        }],
        "usage": { "input_tokens": 6, "output_tokens": 3 }
    }"#;
    let translation_response = r#"{
        "output": [{
            "type": "message",
            "content": [{ "type": "output_text", "text": "Szia" }]
        }],
        "usage": { "input_tokens": 7, "output_tokens": 2 }
    }"#;
    let (api_url, requests, server) = spawn_capturing_openai_mock(vec![
        MockResponse {
            status: "200 OK",
            body: detection_response.to_string(),
        },
        MockResponse {
            status: "200 OK",
            body: translation_response.to_string(),
        },
    ]);
    let service = TranslationService::new_with_api_url(api_url);

    let (translated, _) = service
        .translate_outgoing("Hi", "Hungarian")
        .await
        .expect("short outgoing English message should translate");

    assert_eq!(translated, "Szia");
    let requests = requests.lock().expect("lock captured requests");
    assert!(requests[0].contains("primarily in Hungarian"));
    assert!(requests[1].contains("English to Hungarian"));
    server.join().expect("mock server should finish");
}
