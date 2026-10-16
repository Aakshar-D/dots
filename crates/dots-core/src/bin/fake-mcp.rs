//! Test MCP server for dots-core tests: `fake-mcp <script.json> [--http] [args...]`.
//!
//! Over stdio (the default) it reads one JSON-RPC message per line on stdin. With `--http` it
//! listens on 127.0.0.1 (Streamable HTTP at `/mcp`) and prints its URL as the first line of
//! stdout. Other arguments are only recorded.
//!
//! The script is a JSON object:
//! - `tools`: the tools it lists. Besides the tool's own fields, a tool can have `result` (the
//!   `tools/call` result; default: one text part `<name>: <arguments>`), `error` (`{code,
//!   message}`: a JSON-RPC error instead), `delay_ms`, `exit` (the process exits without an
//!   answer), `switch` (the list becomes `after_switch`, and `notifications/tools/list_changed`
//!   is sent before the answer and on GET streams) and `ask_client` (a request with this method
//!   and the id `s-<method>` is sent to the client before the answer).
//!   A tool can also have `big_bytes`: its default result text is that many bytes long.
//! - `after_switch`: the tools after a `switch`.
//! - `page_size`: tools per `tools/list` page (default: all in one).
//! - `protocol_version`: the version `initialize` answers (default: the client's).
//! - `init_delay_ms`, `exit_at_initialize`: a slow or dying `initialize`.
//! - `log`: a file that gets one JSON line per record: `{"start": {pid, args, cwd, env}}` (env:
//!   the variables whose names start with `FAKE_MCP_`), `{"msg": <message received>}` (over
//!   http with `"http": "POST"` and the `session`, `protocol` and `auth` headers), and
//!   `{"http": "GET" | "DELETE", "session": ...}`.
//! - `alive`: a file locked for as long as the process runs.
//! - `stderr_bytes`: written to stderr at start, before anything is read.
//! - `ignore_eof`: over stdio, keep running for 60 s after stdin ends instead of exiting.
//! - `sse`: over http, answer every POST with an event stream (default: JSON, unless there are
//!   messages before the answer).
//! - `require_header`: over http, `{name: value}` every POST must carry (401 otherwise).
//! - `get_stream`: over http, a GET opens an event stream that sends `list_changed` at the
//!   next `switch` and then ends (default: 405).
//! - `stall_cancelled_ms`: over http, a `notifications/cancelled` POST is answered only after
//!   this long.
//! - `redirect_to`: over http, every POST is answered 302 to this URL.

use std::collections::HashSet;
use std::fs::File;
use std::io::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Map, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Notify;

/// Tool fields that script a call; they are not listed.
const CONTROL: [&str; 7] = [
    "big_bytes",
    "result",
    "error",
    "delay_ms",
    "exit",
    "switch",
    "ask_client",
];

struct Fake {
    script: Value,
    tools: Mutex<Vec<Value>>,
    log: Option<Mutex<File>>,
    switched: Notify,
    sessions: Mutex<HashSet<String>>,
    next_session: AtomicU64,
}

/// What the server sends for one request: messages first, then the answer.
struct Reply {
    before: Vec<Value>,
    answer: Value,
}

fn result(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn error(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn list_changed() -> Value {
    json!({"jsonrpc": "2.0", "method": "notifications/tools/list_changed"})
}

fn listed(tool: &Value) -> Value {
    let mut tool = tool.as_object().cloned().unwrap_or_default();
    for key in CONTROL {
        tool.remove(key);
    }
    Value::Object(tool)
}

impl Fake {
    fn record(&self, record: Value) {
        if let Some(log) = &self.log {
            let mut file = log.lock().unwrap();
            writeln!(file, "{record}").unwrap();
            file.flush().unwrap();
        }
    }

    fn flag(&self, name: &str) -> bool {
        self.script[name] == true
    }

    async fn sleep(ms: Option<u64>) {
        if let Some(ms) = ms {
            tokio::time::sleep(Duration::from_millis(ms)).await;
        }
    }

    /// The reply to a request; `None` for a notification or an answer from the client.
    async fn handle(&self, msg: &Value) -> Option<Reply> {
        let id = msg.get("id")?;
        let method = msg.get("method").and_then(Value::as_str)?;
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let mut before = Vec::new();
        let answer = match method {
            "initialize" => {
                Self::sleep(self.script["init_delay_ms"].as_u64()).await;
                if self.flag("exit_at_initialize") {
                    std::process::exit(3);
                }
                let version = self
                    .script
                    .get("protocol_version")
                    .cloned()
                    .unwrap_or_else(|| params["protocolVersion"].clone());
                result(
                    id,
                    json!({"protocolVersion": version,
                           "capabilities": {"tools": {"listChanged": true}},
                           "serverInfo": {"name": "fake-mcp", "version": "0"}}),
                )
            }
            "ping" => result(id, json!({})),
            "tools/list" => {
                let tools: Vec<Value> = self.tools.lock().unwrap().iter().map(listed).collect();
                let start: usize = params["cursor"]
                    .as_str()
                    .map(|c| c.parse().unwrap())
                    .unwrap_or(0);
                let size = match self.script["page_size"].as_u64() {
                    Some(n) if n > 0 => n as usize,
                    _ => tools.len().max(1),
                };
                let end = (start + size).min(tools.len());
                let mut page = json!({"tools": tools[start.min(end)..end]});
                if end < tools.len() {
                    page["nextCursor"] = json!(end.to_string());
                }
                result(id, page)
            }
            "tools/call" => {
                let name = params["name"].as_str().unwrap_or_default();
                let tool = self
                    .tools
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|t| t["name"] == name)
                    .cloned();
                let Some(tool) = tool else {
                    return Some(Reply {
                        before,
                        answer: error(id, -32602, &format!("unknown tool: {name}")),
                    });
                };
                Self::sleep(tool["delay_ms"].as_u64()).await;
                if tool["exit"] == true {
                    std::process::exit(3);
                }
                if let Some(asked) = tool["ask_client"].as_str() {
                    before.push(json!({"jsonrpc": "2.0", "id": format!("s-{asked}"),
                                       "method": asked, "params": {}}));
                }
                if tool["switch"] == true {
                    let after = self.script["after_switch"].as_array().cloned();
                    *self.tools.lock().unwrap() = after.unwrap_or_default();
                    before.push(list_changed());
                    self.switched.notify_waiters();
                }
                if let Some(e) = tool.get("error") {
                    error(
                        id,
                        e["code"].as_i64().unwrap_or(-32000),
                        e["message"].as_str().unwrap_or("error"),
                    )
                } else {
                    let text = match tool["big_bytes"].as_u64() {
                        Some(n) => "x".repeat(n as usize),
                        None => format!("{name}: {}", params["arguments"]),
                    };
                    let default = json!({"content": [{"type": "text", "text": text}]});
                    result(id, tool.get("result").cloned().unwrap_or(default))
                }
            }
            other => error(id, -32601, &format!("method not found: {other}")),
        };
        Some(Reply { before, answer })
    }
}

async fn stdio(fake: Arc<Fake>) {
    let out = Arc::new(tokio::sync::Mutex::new(tokio::io::stdout()));
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        fake.record(json!({"msg": msg}));
        let (fake, out) = (fake.clone(), out.clone());
        // Each request on its own: a slow call does not hold up a cancel or another call.
        tokio::spawn(async move {
            if let Some(reply) = fake.handle(&msg).await {
                let mut out = out.lock().await;
                for m in reply.before.iter().chain([&reply.answer]) {
                    out.write_all(format!("{m}\n").as_bytes()).await.unwrap();
                }
                out.flush().await.unwrap();
            }
        });
    }
    if fake.flag("ignore_eof") {
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}

fn header(headers: &HeaderMap, name: &str) -> Value {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map_or(Value::Null, |v| json!(v))
}

fn event_stream(messages: &[&Value]) -> Response {
    let body: String = messages
        .iter()
        .map(|m| format!("event: message\ndata: {m}\n\n"))
        .collect();
    ([(CONTENT_TYPE, "text/event-stream")], body).into_response()
}

/// `Ok(())` when the request names a session this server gave.
fn check_session(fake: &Fake, headers: &HeaderMap) -> Result<(), StatusCode> {
    match header(headers, "mcp-session-id").as_str() {
        None => Err(StatusCode::BAD_REQUEST),
        Some(id) if !fake.sessions.lock().unwrap().contains(id) => Err(StatusCode::NOT_FOUND),
        Some(_) => Ok(()),
    }
}

async fn on_post(State(fake): State<Arc<Fake>>, headers: HeaderMap, body: Bytes) -> Response {
    let Ok(msg) = serde_json::from_slice::<Value>(&body) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    fake.record(json!({"http": "POST", "msg": msg,
        "session": header(&headers, "mcp-session-id"),
        "protocol": header(&headers, "mcp-protocol-version"),
        "auth": header(&headers, "authorization")}));
    if let Some(to) = fake.script["redirect_to"].as_str() {
        return (
            StatusCode::FOUND,
            [(axum::http::header::LOCATION, to.to_string())],
        )
            .into_response();
    }
    if msg["method"] == "notifications/cancelled" {
        Fake::sleep(fake.script["stall_cancelled_ms"].as_u64()).await;
    }
    if let Some(required) = fake.script["require_header"].as_object() {
        for (name, value) in required {
            if header(&headers, name) != *value {
                return StatusCode::UNAUTHORIZED.into_response();
            }
        }
    }
    let initialize = msg["method"] == "initialize";
    if !initialize {
        if let Err(status) = check_session(&fake, &headers) {
            return status.into_response();
        }
    }
    let Some(reply) = fake.handle(&msg).await else {
        return StatusCode::ACCEPTED.into_response();
    };
    let mut response = if fake.flag("sse") || !reply.before.is_empty() {
        let all: Vec<&Value> = reply.before.iter().chain([&reply.answer]).collect();
        event_stream(&all)
    } else {
        Json(reply.answer).into_response()
    };
    if initialize {
        let n = fake.next_session.fetch_add(1, Ordering::Relaxed);
        let id = format!("fake-session-{n}");
        fake.sessions.lock().unwrap().insert(id.clone());
        response
            .headers_mut()
            .insert("mcp-session-id", id.parse().unwrap());
    }
    response
}

async fn on_get(State(fake): State<Arc<Fake>>, headers: HeaderMap) -> Response {
    if !fake.flag("get_stream") {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    if let Err(status) = check_session(&fake, &headers) {
        return status.into_response();
    }
    // Waiting before the record: a switch the test makes once it sees the GET is not missed.
    let switched = fake.switched.notified();
    tokio::pin!(switched);
    switched.as_mut().enable();
    fake.record(json!({"http": "GET", "session": header(&headers, "mcp-session-id")}));
    switched.await;
    event_stream(&[&list_changed()])
}

async fn on_delete(State(fake): State<Arc<Fake>>, headers: HeaderMap) -> StatusCode {
    let session = header(&headers, "mcp-session-id");
    fake.record(json!({"http": "DELETE", "session": session}));
    if let Some(id) = session.as_str() {
        fake.sessions.lock().unwrap().remove(id);
    }
    StatusCode::OK
}

async fn http(fake: Arc<Fake>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    println!("http://{}/mcp", listener.local_addr().unwrap());
    std::io::stdout().flush().unwrap();
    let app = Router::new()
        .route("/mcp", post(on_post).get(on_get).delete(on_delete))
        .with_state(fake);
    axum::serve(listener, app).await.unwrap();
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let script: Value = serde_json::from_str(&std::fs::read_to_string(&args[0]).unwrap()).unwrap();
    let _alive = script["alive"].as_str().map(|path| {
        let file = File::create(path).unwrap();
        file.lock().unwrap();
        file
    });
    let log = script["log"].as_str().map(|path| {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        Mutex::new(file)
    });
    let fake = Arc::new(Fake {
        tools: Mutex::new(script["tools"].as_array().cloned().unwrap_or_default()),
        script,
        log,
        switched: Notify::new(),
        sessions: Mutex::new(HashSet::new()),
        next_session: AtomicU64::new(1),
    });
    let env: Map<String, Value> = std::env::vars()
        .filter(|(k, _)| k.starts_with("FAKE_MCP_"))
        .map(|(k, v)| (k, json!(v)))
        .collect();
    fake.record(json!({"start": {
        "pid": std::process::id(),
        "args": args,
        "cwd": std::env::current_dir().unwrap(),
        "env": env,
    }}));
    if let Some(n) = fake.script["stderr_bytes"].as_u64() {
        let line = format!("{}\n", "x".repeat(99));
        let mut stderr = std::io::stderr();
        for _ in 0..n.div_ceil(100) {
            stderr.write_all(line.as_bytes()).unwrap();
        }
    }
    if args.iter().any(|a| a == "--http") {
        http(fake).await;
    } else {
        stdio(fake).await;
    }
}
