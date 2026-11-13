//! Workspace guardrails (guardrails spec §3, §4): what an Admin marks as protected in a
//! workspace (paths, git actions, commands) and the highest tool policy a Member's dot may use
//! there. They live on the signed member list; this module holds their shape, their checks,
//! and who they apply to on this PC.

use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::permissions::{access_in, workspace_name, Access};
use super::roster::Role;
use crate::policy::rails::{self, RailHit};
use crate::policy::{Policy, Preset};
use crate::vault::org::Orgs;

/// The most entries each list may have.
pub const MAX_ENTRIES: usize = 100;
/// The longest a protected path may be.
pub const MAX_PATH: usize = 200;
/// The longest a protected command may be.
pub const MAX_COMMAND: usize = 100;

/// What an older Dots does with a list that has guardrails (guardrails spec §3.2), for
/// `dots-admin` and the Guardrails tab to say before signing.
pub const COMPAT_NOTE: &str = "Dots 0.4.0 and older can't read a member list with guardrails: \
     they keep the last list they accepted, or show the account as Waiting. Update every Sage \
     in the account first.";

/// Why no rule can be saved with the approval of a protected call.
pub const GUARDRAIL_REFUSAL: &str =
    "this is protected by the workspace's guardrails, so it always asks: no rule can allow it";

str_enum!(GitAction {
    PushMain => "push_main",
    ForcePush => "force_push",
    DeleteBranch => "delete_branch",
    Tags => "tags",
    MergePr => "merge_pr",
    Release => "release",
});

impl GitAction {
    /// The action as a refusal names it: "<label> is protected in the ... workspace".
    pub fn label(&self) -> &'static str {
        match self {
            Self::PushMain => "pushing to main or master",
            Self::ForcePush => "force pushing",
            Self::DeleteBranch => "deleting a branch",
            Self::Tags => "making or pushing tags",
            Self::MergePr => "merging a pull request",
            Self::Release => "publishing a GitHub release",
        }
    }
}

/// A workspace's guardrails. Every field is required when a workspace has them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Guardrails {
    pub protected_paths: Vec<String>,
    pub protected_git: Vec<GitAction>,
    pub protected_commands: Vec<String>,
    pub member_max_preset: Preset,
}

/// The default protected commands: `rm -rf` and the common deploy and publish commands.
pub const DEFAULT_COMMANDS: &[&str] = &[
    "rm -rf",
    "terraform apply",
    "terraform destroy",
    "kubectl apply",
    "kubectl delete",
    "helm install",
    "helm upgrade",
    "helm uninstall",
    "docker push",
    "npm publish",
    "cargo publish",
    "vercel --prod",
    "netlify deploy",
    "fly deploy",
    "wrangler deploy",
    "firebase deploy",
];

/// The default protected paths: CI, migrations, infrastructure, env files and lock files.
pub const DEFAULT_PATHS: &[&str] = &[
    ".github/**",
    "**/migrations/**",
    "infra/**",
    "**/*.env*",
    "**/Cargo.lock",
    "**/package-lock.json",
    "**/bun.lock",
    "**/yarn.lock",
    "**/pnpm-lock.yaml",
];

impl Guardrails {
    /// What `--defaults` and "Use defaults" write (guardrails spec §3).
    pub fn defaults() -> Guardrails {
        Guardrails {
            protected_paths: DEFAULT_PATHS.iter().map(|s| s.to_string()).collect(),
            protected_git: GitAction::ALL.to_vec(),
            protected_commands: DEFAULT_COMMANDS.iter().map(|s| s.to_string()).collect(),
            member_max_preset: Preset::Sandboxed,
        }
    }

    /// The guardrails as stored: a bare folder (`deploy`, `secrets/`) becomes `deploy/**`, so it
    /// covers what is in it.
    pub fn normalized(&self) -> Guardrails {
        let mut g = self.clone();
        for p in &mut g.protected_paths {
            let t = p.trim().to_string();
            *p = match t.strip_suffix('/') {
                Some(dir) => format!("{dir}/**"),
                None if !t.contains(['*', '?', '[']) => format!("{t}/**"),
                None => t,
            };
        }
        g
    }

    /// The §3.1 rules, in plain words: the first broken one.
    pub fn check(&self) -> Result<(), String> {
        if self.member_max_preset == Preset::Custom {
            return Err(
                "member_max_preset must be read-only, sandboxed or trusted, not custom".into(),
            );
        }
        let mut git = HashSet::new();
        for action in &self.protected_git {
            if !git.insert(action) {
                return Err(format!("protected_git lists '{}' twice", action.as_str()));
            }
        }
        check_list("protected_paths", &self.protected_paths, check_path)?;
        check_list(
            "protected_commands",
            &self.protected_commands,
            check_command,
        )
    }
}

fn check_list(
    field: &str,
    items: &[String],
    each: fn(&str) -> Result<(), String>,
) -> Result<(), String> {
    if items.len() > MAX_ENTRIES {
        return Err(format!("{field} has more than {MAX_ENTRIES} entries"));
    }
    let mut seen = HashSet::new();
    for item in items {
        if item.trim().is_empty() {
            return Err(format!("{field} has an empty entry"));
        }
        each(item).map_err(|why| format!("{field} entry '{item}' {why}"))?;
        if !seen.insert(item.trim().to_lowercase()) {
            return Err(format!("{field} lists '{item}' twice"));
        }
    }
    Ok(())
}

fn check_path(path: &str) -> Result<(), String> {
    if path.chars().count() > MAX_PATH {
        return Err(format!("is longer than {MAX_PATH} characters"));
    }
    if let Some(c) = path
        .chars()
        .find(|c| c.is_control() || ['(', ')', ','].contains(c))
    {
        return Err(format!("may not hold '{c}'"));
    }
    let p = path.trim().replace('\\', "/");
    let b = p.as_bytes();
    let drive = b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':';
    if p.starts_with('/') || drive || p.starts_with('~') {
        return Err("must be relative to the sol's folder".into());
    }
    if p.split('/').any(|c| c == "..") {
        return Err("may not hold '..'".into());
    }
    Ok(())
}

fn check_command(command: &str) -> Result<(), String> {
    if command.chars().count() > MAX_COMMAND {
        return Err(format!("is longer than {MAX_COMMAND} characters"));
    }
    match command
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || " -_./=:@+".contains(*c)))
    {
        Some(c) => Err(format!(
            "may only hold letters, digits, spaces and -_./=:@+, not '{c}'"
        )),
        None => Ok(()),
    }
}

/// Why `policy` reaches past `max`, the highest preset a Member's dot may use (guardrails spec
/// §4.3), as "this policy is Trusted" or "it allows Bash without asking"; `None` when it stays
/// within it.
pub fn over_ceiling(policy: &Policy, max: Preset) -> Option<String> {
    policy.past_ceiling(max)
}

/// `over_ceiling` for a dot that may also read the person's own Claude Code settings
/// (`use_user_settings`): those can add hooks and rules Dots never sees, so they are past every
/// ceiling.
pub fn over_ceiling_spec(policy: &Policy, own_settings: bool, max: Preset) -> Option<String> {
    if own_settings {
        return Some("it uses your own Claude Code settings".into());
    }
    over_ceiling(policy, max)
}

/// How the guardrails treat this PC's dots in a workspace (guardrails spec §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GuardRole {
    /// Protected calls are refused; the policy ceiling holds.
    Member,
    /// Protected calls always ask.
    Admin,
}

/// The guardrails a dot runs under on this PC: the role they treat it with, the workspace's
/// guardrails and the workspace's name for the texts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DotRails {
    pub role: GuardRole,
    pub guardrails: Guardrails,
    pub workspace_name: String,
}

impl DotRails {
    /// What a call hits, or `None` (`policy::rails::check`).
    pub fn hit(&self, tool: &str, input: &Value, workspace: Option<&Path>) -> Option<RailHit> {
        rails::check_as(&self.guardrails, self.role, tool, input, workspace)
    }

    /// The refusal of a protected call on a Member's dot.
    pub fn needs_admin(&self, hit: &RailHit) -> String {
        format!(
            "This needs an Admin: {} is protected in the {} workspace.",
            hit.what, self.workspace_name
        )
    }

    /// What an Admin's approval card says about a protected call.
    pub fn always_asks(&self, hit: &RailHit) -> String {
        format!(
            "{} is protected in the {} workspace, so it always asks.",
            capitalized(&hit.what),
            self.workspace_name
        )
    }

    /// The ceiling a Member's dot runs under; `None` for an Admin's.
    pub fn ceiling(&self) -> Option<Preset> {
        (self.role == GuardRole::Member).then_some(self.guardrails.member_max_preset)
    }

    /// Why `policy` may not be used here, in the words a save is refused with; `None` when it
    /// may.
    pub fn ceiling_refusal(
        &self,
        account: &str,
        policy: &Policy,
        own_settings: bool,
    ) -> Option<String> {
        let max = self.ceiling()?;
        let why = over_ceiling_spec(policy, own_settings, max)?;
        Some(format!(
            "{account}: a Member's sol in the {} workspace may use up to {}, and {why}. Ask an \
             Admin or Owner.",
            self.workspace_name,
            preset_label(max)
        ))
    }

    /// The hold text of a dot whose policy is past the ceiling (guardrails spec §4.3).
    pub fn ceiling_hold(
        &self,
        account: &str,
        policy: &Policy,
        own_settings: bool,
    ) -> Option<String> {
        let max = self.ceiling()?;
        let why = over_ceiling_spec(policy, own_settings, max)?;
        Some(format!(
            "{account}: this sol's tool policy is past what a Member may use in the {} \
             workspace ({why}). Change the policy, or ask an Admin.",
            self.workspace_name
        ))
    }
}

/// A preset as people read it.
pub fn preset_label(preset: Preset) -> &'static str {
    match preset {
        Preset::ReadOnly => "Read-only",
        Preset::Sandboxed => "Sandboxed",
        Preset::Trusted => "Trusted",
        Preset::Custom => "Custom",
    }
}

fn capitalized(text: &str) -> String {
    let mut c = text.chars();
    match c.next() {
        Some(first) => first.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// How the guardrails treat this PC in the workspace `workspace_id` of `org_id` (guardrails spec
/// §4): `None` in Personal, in an account with no member list, and where the workspace has no
/// guardrails. An Owner or Admin is `Admin`; everyone else, a Member and anyone who may not run
/// dots there, is `Member` (fails closed).
pub fn guard_role(
    orgs: Option<&Orgs>,
    org_id: Option<&str>,
    workspace_id: &str,
) -> Option<GuardRole> {
    match access_in(orgs, org_id, workspace_id) {
        Access::Owner | Access::Legacy => None,
        Access::Role(Role::Owner | Role::Admin) => Some(GuardRole::Admin),
        _ => Some(GuardRole::Member),
    }
}

/// What this PC knows of the guardrails over a workspace (guardrails spec §4.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RailsState {
    /// None: Personal, an account with no member list, a workspace without guardrails, or an
    /// Owner's or Admin's place where they don't apply.
    Free,
    Rails(DotRails),
    /// The account is joined, but its member list or the workspace on it can't be read, so the
    /// guardrails are unknown: its dots don't run (the text says why).
    Unknown(String),
    /// The account isn't joined on this PC: its dots are refused elsewhere.
    NotJoined,
}

/// What the accepted member list says of a workspace's guardrails (`Orgs::workspace_guard`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardLookup {
    None,
    Rails(Guardrails),
    Unknown(String),
    NotJoined,
}

/// The guardrails over the workspace `workspace_id` of `org_id` on this PC.
pub fn rails_state(orgs: Option<&Orgs>, org_id: Option<&str>, workspace_id: &str) -> RailsState {
    let (Some(org), Some(service)) = (org_id, orgs) else {
        return RailsState::Free;
    };
    let guardrails = match service.workspace_guard(org, workspace_id) {
        GuardLookup::None => return RailsState::Free,
        GuardLookup::Unknown(reason) => return RailsState::Unknown(reason),
        GuardLookup::NotJoined => return RailsState::NotJoined,
        GuardLookup::Rails(g) => g,
    };
    match guard_role(orgs, org_id, workspace_id) {
        Some(role) => RailsState::Rails(DotRails {
            role,
            guardrails,
            workspace_name: workspace_name(orgs, org, workspace_id),
        }),
        None => RailsState::Free,
    }
}

/// The guardrails the workspace `workspace_id` of `org_id` puts on this PC's dots, or `None`
/// (also when they are unknown: see `enforced_rails`).
pub fn rails_in(orgs: Option<&Orgs>, org_id: Option<&str>, workspace_id: &str) -> Option<DotRails> {
    match rails_state(orgs, org_id, workspace_id) {
        RailsState::Rails(r) => Some(r),
        _ => None,
    }
}

/// The guardrails every call of `dot` is checked against: its workspace's, or, when they are
/// unknown, the defaults as for a Member (fails closed).
pub fn enforced_rails(orgs: Option<&Orgs>, dot: &crate::model::Dot) -> Option<DotRails> {
    match rails_state(orgs, dot.org_id.as_deref(), &dot.workspace_id) {
        RailsState::Free => None,
        RailsState::Rails(r) => Some(r),
        RailsState::Unknown(_) | RailsState::NotJoined => Some(DotRails {
            role: GuardRole::Member,
            guardrails: Guardrails::defaults(),
            workspace_name: dot
                .org_id
                .as_deref()
                .map(|org| workspace_name(orgs, org, &dot.workspace_id))
                .unwrap_or_else(|| dot.workspace_id.clone()),
        }),
    }
}

/// Why `dot` doesn't run: the guardrails of its workspace are unknown on this PC. `None` when
/// they are known (or none apply).
pub fn unknown_rails(
    orgs: Option<&Orgs>,
    account: &str,
    dot: &crate::model::Dot,
) -> Option<String> {
    match rails_state(orgs, dot.org_id.as_deref(), &dot.workspace_id) {
        RailsState::Unknown(reason) => Some(format!(
            "{account}: Sage can't read the guardrails of this sol's workspace ({reason}), so it \
             doesn't run it. Refresh the account, or ask an Admin."
        )),
        _ => None,
    }
}

/// `rails_in` the account and workspace of `dot`.
pub fn dot_rails(orgs: Option<&Orgs>, dot: &crate::model::Dot) -> Option<DotRails> {
    rails_in(orgs, dot.org_id.as_deref(), &dot.workspace_id)
}

/// What an approval card shows about a call the workspace's guardrails protect (guardrails spec
/// §7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApprovalGuardrail {
    /// What the call hits, as "the file migrations/x.sql" or "pushing to main or master".
    pub what: String,
    pub workspace_name: String,
    /// This PC is a Member in the dot's workspace: it can't approve the call.
    pub needs_admin: bool,
    /// The card's words.
    pub text: String,
}

impl ApprovalGuardrail {
    /// The card of `hit` under `rails`.
    pub fn new(rails: &DotRails, hit: &RailHit) -> ApprovalGuardrail {
        let needs_admin = rails.role == GuardRole::Member;
        ApprovalGuardrail {
            what: hit.what.clone(),
            workspace_name: rails.workspace_name.clone(),
            needs_admin,
            text: if needs_admin {
                rails.needs_admin(hit)
            } else {
                rails.always_asks(hit)
            },
        }
    }
}

/// One preset in the dot editor, and why this PC may not pick it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PresetCeiling {
    pub preset: Preset,
    pub refusal: Option<String>,
}

/// The policy ceiling of a dot, or of the workspace a new dot goes in (guardrails spec §4.3,
/// §7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PolicyCeiling {
    /// How the guardrails treat this PC's dots there; `None` when there are none.
    pub applies_as: Option<GuardRole>,
    pub workspace_name: Option<String>,
    /// The highest preset this PC's dots may use there; `None`: no ceiling.
    pub max_preset: Option<Preset>,
    /// Read-only, Sandboxed, Trusted and Custom, each with why it may not be picked. Custom's is
    /// always `None`: a custom policy is judged by `policy_refusal`.
    pub presets: Vec<PresetCeiling>,
    /// Why the policy given (or the dot's) can't be saved.
    pub policy_refusal: Option<String>,
    /// For a dot: the text its runs are held with.
    pub held: Option<String>,
    /// What the workspace protects.
    pub protected: Option<Guardrails>,
}

impl PolicyCeiling {
    /// The ceiling under `rails` (`None`: no guardrails) in the account named `account`, for
    /// `policy` when one is given; `held` is filled for a dot.
    pub fn new(
        rails: Option<&DotRails>,
        account: &str,
        policy: Option<&Policy>,
        own_settings: bool,
        dot: bool,
    ) -> PolicyCeiling {
        let presets = [
            Preset::ReadOnly,
            Preset::Sandboxed,
            Preset::Trusted,
            Preset::Custom,
        ]
        .into_iter()
        .map(|preset| PresetCeiling {
            preset,
            refusal: match (rails, preset) {
                (Some(r), p) if p != Preset::Custom => {
                    r.ceiling_refusal(account, &Policy::preset(p), false)
                }
                _ => None,
            },
        })
        .collect();
        PolicyCeiling {
            applies_as: rails.map(|r| r.role),
            workspace_name: rails.map(|r| r.workspace_name.clone()),
            max_preset: rails.and_then(DotRails::ceiling),
            presets,
            policy_refusal: rails
                .zip(policy)
                .and_then(|(r, p)| r.ceiling_refusal(account, p, own_settings)),
            held: rails
                .zip(policy)
                .filter(|_| dot)
                .and_then(|(r, p)| r.ceiling_hold(account, p, own_settings)),
            protected: rails.map(|r| r.guardrails.clone()),
        }
    }
}
