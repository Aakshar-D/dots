//! A maker lists, changes and deletes the dots it made (dots-make-dots design section 9), for
//! the tools both engines offer it. A tool names a made dot by its name, since a model knows
//! names and not ids, and only a dot whose recorded maker is the calling run's dot is found:
//! any other dot is as if it did not exist.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};

use super::jobs::{deleted_message, JobEnd, JobStatus, MakingJob, JOB_WAITING, JOB_WORKING};
use super::log::{MakingEventKind, NewMakingEvent};
use super::reach::{
    folded, grants, held_extras, preset_label, rank, reach_change, real_path, still_below_maker,
    time_label, Extra, Reach, Wanted, WantedPermissions,
};
use super::requests::{MakeRequest, NewMakeRequest, RequestKind, SOMETHING_GONE};
use super::service::{check_folder_text, checked_purpose, may_make, Makers};
use crate::library::LibraryKind;
use crate::model::{Dot, DotSpec, Run};
use crate::policy::Policy;
use crate::schedule::{Rule, Schedule, Until};
use crate::vault::Consumer;
use crate::{Error, Result};

/// What a `change_dot` that names nothing is told.
pub const NOTHING_TO_CHANGE: &str = "Name at least one thing to change.";

/// One made dot as `list_made_dots` shows it (design section 9).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MadeDotLine {
    pub name: String,
    /// The maker's one line on what the dot is for.
    pub purpose: String,
    /// Its schedule in plain words (`schedule_words`); `None` without one.
    pub schedule: Option<String>,
    pub enabled: bool,
    /// A run of it is queued, running or awaiting approval.
    pub running: bool,
    /// How its last ended run ended, in the store's word for the status, and when:
    /// `"failed at 2026-11-11T09:00:00.000Z"`. `None` when no run of it has ended.
    pub last_run: Option<String>,
    /// A change this maker asked for it waits for the person.
    pub waiting_change: bool,
    /// What it does for the conversation that lists it (design section 8): `JOB_WORKING` while
    /// a job given in this chat topic of the maker has not ended, `JOB_WAITING` while that job
    /// waits for the person's answer; `None` otherwise.
    #[serde(default)]
    pub job: Option<String>,
}

/// What a maker's run asks for with `change_dot` (design section 9). In JSON it is one flat
/// object: `dot`, `purpose` and the fields of `Wanted`, where `name` renames the dot. Only the
/// fields named change. A field none of them has is refused, so a call can not name "Can make
/// dots", the account or the engine.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChangeDot {
    /// The made dot, by its current name.
    pub dot: String,
    /// A new one-line purpose, if it changes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    #[serde(flatten)]
    pub wanted: Wanted,
}

// By hand, as for `MakeDot`: serde's `flatten` and `deny_unknown_fields` do not combine.
impl<'de> Deserialize<'de> for ChangeDot {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        use serde::de::Error as _;
        let mut fields = serde_json::Map::<String, Value>::deserialize(d)?;
        let dot = match fields.remove("dot") {
            Some(Value::String(dot)) => dot,
            Some(Value::Null) | None => return Err(D::Error::missing_field("dot")),
            Some(other) => return Err(D::Error::custom(format!("dot is text, not {other}"))),
        };
        // A model writes `null` for a field it leaves out.
        let purpose = match fields.remove("purpose") {
            Some(Value::String(purpose)) => Some(purpose),
            Some(Value::Null) | None => None,
            Some(other) => return Err(D::Error::custom(format!("purpose is text, not {other}"))),
        };
        let wanted = serde_json::from_value(Value::Object(fields)).map_err(D::Error::custom)?;
        Ok(ChangeDot {
            dot,
            purpose,
            wanted,
        })
    }
}

/// What came of a `change_dot`.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum ChangeOutcome {
    /// Changed at once. `what` names the settings that changed, in plain words; empty when the
    /// dot was already as asked and nothing was written.
    Changed { dot: Dot, what: Vec<String> },
    /// Stored for the person; nothing was changed: the change gives the dot more than the
    /// maker has (`request.extras`), or, with `held`, the dot holds more than the maker has and
    /// the change is more than taking things away, so the person decides on it (`extras` names
    /// what it holds too). `maker_name` is the maker's name, for the words of the extras.
    Waiting {
        request: MakeRequest,
        maker_name: String,
        held: bool,
    },
}

impl ChangeOutcome {
    /// What the tool tells the maker's run (design section 9), the same on both engines.
    pub fn tool_text(&self) -> String {
        match self {
            Self::Changed { dot, what } if what.is_empty() => format!(
                "\"{}\" is already as you asked. Nothing was changed.",
                dot.spec.name
            ),
            Self::Changed { dot, what } => {
                format!("Changed \"{}\": {}.", dot.spec.name, what.join(", "))
            }
            Self::Waiting {
                request,
                maker_name,
                held,
            } => {
                let extras: Vec<String> =
                    request.extras.iter().map(|e| e.line(maker_name)).collect();
                let why = if *held {
                    "it has, or the change gives it, more than you have"
                } else {
                    "the change gives it more than you have"
                };
                format!(
                    "Not changed yet: \"{}\" needs the person's say-so, because {why} ({}). The \
                     request is waiting for them. Do not wait for it and do not ask again: carry \
                     on, and you will be woken with their answer.",
                    request.name,
                    extras.join("; ")
                )
            }
        }
    }
}

/// `n` of a unit: "minute", "2 minutes".
fn counted(n: u32, unit: &str) -> String {
    if n == 1 {
        unit.to_string()
    } else {
        format!("{n} {unit}s")
    }
}

/// A schedule in plain words, for a tool result and the maker's prompt: "every 2 hours", "daily
/// at 09:00", "weekly on Mon, Thu at 09:00", "once at 2026-11-20 09:00", "cron 0 0 9 * * *", and
/// its end when it has one (", 10 times" or ", until 2026-12-01 09:00"). Times are local, as the
/// schedule holds them. The core had no such wording before this one; the UI words schedules
/// itself.
pub fn schedule_words(schedule: &Schedule) -> String {
    let mut words = match &schedule.rule {
        Rule::Once { at } => format!("once at {}", at.format("%Y-%m-%d %H:%M")),
        Rule::Every { minutes, .. } => {
            let every = match *minutes {
                m if m % (24 * 60) == 0 => counted(m / (24 * 60), "day"),
                m if m % 60 == 0 => counted(m / 60, "hour"),
                m => counted(m, "minute"),
            };
            format!("every {every}")
        }
        Rule::Daily { time } => format!("daily at {}", time.format("%H:%M")),
        Rule::Weekly { days, time } => {
            let days: Vec<String> = days.iter().map(|day| day.to_string()).collect();
            format!("weekly on {} at {}", days.join(", "), time.format("%H:%M"))
        }
        Rule::Cron { expr } => format!("cron {expr}"),
    };
    match &schedule.until {
        Some(Until::Count(1)) => words.push_str(", 1 time"),
        Some(Until::Count(n)) => words.push_str(&format!(", {n} times")),
        Some(Until::Date(date)) => {
            words.push_str(&format!(", until {}", date.format("%Y-%m-%d %H:%M")))
        }
        None => {}
    }
    words
}

/// What `list_made_dots` tells the maker's run (design section 9): one line per made dot, then
/// one line per request for a new dot that waits for the person, or that the maker has made no
/// dots yet. `maker` is the maker's name; of `waiting_makes`, the maker's requests the person
/// has not answered, only those for a new dot are named, since a waiting change shows on its
/// dot's line.
pub fn made_dots_text(maker: &str, lines: &[MadeDotLine], waiting_makes: &[MakeRequest]) -> String {
    let mut text: Vec<String> = Vec::new();
    if lines.is_empty() {
        text.push(format!("{maker} has made no dots yet."));
    }
    for line in lines {
        let mut parts = vec![line.purpose.clone()];
        parts.push(match &line.schedule {
            Some(schedule) => format!("schedule: {schedule}"),
            None => "no schedule".to_string(),
        });
        if !line.enabled {
            parts.push("off".to_string());
        }
        if line.running {
            parts.push("running".to_string());
        }
        parts.push(match &line.last_run {
            Some(last) => format!("last run: {last}"),
            None => "no run has ended yet".to_string(),
        });
        if line.waiting_change {
            parts.push("a change is waiting for the person".to_string());
        }
        parts.extend(line.job.clone());
        text.push(format!("\"{}\": {}", line.name, parts.join("; ")));
    }
    for request in waiting_makes {
        if request.kind == RequestKind::Make {
            text.push(format!("Waiting for the person: \"{}\"", request.name));
        }
    }
    text.join("\n")
}

/// Whether `wanted` names nothing. A blank folder is as not named (`Wanted::folder`).
fn names_nothing(wanted: &Wanted) -> bool {
    let mut named = wanted.clone();
    if named.folder.as_deref().is_some_and(|f| f.trim().is_empty()) {
        named.folder = None;
    }
    named == Wanted::default()
}

/// The settings that differ between a made dot before a change and after it, in plain words,
/// for the tool result and the "changed" card (design sections 9 and 11).
fn what_changed(before: &Dot, after: &Dot) -> Vec<String> {
    let (old, new) = (&before.spec, &after.spec);
    let purpose = |dot: &Dot| dot.made.as_ref().map(|made| made.purpose.clone());
    let mut what = Vec::new();
    if old.name != new.name {
        what.push(format!("name (was {})", old.name));
    }
    let named = [
        (purpose(before) != purpose(after), "purpose"),
        (old.instructions != new.instructions, "instructions"),
        (old.model != new.model, "model"),
        (
            old.workdir != new.workdir || old.workspace_mode != new.workspace_mode,
            "folder",
        ),
        (old.policy != new.policy, "permissions"),
        (old.uses != new.uses, "Library items"),
        (old.variables != new.variables, "variables"),
        (old.browser != new.browser, "browser"),
        (
            old.use_user_settings != new.use_user_settings,
            "Claude settings",
        ),
        (old.max_turns != new.max_turns, "turn limit"),
        (old.timeout_secs != new.timeout_secs, "time limit"),
        (old.schedule != new.schedule, "schedule"),
    ];
    what.extend(
        named
            .into_iter()
            .filter(|(changed, _)| *changed)
            .map(|(_, name)| name.to_string()),
    );
    what
}

/// Whether `after` only takes from `before`, the made dot as it is (design section 9): Library
/// items or variables removed, the schedule cleared, a limit lowered, a plain preset no
/// stronger than the dot's own plain preset, the browser or the person's Claude settings
/// turned off. Any other difference, the name, the instructions, the model, the folder, a
/// schedule set or anything added, is no narrowing.
pub(super) fn narrows(before: &DotSpec, after: &DotSpec) -> bool {
    let mut rest = after.clone();
    for kind in LibraryKind::ALL {
        if !after
            .uses
            .names(*kind)
            .iter()
            .all(|name| before.uses.names(*kind).contains(name))
        {
            return false;
        }
    }
    rest.uses = before.uses.clone();
    let kept = after.variables.0.iter().all(|(consumer, names)| {
        names
            .iter()
            .all(|name| grants(&before.variables, consumer, name))
    });
    if !kept {
        return false;
    }
    rest.variables = before.variables.clone();
    if after.schedule != before.schedule && after.schedule.is_some() {
        return false;
    }
    rest.schedule = before.schedule.clone();
    if after.max_turns > before.max_turns || after.timeout_secs > before.timeout_secs {
        return false;
    }
    rest.max_turns = before.max_turns;
    rest.timeout_secs = before.timeout_secs;
    if after.policy != before.policy {
        let preset = after.policy.preset;
        let plain = rank(preset).is_some() && after.policy == Policy::preset(preset);
        if !plain || rank(before.policy.preset).is_none_or(|has| rank(preset) > Some(has)) {
            return false;
        }
    }
    rest.policy = before.policy.clone();
    if (after.browser && !before.browser) || (after.use_user_settings && !before.use_user_settings)
    {
        return false;
    }
    rest.browser = before.browser;
    rest.use_user_settings = before.use_user_settings;
    rest == *before
}

/// `on` as the card says it.
fn on_off(on: bool) -> &'static str {
    if on {
        "on"
    } else {
        "off"
    }
}

/// The variable `name` for `consumer` (a key of `Grants`), in plain words.
fn variable_words(consumer: &str, name: &str) -> String {
    match Consumer::parse(consumer) {
        Ok(Consumer::Mcp(server)) => format!("the variable {name} for the server '{server}'"),
        Ok(Consumer::Shell) => format!("the variable {name} for the shell"),
        Err(_) => format!("the variable {name} for {consumer}"),
    }
}

/// Every setting `ask` names, one short line each, for the ask card of a change (design
/// section 9): what Allow saves is all on the card, not only what is beyond reach. `before` is
/// the made dot and `after` the dot with the change on it; the instructions are in the card in
/// full (`MakeRequestCard::instructions`), so their line only says they are new.
pub(super) fn change_lines(
    ask: &ChangeDot,
    before: &DotSpec,
    after: &DotSpec,
    maker: &str,
) -> Vec<String> {
    let w = &ask.wanted;
    let mut lines = Vec::new();
    if w.name.is_some() {
        lines.push(format!("Name: {}", after.name));
    }
    if let Some(purpose) = &ask.purpose {
        lines.push(format!("Purpose: {purpose}"));
    }
    if w.instructions.is_some() {
        lines.push("New instructions".to_string());
    }
    if w.model.is_some() {
        lines.push(format!("Model: {}", after.model));
    }
    if w.folder.as_deref().is_some_and(|f| !f.trim().is_empty()) {
        lines.push(format!("Folder: {}", after.workdir));
    }
    match w.permissions {
        Some(WantedPermissions::Maker) => lines.push(format!("Permissions: the same as {maker}'s")),
        Some(WantedPermissions::Preset(preset)) => {
            lines.push(format!("Permissions: {}", preset_label(preset)))
        }
        None => {}
    }
    if w.uses.is_some() {
        for kind in LibraryKind::ALL {
            let (old, new) = (before.uses.names(*kind), after.uses.names(*kind));
            for name in new.iter().filter(|name| !old.contains(name)) {
                lines.push(format!("Adds the {} '{name}'", kind.label()));
            }
            for name in old.iter().filter(|name| !new.contains(name)) {
                lines.push(format!("Removes the {} '{name}'", kind.label()));
            }
        }
    }
    if w.variables.is_some() {
        for (consumer, names) in &after.variables.0 {
            for name in names
                .iter()
                .filter(|n| !grants(&before.variables, consumer, n))
            {
                lines.push(format!("Adds {}", variable_words(consumer, name)));
            }
        }
        for (consumer, names) in &before.variables.0 {
            for name in names
                .iter()
                .filter(|n| !grants(&after.variables, consumer, n))
            {
                lines.push(format!("Removes {}", variable_words(consumer, name)));
            }
        }
    }
    if w.browser.is_some() {
        lines.push(format!("Browser: {}", on_off(after.browser)));
    }
    if w.use_user_settings.is_some() {
        lines.push(format!(
            "Your Claude settings: {}",
            on_off(after.use_user_settings)
        ));
    }
    if w.max_turns.is_some() {
        lines.push(format!("Turn limit: {}", after.max_turns));
    }
    if w.timeout_secs.is_some() {
        lines.push(format!("Time limit: {}", time_label(after.timeout_secs)));
    }
    match &w.schedule {
        Some(Some(schedule)) => lines.push(format!("Schedule: {}", schedule_words(schedule))),
        Some(None) => lines.push("No schedule".to_string()),
        None => {}
    }
    lines
}

/// Whether the extra `now` gives the dot the thing the person saw as `seen` on the ask card.
/// The maker's own setting a line names beside it may differ: the thing given is the same.
pub(super) fn same_thing(now: &Extra, seen: &Extra) -> bool {
    match (now, seen) {
        (Extra::Preset { preset: a, .. }, Extra::Preset { preset: b, .. }) => a == b,
        (Extra::Folder { path: a, .. }, Extra::Folder { path: b, .. }) => a == b,
        (Extra::MaxTurns { wanted: a, .. }, Extra::MaxTurns { wanted: b, .. }) => a == b,
        (Extra::Timeout { wanted_secs: a, .. }, Extra::Timeout { wanted_secs: b, .. }) => a == b,
        (now, seen) => now == seen,
    }
}

/// Makes `folder`, below the folder of `maker`, and the folders above it that are missing.
/// Returns those it made, deepest last, for `remove_folders` when the change is not saved. Its
/// real path is checked again once it exists: a link or junction planted in it since the reach
/// check would lead it outside the maker's folder, and then what was made is removed again.
async fn make_folders(maker: &Dot, folder: Option<&Path>) -> Result<Vec<PathBuf>> {
    let mut made: Vec<PathBuf> = Vec::new();
    let Some(folder) = folder else {
        return Ok(made);
    };
    let mut missing = folder;
    while !missing.exists() {
        made.push(missing.to_path_buf());
        match missing.parent() {
            Some(parent) => missing = parent,
            None => break,
        }
    }
    made.reverse();
    tokio::fs::create_dir_all(folder).await?;
    if !still_below_maker(maker, folder) {
        remove_folders(&made).await;
        return Err(Error::Invalid(format!(
            "The folder {} no longer lies inside {}'s folder, so the dot was not changed.",
            folder.display(),
            maker.spec.name
        )));
    }
    Ok(made)
}

/// Removes the folders `make_folders` made. Only an empty folder is removed, so nothing put
/// there in between is lost.
async fn remove_folders(made: &[PathBuf]) {
    for folder in made.iter().rev() {
        let _ = tokio::fs::remove_dir(folder).await;
    }
}

/// A change request held against the made dot as it is now (`Makers::change_stands`).
pub(super) struct Standing {
    /// The made dot as it is now.
    pub(super) dot: Dot,
    /// The change as asked, put on the dot as it is now: `spec` is what Allow saves and
    /// `without` what "without" saves.
    pub(super) reach: Reach,
    /// The new purpose, when the change names one.
    pub(super) purpose: Option<String>,
    /// What the card shows now: `reach.extras`, and, with `held`, what the dot holds beyond
    /// the maker's reach (`Makers::change_extras`).
    pub(super) extras: Vec<Extra>,
    /// The dot holds more than the maker has now, and the change is more than taking things
    /// away: the person decides on the whole of it, so there is nothing to save without it.
    pub(super) held: bool,
    /// The change would now give the dot something beyond the maker's reach that was not on
    /// the card the person saw: the maker's own settings were narrowed since it asked.
    pub(super) beyond_what_was_seen: bool,
}

/// A change judged against the maker and the made dot (`Makers::judge_change`).
struct Judged {
    reach: Reach,
    /// What asks the person (`Makers::change_extras`).
    extras: Vec<Extra>,
    held: bool,
}

impl Standing {
    /// Whether "without" would change anything of the dot: when it would not, everything
    /// asked is beyond reach and there is nothing to change without it.
    pub(super) fn without_changes_something(&self) -> bool {
        let purpose_now = self.dot.made.as_ref().map(|made| made.purpose.as_str());
        if self.held {
            return false;
        }
        self.reach.without != self.dot.spec
            || self
                .purpose
                .as_deref()
                .is_some_and(|p| Some(p) != purpose_now)
    }

    /// The folder to make before `without` is saved: the one `spec` needs, when `without`
    /// works in the same folder.
    pub(super) fn without_folder(&self) -> Option<PathBuf> {
        let same = self.reach.without.workdir == self.reach.spec.workdir;
        self.reach.create_folder.clone().filter(|_| same)
    }
}

/// Whether a change request can still be allowed.
#[allow(clippy::large_enum_variant)]
pub(super) enum Stands {
    Yes(Standing),
    /// What it names that is no longer there, in one sentence for the person.
    Gone(String),
}

/// What a tool that names a dot the maker did not make is told. The same sentence whether no
/// dot has the name or the dot is the person's or another maker's: other dots are not revealed.
fn not_made(maker: &Dot, name: &str) -> Error {
    Error::Invalid(format!(
        "{} did not make a dot named \"{}\". list_made_dots shows the dots it made.",
        maker.spec.name,
        name.trim()
    ))
}

/// What a maker is woken with when the made dot named `name`, for which a change it asked waits
/// for the person, is deleted (design section 9): the request went with the dot.
pub fn change_went_message(name: &str) -> String {
    format!(
        "\"{name}\" was deleted, so the change you asked for it went with it: the person will \
         not answer it. Do not ask for it again."
    )
}

/// Adds `message` to what the topic `topic` of the maker `maker` is woken with, so each topic
/// is woken once.
fn push_wake<'a>(
    wakes: &mut Vec<(&'a str, &'a str, String)>,
    maker: &'a str,
    topic: &'a str,
    message: String,
) {
    match wakes
        .iter_mut()
        .find(|(m, t, _)| (*m, *t) == (maker, topic))
    {
        Some((_, _, text)) => {
            text.push_str("\n\n");
            text.push_str(&message);
        }
        None => wakes.push((maker, topic, message)),
    }
}

impl Makers {
    /// The run `run_id` and its dot, the maker, when the run may use the tools for making dots
    /// (`may_make`).
    pub(super) async fn caller(&self, run_id: &str) -> Result<(Run, Dot)> {
        let store = &self.parts.store;
        let run = store.get_run(run_id).await?;
        let maker = store.get_dot(&run.dot_id).await?;
        may_make(&maker, &run)?;
        Ok((run, maker))
    }

    /// The dot named `name`, in any letter case, that `maker` made (design section 9).
    pub(super) async fn made_dot(&self, maker: &Dot, name: &str) -> Result<Dot> {
        self.parts
            .store
            .made_dot_named(&maker.id, name.trim())
            .await?
            .ok_or_else(|| not_made(maker, name))
    }

    /// The dots the maker of the run `run_id` made that exist now, by name, each with its
    /// state (design section 9). Refused as `make_dot` is for a dot that may not make dots, a
    /// workflow step's run and a run that has ended.
    pub async fn list_made_dots(&self, run_id: &str) -> Result<Vec<MadeDotLine>> {
        let store = &self.parts.store;
        let (run, maker) = self.caller(run_id).await?;
        // One marked gone still stands before the person, so it still waits.
        let waiting = store.list_make_requests(Some(&maker.id), true).await?;
        let mut lines = Vec::new();
        for dot in store.list_made_dots(&maker.id).await? {
            // The job this conversation gave it last, while it has not ended.
            let job = store.latest_making_job(&run.root_run_id, &dot.id).await?;
            let job = match job.map(|job| job.status) {
                Some(JobStatus::Working) => Some(JOB_WORKING.to_string()),
                Some(JobStatus::WaitingAnswer) => Some(JOB_WAITING.to_string()),
                Some(JobStatus::Ended) | None => None,
            };
            let last_run = store.last_ended_run(&dot.id).await?.map(|run| {
                let ended = run.ended_at.unwrap_or_default();
                format!("{} at {ended}", run.status.as_str())
            });
            lines.push(MadeDotLine {
                schedule: dot.spec.schedule.as_ref().map(schedule_words),
                enabled: dot.spec.enabled,
                running: store.latest_run(&dot.id, true).await?.is_some(),
                last_run,
                waiting_change: waiting.iter().any(|request| {
                    request.kind == RequestKind::Change
                        && request.dot_id.as_deref() == Some(dot.id.as_str())
                }),
                job,
                purpose: dot.made.map(|made| made.purpose).unwrap_or_default(),
                name: dot.spec.name,
            });
        }
        Ok(lines)
    }

    /// A maker's run asks for a change to a dot it made (design section 9). `run_id` is the
    /// run that calls; its dot is the maker.
    ///
    /// Only the settings `ask` names change; the rest stays, including what the person edited
    /// and the extras they allowed. `Changed` when everything named is within the maker's
    /// reach now: the dot is saved through `DotAdmin::update`, with its grants exactly as the
    /// change leaves them, and the topic's log has a `changed` entry. `Waiting` when the change
    /// gives the dot more than the maker has: nothing was changed, a request of the kind
    /// `change` is stored for the person with the ask itself, and the topic's log has an
    /// `asked` entry. It counts toward the `MAX_WAITING_REQUESTS` of the maker, and only one
    /// change may wait per made dot. The limits of section 10 do not apply to a change.
    ///
    /// A change of `uses` that does not name `variables` drops the grants of the MCP servers
    /// the dot no longer uses, as a person's save does; it never adds a grant.
    ///
    /// An error, with a sentence the model can act on: what `make_dot` refuses a run for, a
    /// dot the maker did not make, nothing named (`Invalid`), a dot that would not be valid
    /// (`reach_change`), a Library item, variable or consumer that does not exist
    /// (`check_exists`), a new name another dot has (`Conflict`), a change of the folder
    /// while the dot has a run going (`Conflict`, the store's own sentence), and a refusal of
    /// this PC's role (`NotAllowed`).
    pub async fn change_dot(&self, run_id: &str, mut ask: ChangeDot) -> Result<ChangeOutcome> {
        let p = &self.parts;
        let (_, maker) = self.caller(run_id).await?;
        ask.purpose = ask.purpose.as_deref().map(checked_purpose).transpose()?;
        check_folder_text(&ask.wanted)?;
        let made = self.made_dot(&maker, &ask.dot).await?;
        if ask.purpose.is_none() && names_nothing(&ask.wanted) {
            return Err(Error::Invalid(NOTHING_TO_CHANGE.into()));
        }
        // Before the lock, which holds every maker's makes: the reach check reads the disk and
        // `check_exists` the vault.
        let mut judged = self.judge_change(&maker, &made, &ask).await?;
        // With the makes and the answers: the dot is read and saved with no answer to a
        // request for it in between.
        let _one_at_a_time = p.one_at_a_time.lock().await;
        // Judged on the run, the maker and the dot as they are once the lock is held: the run
        // may have ended, and the switch, the maker or the dot changed, while this call waited.
        let (run, maker_now) = self.caller(run_id).await?;
        let made_now = match p.store.get_dot(&made.id).await {
            Ok(dot) => dot,
            Err(Error::NotFound(_)) => return Err(not_made(&maker_now, &ask.dot)),
            Err(e) => return Err(e),
        };
        let makers = made_now.made.as_ref().and_then(|m| m.by_dot_id.as_deref());
        if makers != Some(maker_now.id.as_str()) {
            return Err(not_made(&maker_now, &ask.dot));
        }
        if maker_now.spec != maker.spec || made_now.spec != made.spec || made_now.made != made.made
        {
            judged = self.judge_change(&maker_now, &made_now, &ask).await?;
        }
        let (maker, made) = (maker_now, made_now);
        let Judged {
            reach,
            extras,
            held,
        } = judged;
        self.check_new_name_free(&made, &reach.spec.name).await?;
        if extras.is_empty() {
            let (dot, what) = self
                .apply_change(
                    &maker,
                    &made,
                    reach.spec,
                    ask.purpose.as_deref(),
                    reach.create_folder,
                    &run.id,
                    &run.root_run_id,
                )
                .await?;
            return Ok(ChangeOutcome::Changed { dot, what });
        }
        let waiting = p.store.list_make_requests(Some(&maker.id), true).await?;
        let for_this_dot = |request: &MakeRequest| {
            request.kind == RequestKind::Change
                && request.dot_id.as_deref() == Some(made.id.as_str())
        };
        if waiting.iter().any(for_this_dot) {
            return Err(Error::Invalid(format!(
                "A change to \"{}\" is already waiting for the person.",
                made.spec.name
            )));
        }
        self.check_may_ask(&maker).await?;
        let purpose_now = made.made.as_ref().map(|m| m.purpose.as_str());
        let create_folder = reach
            .create_folder
            .as_ref()
            .map(|folder| folder.to_string_lossy().into_owned());
        let request = p
            .store
            .add_make_request(&NewMakeRequest {
                maker_dot_id: &maker.id,
                run_id: &run.id,
                root_run_id: &run.root_run_id,
                kind: RequestKind::Change,
                dot_id: Some(&made.id),
                purpose: ask.purpose.as_deref().or(purpose_now).unwrap_or_default(),
                spec: &reach.spec,
                without: &reach.without,
                extras: &extras,
                over: None,
                create_folder: create_folder.as_deref(),
                first_job: None,
                ask: Some(&ask),
                make_ask: None,
            })
            .await?;
        self.log_asked(&request, &maker.spec.name, Some(&made.id), &made.spec.name)
            .await;
        Ok(ChangeOutcome::Waiting {
            request,
            maker_name: maker.spec.name,
            held,
        })
    }

    /// `ask` put on `made` against `maker`: what saving it would save, and what asks the
    /// person (`change_extras`). Refused as `change_dot` says for a dot that would not be valid
    /// and for what does not exist (`check_exists`): no request is kept for what can never be
    /// allowed.
    async fn judge_change(&self, maker: &Dot, made: &Dot, ask: &ChangeDot) -> Result<Judged> {
        let reach = self.reach_of_change(maker, made, &ask.wanted).await?;
        self.check_exists(maker, &reach.spec).await?;
        let (extras, held) = self
            .change_extras(maker, made, &reach, ask.purpose.as_deref())
            .await?;
        Ok(Judged {
            reach,
            extras,
            held,
        })
    }

    /// What of the change `reach` to `made`, with the new `purpose`, asks the person (design
    /// section 9): what it gives the dot beyond the maker's reach, and, when the dot itself
    /// holds more than the maker has now (`held_extras`) and the change is more than taking
    /// things away (`narrows`), what the dot holds too: a new prompt, folder or schedule would
    /// steer what the person allowed it for another purpose. `true` with the second.
    pub(super) async fn change_extras(
        &self,
        maker: &Dot,
        made: &Dot,
        reach: &Reach,
        purpose: Option<&str>,
    ) -> Result<(Vec<Extra>, bool)> {
        let mut extras = reach.extras.clone();
        let purpose_now = made.made.as_ref().map(|m| m.purpose.as_str());
        let new_purpose = purpose.is_some_and(|p| Some(p) != purpose_now);
        if !new_purpose && narrows(&made.spec, &reach.spec) {
            return Ok((extras, false));
        }
        let own_folder = self.own_folder(made).await?;
        let held = held_extras(maker, &made.spec, own_folder);
        if held.is_empty() {
            return Ok((extras, false));
        }
        for extra in held {
            if !extras.iter().any(|seen| same_thing(&extra, seen)) {
                extras.push(extra);
            }
        }
        Ok((extras, true))
    }

    /// Whether the folder of `made` is the new folder of its own it was made with (design
    /// section 6), which is within its maker's reach: a folder directly in the folder for new
    /// dots (`Config::new_dot_root`) that no other dot works in.
    async fn own_folder(&self, made: &Dot) -> Result<bool> {
        let workdir = real_path(Path::new(made.spec.workdir.trim()));
        let root = folded(&real_path(self.parts.dot_admin.new_dot_root()));
        if workdir.parent().map(folded) != Some(root) {
            return Ok(false);
        }
        let key = folded(&workdir);
        let dots = self.parts.store.list_dots().await?;
        Ok(!dots.iter().any(|dot| {
            dot.id != made.id && folded(&real_path(Path::new(dot.spec.workdir.trim()))) == key
        }))
    }

    /// `reach_change` for what `wanted` names of `made`, with the grants of MCP servers the
    /// dot would no longer use dropped (`prune_grants`): a maker's change is saved with its
    /// grants exactly as given (`DotAdmin::update`, `exact`), so what a person's save drops for
    /// a server no longer used is dropped here. From the dot as asked, when the change names
    /// `uses` and not `variables`; naming both, the maker says the grants itself. From the dot
    /// without the extras, whenever the change names `uses`: an item that is an extra is not
    /// among its `uses`.
    async fn reach_of_change(&self, maker: &Dot, made: &Dot, wanted: &Wanted) -> Result<Reach> {
        let mut reach = reach_change(maker, made, wanted, &self.parts.worktrees_root)?;
        if wanted.uses.is_some() {
            if wanted.variables.is_none() {
                self.prune_grants(&mut reach.spec).await?;
            }
            self.prune_grants(&mut reach.without).await?;
        }
        Ok(reach)
    }

    /// Drops from `spec` the grants of every `mcp:<server>` consumer whose server the dot
    /// would not use: one neither named in its `uses` nor come with a plugin it names, as
    /// `check_exists` reads it. It never adds a grant, and the shell's grants stay.
    async fn prune_grants(&self, spec: &mut DotSpec) -> Result<()> {
        if spec.variables.0.is_empty() {
            return Ok(());
        }
        let used: Vec<String> = self
            .parts
            .store
            .used_mcp_servers(&spec.uses)
            .await?
            .into_iter()
            .map(|server| server.name)
            .collect();
        spec.variables
            .0
            .retain(|key, _| match Consumer::parse(key) {
                Ok(Consumer::Mcp(server)) => used.contains(&server),
                _ => true,
            });
        Ok(())
    }

    /// Refuses `name` as the new name of `made` when another dot has it, in any letter case
    /// (`Conflict`). As for a person's rename, the dot may take its own name in another case.
    async fn check_new_name_free(&self, made: &Dot, name: &str) -> Result<()> {
        let store = &self.parts.store;
        if name != made.spec.name
            && store
                .dot_name_taken_by_another(name.trim(), &made.id)
                .await?
        {
            return Err(Error::Conflict(format!(
                "A dot named {:?} already exists. Choose another name.",
                name.trim()
            )));
        }
        Ok(())
    }

    /// Saves `spec` as the made dot `made` and, with `purpose`, its new purpose; writes the
    /// `changed` entry of the log of the topic `root_run_id`, as the run `run_id`'s, and
    /// publishes `MakingUpdated`. Returns the dot as it is after and what changed, in plain
    /// words. `spec` is already checked: within the maker's reach, or allowed by the person.
    ///
    /// It goes through `DotAdmin::update` with `exact`, so the name, the Library items and
    /// this PC's rights are checked as for a change the person makes, and a change of the
    /// folder is refused while the dot has a run going. `create_folder` (of `Reach`) is made
    /// first, and removed again when the dot is not saved. When nothing differs from the dot
    /// as it is, nothing is written and nothing is logged.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn apply_change(
        &self,
        maker: &Dot,
        made: &Dot,
        spec: DotSpec,
        purpose: Option<&str>,
        create_folder: Option<PathBuf>,
        run_id: &str,
        root_run_id: &str,
    ) -> Result<(Dot, Vec<String>)> {
        let p = &self.parts;
        let purpose_now = made.made.as_ref().map(|m| m.purpose.as_str());
        let purpose = purpose.filter(|new| Some(*new) != purpose_now);
        if spec == made.spec && purpose.is_none() {
            return Ok((made.clone(), Vec::new()));
        }
        let new_folders = make_folders(maker, create_folder.as_deref()).await?;
        // Also when only the purpose changes: the role is checked there.
        let mut dot = match p.dot_admin.update(&made.id, spec, true).await {
            Ok(dot) => dot,
            Err(e) => {
                remove_folders(&new_folders).await;
                return Err(e);
            }
        };
        if let Some(purpose) = purpose {
            p.store.set_made_purpose(&dot.id, purpose).await?;
            dot = p.store.get_dot(&dot.id).await?;
        }
        let what = what_changed(made, &dot);
        if what.is_empty() {
            return Ok((dot, what));
        }
        // The dot is changed whether or not its entry could be written, so a failure here is
        // logged and the change still succeeds.
        let logged = p
            .store
            .add_making_event(&NewMakingEvent {
                maker_dot_id: &maker.id,
                run_id,
                root_run_id,
                kind: MakingEventKind::Changed,
                dot_id: Some(&dot.id),
                dot_name: &dot.spec.name,
                detail: json!({ "what": what }),
            })
            .await;
        if let Err(e) = logged {
            tracing::error!(dot = %dot.id, run = %run_id, "the change was not logged: {e}");
        }
        self.updated(&maker.id, root_run_id);
        Ok((dot, what))
    }

    /// Holds the change request `request` of `maker` against things as they are now (design
    /// sections 7.2 and 9), for its card and for the person's answer.
    ///
    /// The request keeps the ask itself and not only the dot as it was asked, because the
    /// person may have edited the dot since: saving the stored `spec` whole would undo their
    /// edits. So the ask is put on the dot as it is now (`reach_of_change`), and the result is
    /// what an answer saves: `reach.spec` for Allow, `reach.without` for "without".
    ///
    /// The extras found now are all covered by Allow when the person saw each of them on the
    /// card (`same_thing`). When one is new, because the maker's own settings were narrowed
    /// since it asked, `beyond_what_was_seen` is set: Allow would give the dot something the
    /// person was not shown, so it is refused and the request keeps waiting for a refusal,
    /// after which the maker can ask again. "Without" is not touched by this: it holds
    /// nothing beyond the maker's reach as it is now.
    ///
    /// `Gone` when the made dot no longer exists or is no longer this maker's, the ask can no
    /// longer be put on it, its new name is taken, or something it names is no longer in the
    /// Library or the vault. Deleting the made dot deletes its requests with it (`dot_id`
    /// cascades), so mostly such a request is simply not found.
    pub(super) async fn change_stands(&self, maker: &Dot, request: &MakeRequest) -> Result<Stands> {
        let store = &self.parts.store;
        let name = request
            .ask
            .as_ref()
            .map_or(request.name.as_str(), |ask| ask.dot.trim());
        let gone = || Ok(Stands::Gone(format!("{name} no longer exists.")));
        let (Some(ask), Some(dot_id)) = (&request.ask, &request.dot_id) else {
            return Ok(Stands::Gone(SOMETHING_GONE.to_string()));
        };
        let dot = match store.get_dot(dot_id).await {
            Ok(dot) => dot,
            Err(Error::NotFound(_)) => return gone(),
            Err(e) => return Err(e),
        };
        let makers = dot.made.as_ref().and_then(|made| made.by_dot_id.as_deref());
        if makers != Some(maker.id.as_str()) {
            return gone();
        }
        let reach = match self.reach_of_change(maker, &dot, &ask.wanted).await {
            Ok(reach) => reach,
            // A folder that has gone, or a dot the ask no longer fits.
            Err(Error::Invalid(_)) => return Ok(Stands::Gone(SOMETHING_GONE.to_string())),
            Err(e) => return Err(e),
        };
        let new_name = reach.spec.name.trim();
        if new_name != dot.spec.name && store.dot_name_taken_by_another(new_name, &dot.id).await? {
            return Ok(Stands::Gone(format!(
                "A dot named {new_name:?} exists now."
            )));
        }
        if let Some(reason) = self.gone_in(maker, &reach.spec).await? {
            return Ok(Stands::Gone(reason));
        }
        let (extras, held) = self
            .change_extras(maker, &dot, &reach, ask.purpose.as_deref())
            .await?;
        let seen = |extra: &Extra| request.extras.iter().any(|seen| same_thing(extra, seen));
        let beyond_what_was_seen = !extras.iter().all(seen);
        Ok(Stands::Yes(Standing {
            dot,
            reach,
            purpose: ask.purpose.clone(),
            extras,
            held,
            beyond_what_was_seen,
        }))
    }

    /// A maker's run deletes a dot it made, named `dot_name` (design section 9): the dot goes
    /// as the person's delete takes it, history included, through `DotAdmin::delete`. Returns
    /// what the tool tells the run. The topic's log has a `deleted` entry that keeps the dot's
    /// name. A place in the maker's total is free again; the run's own count is not (section
    /// 10). Dots the deleted dot made itself stay, with its name as who made them. A job of the
    /// dot that waits for the person's answer ends with it: a question that waits is no run
    /// going, so the delete is not refused, and the job's maker topic gets a `job_ended` entry
    /// (`deleted`) and is woken with `jobs::deleted_message` (section 8).
    ///
    /// Refused while the dot has a run queued, running or awaiting approval (`Conflict`, in
    /// words for the model: the store's own sentence says to cancel the runs, which a maker
    /// cannot do). Refused as `make_dot` is for the run, and for a dot the maker did not make.
    pub async fn delete_dot(&self, run_id: &str, dot_name: &str) -> Result<String> {
        let p = &self.parts;
        let (run, _) = self.caller(run_id).await?;
        // A job of the dot whose run has ended is settled first, so its result is not lost
        // with the dot.
        if let Err(e) = self.settle_jobs().await {
            tracing::warn!(run = %run.id, "settling the jobs before a delete failed: {e}");
        }
        // With the answers: no answer to a change request for the dot is carried out while
        // the dot goes.
        let one_at_a_time = p.one_at_a_time.lock().await;
        // As they are once the lock is held: the run may have ended, or the switch turned off.
        let (run, maker) = self.caller(&run.id).await?;
        let made = self.made_dot(&maker, dot_name).await?;
        let name = made.spec.name.as_str();
        let busy = || {
            Error::Conflict(format!(
                "\"{name}\" has a run going or queued. Wait for it to end, then delete it."
            ))
        };
        if p.store.latest_run(&made.id, true).await?.is_some() {
            return Err(busy());
        }
        let (open_jobs, changes, topics) = match self.delete_and_tell(&made).await {
            Ok(gone) => gone,
            // A run of it was queued in between (`Store::delete_dot_if_idle`).
            Err(Error::Conflict(_)) => return Err(busy()),
            Err(e) => return Err(e),
        };
        // The dot is gone whether or not its entry could be written, so a failure here is
        // logged and the delete still succeeds.
        let logged = p
            .store
            .add_making_event(&NewMakingEvent {
                maker_dot_id: &maker.id,
                run_id: &run.id,
                root_run_id: &run.root_run_id,
                kind: MakingEventKind::Deleted,
                dot_id: None,
                dot_name: name,
                detail: json!({}),
            })
            .await;
        if let Err(e) = logged {
            tracing::error!(dot = %made.id, run = %run.id, "the delete was not logged: {e}");
        }
        self.updated(&maker.id, &run.root_run_id);
        // The other topics that show the dot: their cards read again too.
        for (maker_id, topic) in &topics {
            if (maker_id.as_str(), topic.as_str()) != (maker.id.as_str(), run.root_run_id.as_str())
            {
                self.updated(maker_id, topic);
            }
        }
        // After the service's lock is released: `wake` takes the thread lock itself.
        drop(one_at_a_time);
        self.wake_for_deleted(&open_jobs, &changes, name).await;
        Ok(format!("Deleted the dot \"{name}\" and its history."))
    }

    /// The person deletes the dot `dot_id` (design sections 9 and 11): any dot goes through
    /// here, so that what the makers show of it is told. The dot goes as `DotAdmin::delete`
    /// takes it, with its checks and its refusals.
    ///
    /// For a made dot: a job of it that waits for the person's answer ends with it, its
    /// maker's topic gets a `job_ended` entry (`deleted`), and the maker is woken once with
    /// `jobs::deleted_message`, as when the maker deletes the dot itself; a job whose run has
    /// ended is settled first, so its result is not lost. A change request that waited for
    /// the dot goes with it. For a maker: its log, requests, jobs and wakes go with it, and
    /// the dots it made stay. Every topic that showed the dot, or a waiting request of the
    /// deleted maker, gets a `MakingUpdated`, so open cards, the Inbox and its count read
    /// again. The log gets no `deleted` entry: that card says the maker deleted the dot.
    pub async fn person_deletes(&self, dot_id: &str) -> Result<()> {
        let p = &self.parts;
        if !p.store.open_making_jobs(Some(dot_id)).await?.is_empty() {
            if let Err(e) = self.settle_jobs().await {
                tracing::warn!(dot = %dot_id, "settling the jobs before a delete failed: {e}");
            }
        }
        // With the answers: no answer to a change request for the dot is carried out while
        // the dot goes.
        let one_at_a_time = p.one_at_a_time.lock().await;
        let dot = p.store.get_dot(dot_id).await?;
        let (open_jobs, changes, topics) = self.delete_and_tell(&dot).await?;
        for (maker_id, topic) in &topics {
            self.updated(maker_id, topic);
        }
        // After the service's lock is released: `wake` takes the thread lock itself.
        drop(one_at_a_time);
        self.wake_for_deleted(&open_jobs, &changes, &dot.spec.name)
            .await;
        Ok(())
    }

    /// Deletes `dot` through `DotAdmin::delete` and ends the jobs given to it that are open,
    /// which can only wait for the person's answer: their rows go with the dot, so each gets
    /// its `job_ended` entry (`deleted`) here (design section 8). No pass ends one of them in
    /// between. A change a maker asked for the dot that waits for the person goes with the dot
    /// (`dot_id` cascades). The caller holds the service's lock. Returns those jobs and those
    /// requests, whose makers the caller wakes once the lock is released, and the topics that
    /// showed the dot and were not told by a job's entry, for the caller to tell
    /// (`Store::making_topics_of`).
    async fn delete_and_tell(
        &self,
        dot: &Dot,
    ) -> Result<(Vec<MakingJob>, Vec<MakeRequest>, Vec<(String, String)>)> {
        let p = &self.parts;
        let settling = p.settling.lock().await;
        let open_jobs = p.store.open_making_jobs(Some(&dot.id)).await?;
        let changes: Vec<MakeRequest> = p
            .store
            .list_make_requests(None, true)
            .await?
            .into_iter()
            .filter(|r| r.kind == RequestKind::Change && r.dot_id.as_deref() == Some(&dot.id))
            .collect();
        let mut topics = p.store.making_topics_of(&dot.id).await?;
        p.dot_admin.delete(&dot.id).await?;
        drop(settling);
        for job in &open_jobs {
            let detail = json!({
                "job_id": job.id,
                "run_id": job.last_run_id,
                "how": JobEnd::Deleted.as_str(),
            });
            self.log_job(job, MakingEventKind::JobEnded, None, &dot.spec.name, detail)
                .await;
            topics.retain(|(maker_id, topic)| {
                (maker_id, topic) != (&job.maker_dot_id, &job.maker_root_run_id)
            });
        }
        Ok((open_jobs, changes, topics))
    }

    /// Wakes the maker of each of `jobs`, which ended because their made dot, named `name`,
    /// was deleted (design section 8), and the maker of each of `changes`, the changes it
    /// asked for the dot that went with it (section 9), which it was told it would be woken
    /// about. A topic is woken once, with all it is told. The caller holds no lock.
    async fn wake_for_deleted(&self, jobs: &[MakingJob], changes: &[MakeRequest], name: &str) {
        let mut wakes: Vec<(&str, &str, String)> = Vec::new();
        for job in jobs {
            push_wake(
                &mut wakes,
                &job.maker_dot_id,
                &job.maker_root_run_id,
                deleted_message(name),
            );
        }
        for request in changes {
            push_wake(
                &mut wakes,
                &request.maker_dot_id,
                &request.root_run_id,
                change_went_message(name),
            );
        }
        for (maker, topic, message) in wakes {
            if let Err(e) = self.wake(maker, topic, &message).await {
                tracing::warn!(maker = %maker, topic = %topic, "the maker was not woken for a deleted dot: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn words(schedule: Value) -> String {
        schedule_words(&serde_json::from_value(schedule).unwrap())
    }

    #[test]
    fn a_schedule_reads_in_plain_words() {
        let cases = [
            (
                json!({"rule": {"kind": "every", "minutes": 1}}),
                "every minute",
            ),
            (
                json!({"rule": {"kind": "every", "minutes": 45}}),
                "every 45 minutes",
            ),
            (
                json!({"rule": {"kind": "every", "minutes": 60}}),
                "every hour",
            ),
            (
                json!({"rule": {"kind": "every", "minutes": 120}}),
                "every 2 hours",
            ),
            (
                json!({"rule": {"kind": "every", "minutes": 2880}}),
                "every 2 days",
            ),
            (
                json!({"rule": {"kind": "daily", "time": "09:00"}}),
                "daily at 09:00",
            ),
            (
                json!({"rule": {"kind": "weekly", "days": ["mon", "thu"], "time": "18:30"}}),
                "weekly on Mon, Thu at 18:30",
            ),
            (
                json!({"rule": {"kind": "once", "at": "2026-11-20T09:00"}}),
                "once at 2026-11-20 09:00",
            ),
            (
                json!({"rule": {"kind": "cron", "expr": "0 0 9 * * *"}}),
                "cron 0 0 9 * * *",
            ),
            (
                json!({"rule": {"kind": "daily", "time": "09:00"}, "until": {"count": 10}}),
                "daily at 09:00, 10 times",
            ),
            (
                json!({"rule": {"kind": "daily", "time": "09:00"}, "until": {"count": 1}}),
                "daily at 09:00, 1 time",
            ),
            (
                json!({
                    "rule": {"kind": "every", "minutes": 30},
                    "until": {"date": "2026-12-01T09:00"}
                }),
                "every 30 minutes, until 2026-12-01 09:00",
            ),
        ];
        for (schedule, expected) in cases {
            assert_eq!(words(schedule), expected);
        }
    }
}
