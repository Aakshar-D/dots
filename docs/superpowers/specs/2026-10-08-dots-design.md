# Dots — Design Spec

Date: 2026-10-08
Status: Draft, pending review

## 1. Purpose

Dots is a local, always-on agent platform for one user on one Windows machine, modeled on
OpenAI Codex "dots". A *dot* is a named agent with standing instructions, a workspace, a model
backend, a permission policy, and triggers. Dots wake on a schedule, a webhook, or a manual
action; do their work unattended; and bring results (summaries, diffs, branches, proposed
actions) back to a review inbox. Nothing outside a dot's allowed tool set happens without the
user's approval.

### Success criteria

- A dot configured with a cron schedule runs unattended while the app sits in the tray, and its
  result appears in the inbox with a Windows notification.
- A dot can be triggered by `POST` to a local webhook URL with its token.
- A Claude-backed dot runs on the user's Claude subscription via the official `claude` CLI.
- A local-model dot runs against Ollama / LM Studio (any OpenAI-compatible endpoint).
- A tool call that the dot's policy marks "ask" reaches the inbox; approving it lets the dot
  continue (live, or by resuming later); denying it stops or redirects the dot.
- Code-changing dots work in an isolated git worktree and never touch the user's checkout.

### Decisions made during brainstorming

| Topic | Decision |
|---|---|
| Scope | General platform: many dots, each with its own job, tools, schedule |
| Hosting | Local Windows machine |
| Model backends | Hybrid: Claude via `claude` CLI (subscription); local models via own tool loop over OpenAI-compatible API |
| Triggers (v1) | Schedule (cron), webhook, manual (Run now + chat) |
| Autonomy | Per-dot policy: each tool is `allow`, `ask`, or `deny`; new dots default to "sandboxed" preset |
| Language | Rust |
| UI | Tauri 2 desktop app, React + Vite frontend |
| Process model | Single process: Tauri app hosts the runtime; close-to-tray; autostart at login |
| Location | Standalone repo at `C:\dev\dots` (outside OneDrive) |

### Out of scope for v1

Watchers (polling GitHub/SF/folders), connecting remote machines, multi-user, cloud hosting,
MCP servers for local-engine dots, a CLI client, a separate daemon process.

### Constraint: subscription use

A Claude subscription is used only through the official `claude` CLI, logged in as the user,
for the user's own work. Dots never extract or reuse the CLI's OAuth credentials in a custom
HTTP client. The Claude engine is therefore a CLI process driver, not an API client.

## 2. Architecture

```
dots/
  Cargo.toml                 workspace
  crates/dots-core/          library: all logic, no UI dependency
    src/store/               SQLite via sqlx, embedded migrations
    src/scheduler/           tokio-cron-scheduler -> enqueue runs
    src/server/              axum on 127.0.0.1:<port>: webhooks + MCP permission endpoint
    src/runner/              queue, concurrency, timeouts, cancellation, restart recovery
    src/engine/mod.rs        Engine trait + EngineEvent
    src/engine/claude.rs     drives `claude -p` with stream-json I/O
    src/engine/local.rs      own tool-use loop over OpenAI-compatible chat completions
    src/tools/               built-in tools for the local engine
    src/policy/              allow/ask/deny resolution, one-shot grants
    src/workspace/           git worktree or plain folder per run
    src/notify.rs            Notifier trait (implemented by the app)
  crates/fake-claude/        test binary that mimics `claude -p` stream-json output
  apps/dots-app/             Tauri 2 shell: starts runtime, tray, autostart, commands, events
  ui/                        React + Vite + TypeScript
  docs/
```

### Boundaries

- `dots-core` exposes a `Runtime` handle: `start(config) -> Runtime`, plus methods for dot CRUD,
  triggering runs, cancelling, deciding approvals, and a broadcast channel of `RuntimeEvent`s.
  It has no Tauri dependency and is fully testable headless.
- `dots-app` is a thin adapter: maps Tauri commands to `Runtime` methods, forwards
  `RuntimeEvent`s to the webview as Tauri events, implements `Notifier` with
  `tauri-plugin-notification`, owns the tray icon and `tauri-plugin-autostart`.
- The UI talks only to Tauri commands/events. It never calls the local HTTP server.
- Both engines implement:

```rust
#[async_trait]
pub trait Engine: Send + Sync {
    async fn start(&self, ctx: RunContext) -> Result<EngineHandle>;
    async fn resume(&self, ctx: RunContext, input: ResumeInput) -> Result<EngineHandle>;
}
// EngineHandle: stream of EngineEvent + cancel()
pub enum EngineEvent {
    SessionStarted { session_id: String },
    AssistantText { text: String },
    ToolCall { id: String, tool: String, input: serde_json::Value },
    ToolResult { id: String, output: String, is_error: bool },
    Usage { tokens_in: u64, tokens_out: u64 },
    Finished { summary: String },
    Failed { error: String },
}
```

- The policy gate lives outside the engines. The Claude engine reaches it through the MCP
  permission endpoint; the local engine calls it directly before each tool execution.

## 3. Data model (SQLite)

Stored at `%LOCALAPPDATA%\dots\dots.db`. Worktrees at `%LOCALAPPDATA%\dots\worktrees\`.

**dots**
- `id` TEXT PK (ULID), `name` TEXT UNIQUE, `instructions` TEXT (markdown)
- `engine` TEXT (`claude` | `local`), `model` TEXT, `endpoint_url` TEXT NULL (local only)
- `workdir` TEXT, `workspace_mode` TEXT (`worktree` | `folder`)
- `schedule` TEXT NULL (cron, 6-field with seconds), `timezone` TEXT (default local)
- `webhook_token` TEXT (random 32 bytes, base64url)
- `policy` TEXT (JSON, see §5), `mcp_servers` TEXT NULL (JSON, Claude only)
- `use_user_settings` INTEGER (0/1, default 0)
- `max_turns` INTEGER (default 40), `timeout_secs` INTEGER (default 1800),
  `approval_wait_secs` INTEGER (default 600)
- `enabled` INTEGER, `created_at`, `updated_at`

**runs**
- `id` TEXT PK (ULID), `dot_id` FK
- `trigger` TEXT (`schedule` | `webhook` | `manual` | `chat`), `payload` TEXT NULL (JSON)
- `status` TEXT (`queued` | `running` | `awaiting_approval` | `succeeded` | `failed` | `cancelled`)
- `session_id` TEXT NULL (Claude session id, or local conversation key)
- `workspace_path` TEXT NULL, `branch` TEXT NULL
- `summary` TEXT NULL, `error` TEXT NULL
- `tokens_in`, `tokens_out` INTEGER
- `parent_run_id` TEXT NULL (resumes and chat follow-ups link to the previous run)
- `queued_at`, `started_at`, `ended_at`

**run_events**
- `run_id` FK, `seq` INTEGER, `ts`, `kind` TEXT, `data` TEXT (JSON); PK (`run_id`, `seq`)
- The full transcript. Also the source of truth for resuming local-engine conversations.

**approvals**
- `id` TEXT PK, `run_id` FK, `tool` TEXT, `input` TEXT (JSON), `input_hash` TEXT
- `status` TEXT (`pending` | `approved` | `denied` | `expired`)
- `note` TEXT NULL, `created_at`, `decided_at`

**settings** — key/value: `port` (default 47321), `max_concurrent_runs` (default 2),
`claude_path`, `tested_claude_version`, `local_endpoints` (JSON list), `autostart`.

All timestamps are RFC 3339 UTC strings.

## 4. Run lifecycle

### State machine

```
queued -> running -> succeeded
                  -> failed
                  -> cancelled
                  -> awaiting_approval -> (approve / deny-with-note) -> child run queued (parent_run_id)
                                       -> (plain deny) -> parent closed as succeeded
```

A resume creates a child run linked by `parent_run_id` and reuses the parent's workspace and
`session_id`. The UI shows parent and children as one thread.

### Steps

1. **Trigger.** Scheduler tick, `POST /dots/{id}/trigger`, "Run now", or a chat message inserts a
   `queued` run with its payload. A schedule tick for a dot that already has a `queued` run is
   coalesced (dropped, logged).
2. **Dispatch.** The runner starts queued runs FIFO while running count < `max_concurrent_runs`
   and the dot has no other `running` run.
3. **Workspace.**
   - `worktree`: `git -C <workdir> worktree add <data>/worktrees/<run_id> -b dots/<dot-name>/<run_id> <default-branch>`.
     Default branch from `git symbolic-ref refs/remotes/origin/HEAD`, falling back to current `HEAD`.
   - `folder`: use `workdir` directly.
   - `workdir` must exist; `worktree` mode additionally requires a git repo. Otherwise the run
     fails with a clear error.
4. **Prompt.** Dot `instructions` + a trigger block (trigger kind, payload JSON, local time) +
   standing rules: work only inside the workspace; when done, end with a summary listing what
   changed and what needs human review.
5. **Execute.** Engine events are appended to `run_events` and broadcast to the UI.
6. **Finish.** Final assistant text becomes `summary`. Notifier fires on `failed`,
   `awaiting_approval`, and `succeeded`. A worktree with no commits and no uncommitted changes is
   removed (`git worktree remove`, branch deleted); otherwise it is kept for review.
7. **Review actions** (UI): view diff, open folder, discard worktree (remove + delete branch).
   Merging and pushing are the user's job in v1.

### Timeouts, cancellation, recovery

- Each run's child process tree is assigned to a Windows Job Object with
  `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`; cancel/timeout closes the job, killing `claude`/`node`
  descendants and any shell commands.
- On app start, runs left `running` become `failed` with error `interrupted`. `awaiting_approval`
  runs stay resumable; their pending approvals resume on decision.
- On app start, `claude --version` is compared to `tested_claude_version`; a mismatch shows a
  non-blocking warning banner.

## 5. Policy and approvals

### Policy format

```json
{
  "preset": "sandboxed",
  "rules": [
    { "tool": "Read",               "action": "allow" },
    { "tool": "Bash(git push:*)",   "action": "ask" },
    { "tool": "mcp__salesforce__*", "action": "ask" },
    { "tool": "WebFetch",           "action": "deny" }
  ],
  "default": "ask"
}
```

- Tool patterns use Claude Code permission-rule syntax so they pass through unchanged to
  `--allowedTools` / `--disallowedTools`. The local engine matches the same syntax against its
  own tools, which are aliased: `read_file`→`Read`, `write_file`→`Write`, `edit_file`→`Edit`,
  `glob`→`Glob`, `grep`→`Grep`, `list_dir`→`LS`, `shell`→`Bash`.
- Resolution order: one-shot grant for (run lineage, tool, `input_hash`) → `deny` rules →
  `allow` rules → `ask` rules → `default`.
- Presets:
  - `read-only`: allow Read/LS/Glob/Grep; deny Write/Edit/Bash; default deny.
  - `sandboxed` (default): allow Read/LS/Glob/Grep/Write/Edit, `Bash(git status:*)`,
    `Bash(git diff:*)`, `Bash(git add:*)`, `Bash(git commit:*)`; default ask.
  - `trusted`: allow all; ask for `Bash(git push:*)` and `mcp__*`; default allow.

### Claude engine

Spawn per run:

```
claude -p --output-format stream-json --input-format stream-json --verbose
  --model <model> --max-turns <n>
  --allowedTools <allow patterns> --disallowedTools <deny patterns>
  --permission-prompt-tool mcp__dots__approve
  --mcp-config <per-run temp json> --strict-mcp-config
  --setting-sources project            (omitted when use_user_settings = 1)
  [--resume <session_id>]
```

Working directory = run workspace. Env: `MCP_TOOL_TIMEOUT` set to
`(approval_wait_secs + 30) * 1000`. The per-run MCP config contains the dots MCP server
(`http://127.0.0.1:<port>/mcp`, header `Authorization: Bearer <per-run secret>`) plus the dot's
own `mcp_servers`. `--setting-sources project` keeps the user's global hooks (GateGuard,
caveman, memory hooks) out of headless runs; a dot opts back in with `use_user_settings`.

`mcp__dots__approve` handler:
1. Resolve policy. `allow` → return allow. `deny` → return deny with reason.
2. `ask` → insert `approvals` row (`pending`), notify, then wait up to `approval_wait_secs` for a
   decision.
   - Approved in window → return allow with the original input. The run continues.
   - Denied in window → return deny with the user's note.
   - No decision → return deny with message `"Queued for human approval #<id>. Do not retry
     this action. Finish any other work and end with your summary."` and flag the run so that on
     process exit its status becomes `awaiting_approval`.
3. Decision after the window:
   - Approve → insert one-shot grant; queue a child run with `--resume <session_id>` and prompt
     `"Approval #<id> granted. Perform the approved action now, then continue."`
   - Deny with note → queue a child run resuming the session with the note.
   - Plain deny → approval closed; parent run set to `succeeded`.

The stream-json parser maps CLI messages (`system/init`, `assistant` content blocks `text` /
`tool_use`, `user` `tool_result`, final `result`) to `EngineEvent`. Unknown message types are
stored raw and never fatal.

### Local engine

Loop: send messages + tool schemas to `POST {endpoint}/v1/chat/completions` (non-streaming in v1)
→ for each `tool_calls` entry, resolve policy → execute allowed tools, return deny text for
denied ones, park on `ask` using the same wait-window logic → append results → repeat until no
tool calls or `max_turns`. The full message list is persisted in `run_events`, so a resume
rebuilds the conversation and appends the approved tool's result (executed on resume) or the
denial.

Built-in tools: `read_file`, `write_file`, `edit_file` (exact unique-string replace), `list_dir`,
`glob`, `grep`, `shell` (PowerShell, timeout default 120 s, output capped at 30 KB). All paths are
canonicalized and must resolve inside the workspace (symlinks and junctions resolved first);
`shell` runs with cwd = workspace. "Test endpoint" in Settings sends a probe request with one
tool and checks that the model returns a well-formed tool call.

## 6. Local HTTP server

Bound to `127.0.0.1:<port>` only; requests whose peer address is not loopback are rejected.

- `POST /dots/{id}/trigger` — header `Authorization: Bearer <webhook_token>` or `?token=`;
  JSON body (≤ 256 KB) becomes the run payload. Returns `202 { "run_id": ... }`. Disabled dot →
  `409`. Bad token → `401`.
- `POST /mcp` — MCP streamable-HTTP endpoint served with `rmcp`, exposing the single tool
  `approve`. Requires the per-run secret; the secret identifies the run.
- `GET /health` — `200`.

Exposing webhooks beyond localhost (e.g. tunnels) is the user's choice and out of scope.

## 7. UI

React + Vite + TypeScript inside Tauri 2. Screens:

- **Dots** — list: name, engine/model, state, next scheduled run, last result, enable toggle,
  Run now.
- **Dot editor** — name; instructions (markdown); engine, model, endpoint; workdir + mode; cron
  with human-readable preview and next 3 fire times; webhook URL + token (copy, regenerate);
  policy editor (preset picker + rule table); limits; MCP servers (Claude only);
  use-user-settings toggle.
- **Run view** — live transcript (assistant text, collapsible tool calls/results), worktree diff,
  summary, status, tokens; cancel, discard worktree, open folder.
- **Inbox** — pending approvals (tool, pretty-printed input, approve / deny with note) and
  completed runs with changes awaiting review. Unread count mirrored in the tray tooltip.
- **Chat** — per-dot threaded chat; each message is a `chat` run resuming the thread's session.
- **Settings** — port, concurrency, `claude` path and version check, local endpoints with
  "Test endpoint", data dir, autostart toggle.

Window close hides to tray. Tray menu: Open, Inbox (n), Pause all schedules, Quit.

## 8. Error handling

| Failure | Behavior |
|---|---|
| `claude` not found / not logged in | Run `failed` with actionable message; Settings shows status |
| CLI exits non-zero | `failed`, last 4 KB of stderr stored in `error` |
| Unparseable stream-json line | Stored as `run_events` kind `raw`; run continues |
| Local endpoint unreachable / 5xx | Retry 2× with backoff, then `failed` |
| Model returns malformed tool call | Error text returned to model as tool result; counts toward `max_turns` |
| Workspace setup fails | `failed` before engine start; no partial worktree left |
| Timeout | Job Object closed; `failed` with `timeout` |
| App crash/restart mid-run | `failed: interrupted`; worktree kept |
| Port in use | App starts; UI shows error; webhooks disabled and Claude runs blocked (they need the MCP endpoint) until the port is changed |

## 9. Testing

- **Unit (dots-core):** policy resolution and pattern matching; cron parsing / next fire; run state
  transitions; path confinement including `..`, absolute paths, symlinks, junctions; prompt
  assembly.
- **Stream-json parser:** fixtures recorded from real `claude -p` runs (success, tool use,
  permission denial, max turns, error) checked into `crates/dots-core/tests/fixtures/`.
- **fake-claude:** binary accepting the same flags, replaying a scripted transcript, and calling
  the MCP `approve` endpoint at scripted points. Drives end-to-end tests of runner + approval
  flow (approve in window, deny, timeout → park → resume) with no network or subscription use.
- **Local engine:** `wiremock` OpenAI-compatible server returning scripted tool calls; verifies
  the loop, policy gate, and park/resume conversation rebuild.
- **HTTP:** webhook token auth, payload limit, disabled dot, MCP secret validation.
- **UI:** Vitest + Testing Library for editor validation, policy table, inbox actions.
- **Manual smoke:** one Claude dot (real CLI, subscription) and one local dot (Ollama or
  LM Studio), each doing a scheduled run, a webhook run, and an approval round trip.

## 10. Build phases

1. Core skeleton: store + migrations, `Runtime`, runner with a stub engine, workspace manager.
2. Claude engine + stream-json parser + fake-claude + MCP approve endpoint + policy.
3. Scheduler + webhook server.
4. Local engine + built-in tools.
5. Tauri app: tray, autostart, notifications, commands/events.
6. UI screens.
7. Manual smoke and packaging (`tauri build`, NSIS installer).

## 11. Open risks

- `stream-json` and `--permission-prompt-tool` are CLI contracts owned by Claude Code and may
  change; mitigated by recorded fixtures, version pinning, and the startup version check.
- Holding an MCP tool call open for up to 10 minutes depends on `MCP_TOOL_TIMEOUT` being honored
  for HTTP MCP servers; verify in phase 2. Fallback: shorten the wait window to the CLI's limit
  and rely on park/resume.
- Local models vary widely in tool-calling quality; v1 targets simpler jobs on local dots.
