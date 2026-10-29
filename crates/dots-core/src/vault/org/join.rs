//! The join code (env vault spec §6.1): what an admin hands each person to join an org. It is
//! base64url JSON naming the org, its source and the key its policy is signed with. It holds no
//! secrets: the person gives their own credential for the source.

use std::path::Path;

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::policy::plain_serde;
use crate::accounts::Role;
use crate::{Error, Result};

/// The newest join code version this Dots reads and writes. Version 1 is a plain code; version 2
/// may carry an invite offer.
pub const JOIN_CODE_VERSION: u32 = 2;

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
    /// The offer of a role that makes this a personal invite. Only a version 2 code may carry it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invite: Option<InviteOffer>,
}

/// The invite a version 2 join code offers: which invite it is, the role it gives and when the
/// offer ends (RFC 3339). It holds no secrets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InviteOffer {
    pub id: String,
    pub role: Role,
    pub expires_at: String,
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

    /// Where the source is: the address, the site or the file location, as the code gives it
    /// and the `orgs` row keeps it, for fetching. A text, summary or preview shows
    /// `shown_address` instead.
    pub fn address(&self) -> &str {
        match self {
            Self::OpenBao(s) => &s.address,
            Self::Infisical(s) => &s.site_url,
            Self::Sops(s) => &s.location,
        }
    }

    /// Where the source is, as the UI and every message show it: `address` without a URL's
    /// user name, password, query or fragment, which may carry a token.
    pub fn shown_address(&self) -> String {
        shown_address(self.address())
    }

    /// Every text field of the settings, by the name the join code gives it.
    fn texts(&self) -> Vec<(&'static str, &str)> {
        match self {
            Self::OpenBao(s) => {
                let mut texts = vec![
                    ("address", s.address.as_str()),
                    ("mount", s.mount.as_str()),
                    ("path", s.path.as_str()),
                ];
                if let Some(mount) = &s.auth_mount {
                    texts.push(("auth_mount", mount));
                }
                if let Some(namespace) = &s.namespace {
                    texts.push(("namespace", namespace));
                }
                texts
            }
            Self::Infisical(s) => vec![
                ("site_url", s.site_url.as_str()),
                ("project_id", s.project_id.as_str()),
                ("environment", s.environment.as_str()),
                ("secret_path", s.secret_path.as_str()),
            ],
            Self::Sops(s) => vec![("location", s.location.as_str())],
        }
    }

    /// Checks the addresses and the fields a fetch needs. No field may hold a control or bidi
    /// character, which could make it read as something else where it is shown.
    pub fn check(&self) -> Result<()> {
        for (field, text) in self.texts() {
            check_plain(&format!("the source's {field}"), text)?;
        }
        match self {
            Self::OpenBao(s) => {
                check_address("address", &s.address, false)?;
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
                check_address("site_url", &s.site_url, false)?;
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
            Self::Sops(s) if s.location.contains("://") => {
                check_address("location", &s.location, true)
            }
            Self::Sops(s) => {
                if is_network_path(&s.location) {
                    return Err(refused(
                        "the file's location is a network path; use a synced folder or a \
                         mapped drive",
                    ));
                }
                if !Path::new(&s.location).is_absolute() {
                    return Err(refused(
                        "the source's location must be a full path or an https:// URL",
                    ));
                }
                Ok(())
            }
        }
    }
}

impl JoinCode {
    /// The code as the admin hands it out: base64url of the JSON, without padding. A plain code
    /// is written as version 1 so older Dots still read it; one with an invite is version 2.
    pub fn encode(&self) -> String {
        let versioned = Self {
            v: if self.invite.is_some() { 2 } else { 1 },
            ..self.clone()
        };
        let json = serde_json::to_string(&versioned).expect("a join code serializes");
        URL_SAFE_NO_PAD.encode(json)
    }
}

/// Reads a join code as pasted or read from a `.dotsorg` file. Whitespace anywhere is ignored,
/// so a code wrapped in an email still reads, and `=` padding is optional. Every field is
/// checked: an unknown version, a bad org id, a policy key that is not a minisign public key, a
/// source that is not `https://`, or a text field with a control or bidi character is refused.
pub fn parse_join_code(text: &str) -> Result<JoinCode> {
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = URL_SAFE_NO_PAD
        .decode(compact.trim_end_matches('='))
        .map_err(|_| refused("not base64url text"))?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| refused("no JSON inside"))?;
    match value.get("v").map(Value::as_u64) {
        Some(Some(v)) if v == 1 || v == u64::from(JOIN_CODE_VERSION) => {}
        Some(Some(v)) => {
            return Err(refused(format!(
                "version {v} is not supported; update Dots"
            )))
        }
        _ => return Err(refused("no version 'v'")),
    }
    let code: JoinCode = match serde_json::from_slice(&bytes) {
        Ok(code) => code,
        Err(e) => {
            // serde quotes an unknown variant or field: a hidden character in one is named by
            // its field instead, and never quoted.
            check_value_plain(&value, None)?;
            return Err(refused(plain_serde(&e)));
        }
    };
    if code.v == 1 && code.invite.is_some() {
        return Err(refused("a version 1 join code can't carry an invite"));
    }
    // First, so that no refusal below quotes a text that reads as something else.
    check_plain("the org's id", &code.org_id)?;
    check_plain("the org's name", &code.name)?;
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

/// Walks the JSON of a join code: every object key and every string, at any depth, must be free
/// of a hidden character. `key` is the nearest object key above `value` that is itself clean.
fn check_value_plain(value: &Value, key: Option<&str>) -> Result<()> {
    let hit = |key: Option<&str>| {
        refused(format!(
            "the field '{}' holds a control character",
            key.unwrap_or("a field name")
        ))
    };
    match value {
        Value::String(text) if text.chars().any(is_hidden_control) => Err(hit(key)),
        Value::Array(items) => items
            .iter()
            .try_for_each(|item| check_value_plain(item, key)),
        Value::Object(fields) => fields.iter().try_for_each(|(name, item)| {
            if name.chars().any(is_hidden_control) {
                return Err(hit(None));
            }
            check_value_plain(item, Some(name))
        }),
        _ => Ok(()),
    }
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

/// What the fingerprint's digest starts with, before the key's bytes.
const FINGERPRINT_CONTEXT: &[u8] = b"dots policy key v1\0";

/// The policy key's fingerprint, which the UI shows so the person can compare it with the one
/// their admin read from `dots-admin`: the first 128 bits of SHA-256 over
/// `"dots policy key v1\0"` and the key's 42 bytes, as 32 upper-case hex digits in groups of 4
/// (`1A2B 3C4D …`). It covers the whole key: the minisign key id in the `.pub` file's comment is
/// whatever the key's maker chose, so two keys can share one, and it proves nothing.
pub fn key_fingerprint(policy_key: &str) -> Result<String> {
    let digest = Sha256::new()
        .chain_update(FINGERPRINT_CONTEXT)
        .chain_update(policy_key_bytes(policy_key)?)
        .finalize();
    let hex: Vec<String> = digest[..16].iter().map(|b| format!("{b:02X}")).collect();
    Ok(hex
        .chunks(2)
        .map(|pair| pair.concat())
        .collect::<Vec<_>>()
        .join(" "))
}

/// Whether two policy keys are one key: their decoded bytes are compared. A text that is not a
/// key is the same only as the same text.
pub fn same_policy_key(a: &str, b: &str) -> bool {
    match (policy_key_bytes(a), policy_key_bytes(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// A minisign public key's 42 bytes: `Ed` (2 bytes), the key id (8 bytes) and the Ed25519 key
/// (32 bytes).
fn policy_key_bytes(policy_key: &str) -> Result<Vec<u8>> {
    let not_a_key = || Error::Invalid("the policy key is not a minisign public key".into());
    minisign_verify::PublicKey::from_base64(policy_key).map_err(|_| not_a_key())?;
    let bytes = STANDARD.decode(policy_key).map_err(|_| not_a_key())?;
    if bytes.len() != 42 || !bytes.starts_with(b"Ed") {
        return Err(not_a_key());
    }
    Ok(bytes)
}

/// An address or location as Dots shows it: a URL (a text with `://`) loses its user name and
/// password, its query and its fragment, which a signed URL or a pasted address may carry a
/// secret in; anything else, a local path, is shown as it is. The text is cut by hand rather
/// than parsed and printed again, so what is left reads exactly as the admin wrote it.
pub fn shown_address(address: &str) -> String {
    let Some((scheme, rest)) = address.split_once("://") else {
        return address.to_string();
    };
    let rest = match rest.find(['?', '#']) {
        Some(end) => &rest[..end],
        None => rest,
    };
    let (authority, path) = match rest.find(['/', '\\']) {
        Some(start) => rest.split_at(start),
        None => (rest, ""),
    };
    let host = match authority.rfind('@') {
        Some(at) => &authority[at + 1..],
        None => authority,
    };
    format!("{scheme}://{host}{path}")
}

/// A source address: `https://`, or `http://` only to `localhost` or `127.0.0.1`, for test
/// servers. It is written with `://`: `https:host` parses as a URL, but `shown_address` could not
/// cut it, and that refusal quotes nothing. It must name a host and hold no user name or
/// password, and, unless `allow_query` (a signed SOPS URL), no query or fragment. A refusal
/// quotes it as `shown_address` shows it. A text with an `@` that is not a URL with a host is not
/// quoted at all: where its user name and password would end can't be told.
fn check_address(field: &str, address: &str, allow_query: bool) -> Result<()> {
    let not_https = "must start with https:// (http:// only for localhost or 127.0.0.1)";
    let scheme_first = address.split_once("://").is_some_and(|(scheme, _)| {
        scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    });
    if !scheme_first {
        return Err(refused(format!("the source's {field} {not_https}")));
    }
    let parsed = reqwest::Url::parse(address);
    let cut = parsed.as_ref().is_ok_and(|url| url.has_host());
    let quoted = if address.contains('@') && !cut {
        String::new()
    } else {
        format!(" '{}'", shown_address(address))
    };
    let refuse = |why: &str| refused(format!("the source's {field}{quoted} {why}"));
    let url = parsed.map_err(|_| refuse("is not a URL"))?;
    let host = url.host_str().unwrap_or_default();
    if host.is_empty() {
        return Err(refuse("names no host"));
    }
    let local = matches!(host, "localhost" | "127.0.0.1");
    if !(url.scheme() == "https" || url.scheme() == "http" && local) {
        return Err(refuse(not_https));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(refuse("must not hold a user name or password"));
    }
    if !allow_query && address.contains(['?', '#']) {
        return Err(refuse("must not hold a query or a fragment"));
    }
    Ok(())
}

/// Whether a SOPS location is a network path: a UNC path (`\\host\share\…`, `//host/share/…`,
/// `\\?\UNC\…`), or any other path that starts with two separators but a local drive's
/// (`\\?\C:\…`). Opening one would hand this PC's Windows sign-in to whatever host it names.
fn is_network_path(location: &str) -> bool {
    let separator = |c: char| c == '\\' || c == '/';
    let mut chars = location.chars();
    if !(chars.next().is_some_and(separator) && chars.next().is_some_and(separator)) {
        return false;
    }
    let rest = chars.as_str();
    let local_drive = rest
        .strip_prefix(['?', '.'])
        .and_then(|after| after.strip_prefix(separator))
        .map(str::as_bytes)
        .is_some_and(|drive| {
            drive.len() >= 2 && drive[0].is_ascii_alphabetic() && drive[1] == b':'
        });
    !local_drive
}

/// Whether `c` could hide what a text says: a control character (a line break, an escape) or a
/// bidi control (the marks, the Arabic letter mark U+061C, embeddings, overrides and isolates),
/// or a line or paragraph separator (U+2028, U+2029), which can make a name or an address read
/// in another order or on another line than it is.
fn is_hidden_control(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{061C}'
                | '\u{200E}'
                | '\u{200F}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2066}'..='\u{2069}'
        )
}

/// Refuses a field that holds a control or bidi character. The refusal names the field, never
/// its text.
fn check_plain(field: &str, value: &str) -> Result<()> {
    if value.chars().any(is_hidden_control) {
        return Err(refused(format!("{field} holds a control character")));
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
