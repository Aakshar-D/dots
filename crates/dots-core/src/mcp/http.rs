//! The Streamable HTTP transport: each message is a POST, answered with JSON or with an event
//! stream (SSE); the `Mcp-Session-Id` the server gives at `initialize` goes with every later
//! request, and so does `MCP-Protocol-Version`. A GET stream, when the server offers one,
//! brings what the server says on its own, such as `notifications/tools/list_changed`.
//!
//! Errors never show the URL or the headers: they can hold expanded `${VAR}` values.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reqwest::header::{ACCEPT, CONTENT_TYPE};
use reqwest::{RequestBuilder, Response, StatusCode};
use serde_json::Value;
use tokio_util::sync::{CancellationToken, DropGuard};

use super::rpc::{self, Peer};
use super::{McpError, NOTE_TIMEOUT};

const SESSION_HEADER: &str = "Mcp-Session-Id";
const VERSION_HEADER: &str = "MCP-Protocol-Version";
/// The wait before the GET stream is opened again after the server ended it.
const RELISTEN: Duration = Duration::from_millis(500);

pub(super) struct Http {
    inner: Arc<Inner>,
    /// Fired by `close`, or by dropping this: the GET stream ends.
    listening: CancellationToken,
    _on_drop: DropGuard,
}

struct Inner {
    peer: Arc<Peer>,
    client: reqwest::Client,
    url: String,
    headers: BTreeMap<String, String>,
    session_id: Mutex<Option<String>>,
    /// Set once `initialize` has been answered.
    version: Mutex<Option<String>>,
}

impl Http {
    pub(super) fn new(peer: Arc<Peer>, url: String, headers: BTreeMap<String, String>) -> Self {
        let listening = CancellationToken::new();
        Self {
            inner: Arc::new(Inner {
                peer,
                client: reqwest::Client::new(),
                url,
                headers,
                session_id: Mutex::new(None),
                version: Mutex::new(None),
            }),
            _on_drop: listening.clone().drop_guard(),
            listening,
        }
    }

    pub(super) async fn send(
        &self,
        message: &Value,
        awaiting: Option<u64>,
    ) -> Result<(), McpError> {
        self.inner.send(message, awaiting).await
    }

    /// `initialize` was answered with `version`: it is sent with every later request, a
    /// failure to connect from now on means that the server stopped, and the GET stream opens.
    pub(super) fn ready(&self, version: &str) {
        *self.inner.version.lock().unwrap() = Some(version.to_string());
        tokio::spawn(listen(self.inner.clone(), self.listening.clone()));
    }

    /// Ends the GET stream and deletes the session at the server, if it has one.
    pub(super) async fn close(&self) {
        self.listening.cancel();
        if self.inner.peer.is_stopped() || self.inner.session_id().is_none() {
            return;
        }
        let delete = self
            .inner
            .with_headers(self.inner.client.delete(&self.inner.url));
        let _ = tokio::time::timeout(NOTE_TIMEOUT, delete.send()).await;
    }
}

impl Inner {
    fn session_id(&self) -> Option<String> {
        self.session_id.lock().unwrap().clone()
    }

    fn is_ready(&self) -> bool {
        self.version.lock().unwrap().is_some()
    }

    fn with_headers(&self, mut request: RequestBuilder) -> RequestBuilder {
        for (name, value) in &self.headers {
            request = request.header(name, value);
        }
        if let Some(id) = self.session_id() {
            request = request.header(SESSION_HEADER, id);
        }
        if let Some(version) = self.version.lock().unwrap().clone() {
            request = request.header(VERSION_HEADER, version);
        }
        request
    }

    fn post(&self, message: &Value) -> RequestBuilder {
        let request = self
            .client
            .post(&self.url)
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "application/json, text/event-stream")
            .body(message.to_string());
        self.with_headers(request)
    }

    /// POSTs `message` and gives the peer every message of the response, up to the answer to
    /// `awaiting`.
    async fn send(&self, message: &Value, awaiting: Option<u64>) -> Result<(), McpError> {
        let response = self
            .post(message)
            .send()
            .await
            .map_err(|e| self.failed(e))?;
        if let Some(id) = response
            .headers()
            .get(SESSION_HEADER)
            .and_then(|v| v.to_str().ok())
        {
            self.session_id
                .lock()
                .unwrap()
                .get_or_insert_with(|| id.to_string());
        }
        let status = response.status();
        // A session the server no longer knows (MCP: 404) is over.
        if status == StatusCode::NOT_FOUND && self.is_ready() && self.session_id().is_some() {
            self.peer.stop();
            return Err(McpError::Stopped);
        }
        if !status.is_success() {
            return Err(McpError::Protocol(format!(
                "the server answered HTTP {status}"
            )));
        }
        if matches!(status, StatusCode::ACCEPTED | StatusCode::NO_CONTENT) {
            return Ok(());
        }
        let kind = content_type(&response);
        if kind.starts_with("text/event-stream") {
            self.read_stream(response, awaiting).await
        } else if kind.starts_with("application/json") {
            let body = response.bytes().await.map_err(|e| self.failed(e))?;
            if body.trim_ascii().is_empty() {
                return Ok(());
            }
            let messages = match serde_json::from_slice(&body) {
                Ok(Value::Array(messages)) => messages,
                Ok(message) => vec![message],
                Err(e) => {
                    return Err(McpError::Protocol(format!(
                        "the server's answer is not JSON: {e}"
                    )))
                }
            };
            for message in messages {
                self.receive(message).await;
            }
            Ok(())
        } else if awaiting.is_none() {
            Ok(())
        } else {
            Err(McpError::Protocol(format!(
                "the server answered with {kind:?}, not JSON or an event stream"
            )))
        }
    }

    /// Reads an event stream until it ends or brings the answer to `awaiting`.
    async fn read_stream(
        &self,
        mut response: Response,
        awaiting: Option<u64>,
    ) -> Result<(), McpError> {
        let mut events = SseParser::default();
        loop {
            let Some(chunk) = response.chunk().await.map_err(|e| self.failed(e))? else {
                return Ok(());
            };
            for data in events.push(&chunk) {
                let Ok(message) = serde_json::from_str::<Value>(&data) else {
                    tracing::debug!(server = %self.peer.name(), "an event that is not JSON");
                    continue;
                };
                let done = awaiting.is_some_and(|id| rpc::is_answer_to(&message, id));
                self.receive(message).await;
                if done {
                    return Ok(());
                }
            }
        }
    }

    /// Gives `message` to the peer and POSTs its reply, if any.
    async fn receive(&self, message: Value) {
        let Some(reply) = self.peer.receive(message) else {
            return;
        };
        if let Err(e) = self.post(&reply).send().await {
            tracing::debug!(server = %self.peer.name(), "replying to the server failed: {}", describe(e));
        }
    }

    /// What a failed request means: before `initialize` was answered the server cannot be
    /// reached; after, a refused connection means that it stopped; anything else broke this
    /// request only.
    fn failed(&self, e: reqwest::Error) -> McpError {
        let connect = e.is_connect();
        let text = describe(e);
        if !self.is_ready() {
            McpError::Start(format!("could not reach the server: {text}"))
        } else if connect {
            self.peer.stop();
            McpError::Stopped
        } else {
            McpError::Protocol(format!("the connection failed: {text}"))
        }
    }
}

/// Listens on the GET stream until `stop` fires. A server without one (405, or anything that
/// is not an event stream) is left alone; a stream the server ends is opened again.
async fn listen(inner: Arc<Inner>, stop: CancellationToken) {
    loop {
        let request = inner.with_headers(
            inner
                .client
                .get(&inner.url)
                .header(ACCEPT, "text/event-stream"),
        );
        let response = tokio::select! {
            _ = stop.cancelled() => return,
            response = request.send() => response,
        };
        let response = match response {
            Ok(r)
                if r.status().is_success() && content_type(&r).starts_with("text/event-stream") =>
            {
                r
            }
            Ok(r) => {
                tracing::debug!(server = %inner.peer.name(), "no GET stream: HTTP {}", r.status());
                return;
            }
            Err(e) => {
                tracing::debug!(server = %inner.peer.name(), "no GET stream: {}", describe(e));
                return;
            }
        };
        let ended = tokio::select! {
            _ = stop.cancelled() => return,
            ended = inner.read_stream(response, None) => ended,
        };
        if ended.is_err() || inner.peer.is_stopped() {
            return;
        }
        tokio::select! {
            _ = stop.cancelled() => return,
            _ = tokio::time::sleep(RELISTEN) => {}
        }
    }
}

fn content_type(response: &Response) -> String {
    response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// The error and its causes, without the URL.
fn describe(e: reqwest::Error) -> String {
    let e = e.without_url();
    let mut text = e.to_string();
    let mut source = std::error::Error::source(&e);
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

/// Cuts an event stream into the `data` of its events. `event`, `id`, `retry` and comments
/// are not needed.
#[derive(Default)]
struct SseParser {
    buf: Vec<u8>,
    data: Option<String>,
}

impl SseParser {
    fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        self.buf.extend_from_slice(bytes);
        let mut events = Vec::new();
        while let Some(end) = self.buf.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line);
            let line = line.trim_end_matches(['\n', '\r']);
            if line.is_empty() {
                events.extend(self.data.take());
            } else if let Some(rest) = line.strip_prefix("data:") {
                let rest = rest.strip_prefix(' ').unwrap_or(rest);
                match &mut self.data {
                    Some(data) => {
                        data.push('\n');
                        data.push_str(rest);
                    }
                    None => self.data = Some(rest.to_string()),
                }
            }
        }
        events
    }
}
