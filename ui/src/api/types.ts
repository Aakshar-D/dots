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
export const ScheduleStatus = z.enum(["active", "completed", "expired"]);
export const LibraryKind = z.enum(["mcp_server", "skill", "plugin", "context"]);
// Where an MCP server's tool list stands: accepted, changed since, last check failed, or never
// accepted.
export const CatalogStatus = z.enum(["accepted", "changed", "unreachable", "unchecked"]);

export const Rule = z.strictObject({ tool: z.string(), action: Action });
export const Policy = z.strictObject({
  preset: Preset,
  rules: z.array(Rule),
  default: Action,
  mcp_default: Action,
});

// A dot's schedule (Scheduler v2 spec §2). Times are "HH:MM" and date-times
// "YYYY-MM-DDTHH:MM", local and without an offset; the core checks them. `anchor` (RFC 3339
// UTC) is the core's: the UI sends null, and the core keeps the stored phase.
export const Weekday = z.enum(["mon", "tue", "wed", "thu", "fri", "sat", "sun"]);
export const ScheduleRule = z.discriminatedUnion("kind", [
  z.strictObject({ kind: z.literal("once"), at: z.string() }),
  z.strictObject({
    kind: z.literal("every"),
    minutes: z.number().int(),
    anchor: nullable(z.string()),
  }),
  z.strictObject({ kind: z.literal("daily"), time: z.string() }),
  z.strictObject({ kind: z.literal("weekly"), days: z.array(Weekday), time: z.string() }),
  z.strictObject({ kind: z.literal("cron"), expr: z.string() }),
]);
export const Until = z.union([
  z.strictObject({ count: z.number().int() }),
  z.strictObject({ date: z.string() }),
]);
export const Schedule = z.strictObject({
  rule: ScheduleRule,
  catch_up: z.boolean(),
  until: nullable(Until),
});

/// The Library items a dot uses, by name.
export const Uses = z.strictObject({
  mcp_servers: z.array(z.string()),
  skills: z.array(z.string()),
  plugins: z.array(z.string()),
  context: z.array(z.string()),
});

const dotSpecShape = {
  name: z.string(),
  instructions: z.string(),
  engine: EngineKind,
  model: z.string(),
  endpoint_url: nullable(z.string()),
  workdir: z.string(),
  workspace_mode: WorkspaceMode,
  schedule: nullable(Schedule),
  policy: Policy,
  uses: Uses,
  use_global_instructions: z.boolean(),
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
  // The failed scheduled run that this run retries.
  retry_of: nullable(z.string()),
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
  // Something about the run that is not the model's doing: an item left out, a server problem.
  z.strictObject({ kind: z.literal("notice"), text: z.string() }),
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
  // For an MCP call: the server's accepted fingerprint when it asked.
  mcp_fingerprint: nullable(z.string()),
};
export const Approval = z.strictObject(approvalShape);

// An allow rule the core offers to save with an approval: the rule's pattern, and what it
// covers in words.
export const RuleCandidate = z.strictObject({ pattern: z.string(), label: z.string() });

export const RuntimeEvent = z.discriminatedUnion("type", [
  z.strictObject({ type: z.literal("run_updated"), run: Run }),
  z.strictObject({ type: z.literal("run_event"), event: RunEventRecord }),
  z.strictObject({ type: z.literal("approval_requested"), approval: Approval }),
  z.strictObject({ type: z.literal("approval_decided"), approval: Approval }),
  z.strictObject({ type: z.literal("approval_updated"), approval: Approval }),
  // The dot's schedule row was made, changed or deleted.
  z.strictObject({ type: z.literal("schedule_updated"), dot_id: z.string() }),
  // A Library item was added, changed, removed or checked (an MCP server's catalog included).
  z.strictObject({ type: z.literal("library_updated"), item_id: z.string() }),
  // The dot's schedule needs attention now (sent once, with a `schedule_updated`).
  z.strictObject({
    type: z.literal("schedule_attention"),
    dot_id: z.string(),
    run_id: z.string(),
    error: nullable(z.string()),
  }),
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
  global_instructions: z.string(),
});
export const SettingsSaved = z.strictObject({
  settings: Settings,
  restart_required: z.boolean(),
});

// Where a dot's schedule stands (Scheduler v2 spec §8). Times are RFC 3339 local.
// `next_fire` is null when nothing more fires (disabled, not active, nothing left);
// `retry_at` and `retry_attempt` are set only while a retry waits.
export const ScheduleView = z.strictObject({
  status: ScheduleStatus,
  fired_count: z.number().int(),
  next_fire: nullable(z.string()),
  retry_at: nullable(z.string()),
  retry_attempt: nullable(z.number().int()),
  attention: nullable(z.strictObject({ run_id: z.string(), error: nullable(z.string()) })),
});

export const DotOverview = z.strictObject({
  dot: Dot,
  active_run: nullable(Run),
  last_run: nullable(Run),
  // Null without a schedule.
  schedule_state: nullable(ScheduleView),
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
// A schedule whose occurrence failed in the end ("Needs attention").
export const NeedsAttention = z.strictObject({
  dot_id: z.string(),
  dot_name: z.string(),
  run_id: z.string(),
  error: nullable(z.string()),
  failed_at: z.string(),
});
export const DotRef = z.strictObject({ id: z.string(), name: z.string() });
// An MCP server whose tool list waits for review; its tools ask until it is accepted.
export const ToolsChanged = z.strictObject({
  server_id: z.string(),
  name: z.string(),
  dots: z.array(DotRef),
});
export const Inbox = z.strictObject({
  approvals: z.array(PendingApproval),
  review_runs: z.array(ReviewRun),
  // Newest failure first.
  attention: z.array(NeedsAttention),
  // By server name.
  tools_changed: z.array(ToolsChanged),
});

// The Library (MCP/skills spec section 2 and 9). `${VAR}` in a server's config stays a
// reference: the core reads the environment when the server starts.
export const McpServerConfig = z.discriminatedUnion("transport", [
  z.strictObject({
    transport: z.literal("stdio"),
    command: z.string(),
    args: z.array(z.string()),
    cwd: nullable(z.string()),
    env: z.record(z.string(), z.string()),
  }),
  z.strictObject({
    transport: z.literal("http"),
    url: z.string(),
    headers: z.record(z.string(), z.string()),
  }),
]);
export const SkillConfig = z.strictObject({ path: z.string() });
export const PluginConfig = z.strictObject({
  path: z.string(),
  // `<plugin>@<marketplace>` when picked from the installed Claude Code plugins.
  installed_id: nullable(z.string()),
  run_hooks: z.boolean(),
});
// A folder of documents dots read with the `context` tool (context library spec section 3).
export const ContextConfig = z.strictObject({
  path: z.string(),
  description: z.string(),
  // Every dot uses it, ticked or not.
  every_dot: z.boolean(),
});
// An item's config with its kind: what `library_add` and `library_update` take.
export const LibraryConfig = z.discriminatedUnion("kind", [
  z.strictObject({ kind: z.literal("mcp_server"), config: McpServerConfig }),
  z.strictObject({ kind: z.literal("skill"), config: SkillConfig }),
  z.strictObject({ kind: z.literal("plugin"), config: PluginConfig }),
  z.strictObject({ kind: z.literal("context"), config: ContextConfig }),
]);
const libraryItemShape = {
  id: z.string(),
  name: z.string(),
  // The plugin an MCP server comes from; it goes with the plugin.
  owner_id: nullable(z.string()),
  created_at: z.string(),
  updated_at: z.string(),
};
// One strict object per kind: the item's fields and its config sit side by side in the JSON.
const libraryItemOf = <E extends z.ZodRawShape>(extra: E) =>
  z.discriminatedUnion("kind", [
    z.strictObject({
      kind: z.literal("mcp_server"),
      config: McpServerConfig,
      ...libraryItemShape,
      ...extra,
    }),
    z.strictObject({
      kind: z.literal("skill"),
      config: SkillConfig,
      ...libraryItemShape,
      ...extra,
    }),
    z.strictObject({
      kind: z.literal("plugin"),
      config: PluginConfig,
      ...libraryItemShape,
      ...extra,
    }),
    z.strictObject({
      kind: z.literal("context"),
      config: ContextConfig,
      ...libraryItemShape,
      ...extra,
    }),
  ]);
export const LibraryItem = libraryItemOf({});
// A tool of `tools/list` as the server sent it (camelCase, optional fields left out).
export const McpTool = z.object({
  name: z.string(),
  title: z.string().optional(),
  description: z.string().optional(),
  inputSchema: z.unknown(),
  annotations: z.unknown().optional(),
});
export const McpCatalog = z.strictObject({
  server_id: z.string(),
  status: CatalogStatus,
  accepted_fingerprint: nullable(z.string()),
  // `tools/list` results (arrays of `McpTool`), kept as JSON.
  accepted_tools: nullable(z.unknown()),
  seen_fingerprint: nullable(z.string()),
  seen_tools: nullable(z.unknown()),
  checked_at: nullable(z.string()),
  error: nullable(z.string()),
});
export const SkillDetail = z.strictObject({ description: z.string() });
export const ContextDetail = z.strictObject({
  documents: z.number().int(),
  // False when the walk stopped at its bound: `documents` is a lower bound.
  complete: z.boolean(),
});
export const PluginDetail = z.strictObject({
  version: nullable(z.string()),
  description: nullable(z.string()),
  // Qualified: `<plugin>:<skill>`.
  skills: z.array(z.string()),
  commands: z.array(z.string()),
  agents: z.array(z.string()),
  // The names of its MCP server items.
  servers: z.array(z.string()),
  has_hooks: z.boolean(),
  warnings: z.array(z.string()),
});
export const LibraryEntry = libraryItemOf({
  // The dots that use it; for a plugin's server, the dots that use the plugin.
  used_by: z.array(DotRef),
  catalog: nullable(McpCatalog),
  // The `${VAR}` of a server's config that the environment of Dots does not set.
  missing_variables: z.array(z.string()),
  skill: nullable(SkillDetail),
  plugin: nullable(PluginDetail),
  context: nullable(ContextDetail),
  // Why a skill, plugin or context folder could not be read.
  problem: nullable(z.string()),
});
export const ContextDocument = z.strictObject({
  path: z.string(),
  title: z.string(),
  summary: z.string(),
});
// A collection's first documents (`context_documents`); `total` counts them all.
export const ContextDocuments = z.strictObject({
  documents: z.array(ContextDocument),
  total: z.number().int(),
  truncated: z.boolean(),
  // False when the walk stopped at its bound: `total` is a lower bound.
  complete: z.boolean(),
});
export const McpProbe = z.strictObject({ tools: z.array(McpTool), fingerprint: z.string() });
export const ScannedSkill = z.strictObject({
  name: z.string(),
  description: z.string(),
  path: z.string(),
  // Why `name` cannot be a Library name.
  name_error: nullable(z.string()),
});
export const InstalledPlugin = z.strictObject({
  id: z.string(),
  name: z.string(),
  marketplace: nullable(z.string()),
  scope: nullable(z.string()),
  install_path: z.string(),
  version: nullable(z.string()),
  exists: z.boolean(),
});
// Whether the environment of Dots sets a variable. Never its value.
export const EnvVariable = z.strictObject({ name: z.string(), set: z.boolean() });

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

/// Validation limits of the core that the dot editor and Settings mirror, and the attempts a
/// scheduled occurrence has at most, which the Dots list and the run view name. The contract
/// test checks them against the Rust constants.
export const LIMITS = {
  max_timeout_secs: 604800,
  max_approval_wait_secs: 280,
  max_concurrent_runs: 16,
  max_schedule_attempts: 6,
  max_global_instructions: 32768,
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
export type ScheduleStatus = z.infer<typeof ScheduleStatus>;
export type LibraryKind = z.infer<typeof LibraryKind>;
export type CatalogStatus = z.infer<typeof CatalogStatus>;
export type Rule = z.infer<typeof Rule>;
export type Policy = z.infer<typeof Policy>;
export type Weekday = z.infer<typeof Weekday>;
export type ScheduleRule = z.infer<typeof ScheduleRule>;
export type Until = z.infer<typeof Until>;
export type Schedule = z.infer<typeof Schedule>;
export type Uses = z.infer<typeof Uses>;
export type DotSpec = z.infer<typeof DotSpec>;
export type Dot = z.infer<typeof Dot>;
export type Run = z.infer<typeof Run>;
export type RunEventRecord = z.infer<typeof RunEventRecord>;
export type RunEventData = z.infer<typeof RunEventData>;
export type Approval = z.infer<typeof Approval>;
export type RuleCandidate = z.infer<typeof RuleCandidate>;
export type RuntimeEvent = z.infer<typeof RuntimeEvent>;
export type ClaudeStatus = z.infer<typeof ClaudeStatus>;
export type Status = z.infer<typeof Status>;
export type Settings = z.infer<typeof Settings>;
export type SettingsSaved = z.infer<typeof SettingsSaved>;
export type ScheduleView = z.infer<typeof ScheduleView>;
export type DotOverview = z.infer<typeof DotOverview>;
export type PendingApproval = z.infer<typeof PendingApproval>;
export type ReviewRun = z.infer<typeof ReviewRun>;
export type NeedsAttention = z.infer<typeof NeedsAttention>;
export type DotRef = z.infer<typeof DotRef>;
export type ToolsChanged = z.infer<typeof ToolsChanged>;
export type Inbox = z.infer<typeof Inbox>;
export type McpServerConfig = z.infer<typeof McpServerConfig>;
export type SkillConfig = z.infer<typeof SkillConfig>;
export type PluginConfig = z.infer<typeof PluginConfig>;
export type LibraryConfig = z.infer<typeof LibraryConfig>;
export type LibraryItem = z.infer<typeof LibraryItem>;
export type McpTool = z.infer<typeof McpTool>;
export type McpCatalog = z.infer<typeof McpCatalog>;
export type SkillDetail = z.infer<typeof SkillDetail>;
export type PluginDetail = z.infer<typeof PluginDetail>;
export type LibraryEntry = z.infer<typeof LibraryEntry>;
export type ContextConfig = z.infer<typeof ContextConfig>;
export type ContextDocuments = z.infer<typeof ContextDocuments>;
export type McpProbe = z.infer<typeof McpProbe>;
export type ScannedSkill = z.infer<typeof ScannedSkill>;
export type InstalledPlugin = z.infer<typeof InstalledPlugin>;
export type EnvVariable = z.infer<typeof EnvVariable>;
export type FileDiff = z.infer<typeof FileDiff>;
export type WorktreeDiff = z.infer<typeof WorktreeDiff>;
export type CmdErrorKind = z.infer<typeof CmdErrorKind>;
export type CmdError = z.infer<typeof CmdError>;
export type StartupError = z.infer<typeof StartupError>;
