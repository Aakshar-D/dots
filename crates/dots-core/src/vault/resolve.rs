//! Resolving a dot's variables (env vault spec §8): a name resolves only when it is granted to
//! the consumer, and then from the dot's scope. A Personal dot reads Personal. A dot in an org
//! reads the org, as its state, its `use` rules and its `personal` mode allow, and its shell
//! gets no secret unless the org's `shell_secrets` says so. What does not resolve says which
//! variable and why.

use std::collections::{BTreeMap, HashSet};

use zeroize::Zeroizing;

use super::grants::{Consumer, Grants};
use super::org::{OrgLookup, OrgPolicy, OrgView, PersonalMode};
use super::service::Vault;
use crate::accounts::NotMember;
use crate::library::PERSONAL_SCOPE;
use crate::model::Dot;
use crate::{Error, Result};

/// Where a dot's variables resolve: Personal, or the org the dot was made in (`Dot.org_id`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DotScope {
    Personal,
    /// The org's id.
    Org(String),
}

impl DotScope {
    /// The scope of `dot`.
    pub fn of(dot: &Dot) -> DotScope {
        match &dot.org_id {
            Some(org_id) => DotScope::Org(org_id.clone()),
            None => DotScope::Personal,
        }
    }

    /// The scope's key in the store, for its MCP catalogs: `personal`, or the org's id.
    pub fn key(&self) -> &str {
        match self {
            DotScope::Personal => PERSONAL_SCOPE,
            DotScope::Org(org_id) => org_id,
        }
    }
}

/// Why a variable did not resolve (the spec's `MissingWhy`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unresolved {
    /// The dot does not grant the name to the consumer.
    NotGranted,
    /// The scope has no entry of that name, nor Personal where the org's `personal` reaches it.
    NotDefined,
    /// The org's `use` rules do not allow the name.
    DeniedByPolicy,
    /// The value is a secret, the consumer is the shell, and the org's `shell_secrets` is false.
    ShellSecretsForbidden,
    /// The org's source can't be reached, and there is no saved copy that still holds.
    OrgOffline,
    /// The org's offline copy ran out.
    OrgExpired,
    /// The org's source refused the person's sign-in details.
    OrgRevoked,
    /// The org has no valid policy and no good copy: the error.
    PolicyInvalid(String),
    /// The dot's org is not joined on this PC.
    OrgRemoved,
    /// This PC is not on the account's member list, and why (members spec §6.4).
    NotMember(NotMember),
}

/// A variable that did not resolve for a consumer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Missing {
    pub name: String,
    pub consumer: Consumer,
    pub why: Unresolved,
    /// The org's name, for the text; its id when it is not joined (`OrgRemoved`). `None` for
    /// a Personal dot.
    pub org: Option<String>,
}

/// What a dot whose org is not joined on this PC says: at its run's start, when it is saved,
/// and for each of its variables.
pub fn org_removed_text(dot: &str, org_id: &str) -> String {
    format!("dot '{dot}': its org '{org_id}' was removed")
}

impl Missing {
    /// The words of a run event, for the dot `dot`.
    pub fn text(&self, dot: &str) -> String {
        let consumer = self.consumer.key();
        let name = &self.name;
        let org = self.org.as_deref().unwrap_or("Personal");
        match &self.why {
            Unresolved::NotGranted => format!("{name}: not granted to {consumer} in dot '{dot}'"),
            Unresolved::NotDefined => format!("{name}: not in {org} (dot '{dot}', {consumer})"),
            Unresolved::DeniedByPolicy => {
                format!("{name}: {org}'s policy does not allow it (dot '{dot}')")
            }
            Unresolved::ShellSecretsForbidden => {
                format!("{name}: {org} does not allow secrets in the shell (dot '{dot}')")
            }
            Unresolved::OrgExpired => {
                format!("{org}: secrets expired offline; reconnect to refresh")
            }
            Unresolved::OrgRevoked => format!("{org}: access revoked"),
            Unresolved::PolicyInvalid(error) => format!("{org}: policy invalid: {error}"),
            Unresolved::OrgOffline => format!("{org}: offline, with no saved copy"),
            Unresolved::OrgRemoved => org_removed_text(dot, org),
            Unresolved::NotMember(why) => why.text(org),
        }
    }

    /// The words stored on a server's catalog when its Check in an org's scope fails
    /// (`Vault::check_values`): the variable, the scope and why, and never a dot.
    pub fn check_text(&self) -> String {
        let name = &self.name;
        let org = self.org.as_deref().unwrap_or("Personal");
        match &self.why {
            Unresolved::NotGranted => format!("{name}: not granted"),
            Unresolved::NotDefined => format!("{name}: not in {org}"),
            Unresolved::DeniedByPolicy => format!("{name}: {org}'s policy does not allow it"),
            Unresolved::ShellSecretsForbidden => {
                format!("{name}: {org} does not allow secrets in the shell")
            }
            Unresolved::OrgExpired => {
                format!("{name}: {org}: secrets expired offline; reconnect to refresh")
            }
            Unresolved::OrgRevoked => format!("{name}: {org}: access revoked"),
            Unresolved::PolicyInvalid(error) => format!("{name}: {org}: policy invalid: {error}"),
            Unresolved::OrgOffline => format!("{name}: {org}: offline, with no saved copy"),
            Unresolved::OrgRemoved => format!("{name}: no org '{org}' is joined on this PC"),
            Unresolved::NotMember(why) => format!("{name}: {}", why.text(org)),
        }
    }
}

str_enum!(ValueSource {
    Org => "org",
    Personal => "personal",
});

/// What resolving one name found: its value, where it comes from, and whether it is a secret.
/// Its `Debug` leaves the value out.
pub(super) struct Found {
    /// `None` when only the entries that are there were read (`Reading::Presence`).
    pub(super) value: Option<Zeroizing<String>>,
    pub(super) source: ValueSource,
    pub(super) secret: bool,
}

impl std::fmt::Debug for Found {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Found")
            .field("source", &self.source)
            .field("secret", &self.secret)
            .finish_non_exhaustive()
    }
}

/// One name's answer: what it resolves to, or why it does not, with what was found for it.
pub(super) type Answer = std::result::Result<Found, (Missing, Option<Found>)>;

/// How resolving reads Personal: its values, for a run or a Check; or only which entries are
/// there (the status of a dot's grants), so no secret's value is read. `Presence` holds the
/// secret store's Personal keys, lower-cased; `None` when they could not be listed, and then
/// every Personal secret counts as there.
pub(super) enum Reading {
    Values,
    Presence(Option<HashSet<String>>),
}

/// What resolving in a scope reads once for all its names (`Vault::in_scope`).
pub(super) struct InScope {
    /// The org's id and its name for the texts (its id when it is not joined); `None` for
    /// Personal.
    org: Option<(String, String)>,
    /// The org's policy and answers for the names, from one read; `None` for Personal, or
    /// for a vault without orgs.
    view: Option<OrgView>,
}

/// Resolved values by the name they were asked for. Its `Debug` shows the names only.
#[derive(Clone, Default)]
pub struct Resolved(BTreeMap<String, Zeroizing<String>>);

impl std::fmt::Debug for Resolved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Resolved")
            .field("names", &self.0.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl Resolved {
    /// The value of `name`, for `McpServerConfig::expanded`.
    pub fn get(&self, name: &str) -> Option<String> {
        self.0.get(name).map(|v| v.to_string())
    }

    pub fn contains(&self, name: &str) -> bool {
        self.0.contains_key(name)
    }

    /// Each name with its value.
    pub fn pairs(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
}

impl Vault {
    /// Resolves `names` for `consumer` under `grants`, in the dot's `scope`: all of them, or the
    /// first that does not resolve. For each name, in the spec's order (§8): the grant; for an
    /// org, its state, then its `use` rules, then its `personal` mode, then the shell's secrets
    /// rule. A Personal scope reads Personal only.
    pub async fn resolve(
        &self,
        scope: &DotScope,
        grants: &Grants,
        consumer: &Consumer,
        names: &[String],
    ) -> Result<std::result::Result<Resolved, Missing>> {
        let granted = |name: &str| grants.allows(consumer, name);
        self.resolve_where(scope, &granted, consumer, names).await
    }

    /// The values of `names` for the Library's Check of an MCP server in `scope` (§8), as if
    /// all were granted to `consumer`. Personal gives what Personal has, as `personal_values`
    /// does: a name it lacks is left out, and the server says so when it starts. An org
    /// resolves each name as a run in it would: its state, its `use` rules, then its
    /// `personal` mode; the first that does not resolve is the answer. An org not joined on
    /// this PC is `NotFound`.
    pub async fn check_values(
        &self,
        scope: &DotScope,
        consumer: &Consumer,
        names: &[String],
    ) -> Result<std::result::Result<Resolved, Missing>> {
        let DotScope::Org(org_id) = scope else {
            return Ok(Ok(self.personal_values(names).await?));
        };
        if !self.orgs().is_some_and(|orgs| orgs.is_joined(org_id)) {
            return Err(Error::NotFound(format!(
                "no org '{org_id}' is joined on this PC"
            )));
        }
        self.resolve_where(scope, &|_| true, consumer, names).await
    }

    /// `resolve`, with `granted` saying which names are granted to `consumer`.
    async fn resolve_where(
        &self,
        scope: &DotScope,
        granted: &(dyn Fn(&str) -> bool + Sync),
        consumer: &Consumer,
        names: &[String],
    ) -> Result<std::result::Result<Resolved, Missing>> {
        let read = self.in_scope(scope, names).await;
        let mut out = BTreeMap::new();
        for (i, name) in names.iter().enumerate() {
            let answer = self
                .resolve_one(&read, i, name, granted(name), consumer, &Reading::Values)
                .await?;
            match answer {
                Ok(Found {
                    value: Some(value), ..
                }) => {
                    out.insert(name.clone(), value);
                }
                // `Reading::Values` reads every value it finds.
                Ok(Found { value: None, .. }) => {}
                Err((missing, _)) => return Ok(Err(missing)),
            }
        }
        Ok(Ok(Resolved(out)))
    }

    /// What resolving in `scope` reads once for `names`: the org's name, and its policy and
    /// answers for those names from one read of the org.
    pub(super) async fn in_scope(&self, scope: &DotScope, names: &[String]) -> InScope {
        let DotScope::Org(org_id) = scope else {
            return InScope {
                org: None,
                view: None,
            };
        };
        let name = self.org_name(org_id).await;
        // The org's policy and values, from one read of the org.
        let view = match self.orgs() {
            Some(orgs) => Some(orgs.view(org_id, names).await),
            None => None,
        };
        InScope {
            org: Some((org_id.clone(), name)),
            view,
        }
    }

    /// The `i`th of the names `read` was made for, for `consumer`, in the spec's order (§8):
    /// the grant (`granted`); for an org, its state, then its `use` rules, then its `personal`
    /// mode, then the shell's secrets rule. A Personal scope reads Personal only. What does
    /// not resolve comes with what was found for it, if anything: a secret the shell may not
    /// have. `reading` says whether values are read, or only which entries are there.
    pub(super) async fn resolve_one(
        &self,
        read: &InScope,
        i: usize,
        name: &str,
        granted: bool,
        consumer: &Consumer,
        reading: &Reading,
    ) -> Result<Answer> {
        let missing = |why: Unresolved| Missing {
            name: name.to_string(),
            consumer: consumer.clone(),
            org: read.org.as_ref().map(|(id, org_name)| match why {
                Unresolved::OrgRemoved => id.clone(),
                _ => org_name.clone(),
            }),
            why,
        };
        if !granted {
            return Ok(Err((missing(Unresolved::NotGranted), None)));
        }
        let found = match (&read.org, &read.view) {
            (None, _) => self
                .personal_found(reading, name)
                .await?
                .ok_or((Unresolved::NotDefined, None)),
            (Some(_), Some(view)) => {
                let (lookup, policy) = (&view.lookups[i], view.policy.as_ref());
                self.org_value(lookup, policy, consumer, name, reading)
                    .await?
            }
            (Some(_), None) => Err((Unresolved::OrgRemoved, None)),
        };
        Ok(found.map_err(|(why, found)| (missing(why), found)))
    }

    /// The personal values of those of `names` that Personal has, as if all were granted: for
    /// what the Library does outside a run (a probe, a check, a confirmation's origin).
    pub async fn personal_values(&self, names: &[String]) -> Result<Resolved> {
        let mut out = BTreeMap::new();
        for name in names {
            if let Some(value) = self.personal_value(name).await? {
                out.insert(name.clone(), value);
            }
        }
        Ok(Resolved(out))
    }

    /// What a run of `dot`, or a save of it, is refused with when its org is not joined on
    /// this PC; `None` for a Personal dot or a joined org.
    pub fn org_removed(&self, dot: &Dot) -> Option<String> {
        let DotScope::Org(org_id) = DotScope::of(dot) else {
            return None;
        };
        let joined = self.orgs().is_some_and(|orgs| orgs.is_joined(&org_id));
        (!joined).then(|| org_removed_text(&dot.spec.name, &org_id))
    }

    /// Before a run of `dot` starts (§6.4): a dot whose org is not joined does not start, and
    /// an org whose last good fetch is older than its `refresh_minutes` is fetched first. A
    /// fetch that fails leaves the org as the fetch left it: its saved copy, while that holds,
    /// or the reason it gives nothing, which resolving then names. An org removed while the
    /// refresh waited is checked for again, so its dot does not start either.
    pub async fn before_run(&self, dot: &Dot) -> Result<()> {
        if let Some(text) = self.org_removed(dot) {
            return Err(Error::Other(text));
        }
        if let (DotScope::Org(org_id), Some(orgs)) = (DotScope::of(dot), self.orgs()) {
            if let Err(e) = orgs.refresh_if_stale(&org_id).await {
                tracing::warn!("org {org_id:?} was not refreshed before a run: {e}");
            }
            // A Remove may have gone through while the refresh waited for the org's lock.
            if let Some(text) = self.org_removed(dot) {
                return Err(Error::Other(text));
            }
            // A PC that is not on the account's member list runs none of its dots (§6.4).
            if let Some(why) = orgs.not_member(&org_id) {
                let account = self.org_name(&org_id).await;
                return Err(Error::Other(why.text(&account)));
            }
        }
        Ok(())
    }

    /// The org's name for the texts, or its id when it is not joined.
    async fn org_name(&self, org_id: &str) -> String {
        match self.orgs() {
            Some(orgs) => orgs.summary(org_id).await.map(|s| s.name),
            None => None,
        }
        .unwrap_or_else(|| org_id.to_string())
    }

    /// The value of `name` for `consumer`, from the org's `lookup` of it and the `policy`
    /// accepted with it (one `Orgs::view`), or why there is none. The org's state comes first:
    /// an org that gives no values blocks every name, even one Personal has.
    async fn org_value(
        &self,
        lookup: &OrgLookup,
        policy: Option<&OrgPolicy>,
        consumer: &Consumer,
        name: &str,
        reading: &Reading,
    ) -> Result<std::result::Result<Found, (Unresolved, Option<Found>)>> {
        let refused = |why: Unresolved| Ok(Err((why, None)));
        // The org's value and whether it is a secret, if it has one.
        let in_org = match lookup {
            OrgLookup::Value { value, is_plain } => Some(Found {
                value: match reading {
                    Reading::Values => Some(value.clone()),
                    Reading::Presence(_) => None,
                },
                source: ValueSource::Org,
                secret: !is_plain,
            }),
            OrgLookup::NotDefined => None,
            OrgLookup::Expired => return refused(Unresolved::OrgExpired),
            OrgLookup::Revoked => return refused(Unresolved::OrgRevoked),
            OrgLookup::PolicyInvalid(error) => {
                return refused(Unresolved::PolicyInvalid(error.clone()))
            }
            OrgLookup::Offline => return refused(Unresolved::OrgOffline),
            OrgLookup::Removed => return refused(Unresolved::OrgRemoved),
            OrgLookup::NotMember(why) => return refused(Unresolved::NotMember(*why)),
        };
        // An org that gives values has an accepted policy. Without one, nothing is guessed.
        let Some(policy) = policy else {
            let error = "no policy has been accepted".to_string();
            return refused(Unresolved::PolicyInvalid(error));
        };
        if !policy.may_use(name) {
            return refused(Unresolved::DeniedByPolicy);
        }
        let chosen = match policy.personal {
            PersonalMode::Deny => in_org,
            PersonalMode::Fallback => match in_org {
                Some(found) => Some(found),
                None => self.personal_found(reading, name).await?,
            },
            PersonalMode::Override => match self.personal_found(reading, name).await? {
                Some(found) => Some(found),
                None => in_org,
            },
        };
        let Some(found) = chosen else {
            return refused(Unresolved::NotDefined);
        };
        if *consumer == Consumer::Shell && found.secret && !policy.shell_secrets {
            return Ok(Err((Unresolved::ShellSecretsForbidden, Some(found))));
        }
        Ok(Ok(found))
    }

    /// Personal's entry `name`, as `reading` reads it.
    async fn personal_found(&self, reading: &Reading, name: &str) -> Result<Option<Found>> {
        let (value, secret) = match reading {
            Reading::Values => match self.personal_entry(name).await? {
                Some((value, secret)) => (Some(value), secret),
                None => return Ok(None),
            },
            Reading::Presence(stored) => match self.personal_kind(name, stored.as_ref()).await? {
                Some(secret) => (None, secret),
                None => return Ok(None),
            },
        };
        Ok(Some(Found {
            value,
            source: ValueSource::Personal,
            secret,
        }))
    }
}
