//! Writing audit records (env vault spec §10): each one gets this install's ids, the next
//! `seq` of its org and the hash of the line before, in one transaction. A record that cannot
//! be written is logged and never fails what it records.

use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::{Notify, OnceCell};

use super::record::{stamp, AuditDot, AuditRecord, AuditUser, Event, RECORD_VERSION};
use crate::scheduler::Clock;
use crate::store::{AuditLimits, NewAudit, Store};

/// What a record says about the org it is in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrgAudit {
    pub org_id: String,
    /// The org's source kind: `openbao`, `infisical` or `sops`; `None` for an org whose
    /// stored settings are damaged so badly that even the kind does not read.
    pub source: Option<String>,
    /// Who the source knows this PC as (`AuditUser::source_identity`).
    pub source_identity: Option<String>,
    /// Whether the org's accepted policy has `audit` (with none in memory, whether the last one
    /// accepted had): its records are queued and sent. Without it they are only listed in
    /// History.
    pub queue: bool,
}

/// What used a variable: a run of a dot, or neither for a Library check.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Usage {
    pub dot: Option<AuditDot>,
    pub run_id: Option<String>,
}

impl Usage {
    /// A run of the dot `dot_id`, named `dot_name`.
    pub fn run(dot_id: &str, dot_name: &str, run_id: &str) -> Usage {
        Usage {
            dot: Some(AuditDot {
                id: dot_id.to_string(),
                name: dot_name.to_string(),
            }),
            run_id: Some(run_id.to_string()),
        }
    }
}

/// What happened, for one record.
#[derive(Debug, Clone, Copy)]
pub struct Happened<'a> {
    pub event: Event,
    pub name: Option<&'a str>,
    pub consumer: Option<&'a str>,
    /// Where a resolved value came from; `None` for the org's source.
    pub source: Option<&'a str>,
    pub reason: Option<&'a str>,
}

impl<'a> Happened<'a> {
    /// A fetch outcome, with `reason`.
    pub fn fetch(event: Event, reason: Option<&'a str>) -> Happened<'a> {
        Happened {
            event,
            name: None,
            consumer: None,
            source: None,
            reason,
        }
    }
}

/// The Windows account Dots runs as, `DOMAIN\user` (the user alone without a domain).
pub fn windows_user() -> Option<String> {
    let user = std::env::var("USERNAME").ok().filter(|u| !u.is_empty())?;
    match std::env::var("USERDOMAIN").ok().filter(|d| !d.is_empty()) {
        Some(domain) => Some(format!("{domain}\\{user}")),
        None => Some(user),
    }
}

/// The PC's name.
pub fn machine_name() -> Option<String> {
    std::env::var("COMPUTERNAME").ok().filter(|m| !m.is_empty())
}

/// This install's ids, read once.
#[derive(Debug, Clone)]
struct Ids {
    install: String,
    windows: Option<String>,
    machine: Option<String>,
    dots_id: Option<String>,
}

/// The audit writer. Cheap to clone: every clone shares one state.
#[derive(Clone)]
pub struct Audit {
    inner: Arc<Inner>,
}

struct Inner {
    store: Store,
    clock: Arc<dyn Clock>,
    limits: Mutex<AuditLimits>,
    ids: OnceCell<Ids>,
    /// One record at a time, so two never take the same `seq`.
    write: tokio::sync::Mutex<()>,
    /// Woken after each queued record, for the sender.
    written: Notify,
}

impl std::fmt::Debug for Audit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Audit").finish_non_exhaustive()
    }
}

impl Audit {
    pub fn new(store: Store, clock: Arc<dyn Clock>) -> Audit {
        Audit {
            inner: Arc::new(Inner {
                store,
                clock,
                limits: Mutex::new(AuditLimits::default()),
                ids: OnceCell::new(),
                write: tokio::sync::Mutex::new(()),
                written: Notify::new(),
            }),
        }
    }

    /// Keeps `limits` from now on, for tests.
    pub fn set_limits(&self, limits: AuditLimits) {
        *self
            .inner
            .limits
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = limits;
    }

    pub fn limits(&self) -> AuditLimits {
        *self
            .inner
            .limits
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    pub fn store(&self) -> &Store {
        &self.inner.store
    }

    pub fn clock(&self) -> &Arc<dyn Clock> {
        &self.inner.clock
    }

    /// Woken after each record that is queued.
    pub fn written(&self) -> &Notify {
        &self.inner.written
    }

    /// Records `what` in `org`, for `usage`. A failure is logged, never returned.
    pub async fn record(&self, org: &OrgAudit, usage: &Usage, what: Happened<'_>) {
        if let Err(e) = self.write(org, usage, what).await {
            tracing::warn!(
                "org {:?}: an audit record ({}) was not written: {e}",
                org.org_id,
                what.event.as_str()
            );
        }
    }

    async fn write(&self, org: &OrgAudit, usage: &Usage, what: Happened<'_>) -> crate::Result<()> {
        let ids = self.ids().await?;
        let record = AuditRecord {
            v: RECORD_VERSION,
            id: crate::util::new_id(),
            seq: 0,
            prev: None,
            at: stamp(self.inner.clock.now()),
            org_id: org.org_id.clone(),
            install: ids.install.clone(),
            user: AuditUser {
                windows: ids.windows.clone(),
                machine: ids.machine.clone(),
                source_identity: org.source_identity.clone(),
                dots_id: ids.dots_id.clone(),
            },
            dot: usage.dot.clone(),
            run_id: usage.run_id.clone(),
            event: what.event,
            name: what.name.map(str::to_string),
            consumer: what.consumer.map(str::to_string),
            source: what
                .source
                .map(str::to_string)
                .or_else(|| org.source.clone()),
            reason: what.reason.map(str::to_string),
            dropped: None,
        };
        let new = NewAudit {
            record,
            queue: org.queue,
            history: what.event != Event::Fetched,
        };
        let limits = self.limits();
        let _one = self.inner.write.lock().await;
        self.inner.store.append_audit(new, &limits).await?;
        if org.queue {
            self.inner.written.notify_one();
        }
        Ok(())
    }

    async fn ids(&self) -> crate::Result<&Ids> {
        self.inner
            .ids
            .get_or_try_init(|| async {
                let store = &self.inner.store;
                let dots_id = match crate::accounts::identity(store).await {
                    Ok(identity) => Some(identity.fingerprint),
                    Err(_) => None,
                };
                Ok::<_, crate::Error>(Ids {
                    install: store.install_id().await?,
                    windows: windows_user(),
                    machine: machine_name(),
                    dots_id,
                })
            })
            .await
    }
}
