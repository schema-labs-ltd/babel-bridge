use super::*;

impl ChatGptAuth {
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

    pub(super) async fn verify_id_token(
        &self,
        id_token: &str,
        client_id: &str,
    ) -> Result<IdTokenClaims> {
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

    pub(super) async fn fetch_models(&self, access_token: &str) -> Result<Vec<ChatGptModel>> {
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
}
