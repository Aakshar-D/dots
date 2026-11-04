# Approval guards — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Calls touching private folders always ask and never get an always-allow rule; the dot editor lists what is always allowed; harmless plumbing tools never ask.

**Architecture:** A matcher in `crates/dots-core/src/policy/private.rs` (a child of `policy.rs`, so it reuses the path helpers) decides whether a call or a rule pattern touches a private place. `Policy::resolve_in` applies the plumbing allow and the private ask; `allow_candidates` filters; `Runtime::decide_approval` and `Store::approve_with_rule` refuse. The Claude engine adds the plumbing tools to `--allowedTools` and private ask rules through `--settings`. The UI gets one command for the card and a list in `PolicyEditor`.

**Tech Stack:** Rust (tokio, sqlx/SQLite), Tauri 2, React + TypeScript (zod, TanStack Query, vitest).

**Spec:** `docs/superpowers/specs/2026-11-03-approval-guards-design.md`

## Global Constraints

- The private list and the plumbing list are fixed constants (spec §3.1, §5).
- Deny rules always win. A one-time approval of a private call still works.
- Refusal text: `this touches a private folder, so it always asks: no rule can allow it`.
- Card text: `This touches a private folder, so it always asks.`
- Commits carry no AI trailer. Rust builds need `C:\Program Files\LLVM\bin` on PATH.

## Review Focus

1. Trusted preset, `Read ~/.ssh/id_rsa`: asks (Task 2).
2. A deny rule on a private path still denies; a deny rule on `TodoWrite` still denies (Task 2).
3. `process.env.FOO` and `.envrc` are not private (Task 1).
4. A forged `allow_rule` for a private call is refused and the approval stays pending (Task 3).

---

### Task 1: The matcher

**Files:** Create `crates/dots-core/src/policy/private.rs`; modify `crates/dots-core/src/policy.rs` (`mod private; pub use`), `crates/dots-core/src/runtime.rs` (register the vault at start). Test: `crates/dots-core/tests/policy_private_test.rs`.

**Produces:** `policy::touches_private(tool, input, workspace) -> bool`, `policy::pattern_touches_private(pattern) -> bool`, `policy::register_vault(path)`, `policy::cli_private_ask_rules() -> Vec<String>`, `policy::PRIVATE_REFUSAL`.

- [ ] Step 1: failing tests for each place, case, `\`, `~`/env vars, quotes, wildcards, `.env.local` yes, `.envrc`/`process.env.X` no, vault absolute and through `..`, relative path joined to a private workspace, a covering `dir/**` pattern.
- [ ] Step 2: run `cargo test -p dots-core --test policy_private_test` (fails), implement, run (passes).
- [ ] Step 3: commit `feat(core): know when a call touches a private folder`.

### Task 2: Policy: plumbing allow, private ask, no candidates

**Files:** `crates/dots-core/src/policy.rs` (`PLUMBING_TOOLS`, `resolve_in`, `allow_candidates`). Test: `policy_private_test.rs`, `policy_test.rs`.

- [ ] Step 1: failing tests: Trusted/allow rule/`default: allow` give ask for private calls; deny rule still denies; plumbing tools allow under every preset and deny under a deny rule; `WebSearch` unchanged; `allow_candidates` empty for private calls; no home-covering candidate.
- [ ] Step 2: implement; run `cargo test -p dots-core --test policy_test --test policy_private_test`.
- [ ] Step 3: commit `feat(core): a private folder always asks, and plumbing tools never do`.

### Task 3: Refuse saving, and tell the card

**Files:** `crates/dots-core/src/runtime.rs` (`decide_approval`, new `approval_touches_private`), `crates/dots-core/src/store/approvals.rs` (`decide` with a rule). Test: `crates/dots-core/tests/runtime_test.rs` (near the existing candidates test), `crates/dots-core/tests/approvals_test.rs`.

- [ ] Step 1: failing tests: forged rule refused (`Invalid`, still pending), then a plain approval works; store `approve_with_rule` refuses a private call or a private pattern; `approval_touches_private` true/false.
- [ ] Step 2: implement; run the two test files.
- [ ] Step 3: commit `feat(core): no always-allow rule is saved for a private folder`.

### Task 4: Claude CLI arguments

**Files:** `crates/dots-core/src/engine/claude/args.rs`. Test: `crates/dots-core/tests/claude_cli_test.rs`.

- [ ] Step 1: failing tests: `--allowedTools` holds every plumbing tool; `--settings` carries the private ask rules as JSON; the exact-argv test updated.
- [ ] Step 2: implement; run `cargo test -p dots-core --test claude_cli_test`.
- [ ] Step 3: commit `feat(core): the Claude CLI never asks for plumbing tools and always asks for private folders`.

### Task 5: App command and contract

**Files:** `apps/dots-app/src/commands.rs`, `lib.rs`, `contract.rs`; `ui/src/api/commands.ts`; regenerate `ui/src/api/__fixtures__/contract.json` if it changes.

- [ ] Step 1: add `approval_touches_private(id) -> Res<bool>` and the UI wrapper; run `cargo test -p dots-app`.
- [ ] Step 2: commit `feat(app): approval_touches_private`.

### Task 6: UI

**Files:** `ui/src/components/ApprovalCard.tsx` (+ test), `ui/src/screens/dot-editor/PolicyEditor.tsx` (+ test), new `ui/src/lib/allow-rules.ts` (+ test) for the words.

- [ ] Step 1: failing tests: the card line and no always-allow button for a private call; `describeAllowRule` words; the list shows allow rules in words, leaves Dots' own tools out, Remove calls `onChange` without the rule, empty text.
- [ ] Step 2: implement; run `bun run test` for the files and `bunx tsc -b` (or the repo's typecheck script).
- [ ] Step 3: commit `feat(ui): the card says why a private folder always asks, and the dot lists what it always allows`.
