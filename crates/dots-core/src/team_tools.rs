//! The workflow tools (teams and workflows spec section 7), on both engines as the check-in
//! tools are: built in on the local engine, tools of Dots' own `dots` MCP server on the Claude
//! engine. All of them go under the one policy name `Workflow`. This module holds their names,
//! descriptions and arguments; `Workflows` performs a call.

use serde_json::{json, Value};

use crate::model::{EngineKind, StartableWorkflow};

/// The policy name of every workflow tool.
pub const POLICY_NAME: &str = "Workflow";

const START_DESCRIPTION: &str = "Start one of the team workflows listed under \"Workflows you \
    can start\" when the person asks for work it covers. Give the workflow's name, the project's \
    name and the task: what the person wants, written so the team can act on it without asking \
    again. Tell the person it started; its progress shows in this chat.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tool {
    StartWorkflow,
}

impl Tool {
    pub const ALL: [Tool; 1] = [Tool::StartWorkflow];

    /// The name a model calls the tool by on the local engine, and the `dots` server's tool.
    pub fn name(self) -> &'static str {
        match self {
            Tool::StartWorkflow => "start_workflow",
        }
    }

    pub fn from_name(name: &str) -> Option<Tool> {
        Tool::ALL.into_iter().find(|t| t.name() == name)
    }

    /// The tool a `Workflow` call with `input` is for, by its arguments.
    pub fn for_input(_input: &Value) -> Tool {
        Tool::StartWorkflow
    }

    /// The name the run's model knows the tool by.
    pub fn model_name(self, engine: EngineKind) -> &'static str {
        match (engine, self) {
            (EngineKind::Claude, Tool::StartWorkflow) => "mcp__dots__start_workflow",
            (EngineKind::Local, tool) => tool.name(),
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Tool::StartWorkflow => START_DESCRIPTION,
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
        }
    }

    pub fn required(self) -> &'static [&'static str] {
        match self {
            Tool::StartWorkflow => &["workflow", "project", "task"],
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
