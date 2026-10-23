//! Where secret values are kept (env vault spec §5.2): Windows Credential Manager, one generic
//! credential per value, its target `Dots/<install_id>/<key>`. The install id keeps a test or
//! dev data folder away from the installed app's secrets. Tests use `MemorySecrets`.

use std::collections::BTreeMap;
use std::sync::Mutex;

use zeroize::Zeroizing;

use crate::{Error, Result};

/// The longest value Credential Manager holds (`CRED_MAX_CREDENTIAL_BLOB_SIZE`), in bytes.
pub const MAX_SECRET_BYTES: usize = 2560;

/// A store of secret values by key (`personal/<NAME>`). A key is ASCII without `*`. Its
/// `Debug` never shows a value.
pub trait SecretStore: Send + Sync + std::fmt::Debug {
    /// Stores `value` under `key`, replacing what was there.
    fn put(&self, key: &str, value: &str) -> Result<()>;
    /// The value under `key`, if any.
    fn get(&self, key: &str) -> Result<Option<Zeroizing<String>>>;
    /// Deletes the value under `key`; a key with none is not an error.
    fn delete(&self, key: &str) -> Result<()>;
    /// Every key that starts with `prefix`, sorted.
    fn keys(&self, prefix: &str) -> Result<Vec<String>>;
}

fn check(key: &str, value: Option<&str>) -> Result<()> {
    if key.is_empty() || !key.is_ascii() || key.contains('*') {
        return Err(Error::Invalid(format!("not a secret key: {key:?}")));
    }
    if value.is_some_and(|v| v.len() > MAX_SECRET_BYTES) {
        return Err(Error::Invalid(format!(
            "a secret is at most {MAX_SECRET_BYTES} bytes"
        )));
    }
    Ok(())
}

/// Secrets in memory, gone when the process ends. Tests use it; so does `Config::new`.
#[derive(Default)]
pub struct MemorySecrets {
    values: Mutex<BTreeMap<String, Zeroizing<String>>>,
}

impl std::fmt::Debug for MemorySecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let count = self.values.lock().map(|v| v.len()).unwrap_or(0);
        f.debug_struct("MemorySecrets")
            .field("count", &count)
            .finish()
    }
}

impl MemorySecrets {
    pub fn new() -> MemorySecrets {
        MemorySecrets::default()
    }
}

impl SecretStore for MemorySecrets {
    fn put(&self, key: &str, value: &str) -> Result<()> {
        check(key, Some(value))?;
        self.values
            .lock()
            .unwrap()
            .insert(key.to_string(), Zeroizing::new(value.to_string()));
        Ok(())
    }

    fn get(&self, key: &str) -> Result<Option<Zeroizing<String>>> {
        check(key, None)?;
        Ok(self.values.lock().unwrap().get(key).cloned())
    }

    fn delete(&self, key: &str) -> Result<()> {
        check(key, None)?;
        self.values.lock().unwrap().remove(key);
        Ok(())
    }

    fn keys(&self, prefix: &str) -> Result<Vec<String>> {
        Ok(self
            .values
            .lock()
            .unwrap()
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect())
    }
}

/// Windows Credential Manager, for the signed-in Windows user. Each value is a generic
/// credential that persists on this machine only (`CRED_PERSIST_LOCAL_MACHINE`).
#[cfg(windows)]
#[derive(Debug)]
pub struct WindowsCredentials {
    /// `Dots/<install_id>/`.
    prefix: String,
}

#[cfg(windows)]
impl WindowsCredentials {
    pub fn new(install_id: &str) -> WindowsCredentials {
        WindowsCredentials {
            prefix: format!("Dots/{install_id}/"),
        }
    }

    fn target(&self, key: &str) -> Vec<u16> {
        wide(&format!("{}{key}", self.prefix))
    }
}

/// The secret store of the signed-in user for the data folder `install_id`: Credential Manager
/// on Windows, memory elsewhere (Dots ships on Windows; a keychain is later).
pub fn os_secrets(install_id: &str) -> std::sync::Arc<dyn SecretStore> {
    #[cfg(windows)]
    {
        std::sync::Arc::new(WindowsCredentials::new(install_id))
    }
    #[cfg(not(windows))]
    {
        tracing::warn!("no keychain on this system: secrets of {install_id} last until Dots ends");
        std::sync::Arc::new(MemorySecrets::new())
    }
}

#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn last_error(what: &str) -> Error {
    Error::Other(format!(
        "Credential Manager: {what} failed: {}",
        std::io::Error::last_os_error()
    ))
}

#[cfg(windows)]
impl SecretStore for WindowsCredentials {
    fn put(&self, key: &str, value: &str) -> Result<()> {
        use windows_sys::Win32::Security::Credentials::{
            CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
        };
        check(key, Some(value))?;
        let mut target = self.target(key);
        let mut user = wide("Dots");
        let mut blob = Zeroizing::new(value.as_bytes().to_vec());
        let cred = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: target.as_mut_ptr(),
            CredentialBlobSize: blob.len() as u32,
            CredentialBlob: blob.as_mut_ptr(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            UserName: user.as_mut_ptr(),
            ..Default::default()
        };
        // SAFETY: every pointer in `cred` points into a buffer that lives until the call ends.
        if unsafe { CredWriteW(&cred, 0) } == 0 {
            return Err(last_error("saving a secret"));
        }
        Ok(())
    }

    fn get(&self, key: &str) -> Result<Option<Zeroizing<String>>> {
        use windows_sys::Win32::Foundation::ERROR_NOT_FOUND;
        use windows_sys::Win32::Security::Credentials::{
            CredFree, CredReadW, CREDENTIALW, CRED_TYPE_GENERIC,
        };
        check(key, None)?;
        let target = self.target(key);
        let mut cred: *mut CREDENTIALW = std::ptr::null_mut();
        // SAFETY: `target` is NUL-terminated; `cred` receives a buffer freed below.
        if unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut cred) } == 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(ERROR_NOT_FOUND as i32) {
                return Ok(None);
            }
            return Err(Error::Other(format!(
                "Credential Manager: reading a secret failed: {err}"
            )));
        }
        // SAFETY: CredReadW succeeded, so `cred` points to a credential whose blob is
        // `CredentialBlobSize` bytes long; it is copied before the buffer is freed.
        let bytes = unsafe {
            let c = &*cred;
            let bytes = if c.CredentialBlob.is_null() {
                Zeroizing::new(Vec::new())
            } else {
                Zeroizing::new(
                    std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize)
                        .to_vec(),
                )
            };
            CredFree(cred as *const _);
            bytes
        };
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| Error::Other("Credential Manager: a secret is not UTF-8".into()))?;
        Ok(Some(Zeroizing::new(text.to_string())))
    }

    fn delete(&self, key: &str) -> Result<()> {
        use windows_sys::Win32::Foundation::ERROR_NOT_FOUND;
        use windows_sys::Win32::Security::Credentials::{CredDeleteW, CRED_TYPE_GENERIC};
        check(key, None)?;
        let target = self.target(key);
        // SAFETY: `target` is NUL-terminated.
        if unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) } == 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() != Some(ERROR_NOT_FOUND as i32) {
                return Err(Error::Other(format!(
                    "Credential Manager: deleting a secret failed: {err}"
                )));
            }
        }
        Ok(())
    }

    fn keys(&self, prefix: &str) -> Result<Vec<String>> {
        use windows_sys::Win32::Foundation::ERROR_NOT_FOUND;
        use windows_sys::Win32::Security::Credentials::{CredEnumerateW, CredFree, CREDENTIALW};
        let filter = wide(&format!("{}{prefix}*", self.prefix));
        let mut count = 0u32;
        let mut list: *mut *mut CREDENTIALW = std::ptr::null_mut();
        // SAFETY: `filter` is NUL-terminated; `list` receives `count` pointers, freed below.
        if unsafe { CredEnumerateW(filter.as_ptr(), 0, &mut count, &mut list) } == 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(ERROR_NOT_FOUND as i32) {
                return Ok(Vec::new());
            }
            return Err(Error::Other(format!(
                "Credential Manager: listing secrets failed: {err}"
            )));
        }
        let mut keys = Vec::new();
        // SAFETY: CredEnumerateW succeeded: `list` holds `count` credentials, each with a
        // NUL-terminated target name.
        unsafe {
            for i in 0..count as usize {
                let c = &**list.add(i);
                let mut len = 0;
                while *c.TargetName.add(len) != 0 {
                    len += 1;
                }
                let name = String::from_utf16_lossy(std::slice::from_raw_parts(c.TargetName, len));
                if let Some(key) = name.strip_prefix(&self.prefix) {
                    keys.push(key.to_string());
                }
            }
            CredFree(list as *const _);
        }
        keys.sort();
        Ok(keys)
    }
}
