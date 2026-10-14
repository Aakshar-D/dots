use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::mpsc;

use super::{Engine, EngineEvent, PermissionGate, PermissionOutcome, RunContext};
use crate::model::ApplyState;
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
    /// Performs `RunContext::approved_calls` the way a self-applying engine does, without
    /// running anything: claims each through the gate, emits its tool call, settles it as
    /// applied and emits the result `performed`.
    ApplyApproved,
    /// Claims the first approved call and leaves it without an outcome.
    ClaimApproved,
}

#[derive(Clone)]
pub struct ScriptedEngine {
    steps: Arc<Vec<Step>>,
    gate: Option<Arc<dyn PermissionGate>>,
    /// The script of a run that is handed approved calls. `Some` makes this an engine that
    /// performs such calls itself.
    resume_steps: Option<Arc<Vec<Step>>>,
}

impl ScriptedEngine {
    pub fn new(steps: Vec<Step>) -> Self {
        Self {
            steps: Arc::new(steps),
            gate: None,
            resume_steps: None,
        }
    }

    pub fn with_gate(steps: Vec<Step>, gate: Arc<dyn PermissionGate>) -> Self {
        Self {
            steps: Arc::new(steps),
            gate: Some(gate),
            resume_steps: None,
        }
    }

    /// Makes this an engine that performs approved calls itself
    /// (`Engine::applies_approved_calls`). A run that is handed such calls follows `steps`
    /// instead of the usual script.
    pub fn applying(mut self, steps: Vec<Step>) -> Self {
        self.resume_steps = Some(Arc::new(steps));
        self
    }
}

#[async_trait]
impl Engine for ScriptedEngine {
    async fn start(&self, ctx: RunContext) -> Result<mpsc::Receiver<EngineEvent>> {
        let (tx, rx) = mpsc::channel(64);
        let steps = match &self.resume_steps {
            Some(resume) if !ctx.approved_calls.is_empty() => resume.clone(),
            _ => self.steps.clone(),
        };
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
                    Step::ApplyApproved => {
                        let Some(gate) = &gate else { continue };
                        for approval in &ctx.approved_calls {
                            if !matches!(gate.claim_apply(&approval.id).await, Ok(true)) {
                                continue;
                            }
                            let id = format!("approval:{}", approval.id);
                            let call = EngineEvent::ToolCall {
                                id: id.clone(),
                                tool: approval.tool.clone(),
                                input: approval.input.clone(),
                            };
                            let _ = tx.send(call).await;
                            let _ = gate.settle_apply(&approval.id, ApplyState::Applied).await;
                            let result = EngineEvent::ToolResult {
                                id,
                                output: "performed".to_string(),
                                is_error: false,
                            };
                            let _ = tx.send(result).await;
                        }
                    }
                    Step::ClaimApproved => {
                        if let (Some(gate), Some(first)) = (&gate, ctx.approved_calls.first()) {
                            let _ = gate.claim_apply(&first.id).await;
                        }
                    }
                }
            }
        });
        Ok(rx)
    }

    fn applies_approved_calls(&self) -> bool {
        self.resume_steps.is_some()
    }
}
