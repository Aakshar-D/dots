//! Open Dot's thinking: one streamed request to the `claude` CLI per turn (no tools, no MCP
//! servers, none of the person's settings, one turn, in a new empty folder), and whether the
//! spending caps for every dot together let it think now.
//!
//! Nothing here logs what was asked or answered: only how a request ended.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Local, Utc};
use tokio_util::sync::CancellationToken;

use crate::engine::claude::once::{
    ask_once_streamed, ask_started_streamed, start_once_streamed, Once, OnceAnswer, OnceFailure,
    Started,
};
use crate::settings::Settings;
use crate::spend::{
    cost_reached, day_bounds_in, month_bounds_in, tokens_reached, GlobalCaps, Spend,
};
use crate::store::Store;
use crate::Result;

/// The model Open Dot thinks with unless Settings say otherwise.
pub const DEFAULT_MODEL: &str = "haiku";

/// The longest one request may take.
pub const THINK_LIMIT: Duration = Duration::from_secs(45);

/// A CLI started ahead of its request is used when it is no older than this: the CLI gives
/// up on a request that does not come within about three seconds.
pub const WARM_MOST: Duration = Duration::from_millis(2500);
/// One this young is kept when another is asked for: starting a second would gain nothing.
const WARM_FRESH: Duration = Duration::from_millis(1500);

/// Asks the model: the CLI in Dots, a stand-in in tests.
#[async_trait]
pub trait Thinker: Send + Sync {
    /// The model's reply to `prompt` under `system`, on `model`; `on_text` gets the reply's
    /// text as it comes, all of it in order.
    async fn think(
        &self,
        system: &str,
        prompt: &str,
        model: &str,
        cancel: &CancellationToken,
        on_text: &mut (dyn for<'s> FnMut(&'s str) + Send),
    ) -> std::result::Result<OnceAnswer, OnceFailure>;

    /// A request under `system` on `model` is likely to come within a moment (the person
    /// has stopped speaking): whatever can be got ready for it is. Asking for nothing costs
    /// nothing with the model. Does nothing by default.
    fn warm(&self, _system: &str, _model: &str) {}
}

/// The thinker of Dots: `ask_once_streamed` in a new empty folder under `dir` for each request,
/// removed afterwards, ended after `limit`.
pub struct CliThinker {
    program: Option<PathBuf>,
    env: Vec<(String, String)>,
    dir: PathBuf,
    limit: Duration,
    /// A CLI started ahead of the next request, if one was.
    warm: Arc<Mutex<Option<Warm>>>,
    next_warm: AtomicU64,
    warm_used: AtomicU64,
}

/// A CLI started ahead of its request, with the folder it runs in.
struct Warm {
    id: u64,
    // The CLI goes before its folder does.
    started: Started,
    folder: Folder,
    model: String,
    system: String,
}

impl CliThinker {
    /// `program` is the CLI the runtime found, `None` when there is none (`NoCli`).
    pub fn new(
        program: Option<PathBuf>,
        env: Vec<(String, String)>,
        dir: PathBuf,
        limit: Duration,
    ) -> Self {
        Self {
            program,
            env,
            dir,
            limit,
            warm: Arc::new(Mutex::new(None)),
            next_warm: AtomicU64::new(0),
            warm_used: AtomicU64::new(0),
        }
    }

    /// How many requests went to a CLI started ahead of them.
    pub fn warm_used(&self) -> u64 {
        self.warm_used.load(Ordering::SeqCst)
    }

    fn new_folder(&self) -> std::result::Result<Folder, OnceFailure> {
        let cwd = self.dir.join(crate::util::new_id());
        std::fs::create_dir_all(&cwd)
            .map_err(|e| OnceFailure::Failed(format!("could not make {}: {e}", cwd.display())))?;
        // Removed on every path: an answer, an error, this future dropped, a panic in `on_text`.
        Ok(Folder(cwd))
    }

    /// The CLI started ahead for a request under `system` on `model`, if there is one still
    /// of use. Any other one is ended.
    fn take_warm(&self, system: &str, model: &str) -> Option<Warm> {
        let mut warm = self.warm.lock().unwrap_or_else(|e| e.into_inner()).take()?;
        let fits = warm.model == model
            && warm.system == system
            && warm.started.age() < WARM_MOST
            && warm.started.running();
        fits.then_some(warm)
    }
}

#[async_trait]
impl Thinker for CliThinker {
    async fn think(
        &self,
        system: &str,
        prompt: &str,
        model: &str,
        cancel: &CancellationToken,
        on_text: &mut (dyn for<'s> FnMut(&'s str) + Send),
    ) -> std::result::Result<OnceAnswer, OnceFailure> {
        let Some(program) = self.program.as_deref() else {
            return Err(OnceFailure::NoCli);
        };
        let started = std::time::Instant::now();
        let asked = match self.take_warm(system, model) {
            Some(warm) => {
                self.warm_used.fetch_add(1, Ordering::SeqCst);
                let Warm {
                    started, folder, ..
                } = warm;
                let asked =
                    ask_started_streamed(started, prompt, self.limit, cancel, on_text).await;
                drop(folder);
                asked
            }
            None => {
                let folder = self.new_folder()?;
                let once = Once {
                    program,
                    env: &self.env,
                    cwd: &folder.0,
                    model,
                    system_prompt: system,
                    prompt,
                    limit: self.limit,
                };
                let asked = ask_once_streamed(&once, cancel, on_text).await;
                drop(folder);
                asked
            }
        };
        let ended = match &asked {
            Ok(_) => "answered",
            Err(OnceFailure::Cancelled) => "cancelled",
            Err(OnceFailure::TimedOut) => "timed out",
            Err(OnceFailure::Failed(_)) => "failed",
            Err(OnceFailure::NoCli) => "no CLI",
        };
        tracing::debug!(
            ms = started.elapsed().as_millis() as u64,
            ended,
            "open sage request"
        );
        asked
    }

    fn warm(&self, system: &str, model: &str) {
        let Some(program) = self.program.as_deref() else {
            return;
        };
        // The one that is let go after a while needs a runtime to wait on.
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let mut slot = self.warm.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(warm) = slot.as_mut() {
            if warm.model == model
                && warm.system == system
                && warm.started.age() < WARM_FRESH
                && warm.started.running()
            {
                return;
            }
        }
        let Ok(folder) = self.new_folder() else {
            return;
        };
        let Ok(started) = start_once_streamed(program, &self.env, &folder.0, model, system) else {
            return;
        };
        let id = self.next_warm.fetch_add(1, Ordering::SeqCst);
        // The one before it, if any, is ended here.
        *slot = Some(Warm {
            id,
            started,
            folder,
            model: model.to_string(),
            system: system.to_string(),
        });
        drop(slot);
        // No request came: it is ended, and its folder goes.
        let slot = Arc::downgrade(&self.warm);
        runtime.spawn(async move {
            tokio::time::sleep(WARM_MOST).await;
            let Some(slot) = slot.upgrade() else { return };
            let mut slot = slot.lock().unwrap_or_else(|e| e.into_inner());
            if slot.as_ref().is_some_and(|warm| warm.id == id) {
                *slot = None;
            }
        });
    }
}

/// A request's folder, removed when this is dropped. A CLI still ending may hold the folder
/// for a moment (on Windows a process's working folder cannot be removed), so a removal that
/// fails is tried again for a few seconds on a thread of its own.
struct Folder(PathBuf);

/// How long, and how often, a held folder's removal is tried again.
const REMOVE_TRIES: u32 = 50;
const REMOVE_PAUSE: Duration = Duration::from_millis(200);

impl Drop for Folder {
    fn drop(&mut self) {
        if std::fs::remove_dir_all(&self.0).is_ok() || !self.0.exists() {
            return;
        }
        let path = std::mem::take(&mut self.0);
        std::thread::spawn(move || {
            for _ in 0..REMOVE_TRIES {
                std::thread::sleep(REMOVE_PAUSE);
                if std::fs::remove_dir_all(&path).is_ok() || !path.exists() {
                    return;
                }
            }
            tracing::warn!("an Open Sage request folder could not be removed");
        });
    }
}

/// What Open Dot says when a cap for every dot together is reached, if one is: today's cost,
/// today's tokens, then this month's cost, read over the same local day and month the dots'
/// caps use. A dot's own caps never hold Open Dot.
pub async fn cap_hit(
    store: &Store,
    settings: &Settings,
    now: DateTime<Utc>,
) -> Result<Option<String>> {
    let caps = GlobalCaps::from(settings);
    if caps.daily_usd.is_some() || caps.daily_tokens.is_some() {
        let (start, end) = day_bounds_in(now, &Local);
        let today: Spend = store.spend_between(None, &start, &end).await?;
        if cost_reached(today.cost_usd, caps.daily_usd).is_some() {
            return Ok(Some(
                "You've reached today's spending limit, so I can't think right now.".into(),
            ));
        }
        if tokens_reached(today.tokens, caps.daily_tokens).is_some() {
            return Ok(Some(
                "You've reached today's token limit, so I can't think right now.".into(),
            ));
        }
    }
    if caps.monthly_usd.is_some() {
        let (start, end) = month_bounds_in(now, &Local);
        let month = store.spend_between(None, &start, &end).await?;
        if cost_reached(month.cost_usd, caps.monthly_usd).is_some() {
            return Ok(Some(
                "You've reached this month's spending limit, so I can't think right now.".into(),
            ));
        }
    }
    Ok(None)
}
