//! The tools for making dots (dots-make-dots design sections 5 and 7 to 10), on both engines as
//! the workflow tools are: tools of Dots' own `dots` MCP server on the Claude engine, built in
//! on the local engine. All of them go under the one policy name `MakeDots`. This module holds
//! their names, descriptions and arguments, which runs have them, and how a call that the
//! policy let through reaches `Makers`.

use serde::Deserialize;
use serde_json::{json, Value};

use super::jobs::MAX_JOBS_BETWEEN_TYPED;
use super::manage::{made_dots_text, ChangeDot};
use super::reach::{MAKERS_PERMISSIONS, NO_SCHEDULE};
use super::service::{may_make, MakeDot, Makers};
use crate::model::{Dot, EngineKind, Making, Run};
use crate::policy::Preset;
use crate::{Error, Result};

/// The policy name of every tool for making dots.
pub const POLICY_NAME: &str = crate::policy::MAKE_DOTS_TOOL;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tool {
    MakeDot,
    ChangeDot,
    DeleteDot,
    GiveJob,
    ListMadeDots,
}

impl Tool {
    pub const ALL: [Tool; 5] = [
        Tool::MakeDot,
        Tool::ChangeDot,
        Tool::DeleteDot,
        Tool::GiveJob,
        Tool::ListMadeDots,
    ];

    /// The name a model calls the tool by on the local engine, and the `dots` server's tool.
    pub fn name(self) -> &'static str {
        match self {
            Tool::MakeDot => "make_dot",
            Tool::ChangeDot => "change_dot",
            Tool::DeleteDot => "delete_dot",
            Tool::GiveJob => "give_job",
            Tool::ListMadeDots => "list_made_dots",
        }
    }

    pub fn from_name(name: &str) -> Option<Tool> {
        Tool::ALL.into_iter().find(|t| t.name() == name)
    }

    /// The tool a `MakeDots` call is for, known only by what the policy checked
    /// (`policy_input`): an approval's input names it.
    pub fn for_input(checked: &Value) -> Option<Tool> {
        checked
            .get("tool")
            .and_then(Value::as_str)
            .and_then(Tool::from_name)
    }

    /// The name the run's model knows the tool by.
    pub fn model_name(self, engine: EngineKind) -> &'static str {
        match (engine, self) {
            (EngineKind::Claude, Tool::MakeDot) => "mcp__dots__make_dot",
            (EngineKind::Claude, Tool::ChangeDot) => "mcp__dots__change_dot",
            (EngineKind::Claude, Tool::DeleteDot) => "mcp__dots__delete_dot",
            (EngineKind::Claude, Tool::GiveJob) => "mcp__dots__give_job",
            (EngineKind::Claude, Tool::ListMadeDots) => "mcp__dots__list_made_dots",
            (EngineKind::Local, tool) => tool.name(),
        }
    }

    /// What the tool is for, in words a model can act on. `limits` are the maker's own
    /// (design section 10), so `make_dot` names the numbers the person set.
    pub fn description(self, limits: &Making) -> String {
        match self {
            Tool::MakeDot => format!(
                "Make a new dot that works for you. Look at list_made_dots first and reuse a \
                 dot before making another. Give it only what its work needs. A dot within your \
                 reach is made at once and can run at once: your folder or one below it (or no \
                 folder, for a new one of its own), your permissions or a weaker preset, Library \
                 items and variables you have yourself, limits no higher than yours. If you ask \
                 for more than you have, or pass your limit of dots ({} in one run, {} kept), \
                 nothing is made yet: the request waits for the person. Do not wait for it: \
                 carry on or end your turn, and you are woken with their answer. The new dot \
                 cannot make dots itself.",
                limits.per_run, limits.total
            ),
            Tool::ChangeDot => "Change a dot you made. Only the settings you name change; the \
                 rest stay as they are. What you name is checked against what you have now: \
                 giving the dot more than you have waits for the person, as with make_dot, and \
                 taking something away never asks."
                .to_string(),
            Tool::DeleteDot => "Delete a dot you made, with its whole history. It cannot be \
                 undone. Refused while the dot has a run going or queued."
                .to_string(),
            Tool::GiveJob => format!(
                "Give a job to a dot you made: a message that starts a run of it. It returns \
                 at once. Do not wait for the job: carry on or end your turn, and you are woken \
                 in this conversation with how the job ended and the dot's final text. At most \
                 {MAX_JOBS_BETWEEN_TYPED} jobs between two messages of the person. A job for a dot \
                 that has more than you have waits for the person's say-so first."
            ),
            Tool::ListMadeDots => "List the dots you made: each with its purpose, its schedule, \
                 whether it is running and how its last run ended, and your requests that wait \
                 for the person."
                .to_string(),
        }
    }

    /// The tool's arguments, as JSON Schema properties: for `make_dot` and `change_dot`, the
    /// fields of `MakeDot` and `ChangeDot`, one for one.
    pub fn properties(self) -> Value {
        let s = |d: &str| json!({"type": "string", "description": d});
        match self {
            Tool::MakeDot => {
                let mut properties = wanted_properties(false);
                properties.insert(
                    "purpose".into(),
                    s("One line on what the dot is for, up to 200 characters; the person sees it"),
                );
                properties.insert(
                    "first_job".into(),
                    s(
                        "A first job for the new dot, up to 4,000 characters: it starts when the \
                       dot is made, and you are woken with its result as with give_job",
                    ),
                );
                Value::Object(properties)
            }
            Tool::ChangeDot => {
                let mut properties = wanted_properties(true);
                properties.insert("dot".into(), s("The made dot's name, as it is now"));
                properties.insert(
                    "purpose".into(),
                    s("A new one-line purpose, up to 200 characters"),
                );
                Value::Object(properties)
            }
            Tool::DeleteDot => json!({ "dot": s("The made dot's name") }),
            Tool::GiveJob => json!({
                "dot": s("The made dot's name"),
                "message": s("The job, 1 to 4,000 characters: what to do, written so the dot \
                              can act on it without asking again")
            }),
            Tool::ListMadeDots => json!({}),
        }
    }

    pub fn required(self) -> &'static [&'static str] {
        match self {
            Tool::MakeDot => &["name", "instructions", "purpose"],
            Tool::ChangeDot | Tool::DeleteDot => &["dot"],
            Tool::GiveJob => &["dot", "message"],
            Tool::ListMadeDots => &[],
        }
    }

    /// The tool as Dots' own `dots` MCP server lists it for a maker with `limits`.
    pub fn mcp_schema(self, limits: &Making) -> Value {
        json!({
            "name": self.name(),
            "description": self.description(limits),
            "inputSchema": {
                "type": "object",
                "properties": self.properties(),
                "required": self.required(),
                "additionalProperties": false
            }
        })
    }
}

/// Whether `run`, a run of `dot`, is offered the tools (design section 5): exactly where
/// `may_make` would not refuse it, so a model is never shown a tool that always refuses. What
/// the dot's policy says of `MakeDots` is the caller's check.
pub fn offered(dot: &Dot, run: &Run) -> bool {
    may_make(dot, run).is_ok()
}

/// What the policy checks of a call of `tool` with `arguments`, under `MakeDots`: the tool by
/// name and its arguments, as an `App` call names its app and tool. An approval then shows
/// which of the five tools waits, and a grant for one of them is not a grant for another.
pub fn policy_input(tool: Tool, arguments: &Value) -> Value {
    json!({ "tool": tool.name(), "input": arguments })
}

/// The made dot a `MakeDots` call would delete, when what the policy checked is a
/// `delete_dot`: the send-and-delete guard asks before it (`policy::guard::classify`).
pub fn deletes(checked: &Value) -> Option<&str> {
    if Tool::for_input(checked) != Some(Tool::DeleteDot) {
        return None;
    }
    Some(
        checked
            .pointer("/input/dot")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    )
}

/// The arguments of `delete_dot`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Named {
    dot: String,
}

/// The arguments of `give_job`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Job {
    dot: String,
    message: String,
}

/// The arguments of `list_made_dots`: none.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Nothing {}

/// A call's arguments, read.
enum Call {
    Make(MakeDot),
    Change(ChangeDot),
    Delete(Named),
    Job(Job),
    List,
}

/// Reads the arguments of a call of `tool`. `Err` is the sentence the model gets: an argument
/// the tool does not have (design section 7.1: a call can not name "Can make dots", the
/// account or the engine), one that is missing, or one of the wrong kind.
fn read(tool: Tool, arguments: &Value) -> std::result::Result<Call, String> {
    let arguments = arguments.clone();
    let read = match tool {
        Tool::MakeDot => serde_json::from_value(arguments).map(Call::Make),
        Tool::ChangeDot => serde_json::from_value(arguments).map(Call::Change),
        Tool::DeleteDot => serde_json::from_value(arguments).map(Call::Delete),
        Tool::GiveJob => serde_json::from_value(arguments).map(Call::Job),
        Tool::ListMadeDots => serde_json::from_value::<Nothing>(arguments).map(|_| Call::List),
    };
    read.map_err(|e| {
        format!(
            "{} cannot read its arguments: {e}. Send only the arguments the tool lists.",
            tool.name()
        )
    })
}

/// Refuses arguments `call` could not read, before the policy is asked: the person is never
/// asked to approve a call that cannot be performed as written.
pub fn check_arguments(tool: Tool, arguments: &Value) -> std::result::Result<(), String> {
    read(tool, arguments).map(|_| ())
}

/// A service error as the sentence the model gets.
fn sentence(tool: Tool, e: Error) -> String {
    match e {
        Error::Invalid(m) | Error::Conflict(m) | Error::NotFound(m) | Error::NotAllowed(m) => m,
        other => format!("{} failed: {other}", tool.name()),
    }
}

/// Performs the call the policy let through, for the run `run_id`: `checked` is what
/// `policy_input` made, as the policy or the person's approval returned it. `Ok` is the
/// service's text for the model; `Err` the sentence of what it refused, for a tool result that
/// is an error. The same on both engines.
pub async fn call(
    makers: &Makers,
    run_id: &str,
    checked: &Value,
) -> std::result::Result<String, String> {
    let Some(tool) = Tool::for_input(checked) else {
        return Err("this is not a tool for making dots".to_string());
    };
    let arguments = checked.get("input").cloned().unwrap_or_else(|| json!({}));
    let done = match read(tool, &arguments)? {
        Call::Make(ask) => makers
            .make_dot(run_id, ask)
            .await
            .map(|outcome| outcome.tool_text()),
        Call::Change(ask) => makers
            .change_dot(run_id, ask)
            .await
            .map(|outcome| outcome.tool_text()),
        Call::Delete(named) => makers.delete_dot(run_id, &named.dot).await,
        Call::Job(job) => makers.give_job(run_id, &job.dot, &job.message).await,
        Call::List => makers.made_dots_tool_text(run_id).await,
    };
    done.map_err(|e| sentence(tool, e))
}

impl Makers {
    /// What `list_made_dots` tells the run `run_id` (design section 9): `made_dots_text` of
    /// the maker's made dots and of its requests for a new dot that wait for the person.
    pub async fn made_dots_tool_text(&self, run_id: &str) -> Result<String> {
        let lines = self.list_made_dots(run_id).await?;
        let store = &self.parts.store;
        let run = store.get_run(run_id).await?;
        let maker = store.get_dot(&run.dot_id).await?;
        // One marked gone still stands before the person, so it still waits.
        let waiting = store.list_make_requests(Some(&maker.id), true).await?;
        Ok(made_dots_text(&maker.spec.name, &lines, &waiting))
    }
}

/// The fields of `Wanted` as JSON Schema properties, one for one, worded for a new dot or,
/// with `change`, for a change to a made dot (design sections 6, 7.1 and 9).
fn wanted_properties(change: bool) -> serde_json::Map<String, Value> {
    let s = |d: &str| json!({"type": "string", "description": d});
    let names = |d: &str| json!({"type": "array", "items": {"type": "string"}, "description": d});
    // What a field left out means.
    let left_out = |new: &'static str| if change { "" } else { new };
    let permissions = [
        MAKERS_PERMISSIONS,
        Preset::ReadOnly.as_str(),
        Preset::Sandboxed.as_str(),
        Preset::Trusted.as_str(),
    ];
    let properties = json!({
        "name": s(if change {
            "A new name for the dot: 1 to 64 letters, digits, spaces, '-', '_' or '.', that no \
             other dot has"
        } else {
            "The dot's name: 1 to 64 letters, digits, spaces, '-', '_' or '.', that no other \
             dot has"
        }),
        "instructions": s("The dot's whole instructions: what it does on every run, written \
                           for it"),
        "model": s(&format!(
            "A model of your own engine.{}",
            left_out(" Yours when left out.")
        )),
        "folder": s(&format!(
            "The folder it works in: your own folder, or one below it (a relative path starts \
             at your folder; Dots makes it if it is missing). Any other folder waits for the \
             person.{}",
            left_out(" Left out, it gets a new empty folder of its own.")
        )),
        "permissions": {
            "type": "string",
            "enum": permissions,
            "description": format!(
                "\"maker\" is an exact copy of your own permissions. The others are plain \
                 presets, weakest first; one stronger than yours waits for the person.{}",
                left_out(" \"maker\" when left out.")
            )
        },
        "uses": {
            "type": "object",
            "description": format!(
                "The whole set of Library items it uses, by name. An item you do not use \
                 yourself waits for the person.{}",
                left_out(" None when left out.")
            ),
            "properties": {
                "mcp_servers": names("MCP servers"),
                "skills": names("Skills"),
                "plugins": names("Plugins"),
                "context": names("Context collections"),
                "apps": names("Apps")
            },
            "additionalProperties": false
        },
        "variables": {
            "type": "object",
            "description": format!(
                "The whole set of variables it is granted, as names, never values: \
                 {{\"shell\": [\"NAME\"], \"mcp:<server>\": [\"NAME\"]}}. A name you are not \
                 granted yourself for that consumer waits for the person.{}",
                left_out(" None when left out.")
            ),
            "additionalProperties": {"type": "array", "items": {"type": "string"}}
        },
        "browser": {
            "type": "boolean",
            "description": "Whether it has the built-in browser. Within reach only if you \
                            have it."
        },
        "use_user_settings": {
            "type": "boolean",
            "description": "Whether it uses the person's own Claude settings. Within reach \
                            only if you do."
        },
        "max_turns": {
            "type": "integer",
            "description": format!(
                "Its turn limit. One above yours waits for the person.{}",
                left_out(" Yours when left out.")
            )
        },
        "timeout_secs": {
            "type": "integer",
            "description": format!(
                "Its time limit for one run, in seconds. One above yours waits for the \
                 person.{}",
                left_out(" Yours when left out.")
            )
        },
        "schedule": {
            "description": format!(
                "When it runs by itself: a schedule, or \"{NO_SCHEDULE}\" for none.{} Any \
                 valid schedule is within reach. Times are local: \"HH:MM\", and \
                 \"YYYY-MM-DDTHH:MM\" for a date.",
                if change { " \"none\" removes its schedule." } else { " None when left out." }
            ),
            "anyOf": [
                {"type": "string", "enum": [NO_SCHEDULE]},
                {
                    "type": "object",
                    "properties": {
                        "rule": {
                            "type": "object",
                            "description": "One of {\"kind\": \"daily\", \"time\": \"09:00\"}, \
                                {\"kind\": \"weekly\", \"days\": [\"mon\", \"thu\"], \"time\": \
                                \"09:00\"}, {\"kind\": \"every\", \"minutes\": 120}, {\"kind\": \
                                \"once\", \"at\": \"2026-11-20T09:00\"}, {\"kind\": \"cron\", \
                                \"expr\": \"0 0 9 * * *\"}"
                        },
                        "catch_up": {
                            "type": "boolean",
                            "description": "One run for the most recent missed time when Dots \
                                            comes back; true when left out"
                        },
                        "until": {
                            "type": "object",
                            "description": "Its end, if it has one: {\"count\": 10} or \
                                            {\"date\": \"2026-12-01T09:00\"}"
                        }
                    },
                    "required": ["rule"]
                }
            ]
        }
    });
    match properties {
        Value::Object(properties) => properties,
        _ => unreachable!("the properties are an object"),
    }
}
