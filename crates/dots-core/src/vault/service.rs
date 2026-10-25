//! The personal variables of the Env tab (env vault spec §5): a plain value lives in the
//! database, a secret's in the secret store, and a listing never carries a secret's value.

use std::collections::HashSet;
use std::sync::Arc;

use zeroize::Zeroizing;

use super::entries::{check_value, check_variable_name, EntryKind, EnvEntry};
use super::grants::dots_granting;
use super::org::Orgs;
use super::resolve::DotScope;
use super::secrets::SecretStore;
use crate::store::Store;
use crate::Result;

/// Where the personal variables' secrets are kept in the secret store.
const PERSONAL_PREFIX: &str = "personal/";

/// The secret store key of the personal variable `name`.
fn personal_key(name: &str) -> String {
    format!("{PERSONAL_PREFIX}{name}")
}

#[derive(Clone)]
pub struct Vault {
    store: Store,
    secrets: Arc<dyn SecretStore>,
    /// The orgs joined on this PC, where a dot in an org resolves (env vault spec §6, §8).
    /// `None` resolves every org as not joined.
    orgs: Option<Orgs>,
}

impl Vault {
    pub fn new(store: Store, secrets: Arc<dyn SecretStore>) -> Vault {
        Vault {
            store,
            secrets,
            orgs: None,
        }
    }

    /// The vault, with the orgs a dot in an org resolves from. The runtime gives it the
    /// service it loaded at start.
    pub fn with_orgs(self, orgs: Orgs) -> Vault {
        Vault {
            orgs: Some(orgs),
            ..self
        }
    }

    /// The orgs joined on this PC, if the vault has them.
    pub fn orgs(&self) -> Option<&Orgs> {
        self.orgs.as_ref()
    }

    /// The vault of a start: every Personal secret is read once and added to the redactor, so
    /// a value is scrubbed from what Dots writes before any run or Library call reads it. A
    /// secret that cannot be read is logged by name and skipped.
    pub async fn open(store: Store, secrets: Arc<dyn SecretStore>) -> Result<Vault> {
        let vault = Vault::new(store, secrets);
        for entry in vault.store.list_env_entries().await? {
            if entry.kind != EntryKind::Secret {
                continue;
            }
            match vault.secrets.get(&personal_key(&entry.name)) {
                Ok(Some(value)) => super::redact::global().add(&entry.name, &value),
                Ok(None) => {}
                Err(e) => tracing::warn!("could not read the secret {:?}: {e}", entry.name),
            }
        }
        Ok(vault)
    }

    pub(crate) fn store(&self) -> &Store {
        &self.store
    }

    /// A vault whose secrets live in memory: for a Library or a runner made without the
    /// runtime's (tests).
    pub fn in_memory(store: Store) -> Vault {
        Vault::new(store, Arc::new(super::MemorySecrets::new()))
    }

    /// Every personal variable (`env_list`), with the Personal dots that grant it and whether
    /// a secret's value is missing. A secret's `value` is `None`.
    pub async fn list(&self) -> Result<Vec<EnvEntry>> {
        let entries = self.store.list_env_entries().await?;
        self.listed(entries).await
    }

    /// `entries` with `used_by` and `value_missing` filled in. Whether a secret has a value is
    /// read from the secret store's keys, so no value is read. If the keys can't be listed, no
    /// secret is marked missing and the failure is logged.
    async fn listed(&self, mut entries: Vec<EnvEntry>) -> Result<Vec<EnvEntry>> {
        let dots = self.store.list_dots().await?;
        let stored = self.personal_secret_keys();
        for entry in &mut entries {
            entry.used_by = dots_granting(&dots, &DotScope::Personal, &entry.name);
            entry.value_missing = entry.kind == EntryKind::Secret
                && stored.as_ref().is_some_and(|stored| {
                    !stored.contains(&personal_key(&entry.name).to_ascii_lowercase())
                });
        }
        Ok(entries)
    }

    /// The secret store's keys of the Personal secrets, lower-cased, so whether a secret has
    /// a value is known without reading it. `None`, logged, when they can't be listed.
    pub(crate) fn personal_secret_keys(&self) -> Option<HashSet<String>> {
        match self.secrets.keys(PERSONAL_PREFIX) {
            // Credential Manager compares targets without regard to case.
            Ok(keys) => Some(keys.iter().map(|k| k.to_ascii_lowercase()).collect()),
            Err(e) => {
                tracing::warn!("could not list the personal secrets: {e}");
                None
            }
        }
    }

    /// One entry, as `list` gives it.
    async fn listed_one(&self, entry: EnvEntry) -> Result<EnvEntry> {
        let mut listed = self.listed(vec![entry]).await?;
        Ok(listed.remove(0))
    }

    /// Adds a personal variable (`env_add`). A secret's value goes to the secret store; if it
    /// cannot be stored there, the row is taken out again.
    pub async fn add(&self, name: &str, kind: EntryKind, value: &str) -> Result<EnvEntry> {
        check_variable_name(name)?;
        check_value(kind, value)?;
        let entry = match kind {
            EntryKind::Plain => self.store.add_env_entry(name, kind, Some(value)).await?,
            EntryKind::Secret => {
                let entry = self.store.add_env_entry(name, kind, None).await?;
                if let Err(e) = self.secrets.put(&personal_key(&entry.name), value) {
                    self.store.remove_env_entry(&entry.name).await?;
                    return Err(e);
                }
                super::redact::global().add(&entry.name, value);
                entry
            }
        };
        self.listed_one(entry).await
    }

    /// Replaces a personal variable's value (`env_set`): a plain one's is edited, a secret's
    /// replaced without being shown.
    pub async fn set_value(&self, name: &str, value: &str) -> Result<EnvEntry> {
        let entry = self.get(name).await?;
        check_value(entry.kind, value)?;
        let entry = match entry.kind {
            EntryKind::Plain => self.store.set_env_value(&entry.name, Some(value)).await?,
            EntryKind::Secret => {
                self.secrets.put(&personal_key(&entry.name), value)?;
                super::redact::global().add(&entry.name, value);
                self.store.set_env_value(&entry.name, None).await?
            }
        };
        self.listed_one(entry).await
    }

    /// Removes a personal variable (`env_remove`): a secret's value first, so a failure there
    /// keeps the row to try again.
    pub async fn remove(&self, name: &str) -> Result<()> {
        let entry = self.get(name).await?;
        if entry.kind == EntryKind::Secret {
            self.secrets.delete(&personal_key(&entry.name))?;
        }
        self.store.remove_env_entry(&entry.name).await
    }

    /// The value of the personal variable `name`, in any case, if there is one.
    pub(crate) async fn personal_value(&self, name: &str) -> Result<Option<Zeroizing<String>>> {
        Ok(self.personal_entry(name).await?.map(|(value, _)| value))
    }

    /// The value of the personal variable `name`, in any case, and whether it is a secret.
    pub(crate) async fn personal_entry(
        &self,
        name: &str,
    ) -> Result<Option<(Zeroizing<String>, bool)>> {
        let Some(entry) = self.store.find_env_entry(name).await? else {
            return Ok(None);
        };
        match entry.kind {
            EntryKind::Plain => Ok(entry.value.map(|value| (Zeroizing::new(value), false))),
            EntryKind::Secret => {
                let value = self.secrets.get(&personal_key(&entry.name))?;
                if let Some(value) = &value {
                    super::redact::global().add(&entry.name, value);
                }
                Ok(value.map(|value| (value, true)))
            }
        }
    }

    /// Whether Personal has a value for `name`, in any case, and whether it is a secret,
    /// without reading a secret's value: a secret has one when its key is among `stored`
    /// (`personal_secret_keys`), or always when those could not be listed. `None` when it has
    /// none, as `personal_entry` would say.
    pub(crate) async fn personal_kind(
        &self,
        name: &str,
        stored: Option<&HashSet<String>>,
    ) -> Result<Option<bool>> {
        let Some(entry) = self.store.find_env_entry(name).await? else {
            return Ok(None);
        };
        Ok(match entry.kind {
            EntryKind::Plain => entry.value.map(|_| false),
            EntryKind::Secret => {
                let key = personal_key(&entry.name).to_ascii_lowercase();
                stored
                    .is_none_or(|stored| stored.contains(&key))
                    .then_some(true)
            }
        })
    }

    async fn get(&self, name: &str) -> Result<EnvEntry> {
        self.store
            .find_env_entry(name)
            .await?
            .ok_or_else(|| crate::Error::NotFound(format!("no variable named {name}")))
    }
}
