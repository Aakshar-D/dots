// Mirrors of the Rust JSON the app sends and receives. Each schema is strict, and
// contract.test.ts parses a sample of every type written by the Rust side
// (apps/dots-app/src/contract.rs), so the two cannot drift apart silently.
import { z } from "zod";

export const ChatGptStatus = z.strictObject({
  configured: z.boolean(),
  signing_in: z.boolean(),
  account: z.strictObject({ name: z.string().nullable(), email: z.string().nullable() }).nullable(),
  setup_error: z.string().nullable(),
  expires_at: z.iso.datetime({ offset: true }).nullable(),
});
export type ChatGptStatus = z.infer<typeof ChatGptStatus>;

const nullable = <T extends z.ZodType>(t: T) => t.nullable();

export const EngineKind = z.enum(["claude", "local"]);
export const WorkspaceMode = z.enum(["worktree", "folder"]);
export const TriggerKind = z.enum([
  "schedule",
  "webhook",
  "manual",
  "chat",
  "resume",
  // A run a dot scheduled for itself (check-ins spec section 6).
  "follow_up",
  // A run an app started (app library spec section 7).
  "app",
  // A step of a team's workflow (teams spec section 5).
  "workflow",
  // A run a watcher started: a file, an email or a GitHub event (watchers spec section 5).
  "watcher",
]);
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
export const LibraryKind = z.enum(["mcp_server", "skill", "plugin", "context", "app"]);
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
  apps: z.array(z.string()),
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
  // Consumer (`mcp:<server>` or `shell`) to the variable names it may resolve.
  variables: z.record(z.string(), z.array(z.string())),
  // A visible Microsoft Edge with the dot's own profile (browser spec section 2.4).
  browser: z.boolean(),
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
  // The token itself is shown once, when it is made.
  has_webhook_token: z.boolean(),
  created_at: z.string(),
  updated_at: z.string(),
  // When the person last looked at its conversation (RFC 3339 UTC); null before the first time.
  seen_at: nullable(z.string()),
  // The org the dot belongs to, fixed when it is made; null is Personal. The org may have been
  // removed since, and the dot keeps its id.
  org_id: nullable(z.string()),
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
  // What the run would cost on the API, as the claude CLI said; null for a local run.
  cost_usd: nullable(z.number()),
  // The person released it from the hold with Run anyway.
  hold_released: z.boolean(),
  // The workflow step the run works for (teams spec section 4).
  workflow_step_run_id: nullable(z.string()),
  // The part of tokens_in read from the prompt cache.
  tokens_cache_read: z.number().int(),
};
export const Run = z.strictObject(runShape);

export const RunEventRecord = z.strictObject({
  run_id: z.string(),
  seq: z.number().int(),
  ts: z.string(),
  kind: z.string(),
  data: z.unknown(),
});

// Claude's usage limits (usage and limits spec sections 3 to 6). A reading of one window as
// the claude CLI last reported it; utilization is the share used, 0 to 1 (it may exceed 1).
export const LimitReading = z.strictObject({
  window: z.string(),
  utilization: z.number(),
  resets_at: z.string(),
  status: z.string(),
  read_at: z.string(),
});
export const HoldShares = z.strictObject({
  enabled: z.boolean(),
  five_hour: z.number().int(),
  weekly: z.number().int(),
});
// five_hour and weekly are the known readings only (null once a window has reset).
export const LimitsState = z.strictObject({
  windows: z.array(LimitReading),
  five_hour: nullable(LimitReading),
  weekly: nullable(LimitReading),
  shares: HoldShares,
  holding: z.boolean(),
  draining: z.boolean(),
  rejected_until: nullable(z.string()),
  held_run_ids: z.array(z.string()),
  hold_until: nullable(z.string()),
});
export const HoldChange = z.discriminatedUnion("kind", [
  z.strictObject({ kind: z.literal("started"), window: z.string(), utilization: z.number() }),
  z.strictObject({ kind: z.literal("ended") }),
]);
export const UsagePeriod = z.enum(["five_hours", "week", "month"]);
export const UsageRun = z.strictObject({
  run_id: z.string(),
  trigger: TriggerKind,
  started_at: z.string(),
  tokens_in: z.number().int(),
  tokens_out: z.number().int(),
  run_secs: nullable(z.number().int()),
  cost_usd: nullable(z.number()),
});
export const UsageRow = z.strictObject({
  dot_id: z.string(),
  dot_name: z.string(),
  engine: EngineKind,
  runs: z.number().int(),
  tokens_in: z.number().int(),
  tokens_out: z.number().int(),
  run_secs: z.number().int(),
  cost_usd: nullable(z.number()),
  heaviest: z.array(UsageRun),
});
export const UsageSummary = z.strictObject({
  period: UsagePeriod,
  since: z.string(),
  rows: z.array(UsageRow),
});
// Why a run is held: Claude usage, or a role that no longer lets this PC run the dot.
export const HoldReason = z.discriminatedUnion("kind", [
  z.strictObject({ kind: z.literal("usage") }),
  z.strictObject({ kind: z.literal("role"), account: z.string(), text: z.string() }),
]);
// A run held for Claude usage or a role; starts_after is null while held runs start one at a
// time, and for a role hold.
export const HeldRun = z.strictObject({
  run_id: z.string(),
  dot_id: z.string(),
  dot_name: z.string(),
  trigger: TriggerKind,
  queued_at: z.string(),
  starts_after: nullable(z.string()),
  reason: HoldReason,
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
    // The part of tokens_in read from the prompt cache; absent from older events.
    cache_read: z.number().int().optional(),
    // Absent from the events of runs that ended before Dots kept a cost.
    cost_usd: nullable(z.number()).optional(),
  }),
  // The claude CLI's limit reading; not shown in the transcript.
  z.strictObject({
    kind: z.literal("limits"),
    status: z.string(),
    windows: z.array(LimitReading),
    resets_at: nullable(z.string()).optional(),
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

export const MessageKind = z.enum(["note", "question"]);
export const FollowUpStatus = z.enum(["pending", "fired", "cancelled", "skipped"]);
// A dot's note or question (check-ins spec section 7). Times are RFC 3339 UTC.
export const Message = z.strictObject({
  id: z.string(),
  dot_id: z.string(),
  dot_name: z.string(),
  // Null for a note of Dots' own (a skipped follow-up).
  run_id: nullable(z.string()),
  kind: MessageKind,
  text: z.string(),
  choices: nullable(z.array(z.string())),
  created_at: z.string(),
  read_at: nullable(z.string()),
  answered_at: nullable(z.string()),
  // The run the answer started.
  answer_run_id: nullable(z.string()),
  dismissed_at: nullable(z.string()),
});
// A run a dot scheduled for itself (check-ins spec section 6). Times are RFC 3339 UTC.
export const FollowUp = z.strictObject({
  id: z.string(),
  dot_id: z.string(),
  from_run_id: z.string(),
  due_at: z.string(),
  note: z.string(),
  status: FollowUpStatus,
  fired_run_id: nullable(z.string()),
  attempts: z.number().int(),
  retry_at: nullable(z.string()),
  created_at: z.string(),
});

export const MemorySource = z.enum(["dot", "person"]);
// One thing a dot remembers across runs (dot memory spec section 2). Times are RFC 3339 UTC.
export const Memory = z.strictObject({
  id: z.string(),
  dot_id: z.string(),
  text: z.string(),
  pinned: z.boolean(),
  // Who wrote it: a run of the dot, or the person in the Memory tab.
  source: MemorySource,
  // The run that saved it; null for the person's own.
  run_id: nullable(z.string()),
  created_at: z.string(),
  updated_at: z.string(),
});

// Watchers (watchers spec sections 3 and 4): what starts a dot when a folder, a Gmail label or a
// GitHub repository changes.
export const WatcherKind = z.enum(["folder", "email", "github"]);
// Why Dots paused a watcher; the person's own pause is `enabled`.
export const PausedReason = z.enum(["flood", "error"]);
export const FolderEvent = z.enum(["created", "changed", "deleted"]);
export const GithubEvent = z.enum(["issue", "pull_request", "comment", "push", "ci"]);
export const FolderConfig = z.strictObject({
  kind: z.literal("folder"),
  path: z.string(),
  // A file-name pattern such as *.pdf; empty matches every file.
  pattern: z.string(),
  events: z.array(FolderEvent),
  recursive: z.boolean(),
});
export const EmailConfig = z.strictObject({
  kind: z.literal("email"),
  address: z.string(),
  // The vault entry that holds the app password.
  secret: z.string(),
  label: z.string(),
  from: nullable(z.string()),
  subject: nullable(z.string()),
  has_attachment: nullable(z.boolean()),
});
export const GithubConfig = z.strictObject({
  kind: z.literal("github"),
  repos: z.array(z.string()),
  events: z.array(GithubEvent),
  // The vault entry that holds the token.
  secret: z.string(),
  label: nullable(z.string()),
  authors: z.array(z.string()),
  skip_bots: z.boolean(),
  branches: z.array(z.string()),
});
export const WatcherConfig = z.discriminatedUnion("kind", [FolderConfig, EmailConfig, GithubConfig]);
export const Watcher = z.strictObject({
  id: z.string(),
  dot_id: z.string(),
  name: z.string(),
  config: WatcherConfig,
  enabled: z.boolean(),
  paused_reason: nullable(PausedReason),
  last_error: nullable(z.string()),
  last_event_at: nullable(z.string()),
  created_at: z.string(),
  updated_at: z.string(),
});
// A watcher Dots paused, as the Inbox lists it.
export const PausedWatcher = z.strictObject({
  watcher_id: z.string(),
  dot_id: z.string(),
  dot_name: z.string(),
  kind: WatcherKind,
  name: z.string(),
  reason: PausedReason,
  error: nullable(z.string()),
  at: z.string(),
});

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
  // A dot posted a note or asked a question.
  z.strictObject({ type: z.literal("message_posted"), message: Message }),
  // A message was read, answered or dismissed.
  z.strictObject({ type: z.literal("message_updated"), message: Message }),
  // A follow-up of the dot was scheduled, fired, cancelled or skipped.
  z.strictObject({ type: z.literal("follow_ups_updated"), dot_id: z.string() }),
  // The person looked at the dot's conversation, or the core marked the viewed dot seen.
  z.strictObject({ type: z.literal("dot_seen"), dot_id: z.string() }),
  // A memory of the dot was saved, changed or deleted, by a run or by the person.
  z.strictObject({ type: z.literal("memories_updated"), dot_id: z.string() }),
  // A call changed the app's data: its screen reads again.
  z.strictObject({ type: z.literal("app_data_changed"), app: z.string() }),
  // A dot proposed an app, or the person decided a proposal: the review card reads it again.
  z.strictObject({ type: z.literal("app_proposal_changed"), id: z.string(), dot_id: z.string() }),
  // A watcher of the dot was added, changed, paused, resumed, deleted or had an event.
  z.strictObject({
    type: z.literal("watcher_updated"),
    dot_id: z.string(),
    watcher_id: z.string(),
  }),
  // Dots paused a watcher; the app notifies on it.
  z.strictObject({
    type: z.literal("watcher_paused"),
    dot_id: z.string(),
    watcher_id: z.string(),
    name: z.string(),
    reason: PausedReason,
    error: nullable(z.string()),
  }),
  // An org was joined, updated, fetched (by Refresh, its timer or the start-up fetch) or
  // removed: its status, policy or values may have changed.
  z.strictObject({ type: z.literal("org_updated"), org_id: z.string() }),
  // A new limit reading, or a hold that started, ended or changed what it holds; `change` is
  // set when a hold started or ended (the app notifies).
  z.strictObject({
    type: z.literal("limits_updated"),
    limits: LimitsState,
    change: nullable(HoldChange),
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
  // A dot's note notifies; a question always does.
  notify_on_notes: z.boolean(),
  // The Edge program dots browse with; null uses the installed one.
  edge_path: nullable(z.string()),
  // Background Claude runs wait at these shares of the 5-hour and weekly windows (percent).
  hold_enabled: z.boolean(),
  hold_share_five_hour: z.number().int(),
  hold_share_weekly: z.number().int(),
  // Dots installs a downloaded update by itself when idle (updates UI spec section 4.2).
  auto_install_updates: z.boolean(),
  // The vault entry holding the GitHub token Dots publishes with; null when it does not.
  publish_token: nullable(z.string()),
});
export const SettingsSaved = z.strictObject({
  settings: Settings,
  restart_required: z.boolean(),
});
// Where Microsoft Edge and Node.js (npx) were found (browser spec section 7).
export const BrowserStatus = z.strictObject({
  edge: nullable(z.string()),
  npx: nullable(z.string()),
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
  // Its open questions and pending approvals.
  needs_you: z.number().int(),
  // `needs_you`, plus its runs that ended and its notes posted since it was last seen.
  unread: z.number().int(),
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
// An MCP server whose tool list waits for review in a scope; its tools ask there until it is
// accepted. A server can be listed once per scope.
export const ToolsChanged = z.strictObject({
  server_id: z.string(),
  name: z.string(),
  // `personal` or an org id: accepting accepts that scope's list.
  scope: z.string(),
  // The dots of that scope that use it.
  dots: z.array(DotRef),
});
export const Inbox = z.strictObject({
  approvals: z.array(PendingApproval),
  review_runs: z.array(ReviewRun),
  // Newest failure first.
  attention: z.array(NeedsAttention),
  // Personal's first, then each joined org's; by server name within a scope.
  tools_changed: z.array(ToolsChanged),
  // Watchers Dots paused, newest first.
  watchers: z.array(PausedWatcher),
  // Background runs held for Claude usage, oldest first.
  held: z.array(HeldRun),
  // Apps dots sent for review that wait for the person, newest first. Lazy: the proposal's
  // schema is declared with the apps, below.
  app_reviews: z.array(z.lazy(() => PendingAppReview)),
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
// Where an app came from (app library spec section 8).
export const AppOrigin = z.discriminatedUnion("kind", [
  z.strictObject({ kind: z.literal("bundled"), release: z.string() }),
  z.strictObject({ kind: z.literal("proposed") }),
]);
// An app in the Library (app library spec sections 4 and 6); Dots keeps its code and data.
export const AppConfig = z.strictObject({
  title: z.string(),
  description: z.string(),
  version: z.number(),
  // Kept for "Roll back" while the update after it ran no migrations.
  previous_version: nullable(z.number()),
  // The highest version Dots has given it; a roll back keeps it (0: the active version).
  last_version: z.number(),
  every_dot: z.boolean(),
  // "Can start dots": off until the person switches it on.
  can_start_dots: z.boolean(),
  origin: AppOrigin,
  // A bundled app the person changed: Dots no longer updates it.
  changed: z.boolean(),
});
// What an app may reach outside its data; v1 has one grant.
export const Permission = z.enum(["start_dots"]);
// Who called an app: the person on its screen, a dot in a run, or Dots itself (`on_run_end`).
export const AppActor = z.discriminatedUnion("kind", [
  z.strictObject({ kind: z.literal("person") }),
  z.strictObject({ kind: z.literal("dot"), id: z.string(), name: z.string(), run_id: z.string() }),
  z.strictObject({ kind: z.literal("dots") }),
]);
// One file of an app: its text when it is UTF-8, else its bytes in base64.
export const AppFile = z.strictObject({
  path: z.string(),
  text: nullable(z.string()),
  base64: nullable(z.string()),
});
// A logged call: every call of a tool that writes, and of `on_run_end`.
export const AppCall = z.strictObject({
  id: z.string(),
  app_id: z.string(),
  tool: z.string(),
  actor: AppActor,
  run_id: nullable(z.string()),
  input: z.unknown(),
  // The call's `ctx.now`.
  now: z.string(),
  // The ids `ctx.id()` gave.
  ids: z.array(z.string()),
  result: z.unknown(),
  error: nullable(z.string()),
  changes: z.number(),
  took_ms: z.number(),
  at: z.string(),
});
export const TestResult = z.strictObject({
  file: z.string(),
  // Empty for a file that could not run.
  name: z.string(),
  passed: z.boolean(),
  error: nullable(z.string()),
});
export const ProposalStatus = z.enum(["pending", "added", "changes_asked", "discarded"]);
// Data a removed app of the same name kept that fits a new app's migrations: the review card
// offers to start on it.
export const KeptOffer = z.strictObject({
  id: z.string(),
  title: z.string(),
  removed_at: z.string(),
  bytes: z.number(),
  // The new app's migrations that would run on it.
  migrations: z.array(z.string()),
});
// A tool of an app, as a review card and the Apps screen list it.
export const ToolSummary = z.strictObject({
  name: z.string(),
  description: z.string(),
  writes: z.boolean(),
});
// What a review card's summary shows.
export const ProposalSummary = z.strictObject({
  title: z.string(),
  description: z.string(),
  tools: z.array(ToolSummary),
  // Empty for "Nothing".
  permissions: z.array(Permission),
  tests: z.strictObject({ results: z.array(TestResult) }),
  files: z.number(),
  bytes: z.number(),
  // The migrations accepting it runs: every one for a new app, the new ones for a change.
  migrations: z.array(z.string()),
  // The version a change was checked against; null for a new app.
  update_from: nullable(z.number()),
  kept_data: nullable(KeptOffer),
});
// An app a dot proposed, and what the person decided (app library spec section 6.2).
export const AppProposal = z.strictObject({
  id: z.string(),
  dot_id: z.string(),
  run_id: z.string(),
  root_run_id: z.string(),
  app_name: z.string(),
  app_id: nullable(z.string()),
  base_version: nullable(z.number()),
  note: z.string(),
  summary: ProposalSummary,
  status: ProposalStatus,
  answer: nullable(z.string()),
  answer_run_id: nullable(z.string()),
  created_at: z.string(),
  decided_at: nullable(z.string()),
});
// An app proposal that waits for the person, as the Inbox lists it.
export const PendingAppReview = z.strictObject({
  dot_id: z.string(),
  dot_name: z.string(),
  proposal: AppProposal,
});
export type PendingAppReview = z.infer<typeof PendingAppReview>;
// The person's answer on a review card.
export const ProposalDecision = z.discriminatedUnion("kind", [
  z.strictObject({ kind: z.literal("add") }),
  // "Add to library" on the kept data the card offered.
  z.strictObject({ kind: z.literal("add_with_kept_data") }),
  z.strictObject({ kind: z.literal("ask_for_changes"), text: z.string() }),
  z.strictObject({ kind: z.literal("discard") }),
]);

// An item's config with its kind: what `library_add` and `library_update` take.
export const LibraryConfig = z.discriminatedUnion("kind", [
  z.strictObject({ kind: z.literal("mcp_server"), config: McpServerConfig }),
  z.strictObject({ kind: z.literal("skill"), config: SkillConfig }),
  z.strictObject({ kind: z.literal("plugin"), config: PluginConfig }),
  z.strictObject({ kind: z.literal("context"), config: ContextConfig }),
  z.strictObject({ kind: z.literal("app"), config: AppConfig }),
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
    z.strictObject({
      kind: z.literal("app"),
      config: AppConfig,
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
// An MCP server's catalog in one scope: `personal` or an org id.
export const ScopedCatalog = z.strictObject({
  scope: z.string(),
  catalog: McpCatalog,
  // A catalog row exists in that scope (a Check or a run made it); none reads `unchecked`.
  stored: z.boolean(),
  // The dots of that scope that use the server, whose runs read this catalog.
  used_by: z.array(DotRef),
});
export const LibraryEntry = libraryItemOf({
  // The dots that use it; for a plugin's server, the dots that use the plugin.
  used_by: z.array(DotRef),
  // A server's Personal catalog.
  catalog: nullable(McpCatalog),
  // A server's catalog in Personal, then in each joined org by name; empty for other items.
  catalogs: z.array(ScopedCatalog),
  // The origins confirmed for an http server, by scope; empty for other items.
  confirmed_origins: z.record(z.string(), z.array(z.string())),
  // The `${VAR}` of a server's config that the environment of Dots does not set.
  missing_variables: z.array(z.string()),
  skill: nullable(SkillDetail),
  plugin: nullable(PluginDetail),
  context: nullable(ContextDetail),
  // Why a skill, plugin or context folder could not be read.
  problem: nullable(z.string()),
  // A skill whose folder is in Dots' skills folder: removing it deletes the folder too.
  dots_folder: z.boolean(),
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
// What the Create skill dialog asks for: a first draft of `idea`, or `current` changed as
// `change` says.
export const DraftRequest = z.strictObject({
  idea: z.string(),
  model: z.string(),
  current: nullable(z.string()),
  change: nullable(z.string()),
});
// The name and description the Library will show for a draft that can be saved.
export const DraftSummary = z.strictObject({
  name: z.string(),
  description: z.string(),
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
export const EntryKind = z.enum(["plain", "secret"]);
// A personal variable of the Env tab. A secret's value never leaves the core: it is null.
export const EnvEntry = z.strictObject({
  name: z.string(),
  kind: EntryKind,
  value: nullable(z.string()),
  created_at: z.string(),
  updated_at: z.string(),
  // The Personal dots that grant this name.
  used_by: z.array(DotRef),
  // A secret whose value is gone from Credential Manager: it must be set again.
  value_missing: z.boolean(),
});

// Orgs (env vault spec §6 and §9). No answer ever holds a secret value or sign-in details.
// The person's sign-in details for an org's source, sent to `org_join` and `org_update`.
export const SourceCreds = z.discriminatedUnion("kind", [
  // An OpenBao or Vault token.
  z.strictObject({ kind: z.literal("token"), token: z.string() }),
  // A username and password for a `userpass` or `ldap` mount.
  z.strictObject({ kind: z.literal("login"), username: z.string(), password: z.string() }),
  // An Infisical machine identity.
  z.strictObject({
    kind: z.literal("machine"),
    client_id: z.string(),
    client_secret: z.string(),
  }),
  // This PC's age identity for a SOPS file.
  z.strictObject({ kind: z.literal("age"), identity: z.string() }),
]);
// How an org's dots may read Personal entries: never, when the org lacks the name, or before
// the org's entry.
export const PersonalMode = z.enum(["deny", "fallback", "override"]);
// The names an org's dots may be granted: a name matches an `allow` glob and no `deny` glob
// (deny wins). A glob's `*` stands for any run of characters, and case does not matter.
export const UseRules = z.strictObject({ allow: z.array(z.string()), deny: z.array(z.string()) });
// This PC's place on an account's signed member list.
export const Membership = z.enum(["legacy", "waiting", "active", "removed"]);
// A role on an account's member list.
export const Role = z.enum(["owner", "admin", "member", "viewer"]);
const orgSummaryShape = {
  id: z.string(),
  name: z.string(),
  // `openbao`, `infisical` or `sops`; empty when the org's settings are damaged.
  source_kind: z.string(),
  // The address, the site or the file location.
  address: z.string(),
  // The policy key's id, 16 upper-case hex digits.
  key_fingerprint: z.string(),
  // The status in plain words, with why the last fetch failed.
  status_text: z.string(),
  // The last good fetch, RFC 3339.
  last_fetch_at: nullable(z.string()),
  // Until when the offline copy may be used; null without one.
  copy_until: nullable(z.string()),
  // The accepted policy in plain words, a line per field.
  policy_summary: z.array(z.string()),
  // The accepted policy's `personal` mode: whether the org's dots may read Personal entries;
  // null while no policy is accepted.
  personal_mode: nullable(PersonalMode),
  // The accepted policy's `shell_secrets`: whether a dot's shell may be given a secret.
  shell_secrets: z.boolean(),
  // The accepted policy's `use` globs; null while no policy is accepted.
  use_rules: nullable(UseRules),
  // This PC's place on the account's member list.
  membership: Membership,
  // This PC's role on the list, while active.
  role: nullable(Role),
  // The `seq` of the last member list accepted; null before the first.
  roster_seq: nullable(z.number()),
  // Why the last member list fetched was not accepted, while the one before it is kept.
  roster_error: nullable(z.string()),
  // How many members the kept list has; null when no list is kept.
  member_count: nullable(z.number()),
  // The words a run of the account is refused with while this PC is not on the member list
  // (waiting, its Dots ID changed, or removed); null otherwise.
  not_member_text: nullable(z.string()),
  // The request code this PC made for the account, while it waits to be added; null otherwise.
  request_code: nullable(z.string()),
};
// This install's Dots ID. `name` and `email` are empty until the person sets them.
export const Identity = z.strictObject({
  // The public key, in base64: what the person shares to be added to an account.
  public_key: z.string(),
  // The short form of the public key people compare by eye.
  fingerprint: z.string(),
  name: z.string(),
  email: z.string(),
});
// The account and workspace the person works in now.
export const CurrentScope = z.strictObject({ account: z.string(), workspace: z.string() });
// An org as Settings shows it, by its status.
export const OrgSummary = z.discriminatedUnion("status", [
  z.strictObject({ status: z.literal("connected"), ...orgSummaryShape }),
  // The source could not be reached; the copy is used until `until`, null without one.
  z.strictObject({ status: z.literal("offline"), until: nullable(z.string()), ...orgSummaryShape }),
  z.strictObject({ status: z.literal("revoked"), ...orgSummaryShape }),
  z.strictObject({ status: z.literal("policy_invalid"), error: z.string(), ...orgSummaryShape }),
  z.strictObject({ status: z.literal("expired"), ...orgSummaryShape }),
  // A SOPS join waiting for the admin to add this PC's key.
  z.strictObject({ status: z.literal("pending"), ...orgSummaryShape }),
]);
// How an OpenBao or Vault join code signs in: a token, or a username and password against a
// `userpass` or `ldap` mount.
export const BaoAuth = z.enum(["token", "userpass", "ldap"]);
// The invite a version 2 join code offers: which invite, the role it gives, when it ends.
export const InviteOffer = z.strictObject({
  id: z.string(),
  role: Role,
  expires_at: z.string(),
});
// What a join code would join, or change for an org joined already (Update). Nothing is
// fetched for it.
export const JoinPreview = z.strictObject({
  org_id: z.string(),
  name: z.string(),
  source_kind: z.string(),
  address: z.string(),
  key_fingerprint: z.string(),
  // How an OpenBao code signs in, so Join asks for a token or a username and password; null
  // for Infisical (a machine identity) and SOPS (this PC's key).
  auth: nullable(BaoAuth),
  existing: z.boolean(),
  // For Update, what the code changes, in plain words; a new policy key gets a warning.
  changes: z.array(z.string()),
  // For Update, the code brings another policy key; the last line of changes warns of it.
  key_changed: z.boolean(),
  // For Update, the code changes an OpenBao or Infisical source's settings, or the kind of
  // source: the saved sign-in details are not sent to it, so they must be given again.
  source_changed: z.boolean(),
  // The role the code offers as a personal invite; null for a plain code.
  invite: nullable(InviteOffer),
});
// A member's role in one workspace.
export const WorkspaceRole = z.enum(["admin", "member", "viewer"]);
// One PC (Dots ID) of a member; `signs` is true for one of the policy's admins.
export const KeyView = z.strictObject({
  key: z.string(),
  fingerprint: z.string(),
  is_me: z.boolean(),
  signs: z.boolean(),
});
export const MemberView = z.strictObject({
  id: z.string(),
  name: z.string(),
  email: z.string(),
  role: Role,
  keys: z.array(KeyView),
  workspaces: z.record(z.string(), WorkspaceRole),
  added_at: z.string(),
  // An Owner: only the owner's key, in dots-admin, may change them.
  owner_locked: z.boolean(),
});
export const InviteState = z.enum(["open", "used", "expired", "revoked"]);
export const InviteView = z.strictObject({
  id: z.string(),
  email: z.string(),
  role: Role,
  created_at: z.string(),
  expires_at: z.string(),
  state: InviteState,
});
// A signed member list that was not published, with the reason.
export const DraftView = z.strictObject({
  base_seq: z.number(),
  seq: z.number(),
  summary: z.string(),
  error: z.string(),
  created_at: z.string(),
  // The signed list, to copy into a SOPS file.
  signed: z.string(),
});
// The Members tab of an org (`member_list`).
export const MembersView = z.strictObject({
  seq: z.number(),
  members: z.array(MemberView),
  can_manage: z.boolean(),
  can_sign: z.boolean(),
  sign_blocker: nullable(z.string()),
  publishes: z.boolean(),
  invites: z.array(InviteView),
  draft: nullable(DraftView),
});
// A new invite and the join code that carries it (`invite_create`).
export const CreatedInvite = z.strictObject({
  invite: InviteView,
  code: z.string(),
});
// The member a request's email names; approving adds the PC to them.
export const JoinsMember = z.strictObject({
  id: z.string(),
  name: z.string(),
  role: Role,
});
// What a request code asks (`request_inspect`); `problem` says why it can't be approved.
export const RequestPreview = z.strictObject({
  name: z.string(),
  email: z.string(),
  key: z.string(),
  fingerprint: z.string(),
  invite: nullable(InviteView),
  role: nullable(Role),
  joins_member: nullable(JoinsMember),
  age_recipient: nullable(z.string()),
  problem: nullable(z.string()),
});
// One change to the member list, sent to `member_edit`.
export const RosterEdit = z.discriminatedUnion("kind", [
  z.strictObject({
    kind: z.literal("approve"),
    request: z.string(),
    role: nullable(Role),
    confirm_no_invite: z.boolean(),
  }),
  z.strictObject({ kind: z.literal("set_role"), member_id: z.string(), role: Role }),
  z.strictObject({ kind: z.literal("remove"), member_id: z.string() }),
  z.strictObject({ kind: z.literal("remove_key"), member_id: z.string(), key: z.string() }),
]);
// How a member-list edit ended: published (the org's new row), or kept as a draft.
export const EditOutcome = z.discriminatedUnion("outcome", [
  z.strictObject({ outcome: z.literal("published"), summary: OrgSummary }),
  z.strictObject({ outcome: z.literal("draft"), draft: DraftView }),
]);
// An org's entry for the Env tab. A secret's value is always null.
export const OrgEntry = z.strictObject({
  name: z.string(),
  kind: EntryKind,
  value: nullable(z.string()),
  // A Personal entry of the same name cannot take its place.
  locked: z.boolean(),
  // The org's dots that grant this name.
  used_by: z.array(DotRef),
});
// Whether a dot's grant resolves in its scope now, or why not.
export const GrantState = z.enum([
  "ok",
  "not_granted",
  "not_defined",
  "denied_by_policy",
  "shell_secrets_forbidden",
  "org_offline",
  "org_expired",
  "org_revoked",
  "policy_invalid",
  "org_removed",
  // This PC is not on the account's member list: waiting to be added, or removed.
  "not_member",
]);
// Where a value comes from: the dot's org, or Personal.
export const ValueSource = z.enum(["org", "personal"]);
// One grant of a dot as its editor shows it (`dot_variables`). It never holds a value.
export const GrantStatus = z.strictObject({
  // `mcp:<server>` or `shell`.
  consumer: z.string(),
  name: z.string(),
  status: GrantState,
  // Why it does not resolve, in the words of the run event; null when it resolves.
  text: nullable(z.string()),
  // Where its value comes from, or would but for the shell's secrets rule.
  source: nullable(ValueSource),
  // The value that resolves, or would but for the shell's secrets rule, is a secret.
  secret: z.boolean(),
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
export const CmdErrorKind = z.enum(["not_found", "invalid", "not_allowed", "conflict", "internal", "cancelled"]);
export const CmdError = z.strictObject({ kind: CmdErrorKind, message: z.string() });
export const StartupError = z.strictObject({
  message: z.string(),
  log_dir: z.string(),
  backup: z.string(),
  // The version Go back installs after a failed update; null when there is none.
  go_back: nullable(z.string()),
});

/// What an install is for: a new version, the same one again, or the one before the update.
export const Purpose = z.enum(["update", "repair", "go_back"]);

/// What the update strip shows (apps/dots-app/src/update.rs).
export const UpdateNote = z.discriminatedUnion("state", [
  z.strictObject({ state: z.literal("none") }),
  z.strictObject({
    state: z.literal("ready"),
    version: z.string(),
    working: z.array(z.string()),
    error: nullable(z.string()),
  }),
  z.strictObject({
    state: z.literal("waiting"),
    version: z.string(),
    waiting_for: z.array(z.string()),
    purpose: Purpose,
  }),
  z.strictObject({ state: z.literal("installing"), version: z.string(), purpose: Purpose }),
  z.strictObject({ state: z.literal("failed"), error: z.string() }),
  // The new version did not start properly; back_to is the version Go back installs.
  z.strictObject({
    state: z.literal("unhealthy"),
    version: z.string(),
    back_to: nullable(z.string()),
  }),
  z.strictObject({
    state: z.literal("withdrawn"),
    version: z.string(),
    back_to: nullable(z.string()),
  }),
  z.strictObject({ state: z.literal("updated_itself"), version: z.string(), at: z.string() }),
]);

/// Settings › Updates (updates UI spec section 4.1).
export const UpdateStatus = z.strictObject({
  version: z.string(),
  checks_on: z.boolean(),
  checked_at: nullable(z.string()),
  ready: nullable(z.string()),
  last_error: nullable(z.string()),
  go_back: nullable(z.string()),
  auto_install: z.boolean(),
  publisher: z.boolean(),
  // Why the last install, repair or go back did not happen.
  install_error: nullable(z.string()),
});

/// Whether this Dots publishes (updates UI spec section 5.1).
export const PublisherState = z.discriminatedUnion("state", [
  z.strictObject({ state: z.literal("none") }),
  z.strictObject({ state: z.literal("checking") }),
  z.strictObject({ state: z.literal("publisher"), login: z.string() }),
  z.strictObject({ state: z.literal("refused"), reason: z.string() }),
]);
export const PublisherStatus = z.strictObject({
  token: nullable(z.string()),
  state: PublisherState,
});
export const PullOpened = z.strictObject({ number: z.number().int(), url: z.string() });

/// The Releases page (updates UI spec section 5).
export const Setup = z.discriminatedUnion("state", [
  z.strictObject({ state: z.literal("done") }),
  z.strictObject({ state: z.literal("needed") }),
  // The secrets are in; the pull request with the public key is what is left.
  z.strictObject({ state: z.literal("key_pending") }),
  z.strictObject({
    state: z.literal("half_done"),
    secrets: z.number().int(),
    pubkey: z.boolean(),
  }),
]);
export const MergedPull = z.strictObject({ number: z.number().int(), title: z.string() });
export const ReleaseInfo = z.strictObject({
  id: z.number().int(),
  tag: z.string(),
  name: z.string(),
  body: z.string(),
  prerelease: z.boolean(),
  draft: z.boolean(),
  url: z.string(),
  published_at: nullable(z.string()),
});
export const JobInfo = z.strictObject({
  name: z.string(),
  status: z.string(),
  conclusion: nullable(z.string()),
  steps: z.array(z.tuple([z.string(), z.string(), nullable(z.string())])),
});
export const RunInfo = z.strictObject({
  id: z.number().int(),
  status: z.string(),
  conclusion: nullable(z.string()),
  url: z.string(),
  jobs: z.array(JobInfo),
});
export const Stage = z.discriminatedUnion("stage", [
  z.strictObject({ stage: z.literal("waiting") }),
  z.strictObject({ stage: z.literal("testing") }),
  z.strictObject({ stage: z.literal("building") }),
  z.strictObject({ stage: z.literal("signing") }),
  z.strictObject({ stage: z.literal("published") }),
  z.strictObject({ stage: z.literal("failed"), step: z.string() }),
]);
export const Trial = z.strictObject({
  version: z.string(),
  rc: z.number().int(),
  tag: z.string(),
  release: nullable(ReleaseInfo),
  run: nullable(RunInfo),
  stage: Stage,
  newer_on_master: z.number().int(),
  notes: z.string(),
});
export const Promotion = z.strictObject({
  version: z.string(),
  tag: z.string(),
  run: nullable(RunInfo),
  stage: Stage,
});
export const PageState = z.strictObject({
  setup: Setup,
  live: nullable(ReleaseInfo),
  trial: nullable(Trial),
  promotion: nullable(Promotion),
  changes: z.array(MergedPull),
  suggested: z.string(),
  // Each version's next trial number.
  next_rcs: z.record(z.string(), z.number().int()),
});

/// What "Check for updates" found.
export const CheckOutcome = z.discriminatedUnion("state", [
  z.strictObject({ state: z.literal("off") }),
  z.strictObject({ state: z.literal("up_to_date") }),
  z.strictObject({ state: z.literal("ready"), version: z.string() }),
  z.strictObject({ state: z.literal("failed"), error: z.string() }),
]);

/// A version's notes ("What's new").
export const ReleaseNotes = z.strictObject({
  version: z.string(),
  body: z.string(),
  url: z.string(),
});

/// Validation limits of the core that the dot editor and Settings mirror, and the attempts a
/// scheduled occurrence has at most, which the Dots list and the run view name. The contract
/// test checks them against the Rust constants.
export const LIMITS = {
  max_timeout_secs: 604800,
  max_approval_wait_secs: 280,
  max_concurrent_runs: 16,
  max_schedule_attempts: 6,
  max_global_instructions: 32768,
  max_memories: 200,
  max_memory_text: 500,
  max_watcher_name: 80,
} as const;

export type EngineKind = z.infer<typeof EngineKind>;
export type LimitReading = z.infer<typeof LimitReading>;
export type LimitsState = z.infer<typeof LimitsState>;
export type HoldChange = z.infer<typeof HoldChange>;
export type UsagePeriod = z.infer<typeof UsagePeriod>;
export type UsageSummary = z.infer<typeof UsageSummary>;
export type UsageRow = z.infer<typeof UsageRow>;
export type UsageRun = z.infer<typeof UsageRun>;
export type HoldReason = z.infer<typeof HoldReason>;
export type HeldRun = z.infer<typeof HeldRun>;
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
export type BrowserStatus = z.infer<typeof BrowserStatus>;
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
// An installed app as the Apps screen lists it.
export const AppRow = z.strictObject({
  item: LibraryItem,
  // The names of the dots that use it.
  dots: z.array(z.string()),
  // Why its code cannot be read: it is left out of runs until it can.
  error: nullable(z.string()),
  // The tools its screen may call; none while its code cannot be read.
  tools: z.array(ToolSummary),
  // What it asks to reach outside its data: "start_dots" for "Can start dots".
  permissions: z.array(Permission),
});
export type AppRow = z.infer<typeof AppRow>;
// Data a removed app kept (app library spec section 10).
export const KeptData = z.strictObject({
  id: z.string(),
  name: z.string(),
  title: z.string(),
  removed_at: z.string(),
  bytes: z.number(),
});
export type KeptData = z.infer<typeof KeptData>;
export type AppFile = z.infer<typeof AppFile>;
export type AppCall = z.infer<typeof AppCall>;
export type AppProposal = z.infer<typeof AppProposal>;
export type ProposalDecision = z.infer<typeof ProposalDecision>;
export type McpTool = z.infer<typeof McpTool>;
export type McpCatalog = z.infer<typeof McpCatalog>;
export type SkillDetail = z.infer<typeof SkillDetail>;
export type PluginDetail = z.infer<typeof PluginDetail>;
export type LibraryEntry = z.infer<typeof LibraryEntry>;
export type ContextConfig = z.infer<typeof ContextConfig>;
export type AppConfig = z.infer<typeof AppConfig>;
export type AppOrigin = z.infer<typeof AppOrigin>;
export type ContextDocuments = z.infer<typeof ContextDocuments>;
export type McpProbe = z.infer<typeof McpProbe>;
export type ScannedSkill = z.infer<typeof ScannedSkill>;
export type DraftRequest = z.infer<typeof DraftRequest>;
export type DraftSummary = z.infer<typeof DraftSummary>;
export type InstalledPlugin = z.infer<typeof InstalledPlugin>;
export type EnvVariable = z.infer<typeof EnvVariable>;
export type EntryKind = z.infer<typeof EntryKind>;
export type EnvEntry = z.infer<typeof EnvEntry>;
export type ScopedCatalog = z.infer<typeof ScopedCatalog>;
export type SourceCreds = z.infer<typeof SourceCreds>;
export type Identity = z.infer<typeof Identity>;
export type CurrentScope = z.infer<typeof CurrentScope>;
export type OrgSummary = z.infer<typeof OrgSummary>;
export type Role = z.infer<typeof Role>;
export type BaoAuth = z.infer<typeof BaoAuth>;
export type JoinPreview = z.infer<typeof JoinPreview>;
export type MembersView = z.infer<typeof MembersView>;
export type MemberView = z.infer<typeof MemberView>;
export type InviteView = z.infer<typeof InviteView>;
export type DraftView = z.infer<typeof DraftView>;
export type CreatedInvite = z.infer<typeof CreatedInvite>;
export type RequestPreview = z.infer<typeof RequestPreview>;
export type RosterEdit = z.infer<typeof RosterEdit>;
export type EditOutcome = z.infer<typeof EditOutcome>;
export type OrgEntry = z.infer<typeof OrgEntry>;
export type PersonalMode = z.infer<typeof PersonalMode>;
export type UseRules = z.infer<typeof UseRules>;
export type GrantState = z.infer<typeof GrantState>;
export type ValueSource = z.infer<typeof ValueSource>;
export type GrantStatus = z.infer<typeof GrantStatus>;
export type FileDiff = z.infer<typeof FileDiff>;
export type WorktreeDiff = z.infer<typeof WorktreeDiff>;
export type CmdErrorKind = z.infer<typeof CmdErrorKind>;
export type CmdError = z.infer<typeof CmdError>;
export type StartupError = z.infer<typeof StartupError>;
export type MessageKind = z.infer<typeof MessageKind>;
export type FollowUpStatus = z.infer<typeof FollowUpStatus>;
export type Message = z.infer<typeof Message>;
export type FollowUp = z.infer<typeof FollowUp>;
export type MemorySource = z.infer<typeof MemorySource>;
export type Memory = z.infer<typeof Memory>;
export type WatcherKind = z.infer<typeof WatcherKind>;
export type PausedReason = z.infer<typeof PausedReason>;
export type FolderEvent = z.infer<typeof FolderEvent>;
export type GithubEvent = z.infer<typeof GithubEvent>;
export type FolderConfig = z.infer<typeof FolderConfig>;
export type EmailConfig = z.infer<typeof EmailConfig>;
export type GithubConfig = z.infer<typeof GithubConfig>;
export type WatcherConfig = z.infer<typeof WatcherConfig>;
export type Watcher = z.infer<typeof Watcher>;
export type PausedWatcher = z.infer<typeof PausedWatcher>;
export type UpdateNote = z.infer<typeof UpdateNote>;
export type UpdateStatus = z.infer<typeof UpdateStatus>;
export type CheckOutcome = z.infer<typeof CheckOutcome>;
export type ReleaseNotes = z.infer<typeof ReleaseNotes>;
export type PublisherStatus = z.infer<typeof PublisherStatus>;
export type PageState = z.infer<typeof PageState>;
export type Stage = z.infer<typeof Stage>;
export type Trial = z.infer<typeof Trial>;
