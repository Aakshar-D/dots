// Mirrors of the Rust JSON the app sends and receives. Each schema is strict, and
// contract.test.ts parses a sample of every type written by the Rust side
// (apps/dots-app/src/contract.rs), so the two cannot drift apart silently.
import { z } from "zod";

const nullable = <T extends z.ZodType>(t: T) => t.nullable();

export const EngineKind = z.enum(["claude", "local"]);
export const WorkspaceMode = z.enum(["worktree", "folder"]);
export const TriggerKind = z.enum(["schedule", "webhook", "manual", "chat", "resume"]);
export const RunStatus = z.enum([
  "queued",
  "running",
  "awaiting_approval",
  "succeeded",
  "failed",
  "cancelled",
]);
export const ApprovalStatus = z.enum(["pending", "approved", "denied", "expired"]);
// Where a call that Dots performs itself stands (a parked approval of a local dot).
export const ApplyState = z.enum([
  "queued",
  "applying",
  "applied",
  "failed",
  "unknown",
  "skipped",
]);
export const Action = z.enum(["allow", "ask", "deny"]);
export const Preset = z.enum(["read-only", "sandboxed", "trusted", "custom"]);
export const FileStatus = z.enum(["added", "modified", "deleted", "renamed"]);

export const Rule = z.strictObject({ tool: z.string(), action: Action });
export const Policy = z.strictObject({
  preset: Preset,
  rules: z.array(Rule),
  default: Action,
});

const dotSpecShape = {
  name: z.string(),
  instructions: z.string(),
  engine: EngineKind,
  model: z.string(),
  endpoint_url: nullable(z.string()),
  workdir: z.string(),
  workspace_mode: WorkspaceMode,
  schedule: nullable(z.string()),
  policy: Policy,
  mcp_servers: nullable(z.record(z.string(), z.unknown())),
  use_user_settings: z.boolean(),
  max_turns: z.number().int(),
  timeout_secs: z.number().int(),
  approval_wait_secs: z.number().int(),
  enabled: z.boolean(),
};
export const DotSpec = z.strictObject(dotSpecShape);
export const Dot = z.strictObject({
  id: z.string(),
  webhook_token: z.string(),
  created_at: z.string(),
  updated_at: z.string(),
  ...dotSpecShape,
});

const runShape = {
  id: z.string(),
  dot_id: z.string(),
  root_run_id: z.string(),
  parent_run_id: nullable(z.string()),
  trigger: TriggerKind,
  payload: nullable(z.unknown()),
  status: RunStatus,
  session_id: nullable(z.string()),
  workspace_path: nullable(z.string()),
  branch: nullable(z.string()),
  base_commit: nullable(z.string()),
  summary: nullable(z.string()),
  error: nullable(z.string()),
  tokens_in: z.number().int(),
  tokens_out: z.number().int(),
  queued_at: z.string(),
  started_at: nullable(z.string()),
  ended_at: nullable(z.string()),
  reviewed_at: nullable(z.string()),
};
export const Run = z.strictObject(runShape);

export const RunEventRecord = z.strictObject({
  run_id: z.string(),
  seq: z.number().int(),
  ts: z.string(),
  kind: z.string(),
  data: z.unknown(),
});

// What a run event's `data` holds: the engine event, tagged by `kind`. `RunEventRecord.data`
// stays `unknown` so that an event kind from a newer core cannot break a whole event list;
// the transcript parses each event with this schema and shows what it cannot parse as raw.
export const RunEventData = z.discriminatedUnion("kind", [
  z.strictObject({ kind: z.literal("session_started"), session_id: z.string() }),
  z.strictObject({ kind: z.literal("assistant_text"), text: z.string() }),
  z.strictObject({
    kind: z.literal("tool_call"),
    id: z.string(),
    tool: z.string(),
    input: z.unknown(),
  }),
  z.strictObject({
    kind: z.literal("tool_result"),
    id: z.string(),
    output: z.string(),
    is_error: z.boolean(),
  }),
  z.strictObject({
    kind: z.literal("usage"),
    tokens_in: z.number().int(),
    tokens_out: z.number().int(),
  }),
  z.strictObject({ kind: z.literal("raw"), line: z.string() }),
  z.strictObject({ kind: z.literal("message"), message: z.unknown() }),
  z.strictObject({ kind: z.literal("finished"), summary: z.string() }),
  z.strictObject({ kind: z.literal("failed"), error: z.string() }),
]);

const approvalShape = {
  id: z.string(),
  run_id: z.string(),
  tool: z.string(),
  input: z.unknown(),
  input_hash: z.string(),
  status: ApprovalStatus,
  note: nullable(z.string()),
  parked: z.boolean(),
  resolved: z.boolean(),
  created_at: z.string(),
  decided_at: nullable(z.string()),
  self_apply: z.boolean(),
  apply_state: nullable(ApplyState),
  applied_at: nullable(z.string()),
};
export const Approval = z.strictObject(approvalShape);

export const RuntimeEvent = z.discriminatedUnion("type", [
  z.strictObject({ type: z.literal("run_updated"), run: Run }),
  z.strictObject({ type: z.literal("run_event"), event: RunEventRecord }),
  z.strictObject({ type: z.literal("approval_requested"), approval: Approval }),
  z.strictObject({ type: z.literal("approval_decided"), approval: Approval }),
  z.strictObject({ type: z.literal("approval_updated"), approval: Approval }),
]);

export const ClaudeStatus = z.strictObject({
  program: nullable(z.string()),
  version: nullable(z.string()),
  tested_version: z.string(),
  mismatch: z.boolean(),
});
export const Status = z.strictObject({
  port: z.number().int(),
  server_error: nullable(z.string()),
  claude: ClaudeStatus,
  schedules_paused: z.boolean(),
  restart_required: z.boolean(),
  data_dir: z.string(),
});

export const Settings = z.strictObject({
  port: z.number().int(),
  max_concurrent_runs: z.number().int(),
  claude_path: nullable(z.string()),
  tested_claude_version: z.string(),
  local_endpoints: z.array(z.string()),
  autostart: z.boolean(),
  schedules_paused: z.boolean(),
});
export const SettingsSaved = z.strictObject({
  settings: Settings,
  restart_required: z.boolean(),
});

export const DotOverview = z.strictObject({
  dot: Dot,
  active_run: nullable(Run),
  last_run: nullable(Run),
  next_fire: nullable(z.string()),
});

export const PendingApproval = z.strictObject({
  ...approvalShape,
  dot_id: z.string(),
  dot_name: z.string(),
});
export const ReviewRun = z.strictObject({
  ...runShape,
  dot_name: z.string(),
  worktree_exists: z.boolean(),
});
export const Inbox = z.strictObject({
  approvals: z.array(PendingApproval),
  review_runs: z.array(ReviewRun),
});

export const FileDiff = z.strictObject({
  path: z.string(),
  status: FileStatus,
  patch: z.string(),
  binary: z.boolean(),
});
export const WorktreeDiff = z.strictObject({
  files: z.array(FileDiff),
  truncated: z.boolean(),
  exists: z.boolean(),
});

// `cancelled`: the user declined a native confirmation; nothing was changed.
export const CmdErrorKind = z.enum(["not_found", "invalid", "conflict", "internal", "cancelled"]);
export const CmdError = z.strictObject({ kind: CmdErrorKind, message: z.string() });
export const StartupError = z.strictObject({ message: z.string(), log_dir: z.string() });

/// Validation limits of the core that the dot editor and Settings mirror. The contract test
/// checks them against the Rust constants.
export const LIMITS = {
  max_timeout_secs: 604800,
  max_approval_wait_secs: 280,
  max_concurrent_runs: 16,
} as const;

export type EngineKind = z.infer<typeof EngineKind>;
export type WorkspaceMode = z.infer<typeof WorkspaceMode>;
export type TriggerKind = z.infer<typeof TriggerKind>;
export type RunStatus = z.infer<typeof RunStatus>;
export type ApprovalStatus = z.infer<typeof ApprovalStatus>;
export type ApplyState = z.infer<typeof ApplyState>;
export type Action = z.infer<typeof Action>;
export type Preset = z.infer<typeof Preset>;
export type FileStatus = z.infer<typeof FileStatus>;
export type Rule = z.infer<typeof Rule>;
export type Policy = z.infer<typeof Policy>;
export type DotSpec = z.infer<typeof DotSpec>;
export type Dot = z.infer<typeof Dot>;
export type Run = z.infer<typeof Run>;
export type RunEventRecord = z.infer<typeof RunEventRecord>;
export type RunEventData = z.infer<typeof RunEventData>;
export type Approval = z.infer<typeof Approval>;
export type RuntimeEvent = z.infer<typeof RuntimeEvent>;
export type ClaudeStatus = z.infer<typeof ClaudeStatus>;
export type Status = z.infer<typeof Status>;
export type Settings = z.infer<typeof Settings>;
export type SettingsSaved = z.infer<typeof SettingsSaved>;
export type DotOverview = z.infer<typeof DotOverview>;
export type PendingApproval = z.infer<typeof PendingApproval>;
export type ReviewRun = z.infer<typeof ReviewRun>;
export type Inbox = z.infer<typeof Inbox>;
export type FileDiff = z.infer<typeof FileDiff>;
export type WorktreeDiff = z.infer<typeof WorktreeDiff>;
export type CmdErrorKind = z.infer<typeof CmdErrorKind>;
export type CmdError = z.infer<typeof CmdError>;
export type StartupError = z.infer<typeof StartupError>;
