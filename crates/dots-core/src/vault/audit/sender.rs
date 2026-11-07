//! Sending audit records (env vault spec §10, `docs/vault-audit-format.md`): each org's queued
//! lines go to the endpoint its policy names, at most `BATCH` in one `POST`, with the value of
//! its `token_entry` as the bearer token. A 2xx takes them off the queue; anything else leaves
//! them and waits before that org's next try, from `FIRST_RETRY` doubling up to `MAX_RETRY`.
//! New records go about `SEND_AFTER` after they are written. Neither the token nor an answer's
//! body is ever logged or shown.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::record::stamp;
use crate::vault::org::{OrgLookup, Orgs};

/// The most records in one request.
pub const BATCH: u64 = 500;
/// How long the sender waits after a record is written, so records that follow go with it.
pub const SEND_AFTER: Duration = Duration::seconds(5);
/// The wait after a first failure; each failure after it doubles the wait.
pub const FIRST_RETRY: Duration = Duration::seconds(30);
/// The longest wait between tries.
pub const MAX_RETRY: Duration = Duration::hours(1);
/// How far each wait is varied, either way.
pub const JITTER: f64 = 0.2;

/// The wait before the next try after `failures` failures in a row (1 after the first), spread
/// by `jitter`, from -1 to 1, by up to `JITTER` of it either way, and never over `MAX_RETRY`.
pub fn retry_after(failures: u32, jitter: f64) -> Duration {
    let doublings = failures.saturating_sub(1).min(20);
    let base = FIRST_RETRY.num_milliseconds() as f64 * f64::from(1u32 << doublings);
    let max = MAX_RETRY.num_milliseconds() as f64;
    let spread = base.min(max) * (1.0 + JITTER * jitter.clamp(-1.0, 1.0));
    Duration::milliseconds(spread.min(max).round() as i64)
}

/// A number from -1 to 1, from the random bits of a new ULID.
fn jitter() -> f64 {
    let bits = (ulid::Ulid::new().random() % 2_001) as f64;
    bits / 1_000.0 - 1.0
}

/// Posts a batch. Tests use a fake one.
#[async_trait]
pub trait AuditTransport: Send + Sync + std::fmt::Debug {
    /// Posts `body` to `endpoint` with `token` as the bearer token. `Err` is a short reason
    /// for the person: never the token, never the answer's body.
    async fn post(&self, endpoint: &str, token: &str, body: String) -> Result<(), String>;
}

/// Posts over HTTP with the vault's client: 20 seconds, no redirect (a redirect would take the
/// token to another host), the public and the Windows roots.
#[derive(Debug, Clone)]
pub struct HttpTransport {
    client: reqwest::Client,
}

impl HttpTransport {
    pub fn new(client: reqwest::Client) -> HttpTransport {
        HttpTransport { client }
    }
}

impl Default for HttpTransport {
    fn default() -> Self {
        HttpTransport::new(crate::vault::org::http_client())
    }
}

#[async_trait]
impl AuditTransport for HttpTransport {
    async fn post(&self, endpoint: &str, token: &str, body: String) -> Result<(), String> {
        let answer = self
            .client
            .post(endpoint)
            .header(reqwest::header::CONTENT_TYPE, "application/x-ndjson")
            .bearer_auth(token)
            .body(body)
            .send()
            .await;
        match answer {
            Ok(r) if r.status().is_success() => Ok(()),
            Ok(r) => Err(format!("the endpoint answered {}", r.status().as_u16())),
            Err(e) if e.is_timeout() => Err("no answer within 20 seconds".to_string()),
            Err(e) if e.is_connect() => Err("could not reach the endpoint".to_string()),
            Err(_) => Err("the send did not go through".to_string()),
        }
    }
}

/// How sending stands for one org, as its account page shows it. Times are RFC 3339.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SendState {
    pub last_sent_at: Option<String>,
    /// Why the last try failed; `None` once one works.
    pub last_error: Option<String>,
    pub failed_at: Option<String>,
    /// When the next try is due after a failure.
    pub next_try_at: Option<String>,
}

#[derive(Default)]
struct OrgState {
    shown: SendState,
    failures: u32,
    /// Set after a failure to send: no try before it.
    next_try: Option<DateTime<Utc>>,
}

/// The sender. One runs for the app, from `spawn`.
pub struct AuditSender {
    orgs: Orgs,
    transport: Arc<dyn AuditTransport>,
    states: Mutex<HashMap<String, OrgState>>,
    /// One pass at a time.
    one: tokio::sync::Mutex<()>,
}

impl std::fmt::Debug for AuditSender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuditSender").finish_non_exhaustive()
    }
}

impl AuditSender {
    pub fn new(orgs: Orgs, transport: Arc<dyn AuditTransport>) -> Arc<AuditSender> {
        Arc::new(AuditSender {
            orgs,
            transport,
            states: Mutex::default(),
            one: tokio::sync::Mutex::new(()),
        })
    }

    /// How sending stands for `org_id`.
    pub fn state(&self, org_id: &str) -> SendState {
        self.states()
            .get(org_id)
            .map(|s| s.shown.clone())
            .unwrap_or_default()
    }

    /// Sends what every org has queued, except an org whose next try is not due yet, and says
    /// how many records went. An org whose policy has no `audit` (or that is not joined) keeps
    /// its queue: that is no failure.
    pub async fn pass(&self) -> usize {
        let _one = self.one.lock().await;
        let waiting = match self.orgs.audit().store().audit_waiting().await {
            Ok(waiting) => waiting,
            Err(e) => {
                tracing::warn!("the audit queue could not be read: {e}");
                return 0;
            }
        };
        // An org that lost its target may have no queue left to come up in `waiting`.
        let stale: Vec<String> = self.states().keys().cloned().collect();
        for org_id in stale {
            if self.orgs.audit_target(&org_id).is_none() {
                self.clear_wait(&org_id);
            }
        }
        let now = self.orgs.audit().clock().now();
        let mut sent = 0;
        for (org_id, _) in waiting {
            let due = self
                .states()
                .get(&org_id)
                .and_then(|s| s.next_try)
                .is_none_or(|next| now >= next);
            if due {
                sent += self.send_org(&org_id).await;
            }
        }
        sent
    }

    /// Sends the org's queue, a batch at a time, until it is empty or a send fails.
    async fn send_org(&self, org_id: &str) -> usize {
        let Some(target) = self.orgs.audit_target(org_id) else {
            // Nothing to send to (removed, damaged, or no `audit` now): the queue stays, and
            // so does no wait or old error, which would be due again at once or show after a
            // rejoin.
            self.clear_wait(org_id);
            return 0;
        };
        let token = match self.orgs.lookup(org_id, &target.token_entry).await {
            OrgLookup::Value { value, .. } => value,
            other => {
                // No token while the account gives no values: the next record written (a
                // fetch that works) tries again, so no wait is set.
                let why = format!(
                    "the audit token {} is not available: {}",
                    target.token_entry,
                    no_value(&other)
                );
                self.update(org_id, |s, now| {
                    s.shown.last_error = Some(why);
                    s.shown.failed_at = Some(stamp(now));
                    s.shown.next_try_at = None;
                    s.next_try = None;
                });
                return 0;
            }
        };
        let store = self.orgs.audit().store();
        let mut sent = 0;
        loop {
            let batch = match store.audit_batch(org_id, BATCH).await {
                Ok(batch) => batch,
                Err(e) => {
                    tracing::warn!("org {org_id:?}: its audit queue could not be read: {e}");
                    self.fail(org_id, "the send did not go through".to_string());
                    return sent;
                }
            };
            let Some(&(through, _)) = batch.last() else {
                return sent;
            };
            let body: String = batch.iter().map(|(_, line)| format!("{line}\n")).collect();
            match self.transport.post(&target.endpoint, &token, body).await {
                Ok(()) => {
                    if let Err(e) = store.audit_sent(org_id, through).await {
                        tracing::warn!("org {org_id:?}: sent audit records stay queued: {e}");
                        self.fail(org_id, "the send did not go through".to_string());
                        return sent;
                    }
                    sent += batch.len();
                    self.update(org_id, |s, now| {
                        *s = OrgState {
                            shown: SendState {
                                last_sent_at: Some(stamp(now)),
                                ..SendState::default()
                            },
                            ..OrgState::default()
                        };
                    });
                    if (batch.len() as u64) < BATCH {
                        return sent;
                    }
                }
                Err(why) => {
                    tracing::warn!("org {org_id:?}: audit records were not sent: {why}");
                    self.fail(org_id, why);
                    return sent;
                }
            }
        }
    }

    /// Forgets the org's wait and its last error.
    fn clear_wait(&self, org_id: &str) {
        if let Some(s) = self.states().get_mut(org_id) {
            s.failures = 0;
            s.next_try = None;
            s.shown.last_error = None;
            s.shown.failed_at = None;
            s.shown.next_try_at = None;
        }
    }

    /// Counts a failure for the org and sets the wait before its next try.
    fn fail(&self, org_id: &str, why: String) {
        self.update(org_id, |s, now| {
            s.failures += 1;
            let next = now + retry_after(s.failures, jitter());
            s.shown.last_error = Some(why);
            s.shown.failed_at = Some(stamp(now));
            s.shown.next_try_at = Some(stamp(next));
            s.next_try = Some(next);
        });
    }

    /// The soonest try due after a failure, if any. A deadline that is not ahead has had its
    /// pass already, so it is no reason to wake again.
    fn next_due(&self) -> Option<DateTime<Utc>> {
        let now = self.orgs.audit().clock().now();
        self.states()
            .values()
            .filter_map(|s| s.next_try)
            .filter(|next| *next > now)
            .min()
    }

    /// Sends what is queued now, then again `SEND_AFTER` after each new record and when a
    /// failed org's wait ends, until `shutdown`.
    pub fn spawn(self: Arc<Self>, shutdown: CancellationToken) -> JoinHandle<()> {
        tokio::spawn(async move {
            let audit = self.orgs.audit().clone();
            let clock = audit.clock().clone();
            loop {
                let written = audit.written().notified();
                tokio::pin!(written);
                written.as_mut().enable();
                self.pass().await;
                let next = self.next_due();
                let retry = async {
                    match next {
                        Some(next) => clock.sleep_until(next).await,
                        None => std::future::pending().await,
                    }
                };
                tokio::select! {
                    _ = shutdown.cancelled() => return,
                    _ = retry => {}
                    _ = written => {
                        tokio::select! {
                            _ = shutdown.cancelled() => return,
                            _ = clock.sleep_until(clock.now() + SEND_AFTER) => {}
                        }
                    }
                }
            }
        })
    }

    fn states(&self) -> std::sync::MutexGuard<'_, HashMap<String, OrgState>> {
        self.states.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn update(&self, org_id: &str, change: impl FnOnce(&mut OrgState, DateTime<Utc>)) {
        let now = self.orgs.audit().clock().now();
        change(self.states().entry(org_id.to_string()).or_default(), now);
    }
}

/// Why an org gives no value for the token entry, in plain words.
fn no_value(lookup: &OrgLookup) -> &'static str {
    match lookup {
        OrgLookup::Value { .. } => "it has a value",
        OrgLookup::NotDefined => "the account has no such entry",
        OrgLookup::Expired => "the saved copy expired",
        OrgLookup::Revoked => "access was revoked",
        OrgLookup::PolicyInvalid(_) => "the policy is invalid",
        OrgLookup::Offline => "offline, with no saved copy",
        OrgLookup::Removed => "the account is not joined",
        OrgLookup::NotMember(_) => "this PC is not on the member list",
    }
}
