//! A fake Infisical server for the org source tests, on a port of its own: the Universal Auth
//! login and the v4 list of a folder's secrets, with the shapes and errors `research.md` §2
//! gives. It is programmed through `FakeInfisical` and records every request it sees. It stops
//! when dropped.
//!
//! It is more lenient than Infisical in two ways, so a test can check that the adapter leaves
//! them out: it sends a folder's personal overrides and its `imports` whatever the query says.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde_json::{json, Value};

/// What the next reads, or the next logins, answer instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    /// This status, with Infisical's error envelope. A 3xx also gets a `Location`.
    Status(u16),
    /// A 200 whose body is not JSON.
    NotJson,
    /// A 200 whose body is `{}`: no token, or no list of secrets.
    EmptyObject,
    /// No answer for a minute, so the client's timeout fires first.
    Hang,
}

/// One request the fake saw.
#[derive(Debug, Clone)]
pub struct Seen {
    pub method: String,
    /// The path as sent, still percent-encoded.
    pub path: String,
    /// The query's pairs, decoded, in the order sent.
    pub query: Vec<(String, String)>,
    /// The `Authorization` header.
    pub auth: Option<String>,
    /// The body, when it was JSON.
    pub body: Option<Value>,
}

impl Seen {
    /// The query's value of `name`, if it was sent.
    pub fn param(&self, name: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

struct Identity {
    client_secret: String,
    expires_in: u64,
}

#[derive(Default)]
struct Folder {
    secrets: Vec<Value>,
    imports: Vec<Value>,
}

#[derive(Default)]
struct Fake {
    /// Folders by `(project id, environment slug, path)`, the path without a slash at the end.
    folders: HashMap<(String, String, String), Folder>,
    /// Machine identities by client id.
    identities: HashMap<String, Identity>,
    /// Access tokens it accepts.
    tokens: Vec<String>,
    read_failures: VecDeque<Failure>,
    login_failures: VecDeque<Failure>,
    issued: usize,
    seen: Vec<Seen>,
}

/// The fake server.
pub struct FakeInfisical {
    address: String,
    prefix: String,
    state: Arc<Mutex<Fake>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for FakeInfisical {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// A folder path as Infisical keys it: no slash at the end, except the root's.
fn folder_key(path: &str) -> String {
    match path.trim_end_matches('/') {
        "" => "/".to_string(),
        trimmed => trimmed.to_string(),
    }
}

/// The policy's sub-folder of `path`.
pub fn policy_path(path: &str) -> String {
    match folder_key(path).as_str() {
        "/" => "/policy".to_string(),
        folder => format!("{folder}/policy"),
    }
}

/// A secret as the v4 list gives it.
fn secret(environment: &str, path: &str, kind: &str, key: &str, value: Value) -> Value {
    json!({
        "id": format!("id-{key}"),
        "_id": format!("id-{key}"),
        "workspace": "ws",
        "environment": environment,
        "version": 1,
        "type": kind,
        "secretKey": key,
        "secretValue": value,
        "secretComment": "",
        "secretReminderNote": null,
        "secretReminderRepeatDays": null,
        "skipMultilineEncoding": false,
        "createdAt": "2026-10-23T09:00:00.000Z",
        "updatedAt": "2026-10-23T09:00:00.000Z",
        "secretPath": folder_key(path),
        "secretValueHidden": false,
        "secretMetadata": [],
        "tags": [],
    })
}

impl FakeInfisical {
    /// Starts the fake on `127.0.0.1` and a free port. It holds nothing and knows no identity.
    pub async fn start() -> FakeInfisical {
        Self::start_at("").await
    }

    /// Starts the fake under a path prefix, such as `/infisical`, as a self-hosted instance
    /// behind a proxy. A request outside it is a 404.
    pub async fn start_at(prefix: &str) -> FakeInfisical {
        let state = Arc::new(Mutex::new(Fake::default()));
        let prefix = prefix.trim_end_matches('/').to_string();
        let app = Router::new()
            .fallback(handle)
            .with_state((state.clone(), prefix.clone()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}{prefix}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        FakeInfisical {
            address,
            prefix,
            state,
            task,
        }
    }

    /// `http://127.0.0.1:<port>`, with the prefix if any: the join code's `site_url`.
    pub fn address(&self) -> String {
        self.address.clone()
    }

    fn fake(&self) -> std::sync::MutexGuard<'_, Fake> {
        self.state.lock().unwrap()
    }

    /// Runs `change` on a folder, creating it if needed.
    fn with_folder(
        &self,
        project: &str,
        environment: &str,
        path: &str,
        change: impl FnOnce(&mut Folder),
    ) {
        let key = (
            project.to_string(),
            environment.to_string(),
            folder_key(path),
        );
        change(self.fake().folders.entry(key).or_default());
    }

    /// Creates an empty folder: listing it gives no secrets, not a 404.
    pub fn add_folder(&self, project: &str, environment: &str, path: &str) {
        self.with_folder(project, environment, path, |_| {});
    }

    /// Adds a shared secret to a folder, creating the folder if needed. `value` is the JSON of
    /// `secretValue`, normally a string.
    pub fn put_secret(
        &self,
        project: &str,
        environment: &str,
        path: &str,
        key: &str,
        value: Value,
    ) {
        let secret = secret(environment, path, "shared", key, value);
        self.with_folder(project, environment, path, |f| f.secrets.push(secret));
    }

    /// Adds a personal override of `key` to a folder.
    pub fn put_personal(
        &self,
        project: &str,
        environment: &str,
        path: &str,
        key: &str,
        value: &str,
    ) {
        let secret = secret(environment, path, "personal", key, json!(value));
        self.with_folder(project, environment, path, |f| f.secrets.push(secret));
    }

    /// Adds a secret object exactly as given, for shapes `put_secret` does not make.
    pub fn put_raw(&self, project: &str, environment: &str, path: &str, secret: Value) {
        self.with_folder(project, environment, path, |f| f.secrets.push(secret));
    }

    /// Writes the org's signed policy text, as secret `DOTS_POLICY` of `<path>/policy`.
    pub fn put_policy(&self, project: &str, environment: &str, path: &str, signed: &str) {
        let policy = policy_path(path);
        self.put_secret(project, environment, &policy, "DOTS_POLICY", json!(signed));
    }

    /// Adds an import to a folder: `key` as if imported from the folder `from`.
    pub fn put_import(
        &self,
        project: &str,
        environment: &str,
        path: &str,
        from: &str,
        key: &str,
        value: &str,
    ) {
        let import = json!({
            "secretPath": folder_key(from),
            "environment": environment,
            "folderId": "folder-imported",
            "secrets": [secret(environment, from, "shared", key, json!(value))],
        });
        self.with_folder(project, environment, path, |f| f.imports.push(import));
    }

    /// Deletes a folder and its secrets: listing it is a 404.
    pub fn remove_folder(&self, project: &str, environment: &str, path: &str) {
        let key = (
            project.to_string(),
            environment.to_string(),
            folder_key(path),
        );
        self.fake().folders.remove(&key);
    }

    /// Adds a machine identity. A login gives a new token whose `expiresIn` is `expires_in`.
    pub fn add_identity(&self, client_id: &str, client_secret: &str, expires_in: u64) {
        self.fake().identities.insert(
            client_id.to_string(),
            Identity {
                client_secret: client_secret.to_string(),
                expires_in,
            },
        );
    }

    /// Stops accepting every access token given so far: each read with one is a 401.
    pub fn revoke_all_tokens(&self) {
        self.fake().tokens.clear();
    }

    /// The next `count` reads of secrets answer `failure`. Logins are not affected.
    pub fn fail_next(&self, failure: Failure, count: usize) {
        let mut fake = self.fake();
        fake.read_failures
            .extend(std::iter::repeat_n(failure, count));
    }

    /// The next `count` logins answer `failure`.
    pub fn fail_next_login(&self, failure: Failure, count: usize) {
        let mut fake = self.fake();
        fake.login_failures
            .extend(std::iter::repeat_n(failure, count));
    }

    /// Every request so far, oldest first.
    pub fn requests(&self) -> Vec<Seen> {
        self.fake().seen.clone()
    }

    /// Every request so far as `METHOD /path`, the prefix left out, oldest first.
    pub fn calls(&self) -> Vec<String> {
        self.requests()
            .iter()
            .map(|s| {
                let path = s.path.strip_prefix(&self.prefix).unwrap_or(&s.path);
                format!("{} {path}", s.method)
            })
            .collect()
    }

    /// The reads of secrets so far, oldest first.
    pub fn reads(&self) -> Vec<Seen> {
        self.requests()
            .into_iter()
            .filter(|s| s.path.ends_with("/api/v4/secrets"))
            .collect()
    }

    /// How many logins were tried.
    pub fn logins(&self) -> usize {
        self.requests()
            .iter()
            .filter(|s| s.path.ends_with("/api/v1/auth/universal-auth/login"))
            .count()
    }

    /// Forgets the requests seen so far.
    pub fn clear_requests(&self) {
        self.fake().seen.clear();
    }
}

/// Infisical's error envelope.
fn error(status: u16, name: &str, message: &str) -> Response {
    let code = StatusCode::from_u16(status).unwrap();
    let body = json!({
        "reqId": "req-0001",
        "statusCode": status,
        "message": message,
        "error": name,
    });
    (code, Json(body)).into_response()
}

fn unauthorized() -> Response {
    error(401, "UnauthorizedError", "Invalid credentials")
}

fn answer_failure(failure: Failure) -> Option<Response> {
    match failure {
        Failure::Hang => None,
        Failure::NotJson => Some((StatusCode::OK, "<html>a proxy page</html>").into_response()),
        Failure::EmptyObject => Some(Json(json!({})).into_response()),
        Failure::Status(code) if (300..400).contains(&code) => {
            let mut response = error(code, "Redirect", "moved");
            response
                .headers_mut()
                .insert("location", "/api/elsewhere".parse().unwrap());
            Some(response)
        }
        Failure::Status(401) => Some(unauthorized()),
        Failure::Status(403) => Some(error(
            403,
            "PermissionDenied",
            "You are not allowed to readValue on secrets",
        )),
        Failure::Status(429) => Some(error(
            429,
            "Too Many Requests",
            "Rate limit exceeded. Please try again in 60 seconds",
        )),
        Failure::Status(code) => Some(error(code, "InternalServerError", "Something went wrong")),
    }
}

fn query_pairs(uri: &Uri) -> Vec<(String, String)> {
    let Some(query) = uri.query() else {
        return Vec::new();
    };
    let url = reqwest::Url::parse(&format!("http://fake/?{query}")).unwrap();
    url.query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

async fn handle(
    State((state, prefix)): State<(Arc<Mutex<Fake>>, String)>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let seen = Seen {
        method: method.to_string(),
        path: uri.path().to_string(),
        query: query_pairs(&uri),
        auth: headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string),
        body: serde_json::from_slice(&body).ok(),
    };
    let Some(route) = uri.path().strip_prefix(prefix.as_str()) else {
        return error(404, "Not Found", "Route not found");
    };
    let route = match (method.clone(), route) {
        (Method::POST, "/api/v1/auth/universal-auth/login") => Route::Login,
        (Method::GET, "/api/v4/secrets") => Route::List,
        _ => Route::Other,
    };
    let failure = {
        let mut fake = state.lock().unwrap();
        fake.seen.push(seen.clone());
        match route {
            Route::Login => fake.login_failures.pop_front(),
            Route::List => fake.read_failures.pop_front(),
            Route::Other => None,
        }
    };
    if let Some(failure) = failure {
        match answer_failure(failure) {
            Some(response) => return response,
            None => {
                tokio::time::sleep(Duration::from_secs(60)).await;
                return error(503, "ServiceUnavailable", "Service unavailable");
            }
        }
    }

    let mut fake = state.lock().unwrap();
    match route {
        Route::Login => login(&mut fake, &seen),
        Route::List => list(&fake, &seen),
        Route::Other => error(
            404,
            "Not Found",
            &format!("Route {}:{} not found", seen.method, seen.path),
        ),
    }
}

enum Route {
    Login,
    List,
    Other,
}

fn login(fake: &mut Fake, seen: &Seen) -> Response {
    let body = seen.body.clone().unwrap_or(Value::Null);
    let (Some(id), Some(secret)) = (body["clientId"].as_str(), body["clientSecret"].as_str())
    else {
        return error(
            422,
            "ValidationFailure",
            "clientId and clientSecret are required",
        );
    };
    let expires_in = match fake.identities.get(id) {
        Some(identity) if identity.client_secret == secret => identity.expires_in,
        _ => return unauthorized(),
    };
    fake.issued += 1;
    let token = format!("inf.issued-{}", fake.issued);
    fake.tokens.push(token.clone());
    Json(json!({
        "accessToken": token,
        "expiresIn": expires_in,
        "accessTokenMaxTTL": expires_in,
        "tokenType": "Bearer",
    }))
    .into_response()
}

fn list(fake: &Fake, seen: &Seen) -> Response {
    let token = seen.auth.as_deref().and_then(|a| a.strip_prefix("Bearer "));
    if !token.is_some_and(|t| fake.tokens.iter().any(|known| known == t)) {
        return unauthorized();
    }
    let (Some(project), Some(environment)) = (seen.param("projectId"), seen.param("environment"))
    else {
        return error(400, "BadRequest", "Missing project id or environment");
    };
    let path = folder_key(seen.param("secretPath").unwrap_or("/"));
    let key = (project.to_string(), environment.to_string(), path.clone());
    let Some(folder) = fake.folders.get(&key) else {
        return error(
            404,
            "NotFoundError",
            &format!(
                "Folder with path '{path}' in environment with slug '{environment}' not found"
            ),
        );
    };
    Json(json!({ "secrets": folder.secrets, "imports": folder.imports })).into_response()
}
