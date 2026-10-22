//! The MCP client (MCP/skills spec section 5): a session with one Library MCP server, over stdio
//! or Streamable HTTP. JSON-RPC 2.0 written by hand with `serde_json`, as the server side
//! (`server/mcp.rs`) is.
//!
//! A run opens one `McpSession` per server it uses (`open`: start or connect, `initialize`,
//! `notifications/initialized`), lists the tools (`list_tools`, every page), calls them
//! (`call_tool`, with a time limit and the run's cancel token), watches `list_changed` for the
//! server's `notifications/tools/list_changed`, and closes the session when it ends (`close`,
//! which stops a stdio server). A session is not restarted: once its server has stopped, every
//! request fails at once with `McpError::Stopped`.

mod http;
mod rpc;
mod stdio;
mod tools;
pub(crate) mod vars;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::library::McpServerConfig;
use crate::Error;

pub use stdio::resolve_program;
pub use tools::{fingerprint, McpTool};

/// The protocol version Dots asks for.
pub const PROTOCOL_VERSION: &str = "2025-06-18";
/// The versions Dots accepts when a server answers with another one.
pub const KNOWN_PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];
/// The limit for `initialize` and for `tools/list` with all its pages.
pub const ANSWER_TIMEOUT: Duration = Duration::from_secs(30);
/// The limit for a `tools/call` of the local engine.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(300);
/// How long a stdio server has to exit by itself once its input is closed, before it is killed.
pub const CLOSE_GRACE: Duration = Duration::from_secs(2);
/// The limit for a notification and for the closing `DELETE` of an http session.
const NOTE_TIMEOUT: Duration = Duration::from_secs(5);
/// The most a server may send as one message (a stdio line, an http body, an event of a
/// stream). More is not an MCP server worth trusting with Dots' memory: stdio is stopped, an
/// http request fails with `McpError::Protocol`.
const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;
/// `tools/list` pages read at most: a server that pages forever is not an MCP server.
const MAX_PAGES: usize = 100;

/// How a session starts and ends.
#[derive(Debug, Clone)]
pub struct SessionOptions {
    /// The folder a stdio server starts in when its config names none. `None`: Dots' own.
    pub cwd: Option<PathBuf>,
    /// The limit for `initialize` and `tools/list`.
    pub answer_timeout: Duration,
    /// How long `close` waits for a stdio server to exit by itself.
    pub close_grace: Duration,
}

impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            cwd: None,
            answer_timeout: ANSWER_TIMEOUT,
            close_grace: CLOSE_GRACE,
        }
    }
}

/// Why a request to an MCP server failed. The text never names the server (the caller adds
/// it: "salesforce: could not start: SF_TOKEN is not set") and never holds an expanded
/// `${VAR}` value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum McpError {
    /// The server could not be started or reached: a variable not set, a command not found, a
    /// process that exited before it answered, a refused connection.
    #[error("could not start: {0}")]
    Start(String),
    /// No answer within the time limit.
    #[error("did not answer")]
    Timeout,
    /// The server is gone: its process exited, or its http endpoint stopped taking requests.
    #[error("the server stopped")]
    Stopped,
    /// The caller's cancel token fired first.
    #[error("cancelled")]
    Cancelled,
    /// The server answered with a JSON-RPC error.
    #[error("{message} (error {code})")]
    Server { code: i64, message: String },
    /// The answer is not what MCP says: an HTTP error status, no JSON, an unknown protocol
    /// version, a broken connection.
    #[error("{0}")]
    Protocol(String),
}

impl From<McpError> for Error {
    fn from(e: McpError) -> Self {
        Error::Other(e.to_string())
    }
}

enum Transport {
    Stdio(stdio::Stdio),
    Http(http::Http),
}

impl Transport {
    /// Sends one message. Over http this is the whole exchange: what the server sends back
    /// (an answer to `awaiting`, notifications, requests) goes to the peer before it returns.
    async fn send(&self, message: &Value, awaiting: Option<u64>) -> Result<(), McpError> {
        match self {
            Self::Stdio(t) => t.send(message),
            Self::Http(t) => t.send(message, awaiting).await,
        }
    }

    /// Sends a notice whose outcome nobody waits for. Over http it goes in the background, so
    /// that a stuck server cannot delay the caller (a cancel is answered at once).
    fn notify_detached(&self, message: Value) {
        match self {
            Self::Stdio(t) => {
                let _ = t.send(&message);
            }
            Self::Http(t) => t.send_detached(message),
        }
    }

    async fn close(&self) {
        match self {
            Self::Stdio(t) => t.close().await,
            Self::Http(t) => t.close().await,
        }
    }
}

/// An open session with one MCP server. Its methods take `&self`, so a run can share it (in an
/// `Arc`) between the engine and the gateway.
pub struct McpSession {
    name: String,
    peer: Arc<rpc::Peer>,
    transport: Transport,
    protocol_version: String,
    server_info: Value,
    options: SessionOptions,
}

impl McpSession {
    /// Starts or connects to the server `name` with `config`, its `${VAR}` read through `env`,
    /// and initializes the session. A server that does not start, answer or speak a known
    /// protocol version is stopped again before the error is returned.
    pub async fn open(
        name: &str,
        config: &McpServerConfig,
        env: &(dyn Fn(&str) -> Option<String> + Sync),
        options: SessionOptions,
    ) -> Result<McpSession, McpError> {
        let config = config.expanded(env).map_err(|e| {
            McpError::Start(match e {
                Error::Invalid(message) => message,
                other => other.to_string(),
            })
        })?;
        let peer = Arc::new(rpc::Peer::new(name));
        let transport = match config {
            McpServerConfig::Stdio {
                command,
                args,
                cwd,
                env,
            } => Transport::Stdio(stdio::Stdio::start(
                peer.clone(),
                stdio::Launch {
                    command: &command,
                    args: &args,
                    cwd: cwd.map(PathBuf::from).or_else(|| options.cwd.clone()),
                    env: &env,
                    grace: options.close_grace,
                },
            )?),
            McpServerConfig::Http { url, headers } => {
                Transport::Http(http::Http::new(peer.clone(), url, headers))
            }
        };
        let mut session = McpSession {
            name: name.to_string(),
            peer,
            transport,
            protocol_version: String::new(),
            server_info: Value::Null,
            options,
        };
        match session.initialize().await {
            Ok(()) => Ok(session),
            Err(e) => {
                session.close().await;
                Err(e)
            }
        }
    }

    async fn initialize(&mut self) -> Result<(), McpError> {
        let params = json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": { "name": "dots", "version": env!("CARGO_PKG_VERSION") }
        });
        let limit = self.options.answer_timeout;
        let result = match self.request("initialize", params, limit, None).await {
            Ok(result) => result,
            Err(McpError::Stopped) => {
                return Err(McpError::Start(
                    "the server exited before it answered".into(),
                ))
            }
            Err(e) => return Err(e),
        };
        let version = result
            .get("protocolVersion")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !KNOWN_PROTOCOL_VERSIONS.contains(&version) {
            return Err(McpError::Protocol(format!(
                "the server speaks MCP version {version:?}, which Dots does not know"
            )));
        }
        self.protocol_version = version.to_string();
        self.server_info = result.get("serverInfo").cloned().unwrap_or(Value::Null);
        if let Transport::Http(http) = &self.transport {
            http.ready(version);
        }
        self.notify("notifications/initialized", None).await
    }

    /// The server's name as the Library has it.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The protocol version agreed at `initialize`.
    pub fn protocol_version(&self) -> &str {
        &self.protocol_version
    }

    /// The server's `serverInfo` (`name`, `version`), as it sent it.
    pub fn server_info(&self) -> &Value {
        &self.server_info
    }

    /// The tools, every page (`nextCursor`), within the answer time limit for all pages.
    pub async fn list_tools(&self) -> Result<Vec<McpTool>, McpError> {
        let deadline = tokio::time::Instant::now() + self.options.answer_timeout;
        let mut tools = Vec::new();
        let mut cursors = HashSet::new();
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let params = match &cursor {
                Some(c) => json!({ "cursor": c }),
                None => json!({}),
            };
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            let page = self.request("tools/list", params, left, None).await?;
            let list = page
                .get("tools")
                .and_then(Value::as_array)
                .ok_or_else(|| McpError::Protocol("tools/list gave no tools".into()))?;
            for tool in list {
                tools.push(serde_json::from_value(tool.clone()).map_err(|e| {
                    McpError::Protocol(format!("tools/list gave a tool Dots cannot read: {e}"))
                })?);
            }
            match page.get("nextCursor").and_then(Value::as_str) {
                Some(next) if !next.is_empty() => {
                    if !cursors.insert(next.to_string()) {
                        return Err(McpError::Protocol("tools/list repeats a page".into()));
                    }
                    cursor = Some(next.to_string());
                }
                _ => return Ok(tools),
            }
        }
        Err(McpError::Protocol(format!(
            "tools/list has more than {MAX_PAGES} pages"
        )))
    }

    /// Calls `tool` and returns the `tools/call` result as the server sent it (`content`,
    /// `isError`, ...). A result with `isError: true` is a result, not an error; a JSON-RPC
    /// error is `McpError::Server`. Fails with `Timeout` after `limit` and with `Cancelled` as
    /// soon as `cancel` fires; the server is told with `notifications/cancelled`.
    pub async fn call_tool(
        &self,
        tool: &str,
        arguments: Value,
        limit: Duration,
        cancel: &CancellationToken,
    ) -> Result<Value, McpError> {
        let params = json!({ "name": tool, "arguments": arguments });
        let result = self
            .request("tools/call", params, limit, Some(cancel))
            .await?;
        if !result.is_object() {
            return Err(McpError::Protocol(
                "tools/call gave a result that is not an object".into(),
            ));
        }
        Ok(result)
    }

    /// Counts the server's `notifications/tools/list_changed`: `changed()` wakes on each one.
    /// The session does not list again by itself; the caller does.
    pub fn list_changed(&self) -> watch::Receiver<u64> {
        self.peer.list_changed()
    }

    /// The server has stopped, or the session was closed.
    pub fn is_stopped(&self) -> bool {
        self.peer.is_stopped()
    }

    /// Ends the session: an http session is deleted at the server; a stdio server's input is
    /// closed, and it is killed with everything it started if it has not exited after the
    /// grace time. Returns once it is gone. Closing twice does nothing more.
    pub async fn close(&self) {
        self.transport.close().await;
        self.peer.stop();
    }

    async fn notify(&self, method: &str, params: Option<Value>) -> Result<(), McpError> {
        let message = rpc::notification(method, params);
        match tokio::time::timeout(NOTE_TIMEOUT, self.transport.send(&message, None)).await {
            Ok(sent) => sent,
            Err(_) => Err(McpError::Timeout),
        }
    }

    async fn request(
        &self,
        method: &str,
        params: Value,
        limit: Duration,
        cancel: Option<&CancellationToken>,
    ) -> Result<Value, McpError> {
        if self.peer.is_stopped() {
            return Err(McpError::Stopped);
        }
        let (id, answer) = self.peer.expect();
        let message = rpc::request(id, method, params);
        let exchange = async {
            self.transport.send(&message, Some(id)).await?;
            answer.await.unwrap_or(Err(McpError::Stopped))
        };
        let never = CancellationToken::new();
        let cancel = cancel.unwrap_or(&never);
        let outcome = tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(McpError::Cancelled),
            r = tokio::time::timeout(limit, exchange) => r.unwrap_or(Err(McpError::Timeout)),
        };
        if outcome.is_err() {
            self.peer.forget(id);
        }
        // `initialize` is never cancelled (MCP); a server that did not answer it is stopped.
        if matches!(outcome, Err(McpError::Timeout | McpError::Cancelled)) && method != "initialize"
        {
            let reason = match outcome {
                Err(McpError::Timeout) => "timed out",
                _ => "cancelled",
            };
            let params = json!({ "requestId": id, "reason": reason });
            self.transport
                .notify_detached(rpc::notification("notifications/cancelled", Some(params)));
        }
        outcome
    }
}
