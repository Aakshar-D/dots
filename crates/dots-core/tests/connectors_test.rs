//! Connectors (connectors spec): a key connector connected, given to dots, changed and
//! disconnected through the Library and the vault; a sign-in connector signed in against a fake
//! authorization server (discovery, registration, PKCE, `resource`, state), its token refreshed
//! when due or refused, and "sign in again" when a refresh is refused.

mod common;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{RawQuery, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Form, Json, Router};
use base64::Engine;
use common::fake_mcp::{tool, FakeMcp};
use common::spec;
use dots_core::connectors::catalog::variable_for;
use dots_core::connectors::{
    AuthKind, Category, Connector, ConnectorState, FieldFormat, Instructions, KeyField,
    ServerTemplate, Trust,
};
use dots_core::library::{CatalogStatus, LibraryConfig, LibraryKind, McpServerConfig};
use dots_core::mcp::oauth;
use dots_core::model::{Dot, WorkspaceMode};
use dots_core::vault::{Consumer, DotScope, EntryKind, MemorySecrets};
use dots_core::{Config, Error, Runtime};
use serde_json::{json, Value};
use sha2::Digest;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

struct Rt {
    dir: TempDir,
    rt: Runtime,
}

async fn start() -> Rt {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.secrets = Some(Arc::new(MemorySecrets::new()));
    let rt = Runtime::start(cfg).await.unwrap();
    Rt { dir, rt }
}

impl Rt {
    async fn dot(&self, name: &str) -> Dot {
        let mut s = spec(name);
        s.workdir = self.dir.path().to_string_lossy().to_string();
        s.workspace_mode = WorkspaceMode::Folder;
        self.rt.create_dot(s, None, None).await.unwrap()
    }

    async fn uses(&self, dot: &Dot) -> Vec<String> {
        self.rt
            .store()
            .get_dot(&dot.id)
            .await
            .unwrap()
            .spec
            .uses
            .mcp_servers
    }

    async fn granted(&self, dot: &Dot, server: &str) -> Vec<String> {
        self.rt
            .store()
            .get_dot(&dot.id)
            .await
            .unwrap()
            .spec
            .variables
            .names(&Consumer::Mcp(server.into()))
            .to_vec()
    }
}

/// A key connector on `fake-mcp` over stdio, which records the `FAKE_MCP_` variables it gets.
fn key_connector(fake: &FakeMcp) -> Connector {
    let McpServerConfig::Stdio { command, args, .. } = fake.stdio() else {
        unreachable!()
    };
    Connector {
        id: "fakeapp".into(),
        name: "Fake App".into(),
        description: "A fake app.".into(),
        category: Category::Developer,
        icon: "fakeapp".into(),
        maker: "Tests".into(),
        trust: Trust::Official,
        source_url: "https://example.com".into(),
        auth: AuthKind::Key,
        server: Some(ServerTemplate::Stdio {
            command,
            args,
            env: [(
                "FAKE_MCP_TOKEN".to_string(),
                format!("${{{}}}", variable_for("fakeapp", "TOKEN")),
            )]
            .into(),
        }),
        runtime: None,
        fields: vec![KeyField {
            key: "token".into(),
            label: "Token".into(),
            variable: variable_for("fakeapp", "TOKEN"),
            secret: true,
            placeholder: "fk_…".into(),
            hint: "Starts with fk_.".into(),
            format: FieldFormat::Prefix {
                prefixes: vec!["fk_".into()],
            },
            strip_spaces: false,
        }],
        instructions: Instructions {
            steps: vec!["Make a token.".into()],
            open_url: "https://example.com/tokens".into(),
            open_label: "Open Fake App".into(),
        },
        abilities: vec!["Do fake things".into()],
        note: None,
    }
}

fn values(token: &str) -> BTreeMap<String, String> {
    [("token".to_string(), token.to_string())].into()
}

#[tokio::test]
async fn a_key_connector_stores_its_key_in_personal_and_gives_its_server_to_the_dots_picked() {
    let t = start().await;
    let fake = FakeMcp::new(json!({"tools": [tool("search", true), tool("send", false)]}));
    let connector = key_connector(&fake);
    let alpha = t.dot("alpha").await;
    let beta = t.dot("beta").await;

    let entry =
        t.rt.connect_with(
            &connector,
            &values("  fk_secret "),
            std::slice::from_ref(&alpha.id),
        )
        .await
        .unwrap();
    assert_eq!(entry.state, ConnectorState::Connected);
    assert_eq!(entry.tools, Some(2));
    assert_eq!(entry.catalog_status, Some(CatalogStatus::Accepted));
    assert_eq!(
        entry
            .used_by
            .iter()
            .map(|d| d.name.as_str())
            .collect::<Vec<_>>(),
        ["alpha"]
    );

    // The key is a Personal secret; the Library config holds only its name.
    let vars = t.rt.vault().list().await.unwrap();
    let var = vars
        .iter()
        .find(|v| v.name == "CONNECTOR_FAKEAPP_TOKEN")
        .unwrap();
    assert_eq!(var.kind, EntryKind::Secret);
    assert_eq!(var.value, None);
    let item =
        t.rt.store()
            .find_library_item(LibraryKind::McpServer, "fakeapp")
            .await
            .unwrap()
            .unwrap();
    let text = serde_json::to_string(&item.config).unwrap();
    assert!(!text.contains("fk_secret"), "{text}");
    assert!(text.contains("${CONNECTOR_FAKEAPP_TOKEN}"), "{text}");
    // The check started the server with the trimmed value.
    assert!(fake
        .log()
        .iter()
        .any(|r| r["start"]["env"]["FAKE_MCP_TOKEN"] == "fk_secret"));

    // The dot picked uses it and is granted its variable; the other is not.
    assert_eq!(t.uses(&alpha).await, ["fakeapp"]);
    assert_eq!(
        t.granted(&alpha, "fakeapp").await,
        ["CONNECTOR_FAKEAPP_TOKEN"]
    );
    assert!(t.uses(&beta).await.is_empty());

    // Changing the dots moves it, grants included.
    t.rt.connect_with(
        &connector,
        &values("fk_secret"),
        std::slice::from_ref(&beta.id),
    )
    .await
    .unwrap();
    assert!(t.uses(&alpha).await.is_empty());
    assert!(t.granted(&alpha, "fakeapp").await.is_empty());
    assert_eq!(t.uses(&beta).await, ["fakeapp"]);

    // Disconnect takes it out of the dots, removes the server and forgets the key.
    t.rt.disconnect_with(&connector).await.unwrap();
    assert!(t.uses(&beta).await.is_empty());
    assert!(t.granted(&beta, "fakeapp").await.is_empty());
    assert!(t
        .rt
        .store()
        .find_library_item(LibraryKind::McpServer, "fakeapp")
        .await
        .unwrap()
        .is_none());
    assert!(!t
        .rt
        .vault()
        .list()
        .await
        .unwrap()
        .iter()
        .any(|v| v.name == "CONNECTOR_FAKEAPP_TOKEN"));
    let entry = t.rt.connector_entry_for(&connector).await.unwrap();
    assert_eq!(entry.state, ConnectorState::NotConnected);
}

#[tokio::test]
async fn a_key_in_the_wrong_form_stores_nothing() {
    let t = start().await;
    let fake = FakeMcp::new(json!({"tools": [tool("search", true)]}));
    let connector = key_connector(&fake);
    let e =
        t.rt.connect_with(&connector, &values("nope"), &[])
            .await
            .unwrap_err();
    assert!(
        matches!(&e, Error::Invalid(m) if m.contains("start with fk_")),
        "{e}"
    );
    assert!(t.rt.vault().list().await.unwrap().is_empty());
    assert!(t.rt.library().list().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_server_of_the_persons_own_with_the_name_is_left_alone() {
    let t = start().await;
    let fake = FakeMcp::new(json!({"tools": [tool("search", true)]}));
    let connector = key_connector(&fake);
    let own = LibraryConfig::McpServer(McpServerConfig::Http {
        url: "https://example.com/mcp".into(),
        headers: BTreeMap::new(),
    });
    t.rt.library().add("fakeapp", own, None).await.unwrap();
    let e =
        t.rt.connect_with(&connector, &values("fk_x"), &[])
            .await
            .unwrap_err();
    assert!(
        matches!(&e, Error::Conflict(m) if m.contains("of your own")),
        "{e}"
    );
    assert!(t.rt.vault().list().await.unwrap().is_empty());
    let entry = t.rt.connector_entry_for(&connector).await.unwrap();
    assert_eq!(entry.state, ConnectorState::NameTaken);
}

#[tokio::test]
async fn a_server_that_refuses_the_key_shows_as_an_error_and_keeps_the_connector() {
    let t = start().await;
    // `fake-mcp` exits at initialize: the check fails.
    let fake = FakeMcp::new(json!({"tools": [], "exit_at_initialize": true}));
    let connector = key_connector(&fake);
    let entry =
        t.rt.connect_with(&connector, &values("fk_bad"), &[])
            .await
            .unwrap();
    assert_eq!(entry.state, ConnectorState::Error);
    assert!(entry.error.is_some());
    assert!(entry.server_id.is_some());
}

// ---- Sign-in ----------------------------------------------------------------------------

/// A fake authorization server and MCP server in one: it checks what the spec asks of a client
/// and accepts only the access token it gave last.
#[derive(Default)]
struct Auth {
    base: String,
    /// The PKCE challenge, redirect, client and resource of the last authorization request.
    challenge: String,
    redirect: String,
    client_id: String,
    resource_asked: Vec<String>,
    codes: u32,
    access: String,
    refresh: String,
    expires_in: i64,
    refreshes: u32,
    refuse_refresh: bool,
    registrations: u32,
    /// The server publishes no protected resource metadata (the 2025-03-26 form).
    no_metadata: bool,
}

type Shared = Arc<Mutex<Auth>>;

fn query(raw: Option<String>) -> BTreeMap<String, String> {
    reqwest::Url::parse(&format!("http://x/?{}", raw.unwrap_or_default()))
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect()
}

async fn resource_metadata(State(s): State<Shared>) -> Response {
    let a = s.lock().unwrap();
    if a.no_metadata {
        return StatusCode::NOT_FOUND.into_response();
    }
    Json(json!({
        "resource": format!("{}/mcp", a.base),
        "authorization_servers": [a.base.clone()],
        "scopes_supported": ["read", "write"],
    }))
    .into_response()
}

async fn auth_metadata(State(s): State<Shared>) -> Json<Value> {
    let base = s.lock().unwrap().base.clone();
    Json(json!({
        "issuer": base,
        "authorization_endpoint": format!("{base}/authorize"),
        "token_endpoint": format!("{base}/token"),
        "registration_endpoint": format!("{base}/register"),
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
    }))
}

async fn register(State(s): State<Shared>, Json(body): Json<Value>) -> Json<Value> {
    let mut a = s.lock().unwrap();
    a.registrations += 1;
    assert_eq!(body["token_endpoint_auth_method"], "none");
    let redirect = body["redirect_uris"][0].as_str().unwrap().to_string();
    assert!(redirect.starts_with("http://127.0.0.1:"), "{redirect}");
    a.client_id = format!("client-{}", a.registrations);
    Json(json!({"client_id": a.client_id, "redirect_uris": [redirect]}))
}

async fn authorize(State(s): State<Shared>, RawQuery(raw): RawQuery) -> Response {
    let q = query(raw);
    let mut a = s.lock().unwrap();
    assert_eq!(q["response_type"], "code");
    assert_eq!(q["code_challenge_method"], "S256");
    assert_eq!(q["client_id"], a.client_id);
    assert_eq!(q["scope"], "read write");
    a.resource_asked.push(q["resource"].clone());
    a.challenge = q["code_challenge"].clone();
    a.redirect = q["redirect_uri"].clone();
    a.codes += 1;
    let to = format!(
        "{}?code=code-{}&state={}&iss={}",
        a.redirect, a.codes, q["state"], a.base
    );
    (StatusCode::FOUND, [("location", to)]).into_response()
}

async fn token(State(s): State<Shared>, Form(form): Form<BTreeMap<String, String>>) -> Response {
    let mut a = s.lock().unwrap();
    assert_eq!(form["client_id"], a.client_id);
    a.resource_asked.push(form["resource"].clone());
    match form["grant_type"].as_str() {
        "authorization_code" => {
            let digest = sha2::Sha256::digest(form["code_verifier"].as_bytes());
            let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest);
            if challenge != a.challenge || form["redirect_uri"] != a.redirect {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": "invalid_grant"})),
                )
                    .into_response();
            }
        }
        "refresh_token" => {
            if a.refuse_refresh || form["refresh_token"] != a.refresh {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": "invalid_grant"})),
                )
                    .into_response();
            }
            a.refreshes += 1;
        }
        other => panic!("unexpected grant {other}"),
    }
    let n = a.codes * 100 + a.refreshes;
    a.access = format!("access-{n}");
    a.refresh = format!("refresh-{n}");
    Json(json!({
        "access_token": a.access,
        "token_type": "Bearer",
        "expires_in": a.expires_in,
        "refresh_token": a.refresh,
    }))
    .into_response()
}

async fn mcp(State(s): State<Shared>, headers: HeaderMap, Json(msg): Json<Value>) -> Response {
    let (access, base) = {
        let a = s.lock().unwrap();
        (a.access.clone(), a.base.clone())
    };
    let auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if access.is_empty() || auth != format!("Bearer {access}") {
        let challenge = format!(
            "Bearer resource_metadata=\"{base}/.well-known/oauth-protected-resource/mcp\", scope=\"read write\""
        );
        let no_metadata = s.lock().unwrap().no_metadata;
        let value = if no_metadata {
            "Bearer realm=\"OAuth\"".to_string()
        } else {
            challenge
        };
        return (StatusCode::UNAUTHORIZED, [("www-authenticate", value)]).into_response();
    }
    let id = msg["id"].clone();
    match msg["method"].as_str().unwrap_or("") {
        "initialize" => Json(json!({"jsonrpc": "2.0", "id": id, "result": {
            "protocolVersion": "2025-06-18",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "fake", "version": "1"}
        }}))
        .into_response(),
        "tools/list" => Json(json!({"jsonrpc": "2.0", "id": id, "result": {
            "tools": [tool("find", true)]
        }}))
        .into_response(),
        _ => StatusCode::ACCEPTED.into_response(),
    }
}

async fn fake_auth(expires_in: i64) -> Shared {
    let shared: Shared = Arc::new(Mutex::new(Auth {
        expires_in,
        ..Default::default()
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    shared.lock().unwrap().base = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new()
        .route(
            "/.well-known/oauth-protected-resource/mcp",
            get(resource_metadata),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            get(auth_metadata),
        )
        .route("/register", post(register))
        .route("/authorize", get(authorize))
        .route("/token", post(token))
        .route(
            "/mcp",
            post(mcp)
                .get(|| async { StatusCode::METHOD_NOT_ALLOWED })
                .delete(|| async { StatusCode::OK }),
        )
        .with_state(shared.clone());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    shared
}

fn oauth_connector(base: &str, id: &str) -> Connector {
    Connector {
        id: id.into(),
        name: "Fake Auth".into(),
        description: "A fake app with sign-in.".into(),
        category: Category::Work,
        icon: "fakeauth".into(),
        maker: "Tests".into(),
        trust: Trust::Official,
        source_url: "https://example.com".into(),
        auth: AuthKind::OAuth,
        server: Some(ServerTemplate::Http {
            url: format!("{base}/mcp"),
            bearer: Some(variable_for(id, "ACCESS_TOKEN")),
        }),
        runtime: None,
        fields: Vec::new(),
        instructions: Instructions {
            steps: vec!["Sign in.".into()],
            open_url: "https://example.com".into(),
            open_label: "Read the guide".into(),
        },
        abilities: Vec::new(),
        note: None,
    }
}

/// Opens the authorization URL as a browser would: follows the redirect to the callback.
fn browser(url: &str) -> dots_core::Result<()> {
    let url = url.to_string();
    tokio::spawn(async move {
        let _ = reqwest::get(&url).await;
    });
    Ok(())
}

#[tokio::test]
async fn signing_in_registers_dots_and_keeps_a_token_the_server_takes() {
    let auth = fake_auth(3600).await;
    let base = auth.lock().unwrap().base.clone();
    let t = start().await;
    let alpha = t.dot("alpha").await;
    let connector = oauth_connector(&base, "fakeauth");

    let entry =
        t.rt.sign_in_with(
            &connector,
            std::slice::from_ref(&alpha.id),
            browser,
            Duration::from_secs(20),
        )
        .await
        .unwrap();
    assert_eq!(entry.state, ConnectorState::Connected, "{:?}", entry.error);
    assert_eq!(entry.tools, Some(1));
    assert_eq!(t.uses(&alpha).await, ["fakeauth"]);
    assert_eq!(
        t.granted(&alpha, "fakeauth").await,
        ["CONNECTOR_FAKEAUTH_ACCESS_TOKEN"]
    );
    {
        let a = auth.lock().unwrap();
        // The `resource` went with the authorization and the token request.
        assert_eq!(
            a.resource_asked,
            [format!("{base}/mcp"), format!("{base}/mcp")]
        );
        assert_eq!(a.registrations, 1);
    }
    // The token is a Personal secret; the Library config sends it as a Bearer header.
    let var =
        t.rt.vault()
            .list()
            .await
            .unwrap()
            .into_iter()
            .find(|v| v.name == "CONNECTOR_FAKEAUTH_ACCESS_TOKEN")
            .unwrap();
    assert_eq!(var.kind, EntryKind::Secret);
    let item =
        t.rt.store()
            .find_library_item(LibraryKind::McpServer, "fakeauth")
            .await
            .unwrap()
            .unwrap();
    let LibraryConfig::McpServer(McpServerConfig::Http { headers, .. }) = &item.config else {
        panic!("an http server")
    };
    assert_eq!(
        headers["Authorization"],
        "Bearer ${CONNECTOR_FAKEAUTH_ACCESS_TOKEN}"
    );

    // Disconnect forgets the token and the grant.
    t.rt.disconnect_with(&connector).await.unwrap();
    assert!(!t
        .rt
        .vault()
        .list()
        .await
        .unwrap()
        .iter()
        .any(|v| v.name == "CONNECTOR_FAKEAUTH_ACCESS_TOKEN"));
}

#[tokio::test]
async fn a_token_about_to_expire_is_refreshed_before_a_check() {
    // Two minutes left: under the five-minute margin.
    let auth = fake_auth(120).await;
    let base = auth.lock().unwrap().base.clone();
    let t = start().await;
    let connector = oauth_connector(&base, "expiring");
    let entry =
        t.rt.sign_in_with(&connector, &[], browser, Duration::from_secs(20))
            .await
            .unwrap();
    // The connect's own check refreshed it already.
    let refreshes = auth.lock().unwrap().refreshes;
    assert!(refreshes >= 1, "refreshed {refreshes} times");
    let id = entry.server_id.unwrap();
    let catalog =
        t.rt.library()
            .check(&id, &DotScope::Personal)
            .await
            .unwrap();
    assert_eq!(
        catalog.status,
        CatalogStatus::Accepted,
        "{:?}",
        catalog.error
    );
    assert!(auth.lock().unwrap().refreshes > refreshes);
}

#[tokio::test]
async fn a_token_the_server_refuses_is_refreshed_and_the_check_tried_again() {
    let auth = fake_auth(3600).await;
    let base = auth.lock().unwrap().base.clone();
    let t = start().await;
    let connector = oauth_connector(&base, "revoked");
    let entry =
        t.rt.sign_in_with(&connector, &[], browser, Duration::from_secs(20))
            .await
            .unwrap();
    // The server forgets the token it gave (it still knows the refresh token).
    auth.lock().unwrap().access = "revoked".into();
    let id = entry.server_id.unwrap();
    let catalog =
        t.rt.library()
            .check(&id, &DotScope::Personal)
            .await
            .unwrap();
    assert_eq!(
        catalog.status,
        CatalogStatus::Accepted,
        "{:?}",
        catalog.error
    );
    assert_eq!(auth.lock().unwrap().refreshes, 1);
}

#[tokio::test]
async fn a_refused_refresh_asks_the_person_to_sign_in_again() {
    let auth = fake_auth(3600).await;
    let base = auth.lock().unwrap().base.clone();
    let t = start().await;
    let connector = oauth_connector(&base, "refused");
    let entry =
        t.rt.sign_in_with(&connector, &[], browser, Duration::from_secs(20))
            .await
            .unwrap();
    {
        let mut a = auth.lock().unwrap();
        a.access = "revoked".into();
        a.refuse_refresh = true;
    }
    let id = entry.server_id.unwrap();
    let catalog =
        t.rt.library()
            .check(&id, &DotScope::Personal)
            .await
            .unwrap();
    assert_eq!(catalog.status, CatalogStatus::Unreachable);
    assert!(catalog.error.unwrap().contains("sign in again"));
    let entry = t.rt.connector_entry_for(&connector).await.unwrap();
    assert_eq!(entry.state, ConnectorState::NeedsSignIn);

    // Signing in again clears it.
    auth.lock().unwrap().refuse_refresh = false;
    let entry =
        t.rt.sign_in_with(&connector, &[], browser, Duration::from_secs(20))
            .await
            .unwrap();
    assert_eq!(entry.state, ConnectorState::Connected, "{:?}", entry.error);
}

#[tokio::test]
async fn a_server_without_metadata_signs_in_at_its_own_origin() {
    let auth = fake_auth(3600).await;
    let base = {
        let mut a = auth.lock().unwrap();
        a.no_metadata = true;
        a.base.clone()
    };
    let found = oauth::discover(&oauth::client(), &format!("{base}/mcp"))
        .await
        .unwrap();
    assert_eq!(found.server.issuer, base);
    assert_eq!(found.resource, format!("{base}/mcp"));
    assert_eq!(found.scope, None);
}

#[tokio::test]
async fn a_sign_in_nobody_finishes_times_out_and_one_cancelled_says_so() {
    let auth = fake_auth(3600).await;
    let base = auth.lock().unwrap().base.clone();
    let url = format!("{base}/mcp");
    let e = oauth::sign_in(
        &url,
        |_| Ok(()),
        Duration::from_millis(300),
        CancellationToken::new(),
    )
    .await
    .err()
    .unwrap();
    assert!(e.to_string().contains("timed out"), "{e}");

    let cancel = CancellationToken::new();
    let stop = cancel.clone();
    let e = oauth::sign_in(
        &url,
        move |_| {
            stop.cancel();
            Ok(())
        },
        Duration::from_secs(20),
        cancel,
    )
    .await
    .err()
    .unwrap();
    assert!(
        matches!(&e, Error::Conflict(m) if m == "Sign-in cancelled."),
        "{e}"
    );
}

#[tokio::test]
async fn a_callback_with_another_state_is_refused_and_the_real_one_still_lands() {
    let auth = fake_auth(3600).await;
    let base = auth.lock().unwrap().base.clone();
    let url = format!("{base}/mcp");
    let signed = oauth::sign_in(
        &url,
        |authorize| {
            let authorize = authorize.to_string();
            tokio::spawn(async move {
                let q = query(
                    reqwest::Url::parse(&authorize)
                        .unwrap()
                        .query()
                        .map(String::from),
                );
                let forged = format!("{}?code=evil&state=wrong", q["redirect_uri"]);
                let answer = reqwest::get(&forged).await.unwrap();
                assert_eq!(answer.status(), 400);
                let _ = reqwest::get(&authorize).await;
            });
            Ok(())
        },
        Duration::from_secs(20),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(*signed.access_token, auth.lock().unwrap().access);
    assert!(signed.grant.refresh_token.is_some());
}

#[tokio::test]
async fn an_auth_server_without_registration_is_refused() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::path(
        "/.well-known/oauth-authorization-server",
    ))
    .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
        "issuer": server.uri(),
        "authorization_endpoint": format!("{}/authorize", server.uri()),
        "token_endpoint": format!("{}/token", server.uri()),
        "code_challenge_methods_supported": ["S256"],
    })))
    .mount(&server)
    .await;
    let e = oauth::auth_server(&oauth::client(), &server.uri())
        .await
        .unwrap_err();
    assert!(e.to_string().contains("register"), "{e}");
}

/// Opens the run tools of `dot` as a run does.
async fn open_for(t: &Rt, dot: &Dot) -> (dots_core::run_tools::RunTools, Vec<String>) {
    let dot = t.rt.store().get_dot(&dot.id).await.unwrap();
    let bus = dots_core::events::new_bus();
    let cancel = CancellationToken::new();
    dots_core::run_tools::RunTools::open(dots_core::run_tools::OpenRequest {
        store: t.rt.store(),
        bus: &bus,
        run_id: "01J0RUN",
        uses: &dot.spec.uses,
        workspace: t.dir.path(),
        plugins_dir: None,
        cancel: &cancel,
        vault: t.rt.vault(),
        scope: &DotScope::Personal,
        workspace_id: "default",
        grants: &dot.spec.variables,
        dot_name: &dot.spec.name,
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn a_run_refreshes_a_refused_token_and_leaves_out_a_sign_in_that_is_over() {
    let auth = fake_auth(3600).await;
    let base = auth.lock().unwrap().base.clone();
    let t = start().await;
    let alpha = t.dot("alpha").await;
    let connector = oauth_connector(&base, "runapp");
    t.rt.sign_in_with(
        &connector,
        std::slice::from_ref(&alpha.id),
        browser,
        Duration::from_secs(20),
    )
    .await
    .unwrap();

    // The server forgets its token: the run refreshes it and opens the server once more.
    auth.lock().unwrap().access = "revoked".into();
    let (tools, notices) = open_for(&t, &alpha).await;
    assert!(notices.is_empty(), "{notices:?}");
    assert_eq!(tools.servers().len(), 1);
    assert_eq!(auth.lock().unwrap().refreshes, 1);

    // The refresh is refused too: the server is left out with what to do.
    {
        let mut a = auth.lock().unwrap();
        a.access = "revoked".into();
        a.refuse_refresh = true;
    }
    let (tools, notices) = open_for(&t, &alpha).await;
    assert!(tools.servers().is_empty());
    assert!(
        notices
            .iter()
            .any(|n| n == "runapp: sign in again on the Connectors screen"),
        "{notices:?}"
    );
}
