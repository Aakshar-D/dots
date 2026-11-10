use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::mpsc;

use super::{
    Compaction, Engine, EngineEvent, Handoff, PermissionGate, PermissionOutcome, RunContext,
};
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
    /// Emits text without pause, ignoring cancellation, until the receiver is dropped (an
    /// engine flooding the runner with events).
    Flood,
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
    /// What `compact` answers: `None` cannot compact, `Some(Err)` fails with the text.
    compaction: Option<std::result::Result<Compaction, String>>,
    /// What `handoff_summary` answers, likewise.
    summary: Option<std::result::Result<String, String>>,
    /// What the runner asked, in order: `compact <session>`, `handoff <session>`,
    /// `start <session or ->`, each start followed by its prompt (`prompt <text>`).
    log: Arc<std::sync::Mutex<Vec<String>>>,
    /// What a hand-off reports it used (`passes_using`).
    pass_usage: super::PassUsage,
    /// Each extra call waits this long first (`passes_taking`).
    pass_delay: Duration,
}

impl ScriptedEngine {
    pub fn new(steps: Vec<Step>) -> Self {
        Self {
            steps: Arc::new(steps),
            gate: None,
            resume_steps: None,
            compaction: None,
            summary: None,
            log: Arc::default(),
            pass_usage: super::PassUsage::default(),
            pass_delay: Duration::ZERO,
        }
    }

    /// Makes `compact` answer `outcome`.
    pub fn compacting(mut self, outcome: std::result::Result<Compaction, String>) -> Self {
        self.compaction = Some(outcome);
        self
    }

    /// Makes `handoff_summary` answer `outcome`.
    pub fn summarizing(mut self, outcome: std::result::Result<String, String>) -> Self {
        self.summary = Some(outcome);
        self
    }

    /// Makes a hand-off report `usage` (a compaction reports its own, in `compacting`).
    pub fn passes_using(mut self, usage: super::PassUsage) -> Self {
        self.pass_usage = usage;
        self
    }

    /// Makes each extra call wait `delay` first, or until the run is cancelled.
    pub fn passes_taking(mut self, delay: Duration) -> Self {
        self.pass_delay = delay;
        self
    }

    /// Waits `pass_delay`; an error when the run is cancelled meanwhile.
    async fn pass_wait(&self, ctx: &RunContext) -> Result<()> {
        if self.pass_delay.is_zero() {
            return Ok(());
        }
        tokio::select! {
            _ = tokio::time::sleep(self.pass_delay) => Ok(()),
            _ = ctx.cancel.cancelled() => Err(crate::Error::Other("cancelled".into())),
        }
    }

    /// What the runner asked of this engine so far (`log`).
    pub fn log(&self) -> Vec<String> {
        self.log.lock().expect("log lock").clone()
    }

    pub fn with_gate(steps: Vec<Step>, gate: Arc<dyn PermissionGate>) -> Self {
        Self {
            steps: Arc::new(steps),
            gate: Some(gate),
            resume_steps: None,
            compaction: None,
            summary: None,
            log: Arc::default(),
            pass_usage: super::PassUsage::default(),
            pass_delay: Duration::ZERO,
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
        {
            let mut log = self.log.lock().expect("log lock");
            log.push(format!(
                "start {}",
                ctx.session_id.as_deref().unwrap_or("-")
            ));
            log.push(format!("prompt {}", ctx.prompt));
        }
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
                            parent: None,
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
                    Step::Flood => {
                        while tx
                            .send(EngineEvent::AssistantText { text: "x".into() })
                            .await
                            .is_ok()
                        {}
                        return;
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
                                parent: None,
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

    async fn compact(&self, ctx: &RunContext) -> Result<Option<Compaction>> {
        self.log.lock().expect("log lock").push(format!(
            "compact {}",
            ctx.session_id.as_deref().unwrap_or("-")
        ));
        self.pass_wait(ctx).await?;
        match &self.compaction {
            None => Ok(None),
            Some(Ok(c)) => Ok(Some(*c)),
            Some(Err(e)) => Err(crate::Error::Other(e.clone())),
        }
    }

    async fn handoff_summary(&self, ctx: &RunContext) -> Result<Option<Handoff>> {
        self.log.lock().expect("log lock").push(format!(
            "handoff {}",
            ctx.session_id.as_deref().unwrap_or("-")
        ));
        self.pass_wait(ctx).await?;
        match &self.summary {
            None => Ok(None),
            Some(Ok(s)) => Ok(Some(Handoff {
                summary: s.clone(),
                usage: self.pass_usage,
            })),
            Some(Err(e)) => Err(crate::Error::Other(e.clone())),
        }
    }
}
