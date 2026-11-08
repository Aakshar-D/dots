//! One audit record and its line (`docs/vault-audit-format.md`). A line is the record as
//! canonical JSON: the keys of every object in byte order, no white space, every key present
//! (`null` when it says nothing). `prev` is `line_hash` of the record before it, so the chain
//! is checked on the lines exactly as they were sent.

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The `v` of every record this version writes.
pub const RECORD_VERSION: u64 = 1;

/// `reason` of a `revoked` record: the source refused this PC's sign-in details.
pub const REASON_DENIED: &str = "denied";
/// `reason` of a `revoked` record: this PC was taken off the account's member list.
pub const REASON_REMOVED: &str = "removed";
/// `reason` of an `expired` record: the org's offline copy ran out.
pub const REASON_OFFLINE_COPY: &str = "offline_copy";
/// `reason` of an `expired` record: the queue was full, and the records `dropped` names were
/// deleted unsent.
pub const REASON_QUEUE_FULL: &str = "queue_full";

str_enum!(Event {
    Resolved => "resolved",
    Denied => "denied",
    Fetched => "fetched",
    Offline => "offline",
    Revoked => "revoked",
    Expired => "expired",
    PolicyInvalid => "policy_invalid",
});

/// Who used the org on this install. None of it is a secret.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditUser {
    /// The Windows account, `DOMAIN\user`.
    pub windows: Option<String>,
    /// The PC's name.
    pub machine: Option<String>,
    /// Who the source knows this PC as: an OpenBao login's user name, an Infisical client id
    /// or this PC's SOPS age public key; `None` for an OpenBao token.
    pub source_identity: Option<String>,
    /// This install's Dots ID, as its fingerprint.
    pub dots_id: Option<String>,
}

/// The dot a record is about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditDot {
    pub id: String,
    pub name: String,
}

/// The records a full queue deleted unsent, from `from` to `to`, both included.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dropped {
    pub from: u64,
    pub to: u64,
}

/// One audit record. It never holds a value: only names, ids and what happened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditRecord {
    pub v: u64,
    /// A ULID.
    pub id: String,
    /// 1 for the org's first record on this install, then one more for each.
    pub seq: u64,
    /// `line_hash` of the record before; `None` for seq 1.
    pub prev: Option<String>,
    /// RFC 3339, to the second, in UTC.
    pub at: String,
    pub org_id: String,
    /// This install's id.
    pub install: String,
    pub user: AuditUser,
    /// The dot that resolved; `None` for a fetch or a Library check.
    pub dot: Option<AuditDot>,
    pub run_id: Option<String>,
    pub event: Event,
    /// The variable; `None` for a fetch outcome.
    pub name: Option<String>,
    /// `mcp:<server>` or `shell`; `None` for a fetch outcome.
    pub consumer: Option<String>,
    /// Where the value came from (`openbao`, `infisical`, `sops` or `personal`), or the org's
    /// source for any other event.
    pub source: Option<String>,
    /// Why, as a code: a `denied` record's reason, or one of the `REASON_` codes.
    pub reason: Option<String>,
    /// What an `expired` record of a full queue dropped.
    pub dropped: Option<Dropped>,
}

impl AuditRecord {
    /// The record's line: its canonical JSON, without a line end.
    pub fn line(&self) -> String {
        canonical(&serde_json::to_value(self).expect("a record always serializes"))
    }

    /// Reads one line. A field this version does not know, or a `v` other than 1, is refused.
    pub fn parse(line: &str) -> Result<AuditRecord, String> {
        let record: AuditRecord =
            serde_json::from_str(line).map_err(|e| format!("not a record: {e}"))?;
        if record.v != RECORD_VERSION {
            return Err(format!("record version {} is not supported", record.v));
        }
        Ok(record)
    }
}

/// A record's `at`: RFC 3339 in UTC, to the second.
pub fn stamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// `sha256:` and the lower-case hex SHA-256 of the line's bytes: the next record's `prev`.
pub fn line_hash(line: &str) -> String {
    crate::util::token_hash(line)
}

/// `value` as canonical JSON: the keys of every object sorted by their bytes, no white space.
pub fn canonical(value: &Value) -> String {
    let mut out = String::new();
    write_canonical(value, &mut out);
    out
}

fn write_canonical(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, key) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(key.clone()).to_string());
                out.push(':');
                write_canonical(&map[key], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}
