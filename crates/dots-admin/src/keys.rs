//! The org's policy signing key: a minisign key pair. `policy.key` is the secret key, normally
//! protected by a password; `policy.pub` is the public key, whose `RW…` line goes into every
//! join code.

use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use dots_core::vault::org::key_fingerprint;
use minisign::{KeyPair, PublicKey, SecretKey, SecretKeyBox};

use crate::args::Args;
use crate::{core_failed, failed, say, Ask, Io, PasswordSource, Result};

/// The secret key's file name in `keygen --out <dir>`.
pub const SECRET_KEY_FILE: &str = "policy.key";
/// The public key's file name in `keygen --out <dir>`.
pub const PUBLIC_KEY_FILE: &str = "policy.pub";

/// A key pair `keygen` wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewKeys {
    pub secret_path: PathBuf,
    pub public_path: PathBuf,
    /// The bare `RW…` public key, as a join code holds it.
    pub public_key: String,
    /// The key's fingerprint as Dots shows it when someone joins (`key_fingerprint`, a digest
    /// of the whole key). It is not the key id in `policy.pub`'s comment.
    pub fingerprint: String,
}

/// Makes the org's key pair and writes `policy.key` and `policy.pub` into `dir`, creating `dir`
/// if needed. With `password` the secret key is encrypted with it (minisign's scrypt); `None`
/// writes an unprotected key. A file that is already there is never overwritten: the old key
/// may be the one every join code names.
pub fn keygen(dir: &Path, password: Option<&str>) -> Result<NewKeys> {
    let secret_path = dir.join(SECRET_KEY_FILE);
    let public_path = dir.join(PUBLIC_KEY_FILE);
    for path in [&secret_path, &public_path] {
        if path.exists() {
            return Err(exists(path));
        }
    }
    if password == Some("") {
        return Err(failed(
            "an empty password does not protect the key; give one, or use --no-password for \
             an unprotected key",
        ));
    }
    let keys = match password {
        Some(password) => KeyPair::generate_encrypted_keypair(Some(password.to_string())),
        None => KeyPair::generate_unencrypted_keypair(),
    }
    .map_err(|e| failed(format!("could not make the key pair: {e}")))?;
    let comment = match password {
        Some(_) => "dots-admin policy secret key",
        None => "dots-admin policy secret key, not password-protected",
    };
    let boxed = |e: minisign::PError| failed(format!("could not encode the key pair: {e}"));
    let secret_text = keys.sk.to_box(Some(comment)).map_err(boxed)?.into_string();
    let public_text = keys.pk.to_box().map_err(boxed)?.into_string();
    let public_key = keys.pk.to_base64();
    let fingerprint = key_fingerprint(&public_key).map_err(core_failed)?;

    std::fs::create_dir_all(dir)
        .map_err(|e| failed(format!("could not create {}: {e}", dir.display())))?;
    write_new(&secret_path, &secret_text, true)?;
    if let Err(e) = write_new(&public_path, &public_text, false) {
        // A secret key without its public key is of no use to anyone.
        let _ = std::fs::remove_file(&secret_path);
        return Err(e);
    }
    Ok(NewKeys {
        secret_path,
        public_path,
        public_key,
        fingerprint,
    })
}

/// `keygen --out <dir> [--no-password]`.
pub(crate) fn keygen_command(args: &[String], io: &mut Io<'_>) -> Result<()> {
    let args = Args::parse(args, &["out"], &["no-password"])?;
    args.none()?;
    let dir = Path::new(args.required("out")?);
    // Checked before the password is asked for, so nobody types one for a key that won't be
    // written.
    for name in [SECRET_KEY_FILE, PUBLIC_KEY_FILE] {
        if dir.join(name).exists() {
            return Err(exists(&dir.join(name)));
        }
    }
    let password = if args.switch("no-password") {
        say(
            io.err,
            "warning: policy.key is not password-protected; anyone who copies it can sign \
             policies for this org",
        )?;
        None
    } else {
        Some((io.password)(Ask::NewKey)?)
    };
    let keys = keygen(dir, password.as_deref())?;
    say(io.out, &format!("wrote {}", keys.secret_path.display()))?;
    say(io.out, &format!("wrote {}", keys.public_path.display()))?;
    say(io.out, &format!("public key: {}", keys.public_key))?;
    say(io.out, &format!("key fingerprint: {}", keys.fingerprint))?;
    say(
        io.out,
        "Keep policy.key offline. Join codes carry policy.pub; Dots shows the fingerprint above, \
         not the key id in policy.pub's comment.",
    )
}

/// The bare `RW…` public key from a `policy.pub` file's text, or from that line alone.
pub fn read_public_key(text: &str) -> Result<String> {
    let not_a_key = || {
        failed("the --pub file is not a minisign public key (policy.pub from 'dots-admin keygen')")
    };
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let key = match lines.as_slice() {
        [key] => *key,
        [comment, key] if comment.starts_with("untrusted comment:") => *key,
        _ => return Err(not_a_key()),
    };
    PublicKey::from_base64(key).map_err(|_| not_a_key())?;
    Ok(key.to_string())
}

/// Reads a `policy.key` file's text. The password is asked for only when the key has one.
pub fn unlock_secret_key(text: &str, password: PasswordSource<'_>) -> Result<SecretKey> {
    let not_a_key = || {
        failed("the --key file is not a minisign secret key (policy.key from 'dots-admin keygen')")
    };
    // The box is a comment line, then the key's base64. Its header says whether it is
    // encrypted.
    let line = text.lines().nth(1).ok_or_else(not_a_key)?;
    let bytes = STANDARD.decode(line.trim()).map_err(|_| not_a_key())?;
    let header = SecretKey::from_bytes(&bytes).map_err(|_| not_a_key())?;
    let sk_box = SecretKeyBox::from_string(text).map_err(|_| not_a_key())?;
    if !header.is_encrypted() {
        return SecretKey::from_unencrypted_box(sk_box).map_err(|_| not_a_key());
    }
    let password = password(Ask::Unlock)?;
    SecretKey::from_box(sk_box, Some(password)).map_err(|e| match e.kind() {
        minisign::ErrorKind::Verify => failed("wrong password for the policy key"),
        _ => not_a_key(),
    })
}

/// Creates `path` with `text`, refusing to replace a file. The secret key is readable by its
/// owner only where the OS has such modes; on Windows it takes the folder's permissions.
fn write_new(path: &Path, text: &str, secret: bool) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    if secret {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(not(unix))]
    let _ = secret;
    let mut file = options.open(path).map_err(|e| match e.kind() {
        ErrorKind::AlreadyExists => exists(path),
        _ => failed(format!("could not write {}: {e}", path.display())),
    })?;
    file.write_all(text.as_bytes())
        .map_err(|e| failed(format!("could not write {}: {e}", path.display())))
}

fn exists(path: &Path) -> crate::AdminError {
    failed(format!(
        "{} already exists; dots-admin never overwrites a key. Move it away or choose another \
         --out",
        path.display()
    ))
}
