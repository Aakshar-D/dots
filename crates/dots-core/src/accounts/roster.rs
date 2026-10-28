//! An account's signed member list, the roster (members spec §6.1, §6.2): who belongs, their
//! Dots IDs, roles and workspaces. It is signed like the policy, by the root key or by an admin
//! the policy names, and it fails closed: an unknown field, an unknown role or a broken rule
//! makes the whole list invalid.
//!
//! `{"v":1,"roster":"<roster JSON text>","signature":"<the whole .minisig text>"}`

use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::DateTime;
use serde::{Deserialize, Serialize};

use crate::vault::org::signed::verify_with_any;
use crate::vault::org::UseRules;

/// The only roster version this Dots reads.
pub const ROSTER_VERSION: u32 = 1;
/// The only envelope version this Dots reads.
pub const ROSTER_ENVELOPE_VERSION: u32 = 1;

str_enum!(Role {
    Owner => "owner",
    Admin => "admin",
    Member => "member",
    Viewer => "viewer",
});

impl Role {
    /// The role as people read it.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Owner => "Owner",
            Self::Admin => "Admin",
            Self::Member => "Member",
            Self::Viewer => "Viewer",
        }
    }
}

// A member's role inside one workspace (members spec §6.3). Parsed and checked now; enforced
// with the workspaces work.
str_enum!(WorkspaceRole {
    Admin => "admin",
    Member => "member",
    Viewer => "viewer",
});

/// A workspace of the account. `use_rules`, when given, narrows which entries its dots may use.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    #[serde(rename = "use", default, skip_serializing_if = "Option::is_none")]
    pub use_rules: Option<UseRules>,
}

/// One person on the list. `keys` are their Dots IDs, one per PC.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Member {
    pub id: String,
    pub name: String,
    pub email: String,
    pub role: Role,
    pub keys: Vec<String>,
    pub workspaces: BTreeMap<String, WorkspaceRole>,
    pub added_at: String,
}

/// A roster that parsed and passed every check except `seq` and trust, which need the last
/// accepted roster and the policy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Roster {
    pub org_id: String,
    /// For display only: rosters are signed on different PCs whose clocks may differ.
    pub issued_at: String,
    pub seq: u64,
    pub workspaces: Vec<Workspace>,
    pub members: Vec<Member>,
}

/// Why a roster, or the signed text that carries it, was refused. The text is in plain words.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RosterError {
    /// The signed text is not the `{"v":1,"roster":…,"signature":…}` envelope.
    #[error("the signed member list is not readable: {0}")]
    Envelope(String),
    /// The signature is by neither the root key nor an admin the policy names, or it is not
    /// over this text.
    #[error("the member list is not signed by the owner or an admin")]
    Signature,
    /// The roster text itself: bad JSON, an unknown field, a value out of range, a broken rule.
    #[error("{0}")]
    Invalid(String),
    /// The roster names another org than the join code.
    #[error("the member list is for org '{found}', not '{expected}'")]
    WrongOrg { expected: String, found: String },
    /// The roster is older than the last one accepted for this org.
    #[error("this member list (#{found}) is older than the one already accepted (#{last})")]
    RolledBack { last: u64, found: u64 },
}

/// A roster whose signature and rules checked out, with the exact text that was signed (to keep
/// as the offline copy) and the key that signed it.
#[derive(Debug, Clone, PartialEq)]
pub struct Accepted {
    pub roster: Roster,
    pub text: String,
    pub signer: String,
}

const TOP_FIELDS: &[&str] = &["v", "org_id", "issued_at", "seq", "workspaces", "members"];

/// The top level as written, with `v` which `Roster` does not keep. Duplicate and unknown
/// fields are refused here and in the structs below it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRoster {
    #[serde(rename = "v")]
    _v: u64,
    org_id: String,
    issued_at: String,
    seq: u64,
    workspaces: Vec<Workspace>,
    members: Vec<Member>,
}

/// Parses and checks a roster's JSON text against the join code's `org_id`. The version and the
/// field names are checked before anything else, so a newer roster says it is newer and an
/// unknown field is named. `seq` and trust are not checked here.
pub fn parse_roster(text: &str, org_id: &str) -> Result<Roster, RosterError> {
    let value: serde_json::Value = serde_json::from_str(text)
        .map_err(|e| invalid(format!("not JSON: {}", plain_serde(&e))))?;
    let serde_json::Value::Object(fields) = &value else {
        return Err(invalid("the member list must be a JSON object"));
    };
    match fields.get("v") {
        None => return Err(invalid("missing field 'v'")),
        Some(v) if v.as_u64() == Some(u64::from(ROSTER_VERSION)) => {}
        Some(v) => {
            return Err(invalid(format!(
                "member list version {v} is not supported; this Dots reads version {ROSTER_VERSION}"
            )))
        }
    }
    if let Some(unknown) = fields.keys().find(|k| !TOP_FIELDS.contains(&k.as_str())) {
        return Err(unknown_field(unknown));
    }
    for (list, known) in [
        ("workspaces", &["id", "name", "use"][..]),
        (
            "members",
            &[
                "id",
                "name",
                "email",
                "role",
                "keys",
                "workspaces",
                "added_at",
            ][..],
        ),
    ] {
        let items = fields.get(list).and_then(|v| v.as_array());
        for item in items.into_iter().flatten() {
            let unknown = item
                .as_object()
                .and_then(|o| o.keys().find(|k| !known.contains(&k.as_str())));
            if let Some(unknown) = unknown {
                return Err(unknown_field(unknown));
            }
        }
    }
    // Parsed again from the text, not the `Value`, so a field given twice is refused.
    let raw: RawRoster = serde_json::from_str(text).map_err(|e| invalid(plain_serde(&e)))?;

    if raw.org_id != org_id {
        return Err(RosterError::WrongOrg {
            expected: org_id.to_string(),
            found: raw.org_id,
        });
    }
    if DateTime::parse_from_rfc3339(&raw.issued_at).is_err() {
        return Err(invalid(format!(
            "issued_at '{}' is not an RFC 3339 time, like 2026-10-21T09:00:00Z",
            raw.issued_at
        )));
    }
    let mut seen_workspaces = HashSet::new();
    for workspace in &raw.workspaces {
        if !is_workspace_id(&workspace.id) {
            return Err(invalid(format!(
                "workspace id '{}' must be lowercase letters, digits and dashes, up to 32",
                workspace.id
            )));
        }
        if !seen_workspaces.insert(workspace.id.as_str()) {
            return Err(invalid(format!(
                "workspace id '{}' is listed twice",
                workspace.id
            )));
        }
    }
    if !raw.workspaces.iter().any(|w| w.id == "default") {
        return Err(invalid("the member list needs a 'default' workspace"));
    }
    for member in &raw.members {
        for id in member.workspaces.keys() {
            if !raw.workspaces.iter().any(|w| &w.id == id) {
                return Err(invalid(format!(
                    "member {} names workspace '{id}', which isn't on the list",
                    member.email
                )));
            }
        }
    }
    let mut ids = HashSet::new();
    let mut emails = HashSet::new();
    // Which member (by position) holds each key.
    let mut holders: HashMap<&str, usize> = HashMap::new();
    for (at, member) in raw.members.iter().enumerate() {
        if !ids.insert(member.id.as_str()) {
            return Err(invalid(format!(
                "member id '{}' is listed twice",
                member.id
            )));
        }
        if !emails.insert(member.email.as_str()) {
            return Err(invalid(format!(
                "email {} is listed for two members",
                member.email
            )));
        }
        if member.keys.is_empty() {
            return Err(invalid(format!(
                "member {} needs at least one key",
                member.email
            )));
        }
        for key in &member.keys {
            if minisign_verify::PublicKey::from_base64(key).is_err() {
                return Err(invalid(format!(
                    "member {} has a key that isn't a Dots ID",
                    member.email
                )));
            }
            match holders.insert(key.as_str(), at) {
                Some(other) if other == at => {
                    return Err(invalid(format!(
                        "member {} lists a key twice",
                        member.email
                    )))
                }
                Some(_) => {
                    let short: String = key.chars().take(8).collect();
                    return Err(invalid(format!("key {short}… is listed for two members")));
                }
                None => {}
            }
        }
    }
    if !raw.members.iter().any(|m| m.role == Role::Owner) {
        return Err(invalid("the member list needs at least one Owner"));
    }

    Ok(Roster {
        org_id: raw.org_id,
        issued_at: raw.issued_at,
        seq: raw.seq,
        workspaces: raw.workspaces,
        members: raw.members,
    })
}

impl Roster {
    /// Refuses a roster that is not newer than the last accepted one (`orgs.last_roster_seq`).
    pub fn check_seq(&self, last: Option<u64>) -> Result<(), RosterError> {
        match last {
            Some(last) if self.seq <= last => Err(RosterError::RolledBack {
                last,
                found: self.seq,
            }),
            _ => Ok(()),
        }
    }

    /// The member who lists `key` (a Dots ID) for one of their PCs.
    pub fn member_with_key(&self, key: &str) -> Option<&Member> {
        self.members
            .iter()
            .find(|m| m.keys.iter().any(|k| k == key))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    v: u32,
    roster: String,
    signature: String,
}

#[derive(Serialize)]
struct EnvelopeRef<'a> {
    v: u32,
    roster: &'a str,
    signature: &'a str,
}

/// Wraps a roster text and its minisign signature box into the signed text. `roster_text`
/// goes in unchanged: the signature is over its exact bytes.
pub fn roster_envelope(roster_text: &str, signature_box: &str) -> String {
    serde_json::to_string(&EnvelopeRef {
        v: ROSTER_ENVELOPE_VERSION,
        roster: roster_text,
        signature: signature_box,
    })
    .expect("an envelope serializes")
}

/// Verifies the signed text fetched from an account's source, in this order: the envelope, the
/// signature (by `root_key` or one of `admins`, prehashed `ED` only), then the roster and its
/// org. The roster is not read until its signature checks out. Rollback is the caller's check,
/// `Roster::check_seq`, since it needs the `orgs` row.
pub fn verify_roster(
    signed: &str,
    root_key: &str,
    admins: &[String],
    org_id: &str,
) -> Result<Accepted, RosterError> {
    let envelope: Envelope =
        serde_json::from_str(signed).map_err(|e| RosterError::Envelope(plain_serde(&e)))?;
    if envelope.v != ROSTER_ENVELOPE_VERSION {
        return Err(RosterError::Envelope(format!(
            "version {} is not supported; this Dots reads version {ROSTER_ENVELOPE_VERSION}",
            envelope.v
        )));
    }
    minisign_verify::Signature::decode(&envelope.signature)
        .map_err(|_| RosterError::Envelope("the signature is not a minisign signature".into()))?;
    let keys: Vec<&str> = std::iter::once(root_key)
        .chain(admins.iter().map(String::as_str))
        .collect();
    let signer = verify_with_any(&envelope.roster, &envelope.signature, &keys)
        .map_err(|()| RosterError::Signature)?;

    let roster = parse_roster(&envelope.roster, org_id)?;
    Ok(Accepted {
        roster,
        text: envelope.roster,
        signer: keys[signer].to_string(),
    })
}

/// Reads the signed text of a list **already accepted** for `org_id`, without checking who
/// signed it: the envelope, then the roster and its org. It is for a list the caller knows by
/// its `roster_hash` (members spec §6.1: removing a key from `admins` later never invalidates a
/// list already accepted). Any other list goes through `verify_roster`.
pub fn open_accepted_roster(signed: &str, org_id: &str) -> Result<(Roster, String), RosterError> {
    let envelope: Envelope =
        serde_json::from_str(signed).map_err(|e| RosterError::Envelope(plain_serde(&e)))?;
    if envelope.v != ROSTER_ENVELOPE_VERSION {
        return Err(RosterError::Envelope(format!(
            "version {} is not supported; this Dots reads version {ROSTER_ENVELOPE_VERSION}",
            envelope.v
        )));
    }
    let roster = parse_roster(&envelope.roster, org_id)?;
    Ok((roster, envelope.roster))
}

/// The SHA-256 of a roster's text (the text that was signed), in lower-case hex: how the
/// `orgs` row knows the list it accepted last (`last_roster_hash`).
pub fn roster_hash(text: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A workspace id: `^[a-z0-9][a-z0-9-]{0,31}$`.
fn is_workspace_id(id: &str) -> bool {
    let ok = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit();
    let mut chars = id.chars();
    chars.next().is_some_and(ok) && id.len() <= 32 && chars.all(|c| ok(c) || c == '-')
}

fn unknown_field(name: &str) -> RosterError {
    invalid(format!("unknown field '{name}'"))
}

fn invalid(message: impl Into<String>) -> RosterError {
    RosterError::Invalid(message.into())
}

fn plain_serde(e: &serde_json::Error) -> String {
    crate::vault::org::policy::plain_serde(e)
}
