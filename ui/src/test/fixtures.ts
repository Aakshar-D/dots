// Test data: the Rust-written contract samples, parsed with the UI's own schemas.
import contract from "@/api/__fixtures__/contract.json";
import * as T from "@/api/types";

import { backend } from "./backend";

export const sample = {
  spec: T.DotSpec.parse(contract.DotSpec),
  dot: T.Dot.parse(contract.Dot),
  run: T.Run.parse(contract.Run),
  event: T.RunEventRecord.parse(contract.RunEventRecord),
  approval: T.Approval.parse(contract.Approval),
  /// What the core offers to save with `approval`, narrowest first.
  rules: T.RuleCandidate.array().parse(contract.RuleCandidate),
  status: T.Status.parse(contract.Status),
  settings: T.Settings.parse(contract.Settings),
  overview: T.DotOverview.parse(contract.DotOverview),
  /// A stdio server with a changed tool list and a missing variable; a plugin's HTTP server; a
  /// skill with a problem; a plugin with every part.
  library: T.LibraryEntry.array().parse(contract.LibraryEntry),
  /// The same items without the extras of a list entry: what `library_add` answers.
  libraryItems: T.LibraryItem.array().parse(contract.LibraryItem),
  scannedSkills: T.ScannedSkill.array().parse(contract.ScannedSkill),
  contextDocuments: T.ContextDocuments.parse(contract.ContextDocuments),
  installedPlugins: T.InstalledPlugin.array().parse(contract.InstalledPlugin),
  probe: T.McpProbe.parse(contract.McpProbe),
  inbox: T.Inbox.parse(contract.Inbox),
  /// Holding two runs at 64% of the 5-hour window, then nothing held.
  limits: T.LimitsState.array().parse(contract.LimitsState),
  /// A week: nightly-tests (Claude, heaviest) and local-summarizer (local, no cost).
  usage: T.UsageSummary.parse(contract.UsageSummary),
  pageStates: T.PageState.array().parse(contract.PageState),
  diff: T.WorktreeDiff.parse(contract.WorktreeDiff),
  startupError: T.StartupError.parse(contract.StartupError),
  /// One schedule of each kind: once; every with an anchor and a count; every without an
  /// anchor; daily until a date; weekly; cron.
  schedules: T.Schedule.array().parse(contract.Schedule),
  /// An open question with the choices "yes" and "no", an answered one, a dismissed one, and a
  /// read note of Dots' own.
  messages: T.Message.array().parse(contract.Message),
  /// A pending follow-up and a fired one.
  followUps: T.FollowUp.array().parse(contract.FollowUp),
  /// A pinned memory the dot saved in a run, and one the person wrote.
  memories: T.Memory.array().parse(contract.Memory),
  /// A folder watcher that is on, an email one paused on an error, a GitHub one paused on a flood.
  watchers: T.Watcher.array().parse(contract.Watcher),
  /// This install's Dots ID, with a name and email set.
  identity: T.Identity.parse(contract.Identity),
  /// Personal's default workspace.
  scope: T.CurrentScope.parse(contract.CurrentScope),
  /// Acme Engineering (OpenBao) in each status but pending, then Design (SOPS), pending.
  orgs: T.OrgSummary.array().parse(contract.OrgSummary),
  /// A code for a new org (OpenBao, token), and one that changes Acme's path and policy key.
  joinPreviews: T.JoinPreview.array().parse(contract.JoinPreview),
  /// github's changed list in Personal, then in Acme Engineering.
  toolsChanged: T.ToolsChanged.array().parse(contract.ToolsChanged),
  /// GITHUB_TOKEN (a secret nightly-tests is granted), LOG_LEVEL (plain, not granted), and
  /// NPM_TOKEN, a secret whose value is gone.
  env: T.EnvEntry.array().parse(contract.EnvEntry),
  /// Acme Engineering's API_BASE_URL (plain, locked, granted to acme-release) and GITHUB_TOKEN
  /// (a secret, not locked, not granted).
  orgEntries: T.OrgEntry.array().parse(contract.OrgEntry),
  /// acme-release's grants in Acme Engineering: GITHUB_TOKEN from the org, LOG_LEVEL from
  /// Personal, API_KEY not in the org, PROD_DB_URL denied, GITHUB_TOKEN refused to the shell;
  /// then a grant of a dot whose org was removed.
  grantStatuses: T.GrantStatus.array().parse(contract.GrantStatus),
  /// Acme Engineering's member list (#3): Olu Owner, and Asha Rao (Admin, this PC and one
  /// more), Ben's open invite and Cy's used one, and a draft that removes Ben.
  members: T.MembersView.parse(contract.MembersView),
  /// Ben's new invite and the code that carries it.
  createdInvite: T.CreatedInvite.parse(contract.CreatedInvite),
  /// Ben's request answering his invite; then one with no invite whose email is Asha's, with a
  /// problem.
  requestPreviews: T.RequestPreview.array().parse(contract.RequestPreview),
};

/// What `new_dot_spec` answers: the core's defaults with nothing filled in.
/// What a workspace or a dot refuses when it refuses nothing (`Refusals`, `DotAccess`).
export const allowed: T.Refusals = { edit: null, run: null, answer: null, create_api_keys: null };

export const blankSpec: T.DotSpec = T.DotSpec.parse(contract.NewDotSpec);

/// A status with nothing to warn about.
export const quietStatus: T.Status = {
  ...sample.status,
  claude: { ...sample.status.claude, version: sample.status.claude.tested_version, mismatch: false },
};

/// The answers every screen's frame needs: a started app with nothing to warn about, no dots
/// and an empty inbox. A test replaces what it cares about with `backend.on`.
export function installDefaults(): void {
  backend.on("startup_error", () => null);
  backend.on("status", () => quietStatus);
  backend.on("update_note", () => ({ state: "none" }));
  backend.on("update_status", () => ({
    version: "0.3.0",
    checks_on: true,
    checked_at: null,
    ready: null,
    last_error: null,
    go_back: null,
    auto_install: false,
    publisher: false,
    install_error: null,
  }));
  backend.on("publisher_status", () => ({ token: null, state: { state: "none" } }));
  backend.on("report_activity", () => null);
  backend.on("app_rendered", () => null);
  // No Claude reading yet, nothing held.
  backend.on("limits_get", () => ({
    windows: [],
    five_hour: null,
    weekly: null,
    shares: { enabled: true, five_hour: 60, weekly: 70 },
    holding: false,
    draining: false,
    rejected_until: null,
    held_run_ids: [],
    hold_until: null,
  }));
  backend.on("inbox", () => ({
    approvals: [],
    review_runs: [],
    attention: [],
    tools_changed: [],
    watchers: [], held: [], instructions: [], app_reviews: [], team_scope: [],
  }));
  // No dot proposed instructions.
  backend.on("instruction_proposals_list", () => []);
  // An approval card asks for these when it shows; by default the core offers no rule.
  backend.on("approval_rule_candidates", () => []);
  // A conversation asks for the dot's app proposals; by default it has none.
  backend.on("app_proposals_list", () => []);
  backend.on("dot_overview", () => []);
  backend.on("list_dots", () => []);
  backend.on("get_settings", () => sample.settings);
  backend.on("chatgpt_account", () => ({
    configured: false, signing_in: false, account: null, expires_at: null,
    setup_error: "ChatGPT sign-in needs a registered OpenAI client for Dots.",
  }));
  // Neither Microsoft Edge nor Node.js was found.
  backend.on("browser_status", () => ({ edge: null, npx: null }));
  backend.on("library_list", () => []);
  backend.on("installed_claude_plugins", () => []);
  backend.on("environment_variables", (args) =>
    (args.names as string[]).map((name) => ({ name, set: true })),
  );
  // Personal is empty, and the vault has no one-time offer to make.
  backend.on("env_list", () => []);
  backend.on("env_import_candidates", () => []);
  backend.on("env_old_backups", () => []);
  // No org is joined on this PC.
  backend.on("org_list", () => []);
  backend.on("identity_get", () => sample.identity);
  // Personal, until a test picks an account.
  backend.on("scope_get", () => sample.scope);
  backend.on("scope_set", (args) => ({ account: String(args.account), workspace: String(args.workspace) }));
  // Only the Default workspace, so no workspace list or picker shows; nothing is refused there.
  backend.on("workspace_list", (args) => [
    { scope: String(args.scope), id: "default", name: "Default", use_rules: null, my_role: null, can_manage: true, refusals: allowed },
  ]);
  // No API key is made, and the local server is off.
  backend.on("api_key_list", () => []);
  backend.on("api_base_url", () => {
    throw { kind: "conflict", message: "The local server is not running." };
  });
  // The person may do all with every dot.
  backend.on("dot_access", () => allowed);
  // No grant has a status: the editor shows each as it shows a grant not saved yet.
  backend.on("dot_variables", () => []);
  backend.on("new_dot_spec", () => blankSpec);
  backend.on("list_runs", () => []);
  // No dot has checked in or scheduled anything. A question on screen is marked read.
  backend.on("messages_list", () => []);
  backend.on("follow_ups_list", () => []);
  // No dot remembers anything.
  backend.on("memories_list", () => []);
  backend.on("watchers_list", () => []);
  // Nothing is unread; looking at a dot and saying which one is viewed are taken.
  backend.on("unread_total", () => 0);
  backend.on("mark_dot_seen", () => null);
  backend.on("set_viewing", () => null);
  backend.on("message_mark_read", (args) => ({
    ...sample.messages[0],
    id: String(args.id),
    read_at: sample.run.queued_at,
  }));
  backend.on("webhook_url", (args) => `http://127.0.0.1:47321/dots/${String(args.id)}/trigger`);
  backend.on("preview_schedule", () => [
    "2026-10-13T02:00:00+01:00",
    "2026-10-14T02:00:00+01:00",
    "2026-10-15T02:00:00+01:00",
  ]);
}
