//! Translation service using the OpenAI Responses API.

use anyhow::{Context, Result};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, info, warn};

mod compose;
mod requests;
mod translate;
mod transport;

use transport::*;

const OPENAI_API_URL: &str = "https://api.openai.com/v1/responses";
const OPENAI_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const OPENAI_MAX_ATTEMPTS: usize = 3;
/// What WA_FAKE_AI returns for any free-text request.
const FAKE_AI_TEXT: &str = "[fake AI] This is placeholder text.";
const TOPIC_MAX_OUTPUT_TOKENS: u32 = 16384;

#[derive(Clone, Copy)]
pub(crate) enum RequestPolicy {
    Interactive,
    TopicBatch,
}

impl RequestPolicy {
    fn timeout(self) -> Duration {
        match self {
            Self::Interactive => OPENAI_REQUEST_TIMEOUT,
            Self::TopicBatch => Duration::from_secs(110),
        }
    }

    fn attempts(self) -> usize {
        match self {
            Self::Interactive => OPENAI_MAX_ATTEMPTS,
            // Topic jobs already have durable, bounded retries. Let one request
            // finish instead of repeatedly abandoning generation after 30 seconds.
            Self::TopicBatch => 1,
        }
    }
}

/// Safe upstream diagnostics: never retain response text that could echo private input.
#[derive(Debug)]
pub(crate) struct OpenAiApiFailure {
    pub status: u16,
    pub reason: &'static str,
}

impl OpenAiApiFailure {
    fn from_response(status: reqwest::StatusCode, body: &str) -> Self {
        let body: Value = serde_json::from_str(body).unwrap_or(Value::Null);
        let message = body["error"]["message"]
            .as_str()
            .unwrap_or("")
            .to_lowercase();
        let reason = if status == reqwest::StatusCode::BAD_REQUEST
            && message.contains("input messages must contain")
            && message.contains("json")
        {
            "json_input_required"
        } else {
            match status.as_u16() {
                400 | 422 => "invalid_request",
                401 => "authentication_failed",
                403 => "permission_denied",
                404 => "model_or_endpoint_unavailable",
                429 => "rate_limit_or_quota",
                500..=599 => "upstream_unavailable",
                _ => "request_rejected",
            }
        };
        Self {
            status: status.as_u16(),
            reason,
        }
    }
}

impl std::fmt::Display for OpenAiApiFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "OpenAI Responses API error: HTTP {} ({})",
            self.status, self.reason
        )
    }
}
impl std::error::Error for OpenAiApiFailure {}

#[derive(Debug)]
pub(crate) struct OpenAiOutputFailure(pub &'static str);
impl std::fmt::Display for OpenAiOutputFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for OpenAiOutputFailure {}

const GPT_5_4_INPUT_COST_PER_M: f64 = 2.50;
const GPT_5_4_CACHED_INPUT_COST_PER_M: f64 = 0.25;
const GPT_5_4_OUTPUT_COST_PER_M: f64 = 15.00;

const GPT_5_4_MINI_INPUT_COST_PER_M: f64 = 0.75;
const GPT_5_4_MINI_CACHED_INPUT_COST_PER_M: f64 = 0.075;
const GPT_5_4_MINI_OUTPUT_COST_PER_M: f64 = 4.50;

const GPT_5_4_NANO_INPUT_COST_PER_M: f64 = 0.10;
const GPT_5_4_NANO_CACHED_INPUT_COST_PER_M: f64 = 0.01;
const GPT_5_4_NANO_OUTPUT_COST_PER_M: f64 = 0.625;

const GPT_5_6_SOL_PRICING: PricingTier = PricingTier {
    input_cost_per_m: 5.0,
    cached_input_cost_per_m: 0.5,
    output_cost_per_m: 30.0,
};
const GPT_5_6_TERRA_PRICING: PricingTier = PricingTier {
    input_cost_per_m: 2.5,
    cached_input_cost_per_m: 0.25,
    output_cost_per_m: 15.0,
};
const GPT_5_6_LUNA_PRICING: PricingTier = PricingTier {
    input_cost_per_m: 1.0,
    cached_input_cost_per_m: 0.1,
    output_cost_per_m: 6.0,
};

#[derive(Clone, Copy)]
struct PricingTier {
    input_cost_per_m: f64,
    cached_input_cost_per_m: f64,
    output_cost_per_m: f64,
}

const HIGH_END_PRICING: PricingTier = PricingTier {
    input_cost_per_m: GPT_5_4_INPUT_COST_PER_M,
    cached_input_cost_per_m: GPT_5_4_CACHED_INPUT_COST_PER_M,
    output_cost_per_m: GPT_5_4_OUTPUT_COST_PER_M,
};

const TRANSLATION_PRICING: PricingTier = PricingTier {
    input_cost_per_m: GPT_5_4_MINI_INPUT_COST_PER_M,
    cached_input_cost_per_m: GPT_5_4_MINI_CACHED_INPUT_COST_PER_M,
    output_cost_per_m: GPT_5_4_MINI_OUTPUT_COST_PER_M,
};

const CHEAP_PRICING: PricingTier = PricingTier {
    input_cost_per_m: GPT_5_4_NANO_INPUT_COST_PER_M,
    cached_input_cost_per_m: GPT_5_4_NANO_CACHED_INPUT_COST_PER_M,
    output_cost_per_m: GPT_5_4_NANO_OUTPUT_COST_PER_M,
};

/// Translation service for processing messages and AI replies.
pub struct TranslationService {
    client: Client,
    api_url: String,
    api_key: Option<String>,
    /// When signed in, text AI uses the ChatGPT plan instead of the API key.
    chatgpt: Option<Arc<crate::chatgpt::ChatGptAuth>>,
    /// Fields the plan route rejected, left out of later plan requests.
    chatgpt_dropped_fields: RwLock<std::collections::BTreeSet<&'static str>>,
    detection_model: String,
    translation_model: String,
    high_end_model: String,
    default_language: String,
    runtime_settings: RwLock<crate::storage::OpenAiSettings>,
    /// Answer text requests locally (`WA_FAKE_AI`), for runs without OpenAI.
    fake_ai: bool,
}

/// Result of processing a message for translation.
#[derive(Debug, Clone)]
pub struct TranslationResult {
    pub needs_translation: bool,
    pub original_text: String,
    pub translated_text: Option<String>,
    pub source_language: String,
    pub usage: UsageInfo,
}

/// Token usage and cost information.
#[derive(Debug, Clone, Default)]
pub struct UsageInfo {
    pub input_tokens: u32,
    pub cached_input_tokens: u32,
    pub output_tokens: u32,
    pub cost_usd: f64,
}

#[derive(Deserialize)]
struct OpenAiResponse {
    #[serde(default)]
    output: Vec<OpenAiOutputItem>,
    status: Option<String>,
    usage: Option<ApiUsage>,
    incomplete_details: Option<IncompleteDetails>,
}

#[derive(Deserialize)]
struct IncompleteDetails {
    reason: Option<String>,
}

#[derive(Deserialize)]
struct OpenAiOutputItem {
    #[serde(rename = "type")]
    item_type: String,
    content: Option<Vec<OpenAiContentPart>>,
}

#[derive(Deserialize)]
struct OpenAiContentPart {
    #[serde(rename = "type")]
    part_type: String,
    text: Option<String>,
}

#[derive(Deserialize, Debug, Clone, Copy, Default)]
struct ApiUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
    input_tokens_details: Option<InputTokensDetails>,
}

#[derive(Deserialize, Debug, Clone, Copy, Default)]
struct InputTokensDetails {
    #[serde(default)]
    cached_tokens: u32,
}

#[derive(Deserialize)]
struct LanguageDetection {
    language: String,
    #[serde(rename = "isTargetLanguage", alias = "is_target_language")]
    is_target_language: bool,
}

impl TranslationService {
    pub fn default_language(&self) -> &str {
        &self.default_language
    }

    pub fn audio_api_url(&self, endpoint: &str) -> String {
        format!(
            "{}/audio/{endpoint}",
            self.api_url.trim_end_matches("/responses")
        )
    }

    /// Audio transcripts may be in any language, including when the target is the app default.
    pub async fn translate_voice(
        &self,
        text: &str,
        target: &str,
        style: Option<&str>,
    ) -> Result<TranslationResult> {
        let (already_target, source, detection_usage) = self.detect_language(text, target).await?;
        let (translated, usage) = if already_target || source.eq_ignore_ascii_case(target) {
            (text.to_string(), detection_usage)
        } else {
            let (translated, usage) = self.translate(text, &source, target, style).await?;
            (translated, Self::combine_usage(&detection_usage, &usage))
        };
        if translated.trim().is_empty() {
            anyhow::bail!("Voice translation returned no text");
        }
        Ok(TranslationResult {
            original_text: text.to_string(),
            needs_translation: translated != text,
            translated_text: Some(translated),
            source_language: source,
            usage,
        })
    }

    pub fn new(
        api_key: Option<String>,
        detection_model: String,
        translation_model: String,
        high_end_model: String,
        default_language: String,
    ) -> Self {
        info!(
            "Translation service initialized with OpenAI models (target: {})",
            default_language
        );
        Self {
            client: Self::build_http_client(),
            api_url: OPENAI_API_URL.to_string(),
            api_key,
            chatgpt: None,
            chatgpt_dropped_fields: RwLock::default(),
            detection_model,
            translation_model,
            high_end_model,
            default_language,
            runtime_settings: RwLock::new(Default::default()),
            fake_ai: false,
        }
    }

    #[cfg(test)]
    pub(crate) fn new_with_api_url(api_url: String) -> Self {
        Self {
            client: Self::build_http_client(),
            api_url,
            api_key: Some("test-api-key".to_string()),
            chatgpt: None,
            chatgpt_dropped_fields: RwLock::default(),
            detection_model: "test-detect".to_string(),
            translation_model: "test-translate".to_string(),
            high_end_model: "test-high-end".to_string(),
            default_language: "English".to_string(),
            runtime_settings: RwLock::new(Default::default()),
            fake_ai: false,
        }
    }

    fn build_http_client() -> Client {
        Client::builder()
            .timeout(OPENAI_REQUEST_TIMEOUT)
            .build()
            .expect("OpenAI HTTP client should build")
    }

    /// Replace text AI with local placeholder output: detection guesses from the
    /// characters used, translations are tagged copies, and other text requests
    /// return a fixed reply. Audio still needs a real key.
    pub fn with_fake_ai(mut self) -> Self {
        self.fake_ai = true;
        self
    }

    fn fake_language(text: &str) -> &'static str {
        let lower = text.to_lowercase();
        if lower.contains(['ã', 'õ', 'ç']) || lower.contains("bom dia") || lower.contains(" você")
        {
            "Portuguese"
        } else if lower.contains(['ñ', '¿', '¡', 'á', 'é', 'í', 'ó', 'ú']) {
            "Spanish"
        } else {
            "English"
        }
    }

    pub fn with_chatgpt(mut self, auth: Arc<crate::chatgpt::ChatGptAuth>) -> Self {
        self.chatgpt = Some(auth);
        self
    }

    pub fn chatgpt(&self) -> Option<&Arc<crate::chatgpt::ChatGptAuth>> {
        self.chatgpt.as_ref()
    }

    fn chatgpt_signed_in(&self) -> bool {
        self.chatgpt
            .as_ref()
            .is_some_and(|auth| auth.is_signed_in())
    }

    /// Text AI can run: an API key is set or someone signed in with ChatGPT.
    pub fn is_configured(&self) -> bool {
        self.fake_ai || self.api_key.is_some() || self.chatgpt_signed_in()
    }

    /// ChatGPT plan usage covers the Responses API only, so audio needs the key.
    pub fn audio_api_key(&self) -> Result<&str> {
        self.api_key.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "Voice notes need an OpenAI API key. Sign in with ChatGPT covers text AI only."
            )
        })
    }

    pub fn has_audio(&self) -> bool {
        self.api_key.is_some()
    }

    pub fn get_detection_model(&self) -> String {
        self.runtime_settings
            .read()
            .unwrap()
            .model
            .clone()
            .unwrap_or_else(|| self.detection_model.clone())
    }

    pub fn get_reasoning_effort(&self) -> Option<String> {
        self.runtime_settings
            .read()
            .unwrap()
            .reasoning_effort
            .clone()
    }

    pub fn set_runtime_settings(&self, settings: crate::storage::OpenAiSettings) {
        *self.runtime_settings.write().unwrap() = settings;
    }

    fn pricing_for_model(model: &str, fallback: PricingTier) -> PricingTier {
        match model {
            "gpt-6-astra" => PricingTier {
                input_cost_per_m: 10.0,
                cached_input_cost_per_m: 1.0,
                output_cost_per_m: 50.0,
            },
            "gpt-5.6-sol" => GPT_5_6_SOL_PRICING,
            "gpt-5.6-terra" => GPT_5_6_TERRA_PRICING,
            "gpt-5.6-luna" => GPT_5_6_LUNA_PRICING,
            _ => fallback,
        }
    }

    fn usage_from_api(usage: Option<ApiUsage>, pricing: PricingTier) -> UsageInfo {
        let usage = usage.unwrap_or_default();
        let cached_input_tokens = usage
            .input_tokens_details
            .unwrap_or_default()
            .cached_tokens
            .min(usage.input_tokens);
        let uncached_input_tokens = usage.input_tokens.saturating_sub(cached_input_tokens);

        let input_cost = (uncached_input_tokens as f64 / 1_000_000.0) * pricing.input_cost_per_m;
        let cached_input_cost =
            (cached_input_tokens as f64 / 1_000_000.0) * pricing.cached_input_cost_per_m;
        let output_cost = (usage.output_tokens as f64 / 1_000_000.0) * pricing.output_cost_per_m;

        UsageInfo {
            input_tokens: usage.input_tokens,
            cached_input_tokens,
            output_tokens: usage.output_tokens,
            cost_usd: input_cost + cached_input_cost + output_cost,
        }
    }

    fn combine_usage(a: &UsageInfo, b: &UsageInfo) -> UsageInfo {
        UsageInfo {
            input_tokens: a.input_tokens + b.input_tokens,
            cached_input_tokens: a.cached_input_tokens + b.cached_input_tokens,
            output_tokens: a.output_tokens + b.output_tokens,
            cost_usd: a.cost_usd + b.cost_usd,
        }
    }

    fn extract_output_text(response: &OpenAiResponse) -> String {
        response
            .output
            .iter()
            .filter(|item| item.item_type == "message")
            .filter_map(|item| item.content.as_ref())
            .flat_map(|content| content.iter())
            .filter(|part| part.part_type == "output_text")
            .filter_map(|part| part.text.as_deref())
            .collect::<Vec<_>>()
            .join("")
    }

    fn extract_json_object(text: &str) -> Option<&str> {
        let start = text.find('{')?;
        let end = text.rfind('}')?;
        Some(&text[start..=end])
    }

    fn truncate_for_display(text: String, max_len: usize) -> String {
        if text.chars().count() > max_len {
            let truncated: String = text.chars().take(max_len.saturating_sub(3)).collect();
            format!("{}...", truncated)
        } else {
            text
        }
    }

    fn build_data_url(media_type: &str, base64_data: &str) -> String {
        if base64_data.starts_with("data:") {
            base64_data.to_string()
        } else {
            format!("data:{};base64,{}", media_type, base64_data)
        }
    }
}

#[cfg(test)]
mod tests;
