//! Check-ins (check-ins spec sections 2, 3 and 5): a dot tells the person something with
//! `notify`, asks them with `ask`, and schedules a run of its own with `follow_up`. The tools
//! are the same on both engines: built in on the local engine, tools of Dots' own `dots` MCP
//! server on the Claude engine. This module holds their names, descriptions and input rules.

use chrono::{DateTime, Local, TimeDelta, Utc};
use serde_json::{json, Value};

use crate::model::EngineKind;

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
