//! The orgs joined on this PC, held in memory (env vault spec §6.1, §6.2 and §6.4): joining one
//! with a join code, changing its settings, leaving it, and fetching its entries and policy,
//! with what each outcome of a fetch does to the offline copy. The resolver reads org values
//! from here with `view`: the policy and the values of one fetch, together.
//!
//! Each org has a status. A fetch that works makes it `Connected`. A source that refuses the
//! person's sign-in details makes it `Revoked`, and wipes its values. A source that cannot be
//! reached makes it `Offline`, and one that answers with something Dots cannot use, or with a
//! policy Dots refuses, makes it `PolicyInvalid`: both keep the last good copy until
//! `offline_hours` after the last good fetch, when it becomes `Expired`. A Connected org's values
//! do not run out: a run's start refreshes an org whose last good fetch is older than
//! `refresh_minutes` first, and a failed fetch then applies the expiry.
//!
//! The values sit behind a `RwLock` that no one holds across an `await`. Changes to one org
//! (join, update, refresh, remove) take that org's own async lock, so two of them never
//! interleave, while lookups and listings go on during a fetch.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use zeroize::Zeroizing;

use super::join::{check_org_id, key_fingerprint, parse_join_code, BaoAuth, JoinCode};
use super::offline::{delete_offline, read_offline, write_offline, OfflineCopy, Protector};
use super::policy::{OrgPolicy, PersonalMode, PolicyError, UseRules};
use super::row::OrgRow;
use super::signed::verify_policy;
use super::sources::sops::{self, NOT_A_RECIPIENT};
use super::sources::{source_for, Fetched, Source, SourceCreds, SourceError};
use super::SourceSettings;
use crate::model::DotRef;
use crate::scheduler::Clock;
use crate::store::Store;
use crate::vault::entries::EntryKind;
use crate::vault::grants::dots_granting;
use crate::vault::redact;
use crate::vault::resolve::DotScope;
use crate::vault::secrets::SecretStore;
use crate::{Error, Result};

/// Makes the source adapter for an org's settings. The default is `source_for` with the
/// service's HTTP client; a test can pass a source that answers as it likes.
pub type SourceFactory = Arc<dyn Fn(&SourceSettings) -> Box<dyn Source> + Send + Sync>;

/// What `status_text` says for a SOPS join that waits for the admin.
const PENDING_TEXT: &str = "waiting for the admin to add this PC's key";

/// How long `refresh_if_stale` waits after a failed fetch before it tries again. Without it, a
/// source that hangs would make every run start wait for the HTTP timeout. **Refresh** and the
/// timers do not wait.
pub const RETRY_AFTER: Duration = Duration::minutes(5);

/// How the line of `JoinPreview::changes` that warns of a new policy key starts. The app asks
/// the person before an Update that brings one.
pub const KEY_CHANGE: &str = "the policy key changes";

/// How far ahead of the clock a saved copy's fetch time may be at load. Further ahead, the clock
/// was wrong when the copy was written (or has been put back since), so `offline_hours` can't be
/// counted from it and the copy is treated as expired.
const COPY_CLOCK_SKEW: Duration = Duration::minutes(5);

/// Where an org stands. It is tagged `status`, so an `OrgSummary` reads
/// `"status": "offline", "until": "…"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum OrgStatus {
    /// The last fetch worked.
    Connected,
    /// The source could not be reached. The org resolves from its offline copy until `until`,
    /// RFC 3339; `None` when there is no copy.
    Offline { until: Option<String> },
    /// The source refused the person's sign-in details. The copy and the values are wiped.
    Revoked,
    /// The source answered with something Dots cannot use, the policy was refused, or the org's
    /// settings are damaged. The last good copy, if any, is used until it expires.
    PolicyInvalid { error: String },
    /// The offline copy ran out: `offline_hours` passed since the last good fetch.
    Expired,
    /// A SOPS join waiting for the admin to add this PC's key as a recipient. The org is not
    /// joined, so `summaries` leaves it out.
    Pending,
}

/// An org as Settings shows it. It never holds a value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrgSummary {
    pub id: String,
    pub name: String,
    /// `openbao`, `infisical` or `sops`; empty when the org's settings are damaged.
    pub source_kind: String,
    /// The address, the site or the file location.
    pub address: String,
    /// The policy key's id, 16 upper-case hex digits.
    pub key_fingerprint: String,
    #[serde(flatten)]
    pub status: OrgStatus,
    /// The status in plain words, with why the last fetch failed.
    pub status_text: String,
    /// The last good fetch, RFC 3339.
    pub last_fetch_at: Option<String>,
    /// Until when the offline copy may be used, RFC 3339; `None` when there is no copy.
    pub copy_until: Option<String>,
    /// The accepted policy in plain words (`OrgPolicy::summary`).
    pub policy_summary: Vec<String>,
    /// The accepted policy's `personal` mode, which says whether a dot of the org may read a
    /// Personal entry (the dot editor's name pickers); `None` while no policy is accepted.
    pub personal_mode: Option<PersonalMode>,
    /// The accepted policy's `shell_secrets`: whether a dot's shell may be given a secret.
    /// False while no policy is accepted.
    pub shell_secrets: bool,
    /// The accepted policy's `use` globs, which say which names a dot of the org may be granted
    /// (the shell's name picker leaves out the rest); `None` while no policy is accepted.
    pub use_rules: Option<UseRules>,
}

/// What a join code would join or change, shown before anything is fetched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinPreview {
    pub org_id: String,
    pub name: String,
    pub source_kind: String,
    pub address: String,
    pub key_fingerprint: String,
    /// How an OpenBao code signs in, so the UI asks for a token or a username and password.
    /// `None` for Infisical (a machine identity) and SOPS (this PC's key).
    pub auth: Option<BaoAuth>,
    /// The org is joined already: the code goes through **Update**.
    pub existing: bool,
    /// For **Update**, what the code changes, in plain words. A new policy key gets a warning.
    pub changes: Vec<String>,
    /// For **Update**, the code brings another policy key: the last line of `changes` warns of
    /// it.
    pub key_changed: bool,
    /// For **Update**, the code changes an OpenBao or Infisical source's settings, or the kind
    /// of source: the saved sign-in details are not sent to it, so the person gives them again.
    /// A SOPS file's new location keeps this PC's key.
    pub source_changed: bool,
}

/// One org entry for the Env tab. A secret's `value` is always `None`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrgEntry {
    pub name: String,
    pub kind: EntryKind,
    pub value: Option<String>,
    /// A Personal entry of the same name can't take its place: the policy's `personal` is
    /// `deny`, or `fallback` (the org has the name).
    pub locked: bool,
    /// The org's dots that grant this name (the Env tab's "used by").
    pub used_by: Vec<DotRef>,
}

/// An org's answer for one name, for the resolver. Every case but `Value` and `NotDefined` is
/// why the org can't give values at all, one per `Unresolved` reason.
#[derive(Clone, PartialEq, Eq)]
pub enum OrgLookup {
    /// The org's value, and whether its policy makes the entry plain.
    Value {
        value: Zeroizing<String>,
        is_plain: bool,
    },
    /// The org gives values, but has no entry of that name.
    NotDefined,
    /// The offline copy ran out.
    Expired,
    /// The source refused the person's sign-in details.
    Revoked,
    /// No accepted policy and no good copy: the error, for "policy invalid: <error>".
    PolicyInvalid(String),
    /// The source can't be reached and there is no copy.
    Offline,
    /// No org of that id is joined on this PC.
    Removed,
}

impl std::fmt::Debug for OrgLookup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Value { is_plain, .. } => f
                .debug_struct("Value")
                .field("is_plain", is_plain)
                .finish_non_exhaustive(),
            Self::NotDefined => f.write_str("NotDefined"),
            Self::Expired => f.write_str("Expired"),
            Self::Revoked => f.write_str("Revoked"),
            Self::PolicyInvalid(error) => f.debug_tuple("PolicyInvalid").field(error).finish(),
            Self::Offline => f.write_str("Offline"),
            Self::Removed => f.write_str("Removed"),
        }
    }
}

/// An org's accepted policy and its answers for some names, read together (`Orgs::view`), so
/// a refresh can't pair a value of one fetch with the policy of another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrgView {
    /// The policy accepted with the values; `None` when the org has none (not joined, or its
    /// settings are damaged).
    pub policy: Option<OrgPolicy>,
    /// One answer per name asked, in the order asked, as `lookup` gives it.
    pub lookups: Vec<OrgLookup>,
}

/// The orgs joined on this PC. Cheap to clone: every clone shares one state.
#[derive(Clone)]
pub struct Orgs {
    inner: Arc<Inner>,
}

struct Inner {
    store: Store,
    secrets: Arc<dyn SecretStore>,
    protector: Arc<dyn Protector>,
    /// `data_dir/vault`, where the offline copies are.
    dir: PathBuf,
    clock: Arc<dyn Clock>,
    sources: RwLock<SourceFactory>,
    orgs: RwLock<BTreeMap<String, Slot>>,
    /// One lock per org id, taken by whatever changes that org.
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// When each org's last fetch failed, if the last one did. In memory only.
    failed_at: Mutex<HashMap<String, DateTime<Utc>>>,
}

enum Slot {
    Joined(Box<Joined>),
    /// A row whose settings don't read. It gives no values until it is removed.
    Damaged {
        name: String,
        error: String,
    },
    /// A SOPS join that waits for the admin, with the code it was tried with. In memory only.
    Pending(JoinCode),
}

struct Joined {
    row: OrgRow,
    /// Kept, so a source's login is reused from one fetch to the next.
    source: Arc<dyn Source>,
    /// The policy accepted with `entries`, or with the last good fetch.
    policy: Option<OrgPolicy>,
    /// The values the org gives now; `None` when it gives none.
    entries: Option<BTreeMap<String, Zeroizing<String>>>,
    /// When the offline copy on disk runs out; `None` when there is none. Only set with
    /// `entries`.
    copy_until: Option<DateTime<Utc>>,
    status: OrgStatus,
    /// Why the last fetch failed, for `Offline` and `Revoked`.
    reason: Option<String>,
}

/// What one fetch came to.
enum Outcome {
    Accepted(Fetched, Box<OrgPolicy>),
    Denied(String),
    Unreachable(String),
    Invalid(String),
}

impl std::fmt::Debug for Orgs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ids: Vec<String> = self.read().keys().cloned().collect();
        f.debug_struct("Orgs").field("orgs", &ids).finish()
    }
}

impl Orgs {
    /// The service, with no org loaded yet: `load` reads them. `dir` is `data_dir/vault`.
    pub fn new(
        store: Store,
        secrets: Arc<dyn SecretStore>,
        protector: Arc<dyn Protector>,
        dir: PathBuf,
        http: reqwest::Client,
        clock: Arc<dyn Clock>,
    ) -> Orgs {
        let sources: SourceFactory =
            Arc::new(move |settings: &SourceSettings| source_for(settings, http.clone()));
        Orgs {
            inner: Arc::new(Inner {
                store,
                secrets,
                protector,
                dir,
                clock,
                sources: RwLock::new(sources),
                orgs: RwLock::default(),
                locks: Mutex::default(),
                failed_at: Mutex::default(),
            }),
        }
    }

    /// Makes every org's source with `sources` from now on, for tests.
    pub fn with_sources(self, sources: SourceFactory) -> Orgs {
        *self
            .inner
            .sources
            .write()
            .unwrap_or_else(PoisonError::into_inner) = sources;
        self
    }

    /// Reads every joined org and its offline copy, at start. A copy that ran out is deleted,
    /// and its org is `Expired`; one that holds is used, `Offline` until the first fetch. Each
    /// row is read on its own, so a damaged one shows as `PolicyInvalid` and does not stop the
    /// others. Temp files a crash left beside the copies are deleted. Every secret value read
    /// is added to the redactor.
    pub async fn load(&self) -> Result<()> {
        let dir = self.inner.dir.clone();
        blocking(move || {
            sweep_temp(&dir);
            Ok(())
        })
        .await?;
        let mut loaded = BTreeMap::new();
        for (id, name) in self.inner.store.list_org_names().await? {
            let slot = match self.inner.store.get_org(&id).await {
                Ok(Some(row)) => Slot::Joined(Box::new(self.load_one(row).await)),
                Ok(None) => continue,
                Err(e) => {
                    tracing::warn!("org {id:?} is not loaded: {e}");
                    Slot::Damaged {
                        name,
                        error: format!("its settings are damaged: {e}"),
                    }
                }
            };
            loaded.insert(id, slot);
        }
        *self.write() = loaded;
        Ok(())
    }

    /// What `code` would join, or change for an org already joined. Nothing is fetched.
    pub fn preview_join(&self, code: &str) -> Result<JoinPreview> {
        let code = parse_join_code(code)?;
        let (existing, changes, key_changed, source_changed) = match self.read().get(&code.org_id) {
            Some(Slot::Joined(joined)) => (
                true,
                changes(&joined.row, &code),
                joined.row.policy_key != code.policy_key,
                source_changed(&joined.row.source, &code.source),
            ),
            Some(Slot::Damaged { .. }) => (true, vec![damaged_text(&code.org_id)], false, false),
            Some(Slot::Pending(_)) | None => (false, Vec::new(), false, false),
        };
        Ok(JoinPreview {
            key_fingerprint: key_fingerprint(&code.policy_key)?,
            source_kind: code.source.kind().to_string(),
            address: code.source.address().to_string(),
            auth: match &code.source {
                SourceSettings::OpenBao(s) => Some(s.auth),
                _ => None,
            },
            org_id: code.org_id,
            name: code.name,
            existing,
            changes,
            key_changed,
            source_changed,
        })
    }

    /// This PC's age public key for the SOPS org `org_id`, for the person to send to the admin.
    /// The first call makes the identity and keeps it as the org's sign-in details; later calls
    /// give the same key.
    pub async fn sops_identity(&self, org_id: &str) -> Result<String> {
        check_org_id(org_id)?;
        let _guard = self.lock(org_id).await;
        let not_sops = matches!(
            self.read().get(org_id),
            Some(Slot::Joined(j)) if !matches!(j.row.source, SourceSettings::Sops(_))
        );
        if not_sops {
            return Err(Error::Invalid(format!(
                "org '{org_id}' does not read a SOPS file, so it has no age key"
            )));
        }
        let secrets = self.inner.secrets.as_ref();
        if let Some(SourceCreds::Age { identity }) = SourceCreds::load(secrets, org_id)? {
            return sops::public_key(&identity);
        }
        let (identity, public) = sops::new_identity();
        SourceCreds::Age { identity }.save(secrets, org_id)?;
        Ok(public)
    }

    /// Joins the org of `code` with the person's sign-in details, in this order: the org must
    /// not be joined yet, the details are stored, the source is fetched, the policy verified,
    /// and the row, the copy and the values are kept. If the fetch or the policy fails, the
    /// details are taken out again and nothing is kept, except a SOPS org's age key, which the
    /// person has already sent to the admin. `creds` may be `None` only for a SOPS org, which
    /// uses its stored key. A SOPS file not yet encrypted to that key is not an error: the
    /// answer's status is `Pending`, and nothing is joined until a later join opens the file. A
    /// dot left by an earlier join of the same id belongs to it again.
    pub async fn join(&self, code: &str, creds: Option<SourceCreds>) -> Result<OrgSummary> {
        let code = parse_join_code(code)?;
        let id = code.org_id.clone();
        let _guard = self.lock(&id).await;
        let names = self.inner.store.list_org_names().await?;
        if names.iter().any(|(joined, _)| *joined == id) {
            return Err(Error::Conflict(format!(
                "the org '{id}' is already joined on this PC; use Update to change its settings"
            )));
        }
        let sops = matches!(code.source, SourceSettings::Sops(_));
        let secrets = self.inner.secrets.as_ref();
        let creds = match creds {
            Some(creds) => {
                check_creds_fit(&code.source, &creds)?;
                creds.save(secrets, &id)?;
                creds
            }
            None if sops => match SourceCreds::load(secrets, &id)? {
                Some(creds @ SourceCreds::Age { .. }) => creds,
                _ => {
                    return Err(Error::Invalid(format!(
                        "this PC has no age key for org '{id}' yet; make one, send it to your \
                         admin, then join"
                    )))
                }
            },
            None => {
                return Err(Error::Invalid(format!(
                    "joining {} needs your sign-in details for its {}",
                    code.name,
                    kind_name(&code.source)
                )))
            }
        };

        let source = self.source(&code.source);
        let fetched = match source.fetch(&creds).await {
            Ok(fetched) => fetched,
            Err(SourceError::Denied(why)) if sops && why == NOT_A_RECIPIENT => {
                let slot = Slot::Pending(code);
                let summary = summary_of(&id, &slot);
                self.write().insert(id, slot);
                return Ok(summary);
            }
            Err(e) => return Err(self.join_failed(&code, e.to_string())),
        };
        let policy = match verify(&code.policy_key, &id, None, &fetched) {
            Ok(policy) => policy,
            Err(e) => return Err(self.join_failed(&code, format!("policy invalid: {e}"))),
        };

        let now = self.inner.clock.now();
        let row = OrgRow {
            id: id.clone(),
            name: code.name.clone(),
            source: code.source.clone(),
            policy_key: code.policy_key.clone(),
            last_issued_at: Some(policy.issued_at.clone()),
            last_fetch_at: Some(stamp(now)),
            joined_at: stamp(now),
        };
        if let Err(e) = self.inner.store.add_org(&row).await {
            self.forget_creds(&code);
            return Err(e);
        }
        self.settle(row, source, fetched, policy, now).await;
        self.summary_now(&id).ok_or_else(|| not_joined(&id))
    }

    /// Replaces a joined org's settings with those of a new join code for the same id (§6.4,
    /// **Update**). It fetches with the new settings first, with `creds`, or the stored details
    /// when `None`, and checks the policy as a refresh does. Only when that works are the
    /// settings and the details replaced. A failure leaves the old ones, and their values,
    /// working. Dots and grants are not touched. The stored details are never sent to a changed
    /// source: when the code changes an OpenBao or Infisical source's settings, or the kind of
    /// source, `creds` must be given, and nothing is sent without them. A SOPS file's new
    /// location keeps this PC's age key, which goes nowhere.
    pub async fn update(
        &self,
        org_id: &str,
        code: &str,
        creds: Option<SourceCreds>,
    ) -> Result<OrgSummary> {
        let code = parse_join_code(code)?;
        if code.org_id != org_id {
            return Err(Error::Invalid(format!(
                "this join code is for org '{}', not '{org_id}'",
                code.org_id
            )));
        }
        let _guard = self.lock(org_id).await;
        let old = match self.read().get(org_id) {
            Some(Slot::Joined(joined)) => joined.row.clone(),
            Some(Slot::Damaged { .. }) => return Err(Error::Invalid(damaged_text(org_id))),
            Some(Slot::Pending(_)) | None => return Err(not_joined(org_id)),
        };
        // The saved details were given for the old source: a new address (or anything else
        // about where and how they are sent) must not receive them before the policy's
        // signature is checked.
        if creds.is_none() && source_changed(&old.source, &code.source) {
            return Err(Error::Invalid(format!(
                "{}: the source changed; enter the sign-in details again",
                old.name
            )));
        }
        let secrets = self.inner.secrets.as_ref();
        let given = creds.is_some();
        // Damaged saved details don't stop an Update that brings new ones.
        let old_creds = match SourceCreds::load(secrets, org_id) {
            Ok(old) => old,
            Err(e) if given => {
                tracing::warn!("org {org_id:?}: its saved sign-in details are replaced: {e}");
                None
            }
            Err(e) => return Err(e),
        };
        let creds = match creds.or_else(|| old_creds.clone()) {
            Some(creds) => creds,
            None => {
                return Err(Error::Invalid(format!(
                    "{} has no saved sign-in details; give them with the new code",
                    old.name
                )))
            }
        };
        check_creds_fit(&code.source, &creds)?;

        let failed = |why: String| Error::Other(format!("could not update {}: {why}", code.name));
        let source = self.source(&code.source);
        let fetched = source
            .fetch(&creds)
            .await
            .map_err(|e| failed(e.to_string()))?;
        let policy = verify(
            &code.policy_key,
            org_id,
            old.last_issued_at.as_deref(),
            &fetched,
        )
        .map_err(|e| failed(format!("policy invalid: {e}")))?;

        if given {
            creds.save(secrets, org_id)?;
        }
        let now = self.inner.clock.now();
        let row = OrgRow {
            name: code.name.clone(),
            source: code.source.clone(),
            policy_key: code.policy_key.clone(),
            last_issued_at: Some(policy.issued_at.clone()),
            last_fetch_at: Some(stamp(now)),
            ..old.clone()
        };
        if let Err(e) = self.store_settings(&row, &old).await {
            if given {
                restore_creds(secrets, org_id, old_creds.as_ref());
            }
            return Err(e);
        }
        self.settle(row, source, fetched, policy, now).await;
        self.summary_now(org_id).ok_or_else(|| not_joined(org_id))
    }

    /// Leaves an org (§6.4, **Remove**): wipes its offline copy, its sign-in details and its
    /// row. Its dots keep their `org_id`, and resolve as removed until it is joined again. It
    /// also takes back the age key of a SOPS org that was never joined.
    pub async fn remove(&self, org_id: &str) -> Result<()> {
        check_org_id(org_id)?;
        let _guard = self.lock(org_id).await;
        let names = self.inner.store.list_org_names().await?;
        let has_row = names.iter().any(|(id, _)| id == org_id);
        let has_creds = self
            .inner
            .secrets
            .get(&SourceCreds::store_key(org_id))?
            .is_some();
        if !has_row && !has_creds && !self.read().contains_key(org_id) {
            return Err(not_joined(org_id));
        }
        // The values go first: a step below that fails must not leave them served.
        self.write().remove(org_id);
        self.failed_at().remove(org_id);
        let (dir, id) = (self.inner.dir.clone(), org_id.to_string());
        blocking(move || delete_offline(&dir, &id)).await?;
        SourceCreds::forget(self.inner.secrets.as_ref(), org_id)?;
        if has_row {
            self.inner.store.remove_org(org_id).await?;
        }
        Ok(())
    }

    /// Fetches the org now (**Refresh**, the timer) and applies the outcome: `Connected` with
    /// the new values and copy; `Revoked` with the copy and values wiped; `Offline` or
    /// `PolicyInvalid` with the last good copy kept until it expires. A failed fetch is not an
    /// error: the summary's status says what happened.
    pub async fn refresh(&self, org_id: &str) -> Result<OrgSummary> {
        self.refresh_org(org_id, false).await?;
        self.summary_now(org_id).ok_or_else(|| not_joined(org_id))
    }

    /// Fetches every joined org at once, at start, after `load`. A failure is logged.
    pub async fn refresh_all(&self) {
        let ids: Vec<String> = self
            .read()
            .iter()
            .filter(|(_, slot)| matches!(slot, Slot::Joined(_)))
            .map(|(id, _)| id.clone())
            .collect();
        let tasks: Vec<_> = ids
            .into_iter()
            .map(|id| {
                let orgs = self.clone();
                tokio::spawn(async move {
                    if let Err(e) = orgs.refresh(&id).await {
                        tracing::warn!("org {id:?} was not refreshed: {e}");
                    }
                })
            })
            .collect();
        for task in tasks {
            if let Err(e) = task.await {
                tracing::warn!("an org refresh did not finish: {e}");
            }
        }
    }

    /// Fetches the org only if its last good fetch is older than its policy's
    /// `refresh_minutes`, or if it has no accepted policy: before a run starts (§6.4). It does
    /// not fetch within `RETRY_AFTER` of a failed fetch. Whether it fetched.
    pub async fn refresh_if_stale(&self, org_id: &str) -> Result<bool> {
        self.refresh_org(org_id, true).await
    }

    /// Every joined org, by name without regard to case. A SOPS org waiting for its key is not
    /// joined, so it is not listed.
    pub async fn summaries(&self) -> Vec<OrgSummary> {
        let ids: Vec<String> = self.read().keys().cloned().collect();
        for id in &ids {
            self.expire_if_due(id).await;
        }
        let mut out: Vec<OrgSummary> = self
            .read()
            .iter()
            .filter(|(_, slot)| !matches!(slot, Slot::Pending(_)))
            .map(|(id, slot)| summary_of(id, slot))
            .collect();
        out.sort_by(|a, b| {
            let (x, y) = (a.name.to_lowercase(), b.name.to_lowercase());
            x.cmp(&y).then_with(|| a.id.cmp(&b.id))
        });
        out
    }

    /// The org `org_id`, joined or waiting for its SOPS key.
    pub async fn summary(&self, org_id: &str) -> Option<OrgSummary> {
        self.expire_if_due(org_id).await;
        self.summary_now(org_id)
    }

    /// The org's entries for the Env tab: plain ones with their values, secrets by name only,
    /// each with the org's dots that grant it. Empty while the org gives no values.
    pub async fn entries(&self, org_id: &str) -> Result<Vec<OrgEntry>> {
        self.expire_if_due(org_id).await;
        let mut entries = match self.read().get(org_id) {
            Some(Slot::Joined(joined)) => joined.listing(),
            Some(Slot::Damaged { .. }) => Vec::new(),
            Some(Slot::Pending(_)) | None => return Err(not_joined(org_id)),
        };
        if !entries.is_empty() {
            let dots = self.inner.store.list_dots().await?;
            let scope = DotScope::Org(org_id.to_string());
            for entry in &mut entries {
                entry.used_by = dots_granting(&dots, &scope, &entry.name);
            }
        }
        Ok(entries)
    }

    /// Whether `org_id` is joined on this PC: its row was loaded, even a damaged one. A SOPS
    /// join that waits for the admin is not joined yet.
    pub fn is_joined(&self, org_id: &str) -> bool {
        matches!(
            self.read().get(org_id),
            Some(Slot::Joined(_) | Slot::Damaged { .. })
        )
    }

    /// The org's accepted policy, if one was.
    pub fn policy(&self, org_id: &str) -> Option<OrgPolicy> {
        match self.read().get(org_id) {
            Some(Slot::Joined(joined)) => joined.policy.clone(),
            _ => None,
        }
    }

    /// The org's value of `name`, compared without regard to case, or why there is none.
    pub async fn lookup(&self, org_id: &str, name: &str) -> OrgLookup {
        self.expire_if_due(org_id).await;
        match self.read().get(org_id) {
            Some(Slot::Joined(joined)) => joined.lookup(name),
            Some(Slot::Damaged { error, .. }) => OrgLookup::PolicyInvalid(error.clone()),
            Some(Slot::Pending(_)) | None => OrgLookup::Removed,
        }
    }

    /// The org's accepted policy and its answer for each of `names`, from one read of the org:
    /// for the resolver, which applies the policy to the values it was accepted with. The
    /// expiry is that of `lookup`.
    pub async fn view(&self, org_id: &str, names: &[String]) -> OrgView {
        self.expire_if_due(org_id).await;
        let orgs = self.read();
        let each = |answer: &dyn Fn(&str) -> OrgLookup| names.iter().map(|n| answer(n)).collect();
        match orgs.get(org_id) {
            Some(Slot::Joined(joined)) => OrgView {
                policy: joined.policy.clone(),
                lookups: each(&|name| joined.lookup(name)),
            },
            Some(Slot::Damaged { error, .. }) => OrgView {
                policy: None,
                lookups: each(&|_| OrgLookup::PolicyInvalid(error.clone())),
            },
            Some(Slot::Pending(_)) | None => OrgView {
                policy: None,
                lookups: each(&|_| OrgLookup::Removed),
            },
        }
    }

    /// Fetches the org and applies the outcome; with `only_if_stale`, only when it is stale.
    /// Whether it fetched.
    async fn refresh_org(&self, org_id: &str, only_if_stale: bool) -> Result<bool> {
        let _guard = self.lock(org_id).await;
        let (row, source, refresh_minutes) = match self.read().get(org_id) {
            Some(Slot::Joined(joined)) => (
                joined.row.clone(),
                joined.source.clone(),
                joined.policy.as_ref().map(|p| p.refresh_minutes),
            ),
            Some(Slot::Damaged { .. }) => return Ok(false),
            Some(Slot::Pending(_)) | None => return Err(not_joined(org_id)),
        };
        if only_if_stale {
            let now = self.inner.clock.now();
            let failed = self.failed_at().get(org_id).copied();
            if !is_stale(&row, refresh_minutes, now)
                || failed.is_some_and(|at| now - at < RETRY_AFTER)
            {
                return Ok(false);
            }
        }
        let outcome = self.fetch(&row, source.as_ref()).await;
        if !matches!(outcome, Outcome::Accepted(..)) {
            self.failed_at()
                .insert(org_id.to_string(), self.inner.clock.now());
        }
        match outcome {
            Outcome::Accepted(fetched, policy) => {
                let now = self.inner.clock.now();
                self.inner
                    .store
                    .record_org_fetch(org_id, &stamp(now), &policy.issued_at)
                    .await?;
                let row = OrgRow {
                    last_issued_at: Some(policy.issued_at.clone()),
                    last_fetch_at: Some(stamp(now)),
                    ..row
                };
                self.settle(row, source, fetched, *policy, now).await;
            }
            Outcome::Denied(reason) => {
                self.with_joined(org_id, |joined| {
                    joined.entries = None;
                    joined.copy_until = None;
                    joined.status = OrgStatus::Revoked;
                    joined.reason = Some(reason);
                });
                self.delete_copy(org_id).await;
            }
            Outcome::Unreachable(reason) => self.fall_back(org_id, None, Some(reason)).await,
            Outcome::Invalid(error) => self.fall_back(org_id, Some(error), None).await,
        }
        Ok(true)
    }

    /// One fetch with the org's stored sign-in details, and the policy checked against the
    /// row: its key, its org and the last `issued_at` accepted.
    async fn fetch(&self, row: &OrgRow, source: &dyn Source) -> Outcome {
        let creds = match SourceCreds::load(self.inner.secrets.as_ref(), &row.id) {
            Ok(Some(creds)) => creds,
            Ok(None) => {
                return Outcome::Invalid(
                    "its sign-in details are missing; give them again with Update".into(),
                )
            }
            Err(e) => return Outcome::Invalid(e.to_string()),
        };
        match source.fetch(&creds).await {
            Ok(fetched) => {
                let last = row.last_issued_at.as_deref();
                match verify(&row.policy_key, &row.id, last, &fetched) {
                    Ok(policy) => Outcome::Accepted(fetched, Box::new(policy)),
                    Err(e) => Outcome::Invalid(e.to_string()),
                }
            }
            Err(SourceError::Denied(why)) => Outcome::Denied(why),
            Err(SourceError::Unreachable(why)) => Outcome::Unreachable(why),
            Err(SourceError::Invalid(why)) => Outcome::Invalid(why),
        }
    }

    /// After a fetch that failed but did not revoke: the values stay while the copy holds.
    /// `error` makes the org `PolicyInvalid`; otherwise it is offline for `reason`.
    async fn fall_back(&self, org_id: &str, error: Option<String>, reason: Option<String>) {
        let now = self.inner.clock.now();
        let expired = self.with_joined(org_id, |joined| {
            let kept = joined.copy_until.filter(|until| now < *until);
            let expired = joined.copy_until.is_some() && kept.is_none();
            if kept.is_none() {
                joined.entries = None;
                joined.copy_until = None;
            }
            let was_expired = joined.status == OrgStatus::Expired;
            let was_revoked = joined.status == OrgStatus::Revoked;
            joined.status = match error {
                Some(error) => OrgStatus::PolicyInvalid { error },
                None if was_revoked => OrgStatus::Revoked,
                None if kept.is_some() => OrgStatus::Offline {
                    until: kept.map(stamp),
                },
                None if expired || was_expired => OrgStatus::Expired,
                None => OrgStatus::Offline { until: None },
            };
            // An unreachable source says nothing new about a revocation: keep its reason.
            if !was_revoked || joined.status != OrgStatus::Revoked {
                joined.reason = reason;
            }
            expired
        });
        if expired == Some(true) {
            self.delete_copy(org_id).await;
        }
    }

    /// Lets an org that is not `Connected` lose its copy once `offline_hours` have passed since
    /// its last good fetch. An offline org becomes `Expired`.
    async fn expire_if_due(&self, org_id: &str) {
        let now = self.inner.clock.now();
        let due = |joined: &Joined| {
            joined.status != OrgStatus::Connected
                && joined.copy_until.is_some_and(|until| now >= until)
        };
        if !matches!(self.read().get(org_id), Some(Slot::Joined(j)) if due(j)) {
            return;
        }
        // The file is only deleted by whoever has the org's lock, so a fresh copy is never
        // taken for an old one. Without the lock the values are dropped now but `copy_until`
        // stays: the holder's `fall_back` deletes the due copy, or the next call retries.
        let guard = self.try_lock(org_id);
        let locked = guard.is_some();
        let expired = self.with_joined(org_id, |joined| {
            if !due(joined) {
                return false;
            }
            joined.entries = None;
            if locked {
                joined.copy_until = None;
            }
            if matches!(joined.status, OrgStatus::Offline { .. }) {
                joined.status = OrgStatus::Expired;
            }
            true
        });
        if expired == Some(true) && locked {
            self.delete_copy(org_id).await;
        }
    }

    /// Keeps the values and policy of a good fetch, at `at`, as the org's `row`: writes the
    /// copy, adds the secrets to the redactor, and makes the org `Connected`. A failed fetch
    /// before it no longer holds `refresh_if_stale` back.
    async fn settle(
        &self,
        row: OrgRow,
        source: Arc<dyn Source>,
        fetched: Fetched,
        policy: OrgPolicy,
        at: DateTime<Utc>,
    ) {
        self.failed_at().remove(&row.id);
        let copy_until = self.save_copy(&row.id, &policy, &fetched, at).await;
        add_secrets(&policy, &fetched.entries);
        let id = row.id.clone();
        let joined = Joined {
            row,
            source,
            policy: Some(policy),
            entries: Some(fetched.entries),
            copy_until,
            status: OrgStatus::Connected,
            reason: None,
        };
        self.write().insert(id, Slot::Joined(Box::new(joined)));
    }

    /// Writes the copy of a good fetch, and says until when it may be used. Under
    /// `offline_hours: 0` nothing is written and an old copy is deleted. A copy that cannot be
    /// written is logged, and the old one deleted, so the copy on disk is always the newest.
    async fn save_copy(
        &self,
        org_id: &str,
        policy: &OrgPolicy,
        fetched: &Fetched,
        at: DateTime<Utc>,
    ) -> Option<DateTime<Utc>> {
        if policy.offline_hours == 0 {
            self.delete_copy(org_id).await;
            return None;
        }
        let copy = OfflineCopy {
            fetched_at: stamp(at),
            entries: fetched.entries.clone(),
            policy: fetched.policy.clone(),
        };
        let (dir, protector, id) = (
            self.inner.dir.clone(),
            self.inner.protector.clone(),
            org_id.to_string(),
        );
        match blocking(move || write_offline(&dir, protector.as_ref(), &id, &copy)).await {
            Ok(()) => Some(copy_end(at, policy.offline_hours)),
            Err(e) => {
                tracing::warn!("org {org_id:?}: its offline copy was not written: {e}");
                self.delete_copy(org_id).await;
                None
            }
        }
    }

    /// Deletes the org's copy; a failure is logged.
    async fn delete_copy(&self, org_id: &str) {
        let (dir, id) = (self.inner.dir.clone(), org_id.to_string());
        if let Err(e) = blocking(move || delete_offline(&dir, &id)).await {
            tracing::warn!("org {org_id:?}: its offline copy was not deleted: {e}");
        }
    }

    /// The org of a row at start, with its offline copy if that still holds.
    async fn load_one(&self, row: OrgRow) -> Joined {
        let mut joined = Joined {
            source: self.source(&row.source),
            row,
            policy: None,
            entries: None,
            copy_until: None,
            status: OrgStatus::Offline { until: None },
            reason: None,
        };
        let row = &joined.row;
        let (dir, protector, id) = (
            self.inner.dir.clone(),
            self.inner.protector.clone(),
            row.id.clone(),
        );
        let copy = match blocking(move || read_offline(&dir, protector.as_ref(), &id)).await {
            Ok(copy) => copy,
            Err(e) => {
                tracing::warn!("org {:?}: its offline copy is not used: {e}", row.id);
                let id = row.id.clone();
                self.delete_copy(&id).await;
                None
            }
        };
        let Some(copy) = copy else {
            return joined;
        };
        let last = row.last_issued_at.as_deref();
        let policy = match verify_policy(&copy.policy, &row.policy_key, &row.id)
            .and_then(|policy| policy.check_not_rolled_back(last).map(|()| policy))
        {
            Ok(policy) => policy,
            Err(e) => {
                let id = row.id.clone();
                self.delete_copy(&id).await;
                joined.status = OrgStatus::PolicyInvalid {
                    error: format!("its offline copy was refused: {e}"),
                };
                return joined;
            }
        };
        let fetched_at = parse_time(&copy.fetched_at).unwrap_or(DateTime::<Utc>::MIN_UTC);
        let until = copy_end(fetched_at, policy.offline_hours);
        let now = self.inner.clock.now();
        if policy.offline_hours == 0 || now >= until || fetched_at > now + COPY_CLOCK_SKEW {
            let id = row.id.clone();
            self.delete_copy(&id).await;
            joined.policy = Some(policy);
            joined.status = OrgStatus::Expired;
            return joined;
        }
        add_secrets(&policy, &copy.entries);
        joined.entries = Some(copy.entries);
        joined.policy = Some(policy);
        joined.copy_until = Some(until);
        joined.status = OrgStatus::Offline {
            until: Some(stamp(until)),
        };
        joined
    }

    /// Writes an Update's settings, then its fetch. If the fetch can't be recorded, the old
    /// settings are put back.
    async fn store_settings(&self, row: &OrgRow, old: &OrgRow) -> Result<()> {
        let store = &self.inner.store;
        store
            .update_org_settings(&row.id, &row.name, &row.source, &row.policy_key)
            .await?;
        let last_fetch_at = row.last_fetch_at.as_deref().unwrap_or_default();
        let last_issued_at = row.last_issued_at.as_deref().unwrap_or_default();
        if let Err(e) = store
            .record_org_fetch(&row.id, last_fetch_at, last_issued_at)
            .await
        {
            let restored = store
                .update_org_settings(&old.id, &old.name, &old.source, &old.policy_key)
                .await;
            if let Err(again) = restored {
                tracing::warn!(
                    "org {:?}: its old settings were not put back: {again}",
                    old.id
                );
            }
            return Err(e);
        }
        Ok(())
    }

    /// The error of a join whose fetch or policy failed, after taking out what it stored.
    fn join_failed(&self, code: &JoinCode, why: String) -> Error {
        self.forget_creds(code);
        Error::Other(format!("could not join {}: {why}", code.name))
    }

    /// Takes out the sign-in details a failed join stored. A SOPS org keeps its age key: the
    /// person has sent its public half to the admin.
    fn forget_creds(&self, code: &JoinCode) {
        if matches!(code.source, SourceSettings::Sops(_)) {
            return;
        }
        if let Err(e) = SourceCreds::forget(self.inner.secrets.as_ref(), &code.org_id) {
            tracing::warn!(
                "org {:?}: its sign-in details were not taken out: {e}",
                code.org_id
            );
        }
    }

    fn source(&self, settings: &SourceSettings) -> Arc<dyn Source> {
        let make = self
            .inner
            .sources
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        Arc::from(make(settings))
    }

    fn summary_now(&self, org_id: &str) -> Option<OrgSummary> {
        self.read().get(org_id).map(|slot| summary_of(org_id, slot))
    }

    fn with_joined<T>(&self, org_id: &str, change: impl FnOnce(&mut Joined) -> T) -> Option<T> {
        match self.write().get_mut(org_id) {
            Some(Slot::Joined(joined)) => Some(change(joined)),
            _ => None,
        }
    }

    fn failed_at(&self) -> std::sync::MutexGuard<'_, HashMap<String, DateTime<Utc>>> {
        self.inner
            .failed_at
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// The lock for changes to `org_id`.
    async fn lock(&self, org_id: &str) -> tokio::sync::OwnedMutexGuard<()> {
        self.org_lock(org_id).lock_owned().await
    }

    /// The lock for changes to `org_id`, if nothing holds it now.
    fn try_lock(&self, org_id: &str) -> Option<tokio::sync::OwnedMutexGuard<()>> {
        self.org_lock(org_id).try_lock_owned().ok()
    }

    fn org_lock(&self, org_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.inner
            .locks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(org_id.to_string())
            .or_default()
            .clone()
    }

    /// The orgs, whether or not a panic poisoned the lock: each change is made whole.
    fn read(&self) -> RwLockReadGuard<'_, BTreeMap<String, Slot>> {
        self.inner
            .orgs
            .read()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, BTreeMap<String, Slot>> {
        self.inner
            .orgs
            .write()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

impl Joined {
    fn lookup(&self, name: &str) -> OrgLookup {
        if let Some(entries) = &self.entries {
            return match entries.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)) {
                Some((key, value)) => OrgLookup::Value {
                    value: value.clone(),
                    is_plain: self.policy.as_ref().is_some_and(|p| p.is_plain(key)),
                },
                None => OrgLookup::NotDefined,
            };
        }
        match &self.status {
            OrgStatus::Revoked => OrgLookup::Revoked,
            OrgStatus::Expired => OrgLookup::Expired,
            OrgStatus::PolicyInvalid { error } => OrgLookup::PolicyInvalid(error.clone()),
            OrgStatus::Connected | OrgStatus::Offline { .. } | OrgStatus::Pending => {
                OrgLookup::Offline
            }
        }
    }

    fn listing(&self) -> Vec<OrgEntry> {
        let (Some(entries), Some(policy)) = (&self.entries, &self.policy) else {
            return Vec::new();
        };
        let locked = policy.personal != PersonalMode::Override;
        let mut out: Vec<OrgEntry> = entries
            .iter()
            .map(|(name, value)| {
                let plain = policy.is_plain(name);
                OrgEntry {
                    name: name.clone(),
                    kind: if plain {
                        EntryKind::Plain
                    } else {
                        EntryKind::Secret
                    },
                    value: plain.then(|| value.to_string()),
                    locked,
                    used_by: Vec::new(),
                }
            })
            .collect();
        out.sort_by_key(|entry| entry.name.to_lowercase());
        out
    }

    fn status_text(&self) -> String {
        let with_reason = |text: &str| match &self.reason {
            Some(reason) => format!("{text} ({reason})"),
            None => text.to_string(),
        };
        let copy = self
            .copy_until
            .filter(|_| self.entries.is_some())
            .map(stamp);
        match (&self.status, copy) {
            (OrgStatus::Connected, _) => "connected".into(),
            (OrgStatus::Offline { until: Some(until) }, _) => {
                format!(
                    "{}; using the saved copy until {until}",
                    with_reason("offline")
                )
            }
            (OrgStatus::Offline { until: None }, _) => {
                format!("{}, with no saved copy", with_reason("offline"))
            }
            (OrgStatus::Revoked, _) => with_reason("access revoked"),
            (OrgStatus::PolicyInvalid { error }, Some(until)) => {
                format!("policy invalid: {error}; using the saved copy until {until}")
            }
            (OrgStatus::PolicyInvalid { error }, None) => format!("policy invalid: {error}"),
            (OrgStatus::Expired, _) => "secrets expired offline; reconnect to refresh".into(),
            (OrgStatus::Pending, _) => PENDING_TEXT.into(),
        }
    }
}

fn summary_of(id: &str, slot: &Slot) -> OrgSummary {
    match slot {
        Slot::Joined(joined) => OrgSummary {
            id: id.to_string(),
            name: joined.row.name.clone(),
            source_kind: joined.row.source.kind().to_string(),
            address: joined.row.source.address().to_string(),
            key_fingerprint: key_fingerprint(&joined.row.policy_key).unwrap_or_default(),
            status_text: joined.status_text(),
            status: joined.status.clone(),
            last_fetch_at: joined.row.last_fetch_at.clone(),
            copy_until: joined
                .copy_until
                .filter(|_| joined.entries.is_some())
                .map(stamp),
            policy_summary: joined
                .policy
                .as_ref()
                .map(OrgPolicy::summary)
                .unwrap_or_default(),
            personal_mode: joined.policy.as_ref().map(|p| p.personal),
            shell_secrets: joined.policy.as_ref().is_some_and(|p| p.shell_secrets),
            use_rules: joined.policy.as_ref().map(|p| p.use_rules.clone()),
        },
        Slot::Damaged { name, error } => OrgSummary {
            id: id.to_string(),
            name: name.clone(),
            source_kind: String::new(),
            address: String::new(),
            key_fingerprint: String::new(),
            status: OrgStatus::PolicyInvalid {
                error: error.clone(),
            },
            status_text: format!("policy invalid: {error}"),
            last_fetch_at: None,
            copy_until: None,
            policy_summary: Vec::new(),
            personal_mode: None,
            shell_secrets: false,
            use_rules: None,
        },
        Slot::Pending(code) => OrgSummary {
            id: id.to_string(),
            name: code.name.clone(),
            source_kind: code.source.kind().to_string(),
            address: code.source.address().to_string(),
            key_fingerprint: key_fingerprint(&code.policy_key).unwrap_or_default(),
            status: OrgStatus::Pending,
            status_text: PENDING_TEXT.into(),
            last_fetch_at: None,
            copy_until: None,
            policy_summary: Vec::new(),
            personal_mode: None,
            shell_secrets: false,
            use_rules: None,
        },
    }
}

/// The policy of a fetch, verified against the org's key and id, and not older than the last
/// one accepted.
fn verify(
    policy_key: &str,
    org_id: &str,
    last_issued_at: Option<&str>,
    fetched: &Fetched,
) -> std::result::Result<OrgPolicy, PolicyError> {
    let policy = verify_policy(&fetched.policy, policy_key, org_id)?;
    policy.check_not_rolled_back(last_issued_at)?;
    Ok(policy)
}

/// Scrubs every secret value of a fetch from what Dots writes from now on. Plain values are
/// shown anyway, so they are left alone.
fn add_secrets(policy: &OrgPolicy, entries: &BTreeMap<String, Zeroizing<String>>) {
    for (name, value) in entries {
        if !policy.is_plain(name) {
            redact::global().add(name, value);
        }
    }
}

/// Whether the org should be fetched before a run: no policy accepted, no good fetch, or the
/// last one at least `refresh_minutes` ago.
fn is_stale(row: &OrgRow, refresh_minutes: Option<u32>, now: DateTime<Utc>) -> bool {
    let last = row.last_fetch_at.as_deref().and_then(parse_time);
    match (refresh_minutes, last) {
        (Some(minutes), Some(last)) => now - last >= Duration::minutes(minutes.into()),
        _ => true,
    }
}

/// Checks that `creds` are the kind of sign-in details the source takes, before they are
/// stored.
fn check_creds_fit(source: &SourceSettings, creds: &SourceCreds) -> Result<()> {
    let (fits, needs) = match source {
        SourceSettings::OpenBao(s) if s.auth == BaoAuth::Token => (
            matches!(creds, SourceCreds::Token { .. }),
            "this org's OpenBao signs in with a token",
        ),
        SourceSettings::OpenBao(_) => (
            matches!(creds, SourceCreds::Login { .. }),
            "this org's OpenBao signs in with a username and password",
        ),
        SourceSettings::Infisical(_) => (
            matches!(creds, SourceCreds::Machine { .. }),
            "this org's Infisical signs in with a machine identity's client id and secret",
        ),
        SourceSettings::Sops(_) => (
            matches!(creds, SourceCreds::Age { .. }),
            "this org's SOPS file is opened with this PC's age key",
        ),
    };
    if !fits {
        return Err(Error::Invalid(needs.to_string()));
    }
    Ok(())
}

/// Whether an Update from `old` to `new` needs the sign-in details given again: any change to an
/// OpenBao or Infisical source's settings (its address or site, auth method, mounts, namespace,
/// path, project, environment or folder), or a change of kind. A SOPS file's new location
/// keeps this PC's age key, which is never sent anywhere.
fn source_changed(old: &SourceSettings, new: &SourceSettings) -> bool {
    match (old, new) {
        (SourceSettings::Sops(_), SourceSettings::Sops(_)) => false,
        _ => old != new,
    }
}

/// Puts the sign-in details an Update replaced back; a failure is logged.
fn restore_creds(secrets: &dyn SecretStore, org_id: &str, old: Option<&SourceCreds>) {
    let restored = match old {
        Some(creds) => creds.save(secrets, org_id),
        None => SourceCreds::forget(secrets, org_id),
    };
    if let Err(e) = restored {
        tracing::warn!("org {org_id:?}: its old sign-in details were not put back: {e}");
    }
}

/// What a new join code changes for a joined org, in plain words.
fn changes(old: &OrgRow, code: &JoinCode) -> Vec<String> {
    let mut out = Vec::new();
    if old.name != code.name {
        out.push(format!("name changes from {} to {}", old.name, code.name));
    }
    if old.source.kind() != code.source.kind() {
        out.push(format!(
            "source changes from {} at {} to {} at {}",
            kind_name(&old.source),
            old.source.address(),
            kind_name(&code.source),
            code.source.address()
        ));
    } else {
        let (before, after) = (fields(&old.source), fields(&code.source));
        let keys: BTreeSet<&String> = before.keys().chain(after.keys()).collect();
        for key in keys.into_iter().filter(|key| *key != "kind") {
            let (was, will) = (before.get(key), after.get(key));
            if was != will {
                out.push(format!(
                    "{key} changes from {} to {}",
                    shown(was),
                    shown(will)
                ));
            }
        }
    }
    if old.policy_key != code.policy_key {
        let fingerprint = |key: &str| key_fingerprint(key).unwrap_or_default();
        out.push(format!(
            "{KEY_CHANGE} from {} to {}: the key that signs the org's policy is new, so accept \
             this code only if your admin told you the key changed",
            fingerprint(&old.policy_key),
            fingerprint(&code.policy_key)
        ));
    }
    out
}

/// The settings' fields by name, as the join code spells them.
fn fields(source: &SourceSettings) -> serde_json::Map<String, Value> {
    match serde_json::to_value(source) {
        Ok(Value::Object(fields)) => fields,
        _ => serde_json::Map::new(),
    }
}

fn shown(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => "(none)".into(),
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
    }
}

/// The kind of source as the person reads it.
fn kind_name(source: &SourceSettings) -> &'static str {
    match source {
        SourceSettings::OpenBao(_) => "OpenBao",
        SourceSettings::Infisical(_) => "Infisical",
        SourceSettings::Sops(_) => "SOPS file",
    }
}

fn damaged_text(org_id: &str) -> String {
    format!("the settings of org '{org_id}' are damaged; remove it and join it again")
}

fn not_joined(org_id: &str) -> Error {
    Error::NotFound(format!("no org '{org_id}' is joined on this PC"))
}

/// When a copy fetched `at` runs out, `offline_hours` later.
fn copy_end(at: DateTime<Utc>, offline_hours: u32) -> DateTime<Utc> {
    at.checked_add_signed(Duration::hours(offline_hours.into()))
        .unwrap_or(DateTime::<Utc>::MAX_UTC)
}

/// A time as the orgs table keeps it: RFC 3339, UTC, to the millisecond.
fn stamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn parse_time(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|at| at.with_timezone(&Utc))
}

/// Runs file work on the blocking pool: the offline copy's reads, writes and renames block.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| Error::Other(format!("an offline copy task failed: {e}")))?
}

/// Deletes the temp files a crash left beside the offline copies. They hold only protected
/// bytes, but no stray copy of an org's secrets should stay.
fn sweep_temp(dir: &Path) {
    let Ok(listing) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in listing.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "tmp") && path.is_file() {
            if let Err(e) = std::fs::remove_file(&path) {
                tracing::warn!("{} was not deleted: {e}", path.display());
            }
        }
    }
}
