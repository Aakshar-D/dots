//! A SOPS file encrypted with age (env vault spec §6.1): the file at the join code's
//! `location`, a local path synced by git or IT, or an `https://` URL. Only SOPS's JSON format
//! is read. The file holds `{"env": {NAME: value, …}, "policy": "<signed policy text>"}` and may
//! hold `"roster": "<signed member list>"`. Other keys are ignored but covered by the MAC.
//!
//! This PC holds its own age identity for the org, and the admin adds its public key as a
//! recipient. Opening the file follows SOPS 3.13 exactly:
//!
//! 1. The JSON is read with every object's keys in the file's order, since the MAC hashes the
//!    values in that order. (`serde_json`'s `preserve_order` feature is off in this workspace,
//!    and turning it on would change the order of every map other code writes, so the file is
//!    read into a small tree of its own.)
//! 2. The `sops` metadata gives the age entries, `lastmodified`, the MAC and the rule for which
//!    values are encrypted. One key group at most; a key split with Shamir is refused.
//! 3. The 32-byte data key is unwrapped from an age entry with this PC's identity.
//! 4. Every leaf, in the file's order, is decrypted with AES-256-GCM and a 32-byte IV. Its AAD
//!    is its path of keys joined with `:`, plus a trailing `:`. An array adds nothing to the
//!    path. Each plain value is hashed into the MAC with SHA-512, unencrypted ones too unless
//!    the file says `mac_only_encrypted`.
//! 5. The MAC in the file, decrypted with `lastmodified` as its AAD, must match, compared in
//!    constant time. Nothing is read from the values before it does.
//! 6. Only values the file encrypted are taken. SOPS does not authenticate its metadata (the
//!    rule for which keys are encrypted), and a `mac_only_encrypted` MAC does not cover plain
//!    values, so a plain value in `env` or `policy` could have been added by anyone who can
//!    write the file. A non-empty one is refused. An empty text is skipped, since SOPS never
//!    encrypts one and the MAC gains nothing from it.
//!
//! A file that can't be read, or a server error, is `Unreachable`, since a synced file may not
//! have arrived yet. A key that is not a recipient, a value that does not decrypt and a MAC that
//! does not match are `Denied`: a failed MAC counts as revoked (§6.2). Anything that is not a
//! SOPS file Dots can open is `Invalid`. No message holds a value.

use std::collections::{BTreeMap, HashSet};
use std::io::Read;
use std::str::FromStr;
use std::sync::LazyLock;

use aes_gcm::aead::consts::U32;
use aes_gcm::aead::{AeadInPlace, KeyInit};
use aes_gcm::aes::Aes256;
use aes_gcm::{AesGcm, Key, Nonce, Tag};
use age::armor::ArmoredReader;
use age::secrecy::ExposeSecret;
use age::x25519;
use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use chrono::{DateTime, FixedOffset, Timelike};
use regex::{Regex, RegexBuilder};
use reqwest::{StatusCode, Url};
use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;
use sha2::{Digest, Sha512};
use subtle::ConstantTimeEq;
use tokio::io::AsyncReadExt;
use zeroize::Zeroizing;

use super::{
    collect_entries, read_json, request_failed, status_failed, Fetched, Field, Source, SourceCreds,
    SourceError, MAX_ANSWER_BYTES, ROSTER_ENTRY,
};
use crate::vault::org::SopsSettings;
use crate::{Error, Result};

/// The key that holds the file's metadata.
const METADATA: &str = "sops";

/// SOPS's default rule: a key ending with this, or under one that does, is not encrypted.
const DEFAULT_UNENCRYPTED_SUFFIX: &str = "_unencrypted";

/// The bytes a `mac_only_encrypted` MAC starts from: SHA-256 of `sops`, as SOPS sets it.
const MAC_ONLY_ENCRYPTED_INIT: [u8; 32] = [
    0x8a, 0x3f, 0xd2, 0xad, 0x54, 0xce, 0x66, 0x52, 0x7b, 0x10, 0x34, 0xf3, 0xd1, 0x47, 0xbe, 0x0b,
    0x0b, 0x97, 0x5b, 0x3b, 0xf4, 0x4f, 0x72, 0xc6, 0xfd, 0xad, 0xec, 0x81, 0x76, 0xf2, 0x7d, 0x69,
];

/// The six rules SOPS allows, at most one per file.
const RULES: [&str; 6] = [
    "unencrypted_suffix",
    "encrypted_suffix",
    "unencrypted_regex",
    "encrypted_regex",
    "unencrypted_comment_regex",
    "encrypted_comment_regex",
];

/// What `fetch` says when this PC's key opens none of the file's age entries. A join that gets
/// it waits for the admin to add the key, rather than failing for good.
pub const NOT_A_RECIPIENT: &str = "this PC's key is not a recipient of the file";

/// What `fetch` says when the MAC is missing, does not decrypt, or does not match.
const MAC_MISMATCH: &str = "the file's MAC does not match";

/// AES-256-GCM with the 32-byte IV SOPS uses.
type Aes256Gcm32 = AesGcm<Aes256, U32>;

/// An encrypted value: `ENC[AES256_GCM,data:…,iv:…,tag:…,type:…]`, as SOPS matches it.
static ENCRYPTED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^ENC\[AES256_GCM,data:(.+),iv:(.+),tag:(.+),type:(.+)\]").expect("the pattern")
});

/// Makes a new age identity for an org: the secret `AGE-SECRET-KEY-1…`, kept in the secret
/// store, and its public key `age1…`, which the person sends to the admin.
pub fn new_identity() -> (Zeroizing<String>, String) {
    let identity = x25519::Identity::generate();
    let secret = Zeroizing::new(identity.to_string().expose_secret().to_string());
    (secret, identity.to_public().to_string())
}

/// The public key `age1…` of an age identity.
pub fn public_key(identity: &str) -> Result<String> {
    let identity = parse_identity(identity).map_err(|_| Error::Invalid(bad_identity()))?;
    Ok(identity.to_public().to_string())
}

fn bad_identity() -> String {
    "this PC's age key is not an age identity (AGE-SECRET-KEY-1…)".to_string()
}

fn parse_identity(text: &str) -> std::result::Result<x25519::Identity, ()> {
    x25519::Identity::from_str(text.trim()).map_err(|_| ())
}

/// The SOPS source: reads the file, then opens it with this PC's age identity.
pub struct Sops {
    settings: SopsSettings,
    http: reqwest::Client,
}

impl Sops {
    pub fn new(settings: SopsSettings, http: reqwest::Client) -> Sops {
        Sops { settings, http }
    }

    /// Reads and parses the file, from disk or over HTTPS.
    async fn read(&self) -> std::result::Result<Node, SourceError> {
        let location = self.settings.location.as_str();
        if location.contains("://") {
            self.download(location).await
        } else {
            read_file(location).await
        }
    }

    /// GETs the file. A 404 means the URL is wrong, so it is `Invalid`; other statuses follow
    /// the shared rules.
    async fn download(&self, location: &str) -> std::result::Result<Node, SourceError> {
        // The location is not quoted: its query could carry an access token.
        let url = Url::parse(location)
            .map_err(|_| SourceError::Invalid("the source's location is not a URL".into()))?;
        let shown = shown_url(&url);
        let response = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|e| request_failed(&shown, e))?;
        match response.status() {
            s if s.is_success() => read_json(response, &shown, "the SOPS file").await,
            StatusCode::NOT_FOUND => Err(SourceError::Invalid(format!(
                "there is no file at {shown} (HTTP 404)"
            ))),
            s => Err(status_failed(&shown, "the SOPS file", s)),
        }
    }
}

#[async_trait]
impl Source for Sops {
    async fn fetch(&self, creds: &SourceCreds) -> std::result::Result<Fetched, SourceError> {
        let SourceCreds::Age { identity } = creds else {
            return Err(SourceError::Invalid(
                "this org's SOPS file is opened with this PC's age key, so it needs one".into(),
            ));
        };
        let identity =
            parse_identity(identity).map_err(|_| SourceError::Invalid(bad_identity()))?;
        let file = self.read().await?;
        open(&file, &identity)
    }
}

/// A URL as messages show it: no query or fragment, which could carry an access token.
fn shown_url(url: &Url) -> String {
    let mut shown = url.clone();
    shown.set_query(None);
    shown.set_fragment(None);
    shown.to_string()
}

/// Reads a local file of at most `MAX_ANSWER_BYTES`. A file that is missing or can't be read is
/// `Unreachable`: it may not be synced yet.
async fn read_file(path: &str) -> std::result::Result<Node, SourceError> {
    let unreadable =
        |e: std::io::Error| SourceError::Unreachable(format!("cannot read {path}: {e}"));
    let file = tokio::fs::File::open(path).await.map_err(unreadable)?;
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(MAX_ANSWER_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(unreadable)?;
    if bytes.len() > MAX_ANSWER_BYTES {
        return Err(SourceError::Invalid(format!("{path} is larger than 1 MiB")));
    }
    serde_json::from_slice(&bytes).map_err(|e| {
        SourceError::Invalid(format!(
            "{path} is not readable as JSON (line {}, column {})",
            e.line(),
            e.column()
        ))
    })
}

/// Opens a parsed SOPS file with `identity`: the entries and the policy, once the MAC matches.
fn open(file: &Node, identity: &x25519::Identity) -> std::result::Result<Fetched, SourceError> {
    let Node::Object(top) = file else {
        return Err(not_sops());
    };
    check_keys(file)?;
    let metadata = top
        .iter()
        .find(|(key, _)| key == METADATA)
        .ok_or_else(not_sops)?;
    let metadata = Metadata::read(&metadata.1)?;
    let data_key = data_key(&metadata.age, identity)?;
    let mut run = Decrypting::new(&data_key, &metadata);

    let mut env = Part::Missing;
    let mut policy = Part::Missing;
    let mut roster = Part::Missing;
    let mut path = Vec::new();
    for (key, node) in top {
        if key == METADATA {
            continue;
        }
        path.push(key.as_str());
        match (key.as_str(), node) {
            ("env", Node::Object(fields)) => {
                let mut values = Vec::with_capacity(fields.len());
                for (name, value) in fields {
                    path.push(name.as_str());
                    values.push((name.as_str(), run.walk(value, &mut path)?));
                    path.pop();
                }
                env = Part::Found(values);
            }
            ("env", other) => {
                run.walk(other, &mut path)?;
                env = Part::NotItsShape;
            }
            ("policy", other) => {
                policy = match run.walk(other, &mut path)? {
                    Some(plain) => Part::Found(plain),
                    None => Part::NotItsShape,
                }
            }
            ("roster", other) => {
                roster = match run.walk(other, &mut path)? {
                    Some(plain) => Part::Found(plain),
                    None => Part::NotItsShape,
                }
            }
            (_, other) => {
                run.walk(other, &mut path)?;
            }
        }
        path.pop();
    }
    run.check_mac(&metadata)?;

    // The MAC matches: only now is anything read from the values.
    let not_encrypted = |path: &[&str]| {
        SourceError::Invalid(format!(
            "the file's value at {} is not encrypted",
            shown_path(path)
        ))
    };
    let policy = match policy {
        Part::Missing => return Err(SourceError::Invalid("the file has no policy".into())),
        Part::NotItsShape => None,
        Part::Found(Plain::Unencrypted) => return Err(not_encrypted(&["policy"])),
        Part::Found(plain) => plain.into_text(),
    };
    let policy =
        policy.ok_or_else(|| SourceError::Invalid("the file's policy is not text".into()))?;
    // The roster is optional, but one that is there must be encrypted text like the policy.
    let roster = match roster {
        Part::Missing => None,
        Part::NotItsShape => return Err(roster_not_text()),
        Part::Found(Plain::Unencrypted) => return Err(not_encrypted(&["roster"])),
        Part::Found(plain) => Some(plain.into_text().ok_or_else(roster_not_text)?.to_string()),
    };
    let env = match env {
        Part::Missing => return Err(SourceError::Invalid("the file has no env object".into())),
        Part::NotItsShape => {
            return Err(SourceError::Invalid(
                "the file's env is not an object".into(),
            ))
        }
        Part::Found(env) => env,
    };
    let mut raw: BTreeMap<String, Field> = BTreeMap::new();
    for (name, plain) in env {
        let field = match plain {
            Some(Plain::Unencrypted) => return Err(not_encrypted(&["env", name])),
            Some(Plain::Empty) => {
                // SOPS never encrypts an empty text and the MAC gains nothing from it, so it
                // is no secret: skipped. Only the name's length is logged, as `collect_entries`
                // does: a value pasted into a name's place must not reach the logs.
                tracing::warn!(
                    "a SOPS file's env has an empty value under a name {} characters long, \
                     skipped",
                    name.chars().count()
                );
                continue;
            }
            other => match other.and_then(Plain::into_text) {
                Some(text) => Field::Text(text),
                None => Field::NotText,
            },
        };
        raw.insert(name.to_string(), field);
    }
    Ok(Fetched {
        entries: collect_entries(raw)?,
        policy: policy.to_string(),
        roster,
        roster_note: None,
    })
}

fn roster_not_text() -> SourceError {
    SourceError::Invalid(format!("{ROSTER_ENTRY} is not text"))
}

/// What the file's `env`, `policy` or `roster` held: nothing, a value of the wrong shape (`env`
/// must be an object, the others a leaf), or its values, not yet read.
enum Part<T> {
    Missing,
    NotItsShape,
    Found(T),
}

fn not_sops() -> SourceError {
    SourceError::Invalid("the file is not a SOPS file: it has no sops metadata".into())
}

fn damaged(why: &str) -> SourceError {
    SourceError::Invalid(format!("the file's sops metadata is damaged: {why}"))
}

/// Refuses an object with the same key twice anywhere in the file: which one counts would be
/// a guess.
fn check_keys(node: &Node) -> std::result::Result<(), SourceError> {
    match node {
        Node::Object(fields) => {
            let mut seen = HashSet::with_capacity(fields.len());
            for (key, value) in fields {
                if !seen.insert(key.as_str()) {
                    return Err(SourceError::Invalid(format!(
                        "the file has the key \"{}\" twice in one object",
                        key.escape_debug()
                    )));
                }
                check_keys(value)?;
            }
            Ok(())
        }
        Node::Array(items) => items.iter().try_for_each(check_keys),
        _ => Ok(()),
    }
}

/// What the `sops` object says, as far as an age-encrypted JSON file needs.
struct Metadata<'n> {
    /// `lastmodified` as SOPS formats it for the MAC's AAD.
    mac_aad: String,
    mac: Option<&'n str>,
    /// The armored `enc` of each age entry.
    age: Vec<&'n str>,
    rule: Rule,
    mac_only_encrypted: bool,
}

impl<'n> Metadata<'n> {
    fn read(node: &'n Node) -> std::result::Result<Metadata<'n>, SourceError> {
        let Node::Object(fields) = node else {
            return Err(not_sops());
        };
        match field(fields, "shamir_threshold") {
            None | Some(Node::Null) | Some(Node::Int(0 | 1)) => {}
            Some(Node::Int(n)) => {
                return Err(SourceError::Invalid(format!(
                    "the file splits its data key with shamir_threshold {n}, which is not \
                     supported"
                )))
            }
            Some(_) => return Err(damaged("shamir_threshold is not a number")),
        }
        let groups = match field(fields, "key_groups") {
            None | Some(Node::Null) => &[][..],
            Some(Node::Array(groups)) => groups.as_slice(),
            Some(_) => return Err(damaged("key_groups is not a list")),
        };
        let age = match groups {
            [] => field(fields, "age"),
            [Node::Object(group)] => field(group, "age"),
            [_] => return Err(damaged("its key group is not an object")),
            _ => {
                return Err(SourceError::Invalid(format!(
                    "the file has {} key groups, which is not supported",
                    groups.len()
                )))
            }
        };
        let last_modified =
            text(fields, "lastmodified")?.ok_or_else(|| damaged("it has no lastmodified"))?;
        let mac_only_encrypted = match field(fields, "mac_only_encrypted") {
            None | Some(Node::Null) => false,
            Some(Node::Bool(b)) => *b,
            Some(_) => return Err(damaged("mac_only_encrypted is not true or false")),
        };
        Ok(Metadata {
            mac_aad: mac_aad(last_modified)
                .ok_or_else(|| damaged("lastmodified is not an RFC 3339 time"))?,
            mac: text(fields, "mac")?,
            age: age_entries(age)?,
            rule: Rule::read(fields)?,
            mac_only_encrypted,
        })
    }
}

fn field<'n>(fields: &'n [(String, Node)], name: &str) -> Option<&'n Node> {
    fields
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value)
}

/// A text field of the metadata, if it is set. An empty one counts as not set, as in SOPS.
fn text<'n>(
    fields: &'n [(String, Node)],
    name: &str,
) -> std::result::Result<Option<&'n str>, SourceError> {
    match field(fields, name) {
        None | Some(Node::Null) => Ok(None),
        Some(Node::Str(s)) if s.is_empty() => Ok(None),
        Some(Node::Str(s)) => Ok(Some(s)),
        Some(_) => Err(damaged(&format!("{name} is not text"))),
    }
}

fn age_entries(list: Option<&Node>) -> std::result::Result<Vec<&str>, SourceError> {
    let entries = match list {
        None | Some(Node::Null) => return Ok(Vec::new()),
        Some(Node::Array(entries)) => entries,
        Some(_) => return Err(damaged("age is not a list")),
    };
    entries
        .iter()
        .map(|entry| match entry {
            Node::Object(fields) => {
                text(fields, "enc")?.ok_or_else(|| damaged("an age entry has no enc"))
            }
            _ => Err(damaged("an age entry is not an object")),
        })
        .collect()
}

/// `lastmodified` as SOPS writes it into the MAC's AAD: Go's RFC 3339, whole seconds, `Z` for
/// UTC. A file SOPS wrote holds exactly that already.
fn mac_aad(last_modified: &str) -> Option<String> {
    let time = DateTime::parse_from_rfc3339(last_modified).ok()?;
    go_rfc3339(&time, false)
}

/// A time as Go formats it: `RFC3339`, or with `nanos` `RFC3339Nano`, whose fraction has no
/// trailing zeros. A zero offset is `Z`. A leap second, which Go refuses, is `None`.
fn go_rfc3339(time: &DateTime<FixedOffset>, nanos: bool) -> Option<String> {
    if time.nanosecond() >= 1_000_000_000 {
        return None;
    }
    let mut text = time.format("%Y-%m-%dT%H:%M:%S").to_string();
    if nanos && time.nanosecond() != 0 {
        let fraction = format!("{:09}", time.nanosecond());
        text.push('.');
        text.push_str(fraction.trim_end_matches('0'));
    }
    let offset = time.offset().local_minus_utc();
    if offset == 0 {
        text.push('Z');
    } else {
        let sign = if offset < 0 { '-' } else { '+' };
        let minutes = offset.abs() / 60;
        text.push_str(&format!("{sign}{:02}:{:02}", minutes / 60, minutes % 60));
    }
    Some(text)
}

/// Which values the file encrypts. A rule is checked against every key on a value's path, its
/// parents' included.
enum Rule {
    UnencryptedSuffix(String),
    EncryptedSuffix(String),
    UnencryptedRegex(Regex),
    EncryptedRegex(Regex),
}

impl Rule {
    fn read(fields: &[(String, Node)]) -> std::result::Result<Rule, SourceError> {
        let mut set = Vec::new();
        for name in RULES {
            if let Some(value) = text(fields, name)? {
                set.push((name, value));
            }
        }
        let (name, value) = match set.as_slice() {
            [] => return Ok(Rule::UnencryptedSuffix(DEFAULT_UNENCRYPTED_SUFFIX.into())),
            [one] => *one,
            _ => {
                return Err(SourceError::Invalid(format!(
                    "the file's sops metadata sets more than one of {}",
                    RULES.join(", ")
                )))
            }
        };
        let regex = || {
            RegexBuilder::new(value)
                .size_limit(1 << 20)
                .build()
                .map_err(|_| damaged(&format!("{name} is not a regular expression")))
        };
        match name {
            "unencrypted_suffix" => Ok(Rule::UnencryptedSuffix(value.into())),
            "encrypted_suffix" => Ok(Rule::EncryptedSuffix(value.into())),
            "unencrypted_regex" => Ok(Rule::UnencryptedRegex(regex()?)),
            "encrypted_regex" => Ok(Rule::EncryptedRegex(regex()?)),
            _ => Err(SourceError::Invalid(format!(
                "the file's {name} is not supported: a JSON file has no comments"
            ))),
        }
    }

    fn encrypts(&self, path: &[&str]) -> bool {
        match self {
            Rule::UnencryptedSuffix(s) => !path.iter().any(|key| key.ends_with(s.as_str())),
            Rule::EncryptedSuffix(s) => path.iter().any(|key| key.ends_with(s.as_str())),
            Rule::UnencryptedRegex(r) => !path.iter().any(|key| r.is_match(key)),
            Rule::EncryptedRegex(r) => path.iter().any(|key| r.is_match(key)),
        }
    }
}

/// Unwraps the 32-byte data key from the first age entry this identity opens.
fn data_key(
    entries: &[&str],
    identity: &x25519::Identity,
) -> std::result::Result<Zeroizing<[u8; 32]>, SourceError> {
    let mut wrong_size = false;
    for enc in entries {
        let Ok(decryptor) = age::Decryptor::new_buffered(ArmoredReader::new(enc.as_bytes())) else {
            continue;
        };
        let Ok(reader) = decryptor.decrypt(std::iter::once(identity as &dyn age::Identity)) else {
            continue;
        };
        // One byte more than a key, to see a longer one, and no more: the buffer never grows.
        let mut plain = Zeroizing::new(Vec::with_capacity(33));
        if reader.take(33).read_to_end(&mut plain).is_err() {
            continue;
        }
        if plain.len() != 32 {
            wrong_size = true;
            continue;
        }
        let mut key = Zeroizing::new([0u8; 32]);
        key.copy_from_slice(&plain);
        return Ok(key);
    }
    if wrong_size {
        return Err(SourceError::Invalid(
            "the file's data key is not 32 bytes".into(),
        ));
    }
    Err(SourceError::Denied(NOT_A_RECIPIENT.into()))
}

/// The AAD of the value at `path`: its keys joined with `:`, plus a trailing `:`.
fn aad(path: &[&str]) -> String {
    let mut aad = path.join(":");
    aad.push(':');
    aad
}

/// A path as messages show it, `env:API_KEY`.
fn shown_path(path: &[&str]) -> String {
    path.join(":").escape_debug().to_string()
}

/// One plain leaf, kept only as far as the layout needs: encrypted text, encrypted something
/// else, an empty text (which SOPS never encrypts), or a non-empty value the file left in the
/// clear. The last is dropped at once: with `mac_only_encrypted` the MAC does not cover it, and
/// in any mode it is not a secret the admin encrypted for this org.
enum Plain {
    Text(Zeroizing<Vec<u8>>),
    Other,
    Empty,
    Unencrypted,
}

impl Plain {
    /// The text, if it is UTF-8 text.
    fn into_text(self) -> Option<Zeroizing<String>> {
        let Plain::Text(mut bytes) = self else {
            return None;
        };
        std::str::from_utf8(&bytes).ok()?;
        let text = String::from_utf8(std::mem::take(&mut *bytes)).expect("checked UTF-8");
        Some(Zeroizing::new(text))
    }
}

/// A value as SOPS types it, plain.
enum Value {
    Str(Zeroizing<Vec<u8>>),
    Int(i64),
    Float(f64),
    Bool(bool),
    /// The time as Go's `RFC3339Nano` text.
    Time(String),
}

/// The bytes SOPS hashes into the MAC for a value (Go's `ToBytes`): the text, the number in
/// decimal (a float as short as it round-trips, never with an exponent), or `True`/`False`.
fn mac_bytes(value: &Value) -> Zeroizing<Vec<u8>> {
    Zeroizing::new(match value {
        Value::Str(bytes) => bytes.to_vec(),
        Value::Int(n) => n.to_string().into_bytes(),
        Value::Float(f) => go_float(*f).into_bytes(),
        Value::Bool(true) => b"True".to_vec(),
        Value::Bool(false) => b"False".to_vec(),
        Value::Time(text) => text.clone().into_bytes(),
    })
}

/// A float as Go's `strconv.FormatFloat(f, 'f', -1, 64)` writes it. Rust's `Display` gives the
/// same shortest digits with no exponent.
fn go_float(f: f64) -> String {
    if f.is_nan() {
        "NaN".into()
    } else if f.is_infinite() {
        if f > 0.0 { "+Inf" } else { "-Inf" }.into()
    } else {
        f.to_string()
    }
}

/// Go's `strconv.ParseBool`.
fn go_bool(text: &str) -> Option<bool> {
    match text {
        "1" | "t" | "T" | "TRUE" | "true" | "True" => Some(true),
        "0" | "f" | "F" | "FALSE" | "false" | "False" => Some(false),
        _ => None,
    }
}

/// Why one encrypted value could not be used.
enum ValueError {
    /// It is not `ENC[AES256_GCM,…]` with base64 parts of the right sizes.
    NotSops,
    /// AES-GCM refused it: changed, moved or made with another key.
    DoesNotDecrypt,
    /// Its type is one Dots does not read, or no type at all.
    Unsupported,
    /// The plain text is not a value of its type.
    NotItsType,
}

/// Decrypts one `ENC[…]` value with `aad`.
fn decrypt_value(
    cipher: &Aes256Gcm32,
    text: &str,
    aad: &[u8],
) -> std::result::Result<Value, ValueError> {
    let parts = ENCRYPTED.captures(text).ok_or(ValueError::NotSops)?;
    let decode = |i: usize| STANDARD.decode(&parts[i]).map_err(|_| ValueError::NotSops);
    let mut plain = Zeroizing::new(decode(1)?);
    let iv = decode(2)?;
    let tag = decode(3)?;
    if iv.len() != 32 || tag.len() != 16 {
        return Err(ValueError::NotSops);
    }
    cipher
        .decrypt_in_place_detached(
            Nonce::<U32>::from_slice(&iv),
            aad,
            &mut plain,
            Tag::from_slice(&tag),
        )
        .map_err(|_| ValueError::DoesNotDecrypt)?;
    let as_text = || std::str::from_utf8(&plain).map_err(|_| ValueError::NotItsType);
    Ok(match &parts[4] {
        "str" => Value::Str(plain),
        "int" => Value::Int(as_text()?.parse().map_err(|_| ValueError::NotItsType)?),
        "float" => Value::Float(as_text()?.parse().map_err(|_| ValueError::NotItsType)?),
        "bool" => Value::Bool(go_bool(as_text()?).ok_or(ValueError::NotItsType)?),
        "time" => {
            let time =
                DateTime::parse_from_rfc3339(as_text()?).map_err(|_| ValueError::NotItsType)?;
            Value::Time(go_rfc3339(&time, true).ok_or(ValueError::NotItsType)?)
        }
        _ => return Err(ValueError::Unsupported),
    })
}

/// The walk over the file's values: decrypts each leaf and feeds the MAC.
struct Decrypting<'m> {
    cipher: Aes256Gcm32,
    metadata: &'m Metadata<'m>,
    hash: Sha512,
}

impl<'m> Decrypting<'m> {
    fn new(data_key: &[u8; 32], metadata: &'m Metadata<'m>) -> Decrypting<'m> {
        let mut hash = Sha512::new();
        if metadata.mac_only_encrypted {
            hash.update(MAC_ONLY_ENCRYPTED_INIT);
        }
        Decrypting {
            cipher: Aes256Gcm32::new(Key::<Aes256Gcm32>::from_slice(data_key)),
            metadata,
            hash,
        }
    }

    /// Walks `node` at `path`, in the file's order. A leaf gives its plain value; an object,
    /// an array or `null` gives `None`. `null` is not part of the MAC.
    fn walk<'n>(
        &mut self,
        node: &'n Node,
        path: &mut Vec<&'n str>,
    ) -> std::result::Result<Option<Plain>, SourceError> {
        match node {
            Node::Null => Ok(None),
            Node::Array(items) => {
                for item in items {
                    self.walk(item, path)?;
                }
                Ok(None)
            }
            Node::Object(fields) => {
                for (key, value) in fields {
                    path.push(key.as_str());
                    self.walk(value, path)?;
                    path.pop();
                }
                Ok(None)
            }
            leaf => self.leaf(leaf, path).map(Some),
        }
    }

    fn leaf(&mut self, node: &Node, path: &[&str]) -> std::result::Result<Plain, SourceError> {
        let encrypted = self.metadata.rule.encrypts(path);
        let value = match node {
            // SOPS leaves an empty string as it is, encrypted or not.
            Node::Str(s) if s.is_empty() => Value::Str(Zeroizing::new(Vec::new())),
            Node::Str(s) if encrypted => decrypt_value(&self.cipher, s, aad(path).as_bytes())
                .map_err(|e| {
                    let at = shown_path(path);
                    match e {
                        ValueError::NotSops => SourceError::Invalid(format!(
                            "the file's value at {at} is not a SOPS value"
                        )),
                        ValueError::DoesNotDecrypt => SourceError::Denied(format!(
                            "the file's value at {at} does not decrypt: the file was changed"
                        )),
                        ValueError::Unsupported => SourceError::Invalid(format!(
                            "the file's value at {at} has a type Dots does not read"
                        )),
                        ValueError::NotItsType => SourceError::Invalid(format!(
                            "the file's value at {at} does not match its type"
                        )),
                    }
                })?,
            _ if encrypted => {
                return Err(SourceError::Invalid(format!(
                    "the file's value at {} is not encrypted",
                    shown_path(path)
                )))
            }
            Node::Str(s) => Value::Str(Zeroizing::new(s.as_bytes().to_vec())),
            Node::Int(n) => Value::Int(*n),
            Node::Float(f) => Value::Float(*f),
            Node::Bool(b) => Value::Bool(*b),
            Node::Null | Node::Array(_) | Node::Object(_) => unreachable!("walk gives leaves"),
        };
        if encrypted || !self.metadata.mac_only_encrypted {
            self.hash.update(&*mac_bytes(&value));
        }
        Ok(match value {
            Value::Str(bytes) if bytes.is_empty() => Plain::Empty,
            _ if !encrypted => Plain::Unencrypted,
            Value::Str(bytes) => Plain::Text(bytes),
            _ => Plain::Other,
        })
    }

    /// Checks the walk's MAC against the file's, in constant time.
    fn check_mac(self, metadata: &Metadata) -> std::result::Result<(), SourceError> {
        let mismatch = || SourceError::Denied(MAC_MISMATCH.into());
        let digest = self.hash.finalize();
        let mut computed = Zeroizing::new(String::with_capacity(128));
        for byte in digest {
            computed.push_str(&format!("{byte:02X}"));
        }
        let mac = metadata.mac.ok_or_else(mismatch)?;
        let file_mac = match decrypt_value(&self.cipher, mac, metadata.mac_aad.as_bytes()) {
            Ok(Value::Str(bytes)) => bytes,
            Ok(_) => return Err(SourceError::Invalid("the file's MAC is not text".into())),
            Err(ValueError::NotSops) => {
                return Err(SourceError::Invalid(
                    "the file's MAC is not a SOPS value".into(),
                ))
            }
            Err(_) => return Err(mismatch()),
        };
        if bool::from(file_mac.as_slice().ct_eq(computed.as_bytes())) {
            Ok(())
        } else {
            Err(mismatch())
        }
    }
}

/// The file's JSON, keeping every object's keys in the file's order. A number is an `Int` when
/// it fits an `i64`, else a `Float`, as SOPS reads it.
#[derive(Debug, PartialEq)]
enum Node {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Array(Vec<Node>),
    Object(Vec<(String, Node)>),
}

impl<'de> Deserialize<'de> for Node {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Node, D::Error> {
        d.deserialize_any(NodeVisitor)
    }
}

struct NodeVisitor;

impl<'de> Visitor<'de> for NodeVisitor {
    type Value = Node;

    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("JSON")
    }

    fn visit_unit<E: de::Error>(self) -> std::result::Result<Node, E> {
        Ok(Node::Null)
    }

    fn visit_bool<E: de::Error>(self, v: bool) -> std::result::Result<Node, E> {
        Ok(Node::Bool(v))
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> std::result::Result<Node, E> {
        Ok(Node::Int(v))
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> std::result::Result<Node, E> {
        Ok(i64::try_from(v).map_or(Node::Float(v as f64), Node::Int))
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> std::result::Result<Node, E> {
        Ok(Node::Float(v))
    }

    fn visit_str<E: de::Error>(self, v: &str) -> std::result::Result<Node, E> {
        Ok(Node::Str(v.to_string()))
    }

    fn visit_string<E: de::Error>(self, v: String) -> std::result::Result<Node, E> {
        Ok(Node::Str(v))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<Node, A::Error> {
        let mut items = Vec::new();
        while let Some(item) = seq.next_element()? {
            items.push(item);
        }
        Ok(Node::Array(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<Node, A::Error> {
        let mut fields = Vec::new();
        while let Some(field) = map.next_entry()? {
            fields.push(field);
        }
        Ok(Node::Object(fields))
    }
}

#[cfg(test)]
mod tests {
    use aes_gcm::aead::AeadInPlace;

    use super::*;

    const KEY: [u8; 32] = [7; 32];

    fn cipher() -> Aes256Gcm32 {
        Aes256Gcm32::new(Key::<Aes256Gcm32>::from_slice(&KEY))
    }

    /// Encrypts `plain` as SOPS would, with a fixed IV.
    fn seal(plain: &str, kind: &str, aad: &str) -> String {
        let iv = [9u8; 32];
        let mut data = plain.as_bytes().to_vec();
        let tag = cipher()
            .encrypt_in_place_detached(Nonce::<U32>::from_slice(&iv), aad.as_bytes(), &mut data)
            .unwrap();
        format!(
            "ENC[AES256_GCM,data:{},iv:{},tag:{},type:{kind}]",
            STANDARD.encode(&data),
            STANDARD.encode(iv),
            STANDARD.encode(tag)
        )
    }

    fn metadata(rule: Rule) -> Metadata<'static> {
        Metadata {
            mac_aad: "2026-10-23T18:52:25Z".into(),
            mac: None,
            age: Vec::new(),
            rule,
            mac_only_encrypted: false,
        }
    }

    fn default_rule() -> Rule {
        Rule::UnencryptedSuffix(DEFAULT_UNENCRYPTED_SUFFIX.into())
    }

    fn walk(json: &str, rule: Rule) -> std::result::Result<(), SourceError> {
        let node: Node = serde_json::from_str(json).unwrap();
        let metadata = metadata(rule);
        let mut run = Decrypting::new(&KEY, &metadata);
        run.walk(&node, &mut Vec::new()).map(|_| ())
    }

    #[test]
    fn the_aad_is_the_path_joined_with_colons_and_a_trailing_colon() {
        assert_eq!(aad(&["env", "API_KEY"]), "env:API_KEY:");
        assert_eq!(aad(&["foo"]), "foo:");
    }

    #[test]
    fn a_nested_leaf_and_an_array_leaf_decrypt_with_their_aad() {
        let json = format!(
            r#"{{"a": {{"b": {{"c": "{}"}}, "list": ["{}", {{"d": "{}"}}, ["{}"]]}}}}"#,
            seal("x", "str", "a:b:c:"),
            seal("y", "str", "a:list:"),
            seal("z", "str", "a:list:d:"),
            seal("w", "str", "a:list:"),
        );
        assert!(walk(&json, default_rule()).is_ok());
    }

    #[test]
    fn an_array_index_in_the_aad_does_not_decrypt() {
        let json = format!(r#"{{"list": ["{}"]}}"#, seal("y", "str", "list:0:"));
        let e = walk(&json, default_rule()).unwrap_err();
        assert_eq!(
            e,
            SourceError::Denied(
                "the file's value at list does not decrypt: the file was changed".into()
            )
        );
    }

    #[test]
    fn a_value_moved_to_another_key_does_not_decrypt() {
        let json = format!(r#"{{"a": "{}"}}"#, seal("x", "str", "b:"));
        assert!(matches!(
            walk(&json, default_rule()),
            Err(SourceError::Denied(_))
        ));
    }

    #[test]
    fn the_mac_bytes_follow_go() {
        let bytes = |v: Value| String::from_utf8(mac_bytes(&v).to_vec()).unwrap();
        assert_eq!(bytes(Value::Bool(true)), "True");
        assert_eq!(bytes(Value::Bool(false)), "False");
        assert_eq!(bytes(Value::Int(7)), "7");
        assert_eq!(bytes(Value::Int(-42)), "-42");
        assert_eq!(bytes(Value::Int(i64::MIN)), "-9223372036854775808");
        assert_eq!(bytes(Value::Float(1.5)), "1.5");
        assert_eq!(bytes(Value::Float(0.25)), "0.25");
        assert_eq!(bytes(Value::Float(1.0)), "1");
        assert_eq!(bytes(Value::Float(-0.0)), "-0");
        assert_eq!(bytes(Value::Float(0.1 + 0.2)), "0.30000000000000004");
        assert_eq!(bytes(Value::Float(1e21)), "1000000000000000000000");
        assert_eq!(bytes(Value::Float(1e-7)), "0.0000001");
        assert_eq!(bytes(Value::Float(f64::INFINITY)), "+Inf");
        assert_eq!(bytes(Value::Float(f64::NAN)), "NaN");
        let text = Value::Str(Zeroizing::new(b"plain".to_vec()));
        assert_eq!(bytes(text), "plain");
    }

    #[test]
    fn decrypted_values_are_typed_as_sops_types_them() {
        let open =
            |plain: &str, kind: &str| decrypt_value(&cipher(), &seal(plain, kind, "k:"), b"k:");
        assert!(matches!(open("12", "int"), Ok(Value::Int(12))));
        assert!(matches!(open("1.5", "float"), Ok(Value::Float(f)) if f == 1.5));
        assert!(matches!(open("True", "bool"), Ok(Value::Bool(true))));
        assert!(matches!(open("false", "bool"), Ok(Value::Bool(false))));
        assert!(matches!(open("x", "int"), Err(ValueError::NotItsType)));
        assert!(matches!(open("yes", "bool"), Err(ValueError::NotItsType)));
        assert!(matches!(open("x", "bytes"), Err(ValueError::Unsupported)));
        assert!(matches!(open("x", "comment"), Err(ValueError::Unsupported)));
        assert!(matches!(open("x", "other"), Err(ValueError::Unsupported)));
        match open("2026-10-23T20:52:25.500+02:00", "time") {
            Ok(Value::Time(t)) => assert_eq!(t, "2026-10-23T20:52:25.5+02:00"),
            _ => panic!("a time"),
        }
    }

    #[test]
    fn a_value_that_is_not_sops_syntax_is_refused_as_such() {
        let c = cipher();
        for text in [
            "plain",
            "ENC[AES256_GCM,data:abc",
            "ENC[AES256_GCM,data:@@@,iv:AAAA,tag:AAAA,type:str]",
            // An IV of 12 bytes, not 32.
            "ENC[AES256_GCM,data:AAAA,iv:AAAAAAAAAAAAAAAA,tag:AAAAAAAAAAAAAAAAAAAAAA==,type:str]",
        ] {
            assert!(
                matches!(decrypt_value(&c, text, b"k:"), Err(ValueError::NotSops)),
                "{text}"
            );
        }
    }

    #[test]
    fn go_parses_bools_this_way() {
        for t in ["1", "t", "T", "TRUE", "true", "True"] {
            assert_eq!(go_bool(t), Some(true), "{t}");
        }
        for f in ["0", "f", "F", "FALSE", "false", "False"] {
            assert_eq!(go_bool(f), Some(false), "{f}");
        }
        assert_eq!(go_bool("yes"), None);
        assert_eq!(go_bool("tRUE"), None);
    }

    #[test]
    fn last_modified_is_formatted_as_go_does_for_the_mac() {
        assert_eq!(
            mac_aad("2026-10-23T18:52:25Z").unwrap(),
            "2026-10-23T18:52:25Z"
        );
        assert_eq!(
            mac_aad("2026-10-23T18:52:25+00:00").unwrap(),
            "2026-10-23T18:52:25Z"
        );
        assert_eq!(
            mac_aad("2026-10-23T18:52:25.75Z").unwrap(),
            "2026-10-23T18:52:25Z"
        );
        assert_eq!(
            mac_aad("2026-10-23T14:52:25-04:00").unwrap(),
            "2026-10-23T14:52:25-04:00"
        );
        assert_eq!(mac_aad("yesterday"), None);
    }

    #[test]
    fn the_rules_check_every_key_on_the_path() {
        let path = ["db_unencrypted", "password"];
        assert!(!default_rule().encrypts(&path));
        assert!(default_rule().encrypts(&["db", "password"]));
        let suffix = Rule::EncryptedSuffix("_secret".into());
        assert!(suffix.encrypts(&["db_secret", "password"]));
        assert!(!suffix.encrypts(&["db", "password"]));
        let only = Rule::EncryptedRegex(Regex::new("^(env|policy)$").unwrap());
        assert!(only.encrypts(&["env", "API_KEY"]));
        assert!(!only.encrypts(&["extra", "env_x"]));
        let not = Rule::UnencryptedRegex(Regex::new("^public").unwrap());
        assert!(!not.encrypts(&["public_notes", "x"]));
        assert!(not.encrypts(&["notes", "x"]));
    }

    #[test]
    fn the_mac_only_encrypted_start_is_sha256_of_sops() {
        use sha2::Sha256;
        assert_eq!(Sha256::digest(b"sops").as_slice(), MAC_ONLY_ENCRYPTED_INIT);
    }

    #[test]
    fn the_tree_keeps_the_files_order_and_number_kinds() {
        let node: Node =
            serde_json::from_str(r#"{"b": 1, "a": [1.5, 18446744073709551615, true, null]}"#)
                .unwrap();
        assert_eq!(
            node,
            Node::Object(vec![
                ("b".into(), Node::Int(1)),
                (
                    "a".into(),
                    Node::Array(vec![
                        Node::Float(1.5),
                        Node::Float(18446744073709551615u64 as f64),
                        Node::Bool(true),
                        Node::Null,
                    ])
                ),
            ])
        );
    }
}
