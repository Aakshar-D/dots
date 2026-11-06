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
  EngineKind,
  EntryKind,
  InboxKey,
  IntentFields,
  LibraryConfig,
  McpServerConfig,
  Policy,
  Preset,
  Schedule,
  Settings,
  SetupProposal,
  SetupRequest,
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

/// A command whose argument is raw bytes, with `headers` beside them (`tauri::ipc::Request`):
/// a file's bytes never travel as a JSON list of numbers.
async function callRaw<S extends z.ZodType>(
  schema: S,
  cmd: string,
  bytes: Uint8Array,
  headers: Record<string, string>,
): Promise<z.infer<S>> {
  let answer: unknown;
  try {
    answer = await invoke(cmd, bytes, { headers });
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
  /// The problem report's text: what is previewed, copied and saved.
  problemReport: (description: string, includeLogs: boolean) =>
    call(z.string(), "problem_report", { description, includeLogs }),
  /// Saves `text` where the person chooses in the native save dialog; the path, or null.
  saveProblemReport: (text: string) =>
    call(z.string().nullable(), "save_problem_report", { text }),
  /// Settings › Share usage counts (usage counts spec).
  usageCountsState: () => call(T.UsageCountsState, "usage_counts_state"),
  /// Turns the usage counts on or off; `url`, when given, is the new destination.
  setUsageCounts: (enabled: boolean, url: string | null) =>
    call(T.UsageCountsState, "set_usage_counts", { enabled, url }),
  resetUsageCountsId: () => call(T.UsageCountsState, "reset_usage_counts_id"),
  /// The next report, exactly as it would be posted.
  usageCountsPreview: () => call(T.UsageCountsReport, "usage_counts_preview"),
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
  /// `workspaceId`: the workspace of that account it starts in; null for the default one.
  createDot: (spec: DotSpec, orgId: string | null = null, workspaceId: string | null = null) =>
    call(T.Dot, "create_dot", { spec, orgId, workspaceId }),
  /// The Claude Code agents on this PC, each with the dot it would become.
  scanClaudeAgents: () => call(T.ScannedAgent.array(), "scan_claude_agents"),
  /// Makes a dot of each agent file in `paths` (as a fresh scan finds them), in the account
  /// `orgId` (null for Personal) and its workspace `workspaceId` (null for the default one).
  importClaudeAgents: (
    paths: string[],
    orgId: string | null = null,
    workspaceId: string | null = null,
  ) => call(T.AgentImport, "import_claude_agents", { paths, orgId, workspaceId }),
  /// Asks the setup agent for the setup view `id`: its questions, or a checked proposal. A newer
  /// request of the same id cancels the one before.
  setupAsk: (id: string, request: SetupRequest) =>
    call(T.SetupAnswer, "setup_ask", { id, request }),
  setupCancel: (id: string) => call(nothing, "setup_cancel", { id }),
  /// Creates the proposed dot and starts its first chat. `orgId`, `workspaceId` as `createDot`.
  setupApply: (
    proposal: SetupProposal,
    orgId: string | null = null,
    workspaceId: string | null = null,
  ) => call(T.SetupApplied, "setup_apply", { proposal, orgId, workspaceId }),
  /// Moves a dot to another workspace of its account.
  moveDot: (dotId: string, workspaceId: string) =>
    call(T.Dot, "dot_move", { dotId, workspaceId }),
  /// What this PC may not do with the dot (change, run, answer its approvals), in the core's
  /// words; null is allowed.
  dotAccess: (dotId: string) => call(T.DotAccess, "dot_access", { dotId }),
  /// The workspaces of `scope` (`personal`, or an org's id) this PC may see.
  workspaceList: (scope: string) => call(T.WorkspaceView.array(), "workspace_list", { scope }),
  personalWorkspaceAdd: (name: string) =>
    call(T.WorkspaceView, "personal_workspace_add", { name }),
  personalWorkspaceRename: (id: string, name: string) =>
    call(nothing, "personal_workspace_rename", { id, name }),
  personalWorkspaceDelete: (id: string) => call(nothing, "personal_workspace_delete", { id }),
  /// The API keys of the workspaces of `scope` this PC may see. Never carries a key's text.
  apiKeyList: (scope: string) => call(T.ApiKeyView.array(), "api_key_list", { scope }),
  /// Makes a key. The answer carries its text, the only time it exists: it is returned to the
  /// caller and held nowhere else.
  apiKeyCreate: (scope: string, workspaceId: string, name: string) =>
    call(T.CreatedApiKey, "api_key_create", { scope, workspaceId, name }),
  apiKeyRevoke: (id: string) => call(T.ApiKeyView, "api_key_revoke", { id }),
  /// The address the API answers on; rejects when the local server is not running.
  apiBaseUrl: () => call(text, "api_base_url", {}),
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
  /// For each rule pattern, whether it names or covers a private folder: such an allow rule
  /// always asks.
  rulesTouchPrivate: (patterns: string[]) =>
    call(z.boolean().array(), "rules_touch_private", { patterns }),
  /// The company's Claude Code managed settings (managed policy).
  managedPolicy: () => call(T.ManagedPolicy, "managed_policy"),
  /// The same, with where this policy of a dot on `engine` would skip what they keep.
  managedPolicyCheck: (engine: EngineKind, policy: Policy) =>
    call(T.ManagedCheck, "managed_policy_check", { engine, policy }),
  /// A blank dot; its model is the default model of Settings.
  newDotSpec: () => call(T.DotSpec, "new_dot_spec"),
  /// The models a dot of `engine` can pick: the Claude list, or what the local `endpoint`
  /// lists. Rejects with the reason an endpoint's list could not be read.
  listModels: (engine: EngineKind, endpoint: string | null) =>
    call(T.ModelOption.array(), "list_models", { engine, endpoint }),
  testEndpoint: (endpoint: string, model: string) =>
    call(text, "test_endpoint", { endpoint, model }),

  runNow: (dotId: string) => call(T.Run, "run_now", { dotId }),
  /// `attachmentIds`: staged files the message carries (attachments spec section 4.3); only
  /// sent when there are some.
  chat: (dotId: string, message: string, parentRunId: string | null, attachmentIds: string[] = []) =>
    call(
      T.Run,
      "chat",
      attachmentIds.length > 0
        ? { dotId, message, parentRunId, attachmentIds }
        : { dotId, message, parentRunId },
    ),
  /// Stages a picked or dropped file for a chat message. Rejects with kind `invalid` and the
  /// reason for a folder, a link, a file over 25 MB or one that cannot be read.
  attachmentAddPath: (dotId: string, path: string) =>
    call(T.StagedAttachment, "attachment_add_path", { dotId, path }),
  /// Stages pasted bytes named `name`, sent raw with the dot and the name as headers.
  attachmentAddBytes: (dotId: string, name: string, bytes: Uint8Array) =>
    callRaw(T.StagedAttachment, "attachment_add_bytes", bytes, {
      "x-dots-dot": dotId,
      "x-dots-name": encodeURIComponent(name),
    }),
  /// Removes a staged file; rejects with `conflict` once a message took it.
  attachmentRemove: (id: string) => call(nothing, "attachment_remove", { id }),
  /// The Files panel's rows for the topics `rootRunIds` of the dot, newest first.
  conversationFiles: (dotId: string, rootRunIds: string[]) =>
    call(T.ConversationFile.array(), "conversation_files", { dotId, rootRunIds }),
  /// Opens a listed file; rejects with `not_found` when it is gone and `invalid` for a program.
  conversationFileOpen: (dotId: string, rootRunIds: string[], path: string) =>
    call(nothing, "conversation_file_open", { dotId, rootRunIds, path }),
  /// Shows a listed file in its folder.
  conversationFileReveal: (dotId: string, rootRunIds: string[], path: string) =>
    call(nothing, "conversation_file_reveal", { dotId, rootRunIds, path }),
  /// Send again: a failed chat message, queued once more in its topic.
  resendRun: (runId: string) => call(T.Run, "resend_run", { runId }),
  cancelRun: (runId: string) => call(T.Run, "cancel_run", { runId }),
  keepWaiting: (runId: string) => call(nothing, "keep_waiting", { runId }),
  /// Send a chat message that did not get there again; rejects with `conflict` when it just did.
  retryDelivery: (runId: string) => call(T.Run, "retry_delivery", { runId }),
  discardDelivery: (runId: string) => call(T.Run, "discard_delivery", { runId }),
  deliveryToday: (dotId: string) => call(T.DeliveryCounts, "delivery_today", { dotId }),
  /// Send later: schedules `text` at `dueAt` (RFC 3339) in the topic `rootRunId`, or as a new
  /// topic. Rejects with kind `invalid` for a time under a minute or over a year ahead.
  /// `attachmentIds`: staged files it waits with (attachments spec section 4.6); only sent
  /// when there are some.
  scheduleMessage: (
    dotId: string,
    text: string,
    rootRunId: string | null,
    dueAt: string,
    attachmentIds: string[] = [],
  ) =>
    call(
      T.ScheduledMessage,
      "schedule_message",
      attachmentIds.length > 0
        ? { dotId, text, rootRunId, dueAt, attachmentIds }
        : { dotId, text, rootRunId, dueAt },
    ),
  /// The dot's pending scheduled messages, in due order.
  scheduledMessages: (dotId: string) =>
    call(T.ScheduledMessage.array(), "scheduled_messages", { dotId }),
  /// These reject with kind `conflict` once the message is being sent, was sent or cancelled.
  sendScheduledNow: (id: string) => call(T.ScheduledMessage, "send_scheduled_now", { id }),
  editScheduledMessage: (id: string, text: string, dueAt: string) =>
    call(T.ScheduledMessage, "edit_scheduled_message", { id, text, dueAt }),
  cancelScheduledMessage: (id: string) =>
    call(T.ScheduledMessage, "cancel_scheduled_message", { id }),
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
  /// True when the call touches a private folder (keys, tokens, `.env` files): it always asks,
  /// and no rule can be saved with its approval.
  approvalTouchesPrivate: (id: string) => call(z.boolean(), "approval_touches_private", { id }),
  /// Gives every pending approval of one run the same answer, in the order they were asked.
  /// Answers the approvals it decided.
  /// Decides only `ids`, the calls of the run the person was shown: a snoozed call of the same
  /// run is not decided with them.
  decideRunApprovals: (runId: string, ids: string[], approved: boolean, note: string | null) =>
    call(T.Approval.array(), "decide_run_approvals", { runId, ids, approved, note }),
  inbox: () => call(T.Inbox, "inbox"),
  inboxCount: () => call(z.number().int(), "inbox_count"),
  /// Hides Inbox items until `until` (RFC 3339). Rejects with kind `invalid` for a time not in
  /// the future or more than 365 days ahead, and `not_found` when an item left the Inbox.
  inboxSnooze: (items: InboxKey[], until: string) =>
    call(nothing, "inbox_snooze", { items, until }),
  /// Puts a snoozed item back as it was.
  inboxUnsnooze: (item: InboxKey) => call(nothing, "inbox_unsnooze", { item }),
  /// Marks an item done; an approval is refused with kind `conflict`.
  inboxDone: (item: InboxKey) => call(nothing, "inbox_done", { item }),
  /// Puts a done item back as it was.
  inboxUndoDone: (item: InboxKey) => call(nothing, "inbox_undo_done", { item }),
  /// Clears the "Needs attention" of the dot's schedule; nothing happens when it is not set.
  /// Rejects with kind `not_found` when the dot has no schedule.
  dismissScheduleAttention: (dotId: string) =>
    call(nothing, "dismiss_schedule_attention", { dotId }),

  worktreeDiff: (runId: string) => call(T.WorktreeDiff, "worktree_diff", { runId }),
  discardWorktree: (runId: string) => call(T.Run, "discard_worktree", { runId }),
  markReviewed: (runId: string) => call(T.Run, "mark_reviewed", { runId }),
  openRunFolder: (runId: string) => call(nothing, "open_run_folder", { runId }),

  // Connectors (connectors spec §6).
  connectorList: () => call(T.ConnectorEntry.array(), "connector_list"),
  connectorRuntime: (id: string) => call(T.RuntimeCheck, "connector_runtime", { id }),
  connectorOpenPage: (id: string, page: "instructions" | "source" | "runtime") =>
    call(nothing, "connector_open_page", { id, page }),
  connectorConnect: (id: string, values: Record<string, string>, dotIds: string[]) =>
    call(T.ConnectorEntry, "connector_connect", { id, values, dotIds }),
  connectorSignIn: (id: string, dotIds: string[]) =>
    call(T.ConnectorEntry, "connector_sign_in", { id, dotIds }),
  connectorSignInCancel: (id: string) => call(nothing, "connector_sign_in_cancel", { id }),
  connectorSetDots: (id: string, dotIds: string[]) =>
    call(T.ConnectorEntry, "connector_set_dots", { id, dotIds }),
  connectorDisconnect: (id: string) => call(T.ConnectorEntry, "connector_disconnect", { id }),
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
  // Members, invites and unpublished member lists.
  memberList: (orgId: string) => call(T.MembersView, "member_list", { orgId }),
  inviteCreate: (orgId: string, email: string, role: T.Role) =>
    call(T.CreatedInvite, "invite_create", { orgId, email, role }),
  inviteRevoke: (orgId: string, inviteId: string) =>
    call(nothing, "invite_revoke", { orgId, inviteId }),
  requestInspect: (orgId: string, code: string) =>
    call(T.RequestPreview, "request_inspect", { orgId, code }),
  memberEdit: (orgId: string, baseSeq: number, edit: T.RosterEdit) =>
    call(T.EditOutcome, "member_edit", { orgId, baseSeq, edit }),
  rosterDraftRetry: (orgId: string) => call(T.EditOutcome, "roster_draft_retry", { orgId }),
  rosterDraftDiscard: (orgId: string) => call(nothing, "roster_draft_discard", { orgId }),
  orgRequestAccess: (orgId: string) => call(T.OrgSummary, "org_request_access", { orgId }),
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
  /// The slots skipped because the dot was busy, newest first (at most 50).
  scheduleSkips: (dotId: string, limit?: number) =>
    call(T.ScheduleSkip.array(), "schedule_skips", { dotId, limit }),
  /// The rows of the dot's topics: names and archive marks (topic tools spec section 5).
  listTopics: (dotId: string) => call(T.TopicInfo.array(), "list_topics", { dotId }),
  /// Names a topic by its root run; null or blank clears the name.
  renameTopic: (rootRunId: string, title: string | null) =>
    call(T.TopicInfo, "rename_topic", { rootRunId, title }),
  setTopicArchived: (rootRunId: string, archived: boolean) =>
    call(T.TopicInfo, "set_topic_archived", { rootRunId, archived }),
  /// Moves a run and its replies to another topic of its dot, or to a new topic (null); the
  /// root of the topic it is now in. Refusals come in words to show as they are.
  moveRun: (runId: string, toRootRunId: string | null) =>
    call(z.string(), "move_run", { runId, toRootRunId }),
  mergeTopics: (fromRootRunId: string, intoRootRunId: string) =>
    call(nothing, "merge_topics", { fromRootRunId, intoRootRunId }),
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
  /// The goals of the workspace `workspace` of `scope` ("personal" or an org id), and why
  /// this PC may not change them (Intents).
  intentsList: (scope: string, workspace: string) =>
    call(T.IntentsView, "intents_list", { scope, workspace }),
  /// Adds a goal. Rejects with the core's sentence (kind `invalid`, or `not_allowed` for a
  /// role that can't change goals).
  intentCreate: (scope: string, workspace: string, fields: IntentFields) =>
    call(T.Intent, "intent_create", { scope, workspace, fields }),
  /// Replaces a goal's text, due date, urgency and dot.
  intentUpdate: (id: string, fields: IntentFields) =>
    call(T.Intent, "intent_update", { id, fields }),
  /// Marks a goal done, or open again.
  intentSetDone: (id: string, done: boolean) => call(T.Intent, "intent_set_done", { id, done }),
  intentDelete: (id: string) => call(nothing, "intent_delete", { id }),

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
  /// The apps that come with Dots and are not installed ("Install Projects").
  appsBundledOffers: () => call(T.BundledOffer.array(), "apps_bundled_offers"),
  /// Installs an app that comes with Dots again: on the kept data `keptId`, or fresh (null).
  appInstallBundled: (name: string, keptId: string | null) =>
    call(T.LibraryItem, "app_install_bundled", { name, keptId }),
  /// The one-time prompt to let the app start dots: "Switch on" (true) or "Not now" (false).
  appAnswerStartPrompt: (name: string, on: boolean) =>
    call(T.LibraryItem, "app_answer_start_prompt", { name, on }),
  /// Where a run the app started is; rejects for a run it did not start.
  appRunLink: (name: string, runId: string) => call(T.RunLink, "app_run_link", { name, runId }),
  /// The person's answer on the card. Rejects with kind `conflict` when it was decided already
  /// or its app moved on, and while the dot's thread is still working.
  appProposalDecide: (id: string, decision: T.ProposalDecision) =>
    call(T.AppProposal, "app_proposal_decide", { id, decision }),
  /// The instructions dots proposed, of one dot or of all (null), newest first.
  instructionProposalsList: (dotId: string | null) =>
    call(T.InstructionProposal.array(), "instruction_proposals_list", { dotId }),
  /// The person's answer on the card. Rejects with kind `conflict` when it was decided already,
  /// when the dot's instructions changed since it proposed, and while its chat is working.
  instructionProposalDecide: (id: string, approve: boolean) =>
    call(T.InstructionProposal, "instruction_proposal_decide", { id, approve }),

  watchersList: (dotId: string) => call(T.Watcher.array(), "watchers_list", { dotId }),
  watcherAdd: (dotId: string, name: string, config: T.WatcherConfig, hours: T.WatchHours | null) =>
    call(T.Watcher, "watcher_add", { dotId, name, config, hours }),
  watcherUpdate: (id: string, name: string, config: T.WatcherConfig, hours: T.WatchHours | null) =>
    call(T.Watcher, "watcher_update", { id, name, config, hours }),
  watcherDelete: (id: string) => call(nothing, "watcher_delete", { id }),
  watcherSetEnabled: (id: string, enabled: boolean) =>
    call(T.Watcher, "watcher_set_enabled", { id, enabled }),
  watcherResume: (id: string) => call(T.Watcher, "watcher_resume", { id }),
  // Claude's usage limits and the Usage page (usage and limits spec section 6).
  limitsGet: () => call(T.LimitsState, "limits_get"),
  usageSummary: (period: T.UsagePeriod) => call(T.UsageSummary, "usage_summary", { period }),
  runReleaseHold: (runId: string) => call(T.Run, "run_release_hold", { runId }),
  /// Run once anyway: one of the person's own runs starts past the spending cap holding it.
  runReleaseCap: (runId: string) => call(T.Run, "run_release_cap", { runId }),
  /// Today's spend against the spending caps.
  spendToday: () => call(T.SpendToday, "spend_today"),

  // Teams, their projects and workflows (teams spec section 10). A refusal carries the core's
  // sentence: kind `invalid` for a rule broken, `conflict` for a taken name or a state that
  // moved on, `not_allowed` for a role that may not.
  teamsList: () => call(T.Team.array(), "teams_list"),
  teamGet: (id: string) => call(T.Team, "team_get", { id }),
  teamCreate: (spec: T.TeamSpec) => call(T.Team, "team_create", { spec }),
  teamUpdate: (id: string, spec: T.TeamSpec) => call(T.Team, "team_update", { id, spec }),
  /// Stops the team's runs in flight, then deletes it with its projects and workflows.
  teamDelete: (id: string) => call(nothing, "team_delete", { id }),
  /// Rejects with kind `invalid` when the folder is not a git repo.
  projectAdd: (teamId: string, spec: T.ProjectSpec) =>
    call(T.Project, "project_add", { teamId, spec }),
  projectUpdate: (id: string, spec: T.ProjectSpec) =>
    call(T.Project, "project_update", { id, spec }),
  projectDelete: (id: string) => call(nothing, "project_delete", { id }),
  /// Opens the project's repo folder in Explorer. Rejects with kind `not_found` once it is gone.
  openProjectFolder: (projectId: string) =>
    call(nothing, "open_project_folder", { projectId, workflowRunId: null, branch: null }),
  /// Opens the worktree of a branch the workflow run's deploy listed as ready.
  openBranchFolder: (workflowRunId: string, branch: string) =>
    call(nothing, "open_project_folder", { projectId: null, workflowRunId, branch }),
  workflowsList: (teamId: string) => call(T.Workflow.array(), "workflows_list", { teamId }),
  workflowGet: (id: string) => call(T.Workflow, "workflow_get", { id }),
  workflowCreate: (teamId: string, spec: T.WorkflowSpec) =>
    call(T.Workflow, "workflow_create", { teamId, spec }),
  /// Saves the builder's draft; runs in flight keep the definition they started with.
  workflowUpdate: (id: string, spec: T.WorkflowSpec) =>
    call(T.Workflow, "workflow_update", { id, spec }),
  workflowDelete: (id: string) => call(nothing, "workflow_delete", { id }),
  /// The builder's live check: the first refusal a save would give, or null.
  workflowCheckSpec: (teamId: string, spec: T.WorkflowSpec) =>
    call(T.WorkflowProblem.nullable(), "workflow_check_spec", { teamId, spec }),
  /// Rejects with kind `conflict` while the local server is not running.
  workflowWebhookUrl: (id: string) => call(text, "workflow_webhook_url", { id }),
  /// A new token, shown this once; the old one stops working.
  regenerateWorkflowToken: (id: string) => call(text, "regenerate_workflow_token", { id }),

  // Workflow runs (teams spec sections 5 to 10).
  /// Newest first; a field left out of `filter` does not narrow the list.
  workflowRunsList: (filter: Partial<T.WorkflowRunFilter>) =>
    call(T.WorkflowRun.array(), "workflow_runs_list", { filter }),
  /// The runs the chat runs `runIds` started, oldest first: a conversation's cards.
  workflowRunsForOrigins: (runIds: string[]) =>
    call(T.WorkflowRun.array(), "workflow_runs_for_origins", { runIds }),
  workflowRunGet: (id: string) => call(T.WorkflowRun, "workflow_run_get", { id }),
  /// The dot runs of the run's step runs, oldest first: the run page's links.
  workflowRunDotRuns: (id: string) => call(T.StepDotRun.array(), "workflow_run_dot_runs", { id }),
  /// "Run now", or "Test run" with `test`: its deploys push nothing.
  workflowRunStart: (workflowId: string, projectId: string, task: string, test: boolean) =>
    call(T.WorkflowRun, "workflow_run_start", { workflowId, projectId, task, test }),
  /// What the check the run waits on shows. Rejects with kind `conflict` when it waits on none.
  workflowCheck: (id: string) => call(T.CheckView, "workflow_check", { id }),
  /// A branch's committed work from the project's default branch, at `commit`.
  workflowBranchDiff: (id: string, branch: string, commit: string) =>
    call(T.WorktreeDiff, "workflow_branch_diff", { id, branch, commit }),
  /// Approves the check's step run `stepRunId` with the tips it showed. Rejects with kind
  /// `conflict` when a branch moved since or the run waits on another check: the check is read
  /// again.
  workflowApprove: (id: string, note: string | null, stepRunId: string, tips: T.CheckTip[]) =>
    call(T.WorkflowRun, "workflow_approve", { id, note, stepRunId, tips }),
  /// Sends the work of the check's step run `stepRunId` back. Rejects with kind `conflict` when
  /// the run waits on another check.
  workflowSendBack: (id: string, note: string, stepRunId: string) =>
    call(T.WorkflowRun, "workflow_send_back", { id, note, stepRunId }),
  workflowAddBudget: (id: string) => call(T.WorkflowRun, "workflow_add_budget", { id }),
  /// Lets the send-back held at the cap go on.
  workflowAllowSendBack: (id: string) => call(T.WorkflowRun, "workflow_allow_send_back", { id }),
  workflowRetryStep: (id: string) => call(T.WorkflowRun, "workflow_retry_step", { id }),
  workflowStop: (id: string) => call(T.WorkflowRun, "workflow_stop", { id }),
};
