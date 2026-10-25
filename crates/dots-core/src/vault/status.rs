//! The status of a dot's grants (env vault spec §9, the dot editor's Variables): for each name
//! granted to each consumer, whether it resolves in the dot's scope now, from where, and why
//! not. It is worked out by the resolver's own steps, but no value is kept and no Personal
//! secret's value is read, and the org is not fetched first.

use serde::{Deserialize, Serialize};

use super::grants::Consumer;
use super::resolve::{DotScope, Found, Reading, Unresolved, ValueSource};
use super::service::Vault;
use crate::model::Dot;
use crate::Result;

str_enum!(GrantState {
    Ok => "ok",
    NotGranted => "not_granted",
    NotDefined => "not_defined",
    DeniedByPolicy => "denied_by_policy",
    ShellSecretsForbidden => "shell_secrets_forbidden",
    OrgOffline => "org_offline",
    OrgExpired => "org_expired",
    OrgRevoked => "org_revoked",
    PolicyInvalid => "policy_invalid",
    OrgRemoved => "org_removed",
});

impl GrantState {
    /// The state of a name that does not resolve for `why`.
    fn of(why: &Unresolved) -> GrantState {
        match why {
            Unresolved::NotGranted => GrantState::NotGranted,
            Unresolved::NotDefined => GrantState::NotDefined,
            Unresolved::DeniedByPolicy => GrantState::DeniedByPolicy,
            Unresolved::ShellSecretsForbidden => GrantState::ShellSecretsForbidden,
            Unresolved::OrgOffline => GrantState::OrgOffline,
            Unresolved::OrgExpired => GrantState::OrgExpired,
            Unresolved::OrgRevoked => GrantState::OrgRevoked,
            Unresolved::PolicyInvalid(_) => GrantState::PolicyInvalid,
            Unresolved::OrgRemoved => GrantState::OrgRemoved,
        }
    }
}

/// One grant of a dot as its editor shows it. It never holds a value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrantStatus {
    /// `mcp:<server>` or `shell`.
    pub consumer: String,
    /// The name as it is granted.
    pub name: String,
    pub status: GrantState,
    /// Why it does not resolve, in the words of the run event (`Missing::text`); `None` when
    /// it resolves.
    pub text: Option<String>,
    /// Where its value comes from, or would but for the shell's secrets rule; `None` when
    /// nothing was found.
    pub source: Option<ValueSource>,
    /// The value that resolves, or would but for the shell's secrets rule, is a secret: an
    /// org entry not in `plain`, or a Personal secret.
    pub secret: bool,
}

impl Vault {
    /// The status of each of `dot`'s grants in its scope, by consumer (servers by name, then
    /// the shell), each in the order granted. Each name goes through `resolve`'s steps
    /// (`resolve_one`) as at a run's start, but reads only which entries are there: no value
    /// is kept, and no Personal secret's value is read. The org is not fetched first, so a
    /// stale org answers from what it holds now. A dot whose org is not joined gets
    /// `org_removed` for every grant.
    pub async fn grant_statuses(&self, dot: &Dot) -> Result<Vec<GrantStatus>> {
        let mut granted: Vec<(Consumer, String)> = Vec::new();
        for (key, names) in &dot.spec.variables.0 {
            // A stored dot's grants were validated when it was saved.
            let Ok(consumer) = Consumer::parse(key) else {
                tracing::warn!("dot {:?} grants to {key:?}, which is no consumer", dot.id);
                continue;
            };
            granted.extend(names.iter().map(|name| (consumer.clone(), name.clone())));
        }
        let names: Vec<String> = granted.iter().map(|(_, name)| name.clone()).collect();
        let read = self.in_scope(&DotScope::of(dot), &names).await;
        let reading = Reading::Presence(self.personal_secret_keys());
        let mut out = Vec::new();
        for (i, (consumer, name)) in granted.into_iter().enumerate() {
            let answer = self
                .resolve_one(&read, i, &name, true, &consumer, &reading)
                .await?;
            let (status, text, found) = match answer {
                Ok(found) => (GrantState::Ok, None, Some(found)),
                Err((missing, found)) => (
                    GrantState::of(&missing.why),
                    Some(missing.text(&dot.spec.name)),
                    found,
                ),
            };
            let found = found.as_ref();
            out.push(GrantStatus {
                consumer: consumer.key(),
                name,
                status,
                text,
                source: found.map(|f: &Found| f.source),
                secret: found.is_some_and(|f| f.secret),
            });
        }
        Ok(out)
    }
}
