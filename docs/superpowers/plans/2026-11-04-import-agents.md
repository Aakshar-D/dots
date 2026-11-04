# Import Claude Code agents as dots — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** List the Claude Code agents on this PC and turn the ones the person picks into dots.

**Architecture:** The kit reads agent files (frontmatter now with lists); a new `claude_agents` module scans the user, project and plugin folders and maps each file to a `DotSpec`; the runtime re-scans on import and creates each dot through `create_dot`. The UI opens a dialog from the Dots column.

**Tech Stack:** Rust (tokio, sqlx/SQLite), Tauri 2, React + TypeScript (zod, TanStack Query, vitest).

**Spec:** `docs/superpowers/specs/2026-11-04-import-agents-design.md`

## Global Constraints

- `Frontmatter::fields` keeps its behaviour; lists go in a new `lists` map.
- Import accepts only paths a fresh scan returned without an error, and re-reads them.
- Dot names: `A-Za-z0-9-_`, 1-64; collisions get `-2`, `-3` (case-insensitive).
- Risky tools (`Bash`, `PowerShell`, `Write`, `Edit`, `MultiEdit`, `NotebookEdit`, `WebFetch`) are never allowed by an import.
- Tests never read the real home folder: `Config::new` has `claude_home: None`.
- Commits carry no AI trailer. The contract fixture is edited by hand (it is already stale on the base).

### Task 1: Frontmatter lists

**Files:** `crates/dots-core/src/kit/frontmatter.rs`; test `crates/dots-core/tests/kit_test.rs`.

- [ ] Failing tests: block list, flow list, quoted items, comma string through `list()`, scalars unchanged, a map is still ignored.
- [ ] Implement `lists` and `list()`.
- [ ] `cargo test -p dots-core --test kit_test`; commit `feat(core): the frontmatter reader keeps simple lists`.

### Task 2: Reading an agent file and mapping it

**Files:** create `crates/dots-core/src/kit/agent.rs`; test `crates/dots-core/tests/kit_agent_test.rs`.

- [ ] Failing tests: `read_agent` (name, stem fallback, description, tools as string and list, model, body; empty body and too large are errors); `dot_name`; `agent_model`; `agent_policy` (none → Sandboxed; safe tools allowed; risky tools ask; unknown and `mcp__` ignored with warnings; preset file/command allows removed, deny rules kept; policy validates); `agent_files` recursion.
- [ ] Implement; commit `feat(core): read a Claude Code agent file and map its model and tools`.

### Task 3: Scan and import in the runtime

**Files:** create `crates/dots-core/src/claude_agents.rs`; `runtime.rs`, `lib.rs`; test `crates/dots-core/tests/runtime_claude_agents_test.rs`.

- [ ] Failing tests: user, project and plugin sources found and labelled; a bad file listed with its reason; already imported by case-insensitive name; import makes dots with the mapped spec; workdir for project vs others; collision suffix within a batch and against existing dots; a path not in the scan or with an error is refused; a file changed after the scan is re-read.
- [ ] Implement; commit `feat(core): scan Claude Code agents and import them as dots`.

### Task 4: App commands and contract

**Files:** `apps/dots-app/src/commands.rs`, `lib.rs`, `contract.rs`, `ui/src/api/__fixtures__/contract.json` (hand edit), `ui/src/api/types.ts`, `ui/src/api/commands.ts`, `commands.test.ts`.

- [ ] Commands, registration, samples, schemas; commit `feat(app): commands to scan and import Claude Code agents`.

### Task 5: UI dialog

**Files:** create `ui/src/screens/home/ImportAgentsDialog.tsx` and its test; `DotsColumn.tsx`.

- [ ] Failing tests: grouped list; already imported unchecked and marked; error rows have no checkbox; import sends checked paths and the scope; results shown.
- [ ] Implement; commit `feat(ui): import Claude Code agents from the Dots column`.
