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

#[allow(clippy::result_large_err)] // Handlers return the error response as-is.
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

pub async fn start(State(state): State<Arc<AppState>>, Json(req): Json<SignInRequest>) -> Response {
    let auth = match auth(&state) {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    let redirect_uri = req.redirect_uri.as_deref().unwrap_or(DEFAULT_REDIRECT_URI);
    match auth.start_sign_in(redirect_uri, req.new_account) {
        Ok(url) => Json(json!({"authorizeUrl": url, "redirectUri": redirect_uri})).into_response(),
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
