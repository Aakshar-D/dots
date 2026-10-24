//! Resolving a dot's variables (env vault spec §8): a name resolves only when it is granted to
//! the consumer, and then from the dot's scope. A Personal dot reads Personal. A dot in an org
//! reads the org, as its state, its `use` rules and its `personal` mode allow, and its shell
//! gets no secret unless the org's `shell_secrets` says so. What does not resolve says which
//! variable and why.

use std::collections::BTreeMap;

use zeroize::Zeroizing;

use super::grants::{Consumer, Grants};
use super::org::{OrgLookup, OrgPolicy, PersonalMode};
use super::service::Vault;
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
        }
    }
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
        let org = match scope {
            DotScope::Personal => None,
            DotScope::Org(org_id) => Some((org_id.as_str(), self.org_name(org_id).await)),
        };
        // The org's policy and values, from one read of the org.
        let view = match (&org, self.orgs()) {
            (Some((org_id, _)), Some(orgs)) => Some(orgs.view(org_id, names).await),
            _ => None,
        };
        let mut out = BTreeMap::new();
        for (i, name) in names.iter().enumerate() {
            let missing = |why: Unresolved| Missing {
                name: name.clone(),
                consumer: consumer.clone(),
                org: org.as_ref().map(|(id, org_name)| match why {
                    Unresolved::OrgRemoved => id.to_string(),
                    _ => org_name.clone(),
                }),
                why,
            };
            if !grants.allows(consumer, name) {
                return Ok(Err(missing(Unresolved::NotGranted)));
            }
            let value = match &org {
                None => self
                    .personal_value(name)
                    .await?
                    .ok_or(Unresolved::NotDefined),
                Some(_) => match &view {
                    Some(view) => {
                        let (lookup, policy) = (&view.lookups[i], view.policy.as_ref());
                        self.org_value(lookup, policy, consumer, name).await?
                    }
                    None => Err(Unresolved::OrgRemoved),
                },
            };
            match value {
                Ok(value) => {
                    out.insert(name.clone(), value);
                }
                Err(why) => return Ok(Err(missing(why))),
            }
        }
        Ok(Ok(Resolved(out)))
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
    /// or the reason it gives nothing, which resolving then names.
    pub async fn before_run(&self, dot: &Dot) -> Result<()> {
        if let Some(text) = self.org_removed(dot) {
            return Err(Error::Other(text));
        }
        if let (DotScope::Org(org_id), Some(orgs)) = (DotScope::of(dot), self.orgs()) {
            if let Err(e) = orgs.refresh_if_stale(&org_id).await {
                tracing::warn!("org {org_id:?} was not refreshed before a run: {e}");
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
    ) -> Result<std::result::Result<Zeroizing<String>, Unresolved>> {
        // The org's value and whether it is a secret, if it has one.
        let in_org = match lookup {
            OrgLookup::Value { value, is_plain } => Some((value.clone(), !is_plain)),
            OrgLookup::NotDefined => None,
            OrgLookup::Expired => return Ok(Err(Unresolved::OrgExpired)),
            OrgLookup::Revoked => return Ok(Err(Unresolved::OrgRevoked)),
            OrgLookup::PolicyInvalid(error) => {
                return Ok(Err(Unresolved::PolicyInvalid(error.clone())))
            }
            OrgLookup::Offline => return Ok(Err(Unresolved::OrgOffline)),
            OrgLookup::Removed => return Ok(Err(Unresolved::OrgRemoved)),
        };
        // An org that gives values has an accepted policy. Without one, nothing is guessed.
        let Some(policy) = policy else {
            let error = "no policy has been accepted".to_string();
            return Ok(Err(Unresolved::PolicyInvalid(error)));
        };
        if !policy.may_use(name) {
            return Ok(Err(Unresolved::DeniedByPolicy));
        }
        let chosen = match policy.personal {
            PersonalMode::Deny => in_org,
            PersonalMode::Fallback => match in_org {
                Some(found) => Some(found),
                None => self.personal_entry(name).await?,
            },
            PersonalMode::Override => match self.personal_entry(name).await? {
                Some(found) => Some(found),
                None => in_org,
            },
        };
        let Some((value, secret)) = chosen else {
            return Ok(Err(Unresolved::NotDefined));
        };
        if *consumer == Consumer::Shell && secret && !policy.shell_secrets {
            return Ok(Err(Unresolved::ShellSecretsForbidden));
        }
        Ok(Ok(value))
    }
}
