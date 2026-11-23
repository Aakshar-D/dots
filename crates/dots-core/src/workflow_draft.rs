//! Describe it, and Sage builds it (plain workflows spec section 2): the prompt that asks the
//! `claude` CLI to turn a person's description of a job into one workflow for a team, and the
//! reading of its answer into a definition the builder opens.
//!
//! The model names members as the person does, by name; the reading puts their ids in. It
//! writes no secret: where a step needs a key, it names a variable and says so in its notes,
//! and the person makes the variable. A draft that breaks a rule of the definition is sent
//! back once with the rule it broke; a second one that breaks a rule is refused with that
//! rule. Nothing is saved here: the person sees the draft and saves it.

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::engine::claude::once::{OnceFailure, OnceRunner, NO_CLI};
use crate::skill_creator::{unwrap_fence, DEFAULT_MODEL, MAX_REQUEST_CHARS};
use crate::workflow::{Budget, Definition};
use crate::{Error, Result};

/// The system prompt of a draft.
pub const DRAFT_INSTRUCTIONS: &str = r#"You design Sage workflows. A person describes a job they want done, and you turn it into exactly one workflow for their team. Answer with one JSON object and nothing else.

A workflow is a list of steps that run in order. Sols (AI agents, the team's members) do the thinking steps. Sage itself does the sending, saving and waiting steps, which cost nothing, so give a sol only the work that needs judgement.

The JSON object:
{
  "name": "a short name for the workflow",
  "inputs": [ {"name": "a_z_0_9", "label": "what to ask the person", "kind": "text" | "long_text" | "date_time", "required": true | false, "default": null} ],
  "trigger": one of
    {"kind": "manual"}                                   the person starts it and types the task
    {"kind": "schedule", "schedule": {"rule": {"kind": "daily", "time": "09:00"}, "catch_up": false, "until": null}, "task": "what each run works on", "project": "", "catch_up": true}
    {"kind": "schedule", "schedule": {"rule": {"kind": "weekly", "days": ["mon"], "time": "09:00"}, "catch_up": false, "until": null}, "task": "...", "project": "", "catch_up": true}
    {"kind": "telegram", "token": "BOT_TOKEN", "chats": [], "project": ""}   a message to a Telegram bot starts it; the message is the task
  "steps": [ step, ... ],
  "notes": [ "what the person still has to do before it can run", ... ]
}

Every step has "id" (a-z, 0-9, - and _, unique), "name" (a few words) and "kind":
- {"kind": "agent", "member": "<a member's name, exactly as listed>", "job": "what to do, written to the sol", "produces": ["post"], "options": {"model": "small" | "standard" | "large" | null, "lean": true, "reads": null}}
  "produces" names the texts the step hands on (a post, a subject, a reply). Leave it [] when it hands on nothing. Use "small" for simple writing or sorting, "standard" for most work, "large" only for hard work. Keep "lean": true unless the step needs the sol's memories.
- {"kind": "check"}   the person reads what the step before it produced, may change it, and approves
- {"kind": "wait", "wait": "minutes", "minutes": 30}
  {"kind": "wait", "wait": "time", "at": "09:00", "weekday": null | "mon" ... "sun"}
  {"kind": "wait", "wait": "input", "input": "<the name of a date_time input>"}
- {"kind": "action", "action": "telegram", "token": "BOT_TOKEN", "chat": "{{input.chat_id}}", "text": "{{<agent step id>.<output>}}", "unattended": false}
  {"kind": "action", "action": "http", "method": "post", "url": "https://...", "headers": [{"name": "Authorization", "value": "Bearer {{secret.NAME}}"}], "body": "...", "unattended": false}
  {"kind": "action", "action": "file", "path": "posts/{{input.name}}.md", "content": "...", "unattended": false}

In an action's texts, write {{task}} for the run's task, {{input.<name>}} for an input, {{<agent step id>.<output>}} for a text an earlier agent step produces, {{<agent step id>.summary}} for its summary, and {{secret.<NAME>}} for a key. Never write a key itself.

Rules:
- A telegram or http action needs a check step somewhere before it, unless the person clearly wants it to run with nobody looking; then set "unattended": true.
- A step can only use an output that an earlier agent step lists in "produces".
- A wait on an input needs a date_time input of that name in "inputs".
- A workflow a Telegram message starts should ask for an input named "chat_id" (text, not required) so a telegram action can answer with "chat": "{{input.chat_id}}".
- To send mail or post to a social network, use an http action to the service the person names, and say in "notes" which key to keep as which variable. If they name no service, say in "notes" that they must fill in the address.
- Use only the members listed. If the job needs a kind of sol the team lacks, use the closest one and say so in "notes".
- Keep it as short as the job allows: most workflows are two to six steps.
"#;

/// A member of the team a draft is for, as the model is told of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftMember {
    pub id: String,
    pub name: String,
    pub role: String,
}

/// What a draft is written for: the team's members and the names of its projects.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DraftContext {
    pub members: Vec<DraftMember>,
    pub projects: Vec<String>,
}

/// A workflow as drafted: its name, its definition, and what the person still has to do
/// before it can run (make a variable, name a chat, fill in an address).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowDraft {
    pub name: String,
    pub definition: Definition,
    pub notes: Vec<String>,
}

/// The request's text, sent on the CLI's stdin. `problem` is the rule the last draft broke,
/// when this is the second asking. `Invalid` for an empty description, one over
/// `MAX_REQUEST_CHARS`, and a team with no member.
pub fn draft_prompt(
    ctx: &DraftContext,
    description: &str,
    problem: Option<&str>,
) -> Result<String> {
    let description = description.trim();
    if description.is_empty() {
        return Err(Error::Invalid("Say what the workflow should do.".into()));
    }
    if description.chars().count() > MAX_REQUEST_CHARS {
        return Err(Error::Invalid(format!(
            "The description is longer than {MAX_REQUEST_CHARS} characters."
        )));
    }
    if ctx.members.is_empty() {
        return Err(Error::Invalid(
            "Add a member to the team first: a workflow's steps need one.".into(),
        ));
    }
    let mut out = String::from("The team's members:\n");
    for member in &ctx.members {
        let role = member.role.trim();
        match role.is_empty() {
            true => out.push_str(&format!("- {}\n", member.name.trim())),
            false => out.push_str(&format!("- {} ({role})\n", member.name.trim())),
        }
    }
    if !ctx.projects.is_empty() {
        out.push_str("\nThe team's projects, by name (\"project\": \"\" is a plain folder Sage keeps for the team):\n");
        for project in &ctx.projects {
            out.push_str(&format!("- {}\n", project.trim()));
        }
    }
    out.push_str("\nThe job, in the person's words:\n\n");
    out.push_str(description);
    out.push('\n');
    if let Some(problem) = problem {
        out.push_str(&format!(
            "\nYour last answer could not be used: {}\nAnswer again with the whole workflow, with that put right.\n",
            problem.trim()
        ));
    }
    Ok(out)
}

/// The steps of `step`'s own lists, a branch's paths and `otherwise`, to walk a tree of steps.
fn inner_lists(step: &mut Value) -> Vec<&mut Vec<Value>> {
    let mut lists = Vec::new();
    let Some(object) = step.as_object_mut() else {
        return lists;
    };
    for (key, value) in object.iter_mut() {
        match (key.as_str(), value) {
            ("otherwise", Value::Array(steps)) => lists.push(steps),
            ("paths", Value::Array(paths)) => {
                for path in paths {
                    if let Some(Value::Array(steps)) = path.get_mut("steps") {
                        lists.push(steps);
                    }
                }
            }
            _ => {}
        }
    }
    lists
}

/// Puts each step of `steps` in the shape the definition reads: a member's name becomes its
/// id (a name the team does not have becomes the first member, with a note), and a step
/// without an id gets one of its kind.
fn settle(
    steps: &mut Vec<Value>,
    ctx: &DraftContext,
    taken: &mut Vec<String>,
    notes: &mut Vec<String>,
) {
    for step in steps.iter_mut() {
        let kind = step
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("step")
            .to_string();
        let id = step
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| crate::step_template::name_ok(id) && !taken.iter().any(|t| t == id))
            .map(str::to_string);
        let id = id.unwrap_or_else(|| {
            let mut n = 1;
            while taken.iter().any(|t| *t == format!("{kind}-{n}")) {
                n += 1;
            }
            format!("{kind}-{n}")
        });
        taken.push(id.clone());
        let step_name = step
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .unwrap_or(&id)
            .to_string();
        if let Some(object) = step.as_object_mut() {
            object.insert("id".into(), json!(id));
            object.insert("name".into(), json!(step_name));
            // A member by name, without case; an id already there is kept.
            if let Some(named) = object
                .get("member")
                .and_then(Value::as_str)
                .map(str::to_string)
            {
                let wanted = named.trim();
                let found = ctx
                    .members
                    .iter()
                    .find(|m| m.id == wanted || m.name.trim().eq_ignore_ascii_case(wanted));
                let member = match found {
                    Some(member) => member,
                    None => {
                        let first = &ctx.members[0];
                        notes.push(format!(
                            "The step \"{step_name}\" asked for \"{wanted}\", who is not a member of the team: {} does it for now.",
                            first.name.trim()
                        ));
                        first
                    }
                };
                object.insert("member".into(), json!(member.id));
            }
        }
        for list in inner_lists(step) {
            settle(list, ctx, taken, notes);
        }
    }
}

/// Reads the model's `answer` into a draft for the team of `ctx`: the JSON object, with its
/// members named by id and each step given an id. `Err` says why it is no workflow: not JSON
/// of that shape, or, as `Invalid`, the first rule of the definition it breaks.
pub fn read_draft(answer: &str, ctx: &DraftContext) -> Result<WorkflowDraft> {
    let not_one = || Error::Other("Claude's answer was not a workflow; try again.".to_string());
    let text = unwrap_fence(answer);
    let Ok(Value::Object(mut object)) = serde_json::from_str::<Value>(text.trim()) else {
        return Err(not_one());
    };
    if ctx.members.is_empty() {
        return Err(Error::Invalid(
            "Add a member to the team first: a workflow's steps need one.".into(),
        ));
    }
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .unwrap_or("New workflow")
        .chars()
        .take(crate::workflow::MAX_NAME)
        .collect::<String>();
    let mut notes: Vec<String> = object
        .get("notes")
        .and_then(Value::as_array)
        .map(|notes| {
            notes
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let Some(Value::Array(mut steps)) = object.remove("steps") else {
        return Err(not_one());
    };
    settle(&mut steps, ctx, &mut Vec::new(), &mut notes);
    let definition = json!({
        "trigger": object.remove("trigger").unwrap_or_else(|| json!({"kind": "manual"})),
        "steps": steps,
        "inputs": object.remove("inputs").unwrap_or_else(|| json!([])),
        "budget": Budget::default(),
    });
    let definition: Definition = match serde_json::from_value(definition) {
        Ok(definition) => definition,
        Err(e) => {
            return Err(Error::Invalid(format!(
                "its JSON did not read as a workflow: {e}"
            )))
        }
    };
    let members: Vec<String> = ctx.members.iter().map(|m| m.id.clone()).collect();
    definition.validate(&members)?;
    Ok(WorkflowDraft {
        name,
        definition,
        notes,
    })
}

/// The drafts being written, at most one per dialog (`OnceRunner`, by dialog id).
pub struct WorkflowDrafts {
    runner: OnceRunner,
}

impl WorkflowDrafts {
    /// `program` is the CLI the runtime found, `None` when there is none. Each draft's CLI
    /// starts in a new empty folder in `dir`.
    pub fn new(
        program: Option<PathBuf>,
        env: Vec<(String, String)>,
        dir: PathBuf,
        limit: Duration,
        shutdown: CancellationToken,
    ) -> WorkflowDrafts {
        WorkflowDrafts {
            runner: OnceRunner::new(program, env, dir, limit, shutdown),
        }
    }

    /// Asks the CLI once with `prompt`.
    async fn ask(&self, dialog: &str, prompt: &str) -> Result<String> {
        match self
            .runner
            .run(dialog, DRAFT_INSTRUCTIONS, DEFAULT_MODEL, prompt)
            .await
        {
            Ok(answer) => Ok(answer),
            Err(OnceFailure::NoCli) => Err(Error::Other(NO_CLI.into())),
            Err(OnceFailure::Cancelled) => Err(Error::Other("The draft was cancelled.".into())),
            Err(OnceFailure::TimedOut) => Err(Error::Other(
                "Claude took too long to write the workflow.".into(),
            )),
            Err(OnceFailure::Failed(e)) => Err(Error::Other(format!(
                "Claude could not write the workflow: {e}"
            ))),
        }
    }

    /// Writes a draft of the workflow `description` asks for, for the team of `ctx`. A draft
    /// the same dialog asked for before is cancelled first. A draft that breaks a rule is asked
    /// for once more with the rule; a second that breaks one is refused with it.
    pub async fn draft(
        &self,
        dialog: &str,
        ctx: &DraftContext,
        description: &str,
    ) -> Result<WorkflowDraft> {
        let prompt = match draft_prompt(ctx, description, None) {
            Ok(prompt) => prompt,
            Err(e) => {
                self.runner.cancel(dialog);
                return Err(e);
            }
        };
        let first = self.ask(dialog, &prompt).await?;
        let problem = match read_draft(&first, ctx) {
            Ok(draft) => return Ok(draft),
            Err(Error::Invalid(problem)) => problem,
            Err(e) => e.to_string(),
        };
        let again = draft_prompt(ctx, description, Some(&problem))?;
        let second = self.ask(dialog, &again).await?;
        match read_draft(&second, ctx) {
            Ok(draft) => Ok(draft),
            Err(Error::Invalid(problem)) => Err(Error::Invalid(format!(
                "The workflow Claude wrote cannot be used: {problem}. Say it another way, or build it by hand."
            ))),
            Err(e) => Err(e),
        }
    }

    /// Cancels the dialog's draft, if one is being written.
    pub fn cancel(&self, dialog: &str) {
        self.runner.cancel(dialog);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::{Action, StepKind, Trigger, Wait};

    fn team() -> DraftContext {
        DraftContext {
            members: vec![
                DraftMember {
                    id: "01WRITER".into(),
                    name: "Writer".into(),
                    role: "copywriter".into(),
                },
                DraftMember {
                    id: "01HELPER".into(),
                    name: "Helper".into(),
                    role: String::new(),
                },
            ],
            projects: vec!["Site".into()],
        }
    }

    #[test]
    fn the_prompt_lists_the_team_and_the_job_and_on_a_second_asking_the_rule_broken() {
        let prompt = draft_prompt(&team(), "  Post every Tuesday.  ", None).unwrap();
        assert_eq!(
            prompt,
            "The team's members:\n- Writer (copywriter)\n- Helper\n\nThe team's projects, by name \
             (\"project\": \"\" is a plain folder Sage keeps for the team):\n- Site\n\nThe job, in \
             the person's words:\n\nPost every Tuesday.\n"
        );
        let again =
            draft_prompt(&team(), "Post.", Some("a workflow needs at least one step")).unwrap();
        assert!(again.ends_with(
            "\nYour last answer could not be used: a workflow needs at least one step\nAnswer \
             again with the whole workflow, with that put right.\n"
        ));
        for (ctx, text, why) in [
            (team(), "  ", "Say what the workflow should do."),
            (
                DraftContext::default(),
                "Post.",
                "Add a member to the team first: a workflow's steps need one.",
            ),
        ] {
            assert!(matches!(draft_prompt(&ctx, text, None), Err(Error::Invalid(m)) if m == why));
        }
        let long = "x".repeat(MAX_REQUEST_CHARS + 1);
        assert!(draft_prompt(&team(), &long, None).is_err());
    }

    #[test]
    fn an_answer_is_read_into_a_draft_with_members_by_id_and_ids_for_its_steps() {
        let answer = r#"```json
{
  "name": "  Tuesday post ",
  "inputs": [{"name": "publish_at", "label": "Publish at", "kind": "date_time", "required": true, "default": null}],
  "trigger": {"kind": "manual"},
  "steps": [
    {"id": "draft", "name": "Draft the post", "kind": "agent", "member": "writer", "job": "Write it.",
     "produces": ["post"], "options": {"model": "small", "lean": true, "reads": null}},
    {"name": "Approve", "kind": "check"},
    {"id": "Bad Id", "name": "", "kind": "wait", "wait": "input", "input": "publish_at"},
    {"id": "draft", "name": "Send it", "kind": "action", "action": "http", "method": "post",
     "url": "https://example.com/post", "headers": [], "body": "{{draft.post}}", "unattended": false}
  ],
  "notes": ["Fill in the address of your posting service.", "  "]
}
```"#;
        let draft = read_draft(answer, &team()).unwrap();
        assert_eq!(draft.name, "Tuesday post");
        assert_eq!(
            draft.notes,
            ["Fill in the address of your posting service."]
        );
        let steps = &draft.definition.steps;
        let ids: Vec<&str> = steps.iter().map(|s| s.id.as_str()).collect();
        // An id it gave is kept; a missing, bad or repeated one is made from the step's kind.
        assert_eq!(ids, ["draft", "check-1", "wait-1", "action-1"]);
        assert_eq!(steps[2].name, "wait-1");
        assert!(matches!(
            &steps[0].kind,
            StepKind::Agent { member, produces, options, .. }
                if member == "01WRITER" && produces == &["post"] && options.lean
        ));
        assert!(
            matches!(&steps[2].kind, StepKind::Wait { wait: Wait::Input { input } } if input == "publish_at")
        );
        assert!(matches!(
            &steps[3].kind,
            StepKind::Action {
                action: Action::Http { .. },
                unattended: false
            }
        ));
        assert_eq!(draft.definition.trigger, Trigger::Manual);
        assert_eq!(draft.definition.budget, Budget::default());
        assert_eq!(draft.definition.inputs.len(), 1);
    }

    #[test]
    fn a_member_the_team_lacks_is_the_first_member_with_a_note() {
        let answer = r#"{"name": "N", "steps": [
            {"id": "a", "name": "Research", "kind": "agent", "member": "Researcher", "job": "Look."}
        ]}"#;
        let draft = read_draft(answer, &team()).unwrap();
        assert!(
            matches!(&draft.definition.steps[0].kind, StepKind::Agent { member, .. } if member == "01WRITER")
        );
        assert_eq!(
            draft.notes,
            ["The step \"Research\" asked for \"Researcher\", who is not a member of the team: Writer does it for now."]
        );
        // No trigger and no inputs: started by hand, asking for nothing.
        assert_eq!(draft.definition.trigger, Trigger::Manual);
        assert!(draft.definition.inputs.is_empty());
    }

    #[test]
    fn an_answer_that_is_no_workflow_or_breaks_a_rule_is_refused_with_why() {
        for bad in ["Sure! Here is your workflow.", "[1, 2]", r#"{"name": "N"}"#] {
            assert!(
                matches!(read_draft(bad, &team()), Err(Error::Other(m)) if m == "Claude's answer was not a workflow; try again."),
                "{bad}"
            );
        }
        // A send with no check before it: the definition's own rule, as `Invalid`.
        let unchecked = r#"{"name": "N", "steps": [
            {"id": "send", "name": "Send", "kind": "action", "action": "http", "method": "post",
             "url": "https://example.com/x", "headers": [], "body": "hi", "unattended": false}
        ]}"#;
        assert!(matches!(
            read_draft(unchecked, &team()),
            Err(Error::Invalid(m)) if m.starts_with("step \"Send\": an action that sends something needs a check")
        ));
        let shape = r#"{"name": "N", "steps": [{"id": "x", "name": "X", "kind": "dance"}]}"#;
        assert!(matches!(
            read_draft(shape, &team()),
            Err(Error::Invalid(m)) if m.starts_with("its JSON did not read as a workflow:")
        ));
    }
}
