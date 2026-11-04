//! The workflow tools (teams and workflows spec section 7), on both engines as the check-in
//! tools are: built in on the local engine, tools of Dots' own `dots` MCP server on the Claude
//! engine. All of them go under the one policy name `Workflow`. This module holds their names,
//! descriptions and arguments, and which of them a run has; `Workflows` performs a call.

use serde_json::{json, Value};

use crate::model::{EngineKind, Run, StartableWorkflow, StepRunStatus};
use crate::store::Store;
use crate::workflow::{Definition, Step, StepKind};
use crate::Result;

/// The policy name of every workflow tool.
pub const POLICY_NAME: &str = "Workflow";

const START_DESCRIPTION: &str = "Start one of the team workflows listed under \"Workflows you \
    can start\" when the person asks for work it covers. Give the workflow's name, the project's \
    name and the task: what the person wants, written so the team can act on it without asking \
    again. Tell the person it started; its progress shows in this chat.";

const START_MEMBER_DESCRIPTION: &str = "Start another member of the team on part of this \
    step's work: one of the members listed under \"Workflow\" as those you may start. Give the \
    member's name and the task: what they should do, written so they can act on it without \
    asking again. They work on their own; you are told when they finish.";

const REPORT_DESCRIPTION: &str = "Report how this step of the workflow came out: one of the \
    outcomes listed under \"Workflow\". The workflow goes on by the outcome you report; if you \
    report again, the last one counts. Report it before you end with your summary.";

const SEND_BACK_DESCRIPTION: &str = "Send the work back to an earlier step of the workflow, \
    one of those listed under \"Workflow\", when it must be redone there. Give the step's name \
    and a note that says what to change. Then end your turn: the work goes back when you \
    finish. If you send back again, the last one counts.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tool {
    StartWorkflow,
    /// A delegate step's run starts a member (section 5.3).
    StartMember,
    /// An agent step's run reports its outcome, for a branch on outcome after it.
    ReportOutcome,
    /// An agent step's run asks for its work to go back to an earlier step (section 5.4).
    SendBack,
}

impl Tool {
    pub const ALL: [Tool; 4] = [
        Tool::StartWorkflow,
        Tool::StartMember,
        Tool::ReportOutcome,
        Tool::SendBack,
    ];

    /// The name a model calls the tool by on the local engine, and the `dots` server's tool.
    pub fn name(self) -> &'static str {
        match self {
            Tool::StartWorkflow => "start_workflow",
            Tool::StartMember => "start_member",
            Tool::ReportOutcome => "report_outcome",
            Tool::SendBack => "send_back",
        }
    }

    pub fn from_name(name: &str) -> Option<Tool> {
        Tool::ALL.into_iter().find(|t| t.name() == name)
    }

    /// The tool a `Workflow` call with `input` is for, by its arguments: each of the others has
    /// one that `start_workflow` lacks.
    pub fn for_input(input: &Value) -> Tool {
        if input.get("outcome").is_some() {
            Tool::ReportOutcome
        } else if input.get("to_step").is_some() {
            Tool::SendBack
        } else if input.get("member").is_some() {
            Tool::StartMember
        } else {
            Tool::StartWorkflow
        }
    }

    /// The name the run's model knows the tool by.
    pub fn model_name(self, engine: EngineKind) -> &'static str {
        match (engine, self) {
            (EngineKind::Claude, Tool::StartWorkflow) => "mcp__dots__start_workflow",
            (EngineKind::Claude, Tool::StartMember) => "mcp__dots__start_member",
            (EngineKind::Claude, Tool::ReportOutcome) => "mcp__dots__report_outcome",
            (EngineKind::Claude, Tool::SendBack) => "mcp__dots__send_back",
            (EngineKind::Local, tool) => tool.name(),
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Tool::StartWorkflow => START_DESCRIPTION,
            Tool::StartMember => START_MEMBER_DESCRIPTION,
            Tool::ReportOutcome => REPORT_DESCRIPTION,
            Tool::SendBack => SEND_BACK_DESCRIPTION,
        }
    }

    /// The tool's arguments, as JSON Schema properties.
    pub fn properties(self) -> Value {
        let s = |d: &str| json!({"type": "string", "description": d});
        match self {
            Tool::StartWorkflow => json!({
                "workflow": s("The workflow's name, as listed; \"<team>/<workflow>\" when two teams have one of that name"),
                "project": s("The project's name, as listed for that workflow"),
                "task": s("The task, up to 8,000 characters")
            }),
            Tool::StartMember => json!({
                "member": s("The member's name, as listed"),
                "task": s("The task, up to 8,000 characters")
            }),
            Tool::ReportOutcome => json!({
                "outcome": s("The outcome, as listed")
            }),
            Tool::SendBack => json!({
                "to_step": s("The name of the step to send the work back to, as listed"),
                "note": s("What to change, up to 8,000 characters")
            }),
        }
    }

    pub fn required(self) -> &'static [&'static str] {
        match self {
            Tool::StartWorkflow => &["workflow", "project", "task"],
            Tool::StartMember => &["member", "task"],
            Tool::ReportOutcome => &["outcome"],
            Tool::SendBack => &["to_step", "note"],
        }
    }

    /// The tool as Dots' own `dots` MCP server lists it.
    pub fn mcp_schema(self) -> Value {
        json!({
            "name": self.name(),
            "description": self.description(),
            "inputSchema": {
                "type": "object",
                "properties": self.properties(),
                "required": self.required()
            }
        })
    }
}

/// The prompt section that lists the workflows a chat run may start, naming the tool as the
/// run's model knows it; empty when there are none.
pub fn prompt_section(startable: &[StartableWorkflow], engine: EngineKind) -> String {
    if startable.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for w in startable {
        out.push_str(&format!(
            "- \"{}\" of the team \"{}\": projects {}\n",
            w.workflow,
            w.team,
            w.projects
                .iter()
                .map(|p| format!("\"{p}\""))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    out.push_str(&format!(
        "\nStart one with `{}` when the person asks for work it covers.\n",
        Tool::StartWorkflow.model_name(engine)
    ));
    out
}

/// An earlier step a step may send its work back to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendBackTarget {
    pub step_id: String,
    pub name: String,
}

/// A member a delegate step may start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartableMember {
    pub dot_id: String,
    pub name: String,
}

/// What a delegate step's run may start (section 5.3): the members, and how many at once and
/// in all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelegateTools {
    /// In the definition's order.
    pub may_start: Vec<StartableMember>,
    pub at_once: u32,
    pub total: u32,
}

/// The workflow tools of a run that works for an open agent step (section 7): what the step
/// declares, read from the workflow run's definition when the run starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepTools {
    pub step_run_id: String,
    pub step_id: String,
    /// The outcome labels it may report, trimmed, in the definition's order.
    pub outcomes: Vec<String>,
    /// The steps it may send its work back to, in the definition's order.
    pub send_back_to: Vec<SendBackTarget>,
    /// For a delegate step, the members it may start.
    pub delegate: Option<DelegateTools>,
}

impl StepTools {
    /// The tools of the step run `step_run_id` of `step`, an agent step of `definition`; `None`
    /// for a step of another kind. The members a delegate step may start are named by their
    /// dot ids here: `read` names them as the store has them.
    pub fn of(step_run_id: &str, step: &Step, definition: &Definition) -> Option<StepTools> {
        let StepKind::Agent {
            can_send_back_to,
            outcomes,
            delegate,
            ..
        } = &step.kind
        else {
            return None;
        };
        Some(StepTools {
            step_run_id: step_run_id.to_string(),
            step_id: step.id.clone(),
            outcomes: outcomes.iter().map(|o| o.trim().to_string()).collect(),
            // Validation keeps only earlier agent steps here; a definition that names a step it
            // no longer has gives that step no target.
            send_back_to: can_send_back_to
                .iter()
                .filter_map(|id| definition.step(id))
                .map(|s| SendBackTarget {
                    step_id: s.id.clone(),
                    name: s.name.clone(),
                })
                .collect(),
            delegate: delegate.as_ref().map(|d| DelegateTools {
                may_start: d
                    .may_start
                    .iter()
                    .map(|id| StartableMember {
                        dot_id: id.clone(),
                        name: id.clone(),
                    })
                    .collect(),
                at_once: d.at_once,
                total: d.total,
            }),
        })
    }

    /// `of`, with the members a delegate step may start named as their dots are now. A member
    /// that is no longer a dot is left out: it could not be started.
    pub async fn read(
        store: &Store,
        step_run_id: &str,
        step: &Step,
        definition: &Definition,
    ) -> Result<Option<StepTools>> {
        let Some(mut tools) = StepTools::of(step_run_id, step, definition) else {
            return Ok(None);
        };
        if let Some(delegate) = &mut tools.delegate {
            let mut named = Vec::new();
            for member in &delegate.may_start {
                match store.get_dot(&member.dot_id).await {
                    Ok(dot) => named.push(StartableMember {
                        dot_id: dot.id,
                        name: dot.spec.name,
                    }),
                    Err(crate::Error::NotFound(_)) => {}
                    Err(e) => return Err(e),
                }
            }
            delegate.may_start = named;
        }
        Ok(Some(tools))
    }

    /// Whether the run has `tool`: `report_outcome` with outcomes to report, `send_back` with
    /// steps to send back to, `start_member` with members to start. Never `start_workflow`,
    /// which is a chat's tool.
    pub fn offers(&self, tool: Tool) -> bool {
        match tool {
            Tool::ReportOutcome => !self.outcomes.is_empty(),
            Tool::SendBack => !self.send_back_to.is_empty(),
            Tool::StartMember => self
                .delegate
                .as_ref()
                .is_some_and(|d| !d.may_start.is_empty()),
            Tool::StartWorkflow => false,
        }
    }

    /// The lines of the `## Workflow` section that name the run's tools as its model knows
    /// them, with the members it may start and its limits, the outcomes and the steps it may
    /// send back to; empty without any.
    pub fn prompt_lines(&self, engine: EngineKind) -> String {
        let mut out = String::new();
        if let Some(d) = self
            .delegate
            .as_ref()
            .filter(|_| self.offers(Tool::StartMember))
        {
            out.push_str(&format!(
                "- Start other members of the team on parts of the work with `{}`, one of: {}. \
                 At most {} may run at once, and {} in all. End your turn once you have started \
                 them: you are told when they finish.\n",
                Tool::StartMember.model_name(engine),
                quoted(d.may_start.iter().map(|m| m.name.as_str())),
                d.at_once,
                d.total
            ));
        }
        if self.offers(Tool::ReportOutcome) {
            out.push_str(&format!(
                "- Report how the step came out with `{}`, one of: {}. The workflow goes on by \
                 the outcome you report.\n",
                Tool::ReportOutcome.model_name(engine),
                quoted(self.outcomes.iter().map(String::as_str))
            ));
        }
        if self.offers(Tool::SendBack) {
            out.push_str(&format!(
                "- If the work must be redone in an earlier step, send it back with `{}` to one \
                 of: {}, with a note that says what to change.\n",
                Tool::SendBack.model_name(engine),
                quoted(self.send_back_to.iter().map(|t| t.name.as_str()))
            ));
        }
        out
    }
}

/// `names` in double quotes, separated by commas: `"a", "b"`.
pub fn quoted<'a>(names: impl Iterator<Item = &'a str>) -> String {
    names
        .map(|n| format!("\"{n}\""))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The workflow tools of `run`: those of the agent step it works for while that step is open
/// (its step run `running` or `waiting`). A run in the thread of a member a delegate step
/// started (its root run is a member's) gets none; nor does any other run.
pub async fn for_run(store: &Store, run: &Run) -> Result<Option<StepTools>> {
    let Some(step_run_id) = &run.workflow_step_run_id else {
        return Ok(None);
    };
    if store.step_member(&run.root_run_id).await?.is_some() {
        return Ok(None);
    }
    let step_run = store.get_step_run(step_run_id).await?;
    if !matches!(
        step_run.status,
        StepRunStatus::Running | StepRunStatus::Waiting
    ) {
        return Ok(None);
    }
    let wr = store.get_workflow_run(&step_run.workflow_run_id).await?;
    match wr.definition.step(&step_run.step_id) {
        Some(step) => StepTools::read(store, &step_run.id, step, &wr.definition).await,
        None => Ok(None),
    }
}
