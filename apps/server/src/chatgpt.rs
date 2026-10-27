//! Sign in with ChatGPT, so text AI can draw on the user's ChatGPT plan.
//!
//! Follows OpenAI's open-source and self-hosted flow: dynamic client
//! registration, PKCE, and a `127.0.0.1` loopback redirect. The server keeps the
//! PKCE secrets and tokens. A browser or native app only opens the authorize URL
//! and hands the loopback callback URL back, because the redirect reaches the
//! signed-in device rather than a remotely hosted server.
//!
//! Plan usage covers the Responses API only. Audio transcription and speech
//! still need `OPENAI_API_KEY`.

use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tracing::{info, warn};
mod sign_in;
mod tokens;

const AUTH_ISSUER: &str = "https://auth.openai.com";
const AUTHORIZE_URL: &str = "https://auth.openai.com/api/accounts/authorize";
const TOKEN_URL: &str = "https://auth.openai.com/api/accounts/oauth/token";
const DISCOVERY_URL: &str = "https://auth.openai.com/.well-known/openid-configuration";
const MODELS_URL: &str = "https://api.openai.com/v1/models";
pub const RESOURCE: &str = "https://api.openai.com/v1";
const SCOPES: &str =
    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
const PLAN_SCOPE: &str = "chatgpt.tokens.use.direct";
const DYNAMIC_CLIENT_ID: &str = "dynamic_agent_client";
const AGENT_NAME: &str = "Babel Bridge";
const CALLBACK_PATH: &str = "/auth/callback";
pub const DEFAULT_REDIRECT_URI: &str = "http://127.0.0.1:1455/auth/callback";
const PENDING_TTL: Duration = Duration::from_secs(15 * 60);
const MAX_PENDING: usize = 16;
/// Refresh this long before the one-hour access token expires.
const REFRESH_MARGIN_SECS: i64 = 300;
const HTTP_TIMEOUT: Duration = Duration::from_secs(20);
const AUTH_FILE: &str = "chatgpt-auth.json";
const HOST_FILE: &str = "chatgpt-host.json";

/// Refresh errors that mean the refresh token can never work again.
const TERMINAL_REFRESH_ERRORS: &[&str] = &[
    "invalid_grant",
    "invalid_refresh_token",
    "token_expired",
    "refresh_token_expired",
    "refresh_token_invalidated",
    "refresh_token_reused",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChatGptModel {
    pub slug: String,
    pub display_name: String,
}

/// One registration: the issued client and, while signed in, its tokens.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Registration {
    client_id: String,
    issuer: String,
    subject: String,
    email: Option<String>,
    name: Option<String>,
    ext_agent_host_id: String,
    id_token: Option<String>,
    access_token: Option<String>,
    refresh_token: Option<String>,
    token_type: Option<String>,
    /// Unix seconds.
    expires_at: i64,
    earliest_refresh_at: Option<i64>,
    scopes: Vec<String>,
    saved_at: i64,
    model: Option<String>,
    #[serde(default)]
    models: Vec<ChatGptModel>,
}

impl Registration {
    fn signed_in(&self) -> bool {
        self.access_token.is_some() && self.scopes.iter().any(|scope| scope == PLAN_SCOPE)
    }

    fn clear_tokens(&mut self) {
        self.id_token = None;
        self.access_token = None;
        self.refresh_token = None;
        self.token_type = None;
        self.expires_at = 0;
        self.earliest_refresh_at = None;
        self.scopes.clear();
        self.saved_at = now();
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct HostFile {
    ext_agent_host_id: String,
}

struct PendingSignIn {
    verifier: String,
    nonce: String,
    redirect_uri: String,
    client_id: String,
    created: Instant,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    id_token: Option<String>,
    token_type: Option<String>,
    expires_in: Option<i64>,
    scope: Option<String>,
    earliest_refresh_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct IdTokenClaims {
    sub: String,
    nonce: Option<String>,
    email: Option<String>,
    name: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatGptStatus {
    pub signed_in: bool,
    pub email: Option<String>,
    pub name: Option<String>,
    pub model: Option<String>,
    pub models: Vec<ChatGptModel>,
}

/// Outcome of signing out: whether OpenAI confirmed the revocation.
pub struct SignOutResult {
    pub revocation_confirmed: bool,
}

pub struct ChatGptAuth {
    client: Client,
    data_dir: PathBuf,
    host_id: String,
    registration: RwLock<Option<Registration>>,
    pending: RwLock<HashMap<String, PendingSignIn>>,
    /// Serializes refreshes so two requests never race a rotating refresh token.
    refresh_lock: Mutex<()>,
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Accept only `http://127.0.0.1:<port>/auth/callback`, as OpenAI requires.
pub fn validate_redirect_uri(uri: &str) -> Result<String> {
    let url = Url::parse(uri).context("Invalid redirect URI")?;
    if url.scheme() != "http"
        || url.host_str() != Some("127.0.0.1")
        || url.port().is_none()
        || url.path() != CALLBACK_PATH
        || url.query().is_some()
        || url.fragment().is_some()
    {
        bail!("Redirect URI must be http://127.0.0.1:<port>{CALLBACK_PATH}");
    }
    Ok(url.to_string())
}

fn same_callback_target(callback: &Url, redirect_uri: &str) -> bool {
    let Ok(expected) = Url::parse(redirect_uri) else {
        return false;
    };
    callback.scheme() == expected.scheme()
        && callback.host_str() == expected.host_str()
        && callback.port() == expected.port()
        && callback.path() == expected.path()
}

/// Write owner-only, atomically, so a crash never leaves half a credential file.
fn write_private_file(path: &Path, contents: &[u8]) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    {
        use std::io::Write;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&tmp)
            .with_context(|| format!("Failed to write {}", tmp.display()))?;
        file.write_all(contents)?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path).with_context(|| format!("Failed to save {}", path.display()))?;
    Ok(())
}

fn oauth_error_code(body: &str) -> Option<String> {
    let value: Value = serde_json::from_str(body).ok()?;
    value["error"]
        .as_str()
        .or_else(|| value["error"]["code"].as_str())
        .map(str::to_string)
}

impl ChatGptAuth {
    /// Load the saved host ID and registration, creating a host ID on first run.
    pub fn load(data_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(data_dir).context("Failed to create data directory")?;
        let host_path = data_dir.join(HOST_FILE);
        let host_id = match std::fs::read(&host_path) {
            Ok(bytes) => {
                serde_json::from_slice::<HostFile>(&bytes)
                    .context("Invalid ChatGPT host file")?
                    .ext_agent_host_id
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let host_id = format!("urn:uuid:{}", uuid::Uuid::new_v4());
                let file = HostFile {
                    ext_agent_host_id: host_id.clone(),
                };
                write_private_file(&host_path, &serde_json::to_vec_pretty(&file)?)?;
                host_id
            }
            Err(error) => return Err(error).context("Failed to read ChatGPT host file"),
        };

        let registration = match std::fs::read(data_dir.join(AUTH_FILE)) {
            Ok(bytes) => Some(
                serde_json::from_slice::<Registration>(&bytes)
                    .context("Invalid ChatGPT credential file")?,
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error).context("Failed to read ChatGPT credentials"),
        };

        Ok(Self {
            client: Client::builder()
                .timeout(HTTP_TIMEOUT)
                .build()
                .expect("ChatGPT HTTP client should build"),
            data_dir: data_dir.to_path_buf(),
            host_id,
            registration: RwLock::new(registration),
            pending: RwLock::new(HashMap::new()),
            refresh_lock: Mutex::new(()),
        })
    }

    /// A signed-in session with a fresh token, for tests elsewhere in the crate.
    #[cfg(test)]
    pub(crate) fn signed_in_for_test(data_dir: &Path, access_token: &str, models: &[&str]) -> Self {
        let auth = Self::load(data_dir).unwrap();
        auth.save(&Registration {
            client_id: "issued-client".into(),
            access_token: Some(access_token.into()),
            scopes: vec![PLAN_SCOPE.into()],
            expires_at: now() + 3600,
            models: models
                .iter()
                .map(|slug| ChatGptModel {
                    slug: slug.to_string(),
                    display_name: slug.to_string(),
                })
                .collect(),
            ..Default::default()
        })
        .unwrap();
        auth
    }

    fn save(&self, registration: &Registration) -> Result<()> {
        write_private_file(
            &self.data_dir.join(AUTH_FILE),
            &serde_json::to_vec_pretty(registration)?,
        )?;
        *self.registration.write().unwrap() = Some(registration.clone());
        Ok(())
    }

    pub fn is_signed_in(&self) -> bool {
        self.registration
            .read()
            .unwrap()
            .as_ref()
            .is_some_and(Registration::signed_in)
    }

    /// The model chosen for plan usage, or the first one the account lists.
    pub fn model(&self) -> Option<String> {
        let registration = self.registration.read().unwrap();
        let registration = registration.as_ref()?;
        registration
            .model
            .clone()
            .or_else(|| registration.models.first().map(|model| model.slug.clone()))
    }

    pub fn status(&self) -> ChatGptStatus {
        let registration = self.registration.read().unwrap();
        match registration.as_ref().filter(|r| r.signed_in()) {
            Some(registration) => ChatGptStatus {
                signed_in: true,
                email: registration.email.clone(),
                name: registration.name.clone(),
                model: registration
                    .model
                    .clone()
                    .or_else(|| registration.models.first().map(|m| m.slug.clone())),
                models: registration.models.clone(),
            },
            None => ChatGptStatus {
                signed_in: false,
                email: None,
                name: None,
                model: None,
                models: Vec::new(),
            },
        }
    }
}

/// Authenticated web API for Sign in with ChatGPT.
pub mod routes;

fn parse_models(body: &Value) -> Vec<ChatGptModel> {
    body["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|model| model["visibility"].as_str() == Some("list"))
        .filter_map(|model| {
            let slug = model["slug"].as_str()?.to_string();
            let display_name = model["display_name"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| slug.clone());
            Some(ChatGptModel { slug, display_name })
        })
        .collect()
}

#[cfg(test)]
mod tests;
