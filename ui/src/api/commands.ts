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
  LibraryConfig,
  McpServerConfig,
  Preset,
  Schedule,
  Settings,
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

  dotOverview: () => call(T.DotOverview.array(), "dot_overview"),
  listDots: () => call(T.Dot.array(), "list_dots"),
  getDot: (id: string) => call(T.Dot, "get_dot", { id }),
  createDot: (spec: DotSpec) => call(T.Dot, "create_dot", { spec }),
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
  libraryCheck: (id: string) => call(T.McpCatalog, "library_check", { id }),
  /// Rejects with kind `conflict` when the list moved since `fingerprint` was seen.
  libraryAccept: (id: string, fingerprint: string) =>
    call(T.McpCatalog, "library_accept", { id, fingerprint }),
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
};
