//! An org's policy (env vault spec §6.3): which of its entries are plain, which names dots may be
//! granted, how Personal entries mix in, and how often to refresh. It fails closed: an unknown
//! field, an unknown version or a value out of range makes the whole policy invalid.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::vault::entries::{check_variable_name, MAX_VARIABLE_NAME};

str_enum!(PersonalMode {
    Deny => "deny",
    Fallback => "fallback",
    Override => "override",
});

impl Default for PersonalMode {
    /// Only the org's entries resolve unless the admin says otherwise.
    fn default() -> Self {
        Self::Deny
    }
}

// Whether an org keeps a signed member list (members spec §6.1). `Required` makes the roster
// the gate for who may use the org's entries.
str_enum!(RosterMode {
    Off => "off",
    Required => "required",
});

impl Default for RosterMode {
    /// No member list unless the admin turns it on.
    fn default() -> Self {
        Self::Off
    }
}

/// The newest policy version this Dots reads, and the one a roster policy is written as.
pub const POLICY_VERSION: u32 = 2;
/// The first policy version, with no member list. `policy new` still writes it.
pub const POLICY_V1: u32 = 1;
/// Every policy version this Dots reads.
pub const POLICY_VERSIONS: [u32; 2] = [POLICY_V1, POLICY_VERSION];

const FIELDS: &[&str] = &[
    "v",
    "org_id",
    "issued_at",
    "plain",
    "use",
    "personal",
    "shell_secrets",
    "refresh_minutes",
    "offline_hours",
    "audit",
];

/// The fields only a v2 policy may carry.
const FIELDS_V2: &[&str] = &["roster", "admins"];

/// Why a policy, or the signed text that carries it, was refused. The text names what failed in
/// plain words, for "Acme Engineering: policy invalid: <error>".
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PolicyError {
    /// The signed text is not the `{"v":1,"policy":…,"signature":…}` envelope.
    #[error("the signed policy is not readable: {0}")]
    Envelope(String),
    /// The join code's `policy_key` is not a minisign public key.
    #[error("the org's policy key is not a minisign public key")]
    Key,
    /// The signature is not by the org's key, or not over this policy text.
    #[error("signature does not match")]
    Signature,
    /// The policy text itself: bad JSON, an unknown field, a value out of range.
    #[error("{0}")]
    Invalid(String),
    /// The policy names another org than the join code.
    #[error("for org '{found}', not '{expected}'")]
    WrongOrg { found: String, expected: String },
    /// The policy is older than the last one accepted for this org.
    #[error("issued before the last accepted policy ({issued_at}, last {last})")]
    RolledBack { issued_at: String, last: String },
}

/// Which org entry names dots may be granted. A name must match an `allow` glob and no `deny`
/// glob: deny wins. It is also sent to the UI with the org's summary, so the dot editor offers
/// only names the org lets its dots use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UseRules {
    #[serde(default = "allow_all")]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
}

impl Default for UseRules {
    fn default() -> Self {
        Self {
            allow: allow_all(),
            deny: Vec::new(),
        }
    }
}

fn allow_all() -> Vec<String> {
    vec!["*".into()]
}

/// Where audit records go (plan C sends them). `token_entry` names an org entry sent as
/// `Authorization: Bearer`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditTarget {
    pub endpoint: String,
    pub token_entry: String,
}

/// A policy that parsed and passed every check. It is only made by `parse_policy`, so it is
/// always valid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrgPolicy {
    pub org_id: String,
    /// The `issued_at` text as the admin wrote it, for display and for `orgs.last_issued_at`.
    pub issued_at: String,
    pub plain: Vec<String>,
    pub use_rules: UseRules,
    pub personal: PersonalMode,
    pub shell_secrets: bool,
    pub refresh_minutes: u32,
    pub offline_hours: u32,
    pub audit: Option<AuditTarget>,
    /// Whether the org keeps a signed member list; always `Off` for a v1 policy.
    pub roster: RosterMode,
    /// The Dots IDs (minisign public keys, base64) of the org's admins; empty for a v1 policy.
    pub admins: Vec<String>,
    /// `issued_at` as an instant, so `+01:00` and `Z` spellings compare right.
    issued: DateTime<Utc>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPolicy {
    /// Checked to be 1 or 2 before this struct is parsed.
    #[serde(rename = "v")]
    _v: u64,
    org_id: String,
    issued_at: String,
    #[serde(default)]
    plain: Vec<String>,
    #[serde(default, rename = "use")]
    use_rules: UseRules,
    #[serde(default)]
    personal: PersonalMode,
    #[serde(default)]
    shell_secrets: bool,
    #[serde(default = "default_refresh_minutes")]
    refresh_minutes: u32,
    #[serde(default = "default_offline_hours")]
    offline_hours: u32,
    #[serde(default)]
    audit: Option<AuditTarget>,
    #[serde(default)]
    roster: RosterMode,
    #[serde(default)]
    admins: Vec<String>,
}

/// `refresh_minutes` when the policy leaves it out. The org timers use it too while an org
/// has no accepted policy.
pub const DEFAULT_REFRESH_MINUTES: u32 = 15;

fn default_refresh_minutes() -> u32 {
    DEFAULT_REFRESH_MINUTES
}

fn default_offline_hours() -> u32 {
    24
}

/// Parses and checks a policy's JSON text. The version and the top-level field names are checked
/// before anything else, so a newer policy says it is newer instead of naming a field.
pub fn parse_policy(text: &str) -> Result<OrgPolicy, PolicyError> {
    let value: Value = serde_json::from_str(text)
        .map_err(|e| invalid(format!("not JSON: {}", plain_serde(&e))))?;
    let Value::Object(fields) = &value else {
        return Err(invalid("the policy must be a JSON object"));
    };
    let version = match fields.get("v") {
        None => return Err(invalid("missing field 'v'")),
        Some(v) => match v.as_u64().and_then(|n| u32::try_from(n).ok()) {
            Some(n) if POLICY_VERSIONS.contains(&n) => n,
            _ => {
                return Err(invalid(format!(
                    "policy version {v} is not supported; this Dots reads versions 1 and 2"
                )))
            }
        },
    };
    let allowed = |k: &str| FIELDS.contains(&k) || (version >= 2 && FIELDS_V2.contains(&k));
    if let Some(unknown) = fields.keys().find(|k| !allowed(k.as_str())) {
        return Err(invalid(format!("unknown field '{unknown}'")));
    }
    // Parsed again from the text, not the `Value`, so a field given twice is refused.
    let raw: RawPolicy = serde_json::from_str(text).map_err(|e| invalid(plain_serde(&e)))?;

    super::join::org_id_rule(&raw.org_id).map_err(invalid)?;
    let issued = DateTime::parse_from_rfc3339(&raw.issued_at)
        .map_err(|_| {
            invalid(format!(
                "issued_at '{}' is not an RFC 3339 time, like 2026-10-21T09:00:00Z",
                raw.issued_at
            ))
        })?
        .with_timezone(&Utc);
    for name in &raw.plain {
        check_variable_name(name)
            .map_err(|_| invalid(format!("plain: '{name}' is not a variable name")))?;
    }
    for (list, globs) in [
        ("use.allow", &raw.use_rules.allow),
        ("use.deny", &raw.use_rules.deny),
    ] {
        for glob in globs {
            check_glob(glob).map_err(|why| invalid(format!("{list}: '{glob}' {why}")))?;
        }
    }
    if raw.refresh_minutes < 1 {
        return Err(invalid("refresh_minutes must be at least 1"));
    }
    if let Some(audit) = &raw.audit {
        check_audit(audit)?;
    }
    for (i, admin) in raw.admins.iter().enumerate() {
        if minisign_verify::PublicKey::from_base64(admin).is_err() {
            return Err(invalid(format!("admins[{i}] is not a Dots ID")));
        }
    }

    Ok(OrgPolicy {
        org_id: raw.org_id,
        issued_at: raw.issued_at,
        plain: raw.plain,
        use_rules: raw.use_rules,
        personal: raw.personal,
        shell_secrets: raw.shell_secrets,
        refresh_minutes: raw.refresh_minutes,
        offline_hours: raw.offline_hours,
        audit: raw.audit,
        roster: raw.roster,
        admins: raw.admins,
        issued,
    })
}

impl OrgPolicy {
    /// Whether dots may be granted the org entry `name`: an `allow` glob matches and no `deny`
    /// glob does. Case is ignored, as names compare that way.
    pub fn may_use(&self, name: &str) -> bool {
        let matches = |globs: &[String]| globs.iter().any(|g| glob_matches(g, name));
        !matches(&self.use_rules.deny) && matches(&self.use_rules.allow)
    }

    /// Whether the org entry `name` is plain, shown with its value. Every other entry is secret.
    pub fn is_plain(&self, name: &str) -> bool {
        self.plain.iter().any(|p| p.eq_ignore_ascii_case(name))
    }

    /// The rollback check (§6.3): refuses this policy when it was issued before `last_issued_at`,
    /// the `issued_at` of the last policy accepted for the org. The same instant is a re-fetch of
    /// the same policy and passes. `None`, no policy accepted yet, always passes. A `last` that is
    /// not RFC 3339 is refused too, so a damaged row never turns the check off.
    pub fn check_not_rolled_back(&self, last_issued_at: Option<&str>) -> Result<(), PolicyError> {
        let Some(last) = last_issued_at else {
            return Ok(());
        };
        let rolled_back = || PolicyError::RolledBack {
            issued_at: self.issued_at.clone(),
            last: last.to_string(),
        };
        let last_at = DateTime::parse_from_rfc3339(last).map_err(|_| rolled_back())?;
        if self.issued < last_at {
            return Err(rolled_back());
        }
        Ok(())
    }

    /// The policy in plain words, one line per field: what `dots-admin policy check` prints and
    /// what the app shows for a joined org, so the admin and the person read the same words.
    pub fn summary(&self) -> Vec<String> {
        let rules = &self.use_rules;
        let plain = if self.plain.is_empty() {
            "plain entries: none; every org entry is a secret".to_string()
        } else {
            format!(
                "plain entries, shown with their values: {}; every other org entry is a secret",
                self.plain.join(", ")
            )
        };
        let granted = match (rules.allow.as_slice(), rules.deny.as_slice()) {
            ([], _) => "no org entry (use.allow is empty)".to_string(),
            ([all], []) if all == "*" => "every org entry".to_string(),
            ([all], deny) if all == "*" => {
                format!("every org entry except names matching {}", deny.join(", "))
            }
            (allow, []) => format!("names matching {}", allow.join(", ")),
            (allow, deny) => format!(
                "names matching {}, except names matching {}",
                allow.join(", "),
                deny.join(", ")
            ),
        };
        let personal = match self.personal {
            PersonalMode::Deny => "personal entries: not used; only the org's entries resolve",
            PersonalMode::Fallback => {
                "personal entries: used when the org has no entry of that name"
            }
            PersonalMode::Override => "personal entries: win over the org's entry of the same name",
        };
        let shell = if self.shell_secrets {
            "secrets in the shell: allowed"
        } else {
            "secrets in the shell: not allowed"
        };
        let offline = match self.offline_hours {
            0 => "offline copy: none; offline, the org's entries are not available".to_string(),
            hours => format!(
                "offline copy: kept for {} after the last good fetch",
                count(hours, "hour")
            ),
        };
        let audit = match &self.audit {
            None => "audit: off".to_string(),
            Some(audit) => format!(
                "audit: records go to {}, with the token in entry {}",
                audit.endpoint, audit.token_entry
            ),
        };
        let mut lines = vec![
            format!("org: {}", self.org_id),
            format!("issued at: {}", self.issued_at),
            plain,
            format!("dots may be granted: {granted}"),
            personal.to_string(),
            shell.to_string(),
            format!("refresh: every {}", count(self.refresh_minutes, "minute")),
            offline,
            audit,
        ];
        if self.roster == RosterMode::Required {
            lines.push(format!(
                "Member list: required, {} admin(s)",
                self.admins.len()
            ));
        }
        lines
    }
}

/// `n` of `unit`, "1 hour" or "24 hours".
fn count(n: u32, unit: &str) -> String {
    match n {
        1 => format!("1 {unit}"),
        n => format!("{n} {unit}s"),
    }
}

fn check_audit(audit: &AuditTarget) -> Result<(), PolicyError> {
    let url = reqwest::Url::parse(&audit.endpoint);
    let https = url
        .as_ref()
        .is_ok_and(|u| u.scheme() == "https" && u.host_str().is_some_and(|h| !h.is_empty()));
    if !https {
        return Err(invalid(format!(
            "audit.endpoint '{}' must be an https:// URL",
            audit.endpoint
        )));
    }
    check_variable_name(&audit.token_entry).map_err(|_| {
        invalid(format!(
            "audit.token_entry '{}' is not a variable name",
            audit.token_entry
        ))
    })
}

/// A glob is a variable name in which `*` may stand for any run of characters, `*` alone
/// included. Nothing else is special.
fn check_glob(glob: &str) -> Result<(), &'static str> {
    if glob.is_empty() || glob.len() > MAX_VARIABLE_NAME {
        return Err("must be 1 to 128 characters");
    }
    if !glob
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'*')
    {
        return Err("may hold only A-Z, a-z, 0-9, '_' and '*'");
    }
    Ok(())
}

/// Matches `name` against `glob`, where `*` is any run of characters, without regard to case.
/// Both are ASCII by the name and glob rules. The classic two-pointer match: on a mismatch,
/// retry from the last `*`, letting it take one more character.
fn glob_matches(glob: &str, name: &str) -> bool {
    let (g, n) = (glob.as_bytes(), name.as_bytes());
    let (mut gi, mut ni) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ni < n.len() {
        if gi < g.len() && g[gi] == b'*' {
            star = Some((gi, ni));
            gi += 1;
        } else if gi < g.len() && g[gi].eq_ignore_ascii_case(&n[ni]) {
            gi += 1;
            ni += 1;
        } else if let Some((sg, sn)) = star {
            gi = sg + 1;
            ni = sn + 1;
            star = Some((sg, sn + 1));
        } else {
            return false;
        }
    }
    g[gi..].iter().all(|&b| b == b'*')
}

fn invalid(message: impl Into<String>) -> PolicyError {
    PolicyError::Invalid(message.into())
}

/// A serde error as plain words: no "at line 1 column 9", and 'quotes' for serde's backticks.
pub(crate) fn plain_serde(e: &serde_json::Error) -> String {
    let text = e.to_string();
    let suffix = format!(" at line {} column {}", e.line(), e.column());
    text.strip_suffix(&suffix)
        .unwrap_or(&text)
        .replace('`', "'")
}
