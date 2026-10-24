//! The org's offline copy (env vault spec §5.2 and §6.4): the last fetched entries and the signed
//! policy, kept so an org still resolves while its source is unreachable, until the policy's
//! `offline_hours` run out. The copy is JSON, protected with DPAPI for the signed-in Windows user
//! with the org id as entropy, and written to `data_dir/vault/<org_id>.bin`. Nothing in it is
//! ever on disk in the clear (spec §2).

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::check_org_id;
use crate::{Error, Result};

/// The folder of the offline copies, under the data folder.
pub const OFFLINE_DIR: &str = "vault";

/// The offline copy's format. A copy of another version is refused, never guessed at.
const FORMAT: u32 = 1;

const RENAME_ATTEMPTS: u32 = 5;
const RENAME_PAUSE: std::time::Duration = std::time::Duration::from_millis(100);

/// Encrypts bytes so that only this Windows user on this PC can read them back. `entropy` must be
/// the same on both sides: the offline copy passes its org id, so one org's file never opens as
/// another's. Its `Debug` never shows data.
pub trait Protector: Send + Sync + std::fmt::Debug {
    /// `data`, encrypted.
    fn protect(&self, data: &[u8], entropy: &[u8]) -> Result<Vec<u8>>;
    /// The bytes `protect` was given. Data from another user, another entropy or another
    /// protector is an error.
    fn unprotect(&self, data: &[u8], entropy: &[u8]) -> Result<Zeroizing<Vec<u8>>>;
}

/// DPAPI (`CryptProtectData`), in the signed-in user's scope, never asking for a prompt.
#[cfg(windows)]
#[derive(Debug, Default, Clone, Copy)]
pub struct Dpapi;

#[cfg(windows)]
mod dpapi {
    use windows_sys::Win32::Foundation::{LocalFree, ERROR_INVALID_DATA};
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };
    use zeroize::{Zeroize, Zeroizing};

    use super::{Dpapi, Protector};
    use crate::{Error, Result};

    /// A blob that borrows `bytes`. DPAPI only reads an input blob.
    fn blob(bytes: &[u8]) -> Result<CRYPT_INTEGER_BLOB> {
        let len = u32::try_from(bytes.len())
            .map_err(|_| Error::Invalid("DPAPI protects at most 4 GiB at a time".into()))?;
        Ok(CRYPT_INTEGER_BLOB {
            cbData: len,
            pbData: bytes.as_ptr() as *mut u8,
        })
    }

    fn empty() -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        }
    }

    fn failed(what: &str) -> Error {
        let err = std::io::Error::last_os_error();
        let why = if err.raw_os_error() == Some(ERROR_INVALID_DATA as i32) {
            " (it was protected by another Windows user, with other entropy, or is damaged)"
        } else {
            ""
        };
        Error::Other(format!("DPAPI: {what} failed: {err}{why}"))
    }

    /// Copies a blob DPAPI returned, wipes it and frees it.
    ///
    /// # Safety
    ///
    /// `out` must be a blob a successful DPAPI call filled: `pbData` is null or holds `cbData`
    /// bytes allocated with `LocalAlloc`, owned by the caller.
    unsafe fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
        if out.pbData.is_null() {
            return Vec::new();
        }
        let bytes = std::slice::from_raw_parts_mut(out.pbData, out.cbData as usize);
        let copy = bytes.to_vec();
        bytes.zeroize();
        LocalFree(out.pbData.cast());
        copy
    }

    impl Protector for Dpapi {
        fn protect(&self, data: &[u8], entropy: &[u8]) -> Result<Vec<u8>> {
            let input = blob(data)?;
            let entropy = blob(entropy)?;
            let mut out = empty();
            // SAFETY: `input` and `entropy` borrow slices that live until the call ends, and
            // `out` receives a buffer that `take` frees.
            let ok = unsafe {
                CryptProtectData(
                    &input,
                    std::ptr::null(),
                    &entropy,
                    std::ptr::null(),
                    std::ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut out,
                )
            };
            if ok == 0 {
                return Err(failed("protecting data"));
            }
            // SAFETY: the call succeeded, so `out` is a blob it allocated.
            Ok(unsafe { take(out) })
        }

        fn unprotect(&self, data: &[u8], entropy: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
            let input = blob(data)?;
            let entropy = blob(entropy)?;
            let mut out = empty();
            // SAFETY: as in `protect`. No description is asked for, so none needs freeing.
            let ok = unsafe {
                CryptUnprotectData(
                    &input,
                    std::ptr::null_mut(),
                    &entropy,
                    std::ptr::null(),
                    std::ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut out,
                )
            };
            if ok == 0 {
                return Err(failed("unprotecting data"));
            }
            // SAFETY: the call succeeded, so `out` is a blob it allocated. `take` wipes it.
            Ok(Zeroizing::new(unsafe { take(out) }))
        }
    }
}

/// For tests only: it hides nothing from anyone. It XORs with a fixed key behind a `DOTSTEST`
/// prefix and the SHA-256 of the entropy, so a test can check that a file holds no plain value
/// and that one org's copy does not open as another's, as under DPAPI.
#[derive(Debug, Default, Clone, Copy)]
pub struct TestProtector;

const TEST_MAGIC: &[u8] = b"DOTSTEST";
const TEST_KEY: &[u8] = b"dots-test-protector-key";

fn xor(data: &[u8]) -> impl Iterator<Item = u8> + '_ {
    data.iter().zip(TEST_KEY.iter().cycle()).map(|(b, k)| b ^ k)
}

impl Protector for TestProtector {
    fn protect(&self, data: &[u8], entropy: &[u8]) -> Result<Vec<u8>> {
        let mut out = Vec::with_capacity(TEST_MAGIC.len() + 32 + data.len());
        out.extend_from_slice(TEST_MAGIC);
        out.extend_from_slice(&Sha256::digest(entropy));
        out.extend(xor(data));
        Ok(out)
    }

    fn unprotect(&self, data: &[u8], entropy: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
        let rest = data
            .strip_prefix(TEST_MAGIC)
            .filter(|rest| rest.len() >= 32)
            .ok_or_else(|| Error::Other("test protector: this is not its data".into()))?;
        let (hash, body) = rest.split_at(32);
        if hash != Sha256::digest(entropy).as_slice() {
            return Err(Error::Other(
                "test protector: the data was protected with other entropy".into(),
            ));
        }
        let mut plain = Zeroizing::new(Vec::with_capacity(body.len()));
        plain.extend(xor(body));
        Ok(plain)
    }
}

/// Refuses everything, on a system without DPAPI: there is no offline copy rather than one in
/// the clear.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoProtector;

impl NoProtector {
    fn refuse<T>() -> Result<T> {
        Err(Error::Other("no offline copy on this platform".into()))
    }
}

impl Protector for NoProtector {
    fn protect(&self, _data: &[u8], _entropy: &[u8]) -> Result<Vec<u8>> {
        NoProtector::refuse()
    }

    fn unprotect(&self, _data: &[u8], _entropy: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
        NoProtector::refuse()
    }
}

/// The protector of the signed-in user: DPAPI on Windows, none elsewhere (Dots ships on
/// Windows), so an offline copy is never written in the clear.
pub fn os_protector() -> Arc<dyn Protector> {
    #[cfg(windows)]
    {
        Arc::new(Dpapi)
    }
    #[cfg(not(windows))]
    {
        tracing::warn!("no DPAPI on this system: orgs keep no offline copy");
        Arc::new(NoProtector)
    }
}

/// An org's last good fetch, as kept on disk. Its JSON carries `"v": 1`, and another version is
/// refused. Its `Debug` shows only `fetched_at` and the names.
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "Stored")]
pub struct OfflineCopy {
    /// When the entries were fetched, RFC 3339. The copy expires `offline_hours` after it.
    pub fetched_at: String,
    /// The org's entries by name, plain values and secrets alike.
    pub entries: BTreeMap<String, Zeroizing<String>>,
    /// The signed policy text the entries came with.
    pub policy: String,
}

impl std::fmt::Debug for OfflineCopy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OfflineCopy")
            .field("fetched_at", &self.fetched_at)
            .field("names", &self.entries.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// The JSON form, read.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    v: u32,
    fetched_at: String,
    entries: BTreeMap<String, Zeroizing<String>>,
    policy: String,
}

/// The JSON form, written.
#[derive(Serialize)]
struct StoredRef<'a> {
    v: u32,
    fetched_at: &'a str,
    entries: &'a BTreeMap<String, Zeroizing<String>>,
    policy: &'a str,
}

/// Only the version, read first so a copy of a later format says so rather than naming a field.
#[derive(Deserialize)]
struct Head {
    v: u32,
}

fn check_version(v: u32) -> std::result::Result<(), String> {
    if v != FORMAT {
        return Err(format!("it has format v{v}, and this Dots reads v{FORMAT}"));
    }
    Ok(())
}

impl OfflineCopy {
    fn check(&self) -> std::result::Result<(), String> {
        chrono::DateTime::parse_from_rfc3339(&self.fetched_at)
            .map(|_| ())
            .map_err(|_| "its fetched_at is not an RFC 3339 time".to_string())
    }
}

impl TryFrom<Stored> for OfflineCopy {
    type Error = String;

    fn try_from(stored: Stored) -> std::result::Result<OfflineCopy, String> {
        check_version(stored.v)?;
        let copy = OfflineCopy {
            fetched_at: stored.fetched_at,
            entries: stored.entries,
            policy: stored.policy,
        };
        copy.check()?;
        Ok(copy)
    }
}

impl Serialize for OfflineCopy {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        StoredRef {
            v: FORMAT,
            fetched_at: &self.fetched_at,
            entries: &self.entries,
            policy: &self.policy,
        }
        .serialize(s)
    }
}

/// Where the offline copy of `org_id` lives in `dir`: `dir/<org_id>.bin`. The id must be a valid
/// org id, since it names a file.
pub fn offline_path(dir: &Path, org_id: &str) -> Result<PathBuf> {
    check_org_id(org_id)?;
    Ok(dir.join(format!("{org_id}.bin")))
}

/// Counts the bytes written to it, to size the plain buffer once: a buffer that grows would leave
/// copies of the values behind in memory it gave back.
struct Count(usize);

impl std::io::Write for Count {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0 += buf.len();
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Saves `copy` as the offline copy of `org_id` in `dir`, replacing the one there. The file is
/// written beside the target and renamed over it, so a write that fails leaves the old copy.
pub fn write_offline(
    dir: &Path,
    protector: &dyn Protector,
    org_id: &str,
    copy: &OfflineCopy,
) -> Result<()> {
    let path = offline_path(dir, org_id)?;
    copy.check()
        .map_err(|why| Error::Invalid(format!("org '{org_id}': an offline copy: {why}")))?;
    let mut count = Count(0);
    serde_json::to_writer(&mut count, copy)?;
    let mut plain = Zeroizing::new(Vec::with_capacity(count.0));
    serde_json::to_writer(&mut *plain, copy)?;
    let sealed = protector.protect(&plain, org_id.as_bytes())?;
    drop(plain);

    let failed = |e: std::io::Error| {
        Error::Other(format!(
            "org '{org_id}': saving its offline copy failed: {e}"
        ))
    };
    std::fs::create_dir_all(dir).map_err(failed)?;
    let tmp = dir.join(format!(".{org_id}.{}.tmp", crate::util::new_id()));
    let written = write_synced(&tmp, &sealed).and_then(|()| rename(&tmp, &path));
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(failed(e));
    }
    Ok(())
}

fn write_synced(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = std::fs::File::create_new(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// Renames `from` over `to`. A file just written may be held for a moment (antivirus, the
/// indexer), and Windows then refuses the rename: try again a few times.
fn rename(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut attempt = 1;
    loop {
        match std::fs::rename(from, to) {
            Err(_) if attempt < RENAME_ATTEMPTS => {
                attempt += 1;
                std::thread::sleep(RENAME_PAUSE);
            }
            other => return other,
        }
    }
}

/// The offline copy of `org_id` in `dir`, or `None` if there is none. A copy that does not
/// unprotect or parse is an error naming the org; the error never holds a value.
pub fn read_offline(
    dir: &Path,
    protector: &dyn Protector,
    org_id: &str,
) -> Result<Option<OfflineCopy>> {
    let path = offline_path(dir, org_id)?;
    let sealed = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(Error::Other(format!(
                "org '{org_id}': reading its offline copy failed: {e}"
            )))
        }
    };
    let unreadable = |why: String| {
        Error::Other(format!(
            "org '{org_id}': its offline copy cannot be read: {why}"
        ))
    };
    let plain = protector
        .unprotect(&sealed, org_id.as_bytes())
        .map_err(|e| unreadable(e.to_string()))?;
    // serde_json's messages may quote the text they choke on, which could be a value: name only
    // the place.
    let damaged = |e: serde_json::Error| {
        unreadable(format!(
            "it is damaged (line {}, column {})",
            e.line(),
            e.column()
        ))
    };
    let head: Head = serde_json::from_slice(&plain).map_err(damaged)?;
    check_version(head.v).map_err(unreadable)?;
    let stored: Stored = serde_json::from_slice(&plain).map_err(damaged)?;
    OfflineCopy::try_from(stored).map(Some).map_err(unreadable)
}

/// Deletes the offline copy of `org_id` in `dir`, with any temp file a crash left behind. A copy
/// that is not there is not an error.
pub fn delete_offline(dir: &Path, org_id: &str) -> Result<()> {
    let path = offline_path(dir, org_id)?;
    let failed = |e: std::io::Error| {
        Error::Other(format!(
            "org '{org_id}': deleting its offline copy failed: {e}"
        ))
    };
    match std::fs::remove_file(&path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(failed(e)),
        _ => {}
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(failed(e)),
    };
    // An org id has no '.', so `.acme.` never matches a temp file of `acme-eng`.
    let prefix = format!(".{org_id}.");
    for entry in entries {
        let name = entry.map_err(failed)?.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(&prefix) && name.ends_with(".tmp") {
            match std::fs::remove_file(dir.join(&*name)) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(failed(e)),
                _ => {}
            }
        }
    }
    Ok(())
}
