use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::mpsc;

use super::{Engine, EngineEvent, PermissionGate, PermissionOutcome, RunContext};
use crate::Result;

#[derive(Debug, Clone)]
pub enum Step {
    Emit(EngineEvent),
    Sleep(Duration),
    /// Emits a ToolCall, asks the gate, then emits a ToolResult describing the outcome.
    Ask {
        tool: String,
        input: Value,
    },
    /// Blocks until cancelled, then closes the channel.
    WaitForCancel,
    /// Blocks forever and ignores cancellation (a process that never exits).
    Hang,
}

#[derive(Clone)]
pub struct ScriptedEngine {
    steps: Arc<Vec<Step>>,
    gate: Option<Arc<dyn PermissionGate>>,
}

impl ScriptedEngine {
    pub fn new(steps: Vec<Step>) -> Self {
        Self {
            steps: Arc::new(steps),
            gate: None,
        }
    }

    pub fn with_gate(steps: Vec<Step>, gate: Arc<dyn PermissionGate>) -> Self {
        Self {
            steps: Arc::new(steps),
            gate: Some(gate),
        }
    }
}

#[async_trait]
impl Engine for ScriptedEngine {
    async fn start(&self, ctx: RunContext) -> Result<mpsc::Receiver<EngineEvent>> {
        let (tx, rx) = mpsc::channel(64);
        let steps = self.steps.clone();
        let gate = self.gate.clone();
        tokio::spawn(async move {
            for (i, step) in steps.iter().enumerate() {
                if ctx.cancel.is_cancelled() && !matches!(step, Step::Hang) {
                    return;
                }
                match step {
                    Step::Emit(ev) => {
                        if tx.send(ev.clone()).await.is_err() {
                            return;
                        }
                    }
                    Step::Sleep(d) => {
                        tokio::select! {
                            _ = tokio::time::sleep(*d) => {}
                            _ = ctx.cancel.cancelled() => return,
                        }
                    }
                    Step::Ask { tool, input } => {
                        let id = format!("tool-{i}");
                        let call = EngineEvent::ToolCall {
                            id: id.clone(),
                            tool: tool.clone(),
                            input: input.clone(),
                        };
                        let _ = tx.send(call).await;
                        let outcome = match &gate {
                            Some(g) => g.check(&ctx.run_id, tool, input.clone()).await,
                            None => Ok(PermissionOutcome::Allow {
                                input: input.clone(),
                            }),
                        };
                        let (output, is_error) = match outcome {
                            Ok(PermissionOutcome::Allow { .. }) => ("allowed".to_string(), false),
                            Ok(PermissionOutcome::Deny { message }) => (message, true),
                            Err(e) => (format!("permission check failed: {e}"), true),
                        };
                        let _ = tx
                            .send(EngineEvent::ToolResult {
                                id,
                                output,
                                is_error,
                            })
                            .await;
                    }
                    Step::WaitForCancel => {
                        ctx.cancel.cancelled().await;
                        return;
                    }
                    Step::Hang => {
                        let _keep_open = tx.clone();
                        std::future::pending::<()>().await;
                    }
                }
            }
        });
        Ok(rx)
    }
}
