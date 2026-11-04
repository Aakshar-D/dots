//! Workflows (teams and workflows spec section 5): a team's steps, with a trigger and a budget.
//! This module holds the definition as the builder saves it, its validation (section 5.8) and
//! the moves through its tree: a branch step holds lists of steps of its own. Steps name
//! members by dot id, so renaming a dot changes nothing.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::schedule::Schedule;
use crate::workflows::MAX_TASK;
use crate::{Error, Result};

/// The most steps a workflow may have, counting the steps inside branches.
pub const MAX_STEPS: usize = 50;
/// The longest step name, in characters after trimming.
pub const MAX_NAME: usize = 80;
/// The longest job of an agent step, in characters after trimming.
pub const MAX_JOB: usize = 8_000;
/// The longest outcome label, in characters after trimming.
pub const MAX_OUTCOME: usize = 40;

/// What starts a workflow run (section 5.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Trigger {
    /// "Run now" or "Test run": the person types the task and picks the project.
    Manual,
    /// A member starts it with `start_workflow` while chatting with the person. `starters` are
    /// the members (dot ids) that may.
    Chat { starters: Vec<String> },
    /// Dots starts it at each slot of `schedule`, with this task, on the project named
    /// `project`. The schedule's `catch_up` is ignored: a workflow schedule never catches up.
    Schedule {
        schedule: Schedule,
        task: String,
        project: String,
    },
    /// A `POST` to the workflow's URL with its token starts it. The body's `task` and `project`
    /// (by name) are used; these are the fallbacks for a body without them.
    Webhook {
        #[serde(default)]
        task: Option<String>,
        #[serde(default)]
        project: Option<String>,
    },
}

/// One step (sections 5.2 to 5.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    /// Stable, made by the builder: 1 to 40 of `a-z`, `0-9`, `-` and `_`, unique across the
    /// whole tree.
    pub id: String,
    pub name: String,
    #[serde(flatten)]
    pub kind: StepKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StepKind {
    /// A member (a dot id) does one job, added to its own instructions. With `delegate`, it
    /// may start other members (section 5.3).
    Agent {
        member: String,
        job: String,
        /// The ids of earlier agent steps it may send work back to.
        #[serde(default)]
        can_send_back_to: Vec<String>,
        /// The labels it may report, for a branch on outcome after it.
        #[serde(default)]
        outcomes: Vec<String>,
        #[serde(default)]
        delegate: Option<Delegate>,
    },
    /// The workflow waits for the person to approve or send back.
    Check {
        /// The id of the agent step a send-back goes to; `None`: the nearest agent step before
        /// it on its path.
        #[serde(default)]
        send_back_to: Option<String>,
    },
    /// Dots ships the branches the check before it approved (section 5.5).
    Deploy {
        #[serde(flatten)]
        mode: DeployMode,
    },
    /// The workflow takes the first path whose label matches, else `otherwise`, then goes on
    /// after the branch (section 5.6).
    Branch {
        on: BranchOn,
        paths: Vec<BranchPath>,
        #[serde(default)]
        otherwise: Vec<Step>,
    },
}

/// What a delegate step's member may start (section 5.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delegate {
    /// The members (dot ids) it may start.
    pub may_start: Vec<String>,
    /// The most of them running at once.
    #[serde(default = "default_at_once")]
    pub at_once: u32,
    /// The most it starts in all.
    #[serde(default = "default_total")]
    pub total: u32,
}

/// `Delegate::at_once` when the JSON leaves it out.
fn default_at_once() -> u32 {
    3
}

/// `Delegate::total` when the JSON leaves it out.
fn default_total() -> u32 {
    6
}

/// How a deploy step ships (section 5.5). JSON: `"mode"` beside the step's other fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum DeployMode {
    /// The branches are kept and listed as ready.
    Branch,
    /// `git push` of each branch.
    Push,
    /// `push`, then a pull request against the default branch.
    Pr,
    /// `pr`, then auto-merge with `merge_method`.
    PrMerge {
        #[serde(default)]
        merge_method: MergeMethod,
    },
    /// A member does the deploy as an agent step with this job.
    Member { member: String, job: String },
}

/// How `gh pr merge` merges.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeMethod {
    Merge,
    #[default]
    Squash,
    Rebase,
}

/// What a branch step compares its labels with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchOn {
    /// The outcome the nearest agent step before it reported.
    Outcome,
    /// The name of the workflow run's project.
    Project,
}

/// One path of a branch step: an outcome label or a project name, and its steps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchPath {
    pub label: String,
    pub steps: Vec<Step>,
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

/// Whether `text`, trimmed, is longer than `max` characters.
fn too_long(text: &str, max: usize) -> bool {
    text.trim().chars().count() > max
}

impl StepKind {
    /// The lists of steps inside a branch step, in run order: its paths, then `otherwise`.
    /// Empty for the other kinds.
    fn lists(&self) -> Vec<&[Step]> {
        match self {
            StepKind::Branch {
                paths, otherwise, ..
            } => paths
                .iter()
                .map(|p| p.steps.as_slice())
                .chain(std::iter::once(otherwise.as_slice()))
                .collect(),
            _ => Vec::new(),
        }
    }
}

/// Every step of `steps`, depth first: a branch, then its paths in order, then `otherwise`.
fn walk<'a>(steps: &'a [Step], out: &mut Vec<&'a Step>) {
    for step in steps {
        out.push(step);
        for list in step.kind.lists() {
            walk(list, out);
        }
    }
}

/// The way down to the step `id`: for each level from the top, the list it is in and its place
/// in that list. The last entry is the step itself; the ones before it are its enclosing
/// branches. False when no step has the id.
fn trail<'a>(steps: &'a [Step], id: &str, out: &mut Vec<(&'a [Step], usize)>) -> bool {
    for (i, step) in steps.iter().enumerate() {
        out.push((steps, i));
        if step.id == id {
            return true;
        }
        for list in step.kind.lists() {
            if trail(list, id, out) {
                return true;
            }
        }
        out.pop();
    }
    false
}

/// Whether running `steps` always passes a check: a check in the list, or a branch whose every
/// path and `otherwise` always passes one, at any depth.
fn always_checks(steps: &[Step]) -> bool {
    steps.iter().any(passes_check)
}

/// Whether running `step` always passes a check: it is one, or it is a branch every one of
/// whose lists always passes one.
fn passes_check(step: &Step) -> bool {
    match &step.kind {
        StepKind::Check { .. } => true,
        StepKind::Branch { .. } => step.kind.lists().into_iter().all(always_checks),
        _ => false,
    }
}

impl Definition {
    /// Checks the definition against the team's `members` (dot ids): section 5.8's rules. The
    /// message names the step it is about.
    pub fn validate(&self, members: &[String]) -> Result<()> {
        let member = |id: &str| members.iter().any(|m| m == id);
        self.validate_trigger(&member)?;
        if self.steps.is_empty() {
            return invalid("a workflow needs at least one step".into());
        }
        let all = self.all();
        if all.len() > MAX_STEPS {
            return invalid(format!("a workflow has at most {MAX_STEPS} steps"));
        }
        let mut ids = HashSet::new();
        for step in &all {
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
        }
        // The rules below look back along a step's path, so every id is known to be unique.
        for step in &all {
            self.validate_step(step, &member)?;
        }
        let b = &self.budget;
        if b.max_runs == 0 || b.max_tokens == 0 || b.max_minutes == 0 || b.max_send_backs == 0 {
            return invalid("every budget limit must be at least 1".into());
        }
        Ok(())
    }

    /// The trigger's rules; `member` tells whether a dot id is a member of the team.
    fn validate_trigger(&self, member: &dyn Fn(&str) -> bool) -> Result<()> {
        match &self.trigger {
            Trigger::Manual => {}
            Trigger::Chat { starters } => {
                if starters.is_empty() {
                    return invalid(
                        "a chat trigger needs at least one member who starts it".into(),
                    );
                }
                if let Some(s) = starters.iter().find(|s| !member(s)) {
                    return invalid(format!(
                        "the trigger names {s}, who is not a member of the team"
                    ));
                }
            }
            Trigger::Schedule {
                schedule,
                task,
                project,
            } => {
                match schedule.validate() {
                    Err(Error::Invalid(m)) => {
                        return invalid(format!("the trigger's schedule: {m}"));
                    }
                    other => other?,
                }
                if task.trim().is_empty() || too_long(task, MAX_TASK) {
                    return invalid("the trigger's task must be 1 to 8,000 characters".into());
                }
                if project.trim().is_empty() {
                    return invalid("the trigger needs a project".into());
                }
            }
            Trigger::Webhook { task, .. } => {
                // A blank fallback is no fallback: the body must then carry the task.
                if task.as_deref().is_some_and(|t| too_long(t, MAX_TASK)) {
                    return invalid("the trigger's task must be 1 to 8,000 characters".into());
                }
            }
        }
        Ok(())
    }

    /// The rules of one step's own fields and of what must come before it on its path.
    fn validate_step(&self, step: &Step, member: &dyn Fn(&str) -> bool) -> Result<()> {
        let name = step.name.trim();
        // An agent step `id` before `step` on its path, as a send-back target must be.
        let earlier_agent = |id: &str| {
            self.before(&step.id)
                .iter()
                .any(|s| s.id == id && matches!(s.kind, StepKind::Agent { .. }))
        };
        let refused_target = |id: &str| {
            invalid(format!(
                "step \"{name}\": work can be sent back only to an agent step before it, not to \
                 {id:?}"
            ))
        };
        let member_and_job = |m: &str, job: &str| {
            if !member(m) {
                return invalid(format!("step \"{name}\": {m} is not a member of the team"));
            }
            let job = job.trim();
            if job.is_empty() || job.chars().count() > MAX_JOB {
                return invalid(format!(
                    "step \"{name}\": the job must be 1 to 8,000 characters"
                ));
            }
            Ok(())
        };
        match &step.kind {
            StepKind::Agent {
                member: m,
                job,
                can_send_back_to,
                outcomes,
                delegate,
            } => {
                member_and_job(m, job)?;
                let mut seen = HashSet::new();
                for outcome in outcomes {
                    let outcome = outcome.trim();
                    if outcome.is_empty() || outcome.chars().count() > MAX_OUTCOME {
                        return invalid(format!(
                            "step \"{name}\": an outcome must be 1 to {MAX_OUTCOME} characters"
                        ));
                    }
                    if !seen.insert(outcome) {
                        return invalid(format!(
                            "step \"{name}\": the outcome \"{outcome}\" is listed twice"
                        ));
                    }
                }
                if let Some(id) = can_send_back_to.iter().find(|id| !earlier_agent(id)) {
                    return refused_target(id);
                }
                if let Some(d) = delegate {
                    if d.may_start.is_empty() {
                        return invalid(format!(
                            "step \"{name}\": a delegate step needs at least one member it may \
                             start"
                        ));
                    }
                    if let Some(s) = d.may_start.iter().find(|s| !member(s)) {
                        return invalid(format!(
                            "step \"{name}\": it may start {s}, who is not a member of the team"
                        ));
                    }
                    if d.total == 0 {
                        return invalid(format!(
                            "step \"{name}\": the members it may start in all must be at least 1"
                        ));
                    }
                    if d.at_once == 0 || d.at_once > d.total {
                        return invalid(format!(
                            "step \"{name}\": the members it may start at once must be 1 to {}",
                            d.total
                        ));
                    }
                }
            }
            StepKind::Check { send_back_to } => {
                if let Some(id) = send_back_to.as_deref().filter(|id| !earlier_agent(id)) {
                    return refused_target(id);
                }
            }
            StepKind::Deploy { mode } => {
                if let DeployMode::Member { member: m, job } = mode {
                    member_and_job(m, job)?;
                }
                if !self.checked_before(&step.id) {
                    return invalid(format!(
                        "step \"{name}\": a deploy needs a check before it on every path"
                    ));
                }
            }
            StepKind::Branch { on, paths, .. } => {
                if paths.is_empty() {
                    return invalid(format!("step \"{name}\": a branch needs at least one path"));
                }
                let mut seen = HashSet::new();
                for path in paths {
                    let label = path.label.trim();
                    if label.is_empty() {
                        return invalid(format!("step \"{name}\": every path needs a label"));
                    }
                    // Projects are matched without case, so two labels that differ only in
                    // case could never both be taken.
                    if !seen.insert(label.to_lowercase()) {
                        return invalid(format!(
                            "step \"{name}\": two paths have the label \"{label}\""
                        ));
                    }
                }
                if *on == BranchOn::Outcome {
                    // The outcome a run branches on is the nearest agent step's before it.
                    let source =
                        self.before(&step.id)
                            .into_iter()
                            .rev()
                            .find_map(|s| match &s.kind {
                                StepKind::Agent { outcomes, .. } => Some((s, outcomes)),
                                _ => None,
                            });
                    let Some((source, outcomes)) = source else {
                        return invalid(format!(
                            "step \"{name}\": a branch on outcome needs an agent step before it"
                        ));
                    };
                    let declared = |label: &str| outcomes.iter().any(|o| o.trim() == label);
                    if let Some(path) = paths.iter().find(|p| !declared(p.label.trim())) {
                        return invalid(format!(
                            "step \"{name}\": \"{}\" is not an outcome of the step \"{}\"",
                            path.label.trim(),
                            source.name.trim()
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    /// Every step, depth first in run order: a branch, then its paths in order, then
    /// `otherwise`.
    pub fn all(&self) -> Vec<&Step> {
        let mut out = Vec::new();
        walk(&self.steps, &mut out);
        out
    }

    /// The step with `id`, at any depth.
    pub fn step(&self, id: &str) -> Option<&Step> {
        self.all().into_iter().find(|s| s.id == id)
    }

    /// The place of the step `id` in `all()`, from 0: a prompt's "<n> of <total>" is this
    /// plus one, of `all().len()`.
    pub fn position(&self, id: &str) -> Option<usize> {
        self.all().iter().position(|s| s.id == id)
    }

    /// The step a workflow run begins with.
    pub fn first(&self) -> Option<&Step> {
        self.steps.first()
    }

    /// The step after `id` once it has run: its next sibling, else the step after its
    /// enclosing branch, and so on up the tree. `None` past the end, or for an unknown id. For
    /// a branch step this is the step after the whole branch: the path it takes is the run's
    /// to choose.
    pub fn next_after(&self, id: &str) -> Option<&Step> {
        let mut way = Vec::new();
        if !trail(&self.steps, id, &mut way) {
            return None;
        }
        way.into_iter().rev().find_map(|(list, i)| list.get(i + 1))
    }

    /// The steps that run before `id` on its path, oldest first: at each level from the top,
    /// the siblings before it and then the branch it is in. Steps inside an earlier branch's
    /// paths are not in it: which path ran is not known here. Empty for an unknown id.
    pub fn before(&self, id: &str) -> Vec<&Step> {
        let mut way = Vec::new();
        if !trail(&self.steps, id, &mut way) {
            return Vec::new();
        }
        let last = way.len() - 1;
        let mut out = Vec::new();
        for (level, (list, i)) in way.into_iter().enumerate() {
            out.extend(&list[..i]);
            if level < last {
                out.push(&list[i]);
            }
        }
        out
    }

    /// Whether every path to the step `id` passes a check before it: at some level, an
    /// earlier sibling is a check or a branch that always passes one. The enclosing branches
    /// do not count themselves: one of their lists is the step's own path, where a check may
    /// come after the step.
    fn checked_before(&self, id: &str) -> bool {
        let mut way = Vec::new();
        trail(&self.steps, id, &mut way)
            && way
                .into_iter()
                .any(|(list, i)| list[..i].iter().any(passes_check))
    }

    /// Whether the member `dot_id` may start this workflow from a chat.
    pub fn started_from_chat_by(&self, dot_id: &str) -> bool {
        matches!(&self.trigger, Trigger::Chat { starters } if starters.iter().any(|s| s == dot_id))
    }
}
