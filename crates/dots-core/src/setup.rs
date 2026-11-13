//! The setup agent (setup agent spec §6): the prompt that asks the `claude` CLI to turn a
//! person's description into one dot, the check of its answer, and the extras a proposal may
//! add to its preset.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::Local;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio_util::sync::CancellationToken;

use crate::engine::claude::once::{OnceFailure, OnceRunner, OnceUsage, NO_CLI};
use crate::library::{check_name, LibraryKind, Uses};
use crate::model::Dot;
use crate::policy::{Action, Policy, Preset, Rule};
use crate::schedule::Schedule;
use crate::skill_creator::{unwrap_fence, DEFAULT_MODEL, MAX_REQUEST_CHARS};
use crate::{Error, Result};

/// The system prompt of the setup agent (spec §6, §6.1, §6.3).
pub const SETUP_INSTRUCTIONS: &str = r#"You set up Sage sols. A person describes a task they want done, once or again and again, and you turn it into exactly one sol. Ask only what you truly cannot guess, in at most two rounds of questions.

What a sol is: a Claude Code agent with instructions it follows on every run, a folder it works in, an optional schedule, permissions, and items from the person's Library (skills, MCP servers, plugins, context collections) it may use.

How to write the instructions:
- Write to the sol, in the second person ("You read ...").
- Use numbered steps where there are steps.
- Say what to produce and where to put it, what to tell the person at the end, and what never to do.
- Plain Markdown, at most 4000 characters.

Answer with exactly one JSON object and nothing else: no greeting, no explanation, no text before or after it.

To ask questions, answer:
{ "questions": ["Which folder should it read?", "What time each day?"] }
with 1 to 3 questions, each at most 300 characters.

To propose the sol, answer:
{
  "proposal": {
    "name": "Downloads digest",
    "instructions": "1. List the files in ...",
    "folder": "C:\\Users\\pat\\Downloads",
    "schedule": { "rule": { "kind": "weekly", "days": ["mon", "tue", "wed", "thu", "fri"], "time": "09:00" } },
    "preset": "read-only",
    "extras": [],
    "uses": { "skills": [], "mcp_servers": [], "plugins": [], "context": [] },
    "first_message": "Run it now so I can see the first digest.",
    "notes": "I assumed weekdays at 9:00. Change the schedule if you want weekends too."
  }
}
- "name": 1 to 64 characters: letters, digits, spaces, '-', '_' or '.'. Pick one that none of the person's existing sols has.
- "folder": an existing folder the person named, written as a full path, or null and Sage makes a new folder for the sol.
- "schedule": null when the sol runs only when the person talks to it, or one of these:
  { "rule": { "kind": "once", "at": "2026-11-04T09:00" } }   once, at a local date and time
  { "rule": { "kind": "daily", "time": "09:00" } }   every day at a local time
  { "rule": { "kind": "weekly", "days": ["mon", "fri"], "time": "09:00" } }   on these weekdays ("mon" to "sun")
  { "rule": { "kind": "every", "minutes": 30 } }   every 1 to 10080 minutes
  { "rule": { "kind": "cron", "expr": "0 0 9 * * MON-FRI" } }   cron with seconds first; only when nothing above fits
  Times are local, HH:MM in 24 hours. A schedule other than "once" may also stop: add "until": { "count": 10 } or "until": { "date": "2026-12-31T23:59" }.
- "preset": what the sol may do. Pick the least power that does the job:
  "read-only": May read files. Never writes, runs commands, reads websites or uses MCP servers.
  "sandboxed": May edit files in its folder and run a few safe git commands. Asks before anything else.
  "trusted": May run commands without asking.
  Never pick "trusted" unless the person asked for it in their own words. A task that saves files needs at least "sandboxed".
  A "read-only" sol can only use Read, Glob and Grep. It cannot run commands, so it cannot see file dates or sizes, though Glob lists files newest first.
  Write instructions that use only what the chosen preset allows.
  When the task needs file dates, sizes or commands, pick "sandboxed" and say in "notes" that each command will ask for approval; or, if you keep a smaller preset, say in "notes" what the sol cannot do.
- "extras": permissions added on top of the preset so the sol can run without stopping to ask. Only these two exist:
  "web": reads websites and searches the web without asking. Add it only when the task reads websites.
  "mcp:<server>": uses that MCP server without asking, for example "mcp:gmail". Add it only for a server you also put in "uses" and the task needs.
- "uses": names from the person's Library only. Leave out any group you do not need.
- "first_message": what Sage sends the sol as its first chat message, right after it is made.
- "notes": a short line or two for the person: what you assumed and what they may want to change. May be empty.

Guessing rules:
- Prefer a sensible default, and say so in "notes", over asking.
- Ask only about things that change what the sol does: which folder or website, what time, what to do with the results.
- Never ask about the model, limits or permissions.
- "My Downloads folder" and the like are folders under the person's home folder.

When you are given a current proposal and a change, answer with the whole revised proposal: changed as asked, and otherwise kept as it was.
"#;

/// The line added when the agent may no longer ask (spec §6).
const MUST_PROPOSE: &str = "Answer with a proposal now. Do not ask questions.";

const UNREADABLE: &str = "The setup agent's answer could not be read. Try again.";
const FAILED: &str = "Setup could not finish. Try describing the task again, or fill in the form.";

const MAX_QUESTIONS: usize = 3;
/// The most rounds of questions before the agent must propose.
const MAX_ROUNDS: usize = 2;
const MAX_QUESTION_CHARS: usize = 300;
/// The most a proposal's instructions may hold, in characters; longer ones are cut.
pub const MAX_INSTRUCTIONS_CHARS: usize = 4000;
const MAX_NAME_CHARS: usize = 64;
/// The name a proposal gets when nothing of its own name is left.
const FALLBACK_NAME: &str = "New sol";

/// One round of the agent's questions and the person's answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetupRound {
    pub questions: Vec<String>,
    pub answers: Vec<String>,
}

/// What the setup view asks for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetupRequest {
    pub description: String,
    /// The rounds of questions before the first proposal.
    pub rounds: Vec<SetupRound>,
    /// The proposal to change, for "Ask for changes".
    pub current: Option<SetupProposal>,
    pub change: Option<String>,
    /// "Skip, just propose".
    pub skip: bool,
}

/// The dot the agent proposes (spec §6.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetupProposal {
    pub name: String,
    pub instructions: String,
    pub folder: Option<String>,
    pub schedule: Option<Schedule>,
    pub preset: Preset,
    #[serde(default)]
    pub extras: Vec<String>,
    #[serde(default)]
    pub uses: Uses,
    #[serde(default)]
    pub first_message: String,
    #[serde(default)]
    pub notes: String,
}

/// The checked answer: more questions, or a proposal and the folder new dots go under.
// One answer per request, passed by value once: not boxed, so callers match on it plainly.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SetupAnswer {
    Questions {
        questions: Vec<String>,
    },
    Proposal {
        proposal: SetupProposal,
        new_dot_root: String,
    },
}

/// The person's Library, by name, as the agent is told about it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SetupLibrary {
    /// (name, description)
    pub skills: Vec<(String, String)>,
    pub mcp_servers: Vec<String>,
    pub plugins: Vec<String>,
    pub context: Vec<String>,
}

/// What the agent is given besides the request (spec §6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetupContext {
    /// For example "2026-11-03 (Tuesday)".
    pub today: String,
    /// The local UTC offset, for example "+01:00".
    pub utc_offset: String,
    pub home: String,
    pub new_dot_root: String,
    pub library: SetupLibrary,
    pub dot_names: Vec<String>,
}

/// The agent must answer with a proposal: the person skipped the questions, asked for a
/// change, or has answered two rounds already.
pub fn must_propose(req: &SetupRequest) -> bool {
    req.skip || req.current.is_some() || req.rounds.len() >= MAX_ROUNDS
}

fn invalid<T>(m: impl Into<String>) -> Result<T> {
    Err(Error::Invalid(m.into()))
}

/// The request's text, sent on the CLI's stdin. `Invalid` for an empty description, a
/// description, change or answer over `MAX_REQUEST_CHARS`, more than two rounds, a round with
/// more answers than questions, and a change without a current proposal (or the reverse).
pub fn setup_prompt(ctx: &SetupContext, req: &SetupRequest) -> Result<String> {
    let description = req.description.trim();
    if description.is_empty() {
        return invalid("Describe the task first.");
    }
    let too_long = |s: &str| s.chars().count() > MAX_REQUEST_CHARS;
    if too_long(description) {
        return invalid(format!(
            "Describe the task in at most {MAX_REQUEST_CHARS} characters."
        ));
    }
    if req.rounds.len() > MAX_ROUNDS {
        return invalid(format!("Answer at most {MAX_ROUNDS} rounds of questions."));
    }
    for round in &req.rounds {
        if round.answers.len() > round.questions.len() {
            return invalid("A round has more answers than questions.");
        }
        if round.answers.iter().any(|a| too_long(a.trim())) {
            return invalid(format!(
                "Answer each question in at most {MAX_REQUEST_CHARS} characters."
            ));
        }
    }
    let change = match (req.current.as_ref(), req.change.as_deref().map(str::trim)) {
        (None, None) => None,
        (Some(_), None) | (Some(_), Some("")) | (None, Some(_)) => {
            return invalid("Say what should change.")
        }
        (Some(current), Some(change)) => {
            if too_long(change) {
                return invalid(format!(
                    "Say what should change in at most {MAX_REQUEST_CHARS} characters."
                ));
            }
            Some((current, change))
        }
    };

    let mut out = String::new();
    out.push_str(&format!(
        "Today is {}. The local time is UTC{}.\n",
        ctx.today, ctx.utc_offset
    ));
    out.push_str(&format!("The person's home folder is {}.\n", ctx.home));
    out.push_str(&format!(
        "A sol without a folder gets a new one under {}.\n",
        ctx.new_dot_root
    ));

    let lib = &ctx.library;
    let mut library = String::new();
    if !lib.skills.is_empty() {
        library.push_str("\nSkills:\n");
        for (name, description) in &lib.skills {
            let description = description.trim();
            if description.is_empty() {
                library.push_str(&format!("- {name}\n"));
            } else {
                library.push_str(&format!("- {name}: {description}\n"));
            }
        }
    }
    for (title, names) in [
        ("MCP servers", &lib.mcp_servers),
        ("Plugins", &lib.plugins),
        ("Context collections", &lib.context),
    ] {
        if !names.is_empty() {
            library.push_str(&format!("\n{title}:\n"));
            for name in names {
                library.push_str(&format!("- {name}\n"));
            }
        }
    }
    if library.is_empty() {
        out.push_str("\nThe person's Library is empty.\n");
    } else {
        out.push_str("\nThe person's Library:\n");
        out.push_str(&library);
    }

    if !ctx.dot_names.is_empty() {
        out.push_str("\nThe person's existing sols (a new name must differ from these):\n");
        for name in &ctx.dot_names {
            out.push_str(&format!("- {name}\n"));
        }
    }

    out.push_str(&format!(
        "\nThe person's description, between the lines <<<DESCRIPTION and DESCRIPTION>>>:\n\n\
         <<<DESCRIPTION\n{description}\nDESCRIPTION>>>\n"
    ));

    for (i, round) in req.rounds.iter().enumerate() {
        out.push_str(&format!("\nRound {} of your questions:\n", i + 1));
        for q in &round.questions {
            out.push_str(&format!("- {}\n", q.trim()));
        }
        out.push_str("The person's answers:\n");
        for a in &round.answers {
            out.push_str(&format!("- {}\n", a.trim()));
        }
    }

    if let Some((current, change)) = change {
        let json = serde_json::to_string_pretty(current)?;
        out.push_str(&format!(
            "\nThe current proposal:\n\n{json}\n\nChange it like this:\n\n{change}\n"
        ));
    }

    if must_propose(req) {
        out.push_str(&format!("\n{MUST_PROPOSE}\n"));
    } else {
        out.push_str("\nAnswer with your questions or with a proposal.\n");
    }
    Ok(out)
}

fn unreadable<T>() -> Result<T> {
    Err(Error::Other(UNREADABLE.into()))
}

/// The agent's answer, checked (spec §6.2). Text that is not one JSON object with exactly one
/// of `questions` and `proposal`, bad questions, and a proposal that cannot be read are
/// "could not be read"; questions when the agent must propose are "could not finish". A
/// proposal is repaired by `check_proposal`, renamed on a clash.
pub fn check_setup_answer(
    text: &str,
    ctx: &SetupContext,
    req: &SetupRequest,
) -> Result<SetupAnswer> {
    let Ok(Value::Object(mut answer)) = serde_json::from_str::<Value>(unwrap_fence(text).trim())
    else {
        return unreadable();
    };
    match (answer.remove("questions"), answer.remove("proposal")) {
        (Some(questions), None) => {
            let questions = read_questions(questions)?;
            if must_propose(req) {
                return Err(Error::Other(FAILED.into()));
            }
            Ok(SetupAnswer::Questions { questions })
        }
        (None, Some(Value::Object(proposal))) => {
            let mut notes = Vec::new();
            let mut proposal = read_proposal(proposal, &mut notes)?;
            add_notes(&mut proposal.notes, notes);
            if proposal.first_message.trim().is_empty() {
                proposal.first_message = req.description.trim().to_string();
            }
            let proposal = match check_proposal(proposal, ctx, true) {
                Ok(p) => p,
                Err(Error::Invalid(_)) => return unreadable(),
                Err(e) => return Err(e),
            };
            Ok(SetupAnswer::Proposal {
                proposal,
                new_dot_root: ctx.new_dot_root.clone(),
            })
        }
        _ => unreadable(),
    }
}

fn read_questions(value: Value) -> Result<Vec<String>> {
    let Ok(questions) = serde_json::from_value::<Vec<String>>(value) else {
        return unreadable();
    };
    let questions: Vec<String> = questions.iter().map(|q| q.trim().to_string()).collect();
    let ok = (1..=MAX_QUESTIONS).contains(&questions.len())
        && questions
            .iter()
            .all(|q| !q.is_empty() && q.chars().count() <= MAX_QUESTION_CHARS);
    if ok {
        Ok(questions)
    } else {
        unreadable()
    }
}

/// A proposal from its JSON. A preset other than read-only, sandboxed or trusted becomes
/// sandboxed and a schedule that does not parse is left out, each with a note; anything else
/// that does not read is "could not be read".
fn read_proposal(mut p: Map<String, Value>, notes: &mut Vec<String>) -> Result<SetupProposal> {
    let preset = match p.remove("preset") {
        Some(Value::String(s)) => match Preset::parse(s.trim()) {
            Ok(Preset::Custom) | Err(_) => None,
            Ok(preset) => Some(preset),
        },
        _ => None,
    };
    let preset = preset.unwrap_or_else(|| {
        notes.push(CUSTOM_NOTE.to_string());
        Preset::Sandboxed
    });
    p.insert("preset".into(), serde_json::to_value(preset)?);

    if let Some(schedule) = p.remove("schedule") {
        if !schedule.is_null() {
            if serde_json::from_value::<Schedule>(schedule.clone()).is_ok() {
                p.insert("schedule".into(), schedule);
            } else {
                notes.push(SCHEDULE_NOTE.to_string());
            }
        }
    }
    for key in ["folder", "schedule"] {
        p.entry(key).or_insert(Value::Null);
    }
    for key in ["extras", "uses", "first_message", "notes"] {
        if p.get(key).is_some_and(Value::is_null) {
            p.remove(key);
        }
    }
    match serde_json::from_value::<SetupProposal>(Value::Object(p)) {
        Ok(proposal) => Ok(proposal),
        Err(_) => unreadable(),
    }
}

const CUSTOM_NOTE: &str =
    "Set what it may do to sandboxed: the setup agent picked no preset Sage knows.";

const SCHEDULE_NOTE: &str =
    "Left out the schedule: Sage could not use it. It runs only when you talk to it; set a schedule in the editor.";

fn add_notes(notes: &mut String, added: Vec<String>) {
    for line in added {
        if !notes.trim().is_empty() {
            notes.push('\n');
        }
        notes.push_str(&line);
    }
}

/// A proposal repaired so Dots can make it (spec §6.2): the name passes the name rule, Library
/// items the Library does not have are dropped, extras are known and their servers used, the
/// schedule is valid, the instructions are cut at `MAX_INSTRUCTIONS_CHARS`, and the folder
/// exists. Each change is named in `notes`. A name that clashes with an existing dot (ignoring
/// case) gets " 2", " 3", ... with `rename_on_clash`, and is `Conflict` without it. Blank
/// instructions are `Invalid`.
pub fn check_proposal(
    mut p: SetupProposal,
    ctx: &SetupContext,
    rename_on_clash: bool,
) -> Result<SetupProposal> {
    let mut notes = Vec::new();

    if p.instructions.trim().is_empty() {
        return invalid("Write what the sol should do.");
    }

    // The preset: `custom` is a person's own rules in the editor; a proposal is a preset plus
    // extras, so it holds the same rule here as in the agent's answer.
    if p.preset == Preset::Custom {
        p.preset = Preset::Sandboxed;
        notes.push(CUSTOM_NOTE.to_string());
    }

    // The name.
    let repaired = repair_name(&p.name);
    if repaired != p.name {
        notes.push(format!(
            "Named it {repaired:?}: a sol name uses only letters, digits, spaces, '-', '_' or '.'."
        ));
        p.name = repaired;
    }
    let clashes = |name: &str| ctx.dot_names.iter().any(|d| d.eq_ignore_ascii_case(name));
    if clashes(&p.name) {
        if !rename_on_clash {
            return Err(Error::Conflict(format!(
                "A sol named {} already exists",
                p.name
            )));
        }
        let renamed = (2u32..)
            .map(|n| {
                let suffix = format!(" {n}");
                let base: String = p.name.chars().take(MAX_NAME_CHARS - suffix.len()).collect();
                format!("{}{suffix}", base.trim_end())
            })
            .find(|name| !clashes(name))
            .expect("some number is free");
        notes.push(format!(
            "Named it {renamed:?}: you already have a sol named {:?}.",
            p.name
        ));
        p.name = renamed;
    }

    // The instructions.
    if p.instructions.chars().count() > MAX_INSTRUCTIONS_CHARS {
        p.instructions = p
            .instructions
            .chars()
            .take(MAX_INSTRUCTIONS_CHARS)
            .collect();
        notes.push(format!(
            "Cut the instructions at {MAX_INSTRUCTIONS_CHARS} characters; check their end."
        ));
    }

    // The folder.
    if let Some(folder) = p.folder.take() {
        let folder = folder.trim().to_string();
        if !folder.is_empty() {
            // Only a full local path is looked at on disk: a relative path would be read
            // against Dots' own folder, and a `\` path may reach another PC or a device.
            let path = Path::new(&folder);
            let local =
                path.is_absolute() && !folder.starts_with("\\\\") && !folder.starts_with("//");
            if !local {
                notes.push(format!(
                    "{folder} is not a full path to a folder on this PC, so Sage makes a new folder for this sol."
                ));
            } else if path.is_dir() {
                p.folder = Some(folder);
            } else {
                notes.push(format!(
                    "{folder} is not a folder on this PC, so Sage makes a new folder for this sol."
                ));
            }
        }
    }

    // The schedule.
    // As the store checks it on save: valid, and with a slot still to come.
    let now = Local::now();
    let unusable =
        |s: &Schedule| s.validate().is_err() || s.check_runs_after(&now, &Local).is_err();
    if p.schedule.as_ref().is_some_and(unusable) {
        p.schedule = None;
        notes.push(SCHEDULE_NOTE.to_string());
    }

    // The Library items.
    let lib = &ctx.library;
    let skills: Vec<&String> = lib.skills.iter().map(|(name, _)| name).collect();
    let mcp: Vec<&String> = lib.mcp_servers.iter().collect();
    let plugins: Vec<&String> = lib.plugins.iter().collect();
    let context: Vec<&String> = lib.context.iter().collect();
    let none: Vec<&String> = Vec::new();
    for (label, names, known) in [
        ("skill", &mut p.uses.skills, &skills),
        ("MCP server", &mut p.uses.mcp_servers, &mcp),
        ("plugin", &mut p.uses.plugins, &plugins),
        ("context collection", &mut p.uses.context, &context),
        ("app", &mut p.uses.apps, &none),
    ] {
        let mut seen = BTreeSet::new();
        names.retain(|name| {
            if !known.contains(&name) {
                notes.push(format!(
                    "Left out {label} {name}: it is not in your Library."
                ));
                return false;
            }
            seen.insert(name.clone())
        });
    }

    // The extras.
    let mut seen = BTreeSet::new();
    let servers = &p.uses.mcp_servers;
    p.extras.retain(|extra| {
        let extra = extra.as_str();
        let keep = if extra == "web" {
            true
        } else if let Some(server) = extra
            .strip_prefix("mcp:")
            .filter(|s| check_name(LibraryKind::McpServer, s).is_ok())
        {
            if servers.iter().any(|s| s == server) {
                true
            } else {
                notes.push(format!(
                    "Left out the extra {extra}: it needs {server} in the Library items it uses."
                ));
                false
            }
        } else {
            notes.push(format!(
                "Left out the extra {extra}: Sage does not know it."
            ));
            false
        };
        keep && seen.insert(extra.to_string())
    });

    add_notes(&mut p.notes, notes);
    Ok(p)
}

/// The name with what the name rule does not allow taken out: characters other than ASCII
/// letters, digits, spaces, '-', '_' and '.' dropped, runs of spaces made one, trimmed, and cut
/// to 64 characters. Nothing left becomes "New dot".
fn repair_name(name: &str) -> String {
    let kept: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.'))
        .collect();
    let joined = kept
        .split(' ')
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let cut: String = joined.chars().take(MAX_NAME_CHARS).collect();
    let cut = cut.trim();
    if cut.is_empty() {
        FALLBACK_NAME.to_string()
    } else {
        cut.to_string()
    }
}

/// The policy of a proposal (spec §6.3): the pure preset with no extras; with any, the
/// preset's rules plus an allow rule per extra, and the preset field `custom`. `web` allows
/// `WebFetch` and `WebSearch`, `mcp:<server>` allows `mcp__<server>__*`; an extra Dots does not
/// know adds nothing, and nothing is trimmed: `extras` must be a proposal's that passed
/// `check_proposal`. Deny rules of the preset still win.
pub fn setup_policy(preset: Preset, extras: &[String]) -> Policy {
    let mut policy = Policy::preset(preset);
    let mut added = Vec::new();
    for extra in extras {
        let extra = extra.as_str();
        if extra == "web" {
            added.push(Rule::new("WebFetch", Action::Allow));
            added.push(Rule::new("WebSearch", Action::Allow));
        } else if let Some(server) = extra
            .strip_prefix("mcp:")
            .filter(|s| check_name(LibraryKind::McpServer, s).is_ok())
        {
            added.push(Rule::new(&format!("mcp__{server}__*"), Action::Allow));
        }
    }
    if !added.is_empty() {
        for rule in added {
            if !policy.rules.contains(&rule) {
                policy.rules.push(rule);
            }
        }
        policy.preset = Preset::Custom;
    }
    policy
}

/// The model the setup agent asks.
pub const SETUP_MODEL: &str = DEFAULT_MODEL;

/// What `Runtime::setup_apply` made: the dot, its first chat run or why that did not start (spec
/// §9: the dot stays when its first run cannot start), and the notes of its own check. A blank
/// first message starts no chat.
#[derive(Debug, Clone, Serialize)]
pub struct SetupApplied {
    pub dot: Dot,
    pub run_id: Option<String>,
    pub run_error: Option<String>,
    /// What the check at apply time changed in the proposal, one line each; empty when nothing.
    pub notes: String,
}

/// The setup requests being answered, at most one per setup view (spec §9). The same runner as
/// the skill creator's drafts (`SkillDrafts`): one `claude -p` call with no tools, in an empty
/// folder of its own, ended by `cancel`, a newer request of the same view, or the time limit.
/// It is the person's own action, so it never waits in the run queue nor for a usage-limit
/// hold.
pub struct SetupDrafts {
    runner: OnceRunner,
}

impl SetupDrafts {
    /// `program` is the CLI the runtime found, `None` when there is none. Each request's CLI
    /// starts in a new empty folder in `dir`.
    pub fn new(
        program: Option<PathBuf>,
        env: Vec<(String, String)>,
        dir: PathBuf,
        limit: Duration,
        shutdown: CancellationToken,
    ) -> SetupDrafts {
        SetupDrafts {
            runner: OnceRunner::new(program, env, dir, limit, shutdown),
        }
    }

    /// Asks the agent for the setup `id` (`setup_ask`) and returns its checked answer. A
    /// request the same setup made before is cancelled first, also when this one is refused. A
    /// request `setup_prompt` refuses is `Invalid` before the CLI starts; the CLI's error,
    /// `cancel` and the time limit fail it with text a person reads.
    pub async fn ask(
        &self,
        id: &str,
        ctx: &SetupContext,
        req: &SetupRequest,
    ) -> Result<SetupAnswer> {
        self.ask_with_usage(id, ctx, req).await.0
    }

    /// `ask`, with the usage the CLI reported for the request (`None` when it made none or
    /// reported none), so a caller that counts its requests can record it.
    pub async fn ask_with_usage(
        &self,
        id: &str,
        ctx: &SetupContext,
        req: &SetupRequest,
    ) -> (Result<SetupAnswer>, Option<OnceUsage>) {
        let prompt = match setup_prompt(ctx, req) {
            Ok(prompt) => prompt,
            Err(e) => {
                self.runner.cancel(id);
                return (Err(e), None);
            }
        };
        let asked = self
            .runner
            .run_with_usage(id, SETUP_INSTRUCTIONS, SETUP_MODEL, &prompt)
            .await;
        let usage = asked.as_ref().ok().and_then(|a| a.usage);
        let answer = match asked {
            Ok(answer) => check_setup_answer(&answer.text, ctx, req),
            Err(OnceFailure::NoCli) => Err(Error::Other(NO_CLI.into())),
            Err(OnceFailure::Cancelled) => Err(Error::Other("The setup was cancelled.".into())),
            Err(OnceFailure::TimedOut) => Err(Error::Other(
                "Claude took too long to set up the sol.".into(),
            )),
            Err(OnceFailure::Failed(e)) => Err(Error::Other(format!(
                "Claude could not set up the sol: {e}"
            ))),
        };
        (answer, usage)
    }

    /// Cancels the setup's request, if one is being answered (`setup_cancel`).
    pub fn cancel(&self, id: &str) {
        self.runner.cancel(id);
    }
}
