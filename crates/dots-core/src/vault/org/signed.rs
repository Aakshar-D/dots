//! The signed policy text (env vault spec §6.3): one JSON string holding the policy text and a
//! minisign signature over its exact UTF-8 bytes, so it fits an OpenBao key, an Infisical secret
//! and the SOPS `policy` field alike.
//!
//! `{"v":1,"policy":"<policy JSON text>","signature":"<the whole .minisig text>"}`

use minisign_verify::{PublicKey, Signature};
use serde::{Deserialize, Serialize};

use super::policy::{parse_policy, plain_serde, OrgPolicy, PolicyError};

/// The only envelope version this Dots reads.
pub const ENVELOPE_VERSION: u32 = 1;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    v: u32,
    policy: String,
    signature: String,
}

#[derive(Serialize)]
struct EnvelopeRef<'a> {
    v: u32,
    policy: &'a str,
    signature: &'a str,
}

/// Wraps a policy text and its minisign signature box into the signed text, for `dots-admin`.
/// `policy_text` goes in unchanged: the signature is over its exact bytes.
pub fn envelope(policy_text: &str, signature_box: &str) -> String {
    serde_json::to_string(&EnvelopeRef {
        v: ENVELOPE_VERSION,
        policy: policy_text,
        signature: signature_box,
    })
    .expect("an envelope serializes")
}

/// Verifies the signed text fetched from an org's source against the join code's `policy_key`
/// and `org_id`, in this order: the envelope, the signature (prehashed `ED` only), the policy,
/// then the org. The policy is not read until its signature checks out. Rollback is the
/// caller's check, `OrgPolicy::check_not_rolled_back`, since it needs the `orgs` row.
pub fn verify_policy(
    signed: &str,
    policy_key: &str,
    org_id: &str,
) -> Result<OrgPolicy, PolicyError> {
    let envelope: Envelope =
        serde_json::from_str(signed).map_err(|e| PolicyError::Envelope(plain_serde(&e)))?;
    if envelope.v != ENVELOPE_VERSION {
        return Err(PolicyError::Envelope(format!(
            "version {} is not supported; this Dots reads version {ENVELOPE_VERSION}",
            envelope.v
        )));
    }
    let key = PublicKey::from_base64(policy_key).map_err(|_| PolicyError::Key)?;
    let signature = Signature::decode(&envelope.signature)
        .map_err(|_| PolicyError::Envelope("the signature is not a minisign signature".into()))?;
    key.verify(envelope.policy.as_bytes(), &signature, false)
        .map_err(|_| PolicyError::Signature)?;

    let policy = parse_policy(&envelope.policy)?;
    if policy.org_id != org_id {
        return Err(PolicyError::WrongOrg {
            found: policy.org_id,
            expected: org_id.to_string(),
        });
    }
    Ok(policy)
}
