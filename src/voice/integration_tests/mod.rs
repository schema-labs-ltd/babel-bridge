use super::*;
use crate::{
    storage::{ConversationSettings, MessageStore},
    translation::TranslationService,
    web::{create_router, BridgeSendResult},
};
use axum::{
    body::{to_bytes, Body},
    http::Request,
    routing::post,
    Router,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use tower::ServiceExt;

struct TestDirectory(PathBuf);
impl TestDirectory {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("voice-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn state(root: &FsPath, translator: Option<Arc<TranslationService>>) -> Arc<AppState> {
    let store = MessageStore::new(root).unwrap();
    store
        .upsert_contact(
            "test@s.whatsapp.net",
            Some("Synthetic test"),
            None,
            Some("private"),
            1,
        )
        .unwrap();
    store
        .update_conversation_settings(
            "test@s.whatsapp.net",
            &ConversationSettings {
                reply_only: false,
                translation_enabled: true,
                language_override: Some("Hungarian".into()),
                translation_style: Some("friendly".into()),
                send_original_follow_up: true,
            },
        )
        .unwrap();
    AppState::new(
        store,
        PathBuf::from("web/public"),
        root.to_path_buf(),
        translator,
        Some("test-password".into()),
        None,
    )
}
fn note(id: String) -> VoiceNote {
    VoiceNote {
        is_translated: true,
        id,
        contact_id: "test@s.whatsapp.net".into(),
        transcript: "Good morning".into(),
        translation: "Jó reggelt".into(),
        target_language: "Hungarian".into(),
        source_language: "English".into(),
        voice: "onyx".into(),
        audio_data: B64.encode("mp3 test"),
        original_data: B64.encode("original"),
        duration_seconds: 2,
        original_duration_seconds: 2,
        send_audio: B64.encode("OggS translated fixture"),
        send_original: B64.encode("OggS original fixture"),
        original_follow_up: true,
        reply_only_if_not_latest: false,
        reply_to: None,
        reply_to_sender: None,
        reply_to_text: None,
    }
}
async fn json_response(response: Response) -> Value {
    serde_json::from_slice(
        &to_bytes(response.into_body(), 24 * 1024 * 1024)
            .await
            .unwrap(),
    )
    .unwrap()
}

mod pipeline;
mod routes;
