//! Where an org's entries and signed policy are read from (env vault spec §6.1 and §6.2): one
//! adapter per kind of source, behind the `Source` trait, and the person's own sign-in details
//! for it, `SourceCreds`, kept in the secret store.
//!
//! Every adapter follows the same rules. A mount or path that is empty or holds a `.` or `..`
//! segment is refused before any request, and each segment is percent-encoded. A timeout, a
//! connection error, rate limiting (429) or a server error (any 5xx) is `Unreachable`; 401 and
//! 403 are `Denied`; any other status, or an answer that doesn't parse, is `Invalid`, with the
//! status. No message ever quotes an answer's body, since it could hold a value.

pub mod openbao;

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use async_trait::async_trait;
use reqwest::{StatusCode, Url};
use serde::de::{self, DeserializeOwned, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

pub use openbao::OpenBao;

use super::{check_org_id, SourceSettings};
use crate::vault::entries::check_variable_name;
use crate::vault::secrets::{SecretStore, MAX_SECRET_BYTES};
use crate::{Error, Result};

/// The key the signed policy is kept under, beside the entries (spec §6.3). It is never an entry.
pub const POLICY_ENTRY: &str = "DOTS_POLICY";

/// How long one request to a source may take, from connecting to the last byte of its answer.
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(20);

/// The largest answer read from a source. Entries and a policy are far smaller.
const MAX_ANSWER_BYTES: usize = 1024 * 1024;

/// One kind of source: OpenBao or Vault, Infisical, or a SOPS file.
#[async_trait]
pub trait Source: Send + Sync {
    /// Logs in if needed and reads every entry plus the signed policy text.
    async fn fetch(&self, creds: &SourceCreds) -> std::result::Result<Fetched, SourceError>;
}

/// What a fetch read: the entries by name, and the signed policy text, not yet verified. Its
/// `Debug` shows only the names.
#[derive(Clone, PartialEq, Eq)]
pub struct Fetched {
    pub entries: BTreeMap<String, Zeroizing<String>>,
    pub policy: String,
}

impl std::fmt::Debug for Fetched {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Fetched")
            .field("names", &self.entries.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

/// Why a fetch failed, which decides what happens to the org's copy (spec §6.2). The messages
/// name the source, the path and the status, never a value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SourceError {
    /// The source could not be reached: offline, the copy is kept until it expires.
    #[error("{0}")]
    Unreachable(String),
    /// The source refused these sign-in details: access revoked, the copy is wiped.
    #[error("{0}")]
    Denied(String),
    /// The source answered with something Dots cannot use: the last good copy is kept.
    #[error("{0}")]
    Invalid(String),
}

/// The person's own sign-in details for an org's source. They are kept as JSON in the secret
/// store under `org/<org_id>/source`, and never shown: `Debug` prints the kind only.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum SourceCreds {
    /// An OpenBao or Vault token.
    Token { token: Zeroizing<String> },
    /// A username and password for an OpenBao or Vault `userpass` or `ldap` mount.
    Login {
        username: Zeroizing<String>,
        password: Zeroizing<String>,
    },
    /// An Infisical Universal Auth machine identity.
    Machine {
        client_id: Zeroizing<String>,
        client_secret: Zeroizing<String>,
    },
    /// This PC's age identity for a SOPS file, `AGE-SECRET-KEY-1…`.
    Age { identity: Zeroizing<String> },
}

impl std::fmt::Debug for SourceCreds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("SourceCreds").field(&self.kind()).finish()
    }
}

impl SourceCreds {
    /// `token`, `login`, `machine` or `age`, as the JSON's `kind` says.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Token { .. } => "token",
            Self::Login { .. } => "login",
            Self::Machine { .. } => "machine",
            Self::Age { .. } => "age",
        }
    }

    /// The secret store key of an org's sign-in details: `org/<org_id>/source`.
    pub fn store_key(org_id: &str) -> String {
        format!("org/{org_id}/source")
    }

    /// Saves these details as the org's, replacing any there.
    pub fn save(&self, secrets: &dyn SecretStore, org_id: &str) -> Result<()> {
        check_org_id(org_id)?;
        let json = self.to_json()?;
        secrets.put(&Self::store_key(org_id), &json)
    }

    /// The org's saved details, if any.
    pub fn load(secrets: &dyn SecretStore, org_id: &str) -> Result<Option<SourceCreds>> {
        check_org_id(org_id)?;
        let Some(json) = secrets.get(&Self::store_key(org_id))? else {
            return Ok(None);
        };
        // serde's messages can quote the text they fail on, and that text is a secret.
        let creds = serde_json::from_str(&json).map_err(|_| {
            Error::Other(format!(
                "org '{org_id}': its saved sign-in details are damaged; give them again"
            ))
        })?;
        Ok(Some(creds))
    }

    /// Deletes the org's saved details. An org with none is not an error.
    pub fn forget(secrets: &dyn SecretStore, org_id: &str) -> Result<()> {
        check_org_id(org_id)?;
        secrets.delete(&Self::store_key(org_id))
    }

    /// The JSON, written into a buffer that never grows, so no copy of a value is left behind
    /// in memory it gave back. Longer than the secret store holds is refused.
    fn to_json(&self) -> Result<Zeroizing<String>> {
        let mut bytes = Zeroizing::new(Vec::with_capacity(MAX_SECRET_BYTES));
        serde_json::to_writer(Bounded(&mut bytes, MAX_SECRET_BYTES), self).map_err(|_| {
            Error::Invalid(format!(
                "these sign-in details are too long to keep: at most {MAX_SECRET_BYTES} bytes \
                 in all"
            ))
        })?;
        let text = String::from_utf8(std::mem::take(&mut *bytes)).expect("serde_json writes UTF-8");
        Ok(Zeroizing::new(text))
    }
}

/// A writer into a `Vec` that refuses to go past a length, set no larger than its capacity.
struct Bounded<'a>(&'a mut Vec<u8>, usize);

impl std::io::Write for Bounded<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if buf.len() > self.1 - self.0.len() {
            return Err(std::io::Error::other("too long"));
        }
        self.0.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The adapter for `settings`, sharing `http`.
pub fn source_for(settings: &SourceSettings, http: reqwest::Client) -> Box<dyn Source> {
    match settings {
        SourceSettings::OpenBao(s) => Box::new(OpenBao::new(s.clone(), http)),
        SourceSettings::Infisical(_) => Box::new(NotBuilt("reading from Infisical")),
        SourceSettings::Sops(_) => Box::new(NotBuilt("reading a SOPS file")),
    }
}

/// A kind of source whose adapter is not built yet.
struct NotBuilt(&'static str);

#[async_trait]
impl Source for NotBuilt {
    async fn fetch(&self, _creds: &SourceCreds) -> std::result::Result<Fetched, SourceError> {
        Err(SourceError::Invalid(format!("{} is not built yet", self.0)))
    }
}

/// The client every source shares: a 20-second limit per request, rustls, and no redirects. A
/// redirect would carry the token header to wherever it points, so a 3xx is an error. TLS trusts
/// the public roots and the Windows certificate store, where a company keeps its own CA.
pub fn http_client() -> reqwest::Client {
    http_client_with_timeout(FETCH_TIMEOUT)
}

/// `http_client` with another limit per request, for tests.
pub fn http_client_with_timeout(timeout: Duration) -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("an http client")
}

/// The segments of a mount or path setting: `team/dots` is `["team", "dots"]`. A slash at
/// either end, or a doubled one, is ignored. An empty value, a `.` or `..` segment (also written
/// with `%2e`, which a URL reads as a dot), or a control character is refused, so the URL built
/// from it stays where the setting points.
fn path_segments<'a>(
    field: &str,
    value: &'a str,
) -> std::result::Result<Vec<&'a str>, SourceError> {
    let refuse = |why: &str| {
        SourceError::Invalid(format!(
            "the source's {field} '{}' {why}",
            value.escape_debug()
        ))
    };
    if value.chars().any(char::is_control) {
        return Err(refuse("holds a control character"));
    }
    let segments: Vec<&str> = value.split('/').filter(|s| !s.is_empty()).collect();
    if segments.is_empty() {
        return Err(refuse("is empty"));
    }
    if segments.iter().any(|s| is_dot_segment(s)) {
        return Err(refuse("must not hold a '.' or '..' segment"));
    }
    Ok(segments)
}

/// Whether a URL reads `segment` as `.` or `..`.
fn is_dot_segment(segment: &str) -> bool {
    let dots = segment.to_ascii_lowercase().replace("%2e", ".");
    dots == "." || dots == ".."
}

/// `address` with `segments` added to its path, each percent-encoded. A query or fragment on the
/// address is dropped.
fn endpoint<'a>(
    address: &str,
    segments: impl IntoIterator<Item = &'a str>,
) -> std::result::Result<Url, SourceError> {
    let not_a_url =
        || SourceError::Invalid(format!("the source's address '{address}' is not a URL"));
    let mut url = Url::parse(address).map_err(|_| not_a_url())?;
    url.set_query(None);
    url.set_fragment(None);
    url.path_segments_mut()
        .map_err(|_| not_a_url())?
        .pop_if_empty()
        .extend(segments);
    Ok(url)
}

/// What a request that got no answer means: a timeout, a refused connection or a connection
/// that broke is `Unreachable`; a request that could not be made is `Invalid`.
fn request_failed(address: &str, e: reqwest::Error) -> SourceError {
    if e.is_builder() || e.is_redirect() || e.is_decode() {
        return SourceError::Invalid(format!("a request to {address} could not be made"));
    }
    if e.is_timeout() {
        return SourceError::Unreachable(format!("{address} did not answer in time"));
    }
    SourceError::Unreachable(format!("cannot reach {address}: {}", describe(e)))
}

/// The error and its causes, without the URL.
fn describe(e: reqwest::Error) -> String {
    let e = e.without_url();
    let mut text = e.to_string();
    let mut source = std::error::Error::source(&e);
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

/// What a status other than success means for `what`. A source that is rate limiting Dots, or
/// any server error (5xx), from the source or a gateway in front of it, is `Unreachable`: it is
/// trouble on the server's side, not in the org's data, so the next refresh tries again and the
/// copy is kept meanwhile.
fn status_failed(address: &str, what: &str, status: StatusCode) -> SourceError {
    let code = status.as_u16();
    match code {
        401 | 403 => {
            SourceError::Denied(format!("{address} refused access to {what} (HTTP {code})"))
        }
        429 => SourceError::Unreachable(format!("{address} is rate limiting Dots (HTTP 429)")),
        500..=599 => SourceError::Unreachable(format!("{address} is not available (HTTP {code})")),
        _ => SourceError::Invalid(format!("{address} answered HTTP {code} for {what}")),
    }
}

/// Reads an answer's body as JSON, at most `MAX_ANSWER_BYTES`. A body that doesn't parse is
/// `Invalid`, with only the line and column: serde's message could quote a value.
async fn read_json<T: DeserializeOwned>(
    mut response: reqwest::Response,
    address: &str,
    what: &str,
) -> std::result::Result<T, SourceError> {
    let too_large =
        || SourceError::Invalid(format!("the answer from {address} for {what} is too large"));
    let expected = response.content_length().unwrap_or(0);
    if expected > MAX_ANSWER_BYTES as u64 {
        return Err(too_large());
    }
    let mut body = Zeroizing::new(Vec::with_capacity(expected as usize));
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| request_failed(address, e))?
    {
        if chunk.len() > MAX_ANSWER_BYTES - body.len() {
            return Err(too_large());
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|e| {
        SourceError::Invalid(format!(
            "the answer from {address} for {what} is not readable (line {}, column {})",
            e.line(),
            e.column()
        ))
    })
}

/// One value of a source's map of entries: text, or anything else, which is refused by name.
/// Text goes straight into a `Zeroizing<String>`.
enum Field {
    Text(Zeroizing<String>),
    NotText,
}

impl<'de> Deserialize<'de> for Field {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Field, D::Error> {
        d.deserialize_any(FieldVisitor)
    }
}

struct FieldVisitor;

impl<'de> Visitor<'de> for FieldVisitor {
    type Value = Field;

    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("a value")
    }

    fn visit_str<E: de::Error>(self, v: &str) -> std::result::Result<Field, E> {
        Ok(Field::Text(Zeroizing::new(v.to_string())))
    }

    fn visit_string<E: de::Error>(self, v: String) -> std::result::Result<Field, E> {
        Ok(Field::Text(Zeroizing::new(v)))
    }

    fn visit_bool<E: de::Error>(self, _: bool) -> std::result::Result<Field, E> {
        Ok(Field::NotText)
    }

    fn visit_i64<E: de::Error>(self, _: i64) -> std::result::Result<Field, E> {
        Ok(Field::NotText)
    }

    fn visit_u64<E: de::Error>(self, _: u64) -> std::result::Result<Field, E> {
        Ok(Field::NotText)
    }

    fn visit_f64<E: de::Error>(self, _: f64) -> std::result::Result<Field, E> {
        Ok(Field::NotText)
    }

    fn visit_unit<E: de::Error>(self) -> std::result::Result<Field, E> {
        Ok(Field::NotText)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<Field, A::Error> {
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(Field::NotText)
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<Field, A::Error> {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(Field::NotText)
    }
}

/// The entries of a source's map. `DOTS_POLICY` is never an entry, and a name that is not a
/// variable name is skipped with a warning. A value that is not text is refused, and so are two
/// names that differ only in case, since names compare without case.
fn collect_entries(
    raw: BTreeMap<String, Field>,
) -> std::result::Result<BTreeMap<String, Zeroizing<String>>, SourceError> {
    let mut entries = BTreeMap::new();
    let mut seen: HashMap<String, String> = HashMap::new();
    for (name, field) in raw {
        if name.eq_ignore_ascii_case(POLICY_ENTRY) {
            continue;
        }
        if check_variable_name(&name).is_err() {
            tracing::warn!("an org entry {name:?} is skipped: it is not a variable name");
            continue;
        }
        let Field::Text(value) = field else {
            return Err(SourceError::Invalid(format!("{name} is not text")));
        };
        if let Some(other) = seen.insert(name.to_ascii_uppercase(), name.clone()) {
            return Err(SourceError::Invalid(format!(
                "{other} and {name} are the same name in a different case"
            )));
        }
        entries.insert(name, value);
    }
    Ok(entries)
}
