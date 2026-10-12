// Typed wrappers over the Tauri commands in apps/dots-app/src/commands.rs. Arguments are
// camelCase here; Tauri maps them to the Rust snake_case parameters.
import { invoke, type InvokeArgs } from "@tauri-apps/api/core";

import {
  CmdError,
  type Approval,
  type CmdErrorKind,
  type Dot,
  type DotOverview,
  type DotSpec,
  type Inbox,
  type Run,
  type RunEventRecord,
  type Settings,
  type SettingsSaved,
  type StartupError,
  type Status,
  type WorktreeDiff,
} from "./types";

export type { StartupError, Status } from "./types";

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
  const parsed = CmdError.safeParse(e);
  if (parsed.success) return new CommandError(parsed.data.kind, parsed.data.message);
  return new CommandError("internal", e instanceof Error ? e.message : String(e));
}

async function call<T>(cmd: string, args?: InvokeArgs): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    throw toCommandError(e);
  }
}

export const commands = {
  startupError: () => call<StartupError | null>("startup_error"),
  status: () => call<Status>("status"),
  markClaudeTested: () => call<Status>("mark_claude_tested"),
  getSettings: () => call<Settings>("get_settings"),
  updateSettings: (settings: Settings) => call<SettingsSaved>("update_settings", { settings }),
  setSchedulesPaused: (paused: boolean) => call<Status>("set_schedules_paused", { paused }),
  restartApp: () => call<void>("restart_app"),

  dotOverview: () => call<DotOverview[]>("dot_overview"),
  listDots: () => call<Dot[]>("list_dots"),
  getDot: (id: string) => call<Dot>("get_dot", { id }),
  createDot: (spec: DotSpec) => call<Dot>("create_dot", { spec }),
  updateDot: (id: string, spec: DotSpec) => call<Dot>("update_dot", { id, spec }),
  deleteDot: (id: string) => call<null>("delete_dot", { id }),
  setDotEnabled: (id: string, enabled: boolean) => call<Dot>("set_dot_enabled", { id, enabled }),
  regenerateWebhookToken: (id: string) => call<string>("regenerate_webhook_token", { id }),
  webhookUrl: (id: string) => call<string>("webhook_url", { id }),
  previewSchedule: (expr: string, count: number) =>
    call<string[]>("preview_schedule", { expr, count }),
  testEndpoint: (endpoint: string, model: string) =>
    call<string>("test_endpoint", { endpoint, model }),

  runNow: (dotId: string) => call<Run>("run_now", { dotId }),
  chat: (dotId: string, message: string, parentRunId: string | null) =>
    call<Run>("chat", { dotId, message, parentRunId }),
  cancelRun: (runId: string) => call<Run>("cancel_run", { runId }),
  listRuns: (dotId: string | null, limit: number) => call<Run[]>("list_runs", { dotId, limit }),
  getRun: (id: string) => call<Run>("get_run", { id }),
  runThread: (runId: string) => call<Run[]>("run_thread", { runId }),
  listRunEvents: (runId: string, afterSeq: number) =>
    call<RunEventRecord[]>("list_run_events", { runId, afterSeq }),
  approvalsForRun: (runId: string) => call<Approval[]>("approvals_for_run", { runId }),
  decideApproval: (id: string, approved: boolean, note: string | null) =>
    call<Approval>("decide_approval", { id, approved, note }),
  inbox: () => call<Inbox>("inbox"),
  inboxCount: () => call<number>("inbox_count"),

  worktreeDiff: (runId: string) => call<WorktreeDiff>("worktree_diff", { runId }),
  discardWorktree: (runId: string) => call<Run>("discard_worktree", { runId }),
  markReviewed: (runId: string) => call<Run>("mark_reviewed", { runId }),
  openRunFolder: (runId: string) => call<null>("open_run_folder", { runId }),
};
