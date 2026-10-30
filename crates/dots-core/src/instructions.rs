//! A dot proposes new instructions for itself (conversational chat spec section 4): from a
//! person's chat it calls `propose_instructions`, Dots keeps the proposal and posts a card, and
//! the person's answer continues the chat, as a decided app proposal does.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::events::{Bus, RuntimeEvent};
use crate::model::{EngineKind, Run};
use crate::store::Store;
use crate::{Error, Result};

/// The one name the policy, approvals and grants know `propose_instructions` by. Every preset
/// allows it: nothing changes until the person approves the card.
pub const POLICY_NAME: &str = "Instructions";
/// The tool's name on both engines (`mcp__dots__propose_instructions` on the Claude engine).
pub const TOOL: &str = "propose_instructions";
/// What the dot is told when the card is posted.
pub const SENT: &str = "Sent to the person for review. Finish your turn; their answer arrives \
    as your next message.";
/// What a run outside a person's chat is told.
pub const NOT_HERE: &str = "propose_instructions works only in a chat the person started";
/// What a proposal that changes nothing is told.
pub const UNCHANGED: &str = "these are your instructions as they are: send them with the change \
    made";
/// What a decision on a decided proposal is told.
pub const DECIDED_ALREADY: &str = "these instructions have already been decided";
/// What approving a proposal is told when the person edited the instructions since.
pub const STALE: &str = "the dot's instructions changed after it proposed these; ask it to \
    propose them again";
/// What the dot reads when the person approves or rejects.
pub const APPROVED: &str = "The person approved your new instructions; your next runs use them.";
pub const REJECTED: &str = "The person rejected your new instructions; they stay as they were.";
/// The most characters of proposed instructions and of a reason.
pub const MAX_INSTRUCTIONS: usize = 20_000;
pub const MAX_REASON: usize = 500;

pub const DESCRIPTION: &str = "Propose new instructions for yourself, when the person asks you \
    to change how you work from now on. Send the whole new text (your instructions with the \
    change made) and a one-line reason for the person. They approve or reject it on a card; \
    nothing changes until they approve.";

/// The tool's arguments.
pub fn properties() -> Value {
    json!({
        "instructions": {
            "type": "string",
            "description": "Your whole new instructions, with the change made"
        },
        "reason": { "type": "string", "description": "What changes and why, in one line" }
    })
}

pub const REQUIRED: [&str; 2] = ["instructions", "reason"];

/// The name the run's model knows the tool by: Dots' own MCP server's tool on the Claude
/// engine, a built-in tool on the local engine.
pub fn model_name(engine: EngineKind) -> &'static str {
    match engine {
        EngineKind::Claude => "mcp__dots__propose_instructions",
        EngineKind::Local => TOOL,
    }
}

/// The tool as Dots' own `dots` MCP server lists it.
pub fn mcp_schema() -> Value {
    json!({
        "name": TOOL,
        "description": DESCRIPTION,
        "inputSchema": { "type": "object", "properties": properties(), "required": REQUIRED }
    })
}

str_enum!(InstructionStatus {
    Pending => "pending",
    Approved => "approved",
    Rejected => "rejected",
    // A newer proposal of the dot took its place.
    Replaced => "replaced",
});

/// Instructions a dot proposed, and what the person decided.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstructionProposal {
    pub id: String,
    pub dot_id: String,
    pub run_id: String,
    pub root_run_id: String,
    /// The dot's instructions when it proposed: the card's "before".
    pub base: String,
    pub instructions: String,
    pub reason: String,
    pub status: InstructionStatus,
    /// The run the decision started.
    pub answer_run_id: Option<String>,
    pub created_at: String,
    pub decided_at: Option<String>,
}

/// A proposal that waits for the person, as the Inbox lists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingInstructions {
    pub proposal: InstructionProposal,
    pub dot_name: String,
}

/// The run may propose instructions: a run of a topic the person started with a message they
/// typed, not a workflow step's.
pub async fn may_propose(store: &Store, run: &Run) -> Result<bool> {
    if run.workflow_step_run_id.is_some() {
        return Ok(false);
    }
    let root = if run.root_run_id == run.id {
        run.clone()
    } else {
        store.get_run(&run.root_run_id).await?
    };
    Ok(crate::prompt::is_typed(&root))
}

/// The proposals, for both engines' `propose_instructions`. Cheap to clone.
#[derive(Clone)]
pub struct Proposals {
    store: Store,
    bus: Bus,
    /// The follow-ups' thread lock (`FollowUps::thread_lock`), which the decisions hold too: a
    /// proposal never replaces one while it is being approved.
    thread_lock: Arc<Mutex<()>>,
}

impl Proposals {
    pub fn new(store: Store, bus: Bus, thread_lock: Arc<Mutex<()>>) -> Self {
        Self {
            store,
            bus,
            thread_lock,
        }
    }

    /// `propose_instructions` for the run `run_id`: checks the text, keeps the proposal in
    /// place of the dot's waiting one and posts the card. `Ok` is what the dot is told; `Err`
    /// the problem, in a sentence for the dot.
    pub async fn propose(
        &self,
        run_id: &str,
        instructions: &str,
        reason: &str,
    ) -> std::result::Result<String, String> {
        let said = |e: Error| match e {
            Error::Invalid(m) | Error::Conflict(m) | Error::NotFound(m) => m,
            other => other.to_string(),
        };
        let instructions = instructions.trim();
        let reason = reason.trim();
        if instructions.is_empty() || instructions.chars().count() > MAX_INSTRUCTIONS {
            return Err(format!(
                "the instructions must be 1 to {MAX_INSTRUCTIONS} characters"
            ));
        }
        if reason.is_empty() || reason.chars().count() > MAX_REASON {
            return Err(format!("the reason must be 1 to {MAX_REASON} characters"));
        }
        let run = self.store.get_run(run_id).await.map_err(said)?;
        if !may_propose(&self.store, &run).await.map_err(said)? {
            return Err(NOT_HERE.to_string());
        }
        // Held from reading the base until the proposal is kept: a decision of the dot's
        // waiting proposal (`Runtime::decide_instruction_proposal`) cannot fall in between.
        // No caller holds it while a run executes, so a run's call never waits on itself.
        let _one_at_a_time = self.thread_lock.lock().await;
        let dot = self.store.get_dot(&run.dot_id).await.map_err(said)?;
        let base = dot.spec.instructions.trim().to_string();
        if base == instructions {
            return Err(UNCHANGED.to_string());
        }
        let proposal = InstructionProposal {
            id: crate::util::new_id(),
            dot_id: run.dot_id.clone(),
            run_id: run.id.clone(),
            root_run_id: run.root_run_id.clone(),
            base,
            instructions: instructions.to_string(),
            reason: reason.to_string(),
            status: InstructionStatus::Pending,
            answer_run_id: None,
            created_at: crate::util::now(),
            decided_at: None,
        };
        let replaced = self
            .store
            .add_instruction_proposal(&proposal)
            .await
            .map_err(said)?;
        for id in replaced {
            self.changed(&id, &proposal.dot_id);
        }
        self.changed(&proposal.id, &proposal.dot_id);
        Ok(SENT.to_string())
    }

    pub(crate) fn changed(&self, id: &str, dot_id: &str) {
        let _ = self.bus.send(RuntimeEvent::InstructionProposalChanged {
            id: id.to_string(),
            dot_id: dot_id.to_string(),
        });
    }
}
