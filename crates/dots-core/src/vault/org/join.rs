//! The join code (env vault spec §6.1): what an admin hands each person to join an org. It is
//! base64url JSON naming the org, its source and the key its policy is signed with. It holds no
//! secrets: the person gives their own credential for the source.

use std::path::Path;

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::policy::plain_serde;
use crate::{Error, Result};

/// The only join code version this Dots reads.
pub const JOIN_CODE_VERSION: u32 = 1;

/// The longest org id. It appears in file names and Credential Manager targets.
pub const MAX_ORG_ID: usize = 63;

/// What a person pastes, or reads from a `.dotsorg` file, to join an org.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JoinCode {
    pub v: u32,
    pub org_id: String,
    /// The org's name as the person sees it, "Acme Engineering".
    pub name: String,
    pub source: SourceSettings,
    /// The minisign public key the org's policy is signed with, the bare `RW…` base64 line.
    pub policy_key: String,
}

/// Where an org's entries and signed policy come from, and how to reach them. It holds no
/// secrets, so it is stored as JSON in the `orgs` row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum SourceSettings {
    /// OpenBao or HashiCorp Vault, KV v2.
    OpenBao(OpenBaoSettings),
    /// Infisical, self-hosted or cloud.
    Infisical(InfisicalSettings),
    /// A SOPS JSON file encrypted to age recipients.
    Sops(SopsSettings),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenBaoSettings {
    pub address: String,
    /// The KV v2 mount.
    #[serde(default = "default_mount")]
    pub mount: String,
    /// Entries are read from `<mount>/data/<path>/env`, the policy from `…/<path>/policy`.
    pub path: String,
    #[serde(default)]
    pub auth: BaoAuth,
    /// The auth mount for `userpass` or `ldap`. `None` means the method's own name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_mount: Option<String>,
    /// Sent as `X-Vault-Namespace`; Vault Enterprise and HCP only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
}

str_enum!(BaoAuth {
    Token => "token",
    Userpass => "userpass",
    Ldap => "ldap",
});

impl Default for BaoAuth {
    /// A token, unless the join code names a login method.
    fn default() -> Self {
        Self::Token
    }
}

impl OpenBaoSettings {
    /// The mount to log in through, `auth/<mount>/login/<user>`: `None` for a token.
    pub fn login_mount(&self) -> Option<&str> {
        match self.auth {
            BaoAuth::Token => None,
            BaoAuth::Userpass | BaoAuth::Ldap => {
                Some(self.auth_mount.as_deref().unwrap_or(self.auth.as_str()))
            }
        }
    }
}

fn default_mount() -> String {
    "secret".into()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InfisicalSettings {
    pub site_url: String,
    pub project_id: String,
    /// The environment slug, `dev` or `prod`.
    pub environment: String,
    /// Entries are read from this folder, the policy from its `policy` sub-folder.
    #[serde(default = "default_secret_path")]
    pub secret_path: String,
}

fn default_secret_path() -> String {
    "/".into()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SopsSettings {
    /// An absolute local path, synced by git or IT, or an `https://` URL.
    pub location: String,
}

impl SourceSettings {
    /// The kind as the join code spells it: `openbao`, `infisical` or `sops`.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::OpenBao(_) => "openbao",
            Self::Infisical(_) => "infisical",
            Self::Sops(_) => "sops",
        }
    }

    /// Where the source is, for the UI: the address, the site or the file location.
    pub fn address(&self) -> &str {
        match self {
            Self::OpenBao(s) => &s.address,
            Self::Infisical(s) => &s.site_url,
            Self::Sops(s) => &s.location,
        }
    }

    /// Checks the addresses and the fields a fetch needs.
    pub fn check(&self) -> Result<()> {
        match self {
            Self::OpenBao(s) => {
                check_address("address", &s.address)?;
                check_not_blank("mount", &s.mount)?;
                check_not_blank("path", &s.path)?;
                if let Some(mount) = &s.auth_mount {
                    check_not_blank("auth_mount", mount)?;
                }
                if let Some(namespace) = &s.namespace {
                    check_not_blank("namespace", namespace)?;
                }
                Ok(())
            }
            Self::Infisical(s) => {
                check_address("site_url", &s.site_url)?;
                check_not_blank("project_id", &s.project_id)?;
                check_not_blank("environment", &s.environment)?;
                if !s.secret_path.starts_with('/') {
                    return Err(refused(format!(
                        "the source's secret_path '{}' must start with '/'",
                        s.secret_path
                    )));
                }
                Ok(())
            }
            Self::Sops(s) if s.location.contains("://") => check_address("location", &s.location),
            Self::Sops(s) => {
                if !Path::new(&s.location).is_absolute() {
                    return Err(refused(format!(
                        "the source's location '{}' must be a full path or an https:// URL",
                        s.location
                    )));
                }
                Ok(())
            }
        }
    }
}

impl JoinCode {
    /// The code as the admin hands it out: base64url of the JSON, without padding.
    pub fn encode(&self) -> String {
        let json = serde_json::to_string(self).expect("a join code serializes");
        URL_SAFE_NO_PAD.encode(json)
    }
}

/// Reads a join code as pasted or read from a `.dotsorg` file. Whitespace anywhere is ignored,
/// so a code wrapped in an email still reads, and `=` padding is optional. Every field is
/// checked: an unknown version, a bad org id, a policy key that is not a minisign public key, or
/// a source that is not `https://` is refused.
pub fn parse_join_code(text: &str) -> Result<JoinCode> {
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = URL_SAFE_NO_PAD
        .decode(compact.trim_end_matches('='))
        .map_err(|_| refused("not base64url text"))?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| refused("no JSON inside"))?;
    match value.get("v").map(Value::as_u64) {
        Some(Some(v)) if v == u64::from(JOIN_CODE_VERSION) => {}
        Some(Some(v)) => {
            return Err(refused(format!(
                "version {v} is not supported; update Dots"
            )))
        }
        _ => return Err(refused("no version 'v'")),
    }
    let code: JoinCode = serde_json::from_slice(&bytes).map_err(|e| refused(plain_serde(&e)))?;
    check_org_id(&code.org_id)?;
    if code.name.trim().is_empty() {
        return Err(refused("the org's name is empty"));
    }
    if minisign_verify::PublicKey::from_base64(&code.policy_key).is_err() {
        return Err(refused(
            "its policy_key is not a minisign public key (the RW… line of the .pub file)",
        ));
    }
    code.source.check()?;
    Ok(code)
}

/// An org id: 1 to 63 characters of `a-z`, `0-9` and `-`, starting with a letter or digit. It
/// appears in file names and Credential Manager targets. `personal` is refused: it is the key
/// of the Personal scope (`PERSONAL_SCOPE`), so an org of that id would share its catalogs.
pub fn check_org_id(org_id: &str) -> Result<()> {
    org_id_rule(org_id).map_err(Error::Invalid)
}

/// `check_org_id`'s rule, its message without the `Error` around it, for the policy's errors.
pub(crate) fn org_id_rule(org_id: &str) -> std::result::Result<(), String> {
    let bytes = org_id.as_bytes();
    let ok = !bytes.is_empty()
        && bytes.len() <= MAX_ORG_ID
        && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        && bytes
            .iter()
            .all(|&b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if !ok {
        return Err(format!(
            "org id '{org_id}': use 1 to {MAX_ORG_ID} characters of a-z, 0-9 or '-', starting \
             with a letter or digit"
        ));
    }
    if org_id == crate::library::PERSONAL_SCOPE {
        return Err("'personal' is reserved for your own keys; choose another org id".into());
    }
    Ok(())
}

/// The policy key's minisign key id, as 16 upper-case hex digits: what `minisign` prints in the
/// `.pub` file's comment, and what the UI shows so the person can compare it with the admin's.
pub fn key_fingerprint(policy_key: &str) -> Result<String> {
    let not_a_key = || Error::Invalid("the policy key is not a minisign public key".into());
    minisign_verify::PublicKey::from_base64(policy_key).map_err(|_| not_a_key())?;
    // `Ed` (2 bytes), the key id (8 bytes, little-endian), the Ed25519 key (32 bytes).
    let bytes = STANDARD.decode(policy_key).map_err(|_| not_a_key())?;
    let id: [u8; 8] = bytes
        .get(2..10)
        .and_then(|b| b.try_into().ok())
        .ok_or_else(not_a_key)?;
    Ok(format!("{:016X}", u64::from_le_bytes(id)))
}

/// A source address: `https://`, or `http://` only to `localhost` or `127.0.0.1`, for test
/// servers. It must name a host and hold no user name or password.
fn check_address(field: &str, address: &str) -> Result<()> {
    let refuse = |why: &str| refused(format!("the source's {field} '{address}' {why}"));
    let url = reqwest::Url::parse(address).map_err(|_| refuse("is not a URL"))?;
    let host = url.host_str().unwrap_or_default();
    if host.is_empty() {
        return Err(refuse("names no host"));
    }
    let local = matches!(host, "localhost" | "127.0.0.1");
    if !(url.scheme() == "https" || url.scheme() == "http" && local) {
        return Err(refuse(
            "must start with https:// (http:// only for localhost or 127.0.0.1)",
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(refuse("must not hold a user name or password"));
    }
    Ok(())
}

fn check_not_blank(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(refused(format!("the source's {field} is empty")));
    }
    Ok(())
}

fn refused(why: impl std::fmt::Display) -> Error {
    Error::Invalid(format!("join code: {why}"))
}
