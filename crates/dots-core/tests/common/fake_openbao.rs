//! A fake OpenBao (or Vault) server for the org source tests, on a port of its own: KV v2 reads,
//! `userpass` and `ldap` login, and a token's `lookup-self` and `renew-self`, with the shapes and
//! errors `research.md` §1 gives. It is programmed through `FakeOpenBao` and records every
//! request it sees. It stops when dropped.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde_json::{json, Value};

/// What the next KV reads answer instead of the data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    /// The read answers as usual: queued before a failure, it lets that failure land on a later
    /// read of the same fetch.
    Pass,
    /// This status, with `{"errors":[…]}`. A 3xx also gets a `Location` to `/v1/elsewhere`.
    Status(u16),
    /// A 200 whose body is not JSON.
    NotJson,
    /// No answer for a minute, so the client's timeout fires first.
    Hang,
    /// A 200 with more than 1 MiB of JSON text and a `Content-Length`.
    TooLarge,
    /// The same, chunked: no `Content-Length`.
    TooLargeChunked,
}

/// One request the fake saw.
#[derive(Debug, Clone)]
pub struct Seen {
    pub method: String,
    /// The path as sent, still percent-encoded.
    pub path: String,
    /// `X-Vault-Token`.
    pub token: Option<String>,
    /// `X-Vault-Namespace`.
    pub namespace: Option<String>,
    /// The body, when it was JSON.
    pub body: Option<Value>,
}

struct User {
    password: String,
    lease_seconds: u64,
}

#[derive(Default)]
struct Fake {
    /// KV v2 secrets by `<mount>/data/<path>`, decoded: the object of `data.data`.
    kv: HashMap<String, Value>,
    /// Tokens it accepts, and whether each is renewable.
    tokens: HashMap<String, bool>,
    /// Users by `(auth mount, username)`.
    users: HashMap<(String, String), User>,
    failures: VecDeque<Failure>,
    /// KV paths, `<mount>/data/<path>`, every read of which is refused (403), as an ACL that
    /// does not grant them answers.
    denied: HashSet<String>,
    /// KV paths every write to which is refused (403), as an ACL that grants only read does.
    denied_writes: HashSet<String>,
    refuse_renewals: bool,
    /// What every login answers instead, as a status and one `errors` text.
    login_error: Option<(u16, String)>,
    issued: usize,
    seen: Vec<Seen>,
}

/// The fake server.
pub struct FakeOpenBao {
    address: String,
    state: Arc<Mutex<Fake>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for FakeOpenBao {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl FakeOpenBao {
    /// Starts the fake on `127.0.0.1` and a free port. It holds nothing and accepts no token.
    pub async fn start() -> FakeOpenBao {
        let state = Arc::new(Mutex::new(Fake::default()));
        let app = Router::new().fallback(handle).with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        FakeOpenBao {
            address,
            state,
            task,
        }
    }

    /// `http://127.0.0.1:<port>`, the join code's `address`.
    pub fn address(&self) -> String {
        self.address.clone()
    }

    fn fake(&self) -> std::sync::MutexGuard<'_, Fake> {
        self.state.lock().unwrap()
    }

    /// Writes the KV v2 secret `<mount>/<path>`: `data` is the object `data.data` reads.
    pub fn put(&self, mount: &str, path: &str, data: Value) {
        self.fake().kv.insert(format!("{mount}/data/{path}"), data);
    }

    /// Writes the org's entries, at `<mount>/<path>/env`.
    pub fn put_env(&self, mount: &str, path: &str, entries: Value) {
        self.put(mount, &format!("{path}/env"), entries);
    }

    /// Writes the org's signed policy text, as key `DOTS_POLICY` of `<mount>/<path>/policy`.
    pub fn put_policy(&self, mount: &str, path: &str, signed: &str) {
        self.put(
            mount,
            &format!("{path}/policy"),
            json!({ "DOTS_POLICY": signed }),
        );
    }

    /// Writes the org's signed roster text, as key `DOTS_ROSTER` of `<mount>/<path>/roster`.
    pub fn put_roster(&self, mount: &str, path: &str, signed: &str) {
        self.put(
            mount,
            &format!("{path}/roster"),
            json!({ "DOTS_ROSTER": signed }),
        );
    }

    /// Deletes the KV v2 secret `<mount>/<path>`: reading it is a 404.
    pub fn remove(&self, mount: &str, path: &str) {
        self.fake().kv.remove(&format!("{mount}/data/{path}"));
    }

    /// Refuses every read of `<mount>/data/<path>` with a 403, as a token whose ACL does not
    /// grant the path is refused.
    pub fn deny(&self, mount: &str, path: &str) {
        self.fake().denied.insert(format!("{mount}/data/{path}"));
    }

    /// Refuses every write to `<mount>/data/<path>` with a 403, as a token whose ACL grants
    /// only reading it is refused. Reads are not affected.
    pub fn deny_writes(&self, mount: &str, path: &str) {
        self.fake()
            .denied_writes
            .insert(format!("{mount}/data/{path}"));
    }

    /// Accepts `token` from now on. `lookup-self` says whether it is `renewable`.
    pub fn allow_token(&self, token: &str, renewable: bool) {
        self.fake().tokens.insert(token.to_string(), renewable);
    }

    /// Stops accepting `token`, as a revoked or expired one: every call with it is a 403.
    pub fn revoke_token(&self, token: &str) {
        self.fake().tokens.remove(token);
    }

    /// Stops accepting every token, those logins gave included.
    pub fn revoke_all_tokens(&self) {
        self.fake().tokens.clear();
    }

    /// Adds a user to the `userpass` or `ldap` mount `auth_mount`. A login gives a new token
    /// with a lease of `lease_seconds` (0: it does not expire), renewable if the lease is not 0.
    pub fn add_user(&self, auth_mount: &str, username: &str, password: &str, lease_seconds: u64) {
        self.fake().users.insert(
            (auth_mount.to_string(), username.to_string()),
            User {
                password: password.to_string(),
                lease_seconds,
            },
        );
    }

    /// The next `count` KV reads or writes answer `failure`. Logins and token calls are not affected.
    pub fn fail_next(&self, failure: Failure, count: usize) {
        let mut fake = self.fake();
        for _ in 0..count {
            fake.failures.push_back(failure);
        }
    }

    /// Every login answers `status` with `{"errors":[message]}` from now on.
    pub fn fail_logins(&self, status: u16, message: &str) {
        self.fake().login_error = Some((status, message.to_string()));
    }

    /// Makes `renew-self` fail with a 500 even for a renewable token.
    pub fn refuse_renewals(&self) {
        self.fake().refuse_renewals = true;
    }

    /// Every request so far, oldest first.
    pub fn requests(&self) -> Vec<Seen> {
        self.fake().seen.clone()
    }

    /// Every request so far as `METHOD /path`, oldest first.
    pub fn calls(&self) -> Vec<String> {
        self.requests()
            .iter()
            .map(|s| format!("{} {}", s.method, s.path))
            .collect()
    }

    /// How many requests went to `path` (as sent), with any method.
    pub fn count(&self, path: &str) -> usize {
        self.requests().iter().filter(|s| s.path == path).count()
    }

    /// How many logins were tried, on any mount.
    pub fn logins(&self) -> usize {
        self.requests()
            .iter()
            .filter(|s| s.path.starts_with("/v1/auth/") && s.path.contains("/login/"))
            .count()
    }

    /// How many `renew-self` calls were made.
    pub fn renewals(&self) -> usize {
        self.count("/v1/auth/token/renew-self")
    }

    /// Forgets the requests seen so far.
    pub fn clear_requests(&self) {
        self.fake().seen.clear();
    }
}

fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

/// `%XX` decoded, as a server reads a path.
fn decode(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(b)) => {
                out.push(b);
                i += 3;
            }
            (b, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn errors(status: u16, messages: &[&str]) -> Response {
    let status = StatusCode::from_u16(status).unwrap();
    (status, Json(json!({ "errors": messages }))).into_response()
}

fn denied() -> Response {
    errors(403, &["permission denied"])
}

/// The envelope every answer has, around `data` and `auth`.
fn envelope(data: Value, auth: Value) -> Response {
    Json(json!({
        "request_id": "8d3c2a6e-0000-4000-8000-000000000001",
        "lease_id": "",
        "renewable": false,
        "lease_duration": 0,
        "data": data,
        "wrap_info": null,
        "warnings": null,
        "auth": auth,
    }))
    .into_response()
}

fn auth_answer(token: &str, username: &str, lease_seconds: u64) -> Value {
    json!({
        "client_token": token,
        "accessor": "acc-0001",
        "policies": ["default", "dots-read"],
        "token_policies": ["default", "dots-read"],
        "metadata": { "username": username },
        "lease_duration": lease_seconds,
        "renewable": lease_seconds > 0,
        "entity_id": "ent-0001",
        "token_type": "service",
        "orphan": true,
    })
}

async fn handle(
    State(state): State<Arc<Mutex<Fake>>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let token = header(&headers, "x-vault-token");
    let path = decode(uri.path());
    let failure = {
        let mut fake = state.lock().unwrap();
        fake.seen.push(Seen {
            method: method.to_string(),
            path: uri.path().to_string(),
            token: token.clone(),
            namespace: header(&headers, "x-vault-namespace"),
            body: serde_json::from_slice(&body).ok(),
        });
        let kv_read =
            (method == Method::GET || method == Method::POST) && !path.starts_with("/v1/auth/");
        if kv_read {
            fake.failures.pop_front()
        } else {
            None
        }
    };
    match failure {
        Some(Failure::Hang) => {
            tokio::time::sleep(Duration::from_secs(60)).await;
            return errors(503, &["Vault is sealed"]);
        }
        Some(Failure::TooLarge) => {
            let filler = "x".repeat(1024 * 1024 + 16);
            return Json(json!({ "data": { "data": { "FILLER": filler } } })).into_response();
        }
        Some(Failure::TooLargeChunked) => {
            let chunks = (0..70).map(|i| {
                let head = if i == 0 {
                    "{\"data\":{\"data\":{\"FILLER\":\""
                } else {
                    ""
                };
                Ok::<_, std::convert::Infallible>(Bytes::from(format!(
                    "{head}{}",
                    "x".repeat(16 * 1024)
                )))
            });
            return Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "application/json")
                .body(axum::body::Body::from_stream(futures_util::stream::iter(
                    chunks,
                )))
                .unwrap();
        }
        Some(Failure::NotJson) => {
            return (StatusCode::OK, "<html>a proxy page</html>").into_response()
        }
        Some(Failure::Status(code)) if (300..400).contains(&code) => {
            let mut response = errors(code, &[]);
            response
                .headers_mut()
                .insert("location", "/v1/elsewhere".parse().unwrap());
            return response;
        }
        Some(Failure::Status(403)) => return denied(),
        Some(Failure::Status(503)) => return errors(503, &["Vault is sealed"]),
        Some(Failure::Status(code)) => return errors(code, &["internal error"]),
        Some(Failure::Pass) | None => {}
    }

    let mut fake = state.lock().unwrap();
    let Some(rest) = path.strip_prefix("/v1/") else {
        return errors(404, &[]);
    };
    if let Some(login) = rest
        .strip_prefix("auth/")
        .and_then(|r| r.rsplit_once("/login/"))
    {
        if method != Method::POST {
            return errors(405, &[]);
        }
        if let Some((status, message)) = fake.login_error.clone() {
            return errors(status, &[message.as_str()]);
        }
        let (mount, username) = (login.0.to_string(), login.1.to_string());
        let password = serde_json::from_slice::<Value>(&body)
            .ok()
            .and_then(|b| b["password"].as_str().map(str::to_string));
        let lease = match fake.users.get(&(mount, username.clone())) {
            Some(user) if Some(&user.password) == password.as_ref() => user.lease_seconds,
            _ => return errors(400, &["invalid username or password"]),
        };
        fake.issued += 1;
        let issued = format!("hvs.issued-{}", fake.issued);
        fake.tokens.insert(issued.clone(), lease > 0);
        return envelope(Value::Null, auth_answer(&issued, &username, lease));
    }
    let renewable = match token.as_ref().and_then(|t| fake.tokens.get(t)) {
        Some(renewable) => *renewable,
        None => return denied(),
    };
    match (method.clone(), rest) {
        (Method::GET, "auth/token/lookup-self") => envelope(
            json!({ "renewable": renewable, "ttl": 3600, "policies": ["default"] }),
            Value::Null,
        ),
        (Method::POST, "auth/token/renew-self") if fake.refuse_renewals => {
            errors(500, &["internal error"])
        }
        (Method::POST, "auth/token/renew-self") if !renewable => {
            errors(400, &["lease is not renewable"])
        }
        (Method::POST, "auth/token/renew-self") => envelope(
            Value::Null,
            auth_answer(token.as_deref().unwrap(), "", 3600),
        ),
        (Method::POST, kv) if fake.denied_writes.contains(kv) => denied(),
        (Method::POST, kv) if kv.contains("/data/") => {
            // A KV v2 write: `{"data": {…}}` replaces the secret, and the answer is its metadata.
            match serde_json::from_slice::<Value>(&body) {
                Ok(Value::Object(mut write)) if write.get("data").is_some_and(Value::is_object) => {
                    let data = write.remove("data").unwrap();
                    fake.kv.insert(kv.to_string(), data);
                    envelope(
                        json!({
                            "created_time": "2026-10-23T09:00:00.000000000Z",
                            "custom_metadata": null,
                            "deletion_time": "",
                            "destroyed": false,
                            "version": 2,
                        }),
                        Value::Null,
                    )
                }
                _ => errors(400, &["no data provided"]),
            }
        }
        (Method::GET, kv) if fake.denied.contains(kv) => denied(),
        (Method::GET, kv) => match fake.kv.get(kv) {
            Some(data) => envelope(
                json!({
                    "data": data,
                    "metadata": {
                        "created_time": "2026-10-23T09:00:00.000000000Z",
                        "custom_metadata": null,
                        "deletion_time": "",
                        "destroyed": false,
                        "version": 1,
                    },
                }),
                Value::Null,
            ),
            None => errors(404, &[]),
        },
        _ => errors(405, &[]),
    }
}
