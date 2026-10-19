//! JSON-RPC 2.0 between a session and its server: messages out, and the `Peer` that takes what
//! the server sends, whichever transport carries it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde_json::{json, Value};
use tokio::sync::{oneshot, watch};

use super::McpError;

type Answer = Result<Value, McpError>;

pub(super) fn request(id: u64, method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

pub(super) fn notification(method: &str, params: Option<Value>) -> Value {
    match params {
        Some(params) => json!({ "jsonrpc": "2.0", "method": method, "params": params }),
        None => json!({ "jsonrpc": "2.0", "method": method }),
    }
}

/// Whether `message` is the answer (a result or an error) to the request `id`.
pub(super) fn is_answer_to(message: &Value, id: u64) -> bool {
    message.get("method").is_none() && message.get("id").and_then(Value::as_u64) == Some(id)
}

/// The requests waiting for an answer, and what the server has said outside of answers.
pub(super) struct Peer {
    name: String,
    next_id: AtomicU64,
    /// `None` once the server has stopped: no request waits any more.
    pending: Mutex<Option<HashMap<u64, oneshot::Sender<Answer>>>>,
    list_changed: watch::Sender<u64>,
}

impl Peer {
    pub(super) fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            next_id: AtomicU64::new(1),
            pending: Mutex::new(Some(HashMap::new())),
            list_changed: watch::Sender::new(0),
        }
    }

    pub(super) fn name(&self) -> &str {
        &self.name
    }

    /// A new request id and where its answer arrives. Taken before the request is sent, so an
    /// answer cannot come first. Once the server has stopped, the answer is `Stopped` at once.
    pub(super) fn expect(&self) -> (u64, oneshot::Receiver<Answer>) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        match self.pending.lock().unwrap().as_mut() {
            Some(pending) => {
                pending.insert(id, tx);
            }
            None => {
                let _ = tx.send(Err(McpError::Stopped));
            }
        }
        (id, rx)
    }

    /// Drops the request `id`: it timed out or was cancelled, and a late answer is ignored.
    pub(super) fn forget(&self, id: u64) {
        if let Some(pending) = self.pending.lock().unwrap().as_mut() {
            pending.remove(&id);
        }
    }

    /// The server has stopped: every waiting request fails with `Stopped`, and so does every
    /// later one.
    pub(super) fn stop(&self) {
        if let Some(pending) = self.pending.lock().unwrap().take() {
            for (_, tx) in pending {
                let _ = tx.send(Err(McpError::Stopped));
            }
        }
    }

    pub(super) fn is_stopped(&self) -> bool {
        self.pending.lock().unwrap().is_none()
    }

    pub(super) fn list_changed(&self) -> watch::Receiver<u64> {
        self.list_changed.subscribe()
    }

    /// Takes one message from the server. An answer goes to the request waiting for it;
    /// `notifications/tools/list_changed` is counted; other notifications are ignored. A request
    /// is refused with a JSON-RPC error, except `ping`: the reply to send back is returned.
    pub(super) fn receive(&self, message: Value) -> Option<Value> {
        let method = message.get("method").and_then(Value::as_str);
        let id = message.get("id");
        match (method, id) {
            (Some("ping"), Some(id)) => Some(json!({ "jsonrpc": "2.0", "id": id, "result": {} })),
            (Some(method), Some(id)) => Some(json!({
                "jsonrpc": "2.0", "id": id,
                "error": { "code": -32601, "message": format!("Dots does not support {method}") }
            })),
            (Some("notifications/tools/list_changed"), None) => {
                self.list_changed.send_modify(|n| *n += 1);
                None
            }
            (Some(_), None) => None,
            (None, Some(id)) => {
                let waiting = id.as_u64().and_then(|id| {
                    self.pending
                        .lock()
                        .unwrap()
                        .as_mut()
                        .and_then(|p| p.remove(&id))
                });
                match waiting {
                    Some(tx) => {
                        let _ = tx.send(answer(&message));
                    }
                    None => {
                        tracing::debug!(server = %self.name, "an answer nobody waits for: {id}")
                    }
                }
                None
            }
            (None, None) => {
                tracing::debug!(server = %self.name, "a message that is not JSON-RPC");
                None
            }
        }
    }
}

fn answer(message: &Value) -> Answer {
    match message.get("error") {
        Some(error) => Err(McpError::Server {
            code: error.get("code").and_then(Value::as_i64).unwrap_or(0),
            message: error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("no message")
                .to_string(),
        }),
        None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
    }
}
