//! The personal variables of the Env tab (env vault spec §5): a plain value lives in the
//! database, a secret's in the secret store, and a listing never carries a secret's value.

use std::sync::Arc;

use zeroize::Zeroizing;

use super::entries::{check_value, check_variable_name, EntryKind, EnvEntry};
use super::secrets::SecretStore;
use crate::store::Store;
use crate::Result;

/// The secret store key of the personal variable `name`.
fn personal_key(name: &str) -> String {
    format!("personal/{name}")
}

#[derive(Clone)]
pub struct Vault {
    store: Store,
    secrets: Arc<dyn SecretStore>,
}

impl Vault {
    pub fn new(store: Store, secrets: Arc<dyn SecretStore>) -> Vault {
        Vault { store, secrets }
    }

    pub(crate) fn store(&self) -> &Store {
        &self.store
    }

    /// A vault whose secrets live in memory: for a Library or a runner made without the
    /// runtime's (tests).
    pub fn in_memory(store: Store) -> Vault {
        Vault::new(store, Arc::new(super::MemorySecrets::new()))
    }

    /// Every personal variable (`env_list`). A secret's `value` is `None`.
    pub async fn list(&self) -> Result<Vec<EnvEntry>> {
        self.store.list_env_entries().await
    }

    /// Adds a personal variable (`env_add`). A secret's value goes to the secret store; if it
    /// cannot be stored there, the row is taken out again.
    pub async fn add(&self, name: &str, kind: EntryKind, value: &str) -> Result<EnvEntry> {
        check_variable_name(name)?;
        check_value(kind, value)?;
        match kind {
            EntryKind::Plain => self.store.add_env_entry(name, kind, Some(value)).await,
            EntryKind::Secret => {
                let entry = self.store.add_env_entry(name, kind, None).await?;
                if let Err(e) = self.secrets.put(&personal_key(&entry.name), value) {
                    self.store.remove_env_entry(&entry.name).await?;
                    return Err(e);
                }
                super::redact::global().add(&entry.name, value);
                Ok(entry)
            }
        }
    }

    /// Replaces a personal variable's value (`env_set`): a plain one's is edited, a secret's
    /// replaced without being shown.
    pub async fn set_value(&self, name: &str, value: &str) -> Result<EnvEntry> {
        let entry = self.get(name).await?;
        check_value(entry.kind, value)?;
        match entry.kind {
            EntryKind::Plain => self.store.set_env_value(&entry.name, Some(value)).await,
            EntryKind::Secret => {
                self.secrets.put(&personal_key(&entry.name), value)?;
                super::redact::global().add(&entry.name, value);
                self.store.set_env_value(&entry.name, None).await
            }
        }
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
        let Some(entry) = self.store.find_env_entry(name).await? else {
            return Ok(None);
        };
        match entry.kind {
            EntryKind::Plain => Ok(entry.value.map(Zeroizing::new)),
            EntryKind::Secret => {
                let value = self.secrets.get(&personal_key(&entry.name))?;
                if let Some(value) = &value {
                    super::redact::global().add(&entry.name, value);
                }
                Ok(value)
            }
        }
    }

    async fn get(&self, name: &str) -> Result<EnvEntry> {
        self.store
            .find_env_entry(name)
            .await?
            .ok_or_else(|| crate::Error::NotFound(format!("no variable named {name}")))
    }
}
