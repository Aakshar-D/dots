//! Minimal non-streaming client for an OpenAI-compatible `/v1/chat/completions` endpoint.

use std::time::Duration;

use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::{Error, Result};

/// Delays before the second and third attempt after an unreachable endpoint or a 5xx reply.
pub const DEFAULT_RETRY_DELAYS: [Duration; 2] = [Duration::from_secs(1), Duration::from_secs(2)];

/// `{endpoint}/v1/chat/completions`. A trailing `/` or `/v1` on the endpoint is accepted, so
/// both `http://127.0.0.1:1234` and `http://127.0.0.1:1234/v1` work.
pub fn completions_url(endpoint: &str) -> String {
    let base = endpoint.trim().trim_end_matches('/');
    let base = base.strip_suffix("/v1").unwrap_or(base);
    format!("{base}/v1/chat/completions")
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCallRequest {
    pub id: String,
    pub name: String,
    /// As sent by the model: usually a JSON string, sometimes an object.
    pub arguments: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Completion {
    /// The assistant message in request form (string `arguments`, an id on every tool call),
    /// ready to append to the conversation.
    pub message: Value,
    pub content: String,
    pub tool_calls: Vec<ToolCallRequest>,
    pub tokens_in: u64,
    pub tokens_out: u64,
}

enum SendError {
    Retryable(String),
    Fatal(String),
}

fn snippet(s: &str) -> String {
    let t = s.trim();
    if t.chars().count() > 500 {
        format!("{}…", t.chars().take(500).collect::<String>())
    } else {
        t.to_string()
    }
}

#[derive(Clone)]
pub struct ChatClient {
    http: reqwest::Client,
    retry_delays: Vec<Duration>,
}

impl ChatClient {
    pub fn new(retry_delays: Vec<Duration>) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .build()
            .expect("building the HTTP client never fails without TLS options");
        Self { http, retry_delays }
    }

    /// Sends one chat completion request. Unreachable endpoints and 5xx replies are retried
    /// after each of the retry delays; 4xx replies and malformed bodies fail at once with the
    /// endpoint's message. Tool calls without an id get `call_<turn>_<index>`.
    /// `Ok(None)` means `cancel` fired first.
    pub async fn complete(
        &self,
        endpoint: &str,
        body: &Value,
        turn: u32,
        cancel: &CancellationToken,
    ) -> Result<Option<Completion>> {
        let url = completions_url(endpoint);
        let mut attempt = 0;
        loop {
            let sent = tokio::select! {
                r = self.send(&url, body) => r,
                _ = cancel.cancelled() => return Ok(None),
            };
            let error = match sent {
                Ok(v) => return parse_completion(&v, turn).map(Some),
                Err(SendError::Fatal(e)) => return Err(Error::Other(e)),
                Err(SendError::Retryable(e)) => e,
            };
            let Some(delay) = self.retry_delays.get(attempt) else {
                return Err(Error::Other(format!(
                    "{error} (gave up after {} attempts)",
                    attempt + 1
                )));
            };
            attempt += 1;
            tokio::select! {
                _ = tokio::time::sleep(*delay) => {}
                _ = cancel.cancelled() => return Ok(None),
            }
        }
    }

    async fn send(&self, url: &str, body: &Value) -> std::result::Result<Value, SendError> {
        let resp =
            self.http.post(url).json(body).send().await.map_err(|e| {
                SendError::Retryable(format!("local endpoint {url} unreachable: {e}"))
            })?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| {
            SendError::Retryable(format!("reading the reply from {url} failed: {e}"))
        })?;
        if status.is_server_error() {
            return Err(SendError::Retryable(format!(
                "local endpoint returned {status}: {}",
                snippet(&text)
            )));
        }
        if !status.is_success() {
            return Err(SendError::Fatal(format!(
                "local endpoint returned {status}: {}",
                snippet(&text)
            )));
        }
        serde_json::from_str(&text).map_err(|e| {
            SendError::Fatal(format!(
                "local endpoint returned invalid JSON ({e}): {}",
                snippet(&text)
            ))
        })
    }
}

/// Reads `choices[0].message` of a chat completion reply.
pub fn parse_completion(v: &Value, turn: u32) -> Result<Completion> {
    let msg = v.pointer("/choices/0/message").ok_or_else(|| {
        Error::Other(format!(
            "local endpoint reply has no choices[0].message: {}",
            snippet(&v.to_string())
        ))
    })?;
    let content = match msg.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    };
    let tool_calls: Vec<ToolCallRequest> = msg
        .get("tool_calls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(i, tc)| ToolCallRequest {
            id: tc
                .get("id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map_or_else(|| format!("call_{turn}_{i}"), String::from),
            name: tc
                .pointer("/function/name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            arguments: tc
                .pointer("/function/arguments")
                .cloned()
                .unwrap_or(Value::Null),
        })
        .collect();
    let mut message = json!({"role": "assistant", "content": content});
    if !tool_calls.is_empty() {
        let calls: Vec<Value> = tool_calls
            .iter()
            .map(|c| {
                let arguments = match &c.arguments {
                    Value::String(s) => s.clone(),
                    Value::Null => "{}".to_string(),
                    other => other.to_string(),
                };
                json!({
                    "id": c.id,
                    "type": "function",
                    "function": {"name": c.name, "arguments": arguments}
                })
            })
            .collect();
        message["tool_calls"] = Value::Array(calls);
    }
    let usage = |k: &str| {
        v.get("usage")
            .and_then(|u| u.get(k))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    Ok(Completion {
        message,
        content,
        tool_calls,
        tokens_in: usage("prompt_tokens"),
        tokens_out: usage("completion_tokens"),
    })
}
