//! Voice-note translation. All network writes require a reviewed, durable preparation.
use crate::{bridge::BridgeCommand, storage::StoredMessage, web::AppState};
use anyhow::{bail, Context, Result};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    path::{Path as FsPath, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::process::Command;

mod audio;
mod notes;
mod sending;

pub use audio::*;
pub use notes::*;
pub use sending::*;

const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_SECONDS: usize = 180;
const RATE: usize = 16000;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceSettings {
    pub voice: String,
}
fn valid_voice(value: &str) -> bool {
    matches!(value, "auto" | "masculine" | "feminine" | "neutral")
}
fn failure(error: anyhow::Error) -> Response {
    tracing::warn!("Voice note operation failed: {error}");
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"error": error.to_string()})),
    )
        .into_response()
}
fn result_response(result: Result<Value>) -> Response {
    match result {
        Ok(value) => Json(value).into_response(),
        Err(error) => failure(error),
    }
}
pub async fn get_settings(
    State(state): State<Arc<AppState>>,
    Path(scope): Path<String>,
) -> Response {
    result_response(
        state
            .store
            .voice_setting(&scope)
            .map(|voice| json!({"voice": voice})),
    )
}
pub async fn put_settings(
    State(state): State<Arc<AppState>>,
    Path(scope): Path<String>,
    Json(settings): Json<VoiceSettings>,
) -> Response {
    if !valid_voice(&settings.voice) {
        return failure(anyhow::anyhow!(
            "Choose automatic, masculine, feminine or neutral."
        ));
    }
    result_response(
        state
            .store
            .set_voice_setting(&scope, &settings.voice)
            .map(|_| json!({"voice": settings.voice})),
    )
}

/// A short built-in voice sample; this never contacts WhatsApp.
pub async fn sample(
    State(state): State<Arc<AppState>>,
    Path(preference): Path<String>,
) -> Response {
    result_response(async {
        if !valid_voice(&preference) { bail!("Unknown voice preference."); }
        let key = format!("sample-v1-{preference}");
        if let Some((payload,_,_,_)) = state.store.voice_note(&key)? { return Ok(serde_json::from_str(&payload)?); }
        let _guard = state.voice_lock.try_lock().map_err(|_| anyhow::anyhow!("Another recording is processing. Try the preview again shortly."))?;
        let translator = state.translator().context("Voice translation is not configured on this server.")?;
        let epoch = state.voice_epoch.load(std::sync::atomic::Ordering::SeqCst);
        let voice = select_voice(&preference, &[]);
        let response = reqwest::Client::builder().timeout(Duration::from_secs(45)).build()?
            .post(translator.audio_api_url("speech")).bearer_auth(translator.audio_api_key()?)
            .json(&json!({"model":"gpt-4o-mini-tts","voice":voice,"input":"This is the voice I will use for translated messages.","response_format":"mp3"})).send().await?;
        let bytes = checked_bytes(response, 1024*1024).await?;
        let value = json!({"audioData":B64.encode(bytes),"mimeType":"audio/mpeg"});
        if epoch != state.voice_epoch.load(std::sync::atomic::Ordering::SeqCst) { bail!("Session changed. Please reconnect."); }
        state.store.save_voice_note(&key, &value.to_string())?;
        Ok(value)
    }.await)
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct VoiceNote {
    #[serde(default = "translated_by_default")]
    is_translated: bool,
    #[serde(default)]
    reply_only_if_not_latest: bool,
    id: String,
    #[serde(deserialize_with = "crate::identity::deserialize_contact_id")]
    contact_id: String,
    transcript: String,
    translation: String,
    target_language: String,
    source_language: String,
    voice: String,
    audio_data: String,
    original_data: String,
    duration_seconds: u32,
    original_duration_seconds: u32,
    send_audio: String,
    send_original: String,
    original_follow_up: bool,
    reply_to: Option<String>,
    reply_to_sender: Option<String>,
    reply_to_text: Option<String>,
}
fn translated_by_default() -> bool {
    true
}
impl VoiceNote {
    fn public(&self) -> Value {
        json!({"id": self.id, "contactId": self.contact_id, "transcript": self.transcript,
        "translation": self.translation, "targetLanguage": self.target_language, "voice": self.voice,
        "audioData": self.audio_data, "originalData": self.original_data, "mimeType": "audio/mpeg",
        "durationSeconds": self.duration_seconds, "originalFollowUp": self.original_follow_up,
        "isTranslated": self.is_translated})
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareRequest {
    #[serde(default)]
    reply_only_if_not_latest: bool,
    #[serde(deserialize_with = "crate::identity::deserialize_contact_id")]
    contact_id: String,
    media_data: String,
    reply_to: Option<String>,
    reply_to_sender: Option<String>,
    reply_to_text: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendRequest {
    preparation_id: String,
}

// No filenames or network URLs supplied by a client are passed to ffmpeg.
struct Scratch(PathBuf);
impl Scratch {
    fn new(root: &FsPath) -> Result<Self> {
        let path = root.join(format!("voice-tmp-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(Self(path))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod integration_tests;
