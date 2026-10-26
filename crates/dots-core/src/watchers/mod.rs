//! Watchers (watchers spec): what starts a dot when a file changes in a folder, an email
//! arrives in a Gmail label, or something happens on a GitHub repository. This module holds the
//! checks a watcher passes before it is saved, with the spec's sentences, and `Watchers`, the
//! service every event goes through (sections 5 and 7).

pub mod acting;
pub mod cap;
pub mod folder;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, TimeDelta, Utc};
use serde_json::Value;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub use acting::{ActingTracker, ACTING_GRACE};
pub use cap::{FLOOD_CAP, FLOOD_WINDOW};

use crate::events::{Bus, RuntimeEvent};
use crate::model::{
    EmailConfig, FolderConfig, GithubConfig, NewRun, PausedReason, Run, TriggerKind, Watcher,
    WatcherConfig, WatcherKind,
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

/// A watcher's name is 1 to 80 characters (spec section 3).
pub const MAX_NAME_CHARS: usize = 80;

pub const NAME_RULE: &str = "A watcher's name is 1 to 80 characters.";
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
}

/// When a polling watcher checks next.
#[derive(Debug, Clone, PartialEq)]
pub enum Poll {
    Next(DateTime<Utc>),
    Stopped,
}

/// Where email and GitHub watchers read; tests point them at fakes.
#[derive(Debug, Clone)]
pub struct Sources {
    pub github_api: String,
}

impl Default for Sources {
    fn default() -> Self {
        Sources {
            github_api: "https://api.github.com".to_string(),
        }
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
}

/// The watchers service (spec sections 5 and 7): one task per running watcher, the one door
/// every event goes through (`fire`), and where sources report failures.
pub struct Watchers {
    deps: WatcherDeps,
    paused: AtomicBool,
    reload: Notify,
    tasks: Mutex<HashMap<String, (WatcherConfig, CancellationToken)>>,
    backoffs: Mutex<HashMap<String, Backoff>>,
}

impl Watchers {
    pub fn new(deps: WatcherDeps) -> Arc<Watchers> {
        Arc::new(Watchers {
            deps,
            paused: AtomicBool::new(false),
            reload: Notify::new(),
            tasks: Mutex::new(HashMap::new()),
            backoffs: Mutex::new(HashMap::new()),
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
            for (_, (_, token)) in this.tasks.lock().unwrap().drain() {
                token.cancel();
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
        self.deps.store.list_watchers(dot_id).await
    }

    pub async fn add(&self, dot_id: &str, name: &str, config: WatcherConfig) -> Result<Watcher> {
        let name = check_name(name)?;
        let config = check_config(&self.deps.store, &config).await?;
        let watcher = self.deps.store.add_watcher(dot_id, &name, &config).await?;
        self.changed(&watcher);
        Ok(watcher)
    }

    pub async fn update(&self, id: &str, name: &str, config: WatcherConfig) -> Result<Watcher> {
        self.deps.store.get_watcher(id).await?;
        let name = check_name(name)?;
        let config = check_config(&self.deps.store, &config).await?;
        let watcher = self.deps.store.update_watcher(id, &name, &config).await?;
        self.backoffs.lock().unwrap().remove(id);
        self.changed(&watcher);
        Ok(watcher)
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        let watcher = self.deps.store.delete_watcher(id).await?;
        self.backoffs.lock().unwrap().remove(id);
        self.stop_task(id);
        self.changed(&watcher);
        Ok(())
    }

    pub async fn set_enabled(&self, id: &str, enabled: bool) -> Result<Watcher> {
        let watcher = self.deps.store.set_watcher_enabled(id, enabled).await?;
        self.changed(&watcher);
        Ok(watcher)
    }

    /// The person resumes a watcher Dots paused; nothing happens to one that was not paused.
    pub async fn resume(&self, id: &str) -> Result<Watcher> {
        let watcher = self.deps.store.get_watcher(id).await?;
        if watcher.paused_reason.is_none() {
            return Ok(watcher);
        }
        let watcher = self.deps.store.resume_watcher(id).await?;
        self.backoffs.lock().unwrap().remove(id);
        self.changed(&watcher);
        Ok(watcher)
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

    /// The one door every event goes through (spec section 5): one queued `watcher` run, unless
    /// the watcher, its dot or Dots is paused or off, or the hour is full.
    pub async fn fire(&self, watcher_id: &str, event: SourceEvent) -> Result<Fired> {
        let watcher = self.deps.store.get_watcher(watcher_id).await?;
        if self.is_paused() || !watcher.enabled || watcher.paused_reason.is_some() {
            return Ok(Fired::Off);
        }
        if !self.deps.store.get_dot(&watcher.dot_id).await?.spec.enabled {
            return Ok(Fired::Off);
        }
        let now = self.deps.clock.now();
        if !cap::take(&self.deps.store, &watcher.id, now).await? {
            self.pause(&watcher.id, PausedReason::Flood, None).await?;
            return Ok(Fired::Capped);
        }
        let mut payload = event.payload;
        if let Value::Object(map) = &mut payload {
            map.insert("watcher_id".into(), Value::String(watcher.id.clone()));
            map.insert("watcher_name".into(), Value::String(watcher.name.clone()));
        }
        let mut new = NewRun::new(&watcher.dot_id, TriggerKind::Watcher);
        new.id = Some(new_id());
        new.payload = Some(payload);
        let run = self.deps.runner.enqueue(new).await?;
        self.deps
            .store
            .touch_watcher_event(&watcher.id, &timestamp(now))
            .await?;
        self.updated(&watcher);
        Ok(Fired::Run(Box::new(run)))
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
        let watcher = self.deps.store.get_watcher(watcher_id).await?;
        match watcher.kind() {
            WatcherKind::Folder | WatcherKind::Email | WatcherKind::Github => Ok(Poll::Stopped),
        }
    }

    fn stop_task(&self, id: &str) {
        if let Some((_, token)) = self.tasks.lock().unwrap().remove(id) {
            token.cancel();
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

    /// Stops the tasks of watchers that should not run or whose config changed, and starts the
    /// missing ones.
    async fn reconcile(self: &Arc<Self>, shutdown: &CancellationToken) -> Result<()> {
        let wanted = self.should_run().await?;
        let mut tasks = self.tasks.lock().unwrap();
        tasks.retain(|id, (config, token)| {
            let keep = wanted.get(id).map(|w| &w.config) == Some(config);
            if !keep {
                token.cancel();
            }
            keep
        });
        for (id, watcher) in wanted {
            if tasks.contains_key(&id) {
                continue;
            }
            let token = shutdown.child_token();
            let started = match watcher.kind() {
                WatcherKind::Folder => self.start_watch(&watcher, &token),
                WatcherKind::Email | WatcherKind::Github => {
                    self.start_polls(&id, &token);
                    true
                }
            };
            if started {
                tasks.insert(id, (watcher.config, token));
            }
        }
        Ok(())
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
                let this = self.clone();
                let id = watcher.id.clone();
                tokio::spawn(async move {
                    if let Err(e) = this.note_failure(&id, failure).await {
                        tracing::warn!(watcher = %id, "reporting a folder failure failed: {e}");
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

    /// A polling watcher's loop: check, then sleep until the time the check gave.
    fn start_polls(self: &Arc<Self>, id: &str, token: &CancellationToken) {
        let this = self.clone();
        let id = id.to_string();
        let token = token.clone();
        tokio::spawn(async move {
            loop {
                match this.poll_now(&id).await {
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
        });
    }
}
