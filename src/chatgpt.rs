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

    /// Build the authorize URL for a new sign-in attempt.
    ///
    /// A saved client is reused unless `new_account` asks for a fresh
    /// registration, because each issued client is bound to one account.
    pub fn start_sign_in(&self, redirect_uri: &str, new_account: bool) -> Result<String> {
        let redirect_uri = validate_redirect_uri(redirect_uri)?;
        let saved_client = if new_account {
            None
        } else {
            self.registration
                .read()
                .unwrap()
                .as_ref()
                .map(|registration| registration.client_id.clone())
        };
        let client_id = saved_client.unwrap_or_else(|| DYNAMIC_CLIENT_ID.to_string());
        let state = crate::oauth::generate_token();
        let nonce = crate::oauth::generate_token();
        let verifier = crate::oauth::generate_token();

        let mut url = Url::parse(AUTHORIZE_URL).expect("authorize URL is valid");
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("client_id", &client_id);
            if client_id == DYNAMIC_CLIENT_ID {
                query.append_pair("agent_name_hint", AGENT_NAME);
            }
            query
                .append_pair("ext_agent_host_id", &self.host_id)
                .append_pair("response_type", "code")
                .append_pair("redirect_uri", &redirect_uri)
                .append_pair("scope", SCOPES)
                .append_pair("resource", RESOURCE)
                .append_pair("state", &state)
                .append_pair("nonce", &nonce)
                .append_pair("code_challenge_method", "S256")
                .append_pair("code_challenge", &pkce_challenge(&verifier));
        }

        let mut pending = self.pending.write().unwrap();
        pending.retain(|_, attempt| attempt.created.elapsed() < PENDING_TTL);
        if pending.len() >= MAX_PENDING {
            if let Some(oldest) = pending
                .iter()
                .min_by_key(|(_, attempt)| attempt.created)
                .map(|(key, _)| key.clone())
            {
                pending.remove(&oldest);
            }
        }
        pending.insert(
            state,
            PendingSignIn {
                verifier,
                nonce,
                redirect_uri,
                client_id,
                created: Instant::now(),
            },
        );
        Ok(url.to_string())
    }

    /// Finish sign-in from the loopback callback URL the browser landed on.
    pub async fn complete_sign_in(&self, callback_url: &str) -> Result<ChatGptStatus> {
        let callback = Url::parse(callback_url.trim())
            .context("Paste the full address from the browser, starting with http://127.0.0.1")?;
        let params: HashMap<String, String> = callback.query_pairs().into_owned().collect();
        let state = params
            .get("state")
            .ok_or_else(|| anyhow!("The callback address has no state. Start sign-in again."))?;
        let attempt = self
            .pending
            .write()
            .unwrap()
            .remove(state)
            .filter(|attempt| attempt.created.elapsed() < PENDING_TTL)
            .ok_or_else(|| anyhow!("This sign-in has expired or was already used. Start again."))?;
        if !same_callback_target(&callback, &attempt.redirect_uri) {
            bail!("The callback address doesn't match this sign-in. Start again.");
        }
        if let Some(error) = params.get("error") {
            if error == "access_denied" {
                bail!("Sign-in was cancelled in ChatGPT.");
            }
            bail!("ChatGPT sign-in failed ({error}).");
        }
        let code = params
            .get("code")
            .ok_or_else(|| anyhow!("The callback address has no code. Start sign-in again."))?;

        let client_id = match (attempt.client_id.as_str(), params.get("client_id")) {
            (DYNAMIC_CLIENT_ID, Some(issued)) if issued != DYNAMIC_CLIENT_ID => issued.clone(),
            (DYNAMIC_CLIENT_ID, _) => bail!("ChatGPT didn't issue a client ID. Start again."),
            (saved, Some(issued)) if issued != saved => {
                bail!("ChatGPT returned a different client. Start again.")
            }
            (saved, _) => saved.to_string(),
        };

        let response = self
            .client
            .post(TOKEN_URL)
            .form(&[
                ("grant_type", "authorization_code"),
                ("client_id", client_id.as_str()),
                ("code", code.as_str()),
                ("code_verifier", attempt.verifier.as_str()),
                ("redirect_uri", attempt.redirect_uri.as_str()),
                ("resource", RESOURCE),
            ])
            .send()
            .await
            .context("Failed to reach ChatGPT sign-in")?;
        if !response.status().is_success() {
            let status = response.status();
            let code = oauth_error_code(&response.text().await.unwrap_or_default());
            bail!(
                "ChatGPT sign-in was rejected (HTTP {}{}). Start again.",
                status.as_u16(),
                code.map(|code| format!(", {code}")).unwrap_or_default()
            );
        }
        let tokens: TokenResponse = response
            .json()
            .await
            .context("Invalid ChatGPT token response")?;

        let id_token = tokens
            .id_token
            .clone()
            .ok_or_else(|| anyhow!("ChatGPT didn't return an ID token."))?;
        let claims = self.verify_id_token(&id_token, &client_id).await?;
        if claims.nonce.as_deref() != Some(attempt.nonce.as_str()) {
            bail!("ChatGPT sign-in failed a security check (nonce). Start again.");
        }
        let scopes = tokens
            .scope
            .as_deref()
            .unwrap_or_default()
            .split_whitespace()
            .map(str::to_string)
            .collect::<Vec<_>>();
        if !scopes.iter().any(|scope| scope == PLAN_SCOPE) {
            bail!("Signed in, but ChatGPT plan usage wasn't granted for this account.");
        }

        let previous = self.registration.read().unwrap().clone();
        let same_account = previous
            .as_ref()
            .is_some_and(|r| r.client_id == client_id && r.subject == claims.sub);
        let mut registration = Registration {
            client_id,
            issuer: AUTH_ISSUER.to_string(),
            subject: claims.sub,
            email: claims.email,
            name: claims.name,
            ext_agent_host_id: self.host_id.clone(),
            id_token: Some(id_token),
            access_token: Some(tokens.access_token),
            refresh_token: tokens.refresh_token,
            token_type: tokens.token_type,
            expires_at: now() + tokens.expires_in.unwrap_or(3600),
            earliest_refresh_at: tokens.earliest_refresh_at,
            scopes,
            saved_at: now(),
            model: previous
                .as_ref()
                .filter(|_| same_account)
                .and_then(|r| r.model.clone()),
            models: Vec::new(),
        };
        match self
            .fetch_models(registration.access_token.as_deref().unwrap())
            .await
        {
            Ok(models) => registration.models = models,
            Err(error) => warn!("Could not list ChatGPT models: {error:#}"),
        }
        if registration
            .model
            .as_ref()
            .is_some_and(|model| !registration.models.iter().any(|m| &m.slug == model))
        {
            registration.model = None;
        }
        self.save(&registration)?;
        info!("Signed in with ChatGPT");
        Ok(self.status())
    }

    async fn verify_id_token(&self, id_token: &str, client_id: &str) -> Result<IdTokenClaims> {
        use jsonwebtoken::{decode, decode_header, jwk::JwkSet, DecodingKey, Validation};

        let header = decode_header(id_token).context("Invalid ID token")?;
        let discovery: Value = self
            .client
            .get(DISCOVERY_URL)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .context("Invalid OpenID configuration")?;
        let jwks_uri = discovery["jwks_uri"]
            .as_str()
            .ok_or_else(|| anyhow!("OpenID configuration has no jwks_uri"))?;
        let jwks: JwkSet = self
            .client
            .get(jwks_uri)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .context("Invalid ChatGPT signing keys")?;
        let jwk = match header.kid.as_deref() {
            Some(kid) => jwks.find(kid),
            None => jwks.keys.first(),
        }
        .ok_or_else(|| anyhow!("ID token signing key not found"))?;
        let key = DecodingKey::from_jwk(jwk).context("Unsupported ID token signing key")?;
        let mut validation = Validation::new(header.alg);
        validation.set_issuer(&[AUTH_ISSUER]);
        validation.set_audience(&[client_id]);
        Ok(decode::<IdTokenClaims>(id_token, &key, &validation)
            .context("ChatGPT ID token failed verification")?
            .claims)
    }

    async fn fetch_models(&self, access_token: &str) -> Result<Vec<ChatGptModel>> {
        let body: Value = self
            .client
            .get(MODELS_URL)
            .bearer_auth(access_token)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(parse_models(&body))
    }

    /// Re-list the account's models, keeping the server's order.
    pub async fn refresh_models(&self) -> Result<ChatGptStatus> {
        let token = self
            .access_token()
            .await?
            .ok_or_else(|| anyhow!("Not signed in with ChatGPT"))?;
        let models = self.fetch_models(&token).await?;
        let mut registration = self
            .registration
            .read()
            .unwrap()
            .clone()
            .ok_or_else(|| anyhow!("Not signed in with ChatGPT"))?;
        if registration
            .model
            .as_ref()
            .is_some_and(|model| !models.iter().any(|m| &m.slug == model))
        {
            registration.model = None;
        }
        registration.models = models;
        self.save(&registration)?;
        Ok(self.status())
    }

    pub fn set_model(&self, model: Option<String>) -> Result<ChatGptStatus> {
        let mut registration = self
            .registration
            .read()
            .unwrap()
            .clone()
            .filter(Registration::signed_in)
            .ok_or_else(|| anyhow!("Not signed in with ChatGPT"))?;
        if let Some(model) = &model {
            if !registration.models.iter().any(|m| &m.slug == model) {
                bail!("That model isn't available on this ChatGPT account");
            }
        }
        registration.model = model;
        self.save(&registration)?;
        Ok(self.status())
    }

    /// A current access token, refreshing it first when it is near expiry.
    /// `None` means nobody is signed in.
    pub async fn access_token(&self) -> Result<Option<String>> {
        if let Some(token) = self.fresh_token() {
            return Ok(Some(token));
        }
        let _guard = self.refresh_lock.lock().await;
        // Another request may have refreshed while this one waited.
        if let Some(token) = self.fresh_token() {
            return Ok(Some(token));
        }
        let Some(registration) = self
            .registration
            .read()
            .unwrap()
            .clone()
            .filter(Registration::signed_in)
        else {
            return Ok(None);
        };
        self.refresh(registration).await.map(Some)
    }

    fn fresh_token(&self) -> Option<String> {
        let registration = self.registration.read().unwrap();
        let registration = registration.as_ref().filter(|r| r.signed_in())?;
        (registration.expires_at - now() > REFRESH_MARGIN_SECS)
            .then(|| registration.access_token.clone())
            .flatten()
    }

    /// Treat the access token as expired after the API rejects it.
    pub fn mark_access_token_rejected(&self, rejected: &str) {
        let mut registration = self.registration.write().unwrap();
        if let Some(registration) = registration
            .as_mut()
            .filter(|r| r.access_token.as_deref() == Some(rejected))
        {
            registration.expires_at = 0;
            registration.earliest_refresh_at = None;
        }
    }

    async fn refresh(&self, mut registration: Registration) -> Result<String> {
        let refresh_token = registration
            .refresh_token
            .clone()
            .ok_or_else(|| anyhow!("ChatGPT session expired. Sign in again."))?;
        let response = self
            .client
            .post(TOKEN_URL)
            .form(&[
                ("grant_type", "refresh_token"),
                ("client_id", registration.client_id.as_str()),
                ("refresh_token", refresh_token.as_str()),
                ("resource", RESOURCE),
            ])
            .send()
            .await
            .context("Failed to refresh the ChatGPT session")?;
        if !response.status().is_success() {
            let status = response.status();
            let code = oauth_error_code(&response.text().await.unwrap_or_default());
            if code
                .as_deref()
                .is_some_and(|code| TERMINAL_REFRESH_ERRORS.contains(&code))
            {
                // Keep the client and host ID for the next sign-in.
                registration.clear_tokens();
                self.save(&registration)?;
                bail!("ChatGPT session ended. Sign in with ChatGPT again.");
            }
            // Temporary failures keep the credentials, per OpenAI's guidance.
            bail!(
                "ChatGPT session refresh failed (HTTP {}{})",
                status.as_u16(),
                code.map(|code| format!(", {code}")).unwrap_or_default()
            );
        }
        let tokens: TokenResponse = response
            .json()
            .await
            .context("Invalid ChatGPT refresh response")?;
        registration.access_token = Some(tokens.access_token.clone());
        if let Some(refresh_token) = tokens.refresh_token {
            registration.refresh_token = Some(refresh_token);
        }
        if let Some(id_token) = tokens.id_token {
            registration.id_token = Some(id_token);
        }
        if let Some(scope) = tokens.scope {
            registration.scopes = scope.split_whitespace().map(str::to_string).collect();
        }
        registration.token_type = tokens.token_type.or(registration.token_type);
        registration.expires_at = now() + tokens.expires_in.unwrap_or(3600);
        registration.earliest_refresh_at = tokens.earliest_refresh_at;
        registration.saved_at = now();
        self.save(&registration)?;
        if !registration.signed_in() {
            bail!("ChatGPT plan usage is no longer granted. Sign in again.");
        }
        Ok(tokens.access_token)
    }

    /// Revoke the refresh token, then forget the tokens. The client and host ID
    /// stay so a later sign-in reuses the same registration.
    pub async fn sign_out(&self) -> Result<SignOutResult> {
        let _guard = self.refresh_lock.lock().await;
        let Some(mut registration) = self.registration.read().unwrap().clone() else {
            return Ok(SignOutResult {
                revocation_confirmed: true,
            });
        };
        let revocation_confirmed = match registration.refresh_token.clone() {
            Some(token) => match self.revoke(&registration.client_id, &token).await {
                Ok(()) => true,
                Err(error) => {
                    warn!("ChatGPT token revocation was not confirmed: {error:#}");
                    false
                }
            },
            None => true,
        };
        registration.clear_tokens();
        self.save(&registration)?;
        info!("Signed out of ChatGPT");
        Ok(SignOutResult {
            revocation_confirmed,
        })
    }

    async fn revoke(&self, client_id: &str, refresh_token: &str) -> Result<()> {
        let discovery: Value = self
            .client
            .get(DISCOVERY_URL)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let endpoint = discovery["revocation_endpoint"]
            .as_str()
            .ok_or_else(|| anyhow!("OpenID configuration has no revocation_endpoint"))?;
        let mut last_error = None;
        for attempt in 0..3u64 {
            match self
                .client
                .post(endpoint)
                .form(&[
                    ("token", refresh_token),
                    ("token_type_hint", "refresh_token"),
                    ("client_id", client_id),
                ])
                .send()
                .await
            {
                Ok(response) if response.status().is_success() => return Ok(()),
                Ok(response) if !response.status().is_server_error() => {
                    bail!("revocation rejected (HTTP {})", response.status().as_u16())
                }
                Ok(response) => last_error = Some(anyhow!("HTTP {}", response.status().as_u16())),
                Err(error) => last_error = Some(error.into()),
            }
            tokio::time::sleep(Duration::from_millis(500 * (attempt + 1))).await;
        }
        Err(last_error.unwrap_or_else(|| anyhow!("revocation failed")))
    }
}

/// Authenticated web API for Sign in with ChatGPT.
pub mod routes {
    use super::{ChatGptAuth, DEFAULT_REDIRECT_URI};
    use crate::web::AppState;
    use axum::{
        extract::State,
        http::StatusCode,
        response::{IntoResponse, Response},
        Json,
    };
    use serde::Deserialize;
    use serde_json::json;
    use std::sync::Arc;

    fn auth(state: &AppState) -> Result<&Arc<ChatGptAuth>, Response> {
        state
            .ai_service
            .as_ref()
            .and_then(|service| service.chatgpt())
            .ok_or_else(|| {
                (
                    StatusCode::NOT_FOUND,
                    Json(json!({"error": "Sign in with ChatGPT is not available on this server"})),
                )
                    .into_response()
            })
    }

    fn failure(status: StatusCode, error: anyhow::Error) -> Response {
        (status, Json(json!({"error": format!("{error:#}")}))).into_response()
    }

    fn status_body(state: &AppState, auth: &ChatGptAuth) -> serde_json::Value {
        let service = state.ai_service.as_ref();
        let mut body = serde_json::to_value(auth.status()).unwrap_or_default();
        body["available"] = json!(true);
        body["apiKeyConfigured"] = json!(service.is_some_and(|s| s.has_audio()));
        body["defaultRedirectUri"] = json!(DEFAULT_REDIRECT_URI);
        body
    }

    pub async fn status(State(state): State<Arc<AppState>>) -> Response {
        match auth(&state) {
            Ok(auth) => Json(status_body(&state, auth)).into_response(),
            Err(_) => Json(json!({"available": false, "signedIn": false})).into_response(),
        }
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct SignInRequest {
        redirect_uri: Option<String>,
        #[serde(default)]
        new_account: bool,
    }

    pub async fn start(
        State(state): State<Arc<AppState>>,
        Json(req): Json<SignInRequest>,
    ) -> Response {
        let auth = match auth(&state) {
            Ok(auth) => auth,
            Err(response) => return response,
        };
        let redirect_uri = req.redirect_uri.as_deref().unwrap_or(DEFAULT_REDIRECT_URI);
        match auth.start_sign_in(redirect_uri, req.new_account) {
            Ok(url) => {
                Json(json!({"authorizeUrl": url, "redirectUri": redirect_uri})).into_response()
            }
            Err(error) => failure(StatusCode::BAD_REQUEST, error),
        }
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct CallbackRequest {
        callback_url: String,
    }

    pub async fn callback(
        State(state): State<Arc<AppState>>,
        Json(req): Json<CallbackRequest>,
    ) -> Response {
        let auth = match auth(&state) {
            Ok(auth) => auth,
            Err(response) => return response,
        };
        match auth.complete_sign_in(&req.callback_url).await {
            Ok(_) => Json(status_body(&state, auth)).into_response(),
            Err(error) => failure(StatusCode::BAD_REQUEST, error),
        }
    }

    #[derive(Deserialize)]
    pub struct ModelRequest {
        model: Option<String>,
    }

    pub async fn set_model(
        State(state): State<Arc<AppState>>,
        Json(req): Json<ModelRequest>,
    ) -> Response {
        let auth = match auth(&state) {
            Ok(auth) => auth,
            Err(response) => return response,
        };
        match auth.set_model(req.model.filter(|model| !model.is_empty())) {
            Ok(_) => Json(status_body(&state, auth)).into_response(),
            Err(error) => failure(StatusCode::BAD_REQUEST, error),
        }
    }

    pub async fn refresh_models(State(state): State<Arc<AppState>>) -> Response {
        let auth = match auth(&state) {
            Ok(auth) => auth,
            Err(response) => return response,
        };
        match auth.refresh_models().await {
            Ok(_) => Json(status_body(&state, auth)).into_response(),
            Err(error) => failure(StatusCode::BAD_GATEWAY, error),
        }
    }

    pub async fn sign_out(State(state): State<Arc<AppState>>) -> Response {
        let auth = match auth(&state) {
            Ok(auth) => auth,
            Err(response) => return response,
        };
        match auth.sign_out().await {
            Ok(result) => {
                let mut body = status_body(&state, auth);
                body["revocationConfirmed"] = json!(result.revocation_confirmed);
                Json(body).into_response()
            }
            Err(error) => failure(StatusCode::INTERNAL_SERVER_ERROR, error),
        }
    }
}

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
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wa-chatgpt-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn redirect_uri_must_be_ipv4_loopback_callback() {
        assert!(validate_redirect_uri("http://127.0.0.1:1455/auth/callback").is_ok());
        assert!(validate_redirect_uri("http://127.0.0.1:53111/auth/callback").is_ok());
        for bad in [
            "http://localhost:1455/auth/callback",
            "https://127.0.0.1:1455/auth/callback",
            "http://127.0.0.1/auth/callback",
            "http://127.0.0.1:1455/other",
            "http://127.0.0.1:1455/auth/callback?x=1",
            "http://example.com:1455/auth/callback",
        ] {
            assert!(validate_redirect_uri(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn host_id_is_created_once_and_kept() {
        let dir = temp_dir();
        let first = ChatGptAuth::load(&dir).unwrap().host_id;
        assert!(first.starts_with("urn:uuid:"));
        assert_eq!(ChatGptAuth::load(&dir).unwrap().host_id, first);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join(HOST_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn authorize_url_registers_dynamically_with_pkce_and_plan_scopes() {
        let dir = temp_dir();
        let auth = ChatGptAuth::load(&dir).unwrap();
        let url = Url::parse(&auth.start_sign_in(DEFAULT_REDIRECT_URI, false).unwrap()).unwrap();
        let params: HashMap<String, String> = url.query_pairs().into_owned().collect();
        assert_eq!(url.as_str().split('?').next(), Some(AUTHORIZE_URL));
        assert_eq!(params["client_id"], DYNAMIC_CLIENT_ID);
        assert_eq!(params["agent_name_hint"], AGENT_NAME);
        assert_eq!(params["ext_agent_host_id"], auth.host_id);
        assert_eq!(params["resource"], RESOURCE);
        assert_eq!(params["code_challenge_method"], "S256");
        assert!(params["scope"].contains(PLAN_SCOPE));
        assert!(params["scope"].contains("offline_access"));
        let pending = auth.pending.read().unwrap();
        let attempt = &pending[&params["state"]];
        assert_eq!(params["code_challenge"], pkce_challenge(&attempt.verifier));
        assert_eq!(params["nonce"], attempt.nonce);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn saved_client_is_reused_without_a_name_hint() {
        let dir = temp_dir();
        let auth = ChatGptAuth::load(&dir).unwrap();
        auth.save(&Registration {
            client_id: "issued-client".into(),
            ..Default::default()
        })
        .unwrap();
        let url = Url::parse(&auth.start_sign_in(DEFAULT_REDIRECT_URI, false).unwrap()).unwrap();
        let params: HashMap<String, String> = url.query_pairs().into_owned().collect();
        assert_eq!(params["client_id"], "issued-client");
        assert!(!params.contains_key("agent_name_hint"));
        let url = Url::parse(&auth.start_sign_in(DEFAULT_REDIRECT_URI, true).unwrap()).unwrap();
        assert!(url
            .query_pairs()
            .any(|(k, v)| k == "client_id" && v == DYNAMIC_CLIENT_ID));
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn callback_rejects_unknown_state_and_denied_consent() {
        let dir = temp_dir();
        let auth = ChatGptAuth::load(&dir).unwrap();
        assert!(auth
            .complete_sign_in("http://127.0.0.1:1455/auth/callback?code=x&state=nope")
            .await
            .is_err());
        let url = Url::parse(&auth.start_sign_in(DEFAULT_REDIRECT_URI, false).unwrap()).unwrap();
        let state = url
            .query_pairs()
            .find(|(k, _)| k == "state")
            .unwrap()
            .1
            .into_owned();
        let error = auth
            .complete_sign_in(&format!(
                "http://127.0.0.1:1455/auth/callback?error=access_denied&state={state}"
            ))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("cancelled"));
        // A state is single use.
        assert!(auth
            .complete_sign_in(&format!(
                "http://127.0.0.1:1455/auth/callback?code=x&state={state}"
            ))
            .await
            .is_err());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn signed_in_requires_the_plan_scope() {
        let mut registration = Registration {
            access_token: Some("token".into()),
            scopes: vec!["openid".into()],
            ..Default::default()
        };
        assert!(!registration.signed_in());
        registration.scopes.push(PLAN_SCOPE.into());
        assert!(registration.signed_in());
        registration.clear_tokens();
        assert!(!registration.signed_in());
    }

    #[test]
    fn models_keep_listed_entries_in_server_order() {
        let body = serde_json::json!({"models": [
            {"slug": "b", "display_name": "B", "visibility": "list"},
            {"slug": "hidden", "display_name": "H", "visibility": "hide"},
            {"slug": "a", "visibility": "list"},
        ]});
        let models = parse_models(&body);
        assert_eq!(
            models,
            vec![
                ChatGptModel {
                    slug: "b".into(),
                    display_name: "B".into()
                },
                ChatGptModel {
                    slug: "a".into(),
                    display_name: "a".into()
                },
            ]
        );
    }

    #[tokio::test]
    async fn fresh_tokens_are_returned_without_refreshing() {
        let dir = temp_dir();
        let auth = ChatGptAuth::load(&dir).unwrap();
        assert_eq!(auth.access_token().await.unwrap(), None);
        auth.save(&Registration {
            client_id: "c".into(),
            access_token: Some("live".into()),
            scopes: vec![PLAN_SCOPE.into()],
            expires_at: now() + 3600,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(auth.access_token().await.unwrap().as_deref(), Some("live"));
        auth.mark_access_token_rejected("live");
        assert!(auth.fresh_token().is_none());
        std::fs::remove_dir_all(dir).ok();
    }
}
