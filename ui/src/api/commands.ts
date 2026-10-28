// Typed wrappers over the Tauri commands in apps/dots-app/src/commands.rs. Arguments are
// camelCase here; Tauri maps them to the Rust snake_case parameters. Every answer is parsed
// with its schema, so a shape this UI does not know is an error here, not a broken screen.
import { invoke, type InvokeArgs } from "@tauri-apps/api/core";
import { z } from "zod";

import * as T from "./types";
import type {
  CmdErrorKind,
  DotSpec,
  DraftRequest,
  EntryKind,
  LibraryConfig,
  McpServerConfig,
  Preset,
  Schedule,
  Settings,
  SourceCreds,
} from "./types";

/// A command failure. `kind` mirrors the Rust `CmdError`.
export class CommandError extends Error {
  constructor(
    readonly kind: CmdErrorKind,
    message: string,
  ) {
    super(message);
    this.name = "CommandError";
  }
}

export function toCommandError(e: unknown): CommandError {
  const parsed = T.CmdError.safeParse(e);
  if (parsed.success) return new CommandError(parsed.data.kind, parsed.data.message);
  if (e instanceof Error) return new CommandError("internal", e.message);
  // An error object with a kind this UI does not know still carries its message.
  const message =
    typeof e === "object" && e !== null && "message" in e && typeof e.message === "string"
      ? e.message
      : String(e);
  return new CommandError("internal", message);
}

async function call<S extends z.ZodType>(
  schema: S,
  cmd: string,
  args?: InvokeArgs,
): Promise<z.infer<S>> {
  let answer: unknown;
  try {
    answer = await invoke(cmd, args);
  } catch (e) {
    throw toCommandError(e);
  }
  const parsed = schema.safeParse(answer);
  if (!parsed.success) {
    const issue = parsed.error.issues[0];
    const at = issue.path.join(".") || "the answer";
    throw new CommandError("internal", `${cmd}: unexpected ${at}: ${issue.message}`);
  }
  return parsed.data;
}

// Commands without a result answer `null` (or nothing at all).
const nothing = z
  .null()
  .optional()
  .transform(() => null);
const text = z.string();

export const commands = {
  startupError: () => call(T.StartupError.nullable(), "startup_error"),
  status: () => call(T.Status, "status"),
  chatGptAccount: () => call(T.ChatGptStatus, "chatgpt_account"),
  chatGptSignIn: () => call(T.ChatGptStatus, "chatgpt_sign_in"),
  chatGptSignOut: () => call(T.ChatGptStatus, "chatgpt_sign_out"),
  markClaudeTested: () => call(T.Status, "mark_claude_tested"),
  getSettings: () => call(T.Settings, "get_settings"),
  /// Rejects with kind `cancelled` when the user declines the native confirmation of a new
  /// Claude program; nothing is saved then.
  updateSettings: (settings: Settings) => call(T.SettingsSaved, "update_settings", { settings }),
  setSchedulesPaused: (paused: boolean) => call(T.Status, "set_schedules_paused", { paused }),
  restartApp: () => call(nothing, "restart_app"),
  updateNote: () => call(T.UpdateNote, "update_note"),
  updateOpenNotes: () => call(nothing, "update_open_notes"),
  updateLater: () => call(T.UpdateNote, "update_later"),
  updateRestart: () => call(T.UpdateNote, "update_restart"),
  /// `working`: the dots the confirm named. When another dot is working now, nothing is
  /// installed and the note names the dots working now.
  updateStopAndRestart: (working: string[]) =>
    call(T.UpdateNote, "update_stop_and_restart", { working }),
  updateCancelWait: () => call(T.UpdateNote, "update_cancel_wait"),
  updateStatus: () => call(T.UpdateStatus, "update_status"),
  updateCheckNow: () => call(T.CheckOutcome, "update_check_now"),
  updateReleaseNotes: (version: string) =>
    call(T.ReleaseNotes.nullable(), "update_release_notes", { version }),
  updateRepair: () => call(T.UpdateNote, "update_repair"),
  updateGoBack: () => call(T.UpdateNote, "update_go_back"),
  updateDismissUpdated: () => call(T.UpdateNote, "update_dismiss_updated"),
  /// The window had input, or the composer's text changed (the idle install).
  reportActivity: (composerHasText: boolean) =>
    call(nothing, "report_activity", { composerHasText }),
  /// The window rendered after this start (the health check after an update).
  appRendered: () => call(nothing, "app_rendered"),

  publisherStatus: () => call(T.PublisherStatus, "publisher_status"),
  publisherCheck: () => call(T.PublisherStatus, "publisher_check"),
  releasesPage: () => call(T.PageState, "releases_page"),
  releaseMakeKey: (password: string) => call(z.string(), "release_make_key", { password }),
  /// Opens the native save dialog; the path written, or null when cancelled.
  releaseSaveKey: () => call(z.string().nullable(), "release_save_key"),
  releasePutSecrets: (password: string) => call(nothing, "release_put_secrets", { password }),
  releaseOpenKeyPull: () => call(T.PullOpened, "release_open_key_pull"),
  releasePublishTrial: (version: string) =>
    call(T.Trial, "release_publish_trial", { version }),
  releaseSaveNotes: (version: string, notes: string) =>
    call(nothing, "release_save_notes", { version, notes }),
  releasePromote: (version: string) => call(nothing, "release_promote", { version }),
  releaseRetry: (tag: string) => call(nothing, "release_retry", { tag }),
  releaseDropTrial: (version: string) => call(nothing, "release_drop_trial", { version }),
  releaseWithdraw: (version: string) => call(nothing, "release_withdraw", { version }),

  dotOverview: () => call(T.DotOverview.array(), "dot_overview"),
  listDots: () => call(T.Dot.array(), "list_dots"),
  getDot: (id: string) => call(T.Dot, "get_dot", { id }),
  /// The status of each of the dot's stored grants in its scope: whether it resolves now, from
  /// where, and why not. It holds no value, and no org is fetched for it.
  dotVariables: (dotId: string) => call(T.GrantStatus.array(), "dot_variables", { dotId }),
  /// `orgId`: the joined org the dot belongs to, fixed from then on; null for Personal.
  createDot: (spec: DotSpec, orgId: string | null = null) =>
    call(T.Dot, "create_dot", { spec, orgId }),
  updateDot: (id: string, spec: DotSpec) => call(T.Dot, "update_dot", { id, spec }),
  deleteDot: (id: string) => call(nothing, "delete_dot", { id }),
  setDotEnabled: (id: string, enabled: boolean) =>
    call(T.Dot, "set_dot_enabled", { id, enabled }),
  regenerateWebhookToken: (id: string) => call(text, "regenerate_webhook_token", { id }),
  /// Rejects with kind `conflict` while the local server is not running.
  webhookUrl: (id: string) => call(text, "webhook_url", { id }),
  /// The next `count` slots of a schedule saved now; the core's objection when it is not valid.
  previewSchedule: (schedule: Schedule, count: number) =>
    call(text.array(), "preview_schedule", { schedule, count }),
  policyPreset: (preset: Preset) => call(T.Policy, "policy_preset", { preset }),
  newDotSpec: () => call(T.DotSpec, "new_dot_spec"),
  testEndpoint: (endpoint: string, model: string) =>
    call(text, "test_endpoint", { endpoint, model }),

  runNow: (dotId: string) => call(T.Run, "run_now", { dotId }),
  chat: (dotId: string, message: string, parentRunId: string | null) =>
    call(T.Run, "chat", { dotId, message, parentRunId }),
  cancelRun: (runId: string) => call(T.Run, "cancel_run", { runId }),
  listRuns: (dotId: string | null, limit: number) =>
    call(T.Run.array(), "list_runs", { dotId, limit }),
  getRun: (id: string) => call(T.Run, "get_run", { id }),
  runThread: (runId: string) => call(T.Run.array(), "run_thread", { runId }),
  listRunEvents: (runId: string, afterSeq: number) =>
    call(T.RunEventRecord.array(), "list_run_events", { runId, afterSeq }),
  approvalsForRun: (runId: string) => call(T.Approval.array(), "approvals_for_run", { runId }),
  /// `allowRule`: with an approval, also save this allow rule to the dot's policy. It must be
  /// a pattern that `approvalRuleCandidates` offers for this approval; the core refuses any
  /// other with kind `invalid`, and then decides nothing.
  decideApproval: (id: string, approved: boolean, note: string | null, allowRule: string | null) =>
    call(T.Approval, "decide_approval", { id, approved, note, allowRule }),
  /// The allow rules that can be saved while approving this call, narrowest first. Empty when
  /// the call can only be approved one at a time.
  approvalRuleCandidates: (id: string) =>
    call(T.RuleCandidate.array(), "approval_rule_candidates", { id }),
  /// Gives every pending approval of one run the same answer, in the order they were asked.
  /// Answers the approvals it decided.
  decideRunApprovals: (runId: string, approved: boolean, note: string | null) =>
    call(T.Approval.array(), "decide_run_approvals", { runId, approved, note }),
  inbox: () => call(T.Inbox, "inbox"),
  inboxCount: () => call(z.number().int(), "inbox_count"),
  /// Clears the "Needs attention" of the dot's schedule; nothing happens when it is not set.
  /// Rejects with kind `not_found` when the dot has no schedule.
  dismissScheduleAttention: (dotId: string) =>
    call(nothing, "dismiss_schedule_attention", { dotId }),

  worktreeDiff: (runId: string) => call(T.WorktreeDiff, "worktree_diff", { runId }),
  discardWorktree: (runId: string) => call(T.Run, "discard_worktree", { runId }),
  markReviewed: (runId: string) => call(T.Run, "mark_reviewed", { runId }),
  openRunFolder: (runId: string) => call(nothing, "open_run_folder", { runId }),

  libraryList: () => call(T.LibraryEntry.array(), "library_list"),
  /// `accept`: the fingerprint `libraryProbe` answered. The list the server gives now is accepted
  /// only when it has that fingerprint; with `null` a server is saved unchecked and not started.
  libraryAdd: (name: string, config: LibraryConfig, accept: string | null) =>
    call(T.LibraryItem, "library_add", { name, config, accept }),
  libraryUpdate: (id: string, config: LibraryConfig) =>
    call(T.LibraryItem, "library_update", { id, config }),
  /// Rejects with kind `conflict`, naming the dots, while dots use the item and `confirm` is
  /// false. With `confirm` the item also leaves those dots.
  libraryRemove: (id: string, confirm: boolean) =>
    call(nothing, "library_remove", { id, confirm }),
  /// Starts the server, lists its tools and stops it; nothing is saved.
  libraryProbe: (config: McpServerConfig) => call(T.McpProbe, "library_probe", { config }),
  /// Checks the server in `scope`, an org id; left out, Personal. A server whose URL reaches
  /// an origin not confirmed there asks in a native dialog first; rejects with kind
  /// `cancelled` when the person declines, and nothing is sent.
  libraryCheck: (id: string, scope?: string) =>
    call(T.McpCatalog, "library_check", { id, scope }),
  /// Confirms, in a native dialog, the origin the server reaches in `scope` (left out,
  /// Personal), for a plugin's own server too; asks nothing when it is confirmed already.
  /// Rejects with kind `cancelled` when declined, and with `invalid` when there is no origin to
  /// confirm.
  confirmOrigin: (serverId: string, scope?: string) =>
    call(nothing, "confirm_origin", { serverId, scope }),
  /// Accepts the list seen in `scope` (left out, Personal). Rejects with kind `conflict` when
  /// the list moved since `fingerprint` was seen.
  libraryAccept: (id: string, fingerprint: string, scope?: string) =>
    call(T.McpCatalog, "library_accept", { id, fingerprint, scope }),
  libraryScanSkills: (path: string) =>
    call(T.ScannedSkill.array(), "library_scan_skills", { path }),
  /// Drafts a skill with the claude CLI for the dialog `dialog`, cancelling the dialog's
  /// draft before; answers the SKILL.md text.
  skillDraft: (dialog: string, request: DraftRequest) =>
    call(z.string(), "skill_draft", { dialog, request }),
  skillDraftCancel: (dialog: string) => call(nothing, "skill_draft_cancel", { dialog }),
  /// Rejects with kind `invalid`, saying what to change, when the text cannot be saved.
  skillCheck: (text: string) => call(T.DraftSummary, "skill_check", { text }),
  /// Rejects with kind `conflict` when the name is in the Library or its folder exists.
  skillCreate: (text: string) => call(T.LibraryItem, "skill_create", { text }),
  contextDocuments: (id: string) => call(T.ContextDocuments, "context_documents", { id }),
  installedClaudePlugins: () => call(T.InstalledPlugin.array(), "installed_claude_plugins"),
  environmentVariables: (names: string[]) =>
    call(T.EnvVariable.array(), "environment_variables", { names }),
  /// The personal variables; a secret's value is never in the answer.
  envList: () => call(T.EnvEntry.array(), "env_list"),
  /// Rejects with kind `conflict` when the name is taken in any case.
  envAdd: (name: string, kind: EntryKind, value: string) =>
    call(T.EnvEntry, "env_add", { name, kind, value }),
  /// Edits a plain value, or replaces a secret's.
  envSet: (name: string, value: string) => call(T.EnvEntry, "env_set", { name, value }),
  envRemove: (name: string) => call(nothing, "env_remove", { name }),
  /// Names Library servers read that Windows sets and Personal lacks; never a value. Empty once
  /// the offer is answered.
  envImportCandidates: () => call(z.string().array(), "env_import_candidates"),
  /// Copies them into Personal as secrets and answers the offer; answers the names imported.
  envImport: (names: string[]) => call(z.string().array(), "env_import", { names }),
  envImportDismiss: () => call(nothing, "env_import_dismiss"),
  /// Update backups made before the vault, by file name; empty once the offer is answered.
  envOldBackups: () => call(z.string().array(), "env_old_backups"),
  /// Deletes them when `deleteThem`, and answers the offer; answers the file names that could
  /// not be deleted, and then the offer stays.
  envOldBackupsAnswer: (deleteThem: boolean) =>
    call(z.string().array(), "env_old_backups_answer", { delete: deleteThem }),

  /// This install's Dots ID, with the name and email the person set.
  identityGet: () => call(T.Identity, "identity_get"),
  /// Sets the name and email; rejects with the core's message for one it refuses.
  profileSet: (name: string, email: string) => call(T.Identity, "profile_set", { name, email }),
  /// The account and workspace the person works in now.
  scopeGet: () => call(T.CurrentScope, "scope_get"),
  scopeSet: (account: string, workspace: string) =>
    call(T.CurrentScope, "scope_set", { account, workspace }),

  /// The orgs joined on this PC, by name. Never a value.
  orgList: () => call(T.OrgSummary.array(), "org_list"),
  /// What a join code (its text, or a `.dotsorg` file's) would join or change; nothing is
  /// fetched. Rejects with kind `invalid` for a code Dots cannot read.
  orgPreview: (code: string) => call(T.JoinPreview, "org_preview", { code }),
  /// This PC's age public key for a SOPS org, made on the first call, to send to the admin.
  orgSopsKey: (orgId: string) => call(text, "org_sops_key", { orgId }),
  /// Joins with the person's sign-in details; null for a SOPS org, which uses this PC's key.
  /// Rejects with kind `conflict` for an org joined already.
  orgJoin: (code: string, creds: SourceCreds | null) =>
    call(T.OrgSummary, "org_join", { code, creds }),
  /// Replaces the org's settings with a new code's, with new sign-in details or the saved ones
  /// (null). A new policy key asks in a native dialog first: rejects with kind `cancelled` when
  /// declined, and nothing changes.
  orgUpdate: (orgId: string, code: string, creds: SourceCreds | null = null) =>
    call(T.OrgSummary, "org_update", { orgId, code, creds }),
  /// Fetches the org now. A failed fetch is not a rejection: the answer's status says it.
  orgRefresh: (orgId: string) => call(T.OrgSummary, "org_refresh", { orgId }),
  /// Leaves the org; its dots stay, "org removed". The UI confirms first.
  orgRemove: (orgId: string) => call(nothing, "org_remove", { orgId }),
  /// The org's entries for the Env tab; a secret's value is never in the answer.
  orgEntries: (orgId: string) => call(T.OrgEntry.array(), "org_entries", { orgId }),

  /// Every message that waits (open questions, unread notes) and the newest others, newest
  /// first; of one dot with `dotId`.
  messagesList: (dotId: string | null) => call(T.Message.array(), "messages_list", { dotId }),
  /// Answers a dot's question; the run that continues its thread. Rejects with kind `conflict`
  /// when the question is closed or its thread is still busy, in words to show as they are.
  messageReply: (id: string, text: string) => call(T.Run, "message_reply", { id, text }),
  messageDismiss: (id: string) => call(T.Message, "message_dismiss", { id }),
  messageMarkRead: (id: string) => call(T.Message, "message_mark_read", { id }),
  /// The pending follow-ups, soonest first; of one dot with `dotId`.
  followUpsList: (dotId: string | null) =>
    call(T.FollowUp.array(), "follow_ups_list", { dotId }),
  /// Rejects with kind `conflict` once the follow-up fired, was skipped or was cancelled.
  followUpCancel: (id: string) => call(T.FollowUp, "follow_up_cancel", { id }),

  /// "Open the browser": the dot's Edge, for the person to log in ahead of time. Rejects with
  /// the reason when Edge cannot be opened.
  browserOpen: (dotId: string) => call(nothing, "browser_open", { dotId }),
  /// Where Microsoft Edge and Node.js (npx) were found.
  browserStatus: () => call(T.BrowserStatus, "browser_status"),
  /// A screenshot of a run's browser as a `data:` URL, by the name Playwright gave its file.
  browserScreenshot: (runId: string, name: string) =>
    call(text, "browser_screenshot", { runId, name }),

  /// The person looked at the dot's conversation: its unread count drops to what needs them.
  /// Rejects with kind `not_found` for a dot that is gone.
  markDotSeen: (dotId: string) => call(nothing, "mark_dot_seen", { dotId }),
  /// The dot whose conversation is open in the focused window, or null: nothing notifies about
  /// it, and what it does meanwhile is not new.
  setViewing: (dotId: string | null) => call(nothing, "set_viewing", { dotId }),
  /// The sum of every dot's unread count, for the Home link.
  unreadTotal: () => call(z.number().int(), "unread_total"),

  /// What the dot remembers, pinned first, then newest.
  memoriesList: (dotId: string) => call(T.Memory.array(), "memories_list", { dotId }),
  /// The person's memory of the dot. Rejects with the core's sentence (kind `invalid` for a
  /// blank, too long or secret text, `conflict` for a duplicate or the 200th).
  memoryAdd: (dotId: string, text: string) => call(T.Memory, "memory_add", { dotId, text }),
  /// Changes a memory's text and/or its pin; null leaves it as it is.
  memoryUpdate: (id: string, text: string | null, pinned: boolean | null) =>
    call(T.Memory, "memory_update", { id, text, pinned }),
  memoryDelete: (id: string) => call(nothing, "memory_delete", { id }),

  /// The installed apps, with the dots that use each and the error that keeps one out of runs.
  appsList: () => call(T.AppRow.array(), "apps_list"),
  /// A call from the app's screen, as the person. Rejects with kind `invalid` and the rule the
  /// call broke (an unlisted tool, a bad input, the code's error).
  appCall: (name: string, tool: string, input: unknown) =>
    call(z.unknown(), "app_call", { name, tool, input }),
  /// The active version's files.
  appFiles: (name: string) => call(T.AppFile.array(), "app_files", { name }),
  /// The newest logged calls (at most 500), newest first.
  appCalls: (name: string, limit: number) => call(T.AppCall.array(), "app_calls", { name, limit }),
  appSetEveryDot: (name: string, on: boolean) =>
    call(T.LibraryItem, "app_set_every_dot", { name, on }),
  /// "Can start dots"; rejects for an app that does not ask for it.
  appSetCanStartDots: (name: string, on: boolean) =>
    call(T.LibraryItem, "app_set_can_start_dots", { name, on }),
  appRollBack: (name: string) => call(T.LibraryItem, "app_roll_back", { name }),
  /// `confirm` is the person's yes to "dots use it"; `deleteData` also deletes its data.
  appRemove: (name: string, confirm: boolean, deleteData: boolean) =>
    call(nothing, "app_remove", { name, confirm, deleteData }),
  /// The apps dots proposed, of one dot or of all (null), newest first.
  appProposalsList: (dotId: string | null) =>
    call(T.AppProposal.array(), "app_proposals_list", { dotId }),
  /// A waiting proposal's files; rejects with kind `not_found` once it is decided.
  appProposalFiles: (id: string) => call(T.AppFile.array(), "app_proposal_files", { id }),
  /// The Code tab's diff: against the active version for a change, every file for a new app.
  appProposalDiff: (id: string) => call(T.WorktreeDiff, "app_proposal_diff", { id }),
  /// Opens the card's preview on a throwaway database; answers its id. Rejects with kind
  /// `conflict` once the proposal is decided.
  appPreviewOpen: (id: string) => call(z.string(), "app_preview_open", { id }),
  /// A call from the preview's frame, as the person. Rejects with kind `invalid` and the rule
  /// the call broke, `not_found` once the preview is closed.
  appPreviewCall: (preview: string, tool: string, input: unknown) =>
    call(z.unknown(), "app_preview_call", { preview, tool, input }),
  appPreviewClose: (preview: string) => call(nothing, "app_preview_close", { preview }),
  /// The data removed apps kept, newest first.
  appsKeptList: () => call(T.KeptData.array(), "apps_kept_list"),
  appKeptDelete: (id: string) => call(nothing, "app_kept_delete", { id }),
  /// The person's answer on the card. Rejects with kind `conflict` when it was decided already
  /// or its app moved on, and while the dot's thread is still working.
  appProposalDecide: (id: string, decision: T.ProposalDecision) =>
    call(T.AppProposal, "app_proposal_decide", { id, decision }),

  watchersList: (dotId: string) => call(T.Watcher.array(), "watchers_list", { dotId }),
  watcherAdd: (dotId: string, name: string, config: T.WatcherConfig) =>
    call(T.Watcher, "watcher_add", { dotId, name, config }),
  watcherUpdate: (id: string, name: string, config: T.WatcherConfig) =>
    call(T.Watcher, "watcher_update", { id, name, config }),
  watcherDelete: (id: string) => call(nothing, "watcher_delete", { id }),
  watcherSetEnabled: (id: string, enabled: boolean) =>
    call(T.Watcher, "watcher_set_enabled", { id, enabled }),
  watcherResume: (id: string) => call(T.Watcher, "watcher_resume", { id }),
  // Claude's usage limits and the Usage page (usage and limits spec section 6).
  limitsGet: () => call(T.LimitsState, "limits_get"),
  usageSummary: (period: T.UsagePeriod) => call(T.UsageSummary, "usage_summary", { period }),
  runReleaseHold: (runId: string) => call(T.Run, "run_release_hold", { runId }),
};
