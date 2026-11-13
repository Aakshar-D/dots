//! Open Dot's thinking: one streamed request to the `claude` CLI per turn (no tools, no MCP
//! servers, none of the person's settings, one turn, in a new empty folder), and whether the
//! spending caps for every dot together let it think now.
//!
//! Nothing here logs what was asked or answered: only how a request ended.

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Local, Utc};
use tokio_util::sync::CancellationToken;

use crate::engine::claude::once::{ask_once_streamed, Once, OnceAnswer, OnceFailure};
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
}

/// The thinker of Dots: `ask_once_streamed` in a new empty folder under `dir` for each request,
/// removed afterwards, ended after `limit`.
pub struct CliThinker {
    program: Option<PathBuf>,
    env: Vec<(String, String)>,
    dir: PathBuf,
    limit: Duration,
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
        }
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
        let cwd = self.dir.join(crate::util::new_id());
        if let Err(e) = std::fs::create_dir_all(&cwd) {
            return Err(OnceFailure::Failed(format!(
                "could not make {}: {e}",
                cwd.display()
            )));
        }
        // Removed on every path: an answer, an error, this future dropped, a panic in `on_text`.
        let folder = Folder(cwd);
        let once = Once {
            program,
            env: &self.env,
            cwd: &folder.0,
            model,
            system_prompt: system,
            prompt,
            limit: self.limit,
        };
        let started = std::time::Instant::now();
        let asked = ask_once_streamed(&once, cancel, on_text).await;
        drop(folder);
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
