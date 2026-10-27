//! WhatsApp Translator - CLI application for connecting to WhatsApp and displaying messages.
//!
//! This application uses a Go bridge (wa-bridge) that implements the WhatsApp Web protocol
//! via the whatsmeow library. Communication happens via JSON-lines over stdio.

mod access;
#[cfg(test)]
mod api_contract;
mod bridge;
mod chatgpt;
mod cli;
mod display;
mod event_json;
mod gallery;
mod identity;
mod incoming;
mod link_preview;
mod log_privacy;
mod mcp;
mod message_tones;
mod oauth;
mod outbox;
mod push;
mod read_receipts;
mod shutdown;
mod storage;
mod style_analyzer;
mod terminal_mode;
mod topics;
mod translation;
mod voice;
mod web;
mod web_events;
mod web_mode;

use anyhow::{Context, Result};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

use bridge::{BridgeConfig, BridgeEvent, BridgeProcess, ConnectionState, Message, MessageContent};
use cli::Args;
use display::{
    clear_qr_display, print_connected, print_error, print_info, print_warning, render_qr_code,
    MessageDisplay,
};
use push::ApnsClient;
use storage::{MessageStore, StoredMessage};
use translation::TranslationService;
use web::AppState;

use terminal_mode::*;
use web_events::*;
use web_mode::*;

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse_args();
    if args.web {
        access::validate(
            &args.host,
            args.password.as_deref(),
            args.allow_local_no_auth,
        )?;
    }

    // Initialize logging
    log_privacy::init_logging(args.verbose);

    // Determine data directory
    let data_dir = args
        .data_dir
        .clone()
        .or_else(|| bridge::default_data_dir().ok())
        .context("Could not determine data directory")?;

    info!("Using data directory: {:?}", data_dir);

    // Handle logout request
    if args.logout {
        handle_logout(&data_dir).await?;
    }

    // Find bridge binary
    let bridge_path = args
        .bridge_path
        .clone()
        .or_else(|| bridge::find_bridge_binary().ok())
        .context(
            "Could not find wa-bridge binary. Please build it first:\n\
             cd apps/wa-bridge && go build -o wa-bridge .",
        )?;

    info!("Using bridge binary: {:?}", bridge_path);

    // Create bridge configuration
    let config = BridgeConfig {
        binary_path: bridge_path,
        data_dir: data_dir.clone(),
        verbose: args.verbose,
    };

    // The web server always has an AI service, so a ChatGPT sign-in made in the
    // app takes effect without a restart. Terminal mode needs the API key.
    let translator = if args.web || args.translation_enabled() {
        let mut service = TranslationService::new(
            args.openai_api_key.clone(),
            args.openai_detection_model.clone(),
            args.openai_translation_model.clone(),
            args.openai_high_end_model.clone(),
            args.default_language.clone(),
        );
        if args.web {
            let auth = chatgpt::ChatGptAuth::load(&data_dir)
                .context("Failed to load Sign in with ChatGPT credentials")?;
            service = service.with_chatgpt(Arc::new(auth));
        }
        if args.fake_ai {
            warn!("WA_FAKE_AI is set: text AI returns placeholder output and OpenAI is not called");
            service = service.with_fake_ai();
        }
        if service.is_configured() {
            info!("AI features enabled (target: {})", args.default_language);
        } else {
            info!("AI features off until OPENAI_API_KEY is set or someone signs in with ChatGPT");
        }
        Some(Arc::new(service))
    } else {
        None
    };

    if args.web {
        // Web server mode
        run_web_mode(config, args, data_dir, translator).await
    } else {
        // Terminal mode
        run_terminal_mode(config, args.json, translator).await
    }
}

fn record_bridge_log(level: &str, message: &str) {
    let message = log_privacy::redact_log_text(message);
    match level {
        "error" => error!("{message}"),
        "warn" => warn!("{message}"),
        "info" => info!("{message}"),
        _ => debug!("{message}"),
    }
}

/// Handle logout by removing session data
async fn handle_logout(data_dir: &std::path::Path) -> Result<()> {
    let mut removed = false;

    for filename in ["session.db", "session.db-wal", "session.db-shm"] {
        let path = data_dir.join(filename);
        match tokio::fs::remove_file(&path).await {
            Ok(()) => removed = true,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err).context("Failed to remove session database"),
        }
    }

    if removed {
        print_info("Session cleared. You will need to scan a new QR code.");
    } else {
        print_info("No existing session found.");
    }
    Ok(())
}

/// Find the web directory
fn find_web_dir() -> Result<std::path::PathBuf> {
    // Try various locations
    let candidates = [
        std::env::current_dir()?.join("web/public"),
        // `cargo run` from the repository root.
        std::env::current_dir()?.join("apps/web/public"),
        std::env::current_exe()?
            .parent()
            .unwrap()
            .join("../web/public"),
        std::env::current_exe()?
            .parent()
            .unwrap()
            .join("../../web/public"),
        std::env::current_exe()?
            .parent()
            .unwrap()
            .join("../../apps/web/public"),
        dirs::data_dir()
            .unwrap_or_default()
            .join("whatsapp-translator/web"),
    ];

    for candidate in candidates {
        if candidate.exists() && candidate.join("index.html").exists() {
            return Ok(candidate);
        }
    }

    anyhow::bail!("Could not find web directory. Expected at ./web/public, ./apps/web/public or near the executable.")
}

#[cfg(test)]
mod identity_tests;
