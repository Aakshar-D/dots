//! The account's member list (members spec §6.1, §6.2): a new one, adding and removing people
//! and keys, the same check Dots makes, signing it into the text stored as `DOTS_ROSTER`, and
//! verifying that text as a Dots client does.
//!
//! Every write is checked with `parse_roster` first, so a list Dots would refuse is never
//! written, and the file is left as it was when the check fails.

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::{Cursor, ErrorKind, Write};

use chrono::{DateTime, Utc};
use dots_core::accounts::edit::{default_workspace_role, edit_members, new_member_id, Change};
use dots_core::accounts::roster::{
    parse_roster, roster_envelope, verify_roster, Member, Role, Roster, RosterError, Workspace,
    WorkspaceRole, ROSTER_VERSION,
};
use dots_core::vault::org::{check_org_id, key_fingerprint, UseRules};
use minisign::PublicKey;
use serde::Serialize;
use serde_json::Value;

use crate::args::Args;
use crate::keys::{read_public_key, unlock_secret_key};
use crate::policy::{check_dots_id, timestamp};
use crate::request::read_request;
use crate::{core_failed, failed, read_file, say, usage, write_file, Io, Result};

/// The signature's untrusted comment. Nothing reads it; it tells a person what the box is.
const UNTRUSTED_COMMENT: &str = "dots roster signature from dots-admin";

/// A roster as written to a file: `v`, then the roster's own fields in their usual order.
#[derive(Serialize)]
struct RosterFile<'a> {
    v: u32,
    #[serde(flatten)]
    roster: &'a Roster,
}

/// The roster as pretty JSON with a closing line end, the same way every write lays it out.
fn render(roster: &Roster) -> Result<String> {
    let mut text = serde_json::to_string_pretty(&RosterFile {
        v: ROSTER_VERSION,
        roster,
    })
    .map_err(|e| failed(format!("could not write the member list: {e}")))?;
    text.push('\n');
    Ok(text)
}

fn refused(error: RosterError) -> crate::AdminError {
    failed(error.to_string())
}

/// Reads and checks a roster file. Its own `org_id` is the org it is checked against.
fn read_roster(path: &str) -> Result<Roster> {
    let text = read_file(path, "the member list")?;
    let org = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| Some(v.get("org_id")?.as_str()?.to_string()))
        .unwrap_or_default();
    parse_roster(&text, &org).map_err(refused)
}

/// Checks `roster` as Dots does and writes it to `path`, replacing the file.
fn write_roster(path: &str, roster: &Roster) -> Result<()> {
    let text = render(roster)?;
    parse_roster(&text, &roster.org_id).map_err(refused)?;
    write_file(path, &text)
}

fn parse_role(text: &str) -> Result<Role> {
    Role::parse(text).map_err(|_| {
        failed(format!(
            "'{text}' is not a role; use owner, admin, member or viewer"
        ))
    })
}

fn parse_workspace_role(text: &str) -> Result<WorkspaceRole> {
    WorkspaceRole::parse(text).map_err(|_| {
        failed(format!(
            "'{text}' is not a workspace role; use admin, member or viewer"
        ))
    })
}

/// `<id>=<role>` values as a map, `what` naming them for the error ("--workspace"); the same
/// id twice is refused.
fn workspace_roles(what: &str, values: &[String]) -> Result<BTreeMap<String, WorkspaceRole>> {
    let mut roles = BTreeMap::new();
    for value in values {
        let (id, role) = value
            .split_once('=')
            .ok_or_else(|| usage(format!("{what} '{value}' must look like <id>=<role>")))?;
        if roles
            .insert(id.to_string(), parse_workspace_role(role)?)
            .is_some()
        {
            return Err(usage(format!("{what} {id} is given twice")));
        }
    }
    Ok(roles)
}

/// The key's fingerprint line a command prints for a key it added.
fn key_line(email: &str, key: &str) -> Result<String> {
    Ok(format!(
        "key {} for {email}",
        key_fingerprint(key).map_err(core_failed)?
    ))
}

/// A new roster: `seq` 0, the `default` workspace and the owner, an Admin of it.
pub fn new_roster(
    org_id: &str,
    owner_name: &str,
    owner_email: &str,
    owner_key: &str,
    now: DateTime<Utc>,
) -> Result<Roster> {
    check_org_id(org_id).map_err(core_failed)?;
    check_dots_id(owner_key)?;
    let mut roster = Roster {
        org_id: org_id.to_string(),
        issued_at: timestamp(now),
        seq: 0,
        workspaces: vec![Workspace {
            id: "default".into(),
            name: "Default".into(),
            use_rules: None,
        }],
        members: Vec::new(),
    };
    roster.members.push(Member {
        id: new_member_id(&roster, owner_key),
        name: owner_name.to_string(),
        email: owner_email.to_string(),
        role: Role::Owner,
        keys: vec![owner_key.to_string()],
        workspaces: BTreeMap::from([("default".to_string(), WorkspaceRole::Admin)]),
        added_at: timestamp(now),
    });
    parse_roster(&render(&roster)?, org_id).map_err(refused)?;
    Ok(roster)
}

/// `roster …`: the member list commands.
pub(crate) fn roster_command(args: &[String], io: &mut Io<'_>) -> Result<()> {
    let Some((sub, rest)) = args.split_first() else {
        return Err(usage(
            "'roster' needs one of: new, add, add-request, set-role, set-workspaces, workspace, \
             remove, check, sign, verify",
        ));
    };
    match sub.as_str() {
        "set-workspaces" => set_workspaces_command(rest, io),
        "workspace" => workspace_command(rest, io),
        "new" => new_command(rest, io),
        "add" => add_command(rest, io),
        "add-request" => add_request_command(rest, io),
        "set-role" => set_role_command(rest, io),
        "remove" => remove_command(rest, io),
        "check" => check_command(rest, io),
        "sign" => sign_command(rest, io),
        "verify" => verify_command(rest, io),
        other => Err(usage(format!("unknown roster command '{other}'"))),
    }
}

/// `roster new --org-id <id> --owner-name <n> --owner-email <e> --owner-key <dots-id>
/// [--out <file>]`. The file is never replaced: it may hold the real list.
fn new_command(args: &[String], io: &mut Io<'_>) -> Result<()> {
    let args = Args::parse(
        args,
        &["org-id", "owner-name", "owner-email", "owner-key", "out"],
        &[],
    )?;
    args.none()?;
    let owner_key = args.required("owner-key")?;
    let owner_email = args.required("owner-email")?;
    let roster = new_roster(
        args.required("org-id")?,
        args.required("owner-name")?,
        owner_email,
        owner_key,
        Utc::now(),
    )?;
    let text = render(&roster)?;
    match args.value("out") {
        None => say(io.out, text.trim_end())?,
        Some(path) => {
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
            say(io.out, &format!("wrote {path}"))?;
        }
    }
    say(io.err, &key_line(owner_email, owner_key)?)
}

/// `roster add <roster.json> --name <n> --email <e> --key <dots-id> --role <role>
/// [--workspace <id>=<role> ...]`.
fn add_command(args: &[String], io: &mut Io<'_>) -> Result<()> {
    let args = Args::parse_repeating(args, &["name", "email", "key", "role"], &["workspace"], &[])?;
    let path = args.one("the member list")?;
    let (name, email, key) = (
        args.required("name")?,
        args.required("email")?,
        args.required("key")?,
    );
    let role = parse_role(args.required("role")?)?;
    let given = workspace_roles("--workspace", args.all("workspace"))?;
    check_dots_id(key)?;

    let mut roster = read_roster(path)?;
    let line = if let Some(id) = roster.member_with_email(email).map(|m| m.id.clone()) {
        // Another PC for a person already listed.
        let member = member_mut(&mut roster, &id);
        if member.role != role {
            return Err(failed(format!(
                "{email} is already {}; change that with 'roster set-role'",
                member.role.label()
            )));
        }
        member.keys.push(key.to_string());
        member.workspaces.extend(given);
        format!("added another key for {}", key_line(email, key)?)
    } else {
        let mut workspaces =
            BTreeMap::from([("default".to_string(), default_workspace_role(role))]);
        workspaces.extend(given);
        let member = Member {
            id: new_member_id(&roster, key),
            name: name.to_string(),
            email: email.to_string(),
            role,
            keys: vec![key.to_string()],
            workspaces,
            added_at: timestamp(Utc::now()),
        };
        roster.members.push(member);
        format!("added {}", key_line(email, key)?)
    };
    write_roster(path, &roster)?;
    say(io.out, &line)
}

/// `roster set-role <roster.json> <email> <role>`.
fn set_role_command(args: &[String], io: &mut Io<'_>) -> Result<()> {
    let args = Args::parse(args, &[], &[])?;
    let [path, email, role] = args.words(["the member list", "the member's email", "the role"])?;
    let role = parse_role(role)?;
    let mut roster = read_roster(path)?;
    let member = member_by_email(&mut roster, email)?;
    member.role = role;
    write_roster(path, &roster)?;
    say(io.out, &format!("{email} is now {}", role.label()))
}

/// `roster set-workspaces <roster.json> <email> <id>=<role> ...`: the member's roles in every
/// workspace they are in, replacing the old ones. `default` must be one.
fn set_workspaces_command(args: &[String], io: &mut Io<'_>) -> Result<()> {
    let args = Args::parse(args, &[], &[])?;
    let (Some(path), Some(email)) = (args.rest(0).first(), args.rest(1).first()) else {
        return Err(usage(
            "give the member list, the member's email and one or more <id>=<role>",
        ));
    };
    let workspaces = workspace_roles("a workspace role", args.rest(2))?;
    let mut roster = read_roster(path)?;
    let member_id = member_by_email(&mut roster, email)?.id.clone();
    let change = Change::SetWorkspaces {
        member_id,
        workspaces,
    };
    // The root key signs this list, so changing an Owner's entry is allowed here.
    let (edited, _) = edit_members(&roster, &change, Utc::now(), true).map_err(core_failed)?;
    write_roster(path, &edited)?;
    say(io.out, &format!("{email}'s workspaces are changed"))
}

/// `roster workspace add|rename|remove|use ...`: the account's workspaces.
fn workspace_command(args: &[String], io: &mut Io<'_>) -> Result<()> {
    let Some((sub, rest)) = args.split_first() else {
        return Err(usage(
            "'roster workspace' needs one of: add, rename, remove, use",
        ));
    };
    match sub.as_str() {
        "add" => workspace_add_command(rest, io),
        "rename" => workspace_rename_command(rest, io),
        "remove" => workspace_remove_command(rest, io),
        "use" => workspace_use_command(rest, io),
        other => Err(usage(format!("unknown roster workspace command '{other}'"))),
    }
}

/// The `--allow` and `--deny` patterns as use rules; none given is no rules.
fn use_rules(args: &Args) -> Option<UseRules> {
    let (allow, deny) = (args.all("allow"), args.all("deny"));
    if allow.is_empty() && deny.is_empty() {
        return None;
    }
    let mut rules = UseRules::default();
    if !allow.is_empty() {
        rules.allow = allow.to_vec();
    }
    rules.deny = deny.to_vec();
    Some(rules)
}

/// `roster workspace add <roster.json> <name> [--allow <pattern> ...] [--deny <pattern> ...]`.
fn workspace_add_command(args: &[String], io: &mut Io<'_>) -> Result<()> {
    let args = Args::parse_repeating(args, &[], &["allow", "deny"], &[])?;
    let [path, name] = args.words(["the member list", "the workspace name"])?;
    let mut roster = read_roster(path)?;
    let change = Change::AddWorkspace { name: name.into() };
    let (mut edited, summary) =
        edit_members(&roster, &change, Utc::now(), true).map_err(core_failed)?;
    let added = edited
        .workspaces
        .last_mut()
        .expect("a workspace was just added");
    added.use_rules = use_rules(&args);
    let id = added.id.clone();
    roster = edited;
    write_roster(path, &roster)?;
    say(io.out, &format!("{summary}: id {id}"))
}

/// `roster workspace rename <roster.json> <id> <name>`.
fn workspace_rename_command(args: &[String], io: &mut Io<'_>) -> Result<()> {
    let args = Args::parse(args, &[], &[])?;
    let [path, id, name] = args.words(["the member list", "the workspace id", "the new name"])?;
    let roster = read_roster(path)?;
    let change = Change::RenameWorkspace {
        id: id.into(),
        name: name.into(),
    };
    let (edited, summary) =
        edit_members(&roster, &change, Utc::now(), true).map_err(core_failed)?;
    write_roster(path, &edited)?;
    say(io.out, &summary)
}

/// `roster workspace remove <roster.json> <id>`: never `default`.
fn workspace_remove_command(args: &[String], io: &mut Io<'_>) -> Result<()> {
    let args = Args::parse(args, &[], &[])?;
    let [path, id] = args.words(["the member list", "the workspace id"])?;
    let roster = read_roster(path)?;
    let change = Change::DeleteWorkspace { id: id.into() };
    let (edited, summary) =
        edit_members(&roster, &change, Utc::now(), true).map_err(core_failed)?;
    write_roster(path, &edited)?;
    say(io.out, &summary)
}

/// `roster workspace use <roster.json> <id> [--allow <pattern> ...] [--deny <pattern> ...]`:
/// the workspace's use rules; with no flags, the rules are cleared.
fn workspace_use_command(args: &[String], io: &mut Io<'_>) -> Result<()> {
    let args = Args::parse_repeating(args, &[], &["allow", "deny"], &[])?;
    let [path, id] = args.words(["the member list", "the workspace id"])?;
    let mut roster = read_roster(path)?;
    let Some(workspace) = roster.workspaces.iter_mut().find(|w| w.id == id) else {
        return Err(failed(format!("no workspace '{id}' on the list")));
    };
    workspace.use_rules = use_rules(&args);
    let line = match &workspace.use_rules {
        Some(_) => format!("the {} workspace has new use rules", workspace.name),
        None => format!("the {} workspace has no use rules", workspace.name),
    };
    write_roster(path, &roster)?;
    say(io.out, &line)
}

/// `roster remove <roster.json> <email> [--key <dots-id>]`: the member, or one of their keys.
fn remove_command(args: &[String], io: &mut Io<'_>) -> Result<()> {
    let args = Args::parse(args, &["key"], &[])?;
    let [path, email] = args.words(["the member list", "the member's email"])?;
    let mut roster = read_roster(path)?;
    let line = match args.value("key") {
        Some(key) => {
            let member = member_by_email(&mut roster, email)?;
            let Some(at) = member.keys.iter().position(|k| k == key) else {
                return Err(failed(format!("{email} does not list that key")));
            };
            if member.keys.len() == 1 {
                return Err(failed(format!(
                    "that is {email}'s only key; remove the member instead"
                )));
            }
            member.keys.remove(at);
            format!("removed {}", key_line(email, key)?)
        }
        None => {
            let id = member_by_email(&mut roster, email)?.id.clone();
            roster.members.retain(|m| m.id != id);
            format!("removed {email}")
        }
    };
    write_roster(path, &roster)?;
    say(io.out, &line)
}

/// The member whose email matches as Dots matches it (trimmed, any case).
fn member_by_email<'a>(roster: &'a mut Roster, email: &str) -> Result<&'a mut Member> {
    let id = roster
        .member_with_email(email)
        .map(|m| m.id.clone())
        .ok_or_else(|| failed(format!("no member has the email {email}")))?;
    Ok(member_mut(roster, &id))
}

fn member_mut<'a>(roster: &'a mut Roster, id: &str) -> &'a mut Member {
    roster
        .members
        .iter_mut()
        .find(|m| m.id == id)
        .expect("the id was just read from this list")
}

/// `roster add-request <roster.json> <code> --role <role>`: adds the sender of a request code,
/// or another PC for the person with that email. The list is not signed or numbered; `sign`
/// does that.
fn add_request_command(args: &[String], io: &mut Io<'_>) -> Result<()> {
    let args = Args::parse(args, &["role"], &[])?;
    let [path, code] = args.words(["the member list", "the request code"])?;
    let role = parse_role(args.required("role")?)?;
    let request = read_request(code)?;
    let mut roster = read_roster(path)?;
    if request.org_id != roster.org_id {
        return Err(failed(format!(
            "the request is for org '{}', but this member list is for '{}'",
            request.org_id, roster.org_id
        )));
    }
    // A request is self-asserted: one naming a listed person (even the Owner) adds a PC to
    // them only with --role matching theirs, as `roster add` does.
    let joins = roster
        .member_with_email(&request.email)
        .map(|m| (m.name.clone(), m.role));
    if let Some((_, listed)) = &joins {
        if *listed != role {
            return Err(failed(format!(
                "{} is already {}; change that with 'roster set-role'",
                request.email,
                listed.label()
            )));
        }
    }
    let change = Change::AddKey {
        name: request.name.clone(),
        email: request.email.clone(),
        key: request.key.clone(),
        role,
        workspaces: BTreeMap::from([("default".to_string(), default_workspace_role(role))]),
    };
    // The root key signs this list, so changing an Owner's entry is allowed here.
    let (edited, summary) =
        edit_members(&roster, &change, Utc::now(), true).map_err(core_failed)?;
    roster = edited;
    write_roster(path, &roster)?;
    if let Some((name, listed)) = &joins {
        say(
            io.out,
            &format!("This adds a PC to {name} ({}).", listed.label()),
        )?;
    }
    say(
        io.out,
        &format!("{summary}: {}", key_line(&request.email, &request.key)?),
    )?;
    if let Some(age) = &request.age_recipient {
        say(
            io.out,
            &format!("Add this age recipient to the SOPS file's recipients: {age}"),
        )?;
    }
    Ok(())
}

/// `roster check <roster.json> --org-id <id>`: "ok" and the summary, or why Dots would refuse
/// it.
fn check_command(args: &[String], io: &mut Io<'_>) -> Result<()> {
    let args = Args::parse(args, &["org-id"], &[])?;
    let path = args.one("the member list")?;
    let org = args.required("org-id")?;
    check_org_id(org).map_err(core_failed)?;
    let roster = parse_roster(&read_file(path, "the member list")?, org).map_err(refused)?;
    say(io.out, "ok")?;
    summarize(&roster, io)
}

fn summarize(roster: &Roster, io: &mut Io<'_>) -> Result<()> {
    say(
        io.out,
        &format!(
            "  member list #{} for org '{}': {} member(s), {} workspace(s)",
            roster.seq,
            roster.org_id,
            roster.members.len(),
            roster.workspaces.len()
        ),
    )?;
    for member in &roster.members {
        say(
            io.out,
            &format!(
                "  {} <{}>: {}, {} key(s)",
                member.name,
                member.email,
                member.role.label(),
                member.keys.len()
            ),
        )?;
    }
    Ok(())
}

/// `roster sign <roster.json> --key <policy.key> [--out <file>]`, in this order:
///
/// 1. checks the list as Dots does, before any password is asked for;
/// 2. raises `seq` by one and sets `issued_at` to now;
/// 3. signs the exact text with the trusted comment `dots roster <org_id> <seq>`;
/// 4. verifies the envelope with `verify_roster`, as a Dots client will;
/// 5. only then writes the new `seq` and `issued_at` back to the file, so the next signing
///    grows from them.
fn sign_command(args: &[String], io: &mut Io<'_>) -> Result<()> {
    let args = Args::parse(args, &["key", "out"], &[])?;
    let path = args.one("the member list")?;
    let key_text = read_file(args.required("key")?, "the key")?;
    let mut roster = read_roster(path)?;
    roster.seq += 1;
    roster.issued_at = timestamp(Utc::now());
    let text = render(&roster)?;
    parse_roster(&text, &roster.org_id).map_err(refused)?;

    let secret_key = unlock_secret_key(&key_text, &mut *io.password)?;
    let public_key = PublicKey::from_secret_key(&secret_key)
        .map_err(|_| failed("the --key file is not a minisign secret key"))?;
    let trusted = format!("dots roster {} {}", roster.org_id, roster.seq);
    let signature = minisign::sign(
        Some(&public_key),
        &secret_key,
        Cursor::new(text.as_bytes()),
        Some(&trusted),
        Some(UNTRUSTED_COMMENT),
    )
    .map_err(|e| failed(format!("could not sign the member list: {e}")))?;
    let signed = roster_envelope(&text, &signature.to_string());
    let public_key = public_key.to_base64();
    verify_roster(&signed, &public_key, &[], &roster.org_id)
        .map_err(|e| failed(format!("the signed member list does not verify: {e}")))?;
    write_file(path, &text)?;

    let note = format!(
        "signed member list #{} for org '{}', issued_at {}, with key {}; store the signed text as DOTS_ROSTER",
        roster.seq,
        roster.org_id,
        roster.issued_at,
        key_fingerprint(&public_key).map_err(core_failed)?
    );
    match args.value("out") {
        Some(out) => {
            write_file(out, &format!("{signed}\n"))?;
            say(io.out, &note)?;
            say(io.out, &format!("wrote {out}"))
        }
        None => {
            say(io.out, &signed)?;
            say(io.err, &note)
        }
    }
}

/// `roster verify <signed> --pub <policy.pub> [--admin <dots-id> ...] --org-id <id>`.
fn verify_command(args: &[String], io: &mut Io<'_>) -> Result<()> {
    let args = Args::parse_repeating(args, &["pub", "org-id"], &["admin"], &[])?;
    let path = args.one("the signed member list")?;
    let org = args.required("org-id")?;
    check_org_id(org).map_err(core_failed)?;
    let public_key = read_public_key(&read_file(args.required("pub")?, "the public key")?)?;
    for admin in args.all("admin") {
        check_dots_id(admin)?;
    }
    let signed = read_file(path, "the signed member list")?;
    let accepted = verify_roster(signed.trim(), &public_key, args.all("admin"), org)
        .map_err(|e| failed(format!("a Dots client would refuse it: {e}")))?;
    say(
        io.out,
        &format!(
            "ok: a Dots client in org '{org}' accepts this member list, signed by key {}",
            key_fingerprint(&accepted.signer).map_err(core_failed)?
        ),
    )?;
    summarize(&accepted.roster, io)?;
    say(
        io.out,
        "Dots refuses it all the same if it has accepted a member list with a higher number.",
    )
}
