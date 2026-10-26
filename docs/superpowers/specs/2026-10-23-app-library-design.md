# App library: design

Date: 2026-10-23. Status: agreed with the person in a question round, part by part; written for their review.
Extends the main spec `docs/superpowers/specs/2026-10-08-dots-design.md`, the MCP/skills spec `2026-10-15-mcp-skills-plugins-design.md`, the context library spec `2026-10-19-context-library-design.md`, the check-ins spec `2026-10-20-checkins-followups-design.md` and the chat-first home `2026-10-22-chat-home-design.md`. Reference: Cloudflare OS at `C:\Users\aksha\cloudflare-os` ("gadgets", section 3).

Marking: **(person)** is what the person said or confirmed; **(my call)** is the writer's, for the person to check while reading. Unmarked lines are consequences of the two.

## 1. Intent

The person's words: "add a app library with apps that can be called by agent and used by humans". "Dots would need these apps for deterministic work and it give the user an intuitive way to interact with the data."

So an app is a small program inside Dots with its own data, one screen a person uses, and tools a dot calls. Both work on the same data. An app does the exact, repeatable part of a job (same input and same data give the same result), and the data lives in the app instead of being retold in chat.

- First app: **Projects**, an Asana-like project manager (person). A dot reads a task's spec from it before working, writes progress back as it goes, and helps the person manage their tasks and to-dos (person).
- **A dot can add new apps** to the library (person).
- Apps are **sandboxed code**, not descriptions (person, after the trade-off was laid out: code buys any screen and any logic, at the cost of a sandbox and harder review).

Success:

- The person assigns a Projects task to a dot; the dot wakes, reads the spec, checks off subtasks and posts progress in the task's Activity while it works, and leaves the task **In review** with its summary. The person marks it Done.
- The person asks a dot in chat for a new app ("an app to track my expenses"); a review card arrives with a working preview, the code and passing tests; one click adds it, and the person and the dots that use it can work in it.
- Nothing an app does reaches past its own data unless the person switched it on.

## 2. Goals, non-goals, anti-goals

**Goals**

- An app format, a sandboxed runtime for its logic and its screen, and a store for its data.
- App tools offered to dots on both engines, checked by each dot's policy.
- A dot proposes a new app or a change to one; the person reviews a live preview, the code and the test results, then adds it, asks for changes or discards it.
- An **Apps** place in Dots: the installed apps, each opening its own screen.
- **Projects** shipped with Dots, built in the same format as a dot-made app.
- A per-app switch **"Can start dots"**, off until the person turns it on, so an app can wake a dot.

**Non-goals (v1)**

- Projects views other than the list: no board, no calendar, no timeline (person: "list view is enough").
- Apps reaching files, the network, MCP servers or other apps. The only grant in v1 is "Can start dots" (my call; the person agreed that an app reaches nothing outside its data unless granted, and v1 offers just the one grant the first app needs).
- Editing an app's code by hand inside Dots, and making an app without a dot (my call: the person's flow is "ask a dot"; the files on disk can still be edited outside Dots, see section 4).
- Sharing apps, export and import, templates made from apps (Cloudflare's "blueprints").
- An agent subscribing to arbitrary app changes (Cloudflare's `self` stub). v1 has one waking path: the app starts a dot (section 7).
- Several people; Projects' "assignee" is the person or one of their dots.

**Anti-goals**

- **A wrong outcome** (person: "if it give a wrong outcome"). An app returns a wrong result, a call half-applies, or a dot's mistake lands silently in the person's data. Section 9 is the guard.
- An app or a dot-made change becomes active without the person's click.
- An app's screen or logic reads files, opens a connection or reaches Dots' own data.
- An app starts dots without the person having switched that on, or starts them without limit.

## 3. What Cloudflare OS does, and what Dots takes

Facts from `C:\Users\aksha\cloudflare-os` (paths below `packages/`):

- A gadget is code files: `server.js` exporting a Durable Object class whose methods are its API, and `client.js` that builds the screen. There is no manifest; the agent reads the server code to learn the methods (`workshop-backend/src/agent.ts:468-489, 745`).
- Each gadget has private storage (a Durable Object facet with its own SQLite) (`overseer.ts:1607, 2469-2475`).
- The screen runs in an iframe with `sandbox="allow-scripts allow-popups allow-popups-to-escape-sandbox"` and the CSP `default-src 'none'; script-src data: 'unsafe-inline'; style-src data: 'unsafe-inline'; img-src data:; connect-src 'none'; …`. It talks to its server only through a `MessageChannel` port handed over by `postMessage` (`workshop-frontend` `GadgetUI.tsx:26-32, 109, 502`). The server runs with no outbound network (`globalOutbound: null`, `overseer.ts:2416`).
- An agent's edits are "proposed changes"; the gadget can be run against them for testing, and the person accepts (`mergeChanges`) or reverts them (`overseer.ts:2295, 2370-2386, 8490, 8598`).
- A gadget can start an agent (`spawn`, `spawnCallable` of the Agent Spawner binding, `agent-spawner-binding.d.ts:1-33`), and hooks that wake agents "start out disabled, until the user enables them" (`overseer.ts:3097-3110`).

Dots takes: code files with methods as the API, private storage per app, the iframe sandbox and its CSP, proposed changes tested before acceptance, an app starting a dot, and "off until enabled" for that. Dots changes: an app has a **manifest**, because Claude-engine dots see tools through MCP, which needs each tool described with an input schema; and the logic runs in an embedded script engine in `dots-core`, not in a Worker.

## 4. The app format

An app is a folder (my call on every name below):

```
app.json            manifest
logic.js            the tools (an ES module)
ui/index.html       the screen; may load other files from ui/ by relative path
migrations/0001_*.sql, 0002_*.sql, …   the app's database schema, applied in order
sample.sql          optional: rows for the review preview
tests/*.test.js     tests, run in the sandbox
README.md           optional: notes for the next dot that changes the app
```

`app.json`:

```json
{
  "name": "projects",
  "title": "Projects",
  "description": "Projects, tasks and to-dos, shared by you and your dots",
  "tools": [
    {
      "name": "get_task",
      "description": "A task with its spec, subtasks and recent activity",
      "input": { "type": "object", "properties": { "id": { "type": "string" } }, "required": ["id"] },
      "writes": false
    }
  ],
  "permissions": ["start_dots"],
  "on_run_end": "run_ended"
}
```

- `name` matches `^[a-z][a-z0-9_]{0,23}$`; tool names `^[a-z][a-z0-9_]{0,39}$`. Each tool is a function of the same name exported by `logic.js`. `input` is a JSON Schema (the subset Dots already checks for MCP tool inputs, or a small one the builder picks). `writes` says whether the tool can change data; it is used in policy rules and logged.
- `permissions` may list only `start_dots` in v1; anything else is refused when the app is proposed.
- `on_run_end` names an exported function Dots calls when a run the app started ends (section 7).
- The screen calls the same exported functions as dots, and only those listed in `tools`. There is one API for both users (as in Cloudflare OS). Functions not listed are private to `logic.js`.
- Bounds (my call; the builder may tune them): 2 MB of files per app, 200 tools, 32 KB `app.json`.

Where it lives (my call): `%LOCALAPPDATA%\dots\apps\<app-id>\code\<version>\` for each accepted version (the one before is kept, so a bad update can be rolled back from the Apps screen; roll back is offered only when that update ran no migrations, because old code on a newer schema is a wrong outcome; my call), and `…\<app-id>\data.db` for its data. Proposed versions go to `…\<app-id>\proposed\<proposal-id>\` until decided. Editing the active code folder by hand outside Dots is not prevented. Dots re-reads it on start and refuses to load an app whose `app.json` no longer parses, showing the error on its row.

## 5. The runtime

### Logic

- `logic.js` runs in a script engine embedded in `dots-core`. My call: QuickJS through the `rquickjs` crate. It has no file, network or process access unless the host adds it, and it supports memory limits and an interrupt for time limits. The builder may pick another engine with the same properties.
- Each tool call: Dots checks the input against the tool's schema, opens a transaction on the app's `data.db`, calls `fn(input, ctx)`, and commits if the function returns, or rolls back if it throws, times out or returns a value that is not JSON. **A call applies fully or not at all.**
- `ctx` is all the outside world the logic sees:
  - `ctx.db.query(sql, params) -> rows` and `ctx.db.exec(sql, params) -> { changes }` on the app's own database, inside the call's transaction. No `ATTACH`, no `load_extension`, no `PRAGMA` that changes the file (the builder enforces this with SQLite's authorizer).
  - `ctx.now`: the call's time (RFC 3339, local offset) and `ctx.id()`: a new ULID. Both come from Dots and are recorded with the call. `Date.now`, `new Date()` without arguments and `Math.random` are removed from the engine, so given the same data, input, `now` and ids, a call gives the same result.
  - `ctx.actor`: who is calling, set by Dots and not by the caller: `{ "kind": "person" }` or `{ "kind": "dot", "id": "…", "name": "builder", "run_id": "…" }`.
  - `ctx.dots.list()`: the id and name of each dot that uses this app, for every app without a grant (my call: Projects needs it to assign tasks while starting dots is off; it is names, not the person's data). The review card says "Sees the names of the dots that use it".
  - `ctx.dots.start(dot_id, message, tag)` (only with `start_dots` switched on) returns a run id (section 7). The run is queued only after the call commits, so a rolled-back call starts nothing.
- Limits per call (my call): 2 s of time, 64 MB of memory, a 30 KB result for dots (the screen gets up to 1 MB), and `data.db` at most 512 MB. A breach is a tool error and a rollback.
- Migrations: when an app version is added, Dots applies the migrations newer than the database's recorded one, in one transaction, before switching to the new code. If one fails, the version is not added and the error shows on the review card.

### Screen

- The Apps screen shows the app's `ui/index.html` in an iframe with `sandbox="allow-scripts"` (no same-origin, no forms, no popups) and a CSP like Cloudflare's: `default-src 'none'; script-src 'unsafe-inline' <app files>; style-src 'unsafe-inline' <app files>; img-src data: <app files>; connect-src 'none'; frame-src 'none'; form-action 'none'; base-uri 'none'`. How the app's other `ui/` files are served (inlined into `srcdoc`, `data:` URLs, or a custom protocol limited to that app's `ui/` folder) is the builder's call, as long as the frame can load nothing else.
- The frame gets a `MessagePort` and a tiny client, `dots.call(tool, input) -> Promise<result>`, injected by Dots. The host checks every message: the tool must be listed, and the actor is always the person. The host also sends the current theme (Deep Teal tokens, light or dark) so apps can match Dots, and `dots.onChange(cb)` fires when a dot changes the app's data, so the screen refreshes.
- The app's frame never sees Tauri's IPC: the Dots CSP `connect-src ipc:` applies to the host, not the sandboxed frame. The builder confirms this with a test that a frame cannot reach `ipc:`.

### Call log

Every call that `writes`, from a person or a dot, is recorded in Dots' own database: app, tool, actor, input, `now`, result or error, time taken (my call). This is the audit trail. Projects keeps its own readable Activity on top of it (section 8).

## 6. Dots using apps

- New Library kind `app`. Dots manages it; it is added through review (section 6.2) or bundled (Projects), never through an "Add" form. `Uses` gains `apps: Vec<String>`. An app has an `every_dot` switch like a context collection. **Projects is on for every dot** (my call; the person read and let it stand). The dot editor's "Tools and skills" gains an **Apps** checklist.
- A run's apps are the ticked ones plus every `every_dot` one, opened in `RunTools::open` like collections.
- Each app tool is offered on both engines, the way `context` is (context library spec section 4): built in on the local engine; on the Claude engine as tools of Dots' own `dots` MCP server, named `<app>__<tool>` (so the model sees `mcp__dots__projects__get_task`), listed in `--allowedTools`. The prompt gets a short "Apps" section naming each app, its description and its tools.
- **Policy.** Policy name `App`. A rule `App(<app>)` or `App(<app>.<tool>)` matches those calls; `App(<app>.write)` matches that app's tools with `writes: true`. Every preset allows `App` (my call: apps reach only their own data, every write is logged and Projects can undo; the person said reads should not ask and did not ask for writes to). A dot's own `App` ask or deny rule wins and uses the normal approval card. Migration adds the allow rule to stored policies, as 0007 did for `Context`.

### 6.2 A dot proposes an app

- A dot writes the app's files in its workspace, using the file tools it already has, and calls **`propose_app`** with `{ "path": "<folder in its workspace>", "note": "<what it built or changed>" }` (my call: the dot builds with tools it knows, and one call hands the folder over). Policy name `ProposeApp`, allowed by every preset, since the review below is the gate and cannot be switched off.
- Dots copies the folder into `proposed\<id>\` and checks it: `app.json` parses and follows section 4, every listed tool is exported by `logic.js`, permissions are known, the migrations run on an empty database, `sample.sql` loads. Then it runs the tests (section 9). A check that fails is returned to the dot as a tool error naming the problem, so the dot can fix it and call again. Nothing reaches the person yet.
- A passing proposal posts a **review card** in the dot's conversation, and counts in what needs the person, as an approval does. The tool returns: "Sent for review. Finish your turn; the person's answer arrives as your next message." This is park and resume, as `ask` works (check-ins spec).
- The card (person, from the mockup they approved):
  - **Preview:** the app's screen running for real in its sandbox, on a throwaway database built from the migrations and `sample.sql`. Calls work, and changes vanish when the card closes. Dots cannot be started from a preview.
  - **Code:** every file. For a change to an existing app, the diff against the active version (the existing `DiffView`).
  - **Summary:** description, the tools dots get (with `writes` marked), what it reaches outside its data ("Nothing", or "Can start dots"), test results, file count and size, and for an update, the migrations it will run on real data.
  - **Add to library** (or **Update**), **Ask for changes** (a text box; the text continues the dot's thread as the next message) and **Discard**. Adding with `start_dots` in the manifest adds the app with the switch **off**; the card says so and links to it.
- A change to an existing app (a new version) goes through the same card. Only the app's last accepted version is changed; there are no branches.

## 7. Apps waking dots

- With "Can start dots" on, `ctx.dots.start(dot_id, message, tag)` queues a run of that dot. It must be a dot that uses the app. New trigger kind `app` (`TriggerKind::App`). The run is a new topic in the dot's conversation, shown like a scheduled run: "Started by Projects". The message is the app's text, and `tag` is a short string the app uses to recognize the run later (Projects uses the task id).
- The unit of work is the **topic** the start opened, not its first run (my call). A topic goes on through resume runs after an approval, the person's answer to an `ask` or a review card, and later chat. Dots calls the app's `on_run_end` function when a run of that topic ends **and nothing in the topic is waiting** (no parked approval, open question or pending review card), with `{ run_id, root_run_id, tag, dot_id, status, summary, error }` as a normal call (actor: Dots). If the person continues the topic later, it is called again when that run ends.
- Limits (my call): at most one app-started topic per `(app, dot, tag)` that is still working or waiting (a second start returns its root run id), and at most 20 starts per app per hour. A start over the limit is a tool error. When schedules are paused, app starts are refused too.
- The switch lives on the app's row in the Apps screen and its settings. Turning it off removes `ctx.dots.start`; topics already started go on and still reach `on_run_end`.

## 8. Projects

Built by the builder in the format above, bundled with Dots. **It has its own tests and passes the same checks as a dot-made app.** Its first version asks for `start_dots`, and Dots shows a one-time prompt to switch it on (my call).

**Bundled and changed copies** (my call; the person said "go" to the three options): Projects starts as a *bundled* app, and Dots updates it with each Dots release through its migrations. A dot may propose changes to it like any app. Once the person accepts one, it becomes *the person's*: Dots stops updating it, and its row says "Changed by you; Dots no longer updates it". Bundled updates never clash with a dot's migrations. Removing Projects lasts; the Apps screen offers "Install Projects" to bring back the bundled version, which starts with fresh data or keeps the old data if its schema matches. Dots, not the dot, sets an app's `version`: a counter raised on each accepted version. The registry records each app's origin (`bundled` with the Dots release it came from, or `proposed`) and whether a bundled app has been changed.

### Screen (person: mockup A, list view only)

Left column: **My tasks** (tasks assigned to the person, to-dos included), **Assigned to dots**, then the projects, and **+ New project**. The main area shows the project name and the list grouped by section, with the columns Task, Assignee, Due and Status. Rows can be added inline, sections added and renamed, tasks dragged between sections. Clicking a task opens a side pane: title, assignee (the person or a dot that uses Projects), due date, status, **Spec** (the description, Markdown), subtasks (a checklist), and **Activity** (changes and comments, newest last, with a comment box).

- **To-dos** are tasks with no project. They show in My tasks (my call, read and let stand by the person).
- **Statuses**: To do, In progress, In review, Done. **A dot cannot set Done**; the logic refuses it for a dot actor ("Only the person marks a task Done; set In review"). When a dot's run on a task ends, the task moves to In review unless it is already Done (my call, read and let stand).

### Data (my call)

```
projects(id, name, position, archived_at)
sections(id, project_id, name, position)
tasks(id, project_id NULL for a to-do, section_id NULL, title, spec, assignee_kind 'person'|'dot'|NULL,
      assignee_dot_id NULL, due_on DATE NULL, status, position, created_at, updated_at, completed_at NULL)
subtasks(id, task_id, title, done, position)
activity(id, task_id, at, actor_kind, actor_dot_id NULL, actor_name, kind 'change'|'comment'|'run',
         field NULL, before_json NULL, after_json NULL, text NULL, run_id NULL, undone_by NULL)
```

Every change to a task or subtask writes an `activity` row with its before and after values, in the same call.

### Tools (my call)

`list_projects`, `list_tasks({ project_id?, assignee?: "me"|"person"|<dot>, status?, due_before? })`, `get_task(id)` (spec, subtasks, last 20 activity rows), `create_task`, `update_task(id, { title?, spec?, status?, assignee?, due_on?, section_id? })`, `add_subtask`, `set_subtask(id, done)`, `comment(task_id, text)`, `create_project`, `create_section`. `"me"` resolves to the calling dot. Screen-only functions (reordering, `undo`) are listed too, and a dot may call them, under the same rules.

### Assigning to a dot

When a task's assignee becomes a dot and "Can start dots" is on, the logic starts that dot with a message naming the task and telling it to read the spec with `get_task`, report with `comment` and `set_subtask`, and set In review when done (my call on the wording; the builder writes it). The task moves to In progress, and Activity gets "● builder started" with the run linked. `on_run_end` posts the summary (or the error) to Activity and applies the In review rule. With the switch off, the task just waits, and the dot finds it with `list_tasks({ assignee: "me" })` the next time it runs.

### Undo

Each `change` row a dot made shows **Undo** in Activity. `undo(activity_id)` puts back the `before` value only if the field still holds that row's `after` value. Otherwise it refuses, naming the later change. The undo is itself an activity row. Comments and run rows have no undo.

## 9. Against a wrong outcome

The person's anti-goal, and the guards, each read and let stand by the person:

1. **Deterministic logic:** no clock, random numbers, network or files in the engine. `now` and ids come in through `ctx` and are recorded.
2. **Checked inputs:** every call is checked against the tool's schema before it runs. A bad call is an error to the caller, and nothing changes.
3. **All or nothing:** one transaction per call. Throw, timeout, bad result or a limit breach means a rollback.
4. **Tests before acceptance:** `tests/*.test.js` run in the same sandbox, each against a fresh database with the migrations applied. The test API (my call) is `test(name, fn)` and `assert.equal / deepEqual / throws`, with `call(tool, input, { actor?, now? })` to call the app's tools through the real call path. A proposal needs at least one test, and every test must pass. Otherwise `propose_app` returns the failures to the dot, and no card is posted. The card shows the test names and results.
5. **Visible and reversible changes:** every write is in the call log. Projects shows each change in Activity with its actor and lets the person undo a dot's change.
6. **The person closes the work:** a dot's task ends In review, and only the person marks it Done.

## 10. Screens and routes

- **Icon rail:** a new **Apps** entry (between Inbox and Library; my call). `/apps` lists installed apps (title, description, version, "Every dot" badge, "Can start dots" switch, the dots that use it, "Roll back" when a previous version exists, "Remove" with the usual "dots use it" confirmation, which deletes the code and, after a second confirmation, the data). `/apps/:name` shows the app's screen filling the content area. Selecting Projects opens `/apps/projects`.
- **Dot editor:** an Apps checklist in "Tools and skills".
- **Conversation:** the review card (section 6.2), "Started by <app>" topics, and app tool calls shown as other tool calls (`projects › update_task`).
- **Needs you:** a waiting review card counts like an approval.

## 11. Errors and edge cases

| Case | What happens |
|---|---|
| A dot calls a tool with bad input | Tool error naming the field; no change |
| A tool throws or runs over 2 s | Tool error; rollback; written to the call log |
| An app's code folder is broken on start | The app is left out of runs (run event "app <name>: <error>") and shows the error on its row; its data is kept |
| A migration fails on update | The version is not added; the card shows the error; the active version and data are unchanged |
| `propose_app` with a path outside the dot's workspace | Refused, as file paths outside a workspace are |
| The person discards a proposal | The proposed folder is deleted; the dot's thread gets "The person discarded the app." |
| An app starts a dot that no longer uses it | Tool error; nothing queued |
| The task's dot is deleted | The assignee shows "removed dot"; `on_run_end` for its runs still records them |
| Two dots update one task | Each call is its own transaction; Activity shows both in order; the last write wins on a field |

## 12. Testing

- Core: manifest checks; the engine has no `Date.now`/`Math.random`/I/O; call path (schema check, transaction commit and rollback, limits, result bounds, actor); SQLite authorizer refusals; migrations in order and failing; call log; Library kind, `Uses.apps`, `every_dot`; policy `App(...)` forms and the migration adding the allow rule; `propose_app` checks, test runner and the park-and-resume answer; `start_dots` off and on, limits, `TriggerKind::App`, `on_run_end`; the `dots` MCP server listing and checking app tools; the local engine offering them; the Claude engine through `fake-claude` calling `mcp__dots__projects__get_task`.
- Projects: its own `tests/*.test.js` (statuses, the Done rule, undo rules, activity rows, assignment start, `on_run_end`), plus core tests that load the bundled app.
- UI: the frame cannot reach the network or `ipc:`; the bridge refuses unlisted tools; the Apps screen, switch, roll back and remove; the review card's tabs and buttons; the editor checklist.
- Smoke (agent steps, LM Studio): a local dot is assigned a task, reads it, checks a subtask, comments, and ends In review; a local dot proposes a small app that passes its tests, and the card adds it. Person step: the same assignment on a real Claude-engine dot.

## 13. Open risks

- A dot can write a test suite that passes and an app that is still wrong. Tests catch broken code, not a wrong idea of what the person wanted; the preview and the In review step are the person's checks.
- QuickJS is slower than V8. App logic is small, and the 2 s bound is for runaway code, not for normal work.
- Prompt injection: a dot that read a hostile page could propose a hostile app. The sandbox means that app can only touch its own data, and starting dots stays off until the person switches it on; the card's "reaches outside its data" line is the thing to read.
- Raw SQL in apps gives freedom but leaves data shape to the app. Apps' migrations are the only schema record.

## 14. Notes from the builder check

- `App` ask and deny rules must beat the presets' broad `App` allow: `App` and `ProposeApp` join the treatment the check-in tools and `Memory` get (deny > ask > allow, `policy.rs`; memory spec `2026-10-23-dot-memory-design.md`).
- Dots' own CSP (`default-src 'self'`, `apps/dots-app/tauri.conf.json`) is inherited by a `srcdoc` frame; how `ui/` files reach the frame has to account for it.

## 15. Changes made while building (plan A)

The core was built first (plan A); these are the places where it says more than the sections above, or differs from them.

- **Name lengths.** An app name is at most 20 characters and a tool name at most 31 (section 4 said 24 and 40): a Claude tool name may have 64 characters, and app tools are `mcp__dots__<app>__<tool>`. Neither may hold `__` or end in `_`, since the tool name is split at the first `__`.
- **The tool name `write` is taken.** `App(<app>.write)` names an app's tools that write, so no tool may be called `write`.
- **A denied tool is not offered.** A tool the dot's policy denies (`App(<app>)`, `App(<app>.<tool>)`, `App(<app>.write)` or `App`) is left out of the run's tools and of the prompt's "Apps" section; an app with no tool left is left out too. Ask rules still offer the tool, and the call waits for an approval named `App` whose input is `{ app, tool, writes, input }`.
- **Where the tools show.** On the local engine an app call is shown in the transcript under the policy name `App`, with that input, as the other built-in tools are; on the Claude engine it shows as `mcp__dots__<app>__<tool>`.
- **Proposed folders** are `<data>/apps/proposed/<proposal id>/` for a new app and a change alike (section 4 said `<app id>/proposed/`; a new app has no id yet). A decided proposal's folder is deleted.
- **A change keeps the migrations that ran.** A proposed version must hold the active version's migrations unchanged, with new ones after them; otherwise `propose_app` refuses it, naming the migration. The card's migrations are the new ones; for a new app, all of them.
- **One proposal per run** waits for the person at a time; a second `propose_app` in the run is told to finish its turn, as a second `ask` is.
- **Accepting a stale change is refused.** A change checked against version N cannot be added once the app is at another version; the card stays waiting, and the person asks the dot to propose again.
- **The person's decision continues the thread** as an answered question does: "The person added the app <name> to the Library.", "The person accepted your change to the app <name>; it is now version <n>.", "The person asked for changes to the app <name>: <text>" and "The person discarded the app.". A thread that is still working is refused, with the chat's words.
- **The test API.** Test files are scripts, not modules. `call(tool, input, options)` takes `actor` (`{ kind: "person" }` by default, `{ kind: "dot", id, name, run_id? }`, or `{ kind: "dots" }` to call `on_run_end`), `now` (default `2026-01-01T09:00:00+00:00`), `dots` (what `ctx.dots.list()` gives) and `can_start` (only for an app that asks for `start_dots`); `started()` lists the starts the test's committed calls asked for. Besides `assert.equal`, `deepEqual` and `throws` (with a substring or a regular expression), there is `assert.ok`. A file may hold 500 tests and run 10 s.
- **App-started runs** have the trigger `app` and the payload `{ message, app, app_title, tag }`; their prompt opens with "## Message from the app <title>". The topic is the run the start queued; it is busy while a run of it is queued, running or waiting for an approval, a question of it is open, or a review card of it waits. `on_run_end` is called once per run that ends with the topic not busy. If the person's answer to a question starts a thread of its own (a run without a session), the app does not hear of the runs of that thread.
- **More is removed for determinism.** The engine also has no `performance`, `WeakRef` or `FinalizationRegistry`, and SQL has no `random()` or `randomblob()`, not even in a column's DEFAULT (`ctx.id()` gives new ids). The engine's stack is 256 KB: very deep recursion throws a `RangeError` and is a tool error.
- **The database rules.** `query` and `exec` take one statement each, and `exec`'s `changes` counts every row the statement changed. Temp tables, indexes, views and triggers are refused; a trigger in `temp` on a main table is allowed, since it can write only to the app's own tables and goes with the connection. The call's SQL runs within the call's 2 s, and a call that returns after its time is a timeout. A tool that does not write runs on a read-only transaction. A query's rows are at most 64 MB. Migrations and `sample.sql` get 60 s.
- **Starting dots.** Only a tool that writes, and `on_run_end`, gets `ctx.dots.start`. A start that breaks a rule throws an error the code can catch, and nothing is queued for it. A dot the person switched off is not in `ctx.dots.list()` and cannot be started. A topic is busy also while a follow-up one of its runs scheduled has not fired. `on_run_end` hears only the topic's newest run, once; the ends Dots missed (while it was closed, when its events lagged, or when a follow-up that kept the topic waiting is cancelled, skipped or fires into a thread of its own) are told then. The app's message is quoted in the started dot's prompt, so it cannot open a section of its own.
- **Version numbers are never reused.** A roll back keeps the highest number given, so the next version after it gets a new one, and a change checked against a rolled-back version cannot be added to a later one.
- **Proposals are bounded.** A dot has at most 5 proposals waiting. A change is added only to the app it was checked against: the same app (not another installed under its name) at the same version, checked under the app's lock. "Ask for changes" text is at most 2000 characters. Deleting a dot deletes its proposals' files. The migration check runs on an empty database, as section 6.2 says; at "Update" the migrations run on the real data in one transaction, and a failure leaves the card waiting.
- **Custom policies** stored before the upgrade gain the allow rules `App` and `ProposeApp`, as new Custom policies have them.
- **The Claude engine offers at most 250 app tools** to a run, since each is one entry on its command line; an app past that is left out, and the run's notice names it. A tool offered to a run as not writing is refused if the app now says it writes (the app changed during the run).
- **Tests are judged by Dots.** A test passes when its function returns or its promise resolves, and fails when it throws or rejects. `test`, `assert`, `call` and `started` cannot be replaced, a test registered while the tests run fails the file, and a timeout or memory breach stops the file. A test's name is at most 200 characters and its error at most 1000; the card shows at most 20 failures.
- **The call log** keeps at most 30 KB of a call's input; longer input is logged as its size and its start.
- **The migration is 0015.** The env vault's orgs (0013) and the watchers (0014) reached `master` first, so the app library's tables, its `upgrade_0015` list and `finish_0015` come after them. None of the three touches a table another does.
- **Left to plan B:** the Apps screen and its frame, the bridge, the review card with its preview (calls on a throwaway database built from the migrations and `sample.sql`) and diff, the editor's Apps checklist, the needs-you count for waiting cards, and how `ui/` files reach the frame. Plan B also gives dots a way to read an installed app's files (a tool, under `ProposeApp`, that copies the active version, as written, into the run's workspace): a change must keep the active version's migrations byte for byte, and no dot but the one whose workspace still holds its folder can see them, so without it a dot cannot change Projects or another dot's app. The core gives the screen its commands: `apps_list`, `app_call` (as the person), `app_files`, `app_calls`, the switches, roll back, remove, `app_proposals_list`, `app_proposal_files` and `app_proposal_decide`, and the event `app_proposal_changed`. **Left to plan C:** Projects, bundled.

## 16. What is left to the builder

The script engine if not QuickJS (with the properties of section 5); the JSON Schema subset; how `ui/` files reach the frame; the exact limits; the "Apps" prompt section's wording; the Projects starting message; component layout of the Apps screen and the card beyond the approved mockups; whether the plan is split (my suggestion: A runtime, store, tools and policy; B screens and the review card; C Projects).
