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
  `RuntimeEvent`s to the webview as Tauri events, turns `RuntimeEvent`s into Windows
  notifications with `tauri-plugin-notification` (there is no `Notifier` trait in core; the app
  subscribes to the event bus), owns the tray icon and `tauri-plugin-autostart`. See §12.
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
- `max_turns` INTEGER (default 40), `timeout_secs` INTEGER (default 1800, max 604800),
  `approval_wait_secs` INTEGER (default 240, max 280)
- `workdir` and `workspace_mode` cannot change while the dot has queued, running or
  awaiting runs (conflict).
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
- `reviewed_at` TEXT NULL (migration 0002; set by "Mark reviewed" or "Discard worktree", see
  §12 Inbox)

**run_events**
- `run_id` FK, `seq` INTEGER, `ts`, `kind` TEXT, `data` TEXT (JSON); PK (`run_id`, `seq`)
- The full transcript. Also the source of truth for resuming local-engine conversations.

**approvals**
- `id` TEXT PK, `run_id` FK, `tool` TEXT, `input` TEXT (JSON), `input_hash` TEXT
- `status` TEXT (`pending` | `approved` | `denied` | `expired`)
- `note` TEXT NULL, `created_at`, `decided_at`
- `self_apply` INTEGER (0/1), `apply_state` TEXT NULL (`queued` | `applying` | `applied` |
  `failed` | `unknown` | `skipped`), `applied_at` TEXT NULL (migration 0004; see
  `2026-10-13-approvals-v2-design.md` §3.4)

**settings** — key/value, each value a JSON document: `port` (default 47321),
`max_concurrent_runs` (default 2), `claude_path` (null = search PATH), `tested_claude_version`
(default: the core constant `TESTED_CLAUDE_VERSION`), `local_endpoints` (JSON list of URLs),
`autostart` (default true), `schedules_paused` (default false). Missing keys take their
defaults; unknown keys are ignored. A stored value of the wrong type, or one that validation
rejects (for example `max_concurrent_runs` 0), is ignored on its own and its default is used.

Migrations: `0002` adds `runs.reviewed_at`; `0003` adds the index
`runs(root_run_id, queued_at, id)` that the Inbox query needs.

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

From Approvals v2 an approved, parked approval is resolved in one of two ways, which the
approval itself records (`self_apply`): on an engine that performs approved calls itself (the
local engine) the call is queued and the child run performs it before anything else; on every
other engine a one-shot grant is written and the child run's prompt asks the model to issue
the call again. See `2026-10-13-approvals-v2-design.md` §3.

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
   - A resume or chat follow-up reuses its parent's workspace. If that worktree was removed
     (unchanged) after the parent finished, it is recreated at the same path (the resumed
     session remembers that directory): the branch is reused if it still exists, otherwise
     recreated from the stored base commit.
4. **Prompt.** Dot `instructions` + a trigger block (trigger kind, payload JSON, local time) +
   standing rules: work only inside the workspace; when done, end with a summary listing what
   changed and what needs human review.
5. **Execute.** Engine events are appended to `run_events` and broadcast to the UI.
6. **Finish.** Final assistant text becomes `summary`. The app notifies on `failed`,
   `awaiting_approval`, and `succeeded` (§12). A worktree with no commits and no uncommitted changes is
   removed (`git worktree remove`, branch deleted); otherwise it is kept for review. The same
   cleanup runs when a parked run is closed by a plain deny or cancelled.
7. **Review actions** (UI): view diff, open folder, discard worktree (remove + delete branch).
   Merging and pushing are the user's job in v1.

### Timeouts, cancellation, recovery

- Each run's child process tree is assigned to a Windows Job Object with
  `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`; cancel and timeout terminate the job
  (`TerminateJobObject`), killing `claude`/`node` descendants and any shell commands, and
  closing its handle kills whatever is left (§12, 3c).
- On app start, runs left `running` become `failed` with error `interrupted` and their pending
  approvals are expired in the same transaction. `awaiting_approval` runs stay resumable; their
  pending approvals resume on decision, and awaiting runs whose decisions were parked before
  the restart are resumed during start.
- A resume run that performs its parent's approved calls closes them when it stops running,
  whatever the reason (it ended, parked again, was cancelled before it started, or the app
  restarted while it ran): a call that was claimed and has no outcome becomes `unknown`, a
  call that was not reached becomes `skipped`. Neither is performed later (Approvals v2 §3.4).
- A run is registered with the runner before it is claimed, so a cancel always reaches it; a
  cancel that lands before the engine starts finishes the run `cancelled` without starting it.
- A decision is written together with its `parked` flag; `Live` is reported only when the
  waiting engine actually received it, otherwise the run is resumed (never lost).
- On app start, `claude --version` (5 s timeout, no console window) is compared to
  `tested_claude_version`; a mismatch shows a non-blocking warning banner whose "Mark as tested"
  action stores the detected version as `tested_claude_version`.

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
  `allow` rules → `ask` rules → `default`. From Approvals v2 a grant is written only for a
  dot whose engine does not perform approved calls itself, which is the Claude engine.
- From Approvals v2 an approval can save an allow rule to the dot's policy, in one transaction
  with the decision (`decide_approval` with `allow_rule`). Only a pattern that
  `policy::allow_candidates` derives from that very call is accepted, so the caller cannot
  widen the policy at will; the policy's preset becomes `custom`. The pending approvals of one
  run can also be decided together (`decide_run_approvals`): in the order they were asked,
  with one resume, and without a rule. See `2026-10-13-approvals-v2-design.md` §4 and §5.
- Presets:
  - `read-only`: allow Read/LS/Glob/Grep; deny Write/Edit/Bash; default deny.
  - `sandboxed` (default): allow Read/LS/Glob/Grep, `Write(./**)`, `Edit(./**)`,
    `Bash(git status:*)`, `Bash(git diff:*)`, `Bash(git add:*)`; deny
    `Write(**/.git/**)`, `Edit(**/.git/**)`, `Write(**/.git)`, `Edit(**/.git)`,
    `Write(**/.claude/**)`, `Edit(**/.claude/**)`, `Write(**/.claude)`, `Edit(**/.claude)`;
    default ask. `git commit` is not allowed (it falls to ask) because commit hooks execute
    arbitrary code; `.claude` is denied because the CLI loads settings and hooks from it.
    Resolution is workspace-aware (`resolve_in`): `./` patterns anchor to the run workspace.
    Deny/ask command matching is best-effort; allow matching is strict.
- Path rules on the deny/ask side (pinned for Plan 3):
  - A pattern ending in `/**` also matches the directory itself: `Read(**/.git/**)` covers
    `.git`.
  - A workspace-relative input (as the local engine passes them) is also tested in its absolute
    form `<workspace>/<rel>`, so absolute deny patterns such as `Read(C:/secrets/**)` match local
    dots too. That joined path is normalized first (`..` resolved) and is matched whole, against
    absolute patterns only: a relative rule such as `Write(users/**)` must not match the
    workspace's own location under `C:/Users/...`.
  - The local engine's walking tools (`glob`, `grep`, `list_dir`) skip every entry whose
    workspace-relative path resolves to `deny` under the walking tool's own name or under
    `Read`, and end their output with `(<n> entries hidden by policy)`. A search from the
    workspace root therefore never reads a denied subtree. Entries that resolve to `ask` are not
    skipped (the walking call itself was already judged).
  - Refined in Plan 3c: a *directory* is skipped only when a deny rule matches it; a directory
    that is denied only by the policy's default is still entered, so an allowed subtree stays
    reachable under default deny (`Read(src/**)` allowed, search from the root). A *file* is
    skipped whenever it resolves to `deny`, by rule or by default. A symlink or a junction to a
    directory is judged as a file and is never entered. `list_dir` does not walk: it lists a
    directory that only the default denies by its name, so that the model can reach the allowed
    subtree, and leaves out one that a deny rule matches. The hidden count is one per refused
    entry: a directory matched by a deny rule counts once, whatever it holds. The search root
    is judged like every other entry: a root that is a file is refused whenever it resolves to
    `deny`, a root that is a directory when a deny rule matches it, with an error and no count.
    A rule that names a directory alone (`Read(**/secret)`) does not cover what is below it;
    `Read(secret/**)` does.
  - `trusted`: allow all; ask for `Bash(git push:*)` and `mcp__*`; default allow.

### Claude engine

Spawn per run:

```
claude -p --output-format stream-json --input-format stream-json --verbose
  --model <model> --max-turns <n>
  --allowedTools <allow patterns> --disallowedTools <deny patterns>
  --permission-prompt-tool mcp__dots__approve
  --mcp-config <per-run temp json> --strict-mcp-config
  --setting-sources ""                 ("user,project,local" when use_user_settings = 1)
  [--resume <session_id>]
```

Working directory = run workspace. Env: `MCP_TOOL_TIMEOUT` set to
`(approval_wait_secs + 30) * 1000` on a best-effort basis; CLI 2.1.295 ignores it for the
permission-prompt tool (observed hard cap of about 300 s; CLI 2.1.295 observed 2026-10-09). The per-run MCP config contains the dots MCP server
(`http://127.0.0.1:<port>/mcp`, header `Authorization: Bearer <per-run secret>`) plus the dot's
own `mcp_servers`. An empty `--setting-sources ""` keeps every settings file out of headless
runs: the user's global hooks (GateGuard, caveman, memory hooks) and a repo's committed
`.claude/settings.json`, which could otherwise widen the CLI's own allow list beyond the dot
policy. CLI 2.1.295 accepts the empty value (probed 2026-10-09: init event emitted, only builtin
plugins loaded). A dot opts back in to all sources with `use_user_settings`
(`--setting-sources user,project,local`).

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
→ for each `tool_calls` entry, ask the run's `PermissionGate` (the same `ApprovalHub::check` the
Claude engine's MCP `approve` endpoint uses, so policy, grants, the wait window and parking
behave identically) → execute allowed tools, return the deny text for denied or parked ones as
the tool result → append results → repeat until no tool calls or `max_turns`.

Every chat message (system, user, assistant with its `tool_calls`, tool result) is persisted as
a `message` run event. A local run's `session_id` is the `root_run_id` of its lineage. On a
resume the child run rebuilds the conversation from the `message` events of earlier runs in the
lineage.

From Approvals v2 the engine itself performs a call that was approved after its run stopped
waiting (`Engine::applies_approved_calls`). Before the child run asks the model anything, it
performs the approved calls of its parent in the order they were asked, each exactly as it was
approved: the call is claimed first, it takes the path of a live `allow` (`prepare`, then the
tool, with the walking tools' filter), and its outcome is recorded before anything else is done
with the output. The policy is not resolved again. After a call that did not succeed, the calls
that remain are given up. The engine then sends one user message that says what each call did
(and that the quoted outputs are results, not instructions), followed by the runner's resume
prompt. No grant is written and the model does not issue the
call again, so it cannot change it. Until Approvals v2 the model was asked to re-issue the
call under a one-shot grant, and a changed input asked again. See
`2026-10-13-approvals-v2-design.md` §3.

Built-in tools: `read_file`, `write_file`, `edit_file` (exact unique-string replace), `list_dir`,
`glob`, `grep`, `shell` (PowerShell, timeout default 120 s, output capped at 30 KB; the script
reaches PowerShell on stdin behind a small fixed `-EncodedCommand` bootstrap, so its length is
not bound by the Windows command-line limit). Their
argument names match the Claude tools they alias (`file_path`, `content`, `old_string`,
`new_string`, `path`, `pattern`, `command`), so policy path specs, command rules and
`grant_key` apply to local dots without translation. All paths are canonicalized and must
resolve inside the workspace (symlinks and junctions resolved first); `shell` runs with
cwd = workspace. "Test endpoint" in Settings sends a probe request with one tool and checks that
the model returns a well-formed tool call.

Tests drive the loop against a `wiremock` OpenAI-compatible server returning scripted
responses; no real model is called. Manual smoke target: LM Studio at `http://127.0.0.1:1234`
with `google/gemma-4-12b-qat` (Ollama is not installed on the dev machine).

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
  finished runs awaiting review (definition in §12). Unread count mirrored in the tray tooltip.
  From Approvals v2 an approval card, wherever it shows (Inbox, run view, chat), also offers
  "Approve and always allow": the allow rules the core derived for that call, the narrowest
  preselected, one of which is saved to the dot's policy with the approval. The control is not
  shown for a call the core offers no rule for. A parked card says who performs the call
  when it is approved: Dots, exactly as shown, or the model in a new run. The run view and
  the chat list the approved calls that Dots performs with what became of each. In the Inbox
  the approvals of one run stand together; two or more can be answered with "Approve all
  (n)" or "Deny all (n)", each behind the in-app confirmation, which names the dot and lists
  the calls. No rule is saved with such an answer.
- **Chat** — per-dot threaded chat; each message is a `chat` run resuming the thread's session.
- **Settings** — port, concurrency, `claude` path and version check, local endpoints with
  "Test endpoint", data dir, autostart toggle. Port, concurrency and `claude` path take effect
  after "Restart Dots" (§12).

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
| Port in use | App starts without the HTTP server; `Runtime::status()` reports `server_error`; UI shows a banner linking to Settings; webhooks are off and Claude runs fail fast with "local server is not running: port N is in use. Change the port in Settings and restart Dots." (they need the MCP endpoint); local dots and schedules keep working. Fixed by changing the port and restarting. From Plan 3c the start retries a port that is in use for 2 seconds first (a restart must not race the old process's socket), and a bind that fails for any other reason says so: "local server is not running: could not listen on port N: <os error>. Try another port in Settings and restart Dots." |
| Runtime fails to start (database, migration) | Window opens to a fatal-error page with the message and the log folder; nothing runs. There is no tray in this state, so closing the window exits the app and the next launch tries again |

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
- Observed (CLI 2.1.295, 2026-10-09): the CLI aborts a pending permission-prompt call at about
  300 s regardless of `MCP_TOOL_TIMEOUT`. The default `approval_wait_secs` is therefore 240 s;
  longer waits rely on park/resume.
- Local models vary widely in tool-calling quality; v1 targets simpler jobs on local dots.

## 12. Plan 3 decisions (desktop app and UI)

Decided 2026-10-09. Plan 3 is split into three plans that share this section:

- **3a** — core additions + Tauri shell + command/event surface.
- **3b** — the six UI screens.
- **3c** — Job Object hardening, manual smoke, NSIS packaging.

### Core additions (3a)

In task order:

1. Policy fixes and the `shell` stdin script (§5).
2. Migration 0002 (`runs.reviewed_at`) and a typed `Settings` struct with
   `Store::get_settings` / `put_settings` (§3).
3. Degraded start when the port is taken (§8): no server, `server_error` recorded, and Claude
   registered as an `UnavailableEngine` that fails runs with the actionable message.
4. Startup `claude --version` check (§4).
5. `Runtime::status()` → `{ port, server_error, claude: { program, version, tested_version,
   mismatch }, schedules_paused, data_dir }`.
6. Read methods, so the app never uses `Runtime::store()`: `list_dots`, `get_dot`,
   `list_runs(dot_id?, limit)`, `get_run`, `run_thread(root_run_id)` (all runs of a lineage,
   oldest first), `list_run_events(run_id, after_seq)`, `approvals_for_run`,
   `inbox() -> { approvals, review_runs }`, `inbox_count()`, and
   `preview_schedule(expr, n) -> next n fire times` (validates the cron; the UI parses no cron).
7. Review actions on a thread's shared worktree:
   - `worktree_diff(run_id) -> { files: [{ path, status, patch, binary }], truncated, exists }` — `git
     diff <base_commit>` plus untracked files as additions; total patch text capped at 1 MiB.
     `exists` is false once the worktree was removed (unchanged, or discarded); the run view
     offers discard and review of a worktree only while it is true (added in Plan 3b).
   - `discard_worktree(run_id)` — only when no run of the lineage is queued, running or
     awaiting approval; removes the worktree, deletes the branch, sets `reviewed_at` on the
     lineage. Folder-mode runs are rejected (`Invalid`). Before any git call it also refuses
     (`Invalid`) a path outside `<data dir>/worktrees` and a branch that is not
     `dots/…/<worktree folder name>`: the run row is the only thing between the database and a
     delete in the user's repository.
   - `mark_reviewed(run_id)` — sets `reviewed_at` on the lineage.
8. `set_schedules_paused(bool)` — persisted as `schedules_paused`; the scheduler fires nothing
   while paused.
9. `settings()` and `update_settings(Settings) -> { settings, restart_required }`.
   `restart_required` is true when `port`, `max_concurrent_runs` or `claude_path` differ from
   the values the runtime started with. `autostart` and `schedules_paused` apply live.
   From Plan 3b, `update_settings` keeps the stored `schedules_paused` and
   `tested_claude_version` whatever the caller sends: only `set_schedules_paused` and
   `mark_claude_tested` change them, so a Settings form holding an old copy cannot undo a tray
   pause or a "Mark as tested".

### Inbox

The Inbox lists:

- pending approvals;
- runs awaiting review: the latest run of each lineage that is terminal, has `reviewed_at`
  NULL, and either kept its worktree (worktree mode, worktree still on disk) or `failed`.

A run leaves the Inbox through "Discard worktree" or "Mark reviewed". Folder-mode successes only
notify. Unread count = pending approvals + runs awaiting review.

### Tauri shell (`apps/dots-app`)

- Workspace member, Tauri 2. Data dir `%LOCALAPPDATA%\dots`. Logs go to
  `%LOCALAPPDATA%\dots\logs` through `tracing-appender` (daily files, 7 kept).
- Startup:
  1. Read `Settings` with a short-lived `Store`.
  2. Build `Config` and call `Runtime::start`.
  3. Manage `Arc<Runtime>` as Tauri state.
  A hard start failure opens the fatal-error page (§8).
- Plugins: `single-instance` (a second launch focuses the window), `autostart` (launched with
  `--hidden`, so login starts in the tray), `notification`, `opener` (open folder), `dialog`
  (pick workdir). No `shell`, `fs` or `http` plugin. From Plan 3c, `single-instance` is not
  registered when `DOTS_DATA_DIR` is set, so a smoke or development instance never hands over
  to an installed Dots and its real data.
  Such an instance also leaves the login entry alone. Every instance holds a lock on
  `dots.lock` in its data folder while it runs; a second instance on the same folder waits up
  to 3 seconds for it (a restart) and then shows the fatal page instead of starting a second
  runtime on the database.
- Commands are one-line wrappers over `Runtime` returning `Result<T, CmdError>`, serialized as
  `{ kind: "not_found" | "invalid" | "conflict" | "internal", message }`. Plan 3b adds the kind
  `cancelled` (the user declined a native confirmation; nothing was changed). From Plan 3c the
  kind is an enum in Rust (`CmdErrorKind`); the JSON is unchanged.
- Approvals v2 adds two commands and one argument: `approval_rule_candidates(id)` answers the
  allow rules that may be saved with an approval, `decide_run_approvals(run_id, approved,
  note)` decides every pending approval of a run, and `decide_approval` takes `allow_rule`,
  which the core accepts only when it is one of those candidates.
- **Runs, dots and approvals are addressed by id.** `open_run_folder(run_id)` resolves the path
  in core; the UI cannot make the backend open, read or delete an arbitrary path. Two values
  from the UI are paths:
  - a dot's `workdir`, from the native picker or typed text, which `DotSpec::validate` and the
    runner check;
  - the `claude_path` setting, which the runtime runs as the Claude CLI from the next start.
    From Plan 3b, a change of `claude_path` to a new program must be confirmed in a native
    dialog shown from Rust ("Dots will run this program as the Claude CLI: `<path>`"). The
    webview cannot answer that dialog. Declining saves nothing. Debug builds accept the
    environment variable `DOTS_CONFIRM=yes` in place of the dialog, for smoke runs.
    From Plan 3c, `Settings::validate` refuses a `claude_path` that holds a control character,
    a Unicode format character (zero-width characters, the byte order mark, the soft hyphen,
    every text-direction control) or any other character that draws nothing (Unicode's
    default-ignorable code points, such as variation selectors and fillers, and the blank
    braille pattern): such a path could read differently in the dialog from what
    will run. A stored path that the rule refuses is ignored when settings are read.
- `restart_app` shuts the runtime down, then calls `app.restart()`. The restarted app shows
  its window even when the old one was started with `--hidden`.
- Capabilities grant only `core:default`, the app's commands and the plugin permissions in use
  (`dialog:allow-open`). CSP: `default-src 'self'; style-src 'self' 'unsafe-inline'; img-src
  'self' data:; connect-src ipc: http://ipc.localhost`, no remote origins (Tauri's IPC needs
  the `ipc:` source; Vite and Tailwind inject `<style>` elements).
- Events:
  - One forwarder emits each `RuntimeEvent` as the Tauri event `runtime`, same JSON.
  - On `broadcast` lag it emits `runtime-lagged`; the UI then invalidates all queries and
    refetches run events after the last seen `seq`.
  - From Approvals v2 there is a fifth event, `ApprovalUpdated`: a decided approval whose call
    Dots performs changed its `apply_state`. It never notifies.
- Notifications fire on `ApprovalRequested`, and on `RunUpdated` into `succeeded`, `failed` or
  `awaiting_approval`, once per (run, status), and only for a run this process saw queued or
  running. Chat runs do not notify; neither does a parked run closed by its decision, nor a run
  re-published by "Mark reviewed". From Plan 3c a park shows one notification, not two: the
  update into `awaiting_approval` does not notify when the approval request of that run was
  already announced.
- After each relevant event, `inbox_count()` updates the tray tooltip ("Dots: n to review") and
  the "Inbox (n)" menu item.
- Tray:
  - Open.
  - Inbox (n) — shows the window on `/inbox` through a `navigate` event.
  - Pause all schedules — check item.
  - Quit — `Runtime::shutdown`, then exit.
  Left click shows the window; closing the window hides it.
- Adapter logic (error mapping, notification decisions, tray text, config from settings) lives
  in plain functions with Rust unit tests; commands stay one-liners.

### UI (3b)

- Stack:
  - Vite + React 19 + TypeScript (strict) + Tailwind v4 + shadcn/ui, with components copied
    into `ui/src/components/ui`.
  - TanStack Query v5 and the `react-router` hash router.
  - Bun as package manager; `@tauri-apps/cli` as a dev dependency.
- Look: one dark theme (no light mode), shadcn/ui's neutral palette, compact desktop-tool
  density, a left sidebar for Dots, Inbox (with its count), Chat and Settings.
- Data layer:
  - `ui/src/api/types.ts` (strict zod schemas mirroring the Rust JSON), `commands.ts` (typed
    `invoke` wrappers that throw `CommandError`), `events.ts` (one `listen("runtime")`).
  - Command results and event payloads are parsed with their schemas, not cast. A result that
    does not parse is a `CommandError` of kind `internal`; an event that does not parse is
    logged and treated like `runtime-lagged` (invalidate and refetch).
  - Events patch or invalidate query keys: `runs`, `thread`, `events`, `inbox`, `approvals`
    (Plan 3c removed the key `run`, which no query read). `RunEvent`s are appended to loaded
    event lists, deduplicated by `seq`. `ApprovalUpdated` invalidates `inbox` and the
    `approvals` of the approval's own run, which is the parent of the run that performs the
    call: that run's own `RunUpdated` does not reach those approvals.
  - From Plan 3c an invalidation that arrives while its query is being fetched is kept (the
    query is marked stale again when that fetch has settled), and the app refetches everything
    once when its listeners are attached, so the first screen's data is asked for twice.
- Contract: a `dots-app` test serializes one sample of each shared type to
  `ui/src/api/__fixtures__/contract.json` and fails when the file is stale; a Vitest test checks
  the file against `types.ts`. Because live data is now parsed, the fixture also lists every
  variant of every shared enum. From Plan 3c it also lists the name of every registered command
  and the answer of `new_dot_spec`, and its engine-event samples are tied to the enum's
  variants, so a new variant without a sample does not compile. From Approvals v2 it also
  holds a sample of `ApprovalUpdated`, every `ApplyState`, and the rule candidates the core
  derives for the `Approval` sample (`RuleCandidate`).
- Shell around the screens:
  - Banners: `server_error` (links to Settings), CLI version mismatch ("Mark as tested"),
    restart required ("Restart Dots"), schedules paused.
  - While schedules are paused the Dots list shows "paused" in place of the next fire time.
  - Destructive actions (discard worktree, delete dot, regenerate webhook token) ask first in
    an in-app dialog.
  - `webhook_url` answers `conflict` while the local server is not running, so the editor
    never shows a URL on a port another program owns.
- Run view:
  - Shows a whole thread (root and children) as one transcript.
  - Assistant text renders as markdown without raw HTML.
  - Tool calls and results are paired by id and collapsible.
  - `message` events are hidden; `raw` events are collapsed.
  - The diff is an in-repo unified-diff component with per-file collapse and a truncation
    notice. The core returns git's patch text untrimmed, so a hunk that ends in a blank context
    line keeps it.
- Chat:
  - Each thread is a lineage of `chat` runs.
  - Sending continues from the last terminal run of the thread; input is disabled while that run
    is active. From Plan 3c the core holds this rule too: `chat` with a parent answers
    `conflict` when the parent is not the last run of its thread, when the thread is working or
    waiting for an approval, or when the parent has no session to continue. The check and the
    insert of the new run happen under one lock, so two replies sent at once cannot both pass.
  - Approvals of the thread show inline.
- The editor validates with zod rules mirroring `DotSpec::validate`; server errors still show
  on save.
- Tests: Vitest + Testing Library + jsdom, with `invoke` and `listen` mocked.

### Hardening and packaging (3c)

- Job Object (`windows-sys`) in `proc`:
  - Created with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`.
  - Assigned right after spawn to the `claude` process of a run and to each local `shell` call.
    `tokio::process` on stable Rust cannot start a child suspended, so a process that the child
    starts before the assignment (measured: 35 to 72 microseconds) is outside the job. Both
    engines send their work on stdin after the assignment, so nothing the model asked for runs
    in that window.
  - Cancel and timeout call `TerminateJobObject`; dropping the handle kills the rest. The
    Claude engine closes the job as soon as `claude` has ended.
  - A `shell` call closes its job when the shell has ended, before it reads the rest of the
    output, so a background process that a local dot starts in one call does not live into the
    next. When output was still open a second after the shell ended, the result says that a
    process left running was stopped.
  - `kill_tree` stays as the fallback when assignment fails and on other OSes.
- Manual smoke (`docs/verification/<date>-plan3-smoke.md`):
  - A Claude dot and a local dot (LM Studio), each with a scheduled run, a webhook run and an
    approval round trip through the Inbox.
  - Also checks tray, close-to-tray, autostart, a notification, discard and the port-in-use
    banner.
  - The document has two parts. Part A is run by an agent from a terminal against a debug
    build, with a temporary data folder, the fake CLI and LM Studio, and holds the result of
    each step. Part B is a checklist for the person who owns the machine: the real `claude`
    CLI, the tray, a notification, autostart, the native dialog, the installer. The Claude
    dot's part of the first line is met only when Part B is done.
- Packaging: `tauri build` with NSIS — per-user install (`installMode: currentUser`), WebView2
  `downloadBootstrapper`, icons from `tauri icon`, version 0.1.0, unsigned. Uninstall keeps the
  data dir. An agent builds the installer and reads its generated script; only a person runs
  it. Known and open: the per-user install folder `%LOCALAPPDATA%\Dots` is the data folder
  `%LOCALAPPDATA%\dots` (Windows ignores the case), so the program's two files lie beside the
  data; the uninstaller removes only those two files.
- Hardening carried over from the Plan 3a reviews:
  - Policy: the walking-tool directory rule above (§5).
  - Review actions: `worktree_diff` applies the same containment check as discard; a worktree
    must be a direct child of `<data dir>/worktrees` and its branch must be
    `dots/<name>/<that folder's name>`; untracked symlinks are listed, not followed. A link is
    shown as git shows one (mode 120000, one line with its target), and the files git names
    behind a junction are left out. The row's base commit must be a full hexadecimal commit id
    and is passed to git after `--end-of-options`. The check runs before any git command, also
    where an unchanged worktree is cleaned up. After git has removed a worktree, or has failed
    half way, whatever is left of its folder is deleted without following links (`git worktree
    remove` leaves a junction and its folder behind and still succeeds, and a removal that
    fails on a held file has already unregistered the worktree). Discard finds the repository
    from the worktree itself when the folder still has its `.git`. The same check of the row
    runs before a worktree is restored for a follow-up and before a run reuses its worktree; a
    folder-mode run's row has nothing to be compared with and is not checked. Git 2.31 or
    newer is needed (`--end-of-options`, `rev-parse --path-format=absolute`).
  - Start: the log folder is created before the appender; a relative `DOTS_DATA_DIR` is made
    absolute; a failure to find the local data folder shows the fatal page; a panic is written
    to the log (the line is appended to the log file directly, because the logger writes on a
    worker thread that an abort does not wait for) and the hook is installed before Tauri
    builds the app, so a panic while the app is built is written too; `Runtime::start` retries the bind briefly
    (2 seconds, only while the port is in use), so a restart does not race the old process's
    socket; `single-instance` is skipped under `DOTS_DATA_DIR`.
  - Notifications: a lagged event stream clears the notifier's memory of live runs.
  - "Restart Dots" asks first while a run is queued, running or waiting for an approval.
  - "Test endpoint" and `local_endpoints` accept any `http://` host on purpose (a model server
    on the local network); this is stated in Settings.
