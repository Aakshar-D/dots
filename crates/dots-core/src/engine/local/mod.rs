//! Local engine: a tool loop against an OpenAI-compatible chat endpoint (Ollama, LM Studio).

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::engine::{Engine, EngineEvent, PermissionGate, PermissionOutcome, RunContext};
use crate::store::Store;
use crate::{Error, Result};

use self::client::{ChatClient, ToolCallRequest, DEFAULT_RETRY_DELAYS};
use self::tools::{PreparedCall, ToolOutput};

pub mod client;
pub mod history;
pub mod tools;

const SYSTEM_PROMPT: &str = "You are a dot: an autonomous agent working unattended in a \
    workspace on the user's computer. Use the tools to inspect and change files and to run \
    commands; paths are relative to the workspace root. When the task is done, reply with a \
    short summary and no tool calls.";

/// How long `probe` waits; LM Studio may load the model on the first request.
const PROBE_TIMEOUT: Duration = Duration::from_secs(120);

/// Asks `model` at `endpoint` to call a one-argument `echo` tool and checks that the reply is a
/// well-formed tool call. Returns a short success message; every failure is an error that
/// says what the endpoint or model did instead.
pub async fn probe(endpoint: &str, model: &str) -> Result<String> {
    crate::model::check_endpoint_url(endpoint)?;
    if model.trim().is_empty() {
        return Err(Error::Invalid("model must not be empty".into()));
    }
    let body = json!({
        "model": model,
        "messages": [{
            "role": "user",
            "content": "Call the echo tool with text set to \"ping\". Do not answer with text."
        }],
        "tools": [{
            "type": "function",
            "function": {
                "name": "echo",
                "description": "Echo the given text back.",
                "parameters": {
                    "type": "object",
                    "properties": {"text": {"type": "string"}},
                    "required": ["text"]
                }
            }
        }],
        "stream": false
    });
    let client = ChatClient::new(Vec::new());
    let never = CancellationToken::new();
    let completion =
        tokio::time::timeout(PROBE_TIMEOUT, client.complete(endpoint, &body, 1, &never))
            .await
            .map_err(|_| {
                Error::Other(format!(
                    "{model} did not answer within {} s",
                    PROBE_TIMEOUT.as_secs()
                ))
            })??
            .ok_or_else(|| Error::Other("probe cancelled".into()))?;
    let Some(call) = completion.tool_calls.first() else {
        return Err(Error::Invalid(format!(
            "{model} answered with text instead of a tool call: {}",
            completion.content.trim()
        )));
    };
    if call.name != "echo" {
        return Err(Error::Invalid(format!(
            "{model} called an unknown tool '{}'",
            call.name
        )));
    }
    let args = match &call.arguments {
        Value::String(s) => serde_json::from_str::<Value>(s).unwrap_or(Value::Null),
        other => other.clone(),
    };
    match args.get("text").and_then(Value::as_str) {
        Some(_) => Ok(format!("{model} returned a well-formed tool call")),
        None => Err(Error::Invalid(format!(
            "{model} sent malformed tool arguments: {}",
            call.arguments
        ))),
    }
}

pub struct LocalEngine {
    store: Store,
    gate: Arc<dyn PermissionGate>,
    client: ChatClient,
}

impl LocalEngine {
    pub fn new(store: Store, gate: Arc<dyn PermissionGate>) -> Self {
        Self::with_retry_delays(store, gate, DEFAULT_RETRY_DELAYS.to_vec())
    }

    pub fn with_retry_delays(
        store: Store,
        gate: Arc<dyn PermissionGate>,
        retry_delays: Vec<Duration>,
    ) -> Self {
        Self {
            store,
            gate,
            client: ChatClient::new(retry_delays),
        }
    }
}

#[async_trait]
impl Engine for LocalEngine {
    async fn start(&self, ctx: RunContext) -> Result<mpsc::Receiver<EngineEvent>> {
        let endpoint = ctx
            .dot
            .spec
            .endpoint_url
            .clone()
            .filter(|u| !u.trim().is_empty())
            .ok_or_else(|| Error::Invalid("the local engine requires endpoint_url".into()))?;
        let messages = match ctx.session_id {
            Some(_) => history::load(&self.store, &ctx.run_id).await?,
            None => Vec::new(),
        };
        let (tx, rx) = mpsc::channel(256);
        let session = Session {
            gate: self.gate.clone(),
            client: self.client.clone(),
            endpoint,
            ctx,
            tx,
            messages,
        };
        tokio::spawn(session.run());
        Ok(rx)
    }
}

/// One run's conversation. Every method returns `false` once the run should stop (cancelled,
/// or the runner stopped listening).
struct Session {
    gate: Arc<dyn PermissionGate>,
    client: ChatClient,
    endpoint: String,
    ctx: RunContext,
    tx: mpsc::Sender<EngineEvent>,
    messages: Vec<Value>,
}

impl Session {
    async fn emit(&self, ev: EngineEvent) -> bool {
        self.tx.send(ev).await.is_ok()
    }

    /// Appends a message to the conversation and records it as a `message` event.
    async fn push(&mut self, message: Value) -> bool {
        self.messages.push(message.clone());
        self.emit(EngineEvent::Message { message }).await
    }

    async fn run(mut self) {
        // A local session is named after the root run of its lineage.
        let session_id = self
            .ctx
            .session_id
            .clone()
            .unwrap_or_else(|| self.ctx.run_id.clone());
        if !self.emit(EngineEvent::SessionStarted { session_id }).await {
            return;
        }
        if self.messages.is_empty()
            && !self
                .push(json!({"role": "system", "content": SYSTEM_PROMPT}))
                .await
        {
            return;
        }
        let prompt = self.ctx.prompt.clone();
        if !self.push(json!({"role": "user", "content": prompt})).await {
            return;
        }
        let tools = tools::schemas();
        let max_turns = self.ctx.dot.spec.max_turns;
        for turn in 1..=max_turns {
            let body = json!({
                "model": self.ctx.dot.spec.model,
                "messages": self.messages,
                "tools": tools,
                "stream": false
            });
            let completion = match self
                .client
                .complete(&self.endpoint, &body, turn, &self.ctx.cancel)
                .await
            {
                Ok(Some(c)) => c,
                Ok(None) => return,
                Err(e) => {
                    self.emit(EngineEvent::Failed {
                        error: e.to_string(),
                    })
                    .await;
                    return;
                }
            };
            let usage = EngineEvent::Usage {
                tokens_in: completion.tokens_in,
                tokens_out: completion.tokens_out,
            };
            if !self.emit(usage).await || !self.push(completion.message.clone()).await {
                return;
            }
            let text = completion.content.trim().to_string();
            if !text.is_empty()
                && !self
                    .emit(EngineEvent::AssistantText { text: text.clone() })
                    .await
            {
                return;
            }
            if completion.tool_calls.is_empty() {
                self.emit(EngineEvent::Finished { summary: text }).await;
                return;
            }
            for call in &completion.tool_calls {
                if !self.handle(call).await {
                    return;
                }
            }
        }
        self.emit(EngineEvent::Failed {
            error: format!("error_max_turns: stopped after {max_turns} turns"),
        })
        .await;
    }

    /// Checks, gates and runs one tool call, then records its result.
    async fn handle(&mut self, call: &ToolCallRequest) -> bool {
        if self.ctx.cancel.is_cancelled() {
            return false;
        }
        let prepared = tools::prepare(&call.name, &call.arguments, &self.ctx.workspace);
        let (tool, input) = match &prepared {
            Ok(p) => (p.alias.to_string(), p.input.clone()),
            Err(_) => (call.name.clone(), call.arguments.clone()),
        };
        let shown = EngineEvent::ToolCall {
            id: call.id.clone(),
            tool,
            input,
        };
        if !self.emit(shown).await {
            return false;
        }
        let output = match prepared {
            Err(e) => ToolOutput::err(e),
            Ok(p) => {
                let checked = tokio::select! {
                    r = self.gate.check(&self.ctx.run_id, p.alias, p.input.clone()) => r,
                    _ = self.ctx.cancel.cancelled() => return false,
                };
                match checked {
                    Ok(PermissionOutcome::Allow { input }) => {
                        let allowed = PreparedCall { input, ..p };
                        tools::run(&allowed, &self.ctx.workspace, &self.ctx.cancel).await
                    }
                    Ok(PermissionOutcome::Deny { message }) => ToolOutput::err(message),
                    Err(e) => ToolOutput::err(format!("permission check failed: {e}")),
                }
            }
        };
        // A cancel that stopped the tool leaves the call unanswered; `history::repair` fills it.
        if self.ctx.cancel.is_cancelled() {
            return false;
        }
        let result = EngineEvent::ToolResult {
            id: call.id.clone(),
            output: output.output.clone(),
            is_error: output.is_error,
        };
        self.emit(result).await
            && self
                .push(json!({
                    "role": "tool",
                    "tool_call_id": call.id,
                    "content": output.output
                }))
                .await
    }
}
