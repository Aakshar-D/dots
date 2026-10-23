//! Identity-only Sign in with ChatGPT. Credentials never leave this module.
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{RawQuery, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::{routing::get, Router};
use chrono::Utc;
use openidconnect::core::{CoreIdToken, CoreIdTokenVerifier, CoreJsonWebKeySet};
use openidconnect::{ClientId, CsrfToken, IssuerUrl, Nonce, PkceCodeChallenge};
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::{Error, Result};

const ISSUER: &str = "https://auth.openai.com";
pub const DEFAULT_REDIRECT_URI: &str = "http://127.0.0.1:47322/auth/openai/callback";
const SIGN_IN_LIMIT: Duration = Duration::from_secs(5 * 60);
const SESSION_LIMIT: Duration = Duration::from_secs(8 * 60 * 60);
const SETUP: &str = "ChatGPT sign-in needs a registered OpenAI client for Dots.";

#[derive(Clone)]
pub struct ChatGptConfig {
    client_id: String,
    redirect_uri: Url,
    issuer: Url,
}

impl ChatGptConfig {
    /// Desktop apps must use a public client, without an embedded client secret.
    pub fn new(client_id: String, redirect_uri: &str) -> Result<Self> {
        if client_id.is_empty()
            || client_id.len() > 256
            || client_id.chars().any(char::is_whitespace)
            || client_id.chars().any(char::is_control)
        {
            return Err(Error::Invalid("The OpenAI client ID is invalid.".into()));
        }
        let redirect_uri = Url::parse(redirect_uri)
            .map_err(|_| Error::Invalid("The ChatGPT callback URL is invalid.".into()))?;
        if redirect_uri.scheme() != "http"
            || redirect_uri.host_str() != Some("127.0.0.1")
            || redirect_uri.port().is_none_or(|port| port == 0)
            || !redirect_uri.username().is_empty()
            || redirect_uri.password().is_some()
            || redirect_uri.query().is_some()
            || redirect_uri.fragment().is_some()
        {
            return Err(Error::Invalid(
                "The ChatGPT callback must be an http://127.0.0.1 URL with an explicit nonzero port, without credentials, query, or fragment.".into(),
            ));
        }
        Ok(Self {
            client_id,
            redirect_uri,
            issuer: Url::parse(ISSUER).expect("constant issuer"),
        })
    }
}

/// Only display information crosses the native bridge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatGptAccount {
    pub name: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatGptStatus {
    pub configured: bool,
    pub signing_in: bool,
    pub account: Option<ChatGptAccount>,
    pub setup_error: Option<String>,
    pub expires_at: Option<String>,
}

struct Session {
    // Email alone is never the identity. Retain the verified provider/client/subject tuple.
    _identity: (String, String, String),
    account: ChatGptAccount,
    expires: Instant,
    expires_at: chrono::DateTime<Utc>,
}

#[derive(Default)]
struct AccountState {
    session: Option<Session>,
    pending: Option<CancellationToken>,
    signing_out: usize,
}

pub struct ChatGptAuth {
    config: Option<ChatGptConfig>,
    setup_error: Option<String>,
    state: Mutex<AccountState>,
    gate: tokio::sync::Mutex<()>,
    sign_in_limit: Duration,
}

impl ChatGptAuth {
    pub fn from_env() -> Self {
        let client = std::env::var("DOTS_OPENAI_CLIENT_ID")
            .ok()
            .or_else(|| option_env!("DOTS_OPENAI_CLIENT_ID").map(str::to_owned));
        let redirect = std::env::var("DOTS_OPENAI_REDIRECT_URI")
            .ok()
            .or_else(|| option_env!("DOTS_OPENAI_REDIRECT_URI").map(str::to_owned))
            .unwrap_or_else(|| DEFAULT_REDIRECT_URI.into());
        match client.filter(|id| !id.trim().is_empty()) {
            Some(id) => match ChatGptConfig::new(id, &redirect) {
                Ok(config) => Self::new(Some(config)),
                Err(error) => {
                    let mut auth = Self::new(None);
                    auth.setup_error = Some(error.to_string());
                    auth
                }
            },
            None => Self::new(None),
        }
    }

    pub fn new(config: Option<ChatGptConfig>) -> Self {
        Self {
            setup_error: config.is_none().then(|| SETUP.into()),
            config,
            state: Mutex::new(AccountState::default()),
            gate: tokio::sync::Mutex::new(()),
            sign_in_limit: SIGN_IN_LIMIT,
        }
    }

    pub fn status(&self) -> ChatGptStatus {
        let mut state = self.state.lock().expect("account state");
        if state
            .session
            .as_ref()
            .is_some_and(|s| s.expires <= Instant::now())
        {
            state.session = None;
        }
        ChatGptStatus {
            configured: self.config.is_some(),
            signing_in: state.pending.is_some(),
            account: state.session.as_ref().map(|s| s.account.clone()),
            setup_error: self.setup_error.clone(),
            expires_at: state.session.as_ref().map(|s| s.expires_at.to_rfc3339()),
        }
    }

    /// Completes only after the callback has been exchanged and cryptographically verified.
    pub async fn sign_in<F>(&self, open_browser: F) -> Result<ChatGptStatus>
    where
        F: FnOnce(&str) -> Result<()> + Send,
    {
        let config = self
            .config
            .as_ref()
            .ok_or_else(|| Error::Invalid(SETUP.into()))?;
        let cancel = CancellationToken::new();
        // Publishing cancellation and acquiring the attempt gate are atomic with sign-out.
        let _gate = {
            let mut state = self.state.lock().expect("account state");
            if state.signing_out > 0 {
                return Err(Error::Conflict(
                    "ChatGPT sign-out is in progress. Try again.".into(),
                ));
            }
            let gate = self
                .gate
                .try_lock()
                .map_err(|_| Error::Conflict("A ChatGPT sign-in is already in progress.".into()))?;
            state.pending = Some(cancel.clone());
            gate
        };
        let _pending = PendingGuard(self);

        let state = CsrfToken::new_random();
        let nonce = Nonce::new_random();
        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
        let listener = TcpListener::bind(("127.0.0.1", config.redirect_uri.port().unwrap()))
            .await
            .map_err(|_| Error::Conflict("The ChatGPT callback port is in use. Close the other app using it and try again.".into()))?;
        let (send, receive) = oneshot::channel();
        let callback = Arc::new(CallbackState {
            state: state.secret().clone(),
            issuer: config.issuer.as_str().trim_end_matches('/').into(),
            send: Mutex::new(Some(send)),
        });
        let router = Router::new()
            .route(config.redirect_uri.path(), get(callback_handler))
            .with_state(callback);
        let mut server = CallbackServer(tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        }));
        let flow = async {
            let client = Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(30))
                .build()
                .map_err(|_| Error::Other("Could not start the OpenAI connection.".into()))?;
            let provider = discover(&client, config).await?;
            let mut authorize =
                Url::parse(&provider.authorization_endpoint).map_err(|_| verification_error())?;
            authorize.query_pairs_mut().extend_pairs([
                ("client_id", config.client_id.as_str()),
                ("redirect_uri", config.redirect_uri.as_str()),
                ("response_type", "code"),
                ("scope", "openid profile email"),
                ("state", state.secret()),
                ("nonce", nonce.secret()),
                ("code_challenge", challenge.as_str()),
                ("code_challenge_method", "S256"),
            ]);
            open_browser(authorize.as_str())?;
            let code = receive.await.map_err(|_| verification_error())??;
            // Standard identity-only clients need an ID token, not access or refresh tokens.
            let tokens: IdentityResponse = client
                .post(&provider.token_endpoint)
                .form(&[
                    ("grant_type", "authorization_code"),
                    ("code", code.as_str()),
                    ("client_id", config.client_id.as_str()),
                    ("redirect_uri", config.redirect_uri.as_str()),
                    ("code_verifier", verifier.secret()),
                ])
                .send()
                .await
                .map_err(|_| connection_error())?
                .error_for_status()
                .map_err(|_| connection_error())?
                .json()
                .await
                .map_err(|_| verification_error())?;
            // Load keys after the exchange so a newly rotated signing key is available.
            let keys: CoreJsonWebKeySet = client
                .get(&provider.jwks_uri)
                .send()
                .await
                .map_err(|_| connection_error())?
                .error_for_status()
                .map_err(|_| connection_error())?
                .json()
                .await
                .map_err(|_| verification_error())?;
            verify_identity(&tokens.id_token, config, &provider.issuer, keys, &nonce)
        };
        let outcome = tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(Error::Conflict("ChatGPT sign-in cancelled.".into())),
            _ = tokio::time::sleep(self.sign_in_limit) => Err(Error::Other("ChatGPT sign-in timed out. Try again.".into())),
            result = flow => result,
        };
        // Await socket cleanup before another attempt can acquire the gate.
        server.0.abort();
        let _ = (&mut server.0).await;
        let session = outcome?;
        {
            let mut account = self.state.lock().expect("account state");
            if cancel.is_cancelled() {
                return Err(Error::Conflict("ChatGPT sign-in cancelled.".into()));
            }
            account.session = Some(session);
            account.pending = None;
        }
        Ok(self.status())
    }

    /// Cancels first, then waits for the attempt to release its callback listener.
    pub async fn sign_out(&self) -> ChatGptStatus {
        let gate = {
            let mut state = self.state.lock().expect("account state");
            state.signing_out += 1;
            if let Some(cancel) = &state.pending {
                cancel.cancel();
            }
            state.session = None;
            // If idle, reserve the gate before releasing state so a new attempt cannot
            // slip between sign-out checking pending and waiting for the gate.
            self.gate.try_lock().ok()
        };
        let _sign_out = SignOutGuard(self);
        let _gate = match gate {
            Some(gate) => gate,
            None => self.gate.lock().await,
        };
        self.state.lock().expect("account state").session = None;
        self.status()
    }
}

struct PendingGuard<'a>(&'a ChatGptAuth);
impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        self.0.state.lock().expect("account state").pending = None;
    }
}

struct SignOutGuard<'a>(&'a ChatGptAuth);
impl Drop for SignOutGuard<'_> {
    fn drop(&mut self) {
        self.0.state.lock().expect("account state").signing_out -= 1;
    }
}

struct CallbackServer(tokio::task::JoinHandle<()>);
impl Drop for CallbackServer {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct CallbackState {
    state: String,
    issuer: String,
    send: Mutex<Option<oneshot::Sender<Result<String>>>>,
}

async fn callback_handler(
    State(state): State<Arc<CallbackState>>,
    RawQuery(raw): RawQuery,
) -> Response {
    let Ok(url) = Url::parse(&format!("http://127.0.0.1/?{}", raw.unwrap_or_default())) else {
        return callback_page(StatusCode::BAD_REQUEST, "Sign-in could not be verified.");
    };
    let pairs: Vec<_> = url.query_pairs().collect();
    let values = |key: &str| {
        pairs
            .iter()
            .filter(|(k, _)| k == key)
            .map(|(_, v)| v.as_ref())
            .collect::<Vec<_>>()
    };
    let returned_state = values("state");
    if returned_state.len() != 1 || returned_state[0] != state.state {
        return callback_page(StatusCode::BAD_REQUEST, "Sign-in could not be verified.");
    }
    let mut send = state.send.lock().expect("callback sender");
    let Some(send) = send.take() else {
        return callback_page(
            StatusCode::GONE,
            "This sign-in request has already been used.",
        );
    };
    let issuer = values("iss");
    let code = values("code");
    let result = if !values("error").is_empty() {
        Err(Error::Invalid(
            "ChatGPT sign-in was declined or could not be completed. Try again.".into(),
        ))
    } else if issuer.len() > 1
        || issuer.first().is_some_and(|iss| *iss != state.issuer)
        || code.len() != 1
        || code[0].is_empty()
    {
        Err(verification_error())
    } else {
        Ok(code[0].to_owned())
    };
    let _ = send.send(result);
    callback_page(
        StatusCode::OK,
        "Sign-in received. Return to Dots to check your account.",
    )
}

fn callback_page(status: StatusCode, message: &str) -> Response {
    (
        status,
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::REFERRER_POLICY, "no-referrer"),
            (header::CONTENT_SECURITY_POLICY, "default-src 'none'; frame-ancestors 'none'"),
        ],
        Html(format!("<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><title>Dots sign-in</title><body><h1>Dots</h1><p>{message}</p></body></html>")),
    ).into_response()
}

#[derive(Deserialize)]
struct Provider {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    jwks_uri: String,
}

#[derive(Deserialize)]
struct IdentityResponse {
    id_token: String,
}

async fn discover(client: &Client, config: &ChatGptConfig) -> Result<Provider> {
    let url = config
        .issuer
        .join(".well-known/openid-configuration")
        .map_err(|_| verification_error())?;
    let provider: Provider = client
        .get(url)
        .send()
        .await
        .map_err(|_| connection_error())?
        .error_for_status()
        .map_err(|_| connection_error())?
        .json()
        .await
        .map_err(|_| verification_error())?;
    // URL parsing normalizes an origin's trailing slash; compare the discovery issuer exactly
    // to the production issuer string, and later use that exact string for ID-token validation.
    let expected = config.issuer.as_str().trim_end_matches('/');
    if provider.issuer != expected {
        return Err(verification_error());
    }
    for endpoint in [
        &provider.authorization_endpoint,
        &provider.token_endpoint,
        &provider.jwks_uri,
    ] {
        let url = Url::parse(endpoint).map_err(|_| verification_error())?;
        if url.origin() != config.issuer.origin()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(verification_error());
        }
    }
    Ok(provider)
}

fn verify_identity(
    raw: &str,
    config: &ChatGptConfig,
    issuer: &str,
    keys: CoreJsonWebKeySet,
    nonce: &Nonce,
) -> Result<Session> {
    let token: CoreIdToken = raw.parse().map_err(|_| verification_error())?;
    let verifier = CoreIdTokenVerifier::new_public_client(
        ClientId::new(config.client_id.clone()),
        IssuerUrl::new(issuer.into()).map_err(|_| verification_error())?,
        keys,
    )
    .set_issue_time_verifier_fn(|issued| {
        if issued > Utc::now() + chrono::Duration::seconds(5) {
            Err("The ID token was issued in the future.".into())
        } else {
            Ok(())
        }
    });
    let claims = token
        .claims(&verifier, nonce)
        .map_err(|_| verification_error())?;
    if claims.subject().as_str().trim().is_empty() {
        return Err(verification_error());
    }
    Ok(Session {
        _identity: (
            issuer.into(),
            config.client_id.clone(),
            claims.subject().as_str().into(),
        ),
        account: ChatGptAccount {
            name: claims
                .name()
                .and_then(|name| name.get(None))
                .map(|name| name.as_str().to_owned()),
            email: claims.email().map(|email| email.as_str().to_owned()),
        },
        expires: Instant::now() + SESSION_LIMIT,
        expires_at: Utc::now() + chrono::Duration::hours(8),
    })
}

fn verification_error() -> Error {
    Error::Invalid("ChatGPT sign-in could not be verified. Try again.".into())
}
fn connection_error() -> Error {
    Error::Other(
        "Could not complete sign-in with OpenAI. Check your connection and try again.".into(),
    )
}

#[cfg(test)]
mod tests;
