//! Watchers (watchers spec): what starts a dot when a file changes in a folder, an email
//! arrives in a Gmail label, or something happens on a GitHub repository. This module holds the
//! checks a watcher passes before it is saved, with the spec's sentences, and `Watchers`, the
//! service every event goes through (sections 5 and 7).

pub mod acting;
pub mod cap;
pub mod describe;
pub mod email;
pub mod files;
pub mod folder;
pub mod github;
pub mod hours;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, TimeDelta, Utc};
use serde_json::{json, Value};
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub use acting::{ActingTracker, ACTING_GRACE};
pub use cap::{FLOOD_CAP, FLOOD_WINDOW};

use crate::events::{Bus, RuntimeEvent};
use crate::model::{
    EmailConfig, FolderConfig, GithubConfig, NewRun, PausedReason, Run, TriggerKind, WatchHours,
    Watcher, WatcherConfig, WatcherKind,
};
use crate::runner::Runner;
use crate::scheduler::Clock;
use crate::store::Store;
use crate::util::{new_id, timestamp};
use crate::vault::{EntryKind, Vault};
use crate::{Error, Result};

/// How often an email or GitHub watcher checks (spec sections 4.2 and 4.3).
pub const POLL_EVERY: TimeDelta = TimeDelta::minutes(1);
/// How long a watcher keeps failing before Dots pauses it (spec section 7).
pub const GIVE_UP_AFTER: TimeDelta = TimeDelta::minutes(60);
/// The waits after the first failures; every 10 minutes after them.
pub const BACKOFF_MINUTES: [i64; 4] = [1, 2, 5, 10];
/// The most text of an email body or a GitHub event a run gets.
pub const MAX_BODY_CHARS: usize = 20_000;
/// The most outside text (the held events' bodies, together) an opening run's prompt carries:
/// 100 bodies of `MAX_BODY_CHARS` would not fit the model's context (`describe` shares it out).
pub const MAX_OPENING_BODY_CHARS: usize = 150_000;

/// The held events an opening run carries in full; only these keep their files (watch hours
/// spec section 3).
pub const HELD_IN_FULL: u32 = 100;

/// A watcher's name is 1 to 80 characters (spec section 3).
pub const MAX_NAME_CHARS: usize = 80;

pub const NAME_RULE: &str = "A watcher's name is 1 to 80 characters.";
pub const FOLDER_RULE: &str = "Choose a folder.";
pub const NOT_ABSOLUTE: &str = "The folder must be an absolute path.";
pub const FOLDER_EVENTS_RULE: &str = "Choose at least one of created, changed or deleted.";
pub const ADDRESS_RULE: &str = "Enter the Gmail address.";
pub const FROM_RULE: &str = "From is an address or @domain.";
pub const SECRET_RULE: &str = "Choose the vault entry that holds the password or token.";
pub const REPOS_RULE: &str = "Add at least one repository (owner/repo).";
pub const EVENTS_RULE: &str = "Choose at least one event.";
/// Why a flooded watcher paused, as the notification and the Inbox say it.
pub const FLOOD_TEXT: &str = "20 runs in an hour";

pub fn no_folder(path: &str) -> String {
    format!("No folder at {path}.")
}

pub fn bad_pattern(pattern: &str) -> String {
    format!("The pattern {pattern} is not a valid file pattern.")
}

pub fn no_entry(name: &str) -> String {
    format!("The vault has no entry {name}.")
}

pub fn plain_entry(name: &str) -> String {
    format!("{name} is a plain entry; choose a secret one.")
}

pub fn entry_gone(name: &str) -> String {
    format!("The vault entry {name} is gone.")
}

pub fn bad_repo(repo: &str) -> String {
    format!("{repo} is not owner/repo.")
}

pub fn folder_gone(path: &str) -> String {
    format!("The folder {path} no longer exists.")
}

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

/// The name trimmed, or the rule it breaks.
pub fn check_name(name: &str) -> Result<String> {
    let name = name.trim();
    let chars = name.chars().count();
    if chars == 0 || chars > MAX_NAME_CHARS {
        return Err(invalid(NAME_RULE));
    }
    Ok(name.to_string())
}

/// `None` for a blank text, else the text trimmed.
fn optional(text: &Option<String>) -> Option<String> {
    text.as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

/// The non-blank items, trimmed.
fn items(list: &[String]) -> Vec<String> {
    list.iter()
        .map(|i| i.trim())
        .filter(|i| !i.is_empty())
        .map(str::to_string)
        .collect()
}

/// The vault entry `name` must exist and hold a secret. Its value is never read here.
async fn check_secret(store: &Store, name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(invalid(SECRET_RULE));
    }
    match store.find_env_entry(name).await? {
        None => Err(invalid(no_entry(name))),
        Some(entry) if entry.kind != EntryKind::Secret => Err(invalid(plain_entry(name))),
        Some(_) => Ok(name.to_string()),
    }
}

fn check_folder(config: &FolderConfig) -> Result<FolderConfig> {
    let path = config.path.trim();
    if path.is_empty() {
        return Err(invalid(FOLDER_RULE));
    }
    if !Path::new(path).is_absolute() {
        return Err(invalid(NOT_ABSOLUTE));
    }
    if !Path::new(path).is_dir() {
        return Err(invalid(no_folder(path)));
    }
    if config.events.is_empty() {
        return Err(invalid(FOLDER_EVENTS_RULE));
    }
    let pattern = config.pattern.trim();
    if !pattern.is_empty() && globset::Glob::new(pattern).is_err() {
        return Err(invalid(bad_pattern(pattern)));
    }
    Ok(FolderConfig {
        path: path.to_string(),
        pattern: pattern.to_string(),
        events: config.events.clone(),
        recursive: config.recursive,
    })
}

async fn check_email(store: &Store, config: &EmailConfig) -> Result<EmailConfig> {
    let address = config.address.trim();
    let at = address.find('@');
    if !matches!(at, Some(i) if i > 0 && i + 1 < address.len()) {
        return Err(invalid(ADDRESS_RULE));
    }
    let secret = check_secret(store, &config.secret).await?;
    let from = optional(&config.from);
    if let Some(from) = &from {
        let domain = from.rsplit_once('@').map(|(_, d)| d);
        if !matches!(domain, Some(d) if !d.is_empty() && !d.contains(char::is_whitespace)) {
            return Err(invalid(FROM_RULE));
        }
    }
    let label = config.label.trim();
    Ok(EmailConfig {
        address: address.to_string(),
        secret,
        label: if label.is_empty() { "INBOX" } else { label }.to_string(),
        from,
        subject: optional(&config.subject),
        has_attachment: config.has_attachment,
    })
}

async fn check_github(store: &Store, config: &GithubConfig) -> Result<GithubConfig> {
    let repos = items(&config.repos);
    if repos.is_empty() {
        return Err(invalid(REPOS_RULE));
    }
    let shape = regex::Regex::new(r"^[A-Za-z0-9-]+/[A-Za-z0-9._-]+$").expect("a valid regex");
    if let Some(bad) = repos.iter().find(|r| !shape.is_match(r)) {
        return Err(invalid(bad_repo(bad)));
    }
    if config.events.is_empty() {
        return Err(invalid(EVENTS_RULE));
    }
    let secret = check_secret(store, &config.secret).await?;
    Ok(GithubConfig {
        repos,
        events: config.events.clone(),
        secret,
        label: optional(&config.label),
        authors: items(&config.authors),
        skip_bots: config.skip_bots,
        branches: items(&config.branches),
    })
}

/// The config as it is saved (spec sections 3 and 4): strings trimmed, an empty label
/// `INBOX`, blank filters dropped; or the rule it breaks, in the spec's words.
pub async fn check_config(store: &Store, config: &WatcherConfig) -> Result<WatcherConfig> {
    Ok(match config {
        WatcherConfig::Folder(c) => WatcherConfig::Folder(check_folder(c)?),
        WatcherConfig::Email(c) => WatcherConfig::Email(check_email(store, c).await?),
        WatcherConfig::Github(c) => WatcherConfig::Github(check_github(store, c).await?),
    })
}

/// The retries of a failing watcher (spec section 7): 1, 2, 5 and 10 minutes, then every 10;
/// none once it has failed for `GIVE_UP_AFTER`.
#[derive(Debug, Default, Clone)]
pub struct Backoff {
    since: Option<DateTime<Utc>>,
    attempts: usize,
}

impl Backoff {
    pub fn new() -> Backoff {
        Backoff::default()
    }

    /// A failure at `now`: when to try again, or `None` to give up.
    pub fn failed(&mut self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let since = *self.since.get_or_insert(now);
        if now - since >= GIVE_UP_AFTER {
            return None;
        }
        let wait = BACKOFF_MINUTES.get(self.attempts).copied().unwrap_or(10);
        self.attempts += 1;
        Some(now + TimeDelta::minutes(wait))
    }

    pub fn succeeded(&mut self) {
        *self = Backoff::default();
    }
}

/// How a source failed: a transient failure is tried again, a permanent one pauses at once.
#[derive(Debug, Clone, PartialEq)]
pub enum Failure {
    Transient(String),
    Permanent(String),
}

/// One event of a source: the run's payload, and the files to stage for it (email attachments:
/// name and bytes).
#[derive(Debug, Clone, PartialEq)]
pub struct SourceEvent {
    pub payload: Value,
    pub files: Vec<(String, Vec<u8>)>,
}

impl SourceEvent {
    pub fn new(payload: Value) -> SourceEvent {
        SourceEvent {
            payload,
            files: Vec::new(),
        }
    }
}

/// What `fire` did with an event.
#[derive(Debug)]
pub enum Fired {
    Run(Box<Run>),
    /// The hour was full: this event paused the watcher.
    Capped,
    /// The watcher, its dot or Dots is paused or off: nothing was done.
    Off,
    /// Outside the watcher's hours (or at its opening): the event is kept for the opening run.
    Held,
}

/// When a polling watcher checks next.
#[derive(Debug, Clone, PartialEq)]
pub enum Poll {
    Next(DateTime<Utc>),
    Stopped,
}

/// Where email and GitHub watchers read; tests point them at fakes.
#[derive(Clone)]
pub struct Sources {
    pub github_api: String,
    pub mail: Arc<dyn email::MailSource>,
}

impl Default for Sources {
    fn default() -> Self {
        Sources {
            github_api: "https://api.github.com".to_string(),
            mail: Arc::new(email::ImapMailSource::gmail()),
        }
    }
}

impl std::fmt::Debug for Sources {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sources")
            .field("github_api", &self.github_api)
            .finish_non_exhaustive()
    }
}

pub struct WatcherDeps {
    pub store: Store,
    pub bus: Bus,
    pub runner: Arc<Runner>,
    pub vault: Vault,
    pub clock: Arc<dyn Clock>,
    pub acting: Arc<ActingTracker>,
    pub data_dir: PathBuf,
    pub sources: Sources,
    /// The time zone watch hours are read in: `chrono::Local` in the app.
    pub zone: Arc<dyn hours::Zone>,
}

/// A running watcher's task: what it was started with, and how to stop it.
struct Task {
    config: WatcherConfig,
    hours: Option<WatchHours>,
    token: CancellationToken,
}

/// While alive, the watcher is at its opening (`Watchers::opening`); dropped, it is not.
struct AtOpening<'a> {
    service: &'a Watchers,
    watcher_id: &'a str,
}

impl Drop for AtOpening<'_> {
    fn drop(&mut self) {
        self.service.set_opening(self.watcher_id, false);
    }
}

/// The watchers service (spec sections 5 and 7): one task per running watcher, the one door
/// every event goes through (`fire`), and where sources report failures.
pub struct Watchers {
    deps: WatcherDeps,
    paused: AtomicBool,
    reload: Notify,
    tasks: Mutex<HashMap<String, Task>>,
    backoffs: Mutex<HashMap<String, Backoff>>,
    files: files::WatcherFiles,
    /// The GitHub login behind each watcher's token, asked once.
    logins: Mutex<HashMap<String, String>>,
    /// The last ETag and answer of each (watcher, URL) GitHub answered.
    etags: Mutex<HashMap<(String, String), (String, Value)>>,
    /// One lock per watcher: two checks of it never run at once, and a change of its config
    /// waits for the check in flight (whose cursor would otherwise be written back).
    checks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// The watchers at their opening: what they find now is held for the opening run, even
    /// inside their hours.
    opening: Mutex<HashSet<String>>,
    /// One lock per watcher that `fire`, the opening and dropping what is held all take: an
    /// event `fire` decides to hold is stored before an opening takes the held events, so none
    /// is left behind for the next opening (watch hours spec section 3.8).
    gates: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// The watchers whose stored hours could not be used, already said in the log.
    warned: Mutex<HashSet<String>>,
}

impl Watchers {
    pub fn new(deps: WatcherDeps) -> Arc<Watchers> {
        Arc::new(Watchers {
            files: files::WatcherFiles::new(deps.data_dir.join("watchers")),
            deps,
            paused: AtomicBool::new(false),
            reload: Notify::new(),
            tasks: Mutex::new(HashMap::new()),
            backoffs: Mutex::new(HashMap::new()),
            logins: Mutex::new(HashMap::new()),
            etags: Mutex::new(HashMap::new()),
            checks: Mutex::new(HashMap::new()),
            opening: Mutex::new(HashSet::new()),
            gates: Mutex::new(HashMap::new()),
            warned: Mutex::new(HashSet::new()),
        })
    }

    /// Runs the service until `shutdown`: starts and stops the watchers' tasks on every reload.
    pub fn spawn(self: &Arc<Self>, shutdown: CancellationToken) -> JoinHandle<()> {
        let this = self.clone();
        tokio::spawn(async move {
            loop {
                if let Err(e) = this.reconcile(&shutdown).await {
                    tracing::warn!("watchers: reconcile failed: {e}");
                }
                tokio::select! {
                    biased;
                    _ = shutdown.cancelled() => break,
                    _ = this.reload.notified() => {}
                }
            }
            for (_, task) in this.tasks.lock().unwrap().drain() {
                task.token.cancel();
            }
        })
    }

    /// Asks the service to start and stop tasks to match the watchers now.
    pub fn reload(&self) {
        self.reload.notify_one();
    }

    /// The person's "Pause schedules and watchers" (spec section 5).
    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::SeqCst);
        self.reload();
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    pub fn acting(&self) -> &Arc<ActingTracker> {
        &self.deps.acting
    }

    /// Where watchers stage files for their runs (`<data_dir>/watchers`).
    pub fn files(&self) -> &files::WatcherFiles {
        &self.files
    }

    fn updated(&self, watcher: &Watcher) {
        let _ = self.deps.bus.send(RuntimeEvent::WatcherUpdated {
            dot_id: watcher.dot_id.clone(),
            watcher_id: watcher.id.clone(),
        });
    }

    fn changed(&self, watcher: &Watcher) {
        self.updated(watcher);
        self.reload();
    }

    pub async fn list(&self, dot_id: &str) -> Result<Vec<Watcher>> {
        self.deps.store.get_dot(dot_id).await?;
        let watchers = self.deps.store.list_watchers(dot_id).await?;
        Ok(watchers.into_iter().map(|w| self.with_opening(w)).collect())
    }

    /// The watcher as stored, with its next opening when it is outside its hours now.
    async fn get(&self, id: &str) -> Result<Watcher> {
        let watcher = self.deps.store.get_watcher(id).await?;
        Ok(self.with_opening(watcher))
    }

    /// Sets `next_opening` (RFC 3339, local time) when the watcher is outside its hours now.
    fn with_opening(&self, mut watcher: Watcher) -> Watcher {
        let now = self.deps.clock.now();
        watcher.next_opening = self
            .hours_of(&watcher)
            .and_then(|hours| self.opens_at(&watcher.id, hours, now))
            .map(|at| self.deps.zone.rfc3339(at));
        watcher
    }

    /// The watcher's hours, if they can be used. Hours that fail their check (only a damaged
    /// row can hold them) count as none, so the watcher neither holds forever nor waits for an
    /// opening that never comes; said once in the log.
    fn hours_of<'a>(&self, watcher: &'a Watcher) -> Option<&'a WatchHours> {
        let hours = watcher.hours.as_ref()?;
        if hours::check_hours(hours).is_ok() {
            return Some(hours);
        }
        self.warn_once(
            &watcher.id,
            "its watch hours are not valid; reading them as any time",
        );
        None
    }

    fn warn_once(&self, watcher_id: &str, what: &str) {
        if self.warned.lock().unwrap().insert(watcher_id.to_string()) {
            tracing::warn!(watcher = watcher_id, "{what}");
        }
    }

    /// The hours' next opening after `now`; `None` (said once) when the maths gives one that is
    /// not in the future, so no wait ever spins: the watcher then counts as inside.
    fn next_opening(
        &self,
        watcher_id: &str,
        hours: &WatchHours,
        now: DateTime<Utc>,
    ) -> Option<DateTime<Utc>> {
        let at = self.deps.zone.next_opening(hours, now);
        if at > now {
            return Some(at);
        }
        self.warn_once(
            watcher_id,
            "its next opening is not in the future; reading its hours as any time",
        );
        None
    }

    /// When the watcher opens next, if it is outside its hours at `now`; `None` inside.
    fn opens_at(
        &self,
        watcher_id: &str,
        hours: &WatchHours,
        now: DateTime<Utc>,
    ) -> Option<DateTime<Utc>> {
        if self.deps.zone.is_inside(hours, now) {
            return None;
        }
        self.next_opening(watcher_id, hours, now)
    }

    /// The lock of the watcher's held events (`gates`).
    fn gate(&self, id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.gates
            .lock()
            .unwrap()
            .entry(id.to_string())
            .or_default()
            .clone()
    }

    pub async fn add(
        &self,
        dot_id: &str,
        name: &str,
        config: WatcherConfig,
        hours: Option<WatchHours>,
    ) -> Result<Watcher> {
        let name = check_name(name)?;
        let config = check_config(&self.deps.store, &config).await?;
        let hours = hours.as_ref().map(hours::check_hours).transpose()?;
        let watcher = self
            .deps
            .store
            .add_watcher(dot_id, &name, &config, hours.as_ref())
            .await?;
        self.changed(&watcher);
        Ok(self.with_opening(watcher))
    }

    /// The lock of the watcher's checks.
    fn check_lock(&self, id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.checks
            .lock()
            .unwrap()
            .entry(id.to_string())
            .or_default()
            .clone()
    }

    pub async fn update(
        &self,
        id: &str,
        name: &str,
        config: WatcherConfig,
        hours: Option<WatchHours>,
    ) -> Result<Watcher> {
        // A check in flight finishes first: its cursor is then cleared, not written back.
        let check = self.check_lock(id);
        let no_check = check.lock().await;
        self.deps.store.get_watcher(id).await?;
        let name = check_name(name)?;
        let config = check_config(&self.deps.store, &config).await?;
        let hours = hours.as_ref().map(hours::check_hours).transpose()?;
        let watcher = self
            .deps
            .store
            .update_watcher(id, &name, &config, hours.as_ref())
            .await?;
        self.backoffs.lock().unwrap().remove(id);
        self.forget_remote(id);
        drop(no_check);
        self.changed(&watcher);
        // Hours removed, or changed so that now is inside: what it holds goes now.
        self.release_now(id).await;
        self.get(id).await
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        let gate = self.gate(id);
        let _gate = gate.lock().await;
        // Its held events go with its row, their files with its folder.
        let watcher = self.deps.store.delete_watcher(id).await?;
        self.files.forget_watcher(id);
        self.backoffs.lock().unwrap().remove(id);
        self.forget_remote(id);
        self.stop_task(id);
        self.changed(&watcher);
        Ok(())
    }

    /// Turns the watcher on or off. Turned off, it drops what it holds; turned on inside its
    /// hours, it releases what it holds.
    pub async fn set_enabled(&self, id: &str, enabled: bool) -> Result<Watcher> {
        let watcher = self.deps.store.set_watcher_enabled(id, enabled).await?;
        self.changed(&watcher);
        if enabled {
            self.release_now(id).await;
        } else {
            self.drop_held(id).await?;
        }
        self.get(id).await
    }

    /// The person resumes a watcher Dots paused; nothing happens to one that was not paused.
    /// Resumed inside its hours, it releases what it holds.
    pub async fn resume(&self, id: &str) -> Result<Watcher> {
        let watcher = self.deps.store.get_watcher(id).await?;
        if watcher.paused_reason.is_none() {
            return Ok(self.with_opening(watcher));
        }
        let watcher = self.deps.store.resume_watcher(id).await?;
        self.backoffs.lock().unwrap().remove(id);
        self.changed(&watcher);
        self.release_now(id).await;
        self.get(id).await
    }

    /// Drops the watcher's held events and their files (it was turned off).
    async fn drop_held(&self, id: &str) -> Result<()> {
        let gate = self.gate(id);
        let _gate = gate.lock().await;
        for event in self.deps.store.drop_held(id).await? {
            self.files.remove_held(id, &event.id);
        }
        Ok(())
    }

    /// `release` after the person's change, which is saved and announced already: a failure is
    /// logged, not returned, and the release is tried again when the watcher's task restarts.
    async fn release_now(&self, id: &str) {
        if let Err(e) = self.release(id).await {
            tracing::warn!(watcher = id, "releasing held events failed: {e}");
        }
    }

    /// The release rule (watch hours spec section 3.8): a watcher that holds events releases
    /// them at once, in one opening run, when it is inside its hours now or an opening has
    /// passed since the oldest of them was held (Dots was closed or paused over it, or the
    /// watcher was paused or off).
    async fn release(&self, id: &str) -> Result<()> {
        let watcher = self.deps.store.get_watcher(id).await?;
        if watcher.held_count == 0 {
            return Ok(());
        }
        let now = self.deps.clock.now();
        if let Some(hours) = self.hours_of(&watcher) {
            if self.opens_at(id, hours, now).is_some()
                && !self.missed_opening(id, hours, now).await?
            {
                return Ok(());
            }
        }
        self.open(id).await?;
        Ok(())
    }

    /// Whether one of the hours' openings came, at or before `now`, after the watcher's oldest
    /// held event was held.
    async fn missed_opening(
        &self,
        id: &str,
        hours: &WatchHours,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        let Some(oldest) = self.deps.store.oldest_held_at(id).await? else {
            return Ok(false);
        };
        let oldest = DateTime::parse_from_rfc3339(&oldest)
            .map_err(|e| Error::Other(format!("held_at {oldest}: {e}")))?
            .with_timezone(&Utc);
        Ok(self.deps.zone.next_opening(hours, oldest) <= now)
    }

    /// Dots pauses the watcher. Announced once: a watcher paused already stays as it is.
    pub async fn pause(&self, id: &str, reason: PausedReason, error: Option<&str>) -> Result<()> {
        let error = error.map(|e| crate::vault::redact::global().scrub(e).into_owned());
        let at = timestamp(self.deps.clock.now());
        let paused = self
            .deps
            .store
            .pause_watcher(id, reason, error.as_deref(), &at)
            .await?;
        self.backoffs.lock().unwrap().remove(id);
        self.stop_task(id);
        if let Some(watcher) = paused {
            self.updated(&watcher);
            let _ = self.deps.bus.send(RuntimeEvent::WatcherPaused {
                dot_id: watcher.dot_id.clone(),
                watcher_id: watcher.id.clone(),
                name: watcher.name.clone(),
                reason,
                error,
            });
        }
        self.reload();
        Ok(())
    }

    /// Marks the watcher as at its opening (or no longer): while it is, `fire` holds what it
    /// gets even inside its hours, so the opening poll's events join the opening run.
    #[doc(hidden)]
    pub fn set_opening(&self, watcher_id: &str, opening: bool) {
        let mut set = self.opening.lock().unwrap();
        if opening {
            set.insert(watcher_id.to_string());
        } else {
            set.remove(watcher_id);
        }
    }

    /// Whether `fire` holds the watcher's events now: it has hours and is outside them, or is
    /// at its opening.
    fn holds(&self, watcher: &Watcher, now: DateTime<Utc>) -> bool {
        let Some(hours) = self.hours_of(watcher) else {
            return false;
        };
        self.opening.lock().unwrap().contains(&watcher.id)
            || self.opens_at(&watcher.id, hours, now).is_some()
    }

    /// Whether the watcher, its dot or Dots is paused or off: then nothing is done.
    async fn is_off(&self, watcher: &Watcher) -> Result<bool> {
        if self.is_paused() || !watcher.enabled || watcher.paused_reason.is_some() {
            return Ok(true);
        }
        Ok(!self.deps.store.get_dot(&watcher.dot_id).await?.spec.enabled)
    }

    /// The one door every event goes through (spec section 5): one queued `watcher` run, unless
    /// the watcher, its dot or Dots is paused or off, the watcher is outside its hours (the
    /// event is held), or the hour is full.
    pub async fn fire(&self, watcher_id: &str, event: SourceEvent) -> Result<Fired> {
        // Whether to hold is decided, and the event held, before an opening can take.
        let gate = self.gate(watcher_id);
        let _gate = gate.lock().await;
        let watcher = self.deps.store.get_watcher(watcher_id).await?;
        if self.is_off(&watcher).await? {
            return Ok(Fired::Off);
        }
        let now = self.deps.clock.now();
        let mut payload = event.payload;
        if let Value::Object(map) = &mut payload {
            map.insert("watcher_id".into(), Value::String(watcher.id.clone()));
            map.insert("watcher_name".into(), Value::String(watcher.name.clone()));
        }
        if self.holds(&watcher, now) {
            self.hold(&watcher, &payload, &event.files, now).await?;
            return Ok(Fired::Held);
        }
        if !cap::take(&self.deps.store, &watcher.id, now).await? {
            self.pause(&watcher.id, PausedReason::Flood, None).await?;
            return Ok(Fired::Capped);
        }
        let run_id = new_id();
        self.files.stage(&watcher.id, &run_id, &event.files)?;
        let mut new = NewRun::new(&watcher.dot_id, TriggerKind::Watcher);
        new.id = Some(run_id.clone());
        new.payload = Some(payload);
        let run = match self.deps.runner.enqueue(new).await {
            Ok(run) => run,
            Err(e) => {
                // No run will copy or release what was staged for it.
                let staged = self.files.stage_dir(&watcher.id, &run_id);
                if staged.exists() {
                    let _ = std::fs::remove_dir_all(staged);
                }
                return Err(e);
            }
        };
        self.deps
            .store
            .touch_watcher_event(&watcher.id, &timestamp(now))
            .await?;
        self.updated(&watcher);
        Ok(Fired::Run(Box::new(run)))
    }

    /// The opening (watch hours spec section 4.4): one `watcher` run carrying the held events,
    /// the first `HELD_IN_FULL` in full, oldest first, with their files, and the count of the
    /// rest. It passes `fire`'s checks and the flood cap before anything is taken: `Off` (also
    /// when nothing is held) and `Capped` (the watcher pauses as flooded) keep the events.
    pub async fn open(&self, watcher_id: &str) -> Result<Fired> {
        let gate = self.gate(watcher_id);
        let _gate = gate.lock().await;
        let store = &self.deps.store;
        let watcher = store.get_watcher(watcher_id).await?;
        if watcher.held_count == 0 || self.is_off(&watcher).await? {
            return Ok(Fired::Off);
        }
        let now = self.deps.clock.now();
        if !cap::take(store, &watcher.id, now).await? {
            self.pause(&watcher.id, PausedReason::Flood, None).await?;
            return Ok(Fired::Capped);
        }
        let held = store.held_events(&watcher.id).await?;
        let Some(since) = held.first().map(|e| e.held_at.clone()) else {
            return Ok(Fired::Off);
        };
        let run_id = new_id();
        let in_full = held.len().min(HELD_IN_FULL as usize);
        let mut carried = Vec::with_capacity(in_full);
        let mut attachments = Vec::new();
        // Each moved file and the held event it came from, to put back if no run is made.
        let mut moved = Vec::new();
        for (i, event) in held.iter().take(in_full).enumerate() {
            carried.push(event.payload.clone());
            let names: Vec<String> = event
                .files
                .as_ref()
                .and_then(Value::as_array)
                .map(|names| {
                    names
                        .iter()
                        .filter_map(|n| n.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let files = self
                .files
                .take_held_files(&watcher.id, &event.id, &names, &run_id, i + 1);
            for file in files {
                moved.push((event.id.clone(), file.clone()));
                attachments.push(file);
            }
        }
        let payload = json!({
            "watcher_id": watcher.id,
            "watcher_name": watcher.name,
            "held": carried,
            "held_more": held.len() - in_full,
            "held_since": since,
            "attachments": attachments,
        });
        let mut new = NewRun::new(&watcher.dot_id, TriggerKind::Watcher);
        new.id = Some(run_id.clone());
        new.payload = Some(payload);
        // The run is made and the events taken in one step. The gate keeps any hold out since
        // they were read, so the events taken are the ones the run carries and counts.
        let ids: Vec<String> = held.iter().map(|e| e.id.clone()).collect();
        let (run, taken) = match store.open_held(&watcher.id, &ids, &new).await {
            Ok(opened) => opened,
            Err(e) => {
                // No run: the events stay held, with their files.
                self.files.return_held_files(&watcher.id, &run_id, &moved);
                return Err(e);
            }
        };
        self.deps.runner.enqueued(&run);
        for event in taken {
            self.files.remove_held(&watcher.id, &event.id);
        }
        store
            .touch_watcher_event(&watcher.id, &timestamp(now))
            .await?;
        self.updated(&watcher);
        Ok(Fired::Run(Box::new(run)))
    }

    /// Keeps an event for the watcher's opening run. Its files are staged under the held folder
    /// only while fewer than `HELD_IN_FULL` events are held, so a flood cannot fill the disk;
    /// the held row names them.
    async fn hold(
        &self,
        watcher: &Watcher,
        payload: &Value,
        files: &[(String, Vec<u8>)],
        now: DateTime<Utc>,
    ) -> Result<()> {
        let store = &self.deps.store;
        let staged = !files.is_empty() && store.held_count(&watcher.id).await? < HELD_IN_FULL;
        let names = staged.then(|| Value::from_iter(files.iter().map(|(name, _)| name.as_str())));
        let held = store
            .hold_event(&watcher.id, payload, names.as_ref(), now)
            .await?;
        if staged {
            if let Err(e) = self.files.stage_held(&watcher.id, &held.id, files) {
                // Not held after all: the source tries the event again.
                self.files.remove_held(&watcher.id, &held.id);
                store.unhold_event(&held.id).await?;
                return Err(e);
            }
        }
        self.updated(watcher);
        Ok(())
    }

    /// A source failed: a transient failure is retried by the backoff, until it has lasted
    /// `GIVE_UP_AFTER`; a permanent one pauses the watcher at once.
    pub async fn note_failure(&self, watcher_id: &str, failure: Failure) -> Result<Poll> {
        match failure {
            Failure::Permanent(error) => {
                self.pause(watcher_id, PausedReason::Error, Some(&error))
                    .await?;
                Ok(Poll::Stopped)
            }
            Failure::Transient(error) => {
                let now = self.deps.clock.now();
                let next = self
                    .backoffs
                    .lock()
                    .unwrap()
                    .entry(watcher_id.to_string())
                    .or_default()
                    .failed(now);
                match next {
                    Some(at) => {
                        let scrubbed = crate::vault::redact::global().scrub(&error).into_owned();
                        self.deps
                            .store
                            .set_watcher_error(watcher_id, Some(&scrubbed))
                            .await?;
                        let watcher = self.deps.store.get_watcher(watcher_id).await?;
                        self.updated(&watcher);
                        Ok(Poll::Next(at))
                    }
                    None => {
                        self.pause(watcher_id, PausedReason::Error, Some(&error))
                            .await?;
                        Ok(Poll::Stopped)
                    }
                }
            }
        }
    }

    /// A source worked: its backoff and its last error go.
    pub async fn note_success(&self, watcher_id: &str) -> Result<()> {
        self.backoffs.lock().unwrap().remove(watcher_id);
        let watcher = self.deps.store.get_watcher(watcher_id).await?;
        if watcher.last_error.is_some() && watcher.paused_reason.is_none() {
            self.deps.store.set_watcher_error(watcher_id, None).await?;
            self.updated(&watcher);
        }
        Ok(())
    }

    /// Checks a polling watcher once and says when to check next.
    pub async fn poll_now(&self, watcher_id: &str) -> Result<Poll> {
        let check = self.check_lock(watcher_id);
        let _one_at_a_time = check.lock().await;
        let watcher = self.deps.store.get_watcher(watcher_id).await?;
        let polled = match &watcher.config {
            WatcherConfig::Email(config) => email::poll(self, &watcher, config).await,
            WatcherConfig::Github(config) => github::poll(self, &watcher, config).await,
            WatcherConfig::Folder(_) => return Ok(Poll::Stopped),
        };
        match polled {
            Ok(poll) => Ok(poll),
            // A failure of Dots' own (the database, the vault, a full disk) is retried with the
            // backoff, as a source's is; the poll does not end silently.
            Err(e) => {
                self.note_failure(watcher_id, Failure::Transient(e.to_string()))
                    .await
            }
        }
    }

    /// Forgets what a GitHub watcher knew of its token and answers.
    /// Forgets watchers whose rows are gone with their dot: their staged files, their task
    /// and what the service kept for them.
    pub fn forget(&self, ids: &[String]) {
        for id in ids {
            self.files.forget_watcher(id);
            self.backoffs.lock().unwrap().remove(id);
            self.checks.lock().unwrap().remove(id);
            self.gates.lock().unwrap().remove(id);
            self.forget_remote(id);
            self.stop_task(id);
        }
        self.reload();
    }

    fn forget_remote(&self, id: &str) {
        self.logins.lock().unwrap().remove(id);
        self.etags.lock().unwrap().retain(|(w, _), _| w != id);
    }

    fn stop_task(&self, id: &str) {
        if let Some(task) = self.tasks.lock().unwrap().remove(id) {
            task.token.cancel();
        }
    }

    /// The watchers that should run: enabled, not paused by Dots, of an enabled dot, while Dots
    /// is not paused.
    async fn should_run(&self) -> Result<HashMap<String, Watcher>> {
        if self.is_paused() {
            return Ok(HashMap::new());
        }
        let enabled: std::collections::HashSet<String> = self
            .deps
            .store
            .list_dots()
            .await?
            .into_iter()
            .filter(|d| d.spec.enabled)
            .map(|d| d.id)
            .collect();
        Ok(self
            .deps
            .store
            .all_watchers()
            .await?
            .into_iter()
            .filter(|w| w.enabled && w.paused_reason.is_none() && enabled.contains(&w.dot_id))
            .map(|w| (w.id.clone(), w))
            .collect())
    }

    /// Stops the tasks of watchers that should not run or whose config or hours changed, and
    /// starts the missing ones. A watcher whose task starts (Dots starting, the watcher resumed,
    /// turned on or edited, Dots or its dot no longer paused) first releases what it holds if
    /// it is inside its hours (the release rule), before any new event of it can fire.
    async fn reconcile(self: &Arc<Self>, shutdown: &CancellationToken) -> Result<()> {
        let wanted = self.should_run().await?;
        let starting: Vec<Watcher> = {
            let mut tasks = self.tasks.lock().unwrap();
            tasks.retain(|id, task| {
                let keep = wanted
                    .get(id)
                    .is_some_and(|w| w.config == task.config && w.hours == task.hours);
                if !keep {
                    task.token.cancel();
                }
                keep
            });
            wanted
                .into_values()
                .filter(|w| !tasks.contains_key(&w.id))
                .collect()
        };
        for watcher in starting.iter().filter(|w| w.held_count > 0) {
            if let Err(e) = self.release(&watcher.id).await {
                tracing::warn!(watcher = %watcher.id, "releasing held events failed: {e}");
            }
        }
        let mut tasks = self.tasks.lock().unwrap();
        for watcher in starting {
            let id = watcher.id.clone();
            let token = shutdown.child_token();
            let hours = self.hours_of(&watcher).cloned();
            let started = match watcher.kind() {
                WatcherKind::Folder => {
                    let started = self.start_watch(&watcher, &token);
                    if let (true, Some(hours)) = (started, &hours) {
                        self.start_openings(&id, hours.clone(), &token);
                    }
                    started
                }
                WatcherKind::Email | WatcherKind::Github => {
                    self.start_polls(&id, hours, &token);
                    true
                }
            };
            if started {
                let task = Task {
                    config: watcher.config,
                    hours: watcher.hours,
                    token,
                };
                tasks.insert(id, task);
            }
        }
        Ok(())
    }

    /// A folder watcher with hours: at each opening, its dot runs once with what it held. An
    /// opening that fails with an error (not off or capped) is tried again after the backoff's
    /// waits (1, 2, 5, 10, then every 10 minutes), so few enough to stay under the flood cap,
    /// rather than at the next opening.
    fn start_openings(self: &Arc<Self>, id: &str, hours: WatchHours, token: &CancellationToken) {
        let this = self.clone();
        let id = id.to_string();
        let token = token.clone();
        tokio::spawn(async move {
            loop {
                let now = this.deps.clock.now();
                let Some(at) = this.next_opening(&id, &hours, now) else {
                    // No opening in the future: the hours count as any time, nothing to wait for.
                    token.cancelled().await;
                    break;
                };
                tokio::select! {
                    _ = this.deps.clock.sleep_until(at) => {}
                    _ = token.cancelled() => break,
                }
                let mut attempt = 0;
                while let Err(e) = this.open(&id).await {
                    tracing::warn!(watcher = %id, "the opening failed: {e}");
                    let wait = BACKOFF_MINUTES.get(attempt).copied().unwrap_or(10);
                    attempt += 1;
                    let retry = this.deps.clock.now() + TimeDelta::minutes(wait);
                    tokio::select! {
                        _ = this.deps.clock.sleep_until(retry) => {}
                        _ = token.cancelled() => return,
                    }
                }
            }
        });
    }

    /// An email or GitHub watcher's check at its opening: what it finds is held, then its dot
    /// runs once with that and everything held before.
    async fn opening_poll(&self, id: &str) -> Result<Poll> {
        let polled = {
            self.set_opening(id, true);
            let _at_opening = AtOpening {
                service: self,
                watcher_id: id,
            };
            self.poll_now(id).await
        };
        if let Err(e) = self.open(id).await {
            tracing::warn!(watcher = %id, "the opening failed: {e}");
        }
        polled
    }

    /// Whether the watcher holds events (a failure to tell counts as no).
    async fn holding(&self, id: &str) -> bool {
        self.deps.store.held_count(id).await.is_ok_and(|n| n > 0)
    }

    /// A folder watcher's OS watch; false when it could not start (its failure is reported).
    fn start_watch(self: &Arc<Self>, watcher: &Watcher, token: &CancellationToken) -> bool {
        let WatcherConfig::Folder(config) = &watcher.config else {
            return false;
        };
        match folder::start(
            Arc::downgrade(self),
            watcher.clone(),
            config.clone(),
            self.deps.acting.clone(),
            self.deps.clock.clone(),
            token.clone(),
        ) {
            Ok(()) => true,
            Err(failure) => {
                // Reported; a transient one is tried again when the backoff says.
                let this = self.clone();
                let id = watcher.id.clone();
                tokio::spawn(async move {
                    match this.note_failure(&id, failure).await {
                        Ok(Poll::Next(at)) => {
                            this.deps.clock.sleep_until(at).await;
                            this.reload();
                        }
                        Ok(Poll::Stopped) => {}
                        Err(e) => {
                            tracing::warn!(watcher = %id, "reporting a folder failure failed: {e}")
                        }
                    }
                });
                false
            }
        }
    }

    /// The ids of the watchers whose task runs.
    #[doc(hidden)]
    pub fn running(&self) -> Vec<String> {
        self.tasks.lock().unwrap().keys().cloned().collect()
    }

    /// A polling watcher's loop: check, then sleep until the time the check gave. Outside its
    /// hours it does not check (watch hours spec section 4.5): it sleeps until the opening and
    /// checks then; a wake-up that falls outside the hours (a backoff, the next minute) sleeps
    /// on until the opening.
    fn start_polls(
        self: &Arc<Self>,
        id: &str,
        hours: Option<WatchHours>,
        token: &CancellationToken,
    ) {
        let this = self.clone();
        let id = id.to_string();
        let token = token.clone();
        tokio::spawn(async move {
            loop {
                let now = this.deps.clock.now();
                let opening = hours.as_ref().and_then(|h| this.opens_at(&id, h, now));
                let polled = match opening {
                    Some(at) => {
                        tokio::select! {
                            _ = this.deps.clock.sleep_until(at) => {}
                            _ = token.cancelled() => break,
                        }
                        this.opening_poll(&id).await
                    }
                    // Inside with events still held (an opening that was off or capped): the
                    // check is the opening's, so they go first, with what it finds.
                    None if hours.is_some() && this.holding(&id).await => {
                        this.opening_poll(&id).await
                    }
                    None => this.poll_now(&id).await,
                };
                match polled {
                    Ok(Poll::Next(at)) => {
                        tokio::select! {
                            _ = this.deps.clock.sleep_until(at) => {}
                            _ = token.cancelled() => break,
                        }
                    }
                    Ok(Poll::Stopped) => break,
                    Err(e) => {
                        tracing::warn!(watcher = %id, "watcher poll failed: {e}");
                        break;
                    }
                }
            }
            // Ended on its own (its token is still live, so the entry is still this task's):
            // forget it, so a watcher that should still run is started again by the reload.
            if !token.is_cancelled() {
                this.tasks.lock().unwrap().remove(&id);
                this.reload();
            }
        });
    }
}
