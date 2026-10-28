//! The Dots ID of this install: a minisign key pair made on first start, and the name and
//! email the person gives for it (accounts design §5). The public key is the ID; the secret
//! key stays in the secret store and never leaves this PC.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::store::Store;
use crate::vault::{org::key_fingerprint, SecretStore};
use crate::{Error, Result};

/// The secret store key of the Dots ID secret key.
pub const IDENTITY_SECRET: &str = "identity";
/// The longest name, in characters.
pub const MAX_PROFILE_NAME: usize = 80;
/// The longest email address, in characters.
pub const MAX_EMAIL: usize = 254;

const PUBLIC_KEY_MARKER: &str = "identity_public_key";
const NAME_MARKER: &str = "profile_name";
const EMAIL_MARKER: &str = "profile_email";

/// A Dots ID. `name` and `email` are empty until the person sets them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Identity {
    /// The public key, in base64.
    pub public_key: String,
    /// The short form of the public key people compare by eye.
    pub fingerprint: String,
    pub name: String,
    pub email: String,
}

/// Makes the key pair when the public key marker or the secret is missing (a lost secret
/// gives a new ID, since the old one could no longer sign), and returns the current identity.
pub async fn ensure_identity(store: &Store, secrets: &dyn SecretStore) -> Result<Identity> {
    let have_marker = text_marker(store, PUBLIC_KEY_MARKER).await?.is_some();
    if !have_marker || secrets.get(IDENTITY_SECRET)?.is_none() {
        let keys = minisign::KeyPair::generate_unencrypted_keypair()
            .map_err(|e| Error::Invalid(format!("could not make the Dots ID: {e}")))?;
        let secret = keys
            .sk
            .to_box(Some("dots identity secret key"))
            .map_err(|e| Error::Invalid(format!("could not encode the Dots ID: {e}")))?
            .into_string();
        secrets.put(IDENTITY_SECRET, &secret)?;
        store
            .set_marker(PUBLIC_KEY_MARKER, &json!(keys.pk.to_base64()), false)
            .await?;
    }
    identity(store).await
}

/// The current identity; an error when none has been made yet.
pub async fn identity(store: &Store) -> Result<Identity> {
    let public_key = text_marker(store, PUBLIC_KEY_MARKER)
        .await?
        .ok_or_else(|| Error::Invalid("this install has no Dots ID yet".into()))?;
    Ok(Identity {
        fingerprint: key_fingerprint(&public_key)?,
        public_key,
        name: text_marker(store, NAME_MARKER).await?.unwrap_or_default(),
        email: text_marker(store, EMAIL_MARKER).await?.unwrap_or_default(),
    })
}

/// Sets the name and email, trimmed, and returns the identity.
pub async fn set_profile(store: &Store, name: &str, email: &str) -> Result<Identity> {
    let (name, email) = (name.trim(), email.trim());
    if name.is_empty() {
        return Err(Error::Invalid("a name is needed".into()));
    }
    if name.chars().count() > MAX_PROFILE_NAME {
        return Err(Error::Invalid(format!(
            "a name must be at most {MAX_PROFILE_NAME} characters"
        )));
    }
    match email.split_once('@') {
        Some((local, domain)) if !local.is_empty() && !domain.is_empty() => {}
        _ => return Err(Error::Invalid("an email address needs an @".into())),
    }
    if email.chars().count() > MAX_EMAIL {
        return Err(Error::Invalid(format!(
            "an email address must be at most {MAX_EMAIL} characters"
        )));
    }
    store.set_marker(NAME_MARKER, &json!(name), false).await?;
    store.set_marker(EMAIL_MARKER, &json!(email), false).await?;
    identity(store).await
}

async fn text_marker(store: &Store, key: &str) -> Result<Option<String>> {
    Ok(match store.marker(key).await? {
        Some(Value::String(s)) => Some(s),
        _ => None,
    })
}
