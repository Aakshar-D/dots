//! The release signing key (updates UI spec section 5.2) and GitHub's sealed secrets. The key
//! pair is encoded as `tauri signer generate` writes it: the private key is base64 of
//! minisign's secret-key box text, encrypted with the publisher's password; the public key in
//! `plugins.updater.pubkey` is base64 of the public-key box text.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use crypto_box::aead::OsRng;
use minisign::KeyPair;

use crate::{Error, Result};

/// The prototype's throwaway updater key (`tools/release/release.mjs`): never released with.
pub const PROTOTYPE_PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEI2NTAyQ0VBNUM4N0VCOEEKUldTSzY0ZGM2aXhRdHUxQ0RJcGRmRGZTeVlqcUZJZzIzaEQ2NGxYOHNPUlpwL1lWdCtkTXVFY2MK";

pub const MIN_PASSWORD: usize = 12;

/// A key pair made in memory. Debug prints neither half.
#[derive(Clone)]
pub struct MadeKey {
    pub private_b64: String,
    pub public_b64: String,
}

impl std::fmt::Debug for MadeKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MadeKey { .. }")
    }
}

/// A new key pair, its private half encrypted with `password`.
pub fn make_key(password: &str) -> Result<MadeKey> {
    if password.chars().count() < MIN_PASSWORD {
        return Err(Error::Invalid(
            "The password needs at least 12 characters.".into(),
        ));
    }
    let failed = |e: minisign::PError| Error::Other(format!("could not make the key: {e}"));
    let keys = KeyPair::generate_encrypted_keypair(Some(password.to_string())).map_err(failed)?;
    let secret = keys
        .sk
        .to_box(Some("Dots release signing key"))
        .map_err(failed)?
        .into_string();
    let public = keys.pk.to_box().map_err(failed)?.into_string();
    Ok(MadeKey {
        private_b64: STANDARD.encode(secret),
        public_b64: STANDARD.encode(public),
    })
}

/// `value` sealed for a GitHub secret with the repository's public key (libsodium's
/// `crypto_box_seal`), base64.
pub fn seal(public_key_b64: &str, value: &str) -> Result<String> {
    let bytes: [u8; 32] = STANDARD
        .decode(public_key_b64)
        .map_err(|e| Error::Other(format!("GitHub's secrets key could not be read: {e}")))?
        .try_into()
        .map_err(|_| Error::Other("GitHub's secrets key is not 32 bytes".into()))?;
    let key = crypto_box::PublicKey::from(bytes);
    let sealed = key
        .seal(&mut OsRng, value.as_bytes())
        .map_err(|e| Error::Other(format!("could not seal the secret: {e}")))?;
    Ok(STANDARD.encode(sealed))
}

/// The line that holds `plugins.updater.pubkey` and the value on it.
fn pubkey_line(conf: &str) -> Option<(usize, String)> {
    let parsed: serde_json::Value = serde_json::from_str(conf).ok()?;
    let value = parsed["plugins"]["updater"]["pubkey"].as_str()?.to_string();
    let line = conf
        .lines()
        .position(|l| l.contains("\"pubkey\"") && l.contains(&value))?;
    Some((line, value))
}

/// The updater's public key in a `tauri.conf.json`.
pub fn pubkey_in_conf(tauri_conf_json: &str) -> Option<String> {
    pubkey_line(tauri_conf_json).map(|(_, value)| value)
}

/// `tauri.conf.json` with only `plugins.updater.pubkey`'s value replaced.
pub fn with_pubkey(tauri_conf_json: &str, public_b64: &str) -> Result<String> {
    let (line, old) = pubkey_line(tauri_conf_json)
        .ok_or_else(|| Error::Other("tauri.conf.json has no plugins.updater.pubkey".into()))?;
    let ends_with_newline = tauri_conf_json.ends_with('\n');
    let mut lines: Vec<String> = tauri_conf_json.lines().map(str::to_string).collect();
    lines[line] = lines[line].replace(&old, public_b64);
    let mut out = lines.join("\n");
    if ends_with_newline {
        out.push('\n');
    }
    Ok(out)
}
