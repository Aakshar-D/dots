// Typed wrappers over the Tauri commands in apps/dots-app/src/commands.rs. Arguments are
// camelCase here; Tauri maps them to the Rust snake_case parameters. Every answer is parsed
// with its schema, so a shape this UI does not know is an error here, not a broken screen.
import { invoke, type InvokeArgs } from "@tauri-apps/api/core";
import { z } from "zod";

import * as T from "./types";
import type { CmdErrorKind, DotSpec, Preset, Settings } from "./types";

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
  previewSchedule: (expr: string, count: number) =>
    call(text.array(), "preview_schedule", { expr, count }),
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

  worktreeDiff: (runId: string) => call(T.WorktreeDiff, "worktree_diff", { runId }),
  discardWorktree: (runId: string) => call(T.Run, "discard_worktree", { runId }),
  markReviewed: (runId: string) => call(T.Run, "mark_reviewed", { runId }),
  openRunFolder: (runId: string) => call(nothing, "open_run_folder", { runId }),
};
