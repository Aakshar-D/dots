//! Check-ins (check-ins spec sections 2, 3 and 5): a dot tells the person something with
//! `notify`, asks them with `ask`, and schedules a run of its own with `follow_up`. The tools
//! are the same on both engines: built in on the local engine, tools of Dots' own `dots` MCP
//! server on the Claude engine. This module holds their names, descriptions and input rules,
//! and `CheckIns`, which performs an allowed call.

use std::sync::{Arc, OnceLock, Weak};

use chrono::{DateTime, Local, TimeDelta, Utc};
use serde_json::{json, Value};

use crate::events::{Bus, RuntimeEvent};
use crate::followups::{parse_time, FollowUps};
use crate::memory::Memories;
use crate::model::{EngineKind, Message, NewRun, Run, RunStatus, TriggerKind};
use crate::runner::Runner;
use crate::store::Store;
use crate::{Error, Result};

/// The longest note, question or follow-up note, in characters after trimming.
pub const MAX_TEXT: usize = 2_000;
/// The most choices a question offers.
pub const MAX_CHOICES: usize = 5;
/// The longest choice, in characters after trimming.
pub const MAX_CHOICE: usize = 80;

/// What `notify` answers.
pub const SENT: &str = "Sent.";
/// What `ask` answers.
pub const ASKED: &str = "Your question was sent. Finish your turn now; the person's answer will \
    arrive as your next message.";
/// What a second `ask` in one run answers.
pub const ASKED_ALREADY: &str = "You already asked a question in this run; finish your turn now.";
/// The most notes one run may send, so a dot in a loop cannot flood the Inbox (spec section 1).
pub const MAX_NOTES: usize = 10;
/// The answer to a note past `MAX_NOTES`.
pub const NOTES_SPENT: &str = "This run has sent 10 notes; that is the most it may send.";

const CHOICES_RULE: &str = "`choices` must be a list of 1 to 5 strings.";
const CHOICE_RULE: &str = "Each choice must be 1 to 80 characters.";
const ONE_OF_RULE: &str = "Give exactly one of `at` or `in`.";
const AT_RULE: &str =
    "`at` must be a time in RFC 3339 with an offset, such as 2026-10-21T09:00:00+02:00.";
const IN_RULE: &str = "`in` must be a whole number and a unit m, h or d, such as 90m, 2h or 1d.";

const NOTIFY_DESCRIPTION: &str = "Post a short note to the person and keep working. Use it \
    sparingly, for progress they would want to know now; your summary reaches them when the \
    run ends.";
const ASK_DESCRIPTION: &str = "Ask the person a question, then finish your turn: their answer \
    arrives as your next message. Ask only when you cannot go on well without their judgement. \
    Offer `choices` when the answer is one of a few. One question per run.";
const FOLLOW_UP_DESCRIPTION: &str = "Schedule a run of your own that continues this \
    conversation later, for work that has to wait: a build, a reply, a date. Give `note`, what \
    to do then, and either `at` (RFC 3339 with an offset) or `in` (a whole number and m, h or \
    d, such as 90m, 2h or 1d). It must be 5 minutes to 30 days away, and a dot has at most 3 \
    waiting.";

/// One of the three check-in tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tool {
    Notify,
    Ask,
    FollowUp,
}

impl Tool {
    pub const ALL: [Tool; 3] = [Tool::Notify, Tool::Ask, Tool::FollowUp];

    /// The name a model calls the tool by on the local engine, and the `dots` server's tool.
    pub fn name(self) -> &'static str {
        match self {
            Tool::Notify => "notify",
            Tool::Ask => "ask",
            Tool::FollowUp => "follow_up",
        }
    }

    /// The name the policy, approvals and grants know the tool by (`policy::CHECK_IN_TOOLS`).
    pub fn policy_name(self) -> &'static str {
        match self {
            Tool::Notify => "Notify",
            Tool::Ask => "Ask",
            Tool::FollowUp => "FollowUp",
        }
    }

    pub fn from_name(name: &str) -> Option<Tool> {
        Tool::ALL.into_iter().find(|t| t.name() == name)
    }

    pub fn from_policy_name(name: &str) -> Option<Tool> {
        Tool::ALL.into_iter().find(|t| t.policy_name() == name)
    }

    /// The name the run's model knows the tool by: Dots' own MCP server's tool on the Claude
    /// engine, a built-in tool on the local engine.
    pub fn model_name(self, engine: EngineKind) -> &'static str {
        match (engine, self) {
            (EngineKind::Claude, Tool::Notify) => "mcp__dots__notify",
            (EngineKind::Claude, Tool::Ask) => "mcp__dots__ask",
            (EngineKind::Claude, Tool::FollowUp) => "mcp__dots__follow_up",
            (EngineKind::Local, tool) => tool.name(),
        }
    }

    /// When to use the tool, shared by both engines.
    pub fn description(self) -> &'static str {
        match self {
            Tool::Notify => NOTIFY_DESCRIPTION,
            Tool::Ask => ASK_DESCRIPTION,
            Tool::FollowUp => FOLLOW_UP_DESCRIPTION,
        }
    }

    /// The tool's arguments, as JSON Schema properties.
    pub fn properties(self) -> Value {
        let s = |d: &str| json!({"type": "string", "description": d});
        match self {
            Tool::Notify => json!({ "text": s("What to tell the person, up to 2,000 characters") }),
            Tool::Ask => json!({
                "question": s("The question, up to 2,000 characters"),
                "choices": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Up to 5 short answers the person can pick, each up to 80 characters"
                }
            }),
            Tool::FollowUp => json!({
                "note": s("What to do when the follow-up runs, up to 2,000 characters"),
                "at": s("When, in RFC 3339 with an offset, e.g. 2026-10-21T09:00:00+02:00"),
                "in": s("How long from now: a whole number and m, h or d, e.g. 90m, 2h, 1d")
            }),
        }
    }

    pub fn required(self) -> &'static [&'static str] {
        match self {
            Tool::Notify => &["text"],
            Tool::Ask => &["question"],
            Tool::FollowUp => &["note"],
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

/// When a follow-up is due, as the call gave it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum When {
    At(DateTime<Utc>),
    In(TimeDelta),
}

/// A call whose input passed the rules (`check`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Notify {
        text: String,
    },
    Ask {
        question: String,
        choices: Option<Vec<String>>,
    },
    FollowUp {
        note: String,
        when: When,
    },
}

/// The input rules of section 2: what a call of `tool` with `input` asks for, or the rule it
/// breaks, in one sentence for the model. The follow-up limits are checked when it is
/// scheduled (`followups::due_at`), against the loop's clock.
pub fn check(tool: Tool, input: &Value) -> std::result::Result<Request, String> {
    match tool {
        Tool::Notify => Ok(Request::Notify {
            text: text(input, "text")?,
        }),
        Tool::Ask => Ok(Request::Ask {
            question: text(input, "question")?,
            choices: choices(input)?,
        }),
        Tool::FollowUp => Ok(Request::FollowUp {
            note: text(input, "note")?,
            when: when(input)?,
        }),
    }
}

/// The string argument `key`, trimmed: 1 to `MAX_TEXT` characters.
fn text(input: &Value, key: &str) -> std::result::Result<String, String> {
    let Some(raw) = input.get(key).and_then(Value::as_str) else {
        return Err(format!(
            "`{key}` must be a string of 1 to 2,000 characters."
        ));
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(format!("`{key}` must not be blank."));
    }
    if trimmed.chars().count() > MAX_TEXT {
        return Err(format!("`{key}` must be at most 2,000 characters."));
    }
    Ok(trimmed.to_string())
}

/// The optional `choices`: 1 to `MAX_CHOICES` strings of 1 to `MAX_CHOICE` characters, trimmed.
fn choices(input: &Value) -> std::result::Result<Option<Vec<String>>, String> {
    let list = match input.get("choices") {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Array(list)) if (1..=MAX_CHOICES).contains(&list.len()) => list,
        Some(_) => return Err(CHOICES_RULE.into()),
    };
    let mut out = Vec::new();
    for item in list {
        let Some(choice) = item.as_str().map(str::trim) else {
            return Err(CHOICES_RULE.into());
        };
        if !(1..=MAX_CHOICE).contains(&choice.chars().count()) {
            return Err(CHOICE_RULE.into());
        }
        out.push(choice.to_string());
    }
    Ok(Some(out))
}

/// The one of `at` and `in` that is given; a blank string counts as absent.
fn when(input: &Value) -> std::result::Result<When, String> {
    let given = |key: &str| {
        input
            .get(key)
            .filter(|v| !v.is_null() && v.as_str().is_none_or(|s| !s.trim().is_empty()))
    };
    match (given("at"), given("in")) {
        (Some(at), None) => at
            .as_str()
            .and_then(|s| DateTime::parse_from_rfc3339(s.trim()).ok())
            .map(|t| When::At(t.with_timezone(&Utc)))
            .ok_or_else(|| AT_RULE.to_string()),
        (None, Some(within)) => within
            .as_str()
            .and_then(parse_in)
            .map(When::In)
            .ok_or_else(|| IN_RULE.to_string()),
        _ => Err(ONE_OF_RULE.into()),
    }
}

/// `in` as a duration: digits, then `m`, `h` or `d`. A number too large for a duration reads as
/// the longest one, which the 30-day limit refuses.
fn parse_in(s: &str) -> Option<TimeDelta> {
    let s = s.trim();
    let unit = s.chars().last()?;
    let digits = &s[..s.len() - unit.len_utf8()];
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: i64 = digits.parse().unwrap_or(i64::MAX);
    let delta = match unit {
        'm' => TimeDelta::try_minutes(n),
        'h' => TimeDelta::try_hours(n),
        'd' => TimeDelta::try_days(n),
        _ => return None,
    };
    Some(delta.unwrap_or(TimeDelta::MAX))
}

/// `t` in local time as a person reads it in a result or a prompt, e.g. `Tue 21 Oct 09:00`.
pub fn local_label(t: DateTime<Utc>) -> String {
    t.with_timezone(&Local)
        .format("%a %-d %b %H:%M")
        .to_string()
}

/// Performs the check-in tools' calls for both engines (spec sections 2 and 3), and holds the
/// dots' memories, whose tools are served wherever the check-in tools are (dot memory spec
/// section 3), and the workflow runs, for the workflow tools (teams spec section 7). Cheap to
/// clone; the clones share the workflow runs once they are set.
#[derive(Clone)]
pub struct CheckIns {
    store: Store,
    bus: Bus,
    follow_ups: Arc<FollowUps>,
    memories: Memories,
    /// Weak: the workflow runs hold the runner, which holds these.
    workflows: Arc<OnceLock<Weak<crate::workflows::Workflows>>>,
}

impl CheckIns {
    pub fn new(store: Store, bus: Bus, follow_ups: Arc<FollowUps>) -> Self {
        Self {
            memories: Memories::new(store.clone(), bus.clone()),
            store,
            bus,
            follow_ups,
            workflows: Arc::new(OnceLock::new()),
        }
    }

    /// Gives the workflow tools the workflow runs. The runtime calls it once, at start.
    pub fn set_workflows(&self, workflows: &Arc<crate::workflows::Workflows>) {
        let _ = self.workflows.set(Arc::downgrade(workflows));
    }

    /// The workflow runs, once the runtime has set them and while they exist.
    pub fn workflows(&self) -> Option<Arc<crate::workflows::Workflows>> {
        self.workflows.get().and_then(Weak::upgrade)
    }

    /// The dots' memories: the person's commands and the memory tools go through them.
    pub fn memories(&self) -> &Memories {
        &self.memories
    }

    /// Performs an allowed call of `tool` by the run `run_id`. `Ok` is the text the model gets;
    /// `Err` a tool error: the input rule or the limit the call breaks, in one sentence, or
    /// "Not delivered: <reason>." when it could not be stored (logged). The run goes on either
    /// way.
    pub async fn call(
        &self,
        run_id: &str,
        tool: Tool,
        input: &Value,
    ) -> std::result::Result<String, String> {
        let request = check(tool, input)?;
        match self.perform(run_id, request).await {
            Ok(text) => Ok(text),
            Err(Error::Invalid(rule)) => Err(rule),
            Err(e) => {
                tracing::warn!(run = %run_id, tool = tool.name(), "check-in not delivered: {e}");
                Err(format!("Not delivered: {e}."))
            }
        }
    }

    async fn perform(&self, run_id: &str, request: Request) -> Result<String> {
        let run = self.store.get_run(run_id).await?;
        match request {
            Request::Notify { text } => {
                let sent = self
                    .store
                    .add_run_note(&run.dot_id, &run.id, &text, MAX_NOTES)
                    .await?;
                match sent {
                    Some(message) => {
                        self.posted(message);
                        Ok(SENT.to_string())
                    }
                    None => Err(Error::Invalid(NOTES_SPENT.to_string())),
                }
            }
            Request::Ask { question, choices } => {
                let asked = self
                    .store
                    .add_question(&run.dot_id, &run.id, &question, choices.as_deref())
                    .await?;
                match asked {
                    Some(message) => {
                        self.posted(message);
                        Ok(ASKED.to_string())
                    }
                    None => Err(Error::Invalid(ASKED_ALREADY.to_string())),
                }
            }
            Request::FollowUp { note, when } => {
                let row = self.follow_ups.schedule(&run, &note, when).await?;
                let due = parse_time(&row.due_at)?;
                Ok(format!(
                    "Follow-up {} scheduled for {}.",
                    row.id,
                    local_label(due)
                ))
            }
        }
    }

    fn posted(&self, message: Message) {
        let _ = self.bus.send(RuntimeEvent::MessagePosted { message });
    }
}

/// The run that carries the person's answer to something `asked` posted, in the thread whose
/// last run is `end` (check-ins spec section 5): a chat run that resumes `end`'s session when it
/// has one, else a chat run of its own. `answer` is what the person typed, which the chat shows
/// in place of the message.
pub(crate) fn continuation(
    asked: &Run,
    end: &Run,
    message: String,
    answer: Option<&str>,
) -> NewRun {
    let mut new = NewRun::new(&asked.dot_id, TriggerKind::Chat);
    if end.session_id.is_some() {
        new.parent_run_id = Some(end.id.clone());
        new.root_run_id = Some(end.root_run_id.clone());
        new.session_id = end.session_id.clone();
        new.workspace_path = end.workspace_path.clone();
        new.branch = end.branch.clone();
        new.base_commit = end.base_commit.clone();
    }
    new.payload = Some(match answer {
        Some(answer) => json!({ "message": message, "answer": answer }),
        None => json!({ "message": message }),
    });
    new
}

/// What a reply or a chat message is told while an app the person accepted in the thread is
/// being added (`Runtime::decide_app_proposal`).
pub const DECIDING: &str = "Dots is still adding the app you accepted in this chat; send your \
                            message again in a moment";

/// What the Add button is told when the thread is already adding an app.
pub const ADDING: &str = "Dots is still adding this app; wait until it has finished";

/// The refusal of a reply to a thread that has not ended, in the words `Runtime::chat` gives.
pub fn thread_busy(status: RunStatus) -> Error {
    Error::Conflict(
        match status {
            RunStatus::AwaitingApproval => {
                "this chat is waiting for an approval; answer it, then send your message again"
            }
            _ => {
                "the dot is still working on this chat; wait until it has finished, then send \
                 your message again"
            }
        }
        .into(),
    )
}

/// The person's side (spec section 5): the messages, and the answer to a question.
impl CheckIns {
    /// Every message that waits for the person and the newest others, of one dot or of all,
    /// newest first.
    pub async fn list_messages(&self, dot_id: Option<&str>) -> Result<Vec<Message>> {
        self.store.list_messages(dot_id).await
    }

    /// Marks a message read; a question stays open.
    pub async fn mark_read(&self, id: &str) -> Result<Message> {
        let message = self.store.mark_message_read(id).await?;
        self.updated(message.clone());
        Ok(message)
    }

    /// Dismisses an open question: it leaves the Inbox unanswered, and cannot be answered.
    pub async fn dismiss(&self, id: &str) -> Result<Message> {
        let message = self.store.dismiss_question(id).await?;
        self.updated(message.clone());
        Ok(message)
    }

    /// Answers the open question `id` with `text` (spec section 5). Under the thread lock the
    /// end of the question's thread is read as `Runtime::chat` reads it: a thread that has not
    /// ended is refused with `chat`'s sentences (`thread_busy`); a quiet end with a session is
    /// continued by a chat run; an end without one gets a chat run of its own. The question is
    /// closed and the run stored in one transaction, so a second answer is refused.
    pub async fn reply(&self, runner: &Runner, id: &str, text: &str) -> Result<Run> {
        let text = text.trim();
        if text.is_empty() {
            return Err(Error::Invalid("the answer must not be empty".into()));
        }
        let question = self.store.get_message(id).await?;
        if let Some(refusal) = question.refusal() {
            return Err(refusal);
        }
        let Some(asked_in) = &question.run_id else {
            return Err(Error::Invalid(format!(
                "message {id} was not asked by a run"
            )));
        };
        let asked = self.store.get_run(asked_in).await?;
        // Held until the new run is stored, as `Runtime::chat` holds it.
        let lock = self.follow_ups.thread_lock();
        let _one_at_a_time = lock.lock().await;
        if self.follow_ups.deciding(&asked.root_run_id) {
            return Err(Error::Conflict(DECIDING.into()));
        }
        let thread = self.store.list_thread(&asked.root_run_id).await?;
        if let Some(active) = thread.iter().find(|r| !r.status.is_terminal()) {
            return Err(thread_busy(active.status));
        }
        let end = thread.last().unwrap_or(&asked);
        let message = if end.session_id.is_some() {
            format!("Answer to your question \"{}\": {text}", question.text)
        } else {
            format!(
                "Earlier you asked the person: \"{}\"\nTheir answer: {text}",
                question.text
            )
        };
        let new = continuation(&asked, end, message, Some(text));
        let (answered, run) = self.store.answer_question(id, &new).await?;
        runner.enqueued(&run);
        self.updated(answered);
        Ok(run)
    }

    fn updated(&self, message: Message) {
        let _ = self.bus.send(RuntimeEvent::MessageUpdated { message });
    }
}
