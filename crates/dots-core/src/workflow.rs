//! Workflows (teams and workflows spec section 5): a team's steps, with a trigger and a budget.
//! This module holds the definition as the builder saves it, its validation (section 5.8) and
//! the moves through its tree: a branch step holds lists of steps of its own. Steps name
//! members by dot id, so renaming a dot changes nothing.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::model::ModelTier;
use crate::schedule::Schedule;
use crate::step_template::{self, Mark};
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
/// The most inputs a workflow asks for, and the most outputs an agent step gives.
pub const MAX_INPUTS: usize = 20;
pub const MAX_OUTPUTS: usize = 10;
/// The longest URL of an action, the most headers it sends, and its longest header value and
/// body, in characters.
pub const MAX_URL: usize = 2_000;
pub const MAX_HEADERS: usize = 20;
pub const MAX_HEADER_VALUE: usize = 2_000;
pub const MAX_BODY: usize = 20_000;
/// The longest path and content of a file an action writes, in characters.
pub const MAX_FILE_PATH: usize = 200;
pub const MAX_FILE_CONTENT: usize = 100_000;
/// The longest wait of a wait step: a year, in minutes.
pub const MAX_WAIT_MINUTES: u32 = 366 * 24 * 60;

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
    /// `project` (blank: the team's own folder). The schedule's own `catch_up` is ignored;
    /// with this `catch_up`, the newest slot missed while Dots was closed starts once, when
    /// it is less than a day old (general workflows spec section 9).
    Schedule {
        schedule: Schedule,
        task: String,
        project: String,
        #[serde(default)]
        catch_up: bool,
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
        /// What makes its run cheaper (lean workflows spec section 2). A workflow saved
        /// before these existed has none, and reads as one with every default.
        #[serde(default)]
        options: StepOptions,
        /// The names of the outputs it gives (general workflows spec section 5): its reply ends
        /// with each in a block of its name, and a later action uses them.
        #[serde(default)]
        produces: Vec<String>,
    },
    /// Dots itself does one thing, with no model (general workflows spec section 6).
    Action {
        #[serde(flatten)]
        action: Action,
        /// It may run with no check before it. Off, an action that sends something out needs
        /// a check before it on every path, as a deploy does.
        #[serde(default)]
        unattended: bool,
    },
    /// The workflow waits for a time, with no model and no one to ask (general workflows spec
    /// section 7).
    Wait {
        #[serde(flatten)]
        wait: Wait,
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

/// What an action step does (general workflows spec section 6). JSON: `"action"` beside the
/// step's other fields. Each text may hold `{{…}}` marks (`step_template`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Action {
    /// Sends one HTTP request. The step succeeds on a 2xx answer.
    Http {
        method: HttpMethod,
        url: String,
        #[serde(default)]
        headers: Vec<Header>,
        #[serde(default)]
        body: String,
    },
    /// Writes a text file into the project's folder, replacing one of that path.
    File { path: String, content: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl HttpMethod {
    /// The method as HTTP writes it.
    pub fn as_str(self) -> &'static str {
        match self {
            HttpMethod::Get => "GET",
            HttpMethod::Post => "POST",
            HttpMethod::Put => "PUT",
            HttpMethod::Patch => "PATCH",
            HttpMethod::Delete => "DELETE",
        }
    }
}

/// One header of an HTTP action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    pub name: String,
    pub value: String,
}

/// How long a wait step waits (general workflows spec section 7). JSON: `"wait"` beside the
/// step's other fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "wait", rename_all = "snake_case")]
pub enum Wait {
    /// For this many minutes from when the step begins.
    Minutes { minutes: u32 },
    /// Until the next `at` (`HH:MM`, this PC's time) after the step begins, on `weekday` when
    /// one is set.
    Time {
        at: String,
        #[serde(default)]
        weekday: Option<chrono::Weekday>,
    },
    /// Until the date and time the run's input `input` holds.
    Input { input: String },
}

/// The time of day `at` names, written `HH:MM`.
pub fn time_of_day(at: &str) -> Option<chrono::NaiveTime> {
    chrono::NaiveTime::parse_from_str(at.trim(), "%H:%M").ok()
}

/// Something a run is started with besides its task (general workflows spec section 3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputField {
    /// How steps name it: 1 to 40 of `a-z`, `0-9`, `-` and `_`.
    pub name: String,
    /// What the person is asked for.
    pub label: String,
    #[serde(default)]
    pub kind: InputKind,
    #[serde(default)]
    pub required: bool,
    /// What a start that gives none uses.
    #[serde(default)]
    pub default: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputKind {
    /// One line.
    #[default]
    Text,
    /// Several lines.
    LongText,
    /// A date and a time of this PC, written `YYYY-MM-DDTHH:MM`.
    DateTime,
}

/// The date and time `value` names, written `YYYY-MM-DDTHH:MM` (seconds allowed), as this
/// PC's clock reads it.
pub fn date_time_of(value: &str) -> Option<chrono::NaiveDateTime> {
    let value = value.trim();
    ["%Y-%m-%dT%H:%M", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%d %H:%M"]
        .iter()
        .find_map(|f| chrono::NaiveDateTime::parse_from_str(value, f).ok())
}

/// What makes an agent step's run cheaper (lean workflows spec section 2). Every default is
/// the step as it ran before these existed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepOptions {
    /// The size of model its run starts with, mapped by the settings' Auto models; `None`:
    /// the member's own model. Only a member on the Claude engine has sizes: another engine's
    /// keeps its own model.
    #[serde(default)]
    pub model: Option<ModelTier>,
    /// Its prompt carries the member's instructions and the workflow only: no memories and no
    /// project map, and its run saves no memory.
    #[serde(default)]
    pub lean: bool,
    /// The ids of the earlier steps whose summaries its prompt carries, each in full. `None`:
    /// every earlier step's, the newest in full and the older ones cut short
    /// (`workflows::MAX_OLDER_SUMMARY`).
    #[serde(default)]
    pub reads: Option<Vec<String>>,
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
    /// What a start is asked for besides the task (general workflows spec section 3).
    #[serde(default)]
    pub inputs: Vec<InputField>,
}

/// The first rule of section 5.8 a workflow breaks, and the step it is about, so the builder
/// shows it on that step's card and keeps Save off until it is fixed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowProblem {
    /// The id of the step the message names; `None` for the workflow's name, its trigger, its
    /// budget or its list of steps as a whole.
    pub step_id: Option<String>,
    /// The refusal a save would give.
    pub message: String,
}

impl WorkflowProblem {
    /// The problem `e` (a refusal), about the step `step_id`.
    pub(crate) fn new(step_id: Option<&str>, e: Error) -> Self {
        Self {
            step_id: step_id.map(str::to_string),
            message: match e {
                Error::Invalid(m) => m,
                e => e.to_string(),
            },
        }
    }
}

/// Whether `path`, a file an action writes, stays inside the folder it is joined to: relative,
/// with no drive, no `..` and no empty part. Checked again on the filled path when the step
/// runs.
pub fn relative_path_ok(path: &str) -> bool {
    let path = path.trim();
    !path.is_empty()
        && !path.starts_with(['/', '\\'])
        && !path.contains(':')
        && path
            .split(['/', '\\'])
            .all(|part| !part.is_empty() && part != ".." && part != ".")
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

/// The rules of a step's name and id; `ids` are the ids of the steps before it in `all()`, and
/// gets its own.
fn validate_name_and_id<'a>(step: &'a Step, ids: &mut HashSet<&'a str>) -> Result<()> {
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
    Ok(())
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
    /// message names the step it is about. Refuses with `problem`'s message.
    pub fn validate(&self, members: &[String]) -> Result<()> {
        match self.problem(members) {
            Some(p) => invalid(p.message),
            None => Ok(()),
        }
    }

    /// The first of section 5.8's rules the definition breaks against the team's `members`
    /// (dot ids), with the id of the step it is about; `None` when it breaks none. The one
    /// source of `validate`'s refusals and of the builder's live check.
    pub fn problem(&self, members: &[String]) -> Option<WorkflowProblem> {
        let member = |id: &str| members.iter().any(|m| m == id);
        let whole = |e| Some(WorkflowProblem::new(None, e));
        if let Err(e) = self.validate_trigger(&member) {
            return whole(e);
        }
        if let Err(e) = self.validate_inputs() {
            return whole(e);
        }
        if self.steps.is_empty() {
            return whole(Error::Invalid("a workflow needs at least one step".into()));
        }
        let all = self.all();
        if all.len() > MAX_STEPS {
            return whole(Error::Invalid(format!(
                "a workflow has at most {MAX_STEPS} steps"
            )));
        }
        let mut ids = HashSet::new();
        for step in &all {
            if let Err(e) = validate_name_and_id(step, &mut ids) {
                return Some(WorkflowProblem::new(Some(&step.id), e));
            }
        }
        // The rules below look back along a step's path, so every id is known to be unique.
        for step in &all {
            if let Err(e) = self.validate_step(step, &member) {
                return Some(WorkflowProblem::new(Some(&step.id), e));
            }
        }
        let b = &self.budget;
        if b.max_runs == 0 || b.max_tokens == 0 || b.max_minutes == 0 || b.max_send_backs == 0 {
            return whole(Error::Invalid(
                "every budget limit must be at least 1".into(),
            ));
        }
        None
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
                ..
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
                // A blank project: the team's own folder (general workflows spec section 2).
                let _ = project;
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
                options,
                produces,
            } => {
                member_and_job(m, job)?;
                if produces.len() > MAX_OUTPUTS {
                    return invalid(format!(
                        "step \"{name}\": a step gives at most {MAX_OUTPUTS} outputs"
                    ));
                }
                let mut given = HashSet::new();
                for output in produces {
                    if !step_template::name_ok(output) || output == "summary" {
                        return invalid(format!(
                            "step \"{name}\": an output's name must be 1 to 40 of a-z, 0-9, '-' \
                             and '_', and not \"summary\": {output:?}"
                        ));
                    }
                    if !given.insert(output.as_str()) {
                        return invalid(format!(
                            "step \"{name}\": the output \"{output}\" is listed twice"
                        ));
                    }
                }
                if let Some(reads) = &options.reads {
                    // A summary comes from a step a member worked on: an agent step, anywhere
                    // before this one in run order (a path a run did not take hands on nothing).
                    let all = self.all();
                    let here = all.iter().position(|s| s.id == step.id).unwrap_or(0);
                    let earlier = |id: &str| {
                        all[..here]
                            .iter()
                            .any(|s| s.id == id && matches!(s.kind, StepKind::Agent { .. }))
                    };
                    if let Some(id) = reads.iter().find(|id| !earlier(id)) {
                        return invalid(format!(
                            "step \"{name}\": it can read only an agent step before it, not {id:?}"
                        ));
                    }
                }
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
            StepKind::Action { action, unattended } => {
                self.validate_action(step, action)?;
                // What goes out is the person's to see first, unless they said otherwise.
                let sends = matches!(action, Action::Http { .. });
                if sends && !unattended && !self.checked_before(&step.id) {
                    return invalid(format!(
                        "step \"{name}\": an action that sends something needs a check before it \
                         on every path, or \"Runs without my check\" turned on"
                    ));
                }
            }
            StepKind::Wait { wait } => match wait {
                Wait::Minutes { minutes } => {
                    if *minutes == 0 || *minutes > MAX_WAIT_MINUTES {
                        return invalid(format!(
                            "step \"{name}\": it waits 1 to {MAX_WAIT_MINUTES} minutes"
                        ));
                    }
                }
                Wait::Time { at, .. } => {
                    if time_of_day(at).is_none() {
                        return invalid(format!(
                            "step \"{name}\": the time must be written HH:MM, as in 09:00"
                        ));
                    }
                }
                Wait::Input { input } => {
                    let found = self.inputs.iter().find(|f| f.name == *input);
                    if !found.is_some_and(|f| f.kind == InputKind::DateTime) {
                        return invalid(format!(
                            "step \"{name}\": it waits until an input that holds a date and \
                             time, and {input:?} is not one"
                        ));
                    }
                }
            },
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

    /// The rules of the inputs a start is asked for.
    fn validate_inputs(&self) -> Result<()> {
        if self.inputs.len() > MAX_INPUTS {
            return invalid(format!("a workflow asks for at most {MAX_INPUTS} inputs"));
        }
        let mut seen = HashSet::new();
        for field in &self.inputs {
            if !step_template::name_ok(&field.name) {
                return invalid(format!(
                    "an input's name must be 1 to 40 of a-z, 0-9, '-' and '_': {:?}",
                    field.name
                ));
            }
            if !seen.insert(field.name.as_str()) {
                return invalid(format!("two inputs have the name \"{}\"", field.name));
            }
            let label = field.label.trim();
            if label.is_empty() || label.chars().count() > MAX_NAME {
                return invalid(format!(
                    "the input \"{}\": a label must be 1 to {MAX_NAME} characters",
                    field.name
                ));
            }
            if let Some(default) = field.default.as_deref().filter(|d| !d.trim().is_empty()) {
                if too_long(default, MAX_TASK) {
                    return invalid(format!(
                        "the input \"{}\": its default is longer than 8,000 characters",
                        field.name
                    ));
                }
                if field.kind == InputKind::DateTime && date_time_of(default).is_none() {
                    return invalid(format!(
                        "the input \"{}\": its default must be a date and time, as in \
                         2026-12-01T09:00",
                        field.name
                    ));
                }
            }
        }
        Ok(())
    }

    /// The agent steps that come before the step `id` in run order, the ones inside an earlier
    /// branch's paths included: the steps whose outputs and summaries it may use.
    fn agents_earlier(&self, id: &str) -> Vec<&Step> {
        let all = self.all();
        let here = all.iter().position(|s| s.id == id).unwrap_or(0);
        all[..here]
            .iter()
            .copied()
            .filter(|s| matches!(s.kind, StepKind::Agent { .. }))
            .collect()
    }

    /// The rules of one text of the action step `step`: its marks name what the workflow has.
    /// `secrets`: whether the text may hold a secret (what is sent may; what is written to a
    /// file may not).
    fn validate_text(&self, step: &Step, what: &str, text: &str, secrets: bool) -> Result<()> {
        let name = step.name.trim();
        let refuse = |why: String| invalid(format!("step \"{name}\": {what}: {why}"));
        let marks = match step_template::marks(text) {
            Ok(marks) => marks,
            Err(why) => return refuse(why),
        };
        let earlier = self.agents_earlier(&step.id);
        for mark in marks {
            let written = format!("{{{{{}}}}}", mark.written());
            match &mark {
                Mark::Task => {}
                Mark::Input(input) => {
                    if !self.inputs.iter().any(|f| f.name == *input) {
                        return refuse(format!("{written} names no input of the workflow"));
                    }
                }
                Mark::Secret(_) => {
                    if !secrets {
                        return refuse(format!("{written}: a secret cannot be written to a file"));
                    }
                }
                Mark::Summary { step: from } => {
                    if !earlier.iter().any(|s| s.id == *from) {
                        return refuse(format!("{written} names no agent step before it"));
                    }
                }
                Mark::Output {
                    step: from,
                    name: output,
                } => {
                    let gives = earlier.iter().any(|s| {
                        s.id == *from
                            && matches!(
                                &s.kind,
                                StepKind::Agent { produces, .. } if produces.contains(output)
                            )
                    });
                    if !gives {
                        return refuse(format!(
                            "{written} names no output of an agent step before it"
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    /// The rules of an action step's own fields.
    fn validate_action(&self, step: &Step, action: &Action) -> Result<()> {
        let name = step.name.trim();
        match action {
            Action::Http {
                url, headers, body, ..
            } => {
                let url = url.trim();
                let rest = url
                    .strip_prefix("https://")
                    .or_else(|| url.strip_prefix("http://"));
                let Some(rest) = rest.filter(|r| !r.is_empty()) else {
                    return invalid(format!(
                        "step \"{name}\": the URL must start with https:// or http://"
                    ));
                };
                if url.chars().count() > MAX_URL {
                    return invalid(format!(
                        "step \"{name}\": the URL is longer than {MAX_URL} characters"
                    ));
                }
                // Where it goes is written in the workflow, never filled in by a step.
                let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
                if host.is_empty() || host.contains("{{") || host.contains("}}") {
                    return invalid(format!(
                        "step \"{name}\": the URL's host must be written out, with no mark in it"
                    ));
                }
                self.validate_text(step, "the URL", url, true)?;
                if headers.len() > MAX_HEADERS {
                    return invalid(format!(
                        "step \"{name}\": an action sends at most {MAX_HEADERS} headers"
                    ));
                }
                for header in headers {
                    let h = header.name.trim();
                    let token = !h.is_empty()
                        && h.bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
                    if !token {
                        return invalid(format!(
                            "step \"{name}\": a header's name is letters, digits, '-' and '_': \
                             {h:?}"
                        ));
                    }
                    if too_long(&header.value, MAX_HEADER_VALUE) {
                        return invalid(format!(
                            "step \"{name}\": the header {h} is longer than {MAX_HEADER_VALUE} \
                             characters"
                        ));
                    }
                    self.validate_text(step, &format!("the header {h}"), &header.value, true)?;
                }
                if too_long(body, MAX_BODY) {
                    return invalid(format!(
                        "step \"{name}\": the body is longer than {MAX_BODY} characters"
                    ));
                }
                self.validate_text(step, "the body", body, true)
            }
            Action::File { path, content } => {
                let p = path.trim();
                if p.is_empty() || p.chars().count() > MAX_FILE_PATH {
                    return invalid(format!(
                        "step \"{name}\": the file's path must be 1 to {MAX_FILE_PATH} characters"
                    ));
                }
                if !relative_path_ok(p) {
                    return invalid(format!(
                        "step \"{name}\": the file's path must stay inside the project's folder"
                    ));
                }
                self.validate_text(step, "the file's path", p, false)?;
                if content.chars().count() > MAX_FILE_CONTENT {
                    return invalid(format!(
                        "step \"{name}\": the file's content is longer than \
                         {MAX_FILE_CONTENT} characters"
                    ));
                }
                self.validate_text(step, "the file's content", content, false)
            }
        }
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

    /// The options of the agent step `id`; `None` for a step of another kind or an unknown id.
    pub fn options(&self, id: &str) -> Option<&StepOptions> {
        match &self.step(id)?.kind {
            StepKind::Agent { options, .. } => Some(options),
            _ => None,
        }
    }

    /// Whether the member `dot_id` may start this workflow from a chat.
    pub fn started_from_chat_by(&self, dot_id: &str) -> bool {
        matches!(&self.trigger, Trigger::Chat { starters } if starters.iter().any(|s| s == dot_id))
    }
}
