//! A secret store that fails for some keys, as Credential Manager can for one name.

use dots_core::vault::{MemorySecrets, SecretStore};
use dots_core::{Error, Result};
use zeroize::Zeroizing;

/// `MemorySecrets`, except that a key holding `fails` cannot be read or written.
#[derive(Debug)]
pub struct FailingSecrets {
    inner: MemorySecrets,
    fails: String,
}

impl FailingSecrets {
    pub fn new(fails: &str) -> FailingSecrets {
        FailingSecrets {
            inner: MemorySecrets::new(),
            fails: fails.to_string(),
        }
    }

    /// Writes `value` under `key` past the failure, as a value stored before it began.
    pub fn preload(&self, key: &str, value: &str) {
        self.inner.put(key, value).unwrap();
    }

    fn check(&self, key: &str) -> Result<()> {
        if key.contains(&self.fails) {
            return Err(Error::Other(format!(
                "Credential Manager: {key} failed (test)"
            )));
        }
        Ok(())
    }
}

impl SecretStore for FailingSecrets {
    fn put(&self, key: &str, value: &str) -> Result<()> {
        self.check(key)?;
        self.inner.put(key, value)
    }

    fn get(&self, key: &str) -> Result<Option<Zeroizing<String>>> {
        self.check(key)?;
        self.inner.get(key)
    }

    fn delete(&self, key: &str) -> Result<()> {
        self.check(key)?;
        self.inner.delete(key)
    }

    fn keys(&self, prefix: &str) -> Result<Vec<String>> {
        self.inner.keys(prefix)
    }
}
