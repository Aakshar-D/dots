//! A request code: what a new person sends to an account admin to be added to the member
//! list. It carries the person's Dots ID, name and email, and is signed by that Dots ID, so
//! the admin knows the key really belongs to the sender (accounts design §5).
//!
//! `dotsreq_` + base64url of `{"request":"<AccessRequest JSON text>","signature":"<minisign box>"}`

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use super::identity::{identity, sign_with_identity};
use crate::store::Store;
use crate::vault::org::signed::verify_with_any;
use crate::vault::SecretStore;
use crate::{Error, Result};

/// What a request code starts with.
pub const REQUEST_PREFIX: &str = "dotsreq_";
/// The only request version this Dots reads.
const REQUEST_VERSION: u32 = 1;

/// The signed text of a request code.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AccessRequest {
    pub v: u32,
    pub org_id: String,
    pub invite_id: Option<String>,
    /// The sender's Dots ID (public key, base64).
    pub key: String,
    pub name: String,
    pub email: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub age_recipient: Option<String>,
    pub created_at: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    request: String,
    signature: String,
}

/// Why a request code was not accepted.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RequestError {
    #[error("this is not a Dots request code")]
    NotACode,
    #[error("the request code is not readable: {0}")]
    Unreadable(String),
    #[error("the request code's signature doesn't match its Dots ID")]
    Signature,
    #[error("request code version {0} is not supported")]
    Version(u32),
}

/// Makes a request code for `org_id`, signed with this PC's Dots ID. The name and email come
/// from the profile, which must be filled in first.
pub async fn make_request(
    store: &Store,
    secrets: &dyn SecretStore,
    org_id: &str,
    invite_id: Option<&str>,
    age_recipient: Option<&str>,
    now: DateTime<Utc>,
) -> Result<String> {
    let me = identity(store).await?;
    if me.name.trim().is_empty() || me.email.trim().is_empty() {
        return Err(Error::Invalid(
            "Add your name and email in Settings › Profile first.".into(),
        ));
    }
    let request = AccessRequest {
        v: REQUEST_VERSION,
        org_id: org_id.to_string(),
        invite_id: invite_id.map(str::to_string),
        key: me.public_key,
        name: me.name,
        email: me.email,
        age_recipient: age_recipient.map(str::to_string),
        created_at: now.to_rfc3339_opts(SecondsFormat::Secs, true),
    };
    let text = serde_json::to_string(&request)?;
    let signature =
        sign_with_identity(store, secrets, &text, &format!("dots request {org_id}")).await?;
    let envelope = serde_json::to_vec(&Envelope {
        request: text,
        signature,
    })?;
    Ok(format!(
        "{REQUEST_PREFIX}{}",
        URL_SAFE_NO_PAD.encode(envelope)
    ))
}

/// Reads a request code. The request's own key must verify its signature before any other
/// field is trusted.
pub fn parse_request(code: &str) -> std::result::Result<AccessRequest, RequestError> {
    let compact: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    let body = compact
        .strip_prefix(REQUEST_PREFIX)
        .ok_or(RequestError::NotACode)?;
    let bytes = URL_SAFE_NO_PAD
        .decode(body.trim_end_matches('='))
        .map_err(|_| RequestError::Unreadable("not base64url text".into()))?;
    let envelope: Envelope = serde_json::from_slice(&bytes)
        .map_err(|_| RequestError::Unreadable("no request and signature inside".into()))?;
    let request: AccessRequest = serde_json::from_str(&envelope.request)
        .map_err(|_| RequestError::Unreadable("the request is not in the expected shape".into()))?;
    verify_with_any(&envelope.request, &envelope.signature, &[&request.key])
        .map_err(|()| RequestError::Signature)?;
    if request.v != REQUEST_VERSION {
        return Err(RequestError::Version(request.v));
    }
    Ok(request)
}
