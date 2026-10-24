//! The org's policy (env vault spec §6.3): a new one with every default, the same check Dots
//! makes, signing it into the text stored as `DOTS_POLICY`, and verifying that text as a Dots
//! client does.

use std::fs::OpenOptions;
use std::io::{Cursor, ErrorKind, Write};
use std::ops::Range;

use chrono::{DateTime, SecondsFormat, Utc};
use dots_core::vault::org::policy::POLICY_VERSION;
use dots_core::vault::org::{
    check_org_id, envelope, key_fingerprint, parse_policy, verify_policy, OrgPolicy, PolicyError,
};
use minisign::PublicKey;
use serde_json::Value;

use crate::args::Args;
use crate::keys::{read_public_key, unlock_secret_key};
use crate::{core_failed, failed, read_file, say, write_file, Io, PasswordSource, Result};

/// The signature's untrusted comment. Nothing reads it; it tells a person what the box is.
const UNTRUSTED_COMMENT: &str = "dots policy signature from dots-admin";

/// `issued_at` as `policy new` and `policy sign` write it: UTC, to the second.
pub fn timestamp(now: DateTime<Utc>) -> String {
    now.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// A new policy's text, with every field present at its default so the admin sees what can be
/// set, and `issued_at` set to `now`.
pub fn new_policy(org_id: &str, now: DateTime<Utc>) -> Result<String> {
    check_org_id(org_id).map_err(core_failed)?;
    Ok(format!(
        r#"{{
  "v": {POLICY_VERSION},
  "org_id": {org_id},
  "issued_at": {issued_at},
  "plain": [],
  "use": {{ "allow": ["*"], "deny": [] }},
  "personal": "deny",
  "shell_secrets": false,
  "refresh_minutes": 15,
  "offline_hours": 24,
  "audit": null
}}
"#,
        org_id = json_string(org_id),
        issued_at = json_string(&timestamp(now)),
    ))
}

/// The policy text signed: the envelope to store as `DOTS_POLICY`, and the policy in it.
#[derive(Debug, Clone)]
pub struct Signed {
    pub text: String,
    pub policy: OrgPolicy,
    /// The signing key's fingerprint, which every join code for the org must carry.
    pub fingerprint: String,
}

/// Signs a policy, in this order:
///
/// 1. checks it as Dots does, before any password is asked for;
/// 2. sets `issued_at` to `now`, rewriting only that value, so the admin's layout stays;
/// 3. checks the result again;
/// 4. signs its exact text with the trusted comment `dots policy <org_id> <issued_at>`;
/// 5. verifies the envelope with `verify_policy`, as a Dots client will.
pub fn sign_policy(
    policy_text: &str,
    secret_key_text: &str,
    password: PasswordSource<'_>,
    now: DateTime<Utc>,
) -> Result<Signed> {
    parse_policy(policy_text).map_err(refused)?;
    let text = set_issued_at(policy_text, &timestamp(now))?;
    let policy = parse_policy(&text).map_err(refused)?;

    let secret_key = unlock_secret_key(secret_key_text, password)?;
    let public_key = PublicKey::from_secret_key(&secret_key)
        .map_err(|_| failed("the --key file is not a minisign secret key"))?;
    let trusted = format!("dots policy {} {}", policy.org_id, policy.issued_at);
    let signature = minisign::sign(
        Some(&public_key),
        &secret_key,
        Cursor::new(text.as_bytes()),
        Some(&trusted),
        Some(UNTRUSTED_COMMENT),
    )
    .map_err(|e| failed(format!("could not sign the policy: {e}")))?;
    let signed = envelope(&text, &signature.to_string());

    let public_key = public_key.to_base64();
    verify_policy(&signed, &public_key, &policy.org_id)
        .map_err(|e| failed(format!("the signed policy does not verify: {e}")))?;
    Ok(Signed {
        text: signed,
        policy,
        fingerprint: key_fingerprint(&public_key).map_err(core_failed)?,
    })
}

/// Verifies a signed text as a Dots client in `org_id` does. `None` checks it against the org
/// the policy names. `public_key_text` is a `policy.pub` file's text or its `RW…` line.
pub fn verify_signed(
    signed: &str,
    public_key_text: &str,
    org_id: Option<&str>,
) -> Result<OrgPolicy> {
    let public_key = read_public_key(public_key_text)?;
    let org = match org_id {
        Some(org) => {
            check_org_id(org).map_err(core_failed)?;
            org.to_string()
        }
        // An unreadable text fails `verify_policy` before the org is compared.
        None => claimed_org(signed).unwrap_or_default(),
    };
    verify_policy(signed, &public_key, &org)
        .map_err(|e| failed(format!("a Dots client would refuse it: {e}")))
}

/// `policy new --org-id <id> [--out <file>]`. The file is never replaced: it may hold edits.
pub(crate) fn new_command(args: &[String], io: &mut Io<'_>) -> Result<()> {
    let args = Args::parse(args, &["org-id", "out"], &[])?;
    args.none()?;
    let text = new_policy(args.required("org-id")?, Utc::now())?;
    let Some(path) = args.value("out") else {
        return write_all(io.out, &text);
    };
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| match e.kind() {
            ErrorKind::AlreadyExists => {
                failed(format!("{path} already exists; choose another --out"))
            }
            _ => failed(format!("could not write {path}: {e}")),
        })?;
    file.write_all(text.as_bytes())
        .map_err(|e| failed(format!("could not write {path}: {e}")))?;
    say(io.out, &format!("wrote {path}"))
}

/// `policy check <policy.json>`: "ok" and the summary, or why Dots would refuse it.
pub(crate) fn check_command(args: &[String], io: &mut Io<'_>) -> Result<()> {
    let args = Args::parse(args, &[], &[])?;
    let path = args.one("the policy file")?;
    let policy = parse_policy(&read_file(path, "the policy")?).map_err(refused)?;
    say(io.out, "ok")?;
    for line in policy.summary() {
        say(io.out, &format!("  {line}"))?;
    }
    Ok(())
}

/// `policy sign <policy.json> --key <policy.key> [--out <file>]`.
pub(crate) fn sign_command(args: &[String], io: &mut Io<'_>) -> Result<()> {
    let args = Args::parse(args, &["key", "out"], &[])?;
    let path = args.one("the policy file")?;
    let key_path = args.required("key")?;
    let policy = read_file(path, "the policy")?;
    let key = read_file(key_path, "the key")?;
    let signed = sign_policy(&policy, &key, &mut *io.password, Utc::now())?;
    let note = format!(
        "signed the policy for org '{}', issued_at {}, with key {}",
        signed.policy.org_id, signed.policy.issued_at, signed.fingerprint
    );
    match args.value("out") {
        Some(out) => {
            write_file(out, &format!("{}\n", signed.text))?;
            say(io.out, &note)?;
            say(
                io.out,
                &format!("wrote {out}; store its text as DOTS_POLICY"),
            )
        }
        None => {
            say(io.out, &signed.text)?;
            say(io.err, &note)
        }
    }
}

/// `policy verify <signed> --pub <policy.pub> [--org-id <id>]`.
pub(crate) fn verify_command(args: &[String], io: &mut Io<'_>) -> Result<()> {
    let args = Args::parse(args, &["pub", "org-id"], &[])?;
    let path = args.one("the signed policy file")?;
    let public_key = read_file(args.required("pub")?, "the public key")?;
    let signed = read_file(path, "the signed policy")?;
    let policy = verify_signed(&signed, &public_key, args.value("org-id"))?;
    let fingerprint = key_fingerprint(&read_public_key(&public_key)?).map_err(core_failed)?;
    say(
        io.out,
        &format!(
            "ok: a Dots client in org '{}' accepts this policy, signed by key {fingerprint}",
            policy.org_id
        ),
    )?;
    for line in policy.summary() {
        say(io.out, &format!("  {line}"))?;
    }
    say(
        io.out,
        "Dots refuses it all the same if it has accepted a policy issued later.",
    )
}

fn refused(error: PolicyError) -> crate::AdminError {
    failed(format!("policy invalid: {error}"))
}

fn write_all(to: &mut dyn Write, text: &str) -> Result<()> {
    to.write_all(text.as_bytes())
        .map_err(|e| failed(format!("could not write the output: {e}")))
}

fn json_string(text: &str) -> String {
    Value::String(text.to_string()).to_string()
}

/// The org a signed text's policy names, read without any check, to verify it against.
fn claimed_org(signed: &str) -> Option<String> {
    let envelope: Value = serde_json::from_str(signed).ok()?;
    let policy: Value = serde_json::from_str(envelope.get("policy")?.as_str()?).ok()?;
    Some(policy.get("org_id")?.as_str()?.to_string())
}

/// Replaces the value of the top-level `issued_at` in a policy text that `parse_policy`
/// accepted, leaving every other byte as it was.
fn set_issued_at(text: &str, issued_at: &str) -> Result<String> {
    let span = issued_at_span(text)
        .ok_or_else(|| failed("policy invalid: could not find its issued_at"))?;
    let mut rewritten = String::with_capacity(text.len());
    rewritten.push_str(&text[..span.start]);
    rewritten.push_str(&json_string(issued_at));
    rewritten.push_str(&text[span.end..]);
    Ok(rewritten)
}

/// Where the top-level `issued_at` string value is, quotes included. It walks the JSON text
/// tracking depth and strings, so a nested `issued_at` or one inside a string is passed over,
/// and a key spelled with escapes still counts. `None` if there is none.
fn issued_at_span(text: &str) -> Option<Range<usize>> {
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    // At depth 1: the next string is a key; the last key was `issued_at`; its value is next.
    let (mut key_next, mut is_issued_at, mut value_next) = (false, false, false);
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                let end = string_end(bytes, i)?;
                if depth == 1 && value_next {
                    return Some(i..end);
                }
                if depth == 1 && key_next {
                    let key: String = serde_json::from_str(&text[i..end]).ok()?;
                    is_issued_at = key == "issued_at";
                    key_next = false;
                }
                i = end;
                continue;
            }
            b'{' | b'[' => {
                depth += 1;
                key_next = depth == 1;
                value_next = false;
            }
            b'}' | b']' => depth = depth.checked_sub(1)?,
            b':' if depth == 1 => value_next = is_issued_at,
            b',' if depth == 1 => {
                key_next = true;
                value_next = false;
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// The index just past the string that opens at `start`.
fn string_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut i = start + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return Some(i + 1),
            _ => i += 1,
        }
    }
    None
}
