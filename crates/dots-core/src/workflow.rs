//! Workflows (teams and workflows spec section 5): a team's ordered list of steps, with a
//! trigger and a budget. This module holds the definition as the builder saves it and its
//! validation (section 5.8). Steps name members by dot id, so renaming a dot changes nothing.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// The most steps a workflow may have.
pub const MAX_STEPS: usize = 50;
/// The longest step name, in characters after trimming.
pub const MAX_NAME: usize = 80;
/// The longest job of an agent step, in characters after trimming.
pub const MAX_JOB: usize = 8_000;

/// What starts a workflow run (section 5.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Trigger {
    /// "Run now" or "Test run": the person types the task and picks the project.
    Manual,
    /// A member starts it with `start_workflow` while chatting with the person. `starters` are
    /// the members (dot ids) that may.
    Chat { starters: Vec<String> },
}

/// One entry of the list (sections 5.2 and 5.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    /// Stable, made by the builder: 1 to 40 of `a-z`, `0-9`, `-` and `_`.
    pub id: String,
    pub name: String,
    #[serde(flatten)]
    pub kind: StepKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StepKind {
    /// A member (a dot id) does one job, added to its own instructions.
    Agent { member: String, job: String },
    /// The workflow waits for the person to approve.
    Check {},
}

/// The limits of one workflow run (section 5.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Budget {
    pub max_runs: u32,
    pub max_tokens: u64,
    pub max_minutes: u32,
    pub max_send_backs: u32,
}

impl Default for Budget {
    fn default() -> Self {
        Budget {
            max_runs: 10,
            max_tokens: 1_500_000,
            max_minutes: 120,
            max_send_backs: 3,
        }
    }
}

/// A workflow as the builder saves it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Definition {
    pub trigger: Trigger,
    pub steps: Vec<Step>,
    #[serde(default)]
    pub budget: Budget,
}

fn invalid<T>(message: String) -> Result<T> {
    Err(Error::Invalid(message))
}

fn id_ok(id: &str) -> bool {
    (1..=40).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

impl Definition {
    /// Checks the definition against the team's `members` (dot ids): section 5.8's rules that
    /// apply to the kinds above. The message names the step it is about.
    pub fn validate(&self, members: &[String]) -> Result<()> {
        let member = |id: &str| members.iter().any(|m| m == id);
        if let Trigger::Chat { starters } = &self.trigger {
            if starters.is_empty() {
                return invalid("a chat trigger needs at least one member who starts it".into());
            }
            if let Some(s) = starters.iter().find(|s| !member(s)) {
                return invalid(format!(
                    "the trigger names {s}, who is not a member of the team"
                ));
            }
        }
        if self.steps.is_empty() {
            return invalid("a workflow needs at least one step".into());
        }
        if self.steps.len() > MAX_STEPS {
            return invalid(format!("a workflow has at most {MAX_STEPS} steps"));
        }
        let mut ids = HashSet::new();
        for step in &self.steps {
            let name = step.name.trim();
            if name.is_empty() || name.chars().count() > MAX_NAME {
                return invalid(format!(
                    "step {:?}: a name must be 1 to {MAX_NAME} characters",
                    step.id
                ));
            }
            if !id_ok(&step.id) {
                return invalid(format!(
                    "step \"{name}\": its id must be 1 to 40 of a-z, 0-9, '-' and '_'"
                ));
            }
            if !ids.insert(step.id.as_str()) {
                return invalid(format!(
                    "step \"{name}\": another step has the id {:?}",
                    step.id
                ));
            }
            if let StepKind::Agent { member: m, job } = &step.kind {
                if !member(m) {
                    return invalid(format!("step \"{name}\": {m} is not a member of the team"));
                }
                let job = job.trim();
                if job.is_empty() || job.chars().count() > MAX_JOB {
                    return invalid(format!(
                        "step \"{name}\": the job must be 1 to 8,000 characters"
                    ));
                }
            }
        }
        let b = &self.budget;
        if b.max_runs == 0 || b.max_tokens == 0 || b.max_minutes == 0 || b.max_send_backs == 0 {
            return invalid("every budget limit must be at least 1".into());
        }
        Ok(())
    }

    /// The step with `id` and its place in the list.
    pub fn step(&self, id: &str) -> Option<(usize, &Step)> {
        self.steps.iter().enumerate().find(|(_, s)| s.id == id)
    }

    /// Whether the member `dot_id` may start this workflow from a chat.
    pub fn started_from_chat_by(&self, dot_id: &str) -> bool {
        matches!(&self.trigger, Trigger::Chat { starters } if starters.iter().any(|s| s == dot_id))
    }
}
