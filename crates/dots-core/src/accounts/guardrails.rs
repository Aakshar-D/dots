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
     they keep the last list they accepted, or show the account as Waiting. Update every Dots \
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
        return Err("must be relative to the dot's folder".into());
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
        rails::check(&self.guardrails, tool, input, workspace)
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
    pub fn ceiling_refusal(&self, account: &str, policy: &Policy) -> Option<String> {
        let max = self.ceiling()?;
        let why = over_ceiling(policy, max)?;
        Some(format!(
            "{account}: a Member's dot in the {} workspace may use up to {}, and {why}. Ask an \
             Admin or Owner.",
            self.workspace_name,
            preset_label(max)
        ))
    }

    /// The hold text of a dot whose policy is past the ceiling (guardrails spec §4.3).
    pub fn ceiling_hold(&self, account: &str, policy: &Policy) -> Option<String> {
        let max = self.ceiling()?;
        let why = over_ceiling(policy, max)?;
        Some(format!(
            "{account}: this dot's tool policy is past what a Member may use in the {} \
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

/// The guardrails the workspace `workspace_id` of `org_id` puts on this PC's dots, or `None`.
pub fn rails_in(orgs: Option<&Orgs>, org_id: Option<&str>, workspace_id: &str) -> Option<DotRails> {
    let org = org_id?;
    let role = guard_role(orgs, org_id, workspace_id)?;
    let guardrails = orgs?.workspace_guardrails(org, workspace_id)?;
    Some(DotRails {
        role,
        guardrails,
        workspace_name: workspace_name(orgs, org, workspace_id),
    })
}

/// `rails_in` the account and workspace of `dot`.
pub fn dot_rails(orgs: Option<&Orgs>, dot: &crate::model::Dot) -> Option<DotRails> {
    rails_in(orgs, dot.org_id.as_deref(), &dot.workspace_id)
}
