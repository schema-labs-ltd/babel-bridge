use super::*;

impl ChatGptAuth {
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

    pub(super) fn fresh_token(&self) -> Option<String> {
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

    pub(super) async fn refresh(&self, mut registration: Registration) -> Result<String> {
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

    pub(super) async fn revoke(&self, client_id: &str, refresh_token: &str) -> Result<()> {
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
