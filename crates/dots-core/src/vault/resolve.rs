//! Resolving a dot's variables (env vault spec §8): a name resolves only when it is granted to
//! the consumer, and only from Personal. What does not resolve says which variable and why.

use std::collections::BTreeMap;

use zeroize::Zeroizing;

use super::grants::{Consumer, Grants};
use super::service::Vault;
use crate::Result;

/// Why a variable did not resolve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unresolved {
    NotGranted,
    NotDefined,
}

/// A variable that did not resolve for a consumer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Missing {
    pub name: String,
    pub consumer: Consumer,
    pub why: Unresolved,
}

impl Missing {
    /// The words of a run event, for the dot `dot`.
    pub fn text(&self, dot: &str) -> String {
        let consumer = self.consumer.key();
        match self.why {
            Unresolved::NotGranted => {
                format!("{}: not granted to {consumer} in dot '{dot}'", self.name)
            }
            Unresolved::NotDefined => {
                format!("{}: not in Personal (dot '{dot}', {consumer})", self.name)
            }
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
    /// Resolves `names` for `consumer` under `grants`: all of them, or the first that does not
    /// resolve.
    pub async fn resolve(
        &self,
        grants: &Grants,
        consumer: &Consumer,
        names: &[String],
    ) -> Result<std::result::Result<Resolved, Missing>> {
        let mut out = BTreeMap::new();
        for name in names {
            let missing = |why| Missing {
                name: name.clone(),
                consumer: consumer.clone(),
                why,
            };
            if !grants.allows(consumer, name) {
                return Ok(Err(missing(Unresolved::NotGranted)));
            }
            match self.personal_value(name).await? {
                Some(value) => {
                    out.insert(name.clone(), value);
                }
                None => return Ok(Err(missing(Unresolved::NotDefined))),
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
}
