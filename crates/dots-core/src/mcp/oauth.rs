//! MCP authorization (spec 2025-06-18) for a hosted server that asks for sign-in (connectors
//! spec section 5): discovery of the protected resource (RFC 9728) and its authorization server
//! (RFC 8414, OpenID discovery as a fallback), dynamic client registration (RFC 7591), the
//! authorization code flow with PKCE S256 in the system browser and a loopback callback
//! (RFC 8252), the `resource` parameter (RFC 8707), and refresh.
//!
//! Nothing here logs or returns a token in an error. Where the tokens are kept is the caller's
//! business: `sign_in` and `refresh` hand back an `OAuthGrant` (what a refresh needs) and the
//! access token.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{RawQuery, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::{routing::get, Router};
use chrono::{DateTime, Utc};
use openidconnect::{CsrfToken, PkceCodeChallenge};
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

use crate::{Error, Result};

/// How long a sign-in waits for the person to finish in the browser.
pub const SIGN_IN_LIMIT: Duration = Duration::from_secs(5 * 60);
/// The callback's path on the loopback listener.
const CALLBACK_PATH: &str = "/callback";
/// The name Dots registers under.
const CLIENT_NAME: &str = "Dots";
/// The MCP protocol version the discovery request names.
const PROTOCOL_VERSION: &str = "2025-06-18";

/// An authorization server, as its metadata describes it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AuthServer {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    #[serde(default)]
    pub registration_endpoint: Option<String>,
    #[serde(default)]
    pub code_challenge_methods_supported: Vec<String>,
    #[serde(default)]
    pub token_endpoint_auth_methods_supported: Vec<String>,
}

/// What discovery found for a server URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovered {
    /// The resource the tokens are for (RFC 8707): the protected resource metadata's, or the
    /// server URL when it has none.
    pub resource: String,
    /// The scope to ask for: the 401's, else the metadata's `scopes_supported`, else none.
    pub scope: Option<String>,
    pub server: AuthServer,
}

/// The client Dots registered with an authorization server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registration {
    pub client_id: String,
    /// Set when the server made Dots a confidential client.
    #[serde(default)]
    pub client_secret: Option<String>,
    /// `none`, `client_secret_basic` or `client_secret_post`.
    pub auth_method: String,
}

/// What a refresh needs, kept between runs. Holds the refresh token and the client secret:
/// the caller keeps it in the secret store.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthGrant {
    pub client: Registration,
    pub issuer: String,
    pub token_endpoint: String,
    pub resource: String,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// When the access token expires, if the server said.
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
    /// A refresh was refused: only signing in again helps.
    #[serde(default)]
    pub needs_sign_in: bool,
}

impl std::fmt::Debug for OAuthGrant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthGrant")
            .field("client_id", &self.client.client_id)
            .field("issuer", &self.issuer)
            .field("resource", &self.resource)
            .field("expires_at", &self.expires_at)
            .field("needs_sign_in", &self.needs_sign_in)
            .finish_non_exhaustive()
    }
}

impl OAuthGrant {
    /// Whether the access token has expired or expires within `margin` of `now`. A token whose
    /// expiry the server did not give is taken as fresh.
    pub fn expires_within(&self, now: DateTime<Utc>, margin: Duration) -> bool {
        self.expires_at.is_some_and(|at| {
            at <= now + chrono::Duration::from_std(margin).unwrap_or(chrono::Duration::zero())
        })
    }
}

/// A finished sign-in or refresh: the grant to keep and the access token.
pub struct Signed {
    pub grant: OAuthGrant,
    pub access_token: Zeroizing<String>,
}

/// Why a refresh did not give a token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshFailure {
    /// The server refused the refresh token or the client: sign in again.
    Rejected,
    /// The server could not be reached or answered something else: try again later.
    Failed(String),
}

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

/// The HTTP client of the flow: no redirects (a 3xx is an error, so nothing is sent to a place
/// the metadata did not name), and a time limit on every request.
pub fn client() -> Client {
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .build()
        .expect("an http client")
}

/// Whether a URL may carry the flow: https, or http on a loopback host (tests and local
/// servers). No credentials in it, no fragment.
fn secure(url: &Url) -> bool {
    let loopback = matches!(
        url.host_str(),
        Some("127.0.0.1") | Some("localhost") | Some("[::1]")
    );
    (url.scheme() == "https" || (url.scheme() == "http" && loopback))
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
}

fn parse_secure(text: &str, what: &str) -> Result<Url> {
    let url = Url::parse(text).map_err(|_| invalid(format!("the {what} is not a valid URL")))?;
    if !secure(&url) {
        return Err(invalid(format!("the {what} is not an https URL")));
    }
    Ok(url)
}

/// The parameters of a `WWW-Authenticate: Bearer` challenge, by lower-cased name. Values may be
/// quoted (with `\` escapes) or bare tokens.
pub fn bearer_params(header: &str) -> Vec<(String, String)> {
    let rest = header.trim();
    let Some(rest) = rest
        .get(..6)
        .filter(|s| s.eq_ignore_ascii_case("bearer"))
        .map(|_| &rest[6..])
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut chars = rest.chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| c.is_whitespace() || *c == ',') {
            chars.next();
        }
        let name: String = std::iter::from_fn(|| chars.next_if(|c| *c != '=' && *c != ','))
            .collect::<String>()
            .trim()
            .to_ascii_lowercase();
        if name.is_empty() {
            break;
        }
        if chars.next() != Some('=') {
            continue;
        }
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        let mut value = String::new();
        if chars.peek() == Some(&'"') {
            chars.next();
            while let Some(c) = chars.next() {
                match c {
                    '\\' => value.extend(chars.next()),
                    '"' => break,
                    c => value.push(c),
                }
            }
        } else {
            value = std::iter::from_fn(|| chars.next_if(|c| *c != ','))
                .collect::<String>()
                .trim()
                .to_string();
        }
        out.push((name, value));
    }
    out
}

/// `origin` + `/.well-known/<name>` + `path`, the RFC 8414 / RFC 9728 form for a URL with a
/// path; without one, `origin/.well-known/<name>`.
fn well_known(url: &Url, name: &str) -> Vec<String> {
    let origin = url.origin().ascii_serialization();
    let path = url.path().trim_end_matches('/');
    let mut out = Vec::new();
    if !path.is_empty() {
        out.push(format!("{origin}/.well-known/{name}{path}"));
    }
    out.push(format!("{origin}/.well-known/{name}"));
    out
}

#[derive(Deserialize)]
struct ResourceMetadata {
    resource: Option<String>,
    #[serde(default)]
    authorization_servers: Vec<String>,
    #[serde(default)]
    scopes_supported: Vec<String>,
}

/// GETs `url` as JSON; `None` when it is not there or is not JSON (the next place is tried),
/// an error when the host cannot be reached.
async fn get_json<T: for<'de> Deserialize<'de>>(http: &Client, url: &str) -> Result<Option<T>> {
    let response = http
        .get(url)
        .header(header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|_| unreachable_error())?;
    // A 404 or 405 (or any other failure) sends the caller to the next place.
    if !response.status().is_success() {
        return Ok(None);
    }
    let body = response.bytes().await.map_err(|_| unreachable_error())?;
    Ok(serde_json::from_slice(&body).ok())
}

fn unreachable_error() -> Error {
    Error::Other("Could not reach the sign-in server. Check your connection and try again.".into())
}

/// Finds where and how to sign in to the MCP server at `server_url`.
pub async fn discover(http: &Client, server_url: &str) -> Result<Discovered> {
    let server = parse_secure(server_url, "server URL")?;
    // An `initialize` without a token: a server that wants sign-in answers 401 and names its
    // metadata.
    let probe = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": {"name": CLIENT_NAME, "version": env!("CARGO_PKG_VERSION")}
        }
    });
    let response = http
        .post(server.clone())
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json, text/event-stream")
        .header("MCP-Protocol-Version", PROTOCOL_VERSION)
        .body(probe.to_string())
        .send()
        .await
        .map_err(|_| unreachable_error())?;
    if response.status().is_success() {
        return Err(invalid("this server does not ask for sign-in"));
    }
    let challenge = response
        .headers()
        .get_all(header::WWW_AUTHENTICATE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .map(bearer_params)
        .find(|p| !p.is_empty())
        .unwrap_or_default();
    let param = |name: &str| {
        challenge
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.clone())
    };
    let places = match param("resource_metadata") {
        Some(url) => {
            parse_secure(&url, "resource metadata URL")?;
            vec![url]
        }
        None => well_known(&server, "oauth-protected-resource"),
    };
    let mut metadata: Option<ResourceMetadata> = None;
    for place in places {
        if let Some(found) = get_json(http, &place).await? {
            metadata = Some(found);
            break;
        }
    }
    let (resource, issuer, scopes) = match metadata {
        Some(m) => {
            let resource = m.resource.unwrap_or_else(|| server_url.to_string());
            let at = parse_secure(&resource, "resource")?;
            if at.origin() != server.origin() {
                return Err(invalid(
                    "the server's metadata names a resource on another site",
                ));
            }
            let issuer = m
                .authorization_servers
                .into_iter()
                .next()
                .ok_or_else(|| invalid("the server names no sign-in server"))?;
            (resource, issuer, m.scopes_supported)
        }
        // No metadata: the server's own origin is its authorization server (the 2025-03-26
        // form, which some servers still use).
        None => (
            server_url.to_string(),
            server.origin().ascii_serialization(),
            Vec::new(),
        ),
    };
    let auth = auth_server(http, &issuer).await?;
    let scope = param("scope")
        .filter(|s| !s.trim().is_empty())
        .or_else(|| (!scopes.is_empty()).then(|| scopes.join(" ")));
    Ok(Discovered {
        resource,
        scope,
        server: auth,
    })
}

/// The metadata of the authorization server `issuer`, checked: it names itself, offers S256,
/// lets clients register, and every endpoint is https.
pub async fn auth_server(http: &Client, issuer: &str) -> Result<AuthServer> {
    let url = parse_secure(issuer, "sign-in server")?;
    let origin = url.origin().ascii_serialization();
    let path = url.path().trim_end_matches('/');
    let places = if path.is_empty() {
        vec![
            format!("{origin}/.well-known/oauth-authorization-server"),
            format!("{origin}/.well-known/openid-configuration"),
        ]
    } else {
        vec![
            format!("{origin}/.well-known/oauth-authorization-server{path}"),
            format!("{origin}/.well-known/openid-configuration{path}"),
            format!("{origin}{path}/.well-known/openid-configuration"),
        ]
    };
    let mut found: Option<AuthServer> = None;
    for place in places {
        if let Some(meta) = get_json(http, &place).await? {
            found = Some(meta);
            break;
        }
    }
    let meta = found.ok_or_else(|| invalid("the sign-in server publishes no metadata"))?;
    if meta.issuer.trim_end_matches('/') != issuer.trim_end_matches('/') {
        return Err(invalid(
            "the sign-in server's metadata names another server",
        ));
    }
    if !meta
        .code_challenge_methods_supported
        .iter()
        .any(|m| m == "S256")
    {
        return Err(invalid(
            "the sign-in server does not offer PKCE (S256), so Dots does not sign in to it",
        ));
    }
    parse_secure(&meta.authorization_endpoint, "authorization endpoint")?;
    parse_secure(&meta.token_endpoint, "token endpoint")?;
    match &meta.registration_endpoint {
        Some(r) => {
            parse_secure(r, "registration endpoint")?;
        }
        None => {
            return Err(invalid(
                "the sign-in server does not let apps register, so Dots cannot sign in to it yet",
            ))
        }
    }
    Ok(meta)
}

#[derive(Deserialize)]
struct RegistrationAnswer {
    client_id: String,
    #[serde(default)]
    client_secret: Option<String>,
    #[serde(default)]
    token_endpoint_auth_method: Option<String>,
}

/// Registers Dots with the authorization server (RFC 7591) for `redirect_uri`, as a public
/// client. A server that answers with a secret makes it a confidential one.
pub async fn register(
    http: &Client,
    server: &AuthServer,
    redirect_uri: &str,
) -> Result<Registration> {
    let endpoint = server
        .registration_endpoint
        .as_deref()
        .ok_or_else(|| invalid("the sign-in server does not let apps register"))?;
    let body = serde_json::json!({
        "client_name": CLIENT_NAME,
        "redirect_uris": [redirect_uri],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
    });
    let response = http
        .post(endpoint)
        .json(&body)
        .send()
        .await
        .map_err(|_| unreachable_error())?;
    if !response.status().is_success() {
        return Err(Error::Other(format!(
            "The sign-in server refused to register Dots (HTTP {}).",
            response.status().as_u16()
        )));
    }
    let answer: RegistrationAnswer = response
        .json()
        .await
        .map_err(|_| invalid("the sign-in server's registration answer could not be read"))?;
    if answer.client_id.trim().is_empty() {
        return Err(invalid("the sign-in server gave no client id"));
    }
    let auth_method = match (
        &answer.client_secret,
        answer.token_endpoint_auth_method.as_deref(),
    ) {
        (None, _) => "none".to_string(),
        (Some(_), Some("client_secret_post")) => "client_secret_post".to_string(),
        (Some(_), _) => "client_secret_basic".to_string(),
    };
    Ok(Registration {
        client_id: answer.client_id,
        client_secret: answer.client_secret.filter(|s| !s.is_empty()),
        auth_method,
    })
}

#[derive(Deserialize)]
struct TokenAnswer {
    access_token: String,
    #[serde(default)]
    token_type: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    scope: Option<String>,
}

#[derive(Deserialize)]
struct TokenError {
    #[serde(default)]
    error: String,
}

/// POSTs `form` to the token endpoint with the client's authentication.
async fn token_request(
    http: &Client,
    endpoint: &str,
    client: &Registration,
    mut form: Vec<(&str, String)>,
) -> std::result::Result<reqwest::Response, reqwest::Error> {
    let mut request = http
        .post(endpoint)
        .header(header::ACCEPT, "application/json");
    match (client.auth_method.as_str(), &client.client_secret) {
        ("client_secret_basic", Some(secret)) => {
            request = request.basic_auth(form_encode(&client.client_id), Some(form_encode(secret)));
        }
        ("client_secret_post", Some(secret)) => {
            form.push(("client_id", client.client_id.clone()));
            form.push(("client_secret", secret.clone()));
        }
        _ => form.push(("client_id", client.client_id.clone())),
    }
    request.form(&form).send().await
}

/// RFC 6749 §2.3.1: the client id and secret are form-encoded before they go in Basic auth.
fn form_encode(text: &str) -> String {
    url_encode(text)
}

fn url_encode(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            b => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// A token answer read into a grant and access token. `previous` keeps a refresh token the
/// answer leaves out.
fn signed(
    answer: TokenAnswer,
    base: OAuthGrant,
    previous_refresh: Option<String>,
) -> Result<Signed> {
    if answer
        .token_type
        .as_deref()
        .is_some_and(|t| !t.eq_ignore_ascii_case("bearer"))
    {
        return Err(invalid("the sign-in server gave a token Dots cannot use"));
    }
    if answer.access_token.is_empty() {
        return Err(invalid("the sign-in server gave an empty token"));
    }
    let expires_at = answer
        .expires_in
        .filter(|s| *s > 0)
        .map(|s| Utc::now() + chrono::Duration::seconds(s));
    Ok(Signed {
        grant: OAuthGrant {
            refresh_token: answer.refresh_token.or(previous_refresh),
            scope: answer.scope.or(base.scope.clone()),
            expires_at,
            needs_sign_in: false,
            ..base
        },
        access_token: Zeroizing::new(answer.access_token),
    })
}

/// Gets a new access token with the grant's refresh token. The new grant keeps the old refresh
/// token when the server does not rotate it.
pub async fn refresh(
    http: &Client,
    grant: &OAuthGrant,
) -> std::result::Result<Signed, RefreshFailure> {
    let Some(refresh_token) = grant.refresh_token.clone() else {
        return Err(RefreshFailure::Rejected);
    };
    let form = vec![
        ("grant_type", "refresh_token".to_string()),
        ("refresh_token", refresh_token.clone()),
        ("resource", grant.resource.clone()),
    ];
    let response = token_request(http, &grant.token_endpoint, &grant.client, form)
        .await
        .map_err(|_| RefreshFailure::Failed("could not reach the sign-in server".into()))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.bytes().await.unwrap_or_default();
        let error = serde_json::from_slice::<TokenError>(&body)
            .map(|e| e.error)
            .unwrap_or_default();
        let refused = matches!(
            error.as_str(),
            "invalid_grant" | "invalid_client" | "unauthorized_client"
        ) || status == StatusCode::UNAUTHORIZED
            || (status == StatusCode::BAD_REQUEST && error.is_empty());
        return Err(if refused {
            RefreshFailure::Rejected
        } else {
            RefreshFailure::Failed(format!(
                "the sign-in server answered HTTP {}",
                status.as_u16()
            ))
        });
    }
    let answer: TokenAnswer = response.json().await.map_err(|_| {
        RefreshFailure::Failed("the sign-in server's answer could not be read".into())
    })?;
    signed(answer, grant.clone(), Some(refresh_token))
        .map_err(|e| RefreshFailure::Failed(e.to_string()))
}

/// Signs in to the MCP server at `server_url`: discovery, registration, the browser, the code
/// exchange. `open_browser` opens the authorization URL. Ends with an error after `limit`, or
/// when `cancel` fires ("Sign-in cancelled.", a `Conflict`).
pub async fn sign_in<F>(
    server_url: &str,
    open_browser: F,
    limit: Duration,
    cancel: CancellationToken,
) -> Result<Signed>
where
    F: FnOnce(&str) -> Result<()> + Send,
{
    let http = client();
    let discovered = discover(&http, server_url).await?;
    // A free port on the loopback interface (RFC 8252 §7.3); the registration names it.
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|_| Error::Other("Could not open the sign-in callback on this PC.".into()))?;
    let port = listener
        .local_addr()
        .map_err(|_| Error::Other("Could not open the sign-in callback on this PC.".into()))?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}{CALLBACK_PATH}");
    let client = register(&http, &discovered.server, &redirect_uri).await?;

    let state = CsrfToken::new_random();
    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let (send, receive) = oneshot::channel();
    let callback = Arc::new(Callback {
        state: state.secret().clone(),
        issuer: discovered.server.issuer.trim_end_matches('/').to_string(),
        send: Mutex::new(Some(send)),
    });
    let router = Router::new()
        .route(CALLBACK_PATH, get(callback_handler))
        .with_state(callback);
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    let _server = AbortOnDrop(server);

    let mut authorize = parse_secure(
        &discovered.server.authorization_endpoint,
        "authorization endpoint",
    )?;
    {
        let mut query = authorize.query_pairs_mut();
        query.extend_pairs([
            ("response_type", "code"),
            ("client_id", client.client_id.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("state", state.secret().as_str()),
            ("resource", discovered.resource.as_str()),
        ]);
        if let Some(scope) = &discovered.scope {
            query.append_pair("scope", scope);
        }
    }

    let flow = async {
        open_browser(authorize.as_str())?;
        let code = receive
            .await
            .map_err(|_| invalid("sign-in could not be verified. Try again."))??;
        let form = vec![
            ("grant_type", "authorization_code".to_string()),
            ("code", code.to_string()),
            ("redirect_uri", redirect_uri.clone()),
            ("code_verifier", verifier.secret().clone()),
            ("resource", discovered.resource.clone()),
        ];
        let response = token_request(&http, &discovered.server.token_endpoint, &client, form)
            .await
            .map_err(|_| unreachable_error())?;
        if !response.status().is_success() {
            return Err(Error::Other(format!(
                "The sign-in server refused the sign-in (HTTP {}). Try again.",
                response.status().as_u16()
            )));
        }
        let answer: TokenAnswer = response
            .json()
            .await
            .map_err(|_| invalid("the sign-in server's answer could not be read"))?;
        let base = OAuthGrant {
            client: client.clone(),
            issuer: discovered.server.issuer.clone(),
            token_endpoint: discovered.server.token_endpoint.clone(),
            resource: discovered.resource.clone(),
            scope: discovered.scope.clone(),
            refresh_token: None,
            expires_at: None,
            needs_sign_in: false,
        };
        signed(answer, base, None)
    };
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(Error::Conflict("Sign-in cancelled.".into())),
        _ = tokio::time::sleep(limit) => Err(Error::Other("Sign-in timed out. Try again.".into())),
        result = flow => result,
    }
}

struct AbortOnDrop(tokio::task::JoinHandle<()>);
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct Callback {
    state: String,
    issuer: String,
    send: Mutex<Option<oneshot::Sender<Result<Zeroizing<String>>>>>,
}

async fn callback_handler(State(cb): State<Arc<Callback>>, RawQuery(raw): RawQuery) -> Response {
    let Ok(url) = Url::parse(&format!("http://127.0.0.1/?{}", raw.unwrap_or_default())) else {
        return page(StatusCode::BAD_REQUEST, "Sign-in could not be verified.");
    };
    let pairs: Vec<(String, String)> = url.query_pairs().into_owned().collect();
    let values = |key: &str| -> Vec<&str> {
        pairs
            .iter()
            .filter(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
            .collect()
    };
    // The state is checked first: a request without the right one changes nothing.
    let state = values("state");
    if state.len() != 1 || state[0] != cb.state {
        return page(StatusCode::BAD_REQUEST, "Sign-in could not be verified.");
    }
    let Some(send) = cb.send.lock().expect("callback sender").take() else {
        return page(StatusCode::GONE, "This sign-in has already been used.");
    };
    let code = values("code");
    let issuer = values("iss");
    let result = if !values("error").is_empty() {
        Err(invalid(
            "sign-in was declined or could not be completed. Try again.",
        ))
    } else if issuer.len() > 1
        || issuer
            .first()
            .is_some_and(|iss| iss.trim_end_matches('/') != cb.issuer)
        || code.len() != 1
        || code[0].is_empty()
    {
        Err(invalid("sign-in could not be verified. Try again."))
    } else {
        Ok(Zeroizing::new(code[0].to_string()))
    };
    let ok = result.is_ok();
    let _ = send.send(result);
    if ok {
        page(
            StatusCode::OK,
            "Signed in. You can close this tab and go back to Dots.",
        )
    } else {
        page(
            StatusCode::OK,
            "Sign-in did not finish. Go back to Dots to try again.",
        )
    }
}

fn page(status: StatusCode, message: &str) -> Response {
    (
        status,
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::REFERRER_POLICY, "no-referrer"),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; frame-ancestors 'none'",
            ),
        ],
        Html(format!(
            "<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><title>Dots sign-in</title>\
             <body><h1>Dots</h1><p>{message}</p></body></html>"
        )),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bearer_challenge_is_read_with_quoted_and_bare_values() {
        let p = bearer_params(
            r#"Bearer realm="OAuth", resource_metadata="https://x.example/.well-known/oauth-protected-resource/mcp", scope="read write", error=invalid_token"#,
        );
        let get = |n: &str| p.iter().find(|(k, _)| k == n).map(|(_, v)| v.as_str());
        assert_eq!(
            get("resource_metadata"),
            Some("https://x.example/.well-known/oauth-protected-resource/mcp")
        );
        assert_eq!(get("scope"), Some("read write"));
        assert_eq!(get("error"), Some("invalid_token"));
        assert!(bearer_params("Basic realm=\"x\"").is_empty());
        let escaped = bearer_params(r#"bearer realm="a \"b\"""#);
        assert_eq!(escaped, vec![("realm".to_string(), "a \"b\"".to_string())]);
    }

    #[test]
    fn only_https_or_loopback_http_carries_the_flow() {
        let ok = |u: &str| secure(&Url::parse(u).unwrap());
        assert!(ok("https://mcp.example/mcp"));
        assert!(ok("http://127.0.0.1:8080/x"));
        assert!(ok("http://localhost/x"));
        assert!(!ok("http://mcp.example/mcp"));
        assert!(!ok("https://user:pw@mcp.example/"));
        assert!(!ok("https://mcp.example/#frag"));
    }

    #[test]
    fn well_known_places_put_the_path_after_the_name() {
        let url = Url::parse("https://mcp.example/v1/mcp").unwrap();
        assert_eq!(
            well_known(&url, "oauth-protected-resource"),
            vec![
                "https://mcp.example/.well-known/oauth-protected-resource/v1/mcp".to_string(),
                "https://mcp.example/.well-known/oauth-protected-resource".to_string(),
            ]
        );
        let bare = Url::parse("https://mcp.example").unwrap();
        assert_eq!(
            well_known(&bare, "oauth-protected-resource"),
            vec!["https://mcp.example/.well-known/oauth-protected-resource".to_string()]
        );
    }

    #[test]
    fn a_token_near_its_expiry_needs_a_refresh() {
        let now = Utc::now();
        let mut grant = OAuthGrant {
            client: Registration {
                client_id: "c".into(),
                client_secret: None,
                auth_method: "none".into(),
            },
            issuer: "https://a.example".into(),
            token_endpoint: "https://a.example/token".into(),
            resource: "https://m.example/mcp".into(),
            scope: None,
            refresh_token: Some("r".into()),
            expires_at: Some(now + chrono::Duration::minutes(3)),
            needs_sign_in: false,
        };
        assert!(grant.expires_within(now, Duration::from_secs(300)));
        grant.expires_at = Some(now + chrono::Duration::hours(1));
        assert!(!grant.expires_within(now, Duration::from_secs(300)));
        grant.expires_at = None;
        assert!(!grant.expires_within(now, Duration::from_secs(300)));
        // The debug form never shows the refresh token.
        assert!(!format!("{grant:?}").contains("\"r\""));
    }
}
