//! Hourly usage counts sent home (usage counts spec). Off until the person turns them on in
//! Settings with a destination address; then, shortly after each UTC hour ends, one report of
//! plain numbers for that hour is posted there. Nothing else is ever sent: no names, paths,
//! prompts, messages, model output, emails or account ids.
//!
//! The state is one marker in the `settings` table (`MARKER`), so no migration is needed.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, DurationRound, TimeDelta, Utc};
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::scheduler::Clock;
use crate::store::{HourCounts, Store};
use crate::{Error, Result};

/// The report's `schema`: raised whenever a key is added, removed or changes meaning.
pub const SCHEMA: u32 = 1;
/// The `settings` key the state is kept under.
pub const MARKER: &str = "usage_counts";
/// How far back missed hours are caught up; older ones are dropped.
pub const CATCH_UP_HOURS: i64 = 24;
/// How long one send may take, connecting included.
pub const SEND_TIMEOUT: Duration = Duration::from_secs(15);
/// How long after an hour ends its report is sent, so the runs that end on the hour are in.
pub const SEND_AFTER: TimeDelta = TimeDelta::minutes(2);
/// How long after a failed send the next try is.
pub const RETRY_AFTER: TimeDelta = TimeDelta::minutes(10);
/// The longest destination address.
pub const MAX_URL_LEN: usize = 2048;

/// What is kept between starts.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UsageCountsState {
    pub enabled: bool,
    /// Where reports are posted. Kept when turned off, for next time.
    pub url: Option<String>,
    /// A random UUID made when first turned on; "Reset ID" makes a new one.
    pub install_id: Option<String>,
    /// The first hour that may be sent: the first whole hour after it was turned on.
    pub first_hour: Option<DateTime<Utc>>,
    /// The start of the newest hour sent.
    pub last_sent_hour: Option<DateTime<Utc>>,
    /// When a report was last sent.
    pub last_sent_at: Option<DateTime<Utc>>,
    /// Why the last send failed; cleared by the next one that gets through.
    pub last_error: Option<String>,
}

/// One report, exactly as it is posted. Numbers only, besides the random install id, the
/// version of Dots and the hour.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub schema: u32,
    /// `None` only in a preview before it was first turned on.
    pub install_id: Option<String>,
    pub app_version: String,
    /// The hour's start, UTC.
    pub hour_start: DateTime<Utc>,
    pub runs_started: u64,
    pub runs_succeeded: u64,
    pub runs_failed: u64,
    pub messages_sent: u64,
    pub approvals_asked: u64,
    pub dots: u64,
}

impl Report {
    pub fn new(
        install_id: Option<String>,
        app_version: &str,
        hour_start: DateTime<Utc>,
        c: HourCounts,
    ) -> Report {
        Report {
            schema: SCHEMA,
            install_id,
            app_version: app_version.to_string(),
            hour_start,
            runs_started: c.runs_started,
            runs_succeeded: c.runs_succeeded,
            runs_failed: c.runs_failed,
            messages_sent: c.messages_sent,
            approvals_asked: c.approvals_asked,
            dots: c.dots,
        }
    }
}

/// Refuses a destination that is not `https://`, except `http://localhost` and
/// `http://127.0.0.1` for testing on this PC. An address with a user name or password is
/// refused too: it would be kept and shown in plain text.
pub fn check_url(url: &str) -> Result<()> {
    let invalid = |m: &str| Err(Error::Invalid(m.to_string()));
    let url = url.trim();
    if url.is_empty() {
        return invalid("Enter the address to send the counts to.");
    }
    if url.len() > MAX_URL_LEN {
        return invalid("That address is too long.");
    }
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return invalid("That is not a web address. It starts with https://");
    };
    let host = parsed.host_str().unwrap_or_default();
    if host.is_empty() {
        return invalid("That address has no server name.");
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return invalid("The address must not hold a user name or password.");
    }
    match parsed.scheme() {
        "https" => Ok(()),
        "http" if matches!(host, "localhost" | "127.0.0.1") => Ok(()),
        _ => invalid(
            "The address must start with https:// (http:// is allowed only for localhost or 127.0.0.1).",
        ),
    }
}

/// The start of the UTC hour `t` falls in.
pub fn hour_floor(t: DateTime<Utc>) -> DateTime<Utc> {
    t.duration_trunc(TimeDelta::hours(1))
        .expect("an hour divides any time")
}

/// The completed hours still to send at `now`, oldest first: from the first hour after the
/// last one sent (or the first hour after it was turned on), and no more than
/// `CATCH_UP_HOURS` back.
pub fn due_hours(state: &UsageCountsState, now: DateTime<Utc>) -> Vec<DateTime<Utc>> {
    if !state.enabled || state.url.is_none() {
        return Vec::new();
    }
    let Some(first) = state.first_hour else {
        return Vec::new();
    };
    let current = hour_floor(now);
    let mut from = first.max(current - TimeDelta::hours(CATCH_UP_HOURS));
    if let Some(last) = state.last_sent_hour {
        from = from.max(last + TimeDelta::hours(1));
    }
    let mut hours = Vec::new();
    let mut h = from;
    while h < current {
        hours.push(h);
        h += TimeDelta::hours(1);
    }
    hours
}

/// A random version 4 UUID.
pub fn new_install_id() -> String {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).expect("OS random number generator unavailable");
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// Posts a report. Tests use a fake one.
#[async_trait]
pub trait CountsSender: Send + Sync + std::fmt::Debug {
    /// `Err` holds a short reason, shown to the person as the last error.
    async fn send(&self, url: &str, report: &Report) -> std::result::Result<(), String>;
}

/// Posts the report as JSON over HTTP, with a time limit, following no redirect (a redirect
/// could take the counts to a server the person did not name).
#[derive(Debug, Clone)]
pub struct HttpSender {
    client: reqwest::Client,
    timeout: Duration,
}

impl HttpSender {
    pub fn new(timeout: Duration) -> HttpSender {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .connect_timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("the HTTP client builds");
        HttpSender { client, timeout }
    }
}

impl Default for HttpSender {
    fn default() -> Self {
        HttpSender::new(SEND_TIMEOUT)
    }
}

#[async_trait]
impl CountsSender for HttpSender {
    async fn send(&self, url: &str, report: &Report) -> std::result::Result<(), String> {
        let answer = self.client.post(url).json(report).send().await;
        match answer {
            Ok(r) if r.status().is_success() => Ok(()),
            Ok(r) => Err(format!("the server answered {}", r.status().as_u16())),
            Err(e) if e.is_timeout() => Err(format!(
                "no answer within {} seconds",
                self.timeout.as_secs_f32()
            )),
            Err(e) if e.is_connect() => Err("could not reach the server".to_string()),
            Err(_) => Err("the send did not go through".to_string()),
        }
    }
}

/// The usage counts: their state, the preview, and the loop that sends.
pub struct UsageCounts {
    store: Store,
    clock: Arc<dyn Clock>,
    sender: Arc<dyn CountsSender>,
    app_version: String,
    /// Makes each read-change-write of the state one step.
    lock: tokio::sync::Mutex<()>,
    /// Cancelled when the counts are turned off, so a send on its way stops at once.
    stop: Mutex<CancellationToken>,
    /// Wakes the loop after a change.
    wake: tokio::sync::Notify,
}

impl UsageCounts {
    pub fn new(
        store: Store,
        clock: Arc<dyn Clock>,
        sender: Arc<dyn CountsSender>,
        app_version: String,
    ) -> Arc<UsageCounts> {
        Arc::new(UsageCounts {
            store,
            clock,
            sender,
            app_version,
            lock: tokio::sync::Mutex::new(()),
            stop: Mutex::new(CancellationToken::new()),
            wake: tokio::sync::Notify::new(),
        })
    }

    /// The stored state; all off when nothing was stored (or what was stored is unreadable).
    pub async fn state(&self) -> Result<UsageCountsState> {
        Ok(match self.store.marker(MARKER).await? {
            Some(v) => serde_json::from_value(v).unwrap_or_else(|e| {
                tracing::warn!("ignoring the stored usage counts state: {e}");
                UsageCountsState::default()
            }),
            None => UsageCountsState::default(),
        })
    }

    async fn save(&self, state: &UsageCountsState) -> Result<()> {
        self.store
            .set_marker(MARKER, &serde_json::to_value(state)?, false)
            .await
    }

    /// Turns the counts on or off, and sets the destination when `url` is given (an empty
    /// one clears it while off). Turning on needs a valid destination; it makes the install
    /// id the first time, and the first hour sent is the next whole hour. Turning off clears
    /// what was still to send and stops a send on its way.
    pub async fn set(&self, enabled: bool, url: Option<String>) -> Result<UsageCountsState> {
        let _held = self.lock.lock().await;
        let mut state = self.state().await?;
        if let Some(url) = url {
            let url = url.trim().to_string();
            if url.is_empty() {
                if enabled {
                    check_url(&url)?;
                }
                state.url = None;
            } else {
                check_url(&url)?;
                state.url = Some(url);
            }
        }
        if enabled {
            let Some(url) = &state.url else {
                return Err(Error::Invalid(
                    "Enter the address to send the counts to first.".into(),
                ));
            };
            check_url(url)?;
            if !state.enabled {
                state.enabled = true;
                state.install_id.get_or_insert_with(new_install_id);
                state.first_hour = Some(hour_floor(self.clock.now()) + TimeDelta::hours(1));
                state.last_sent_hour = None;
                state.last_error = None;
            }
        } else {
            if state.enabled {
                let mut stop = self.stop.lock().expect("the stop lock is never poisoned");
                stop.cancel();
                *stop = CancellationToken::new();
            }
            state.enabled = false;
            state.first_hour = None;
            state.last_sent_hour = None;
            state.last_error = None;
        }
        self.save(&state).await?;
        self.wake.notify_waiters();
        Ok(state)
    }

    /// Makes a new install id. Before the counts were first turned on there is none, and none
    /// is made.
    pub async fn reset_id(&self) -> Result<UsageCountsState> {
        let _held = self.lock.lock().await;
        let mut state = self.state().await?;
        if state.install_id.is_some() {
            state.install_id = Some(new_install_id());
            self.save(&state).await?;
        }
        Ok(state)
    }

    /// The next report exactly as it would be posted: the oldest hour still to send, or else
    /// the next one, counted so far.
    pub async fn preview(&self) -> Result<Report> {
        let state = self.state().await?;
        let now = self.clock.now();
        let next = hour_floor(now) + TimeDelta::hours(1);
        let hour = match due_hours(&state, now).first() {
            Some(h) => *h,
            None if state.enabled => {
                let mut h = state.first_hour.unwrap_or(next);
                if let Some(last) = state.last_sent_hour {
                    h = h.max(last + TimeDelta::hours(1));
                }
                h
            }
            None => next,
        };
        self.report(&state, hour).await
    }

    async fn report(&self, state: &UsageCountsState, hour: DateTime<Utc>) -> Result<Report> {
        let counts = self.store.hour_counts(hour).await?;
        Ok(Report::new(
            state.install_id.clone(),
            &self.app_version,
            hour,
            counts,
        ))
    }

    /// Sends every completed hour still to send, oldest first, and returns how many went.
    /// It stops at the first failure, which is kept as the last error; that hour is sent on
    /// the next try.
    pub async fn pass(&self) -> Result<usize> {
        let stop = self
            .stop
            .lock()
            .expect("the stop lock is never poisoned")
            .clone();
        let mut sent = 0;
        loop {
            let state = self.state().await?;
            let now = self.clock.now();
            let (Some(hour), Some(url)) = (due_hours(&state, now).first().copied(), &state.url)
            else {
                return Ok(sent);
            };
            let report = self.report(&state, hour).await?;
            let result = tokio::select! {
                _ = stop.cancelled() => return Ok(sent),
                r = self.sender.send(url, &report) => r,
            };
            let _held = self.lock.lock().await;
            if stop.is_cancelled() {
                return Ok(sent);
            }
            let mut state = self.state().await?;
            if !state.enabled {
                return Ok(sent);
            }
            match result {
                Ok(()) => {
                    state.last_sent_hour = Some(hour);
                    state.last_sent_at = Some(self.clock.now());
                    state.last_error = None;
                    self.save(&state).await?;
                    sent += 1;
                }
                Err(e) => {
                    tracing::warn!("usage counts were not sent: {e}");
                    state.last_error = Some(e);
                    self.save(&state).await?;
                    return Ok(sent);
                }
            }
        }
    }

    /// Sends what is due, then sleeps until shortly after the current hour ends (or until a
    /// retry after a failure, or a change), until `shutdown`.
    pub fn spawn(self: Arc<Self>, shutdown: CancellationToken) -> JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                let changed = self.wake.notified();
                tokio::pin!(changed);
                changed.as_mut().enable();
                if let Err(e) = self.pass().await {
                    tracing::warn!("usage counts pass failed: {e}");
                }
                let now = self.clock.now();
                let state = self.state().await.unwrap_or_default();
                let next = if !state.enabled {
                    now + TimeDelta::days(1)
                } else if state.last_error.is_some() {
                    now + RETRY_AFTER
                } else {
                    hour_floor(now) + TimeDelta::hours(1) + SEND_AFTER
                };
                tokio::select! {
                    _ = shutdown.cancelled() => return,
                    _ = changed => {}
                    _ = self.clock.sleep_until(next) => {}
                }
            }
        })
    }
}
