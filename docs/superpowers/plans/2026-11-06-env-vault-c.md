# Env Vault, Plan C (Audit Records) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every resolution attempt and every fetch outcome of an account writes an audit record that never holds a value, queued in `audit_outbox` and sent to the account's audit endpoint in NDJSON batches with backoff; the format is documented for project 2; `dots-admin audit verify` checks a file of records; the account's page gets a History tab; and the findings plans A and B left open are fixed.

**Architecture:** A new core module `vault::audit` holds the record and its canonical line (`record`), the chain checker that `dots-admin` shares (`verify`), the writer that `Orgs` owns (`writer`), the sender the runtime spawns (`sender`) and the page's view (`view`). The store gets three tables in migration `0037_audit.sql`: `audit_chain` (each account's last `seq` and line hash), `audit_outbox` (lines not yet sent) and `audit_history` (what History lists). `Orgs` writes a record at each fetch outcome; `Vault::resolve_in` and `Vault::check_values` write one per name asked for in an account. The app adds `org_audit` and `org_leftover_keys`; the UI adds the History tab, the Overview line and the leftover sign-in details list.

**Tech Stack:** Rust (dots-core, dots-admin, dots-app on Tauri 2), sqlx/SQLite, reqwest, axum (the test sink), React with TanStack Query v5 and zod 4 strict schemas, vitest with Testing Library. No new dependency.

**Spec:** docs/superpowers/specs/2026-10-21-env-vault-design.md (§10 "Redaction and audit", §12 `dots-admin audit verify`, §13 testing; §14 and §16 for what plans A and B built; Task 14 adds §17). Also `docs/superpowers/specs/2026-10-28-accounts-members-design.md` §4 and §9, which renamed Organisations to Accounts and gave each account page its tabs.

## Scope

This is the third of three plans for the spec:

- **Plan A (merged, PR #18):** Personal vault, grants, base environment, redaction, migration and the token fixes.
- **Plan B (merged, PR #23):** orgs, sources, policy, offline copy, per-scope catalogs and origin pinning, the org picker and `dots-admin`. It parses the policy's `audit` (`AuditTarget { endpoint, token_entry }`) and does nothing else with it.
- **Plan C (this one):** audit records, the outbox and sending, `docs/vault-audit-format.md`, `dots-admin audit verify`, the History list, an end-to-end test against a local HTTP sink, and the parked findings of plans A and B the owner chose to fold in. OIDC login and macOS/Linux stay out.

## Global Constraints

- Repository root `C:/Users/aksha/dots`, branch `feat/env-vault-c` (at `6ad8e70f`, origin/master after PR #67, when this plan was written), worked in `.claude/worktrees/env-vault-c`; all commands run from the worktree root unless a step says otherwise (Git Bash). UI commands run in `ui/`, written as `(cd ui && …)`. Never commit to `master`. `git add` names its files: never `git add -A` or `git add .` (`.memsearch/`, `target*` and the garbled `C<U+F03A>/` folder are not ours). Conventional Commits, no AI or attribution trailer.
- Builds: `export CARGO_TARGET_DIR=C:/Users/aksha/dots/target-env-vault-c-exec CARGO_INCREMENTAL=0`. Every cargo command takes `-j 2`, and every test run `-- --test-threads 4`. Run one cargo or vitest command at a time. Never share a target folder with another worktree. Check free space on C: before a full run (`Get-PSDrive C`). If a build is killed (low memory), do not start it again on your own: ask the controller.
- UI: Bun via `bun` and `bun x`, never `bunx`. Tests: `(cd ui && DEBUG_PRINT_LIMIT=0 bun run test --maxWorkers=2 [files])`; without `--maxWorkers=2` the workers fail to start. Accept a full run only when "Test Files N passed (N)" equals `find ui/src -name '*.test.ts*' | wc -l`. The typecheck, `(cd ui && bun run typecheck)`, runs after the tests. No formatter over UI files; keep new lines within 100 columns.
- Never run the real `claude` CLI, the app or an installer, and never touch `%LOCALAPPDATA%\dots`. Dots never reads, copies or sends the claude CLI's credentials. No network: the end-to-end test's sink listens on `127.0.0.1`. No new crate or npm dependency.
- **No value may ever reach a record, a line, the outbox, History, a log or an error text.** A record holds names, ids, codes and times only. Redaction is a backstop, not the defence. Every test that reads lines or History asserts that the values it served are absent.
- **Migration:** `0037_audit.sql`, after master's `0036_intents.sql`. Before the branch is pushed, fetch and compare the migration numbers with origin/master; renumber if master took 0037.
- **Strict UI schemas.** A change to a type the UI parses updates `ui/src/api/types.ts` and the samples in `apps/dots-app/src/contract.rs` in the same commit, then regenerates the fixture with `UPDATE_CONTRACT=1 cargo test -p dots-app contract -j 2 -- --test-threads 4`.
- The owner's decisions (2026-11-06):
  1. Backoff: 30 seconds, doubling, at most 1 hour, each wait ±20%, back to 30 seconds after a 2xx. New records go about 5 seconds after they are written.
  2. History: a tab on the account's page; this PC's records, newest first, 30 days and at most 1,000 rows; time, event, variable, dot, consumer, reason, and sent or queued. `fetched` is left out of History (it is still queued and sent).
  3. A failed send shows on the account's page only: the Overview line and the top of History. No Inbox item, no notification.
  4. Fold in plan B's final-review follow-ups (run start behind an in-flight fetch, half-failed Remove, start-up fetches outliving shutdown, the SOPS age key nobody can remove), the `dots-admin` footguns (`--out`/`--file` over the input or a key; a partial key file) and the smoke follow-ups ("Not set in Dots' environment" for org names, the offline time in UTC, Env Add defaulting to Secret).
  5. A policy without `audit`: records are listed in History only; nothing is queued and no `seq` is spent.
  6. Remove keeps the queue and the chain and deletes History; joining again sends the queue and goes on from the next `seq`.
  7. The record adds `install` and `user.dots_id`. `source_identity`: an OpenBao login's user name, the Infisical client id, the SOPS age public key, `null` for an OpenBao token.
  8. What counts: each name a run's start asks for (MCP servers, the shell, the retry after a sign-in refresh) and each name of a Library Check or probe in an account (`dot` and `run_id` null). Status reads record nothing.
  9. The Env tab's Add dialog has neither Plain nor Secret chosen; Save is disabled until one is.
- Numbers, as the tests pin them: `BATCH = 500`, `SEND_AFTER = 5 s`, `FIRST_RETRY = 30 s`, `MAX_RETRY = 1 h`, `JITTER = 0.2`; `AuditLimits::default()` is `max_queued: 100_000`, `drop_chunk: 1_000`, `history_rows: 1_000`, `history_days: 30`; `AUDIT_REFRESH_MS = 30_000` (the UI reads the audit view again every 30 seconds while it is shown).
- Exact words, as the tests pin them:
  - Core: `the endpoint answered <status>`, `no answer within 20 seconds`, `could not reach the endpoint`, `the send did not go through`; `the audit token <entry> is not available: <why>` with `<why>` one of `the account has no such entry`, `the saved copy expired`, `access was revoked`, `the policy is invalid`, `offline, with no saved copy`, `the account is not joined`, `this PC is not on the member list`; `audit.endpoint '<e>' must be an https:// URL (http:// only for localhost or 127.0.0.1)`; `removing it did not finish (<error>); remove it again`; the status texts `offline (<why>); using the saved copy` and `policy invalid: <error>; using the saved copy`.
  - Reason codes: `not_granted`, `not_defined`, `denied_by_policy`, `denied_by_workspace`, `shell_secrets_forbidden`, `org_offline`, `org_expired`, `org_revoked`, `policy_invalid`, `org_removed`, `not_member`; `denied`, `removed`; `offline_copy`, `queue_full`.
  - `dots-admin`: `<org> (<install>): <n> records, seq <a> to <b>`, `ok: every chain is whole`, `the records are not whole: <n> problem` / `problems`, `<file> holds no records`, `<n> lines repeat a record already read (a batch sent twice); they are not counted`, the findings' texts in `verify.rs`; `<path> is a file this command reads; choose another output file`; `<path> holds a minisign key; dots-admin never overwrites a key. Choose another output file`.
  - UI: the tab `History`; `Audit: <n> records waiting; last send failed <local time>: <why>`; `Records go to <endpoint>.`; `<account> has no audit endpoint: these records stay on this PC.`; `Nothing recorded yet.`; the events `Used`, `Refused`, `Fetched`, `Offline`, `Access revoked`, `Expired`, `Policy invalid`; the states `Kept on this PC`, `Waiting to send`, `Sent`, `Dropped unsent`; `This PC keeps sign-in details for '<id>', which is not joined.`; `Not in Personal: <names>. A server that needs one cannot start until it is set.`; the Library badge `Not in Personal: <names>`; the Env dialog's `Kind`, `Plain`, `Secret`.
- Names: core `vault::audit::{record, verify, writer, sender, view}`; store `AuditLimits`, `HistoryRow`, `HistoryState`, `NewAudit`; app commands `org_audit`, `org_leftover_keys`; UI `commands.orgAudit`, `commands.orgLeftoverKeys`, `keys.orgAudit`, `keys.orgLeftoverKeys`, `screens/accounts/AccountHistory.tsx`.

## Review Focus

The inputs and failure modes the spec implies that are most likely to bite a person, each pinned to the test that covers it:

1. **A value reaching a record or the request.** The token entry, an org secret, a plain value or the source's sign-in token must never be in a line, the History or a request body; only the bearer header carries the token. Tests: `vault_audit_fetch_test::queued` (every line of Task 3's tests), `vault_audit_resolve_test::resolutions` (Task 4), `vault_audit_send_test::a_pass_posts_the_queue_with_the_token_and_a_2xx_empties_it` (Task 5), `runtime_audit_test::a_runs_records_reach_a_local_sink_as_a_whole_chain_with_the_bearer_token` (Task 7), and `commands.test.ts`'s refusal of a History row with a `value` (Task 8).
2. **Records written at once, and a chain across restarts and leaving.** Two writers must never take one `seq`; the chain goes on after a restart, after the queue empties, and after Remove and a new Join. Tests: `vault_audit_fetch_test::records_written_at_once_take_one_seq_each`, `leaving_keeps_the_queue_and_the_chain_and_joining_again_goes_on` (Task 3); `vault_audit_store_test::queued_records_count_up_and_chain_on_the_line_before` (Task 2).
3. **An endpoint that is down or refuses for a long time.** The batch is kept, the next try waits and doubles, a new record does not jump the wait, and the queue drops its oldest records with a record that says which, which `verify` accepts. Tests: `vault_audit_send_test::a_failed_send_keeps_the_batch_and_waits_before_the_next_try` (Task 5); `vault_audit_store_test::a_full_queue_drops_its_oldest_records_and_says_which` (Task 2); `runtime_audit_test` (503, then 200; Task 7).
4. **A 2xx that never reached Dots.** The receiver gets the batch twice; `verify` counts the repeat and passes, but another record with a seen `seq` fails. Tests: `vault_audit_record_test::a_batch_sent_twice_is_counted_and_another_record_with_a_seen_seq_is_a_conflict` (Task 1); `admin_test::audit_verify_counts_a_batch_sent_twice_and_passes` (Task 6).
5. **An account without `audit`, or one that gives no token.** Nothing is queued or posted, History still lists, and a PC with no token keeps its queue and says why. Tests: `vault_audit_fetch_test::without_audit_in_the_policy_nothing_is_queued_and_history_still_lists` (Task 3); `vault_audit_send_test::without_audit_or_without_the_token_nothing_is_posted` (Task 5); `AccountHistory.test.tsx` `an account with no audit endpoint keeps its records on this PC` (Task 9).

---

## Context

Plans A and B built the vault and the orgs. `Orgs` (`vault/org/service.rs`) fetches each org through its source and applies the outcome (`refresh_org`, `keep_fetch`, `fall_back`, `expire_if_due`, `load_one`); `Vault::resolve_in` (`vault/resolve.rs`) resolves a dot's names for a consumer, called by `RunTools::open` (`run_tools.rs`) for each MCP server, the shell and the retry after a sign-in refresh; `Vault::check_values` serves a Library Check or probe in an account. Since plan B the accounts design renamed Organisations to **Accounts**: `ui/src/screens/Accounts.tsx` shows each account with tabs (Overview, Members, Workspaces, API keys), while the code keeps `orgs` and `vault::org`. The usage counts' sender (`usage_counts.rs`: `CountsSender`, `HttpSender`, `spawn`) is the pattern the audit sender follows.

Task 1 is the record, its line, the chain checker and the format doc; Task 2 the store; Task 3 the writer and the fetch outcomes; Task 4 the resolutions; Task 5 the sender; Task 6 `dots-admin audit verify`; Task 7 the end-to-end test; Task 8 the app command, the contract and the UI's types; Task 9 the History tab; Tasks 10 to 13 the folded findings; Task 14 the docs.

**How this plan's code was written:** every block below was generated from a prototype, one commit per task, on branch `proto/env-vault-c` (`.claude/worktrees/proto-env-vault-c`, based on `6ad8e70f`): Task 1 `a04ba1f1`, Task 2 `bce41019`, Task 3 `19d7bf19`, Task 4 `8452e567`, Task 5 `dc11ed91`, Task 6 `16d22ffb`, Task 7 `2506bf80`, Task 8 `e07c7bd5`, Task 9 `59b1a288`, Task 10 `08ff2876`, Task 11 `626a2079`, Task 12 `aaf37d72`, Task 13 `42b7901e`, Task 14 `9fa09882`. Each commit is `cargo fmt` clean. At Task 14 `cargo clippy -p <crate> --all-targets -j 2 -- -D warnings` is clean for dots-core, dots-admin and dots-app (run per crate: a whole-workspace run was stopped for low memory), the workspace's tests pass (core 2672, app 197, admin 40; in the one full core run, master's `server_v1_test::read_run_status` failed on timing (queued, not yet running) and passed 3 of 3 alone), and the UI suite passes (142 files, 1544 tests; typecheck clean).
The replay (`python replay.py <plan> <checkout> <proto> T1=… T14=…`, 2026-11-06, onto a clean detached checkout of `6ad8e70f`) reported "matches" for all 14 tasks: after each task the staged tree equalled its prototype commit. `python apply_step.py <plan> <checkout> --check 1 proto/env-vault-c 6ad8e70f` found 0 problems in 183 blocks. `gen.py` gained a fallback (`edits_stepwise`) for Task 3's `service.rs` diff, whose hunks sat too close for its first pass to find a unique passage.
Step 2 of each task fails as it says: a Rust task fails to compile (a type, field or function the test uses does not exist yet) unless it says otherwise; a UI task's new test that renders the app fails at its first query for what is not there yet.

Baseline at `6ad8e70f`: core 2630, app 197, admin 35 tests, all passing; UI 141 files, 1535 tests.

## Deviations from the spec

| Spec | This plan | Why |
|---|---|---|
| §10 the record's fields | Adds a top-level `install` (the install id) and `user.dots_id` (the Dots ID fingerprint), and `dropped: {from, to}` on a full queue's `expired` record. Every key is always present, `null` when it says nothing | Decision 7: §12 checks "per org and install", and the record had no install; a receiver reads one shape |
| §10 "the SHA-256 of the previous record's canonical JSON" | Canonical JSON is the line Dots writes: keys sorted by their bytes at every level, no white space. `prev` is `sha256:` and the lower-case hex of the previous line's bytes; `null` for seq 1 | A receiver hashes the lines as it got them, without re-serialising |
| §10 `denied` "with `reason`, a `MissingWhy`" | `reason` is always a code: the `Unresolved` codes (including `denied_by_workspace` and `not_member`, which the accounts design added), `denied`/`removed` for `revoked`, `offline_copy`/`queue_full` for `expired`, `null` otherwise. No free text | A code is stable for project 2, and free text could carry an address or a name it should not |
| §10 "one per resolution attempt" | One per name asked for, in order, up to and including the first that does not resolve for that consumer (later names are not asked for). `source` is `personal` when the value came from Personal. A Library Check or probe in an account records with no dot or run. `Vault::resolve` (no dot) and status reads record nothing | Decision 8 |
| §10 "one per fetch outcome per org" | Each fetch of a joined account: Refresh, the timers, the start-up fetch, a run start's refresh, and Join and Update when they work; plus each expiry of the copy, one found at start included. A Join or Update that fails records nothing. A PC taken off the member list records `revoked` with `removed`; a refused sign-in `revoked` with `denied` | A failed Join has no account to chain into; a failed Update changes nothing |
| §10 the queue and sending (silent on an account without `audit`) | Without `audit`, records go to History only (`local`): nothing is queued and no `seq` is spent | Decision 5 |
| §10 "keeps the newest 100,000 records per org and records the drop as an `expired` event" | At 100,000 queued, the oldest 1,000 go at once, and one `expired` record (`queue_full`, `dropped`) names them | One record per 1,000 dropped, not one per record |
| §10 "anything else retries with backoff" | 30 s doubling to 1 h, ±20%, reset by a 2xx; per account. A PC with no value for the token entry does not count that as a failure: the next record written tries again | Decision 1; a revoked or waiting PC should not back off for an hour once it reconnects |
| §6.3 "`audit.endpoint` must be `https://`" (§16) | Also `http://` to `localhost` or `127.0.0.1` | As join code addresses: the end-to-end test and the owner's smoke use a sink on this PC |
| §10 "People see their own records in a History list" | A History tab on the account's page: 30 days, at most 1,000 rows, newest first, no `fetched`; each row `Kept on this PC`, `Waiting to send`, `Sent` or `Dropped unsent`. Only accounts with tabs (active or legacy members) show it; a waiting or removed PC sees Overview, which still shows a failing send | Decision 2; the accounts design gives waiting and removed PCs no tabs |
| §10 (silent) a failed send | Overview's line `Audit: <n> records waiting; last send failed <time>: <why>`, also at the top of History. The view is read again every 30 s while shown | Decision 3; sends raise no runtime event |
| §6.4 Remove (silent on the queue) | Remove deletes History and keeps `audit_outbox` and `audit_chain` | Decision 6 |
| §16 follow-ups (not in plan B) | Task 10: a run start reads staleness before it waits for the account's lock; the start-up fetches are a `JoinSet` dropped at shutdown. Task 11: a half-failed Remove leaves the account listed as damaged; leftover sign-in details are listed with Remove. Task 12: `dots-admin` refuses an output over its inputs or a key, and deletes a half-written key. Task 13: "Not in Personal", no UTC time in the status, Env Add with no kind chosen | Decision 4 |
| §9 Env tab "Add variable … name, kind and value" | Neither kind is chosen; Save is disabled until one is. A watcher's "Add to vault…" starts as Secret (`initialKind`) | Decision 9; the watcher only asks for secrets |

## File Map

- `crates/dots-core/src/vault/audit/{mod,record,verify}.rs`, `docs/vault-audit-format.md`, `crates/dots-core/tests/vault_audit_record_test.rs` (new); `src/vault/mod.rs` (Task 1).
- `crates/dots-core/migrations/0037_audit.sql`, `src/store/audit.rs`, `tests/vault_audit_store_test.rs` (new); `src/store/{mod,orgs}.rs` (Task 2).
- `crates/dots-core/src/vault/audit/writer.rs`, `tests/vault_audit_fetch_test.rs` (new); `src/vault/audit/mod.rs`, `src/vault/org/service.rs`, `src/vault/org/sources/mod.rs` (Task 3).
- `crates/dots-core/tests/vault_audit_resolve_test.rs` (new); `src/vault/resolve.rs`, `src/run_tools.rs`, `tests/vault_workspace_use_test.rs` (Task 4).
- `crates/dots-core/src/vault/audit/sender.rs`, `tests/vault_audit_send_test.rs` (new); `src/vault/audit/mod.rs`, `src/vault/org/policy.rs`, `src/runtime.rs`, `tests/vault_org_policy_test.rs` (Task 5).
- `crates/dots-admin/src/audit.rs` (new); `src/lib.rs`, `tests/admin_test.rs` (Task 6).
- `crates/dots-core/tests/runtime_audit_test.rs` (new) (Task 7).
- `crates/dots-core/src/vault/audit/view.rs` (new); `src/vault/audit/mod.rs`, `src/runtime.rs`, `tests/runtime_audit_test.rs`; `apps/dots-app/src/{commands,contract,lib}.rs`; `ui/src/api/{types.ts,commands.ts,commands.test.ts,contract.test.ts,__fixtures__/contract.json}`, `ui/src/lib/query.ts` (Task 8).
- `ui/src/screens/accounts/AccountHistory.tsx`, `AccountHistory.test.tsx` (new); `ui/src/screens/Accounts.tsx`, `ui/src/test/fixtures.ts` (Task 9).
- `crates/dots-core/src/vault/org/service.rs`, `tests/{vault_org_resolve_test,vault_org_timers_test}.rs` (Task 10).
- `crates/dots-core/src/vault/org/service.rs`, `tests/vault_orgs_test.rs`; `apps/dots-app/src/{commands,lib}.rs`; `ui/src/api/{commands.ts,commands.test.ts,__fixtures__/contract.json}`, `ui/src/lib/query.ts`, `ui/src/screens/{Accounts.tsx,Accounts.test.tsx}` (Task 11).
- `crates/dots-admin/src/{lib,join,policy,roster,keys}.rs`, `tests/admin_test.rs` (Task 12).
- `crates/dots-core/src/vault/org/service.rs`, `tests/vault_orgs_test.rs`; `apps/dots-app/src/contract.rs`; `ui/src/api/__fixtures__/contract.json`, `ui/src/screens/dot-editor/{DotForm,ToolsAndSkills,ToolsAndSkills.test,WatchersTab}.tsx`, `ui/src/screens/library/{EnvTab,McpServersTab,LibraryScreen.test}.tsx` (Task 13).
- `docs/vault-admin.md`, `docs/superpowers/specs/2026-10-21-env-vault-design.md`, `docs/verification/2026-11-06-env-vault-c-smoke.md` (new) (Task 14).

---

### Task 1: Audit records are canonical JSON lines chained by seq and prev, with a chain checker and the format doc

`AuditRecord` is one record; `line()` is its canonical JSON and `line_hash` the next record's `prev`. `verify_lines` reads a file of lines as a receiver holds them and reports, per account and install, gaps (and whether the PC's own `queue_full` record explains one), a broken link, a seq out of order and a conflicting repeat; a line sent twice is counted. `docs/vault-audit-format.md` is what project 2 implements.

**Files:**
- Create: `crates/dots-core/src/vault/audit/{mod,record,verify}.rs`, `docs/vault-audit-format.md`
- Modify: `crates/dots-core/src/vault/mod.rs`
- Test: `crates/dots-core/tests/vault_audit_record_test.rs`

**Interfaces:**
- Consumes: `crate::util::token_hash(&str) -> String` (`sha256:<hex>`), `str_enum!`.
- Produces: `vault::audit::record::{AuditRecord { v, id, seq, prev, at, org_id, install, user, dot, run_id, event, name, consumer, source, reason, dropped }, AuditUser { windows, machine, source_identity, dots_id }, AuditDot { id, name }, Dropped { from, to }, Event { Resolved, Denied, Fetched, Offline, Revoked, Expired, PolicyInvalid }, RECORD_VERSION, REASON_DENIED, REASON_REMOVED, REASON_OFFLINE_COPY, REASON_QUEUE_FULL}`; `AuditRecord::line(&self) -> String`, `AuditRecord::parse(&str) -> Result<AuditRecord, String>`; `stamp(DateTime<Utc>) -> String` (RFC 3339 to the second); `line_hash(&str) -> String`; `canonical(&Value) -> String`; `vault::audit::verify::{verify_lines(&str) -> ChainReport, ChainReport { chains, findings, repeated }, ChainReport::ok(), ChainSummary { org_id, install, records, first_seq, last_seq }, Finding::{Unreadable, Gap, Conflict, OutOfOrder, BrokenLink}, Finding::is_failure(), Finding::text()}`. All re-exported from `vault::audit`.

- [ ] **Step 1: Write the failing tests**

**Create `crates/dots-core/tests/vault_audit_record_test.rs`:**

```rust
//! Audit records (env vault spec §10, §13): the line is canonical JSON with every key, `prev`
//! chains the lines, and `verify_lines` finds what a receiver must see: gaps, changed
//! records, a seq out of order, and the gaps a full queue explains itself.

use dots_core::vault::audit::record::REASON_QUEUE_FULL;
use dots_core::vault::audit::{
    canonical, line_hash, verify_lines, AuditDot, AuditRecord, AuditUser, ChainSummary, Dropped,
    Event, Finding,
};
use serde_json::json;

const INSTALL: &str = "01J9ZQ3V5X8K2M4N6P8R0T2V4W";

fn record(org_id: &str, seq: u64, prev: Option<String>) -> AuditRecord {
    AuditRecord {
        v: 1,
        id: format!("01J9ZQ3V5X8K2M4N6P8R0T{seq:04}"),
        seq,
        prev,
        at: "2026-10-21T09:12:03Z".into(),
        org_id: org_id.into(),
        install: INSTALL.into(),
        user: AuditUser {
            windows: Some("ACME\\jdoe".into()),
            machine: Some("LT-0423".into()),
            source_identity: Some("jdoe".into()),
            dots_id: Some("B255 6546 0D3C 9A11 7F20 44E1 C0DE 9B7A".into()),
        },
        dot: Some(AuditDot {
            id: "01J9DOT".into(),
            name: "Triage".into(),
        }),
        run_id: Some("01J9RUN".into()),
        event: Event::Resolved,
        name: Some("GITHUB_TOKEN".into()),
        consumer: Some("mcp:github".into()),
        source: Some("openbao".into()),
        reason: None,
        dropped: None,
    }
}

/// `n` chained records of `org_id`, seq 1 to `n`, as lines.
fn chain(org_id: &str, n: u64) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for seq in 1..=n {
        let prev = lines.last().map(|line| line_hash(line));
        lines.push(record(org_id, seq, prev).line());
    }
    lines
}

fn file(lines: &[String]) -> String {
    lines.iter().map(|l| format!("{l}\n")).collect()
}

#[test]
fn a_line_is_canonical_json_with_every_key_and_reads_back() {
    let mut first = record("acme-eng", 1, None);
    first.dot = None;
    first.run_id = None;
    assert_eq!(
        first.line(),
        concat!(
            r#"{"at":"2026-10-21T09:12:03Z","consumer":"mcp:github","dot":null,"dropped":null,"#,
            r#""event":"resolved","id":"01J9ZQ3V5X8K2M4N6P8R0T0001","#,
            r#""install":"01J9ZQ3V5X8K2M4N6P8R0T2V4W","#,
            r#""name":"GITHUB_TOKEN","org_id":"acme-eng","prev":null,"reason":null,"run_id":null,"#,
            r#""seq":1,"source":"openbao","#,
            r#""user":{"dots_id":"B255 6546 0D3C 9A11 7F20 44E1 C0DE 9B7A","#,
            r#""machine":"LT-0423","source_identity":"jdoe","windows":"ACME\\jdoe"},"v":1}"#
        )
    );
    assert_eq!(AuditRecord::parse(&first.line()).unwrap(), first);
}

#[test]
fn canonical_json_sorts_the_keys_of_every_object() {
    let value = json!({"b": 1, "a": {"d": [2, {"z": 1, "y": "é\"\n"}], "c": null}});
    assert_eq!(
        canonical(&value),
        r#"{"a":{"c":null,"d":[2,{"y":"é\"\n","z":1}]},"b":1}"#
    );
}

#[test]
fn a_lines_hash_is_the_sha256_of_its_bytes() {
    assert_eq!(
        line_hash("abc"),
        "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn a_record_with_an_unknown_field_or_another_version_is_refused() {
    let line = record("acme-eng", 1, None).line();
    let extra = line.replacen("{", r#"{"value":"x","#, 1);
    assert!(AuditRecord::parse(&extra)
        .unwrap_err()
        .contains("unknown field `value`"));
    let v2 = line.replace(r#""v":1"#, r#""v":2"#);
    assert_eq!(
        AuditRecord::parse(&v2).unwrap_err(),
        "record version 2 is not supported"
    );
}

#[test]
fn a_whole_chain_has_no_finding() {
    let report = verify_lines(&file(&chain("acme-eng", 5)));
    assert!(report.ok());
    assert_eq!(report.findings, vec![]);
    assert_eq!(
        report.chains,
        vec![ChainSummary {
            org_id: "acme-eng".into(),
            install: INSTALL.into(),
            records: 5,
            first_seq: 1,
            last_seq: 5,
        }]
    );
}

#[test]
fn a_file_may_start_part_way_and_use_crlf_line_ends() {
    let lines = chain("acme-eng", 6);
    let text = lines[3..].join("\r\n") + "\r\n\r\n";
    let report = verify_lines(&text);
    assert_eq!(report.findings, vec![]);
    assert_eq!(report.chains[0].first_seq, 4);
    assert_eq!(report.chains[0].records, 3);
}

#[test]
fn missing_records_are_a_gap() {
    let mut lines = chain("acme-eng", 6);
    lines.drain(1..4);
    let report = verify_lines(&file(&lines));
    assert!(!report.ok());
    let gap = Finding::Gap {
        org_id: "acme-eng".into(),
        install: INSTALL.into(),
        line: 2,
        from: 2,
        to: 4,
        dropped: false,
    };
    assert_eq!(report.findings, vec![gap.clone()]);
    assert_eq!(
        gap.text(),
        format!("acme-eng ({INSTALL}): records 2 to 4 are missing before line 2")
    );
}

#[test]
fn a_gap_the_queue_dropped_is_named_but_not_a_failure() {
    let mut lines = chain("acme-eng", 5);
    let mut expired = record("acme-eng", 6, Some(line_hash(&lines[4])));
    expired.event = Event::Expired;
    expired.reason = Some(REASON_QUEUE_FULL.into());
    expired.dropped = Some(Dropped { from: 2, to: 3 });
    expired.name = None;
    expired.consumer = None;
    lines.push(expired.line());
    lines.drain(1..3);
    let report = verify_lines(&file(&lines));
    assert!(report.ok());
    let gap = Finding::Gap {
        org_id: "acme-eng".into(),
        install: INSTALL.into(),
        line: 2,
        from: 2,
        to: 3,
        dropped: true,
    };
    assert_eq!(report.findings, vec![gap.clone()]);
    assert!(gap.text().ends_with(
        "records 2 to 3 are missing before line 2; the PC's queue was full and dropped them, \
         as its expired record says"
    ));
}

#[test]
fn a_changed_record_breaks_the_link_after_it() {
    let mut lines = chain("acme-eng", 3);
    lines[1] = lines[1].replace("GITHUB_TOKEN", "OTHER_TOKEN");
    let report = verify_lines(&file(&lines));
    let broken = Finding::BrokenLink {
        org_id: "acme-eng".into(),
        install: INSTALL.into(),
        line: 3,
        seq: 3,
    };
    assert_eq!(report.findings, vec![broken.clone()]);
    assert_eq!(
        broken.text(),
        format!(
            "acme-eng ({INSTALL}): line 3 (seq 3): its prev does not match the record before it"
        )
    );
}

#[test]
fn seq_one_has_no_prev_and_a_later_first_record_has_one() {
    let with_prev = record("acme-eng", 1, Some(line_hash("x"))).line();
    let without = record("acme-eng", 7, None).line();
    let report = verify_lines(&file(&[with_prev]));
    assert!(matches!(
        report.findings[..],
        [Finding::BrokenLink { seq: 1, .. }]
    ));
    let report = verify_lines(&file(&[without]));
    assert!(matches!(
        report.findings[..],
        [Finding::BrokenLink { seq: 7, .. }]
    ));
}

#[test]
fn a_batch_sent_twice_is_counted_and_another_record_with_a_seen_seq_is_a_conflict() {
    let lines = chain("acme-eng", 4);
    let mut twice = lines.clone();
    twice.extend(lines[2..].iter().cloned());
    let report = verify_lines(&file(&twice));
    assert_eq!(report.findings, vec![]);
    assert_eq!(report.repeated, 2);
    assert_eq!(report.chains[0].records, 4);

    let mut other = lines.clone();
    other.push(lines[1].replace("Triage", "Other"));
    let report = verify_lines(&file(&other));
    let conflict = Finding::Conflict {
        org_id: "acme-eng".into(),
        install: INSTALL.into(),
        line: 5,
        seq: 2,
    };
    assert_eq!(report.findings, vec![conflict.clone()]);
    assert_eq!(
        conflict.text(),
        format!("acme-eng ({INSTALL}): line 5 has seq 2 again, with another record")
    );
}

#[test]
fn a_seq_lower_than_the_one_before_is_out_of_order() {
    let lines = chain("acme-eng", 4);
    let swapped = vec![
        lines[0].clone(),
        lines[2].clone(),
        lines[1].clone(),
        lines[3].clone(),
    ];
    let report = verify_lines(&file(&swapped));
    assert!(report.findings.contains(&Finding::OutOfOrder {
        org_id: "acme-eng".into(),
        install: INSTALL.into(),
        line: 3,
        seq: 2,
        after: 3,
    }));
    assert!(!report.ok());
}

#[test]
fn each_org_and_install_is_its_own_chain_and_a_bad_line_is_named() {
    let acme = chain("acme-eng", 2);
    let beta = chain("beta", 2);
    let mut other_install = record("acme-eng", 1, None);
    other_install.install = "01J9OTHERINSTALL".into();
    let lines = vec![
        acme[0].clone(),
        beta[0].clone(),
        "not json".to_string(),
        acme[1].clone(),
        other_install.line(),
        beta[1].clone(),
    ];
    let report = verify_lines(&file(&lines));
    let keys: Vec<(&str, &str, usize)> = report
        .chains
        .iter()
        .map(|c| (c.org_id.as_str(), c.install.as_str(), c.records))
        .collect();
    assert_eq!(
        keys,
        vec![
            ("acme-eng", INSTALL, 2),
            ("beta", INSTALL, 2),
            ("acme-eng", "01J9OTHERINSTALL", 1)
        ]
    );
    assert!(matches!(
        &report.findings[..],
        [Finding::Unreadable { line: 3, why }] if why.starts_with("not a record: ")
    ));
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core -j 2 --test vault_audit_record_test -- --test-threads 4`
Expected: fails to compile: `unresolved import dots_core::vault::audit`.

- [ ] **Step 3: Implement**

**Create `crates/dots-core/src/vault/audit/mod.rs`:**

```rust
//! Audit records (env vault spec §10): one record per resolution attempt and per fetch
//! outcome of an org, as one line of canonical JSON. A record never holds a value. Each org's
//! records on this install are numbered by `seq` and chained by `prev`, the SHA-256 of the
//! line before, so a receiver sees a gap or a changed record. The format is
//! `docs/vault-audit-format.md`.

pub mod record;
pub mod verify;

pub use record::{
    canonical, line_hash, stamp, AuditDot, AuditRecord, AuditUser, Dropped, Event, RECORD_VERSION,
};
pub use verify::{verify_lines, ChainReport, ChainSummary, Finding};
```

**Create `crates/dots-core/src/vault/audit/record.rs`:**

```rust
//! One audit record and its line (`docs/vault-audit-format.md`). A line is the record as
//! canonical JSON: the keys of every object in byte order, no white space, every key present
//! (`null` when it says nothing). `prev` is `line_hash` of the record before it, so the chain
//! is checked on the lines exactly as they were sent.

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The `v` of every record this version writes.
pub const RECORD_VERSION: u64 = 1;

/// `reason` of a `revoked` record: the source refused this PC's sign-in details.
pub const REASON_DENIED: &str = "denied";
/// `reason` of a `revoked` record: this PC was taken off the account's member list.
pub const REASON_REMOVED: &str = "removed";
/// `reason` of an `expired` record: the org's offline copy ran out.
pub const REASON_OFFLINE_COPY: &str = "offline_copy";
/// `reason` of an `expired` record: the queue was full, and the records `dropped` names were
/// deleted unsent.
pub const REASON_QUEUE_FULL: &str = "queue_full";

str_enum!(Event {
    Resolved => "resolved",
    Denied => "denied",
    Fetched => "fetched",
    Offline => "offline",
    Revoked => "revoked",
    Expired => "expired",
    PolicyInvalid => "policy_invalid",
});

/// Who used the org on this install. None of it is a secret.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditUser {
    /// The Windows account, `DOMAIN\user`.
    pub windows: Option<String>,
    /// The PC's name.
    pub machine: Option<String>,
    /// Who the source knows this PC as: an OpenBao login's user name, an Infisical client id
    /// or this PC's SOPS age public key; `None` for an OpenBao token.
    pub source_identity: Option<String>,
    /// This install's Dots ID, as its fingerprint.
    pub dots_id: Option<String>,
}

/// The dot a record is about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditDot {
    pub id: String,
    pub name: String,
}

/// The records a full queue deleted unsent, from `from` to `to`, both included.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dropped {
    pub from: u64,
    pub to: u64,
}

/// One audit record. It never holds a value: only names, ids and what happened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditRecord {
    pub v: u64,
    /// A ULID.
    pub id: String,
    /// 1 for the org's first record on this install, then one more for each.
    pub seq: u64,
    /// `line_hash` of the record before; `None` for seq 1.
    pub prev: Option<String>,
    /// RFC 3339, to the second, in UTC.
    pub at: String,
    pub org_id: String,
    /// This install's id.
    pub install: String,
    pub user: AuditUser,
    /// The dot that resolved; `None` for a fetch or a Library check.
    pub dot: Option<AuditDot>,
    pub run_id: Option<String>,
    pub event: Event,
    /// The variable; `None` for a fetch outcome.
    pub name: Option<String>,
    /// `mcp:<server>` or `shell`; `None` for a fetch outcome.
    pub consumer: Option<String>,
    /// Where the value came from (`openbao`, `infisical`, `sops` or `personal`), or the org's
    /// source for any other event.
    pub source: Option<String>,
    /// Why, as a code: a `denied` record's reason, or one of the `REASON_` codes.
    pub reason: Option<String>,
    /// What an `expired` record of a full queue dropped.
    pub dropped: Option<Dropped>,
}

impl AuditRecord {
    /// The record's line: its canonical JSON, without a line end.
    pub fn line(&self) -> String {
        canonical(&serde_json::to_value(self).expect("a record always serializes"))
    }

    /// Reads one line. A field this version does not know, or a `v` other than 1, is refused.
    pub fn parse(line: &str) -> Result<AuditRecord, String> {
        let record: AuditRecord =
            serde_json::from_str(line).map_err(|e| format!("not a record: {e}"))?;
        if record.v != RECORD_VERSION {
            return Err(format!("record version {} is not supported", record.v));
        }
        Ok(record)
    }
}

/// A record's `at`: RFC 3339 in UTC, to the second.
pub fn stamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// `sha256:` and the lower-case hex SHA-256 of the line's bytes: the next record's `prev`.
pub fn line_hash(line: &str) -> String {
    crate::util::token_hash(line)
}

/// `value` as canonical JSON: the keys of every object sorted by their bytes, no white space.
pub fn canonical(value: &Value) -> String {
    let mut out = String::new();
    write_canonical(value, &mut out);
    out
}

fn write_canonical(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, key) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(key.clone()).to_string());
                out.push(':');
                write_canonical(&map[key], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}
```

**Create `crates/dots-core/src/vault/audit/verify.rs`:**

```rust
//! Checking records as a receiver holds them (env vault spec §12, `dots-admin audit verify`):
//! for each org and install, `seq` counts up by one and each `prev` is the hash of the line
//! before. A file may start part way through a chain, and may hold a batch sent twice (a 2xx
//! that never reached Dots): a line seen before is counted, not reported.

use std::collections::{BTreeMap, HashMap};

use super::record::{line_hash, AuditRecord, Event, REASON_QUEUE_FULL};

/// One org's records from one install, as the file holds them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainSummary {
    pub org_id: String,
    pub install: String,
    /// Records read, not counting lines sent again.
    pub records: usize,
    pub first_seq: u64,
    pub last_seq: u64,
}

/// What is wrong with a file of records. Lines count from 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finding {
    /// The line is not a record.
    Unreadable { line: usize, why: String },
    /// Records `from` to `to` of the chain are missing before `line`. `dropped`: an `expired`
    /// record of the chain says its queue was full and dropped them unsent.
    Gap {
        org_id: String,
        install: String,
        line: usize,
        from: u64,
        to: u64,
        dropped: bool,
    },
    /// The line has a `seq` seen before, with another record.
    Conflict {
        org_id: String,
        install: String,
        line: usize,
        seq: u64,
    },
    /// The line's `seq` is lower than the one before it.
    OutOfOrder {
        org_id: String,
        install: String,
        line: usize,
        seq: u64,
        after: u64,
    },
    /// The line's `prev` is not the hash of the record before it (or seq 1 has one, or a later
    /// record has none).
    BrokenLink {
        org_id: String,
        install: String,
        line: usize,
        seq: u64,
    },
}

impl Finding {
    /// Whether the finding means the records are not what the PC sent: anything but a gap
    /// that the PC's own `expired` record explains.
    pub fn is_failure(&self) -> bool {
        !matches!(self, Finding::Gap { dropped: true, .. })
    }

    /// The finding in plain words.
    pub fn text(&self) -> String {
        match self {
            Finding::Unreadable { line, why } => format!("line {line}: {why}"),
            Finding::Gap {
                org_id,
                install,
                line,
                from,
                to,
                dropped,
            } => {
                let what = if from == to {
                    format!("record {from} is missing")
                } else {
                    format!("records {from} to {to} are missing")
                };
                let why = if *dropped {
                    "; the PC's queue was full and dropped them, as its expired record says"
                } else {
                    ""
                };
                format!("{org_id} ({install}): {what} before line {line}{why}")
            }
            Finding::Conflict {
                org_id,
                install,
                line,
                seq,
            } => format!(
                "{org_id} ({install}): line {line} has seq {seq} again, with another record"
            ),
            Finding::OutOfOrder {
                org_id,
                install,
                line,
                seq,
                after,
            } => format!("{org_id} ({install}): line {line} has seq {seq}, after seq {after}"),
            Finding::BrokenLink {
                org_id,
                install,
                line,
                seq,
            } => format!(
                "{org_id} ({install}): line {line} (seq {seq}): its prev does not match the record \
                 before it"
            ),
        }
    }
}

/// What `verify_lines` found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChainReport {
    /// Each chain, in the order its first record appears.
    pub chains: Vec<ChainSummary>,
    pub findings: Vec<Finding>,
    /// Lines that repeat a record already read (a batch sent again).
    pub repeated: usize,
}

impl ChainReport {
    /// No finding is a failure: every chain is whole, or its gaps are the PC's own drops.
    pub fn ok(&self) -> bool {
        self.findings.iter().all(|f| !f.is_failure())
    }
}

/// Where a chain is: its last record, and every seq read with its line's hash.
struct Chain {
    summary: usize,
    last: Option<(u64, String)>,
    seen: HashMap<u64, String>,
}

/// Checks a file of records, one per line (`\n` or `\r\n`; blank lines are skipped).
pub fn verify_lines(text: &str) -> ChainReport {
    let mut report = ChainReport::default();
    let mut read = Vec::new();
    for (i, raw) in text.split('\n').enumerate() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if line.trim().is_empty() {
            continue;
        }
        match AuditRecord::parse(line) {
            Ok(record) => read.push((i + 1, line, record)),
            Err(why) => report
                .findings
                .push(Finding::Unreadable { line: i + 1, why }),
        }
    }
    // What each chain's `expired` records say its queue dropped.
    let mut dropped: BTreeMap<(String, String), Vec<(u64, u64)>> = BTreeMap::new();
    for (_, _, record) in &read {
        if let (Event::Expired, Some(reason), Some(range)) =
            (record.event, record.reason.as_deref(), record.dropped)
        {
            if reason == REASON_QUEUE_FULL {
                dropped
                    .entry((record.org_id.clone(), record.install.clone()))
                    .or_default()
                    .push((range.from, range.to));
            }
        }
    }
    let mut chains: HashMap<(String, String), Chain> = HashMap::new();
    for (line, text, record) in read {
        let key = (record.org_id.clone(), record.install.clone());
        let (org_id, install) = key.clone();
        let chain = chains.entry(key.clone()).or_insert_with(|| {
            report.chains.push(ChainSummary {
                org_id: org_id.clone(),
                install: install.clone(),
                records: 0,
                first_seq: record.seq,
                last_seq: record.seq,
            });
            Chain {
                summary: report.chains.len() - 1,
                last: None,
                seen: HashMap::new(),
            }
        });
        let hash = line_hash(text);
        let seq = record.seq;
        if let Some(seen) = chain.seen.get(&seq) {
            if *seen == hash {
                report.repeated += 1;
            } else {
                report.findings.push(Finding::Conflict {
                    org_id,
                    install,
                    line,
                    seq,
                });
            }
            continue;
        }
        chain.seen.insert(seq, hash.clone());
        let summary = &mut report.chains[chain.summary];
        summary.records += 1;
        summary.first_seq = summary.first_seq.min(seq);
        summary.last_seq = summary.last_seq.max(seq);
        let broken = Finding::BrokenLink {
            org_id: org_id.clone(),
            install: install.clone(),
            line,
            seq,
        };
        match &chain.last {
            Some((last, _)) if seq < *last => {
                report.findings.push(Finding::OutOfOrder {
                    org_id,
                    install,
                    line,
                    seq,
                    after: *last,
                });
                continue;
            }
            Some((last, last_hash)) if seq == last + 1 => {
                if record.prev.as_deref() != Some(last_hash.as_str()) {
                    report.findings.push(broken);
                }
            }
            Some((last, _)) => {
                let (from, to) = (last + 1, seq - 1);
                let covered = dropped
                    .get(&key)
                    .is_some_and(|ranges| ranges.iter().any(|(a, b)| *a <= from && to <= *b));
                report.findings.push(Finding::Gap {
                    org_id,
                    install,
                    line,
                    from,
                    to,
                    dropped: covered,
                });
            }
            // The file's first record of the chain: only seq 1 says what its prev must be.
            None => {
                if (seq == 1) != record.prev.is_none() {
                    report.findings.push(broken);
                }
            }
        }
        chain.last = Some((seq, hash));
    }
    report
}
```

**In `crates/dots-core/src/vault/mod.rs`, replace:**

```rust

pub mod child_env;
```

**with:**

```rust

pub mod audit;
pub mod child_env;
```

**Create `docs/vault-audit-format.md`:**

````markdown
# Dots audit records: format and endpoint

This is what a Dots install sends to an account's audit endpoint, and what a receiver (project 2's admin server, or any HTTP sink) must accept. It is version 1. The env vault spec (`docs/superpowers/specs/2026-10-21-env-vault-design.md`, §10) says why each part exists.

A record never holds a value. It says which variable was used, by which dot and for which consumer, or what a fetch of the account's source came to.

## Turning it on

The account's signed policy names the endpoint and the token:

```json
"audit": { "endpoint": "https://audit.acme.internal/dots", "token_entry": "DOTS_AUDIT_TOKEN" }
```

- `endpoint` is an `https://` URL. `http://` is accepted only for `localhost` and `127.0.0.1`, for a test sink on the same PC.
- `token_entry` names an ordinary entry of the account. Dots sends its value as the bearer token. Grant nothing for it: Dots reads it itself.

Without `audit`, Dots sends nothing. It still lists what happened in the account's **History** on that PC, but those records get no `seq` and are never sent. Records are queued from the first fetch that accepts a policy with `audit`.

Personal dots never write a record.

## The request

```http
POST <endpoint>
Content-Type: application/x-ndjson
Authorization: Bearer <value of token_entry>

<record>\n<record>\n…
```

- One request holds the records of one account, at most 500, in `seq` order. Each record is one line, ended by `\n`.
- Any `2xx` answer means the receiver has stored the batch. Dots deletes it from its queue. The body of the answer is not read.
- Any other answer, a timeout (20 seconds), or no connection: Dots keeps the batch and tries again later. Redirects are not followed, since a redirect would take the token to another host.
- A batch may arrive twice: when the receiver stored it but the `2xx` never reached Dots. Store records by (`org_id`, `install`, `seq`) and ignore a line you already hold.

### When Dots sends

- A new record is sent about 5 seconds after it is written, together with any others waiting.
- After a failure, the next try for that account waits 30 seconds, then 1, 2, 4, 8 minutes and so on, up to 1 hour between tries, each wait varied by up to 20% either way. A `2xx` sets the wait back to 30 seconds.
- While the account gives no values on that PC (access revoked, offline with no saved copy, not on the member list), there is no token, and the records wait.

### The queue

Each install queues at most 100,000 records per account. When that is reached, the oldest 1,000 are deleted unsent, and an `expired` record with reason `queue_full` says which (`dropped`). A receiver then sees a gap of exactly that range, explained by the record.

Leaving an account on a PC keeps that account's queue and its chain. Joining it again sends what was queued and carries on from the next `seq`.

## The record

One JSON object per line, in canonical form: the keys of every object sorted by their bytes, no white space, and every key present (`null` when it does not apply).

```json
{"at":"2026-10-21T09:12:03Z","consumer":"mcp:github","dot":{"id":"01J…","name":"Triage"},"dropped":null,"event":"resolved","id":"01J…","install":"01J…","name":"GITHUB_TOKEN","org_id":"acme-eng","prev":"sha256:…","reason":null,"run_id":"01J…","seq":412,"source":"openbao","user":{"dots_id":"B255 6546 …","machine":"LT-0423","source_identity":"jdoe","windows":"ACME\\jdoe"},"v":1}
```

| Key | Type | What it is |
|---|---|---|
| `v` | number | `1`. A receiver should refuse another version it does not know. |
| `id` | string | A ULID, unique for the record. |
| `seq` | number | `1` for the account's first record on this install, then one more for each record. Never reused. |
| `prev` | string or null | `sha256:` and the lower-case hex SHA-256 of the bytes of the previous record's line (no line end). `null` only for `seq` 1. |
| `at` | string | When it happened, RFC 3339 in UTC, to the second. |
| `org_id` | string | The account's id. |
| `install` | string | The install's id (a ULID). It is fixed for a data folder. |
| `user.windows` | string or null | The Windows account, `DOMAIN\user`. |
| `user.machine` | string or null | The PC's name. |
| `user.source_identity` | string or null | Who the source knows this PC as: the user name of an OpenBao `userpass` or `ldap` login, the Infisical client id, or this PC's SOPS age public key. `null` for an OpenBao token. |
| `user.dots_id` | string or null | The install's Dots ID as its fingerprint: 32 upper-case hex digits in groups of 4. |
| `dot` | object or null | `{ "id", "name" }` of the dot that resolved. `null` for a fetch and for a Library check. |
| `run_id` | string or null | The run that resolved. `null` for a fetch and for a Library check. |
| `event` | string | See below. |
| `name` | string or null | The variable. `null` for a fetch outcome. |
| `consumer` | string or null | `mcp:<server>` or `shell`. `null` for a fetch outcome. |
| `source` | string or null | For `resolved`, where the value came from: `openbao`, `infisical`, `sops`, or `personal` (the person's own entry, where the policy's `personal` mode allows it). For every other event, the account's source kind. |
| `reason` | string or null | A code, below. |
| `dropped` | object or null | `{ "from", "to" }`, both included, on an `expired` record with reason `queue_full`. |

A receiver must not depend on key order; Dots writes the canonical form so that the hash is reproducible from the line as received.

### Events

| `event` | When | `reason` |
|---|---|---|
| `resolved` | A variable was given to a consumer: an MCP server or the shell of a run, or a Library check in the account. | `null` |
| `denied` | A variable was asked for and not given. The run went on without that consumer (an MCP server) or without that variable (the shell). | Why, below. |
| `fetched` | A fetch of the source worked and its policy was accepted. | `null` |
| `offline` | A fetch could not reach the source. Dots uses its saved copy while that holds. | `null` |
| `revoked` | The source refused this PC's sign-in details, or this PC was taken off the member list. Dots wiped its copy. | `denied` or `removed` |
| `expired` | The saved copy ran out (`offline_hours` passed since the last good fetch), or the queue dropped records. | `offline_copy` or `queue_full` |
| `policy_invalid` | The source answered something Dots cannot use, or the policy did not verify. | `null` |

One record is written per variable asked for. A run start that resolves three variables for an MCP server writes three records. Names after the first that does not resolve for that consumer are not asked for, so they get none.

Fetches happen when Dots starts, every `refresh_minutes`, on **Refresh**, on **Update**, and before a run when the copy is older than `refresh_minutes`.

### Reasons of `denied`

| `reason` | Meaning |
|---|---|
| `not_granted` | The dot does not grant the variable to that consumer. |
| `not_defined` | Neither the account (nor Personal, where the policy reaches it) has the variable. |
| `denied_by_policy` | The account's `use` rules do not allow the name. |
| `denied_by_workspace` | The dot's workspace's `use` rules do not allow the name. |
| `shell_secrets_forbidden` | The value is a secret, the consumer is the shell, and the policy's `shell_secrets` is `false`. |
| `org_offline` | The source can't be reached and there is no saved copy. |
| `org_expired` | The saved copy ran out. |
| `org_revoked` | The source refused this PC's sign-in details. |
| `policy_invalid` | The account has no valid policy and no good copy. |
| `org_removed` | The account is not joined on this PC. |
| `not_member` | This PC is not on the account's member list. |

## Checking a file of records

`dots-admin audit verify <records.ndjson>` reads records as a receiver stores them, one per line, and checks each account and install on its own:

- `seq` goes up by one from one record to the next;
- each `prev` is the hash of the line before;
- `seq` 1 has no `prev`, and a later record has one.

It reports each gap, a changed record (the next record's `prev` no longer matches), a `seq` lower than the one before, and another record with a `seq` already read. A file may start part way through a chain, and a line sent twice is counted, not reported. A gap that the install's own `queue_full` record names is reported as dropped, and does not fail the check. It exits 0 when every chain is whole, 1 otherwise.

Records are not signed: a per-device key needs device registration, which is project 2's. The chain shows a gap or an edit to records already received; it does not prove which PC wrote them.
````

- [ ] **Step 4: Run them to see them pass**

Run: `cargo test -p dots-core -j 2 --test vault_audit_record_test -- --test-threads 4`
Expected: `test result: ok. 13 passed`.

- [ ] **Step 5: Check and commit**

Run `cargo fmt --check` and `cargo clippy -p dots-core --all-targets -j 2 -- -D warnings`, then:

```bash
git add crates/dots-core/src/vault/mod.rs crates/dots-core/src/vault/audit/mod.rs crates/dots-core/src/vault/audit/record.rs crates/dots-core/src/vault/audit/verify.rs crates/dots-core/tests/vault_audit_record_test.rs docs/vault-audit-format.md
git commit -m "feat(vault): audit records are canonical JSON lines chained by seq and prev, with a chain checker and the format doc"
```

---

### Task 2: The audit queue, chain and History tables, with the 100,000-record bound and the 30-day History

`Store::append_audit` writes a record in one `BEGIN IMMEDIATE` transaction: a queued record takes the account's next `seq` and the hash of its last line from `audit_chain`, goes into `audit_outbox`, and moves the chain on; at `max_queued` queued records the oldest `drop_chunk` go first, with an `expired` record naming them. A record that is not queued keeps `seq` 0. History rows are written for every record but `fetched` and pruned to `history_days` and `history_rows`. Remove deletes the account's History and keeps its queue and chain.

**Files:**
- Create: `crates/dots-core/migrations/0037_audit.sql`, `crates/dots-core/src/store/audit.rs`
- Modify: `crates/dots-core/src/store/mod.rs`, `crates/dots-core/src/store/orgs.rs`
- Test: `crates/dots-core/tests/vault_audit_store_test.rs`

**Interfaces:**
- Consumes: Task 1's `AuditRecord`, `Dropped`, `Event`, `line_hash`, `stamp`, `REASON_QUEUE_FULL`.
- Produces: `store::{AuditLimits { max_queued, drop_chunk, history_rows, history_days }` (`Default`: 100_000, 1_000, 1_000, 30), `HistoryRow { id, at, event, name, consumer, dot_name, reason, state }` (`Serialize`), `HistoryState { Local, Queued, Sent, Dropped }`, `NewAudit { record, queue, history }}`; `Store::append_audit(NewAudit, &AuditLimits) -> Result<Vec<AuditRecord>>` (the records written, an `expired` one first when the queue was full), `audit_batch(&str, u64) -> Result<Vec<(u64, String)>>` (oldest first), `audit_sent(&str, u64) -> Result<()>` (deletes up to that seq, marks their rows `sent`), `audit_waiting() -> Result<Vec<(String, u64)>>`, `audit_queued(&str) -> Result<u64>`, `audit_history(&str, u64) -> Result<Vec<HistoryRow>>` (newest first).

- [ ] **Step 1: Write the failing tests**

**Create `crates/dots-core/tests/vault_audit_store_test.rs`:**

```rust
//! The audit queue and History in the store (env vault spec §10): queued records get the next
//! `seq` and chain on the line before, a full queue drops its oldest records with an `expired`
//! record naming them, a record of an account without `audit` is only listed, and History
//! keeps 30 days and at most its row limit.

use dots_core::store::{AuditLimits, HistoryRow, HistoryState, NewAudit, Store};
use dots_core::vault::audit::record::REASON_QUEUE_FULL;
use dots_core::vault::audit::{
    line_hash, verify_lines, AuditDot, AuditRecord, AuditUser, Dropped, Event, Finding,
};
use tempfile::TempDir;

async fn store() -> (TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("dots.db")).await.unwrap();
    (dir, store)
}

fn record(org_id: &str, at: &str, name: &str) -> AuditRecord {
    AuditRecord {
        v: 1,
        id: dots_core::util::new_id(),
        seq: 0,
        prev: None,
        at: at.into(),
        org_id: org_id.into(),
        install: "01J9INSTALL".into(),
        user: AuditUser::default(),
        dot: Some(AuditDot {
            id: "01J9DOT".into(),
            name: "Triage".into(),
        }),
        run_id: Some("01J9RUN".into()),
        event: Event::Resolved,
        name: Some(name.into()),
        consumer: Some("shell".into()),
        source: Some("openbao".into()),
        reason: None,
        dropped: None,
    }
}

const AT: &str = "2026-10-22T09:00:00Z";

async fn queue(store: &Store, limits: &AuditLimits, name: &str) -> Vec<AuditRecord> {
    let new = NewAudit {
        record: record("acme-eng", AT, name),
        queue: true,
        history: true,
    };
    store.append_audit(new, limits).await.unwrap()
}

fn lines(batch: &[(u64, String)]) -> String {
    batch.iter().map(|(_, l)| format!("{l}\n")).collect()
}

#[tokio::test]
async fn queued_records_count_up_and_chain_on_the_line_before() {
    let (_dir, store) = store().await;
    let limits = AuditLimits::default();
    let first = queue(&store, &limits, "A").await;
    let second = queue(&store, &limits, "B").await;
    assert_eq!(first.len(), 1);
    assert_eq!((first[0].seq, first[0].prev.clone()), (1, None));
    assert_eq!(second[0].seq, 2);
    assert_eq!(second[0].prev, Some(line_hash(&first[0].line())));

    let batch = store.audit_batch("acme-eng", 500).await.unwrap();
    assert_eq!(batch, vec![(1, first[0].line()), (2, second[0].line())]);
    assert!(verify_lines(&lines(&batch)).findings.is_empty());
    assert_eq!(store.audit_queued("acme-eng").await.unwrap(), 2);
    assert_eq!(
        store.audit_waiting().await.unwrap(),
        vec![("acme-eng".to_string(), 2)]
    );
    // Each org has its own chain.
    let new = NewAudit {
        record: record("beta", AT, "A"),
        queue: true,
        history: true,
    };
    assert_eq!(store.append_audit(new, &limits).await.unwrap()[0].seq, 1);
}

#[tokio::test]
async fn a_record_that_is_not_queued_is_only_listed_and_spends_no_seq() {
    let (_dir, store) = store().await;
    let limits = AuditLimits::default();
    let new = NewAudit {
        record: record("acme-eng", AT, "A"),
        queue: false,
        history: true,
    };
    let written = store.append_audit(new, &limits).await.unwrap();
    assert_eq!((written[0].seq, written[0].prev.clone()), (0, None));
    assert_eq!(store.audit_queued("acme-eng").await.unwrap(), 0);
    let history = store.audit_history("acme-eng", 100).await.unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].state, HistoryState::Local);
    // Once the account sends records, the first is seq 1.
    assert_eq!(queue(&store, &limits, "B").await[0].seq, 1);
}

#[tokio::test]
async fn history_lists_newest_first_and_follows_what_was_sent() {
    let (_dir, store) = store().await;
    let limits = AuditLimits::default();
    for name in ["A", "B", "C"] {
        queue(&store, &limits, name).await;
    }
    let fetched = NewAudit {
        record: AuditRecord {
            event: Event::Fetched,
            name: None,
            consumer: None,
            dot: None,
            run_id: None,
            ..record("acme-eng", AT, "")
        },
        queue: true,
        history: false,
    };
    store.append_audit(fetched, &limits).await.unwrap();
    store.audit_sent("acme-eng", 2).await.unwrap();

    let history = store.audit_history("acme-eng", 100).await.unwrap();
    let shown: Vec<(Option<&str>, HistoryState)> = history
        .iter()
        .map(|row: &HistoryRow| (row.name.as_deref(), row.state))
        .collect();
    assert_eq!(
        shown,
        vec![
            (Some("C"), HistoryState::Queued),
            (Some("B"), HistoryState::Sent),
            (Some("A"), HistoryState::Sent),
        ]
    );
    assert_eq!(history[0].dot_name.as_deref(), Some("Triage"));
    assert_eq!(history[0].consumer.as_deref(), Some("shell"));
    // The fetch is queued but not listed.
    let left: Vec<u64> = store
        .audit_batch("acme-eng", 500)
        .await
        .unwrap()
        .iter()
        .map(|(seq, _)| *seq)
        .collect();
    assert_eq!(left, vec![3, 4]);
}

#[tokio::test]
async fn a_full_queue_drops_its_oldest_records_and_says_which() {
    let (_dir, store) = store().await;
    let limits = AuditLimits {
        max_queued: 5,
        drop_chunk: 2,
        ..AuditLimits::default()
    };
    let mut received = String::new();
    let first = queue(&store, &limits, "A").await;
    received.push_str(&format!("{}\n", first[0].line()));
    store.audit_sent("acme-eng", 1).await.unwrap();
    for name in ["B", "C", "D", "E", "F"] {
        assert_eq!(queue(&store, &limits, name).await.len(), 1);
    }
    // Seq 2 to 6 are queued: the next record drops 2 and 3.
    let written = queue(&store, &limits, "G").await;
    assert_eq!(written.len(), 2);
    let expired = &written[0];
    assert_eq!(expired.seq, 7);
    assert_eq!(expired.event, Event::Expired);
    assert_eq!(expired.reason.as_deref(), Some(REASON_QUEUE_FULL));
    assert_eq!(expired.dropped, Some(Dropped { from: 2, to: 3 }));
    assert_eq!((expired.dot.clone(), expired.name.clone()), (None, None));
    assert_eq!(written[1].seq, 8);

    let batch = store.audit_batch("acme-eng", 500).await.unwrap();
    let seqs: Vec<u64> = batch.iter().map(|(seq, _)| *seq).collect();
    assert_eq!(seqs, vec![4, 5, 6, 7, 8]);
    received.push_str(&lines(&batch));
    let report = verify_lines(&received);
    assert!(report.ok());
    assert_eq!(
        report.findings,
        vec![Finding::Gap {
            org_id: "acme-eng".into(),
            install: "01J9INSTALL".into(),
            line: 2,
            from: 2,
            to: 3,
            dropped: true,
        }]
    );
    let states: Vec<(Option<String>, HistoryState)> = store
        .audit_history("acme-eng", 100)
        .await
        .unwrap()
        .into_iter()
        .map(|row| (row.name, row.state))
        .collect();
    assert!(states.contains(&(Some("B".into()), HistoryState::Dropped)));
    assert!(states.contains(&(Some("C".into()), HistoryState::Dropped)));
    assert!(states.contains(&(None, HistoryState::Queued)));
}

#[tokio::test]
async fn history_keeps_thirty_days_and_at_most_its_row_limit() {
    let (_dir, store) = store().await;
    let limits = AuditLimits {
        history_rows: 3,
        ..AuditLimits::default()
    };
    for (at, name) in [
        ("2026-08-31T09:00:00Z", "OLD"),
        ("2026-10-01T09:00:00Z", "A"),
        ("2026-10-01T09:00:01Z", "B"),
    ] {
        let new = NewAudit {
            record: record("acme-eng", at, name),
            queue: false,
            history: true,
        };
        store.append_audit(new, &limits).await.unwrap();
    }
    let names = |rows: Vec<HistoryRow>| -> Vec<String> {
        rows.into_iter().filter_map(|r| r.name).collect()
    };
    // 2026-08-31 is more than 30 days before 2026-10-01.
    assert_eq!(
        names(store.audit_history("acme-eng", 100).await.unwrap()),
        vec!["B", "A"]
    );
    for name in ["C", "D"] {
        let new = NewAudit {
            record: record("acme-eng", "2026-10-01T09:00:02Z", name),
            queue: false,
            history: true,
        };
        store.append_audit(new, &limits).await.unwrap();
    }
    assert_eq!(
        names(store.audit_history("acme-eng", 100).await.unwrap()),
        vec!["D", "C", "B"]
    );
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core -j 2 --test vault_audit_store_test -- --test-threads 4`
Expected: fails to compile: `unresolved imports dots_core::store::AuditLimits, … NewAudit`.

- [ ] **Step 3: Implement**

**Create `crates/dots-core/migrations/0037_audit.sql`:**

```sql
-- Audit records (env vault spec §10). `audit_chain` keeps each org's last `seq` and the hash
-- of its last line, so the chain goes on after its queue is empty, and after the org is left
-- and joined again. `audit_outbox` holds the lines not yet sent; a 2xx deletes them.
-- `audit_history` is what the account's History lists on this PC: 30 days, at most 1,000 rows
-- per org; `seq` is NULL for a record of an account that sends none. No table holds a value.
CREATE TABLE audit_chain (
    org_id    TEXT PRIMARY KEY,
    seq       INTEGER NOT NULL,
    last_hash TEXT NOT NULL
);
CREATE TABLE audit_outbox (
    org_id TEXT NOT NULL,
    seq    INTEGER NOT NULL,
    line   TEXT NOT NULL,
    PRIMARY KEY (org_id, seq)
) WITHOUT ROWID;
CREATE TABLE audit_history (
    n        INTEGER PRIMARY KEY AUTOINCREMENT,
    id       TEXT NOT NULL,
    org_id   TEXT NOT NULL,
    seq      INTEGER,
    at       TEXT NOT NULL,
    event    TEXT NOT NULL,
    name     TEXT,
    consumer TEXT,
    dot_name TEXT,
    reason   TEXT,
    state    TEXT NOT NULL CHECK (state IN ('local', 'queued', 'sent', 'dropped'))
);
CREATE INDEX audit_history_org ON audit_history (org_id, n);
CREATE INDEX audit_history_seq ON audit_history (org_id, seq);
```

**Create `crates/dots-core/src/store/audit.rs`:**

```rust
//! The audit queue and History (env vault spec §10). `audit_chain` holds each org's last `seq`
//! and the hash of its last line, `audit_outbox` the lines not yet sent, and `audit_history` what
//! the account's History lists on this PC. None of them holds a value.

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, Sqlite, Transaction};

use super::Store;
use crate::vault::audit::record::REASON_QUEUE_FULL;
use crate::vault::audit::{line_hash, AuditRecord, Dropped, Event};
use crate::{Error, Result};

str_enum!(HistoryState {
    Local => "local",
    Queued => "queued",
    Sent => "sent",
    Dropped => "dropped",
});

/// How much is kept (env vault spec §10, and the person's answers of 2026-11-06).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuditLimits {
    /// The most records queued per org. Reaching it drops the oldest `drop_chunk`.
    pub max_queued: u64,
    pub drop_chunk: u64,
    /// The most History rows kept per org, and for how many days.
    pub history_rows: u64,
    pub history_days: i64,
}

impl Default for AuditLimits {
    fn default() -> Self {
        AuditLimits {
            max_queued: 100_000,
            drop_chunk: 1_000,
            history_rows: 1_000,
            history_days: 30,
        }
    }
}

/// One row of an account's History. It never holds a value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HistoryRow {
    pub id: String,
    /// RFC 3339, to the second.
    pub at: String,
    pub event: Event,
    pub name: Option<String>,
    pub consumer: Option<String>,
    pub dot_name: Option<String>,
    pub reason: Option<String>,
    /// `local` when the account sends no audit records, else `queued`, `sent` or `dropped`.
    pub state: HistoryState,
}

/// A record to keep: `queue` puts it in the org's chain and queue (the policy has `audit`),
/// `history` lists it in History (every event but `fetched`).
#[derive(Debug, Clone)]
pub struct NewAudit {
    pub record: AuditRecord,
    pub queue: bool,
    pub history: bool,
}

fn history_from_row(row: &SqliteRow) -> Result<HistoryRow> {
    Ok(HistoryRow {
        id: row.try_get("id")?,
        at: row.try_get("at")?,
        event: Event::parse(&row.try_get::<String, _>("event")?)?,
        name: row.try_get("name")?,
        consumer: row.try_get("consumer")?,
        dot_name: row.try_get("dot_name")?,
        reason: row.try_get("reason")?,
        state: HistoryState::parse(&row.try_get::<String, _>("state")?)?,
    })
}

fn seq_of(value: i64) -> Result<u64> {
    u64::try_from(value).map_err(|_| Error::Other(format!("audit seq {value} is not valid")))
}

fn as_i64(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| Error::Other(format!("audit seq {value} is too large")))
}

impl Store {
    /// Keeps `new`, under `limits`, and gives the records it wrote with their `seq` and `prev`:
    /// the record, after an `expired` one when the org's queue was full and its oldest
    /// `drop_chunk` records were dropped. A record that is not queued keeps `seq` 0 and no
    /// `prev`. History rows older than `history_days` before the record, or past the newest
    /// `history_rows`, are deleted.
    pub async fn append_audit(
        &self,
        new: NewAudit,
        limits: &AuditLimits,
    ) -> Result<Vec<AuditRecord>> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let org_id = new.record.org_id.clone();
        let mut written = Vec::new();
        if new.queue {
            let chain = sqlx::query("SELECT seq, last_hash FROM audit_chain WHERE org_id = ?")
                .bind(&org_id)
                .fetch_optional(&mut *tx)
                .await?;
            let (mut seq, mut prev) = match chain {
                Some(row) => (
                    seq_of(row.try_get("seq")?)?,
                    Some(row.try_get("last_hash")?),
                ),
                None => (0, None),
            };
            let oldest: Option<i64> =
                sqlx::query_scalar("SELECT MIN(seq) FROM audit_outbox WHERE org_id = ?")
                    .bind(&org_id)
                    .fetch_one(&mut *tx)
                    .await?;
            if let Some(oldest) = oldest.map(seq_of).transpose()? {
                if seq + 1 - oldest >= limits.max_queued {
                    let to = (oldest + limits.drop_chunk - 1).min(seq);
                    drop_queued(&mut tx, &org_id, oldest, to).await?;
                    let expired = AuditRecord {
                        id: crate::util::new_id(),
                        dot: None,
                        run_id: None,
                        event: Event::Expired,
                        name: None,
                        consumer: None,
                        reason: Some(REASON_QUEUE_FULL.into()),
                        dropped: Some(Dropped { from: oldest, to }),
                        ..new.record.clone()
                    };
                    let line = queue(&mut tx, expired.clone(), &mut seq, &mut prev).await?;
                    insert_history(&mut tx, &line, HistoryState::Queued).await?;
                    written.push(line);
                }
            }
            let line = queue(&mut tx, new.record, &mut seq, &mut prev).await?;
            if new.history {
                insert_history(&mut tx, &line, HistoryState::Queued).await?;
            }
            written.push(line);
        } else {
            if new.history {
                insert_history(&mut tx, &new.record, HistoryState::Local).await?;
            }
            written.push(new.record);
        }
        if let Some(last) = written.last() {
            prune_history(&mut tx, &org_id, &last.at, limits).await?;
        }
        tx.commit().await?;
        Ok(written)
    }

    /// The org's oldest queued lines, at most `limit`, with their `seq`, oldest first.
    pub async fn audit_batch(&self, org_id: &str, limit: u64) -> Result<Vec<(u64, String)>> {
        let rows =
            sqlx::query("SELECT seq, line FROM audit_outbox WHERE org_id = ? ORDER BY seq LIMIT ?")
                .bind(org_id)
                .bind(as_i64(limit)?)
                .fetch_all(&self.pool)
                .await?;
        rows.iter()
            .map(|row| Ok((seq_of(row.try_get("seq")?)?, row.try_get("line")?)))
            .collect()
    }

    /// The receiver took the org's records up to `through`: they leave the queue, and their
    /// History rows are sent.
    pub async fn audit_sent(&self, org_id: &str, through: u64) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM audit_outbox WHERE org_id = ? AND seq <= ?")
            .bind(org_id)
            .bind(as_i64(through)?)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE audit_history SET state = 'sent' \
             WHERE org_id = ? AND seq <= ? AND state = 'queued'",
        )
        .bind(org_id)
        .bind(as_i64(through)?)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Every org with queued records, and how many, by org id.
    pub async fn audit_waiting(&self) -> Result<Vec<(String, u64)>> {
        let rows = sqlx::query(
            "SELECT org_id, COUNT(*) AS n FROM audit_outbox GROUP BY org_id ORDER BY org_id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| Ok((row.try_get("org_id")?, seq_of(row.try_get("n")?)?)))
            .collect()
    }

    /// How many of the org's records wait to be sent.
    pub async fn audit_queued(&self, org_id: &str) -> Result<u64> {
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_outbox WHERE org_id = ?")
            .bind(org_id)
            .fetch_one(&self.pool)
            .await?;
        seq_of(n)
    }

    /// The org's History, newest first, at most `limit` rows.
    pub async fn audit_history(&self, org_id: &str, limit: u64) -> Result<Vec<HistoryRow>> {
        let rows = sqlx::query(
            "SELECT id, at, event, name, consumer, dot_name, reason, state FROM audit_history \
             WHERE org_id = ? ORDER BY n DESC LIMIT ?",
        )
        .bind(org_id)
        .bind(as_i64(limit)?)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(history_from_row).collect()
    }
}

/// Gives `record` the next `seq` and the `prev` after `seq` and `prev`, queues its line and
/// moves the chain on.
async fn queue(
    tx: &mut Transaction<'_, Sqlite>,
    record: AuditRecord,
    seq: &mut u64,
    prev: &mut Option<String>,
) -> Result<AuditRecord> {
    let record = AuditRecord {
        seq: *seq + 1,
        prev: prev.clone(),
        ..record
    };
    let line = record.line();
    let hash = line_hash(&line);
    sqlx::query("INSERT INTO audit_outbox (org_id, seq, line) VALUES (?, ?, ?)")
        .bind(&record.org_id)
        .bind(as_i64(record.seq)?)
        .bind(&line)
        .execute(&mut **tx)
        .await?;
    sqlx::query(
        "INSERT INTO audit_chain (org_id, seq, last_hash) VALUES (?, ?, ?) \
         ON CONFLICT (org_id) DO UPDATE SET seq = excluded.seq, last_hash = excluded.last_hash",
    )
    .bind(&record.org_id)
    .bind(as_i64(record.seq)?)
    .bind(&hash)
    .execute(&mut **tx)
    .await?;
    *seq = record.seq;
    *prev = Some(hash);
    Ok(record)
}

/// Deletes the org's queued records `from` to `to`, unsent; their History rows are dropped.
async fn drop_queued(
    tx: &mut Transaction<'_, Sqlite>,
    org_id: &str,
    from: u64,
    to: u64,
) -> Result<()> {
    sqlx::query("DELETE FROM audit_outbox WHERE org_id = ? AND seq BETWEEN ? AND ?")
        .bind(org_id)
        .bind(as_i64(from)?)
        .bind(as_i64(to)?)
        .execute(&mut **tx)
        .await?;
    sqlx::query(
        "UPDATE audit_history SET state = 'dropped' \
         WHERE org_id = ? AND seq BETWEEN ? AND ? AND state = 'queued'",
    )
    .bind(org_id)
    .bind(as_i64(from)?)
    .bind(as_i64(to)?)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn insert_history(
    tx: &mut Transaction<'_, Sqlite>,
    record: &AuditRecord,
    state: HistoryState,
) -> Result<()> {
    let seq = match state {
        HistoryState::Local => None,
        _ => Some(as_i64(record.seq)?),
    };
    sqlx::query(
        "INSERT INTO audit_history \
         (id, org_id, seq, at, event, name, consumer, dot_name, reason, state) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&record.id)
    .bind(&record.org_id)
    .bind(seq)
    .bind(&record.at)
    .bind(record.event.as_str())
    .bind(&record.name)
    .bind(&record.consumer)
    .bind(record.dot.as_ref().map(|d| d.name.as_str()))
    .bind(&record.reason)
    .bind(state.as_str())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Deletes the org's History rows older than `history_days` before `now`, then all but the
/// newest `history_rows`.
async fn prune_history(
    tx: &mut Transaction<'_, Sqlite>,
    org_id: &str,
    now: &str,
    limits: &AuditLimits,
) -> Result<()> {
    let now = DateTime::parse_from_rfc3339(now)
        .map_err(|e| Error::Other(format!("an audit record's time is not valid: {e}")))?
        .with_timezone(&Utc);
    let cutoff = crate::vault::audit::record::stamp(now - Duration::days(limits.history_days));
    sqlx::query("DELETE FROM audit_history WHERE org_id = ? AND at < ?")
        .bind(org_id)
        .bind(cutoff)
        .execute(&mut **tx)
        .await?;
    sqlx::query(
        "DELETE FROM audit_history WHERE org_id = ?1 AND n < \
         (SELECT n FROM audit_history WHERE org_id = ?1 ORDER BY n DESC LIMIT 1 OFFSET ?2)",
    )
    .bind(org_id)
    .bind(as_i64(limits.history_rows.saturating_sub(1))?)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
```

**In `crates/dots-core/src/store/mod.rs`, replace:**

```rust
mod attachments;
mod browser;
```

**with:**

```rust
mod attachments;
mod audit;
mod browser;
```

**In `crates/dots-core/src/store/mod.rs`, replace:**

```rust
pub use attachments::{WrittenFile, WRITING_TOOLS};
pub use held::HeldEvent;
```

**with:**

```rust
pub use attachments::{WrittenFile, WRITING_TOOLS};
pub use audit::{AuditLimits, HistoryRow, HistoryState, NewAudit};
pub use held::HeldEvent;
```

**In `crates/dots-core/src/store/orgs.rs`, replace:**

```rust
        }
        // `workspaces.scope` can't be a foreign key (it can be 'personal'), so they go here.
```

**with:**

```rust
        }
        // Its History goes; its audit queue and chain stay, so a re-join sends what is queued
        // and goes on from the next seq (env vault spec §10).
        sqlx::query("DELETE FROM audit_history WHERE org_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        // `workspaces.scope` can't be a foreign key (it can be 'personal'), so they go here.
```

- [ ] **Step 4: Run them to see them pass**

Run: `cargo test -p dots-core -j 2 --test vault_audit_store_test -- --test-threads 4`
Expected: `test result: ok. 5 passed`.

- [ ] **Step 5: Check and commit**

Run `cargo fmt --check` and `cargo clippy -p dots-core --all-targets -j 2 -- -D warnings`, then:

```bash
git add crates/dots-core/migrations/0037_audit.sql crates/dots-core/src/store/audit.rs crates/dots-core/src/store/mod.rs crates/dots-core/src/store/orgs.rs crates/dots-core/tests/vault_audit_store_test.rs
git commit -m "feat(store): audit queue, chain and History tables, with the 100,000-record bound and the 30-day History"
```

---

### Task 3: Each fetch outcome of an account writes an audit record, queued while its policy has audit

`Orgs` owns an `Audit` writer (made in `Orgs::new` from its store and clock). The writer fills in this install's ids once (install id, Windows account, PC name, Dots ID fingerprint) and serialises writes with one async lock. `Orgs` writes `fetched` for a good fetch (`keep_fetch`), `revoked`/`removed` for a PC taken off the list, `revoked`/`denied` for a refused sign-in, `offline` or `policy_invalid` for a failed fetch followed by `expired`/`offline_copy` when the copy ran out with it, `expired`/`offline_copy` from `expire_if_due` (once) and from `load_one` (a copy that ran out while Dots was closed). `queue` is the accepted policy's `audit.is_some()`. Who the source knows this PC as is read from the sign-in details when they are used, and kept in memory.

**Files:**
- Create: `crates/dots-core/src/vault/audit/writer.rs`
- Modify: `crates/dots-core/src/vault/audit/mod.rs`, `crates/dots-core/src/vault/org/service.rs`, `crates/dots-core/src/vault/org/sources/mod.rs`
- Test: `crates/dots-core/tests/vault_audit_fetch_test.rs`

**Interfaces:**
- Consumes: Task 2's `Store::append_audit`, `NewAudit`, `AuditLimits`; `crate::accounts::identity`, `Store::install_id`.
- Produces: `vault::audit::writer::{Audit, OrgAudit { org_id, source, source_identity, queue }, Usage { dot, run_id }, Happened { event, name, consumer, source, reason }, windows_user() -> Option<String>, machine_name() -> Option<String>}`; `Audit::new(Store, Arc<dyn Clock>)`, `set_limits(AuditLimits)`, `limits()`, `store()`, `clock()`, `written() -> &Notify` (woken after each queued record), `record(&OrgAudit, &Usage, Happened<'_>)` (logs a failure, never returns one); `Usage::run(dot_id, dot_name, run_id)`; `Happened::fetch(Event, Option<&str>)`; `Orgs::audit() -> &Audit`; `Orgs::org_audit(&str) -> Option<OrgAudit>` (`None` when not joined or damaged); `SourceCreds::identity() -> Option<String>`. `Audit`, `Happened`, `OrgAudit`, `Usage` re-exported from `vault::audit`.

- [ ] **Step 1: Write the failing tests**

**Create `crates/dots-core/tests/vault_audit_fetch_test.rs`:**

```rust
//! A record for each fetch outcome of an org (env vault spec §10): `fetched`, `offline`,
//! `revoked`, `expired` and `policy_invalid`, queued while the policy has `audit` and only
//! listed in History without it. History leaves `fetched` out. Leaving the org keeps its queue
//! and chain, and joining again goes on from the next `seq`.

mod common;

use chrono::Duration;
use common::fake_openbao::Failure;
use common::orgs::{keys, policy, sign, OrgsHarness, ACME, ISSUED};
use dots_core::store::HistoryState;
use dots_core::vault::audit::record::{REASON_DENIED, REASON_OFFLINE_COPY};
use dots_core::vault::audit::writer::{machine_name, windows_user};
use dots_core::vault::audit::{
    line_hash, verify_lines, AuditRecord, AuditUser, Event, Happened, Usage,
};
use dots_core::vault::org::sources::sops;
use dots_core::vault::org::SourceCreds;
use serde_json::{json, Value};
use zeroize::Zeroizing;

const AUDIT_TOKEN: &str = "audit-token-value-7d41c2";
const API_KEY: &str = "org-secret-api-key-0b9e55";

fn entries() -> Value {
    json!({"API_KEY": API_KEY, "DOTS_AUDIT_TOKEN": AUDIT_TOKEN})
}

fn audited() -> Value {
    json!({"audit": {"endpoint": "https://audit.acme.test/dots", "token_entry": "DOTS_AUDIT_TOKEN"}})
}

async fn queued(h: &OrgsHarness) -> Vec<AuditRecord> {
    let batch = h.store.audit_batch(ACME, 500).await.unwrap();
    for (_, line) in &batch {
        assert!(
            !line.contains(AUDIT_TOKEN) && !line.contains(API_KEY),
            "{line}"
        );
    }
    batch
        .iter()
        .map(|(_, line)| AuditRecord::parse(line).unwrap())
        .collect()
}

async fn history(h: &OrgsHarness) -> Vec<(Event, Option<String>, HistoryState)> {
    h.store
        .audit_history(ACME, 100)
        .await
        .unwrap()
        .into_iter()
        .map(|row| (row.event, row.reason, row.state))
        .collect()
}

#[tokio::test]
async fn a_join_queues_a_fetched_record_that_history_leaves_out() {
    let h = OrgsHarness::new().await;
    let identity = dots_core::accounts::ensure_identity(&h.store, h.secrets.as_ref())
        .await
        .unwrap();
    h.join_acme(entries(), audited()).await;

    let records = queued(&h).await;
    assert_eq!(records.len(), 1);
    let r = &records[0];
    assert_eq!((r.seq, r.prev.clone(), r.event), (1, None, Event::Fetched));
    assert_eq!(r.at, "2026-10-22T09:00:00Z");
    assert_eq!(r.org_id, ACME);
    assert_eq!(r.install, h.store.install_id().await.unwrap());
    assert_eq!(
        r.user,
        AuditUser {
            windows: windows_user(),
            machine: machine_name(),
            source_identity: None,
            dots_id: Some(identity.fingerprint),
        }
    );
    assert_eq!(r.source.as_deref(), Some("openbao"));
    assert_eq!((r.dot.clone(), r.run_id.clone()), (None, None));
    assert_eq!(
        (r.name.clone(), r.consumer.clone(), r.reason.clone()),
        (None, None, None)
    );
    assert_eq!(history(&h).await, vec![]);
}

#[tokio::test]
async fn each_failed_fetch_is_recorded_with_its_outcome() {
    let h = OrgsHarness::new().await;
    h.join_acme(entries(), audited()).await;

    h.bao.fail_next(Failure::Status(503), 1);
    h.orgs.refresh(ACME).await.unwrap();
    // A policy signed with another key.
    let forged = sign(&keys(), &policy(ACME, ISSUED, audited()));
    h.serve("dots", entries(), &forged);
    h.orgs.refresh(ACME).await.unwrap();
    h.bao.revoke_token(common::orgs::TOKEN);
    h.orgs.refresh(ACME).await.unwrap();

    let events: Vec<(u64, Event, Option<String>)> = queued(&h)
        .await
        .into_iter()
        .map(|r| (r.seq, r.event, r.reason))
        .collect();
    assert_eq!(
        events,
        vec![
            (1, Event::Fetched, None),
            (2, Event::Offline, None),
            (3, Event::PolicyInvalid, None),
            (4, Event::Revoked, Some(REASON_DENIED.into())),
        ]
    );
    assert_eq!(
        history(&h).await,
        vec![
            (
                Event::Revoked,
                Some(REASON_DENIED.into()),
                HistoryState::Queued
            ),
            (Event::PolicyInvalid, None, HistoryState::Queued),
            (Event::Offline, None, HistoryState::Queued),
        ]
    );
}

#[tokio::test]
async fn a_copy_that_runs_out_is_recorded_once_and_at_start() {
    let h = OrgsHarness::new().await;
    h.join_acme(entries(), audited()).await;
    h.bao.fail_next(Failure::Status(503), 1);
    h.orgs.refresh(ACME).await.unwrap();
    h.advance(Duration::hours(25));
    h.orgs.lookup(ACME, "API_KEY").await;
    h.orgs.lookup(ACME, "API_KEY").await;
    let events: Vec<(Event, Option<String>)> = queued(&h)
        .await
        .into_iter()
        .map(|r| (r.event, r.reason))
        .collect();
    assert_eq!(
        events,
        vec![
            (Event::Fetched, None),
            (Event::Offline, None),
            (Event::Expired, Some(REASON_OFFLINE_COPY.into())),
        ]
    );

    // A copy that ran out while Dots was closed is recorded when it starts.
    let h = OrgsHarness::new().await;
    h.join_acme(entries(), audited()).await;
    h.advance(Duration::hours(25));
    let again = h.service();
    again.load().await.unwrap();
    let last = queued(&h).await.pop().unwrap();
    assert_eq!(
        (last.seq, last.event, last.reason),
        (2, Event::Expired, Some(REASON_OFFLINE_COPY.into()))
    );
}

#[tokio::test]
async fn without_audit_in_the_policy_nothing_is_queued_and_history_still_lists() {
    let h = OrgsHarness::new().await;
    h.join_acme(entries(), json!({})).await;
    h.bao.fail_next(Failure::Status(503), 1);
    h.orgs.refresh(ACME).await.unwrap();
    assert_eq!(queued(&h).await, vec![]);
    assert_eq!(
        history(&h).await,
        vec![(Event::Offline, None, HistoryState::Local)]
    );
}

#[tokio::test]
async fn leaving_keeps_the_queue_and_the_chain_and_joining_again_goes_on() {
    let h = OrgsHarness::new().await;
    h.join_acme(entries(), audited()).await;
    h.bao.fail_next(Failure::Status(503), 1);
    h.orgs.refresh(ACME).await.unwrap();
    h.orgs.remove(ACME).await.unwrap();
    assert_eq!(history(&h).await, vec![]);
    let kept = queued(&h).await;
    assert_eq!(kept.len(), 2);

    h.join_acme(entries(), audited()).await;
    let records = queued(&h).await;
    let rejoined = records.last().unwrap();
    assert_eq!((rejoined.seq, rejoined.event), (3, Event::Fetched));
    assert_eq!(rejoined.prev, Some(line_hash(&kept[1].line())));
}

/// Records written at once (a run's start and a fetch, say) never take one seq: the writer
/// takes them one at a time, and the chain stays whole.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn records_written_at_once_take_one_seq_each() {
    let h = OrgsHarness::new().await;
    h.join_acme(entries(), audited()).await;
    let org = h.orgs.org_audit(ACME).unwrap();
    let writes: Vec<_> = (0..20)
        .map(|i| {
            let (audit, org) = (h.orgs.audit().clone(), org.clone());
            tokio::spawn(async move {
                let name = format!("NAME_{i}");
                let what = Happened {
                    event: Event::Resolved,
                    name: Some(&name),
                    consumer: Some("shell"),
                    source: None,
                    reason: None,
                };
                audit.record(&org, &Usage::default(), what).await;
            })
        })
        .collect();
    for write in writes {
        write.await.unwrap();
    }
    let seqs: Vec<u64> = queued(&h).await.iter().map(|r| r.seq).collect();
    assert_eq!(seqs, (1..=21).collect::<Vec<u64>>());
    let lines: String = h
        .store
        .audit_batch(ACME, 500)
        .await
        .unwrap()
        .iter()
        .map(|(_, line)| format!("{line}\n"))
        .collect();
    assert!(verify_lines(&lines).findings.is_empty());
}

#[test]
fn the_source_identity_is_a_login_name_a_client_id_or_the_age_public_key() {
    let token = SourceCreds::Token {
        token: Zeroizing::new("hvs.x".into()),
    };
    assert_eq!(token.identity(), None);
    let login = SourceCreds::Login {
        username: Zeroizing::new("jdoe".into()),
        password: Zeroizing::new("pw-5c1e9a".into()),
    };
    assert_eq!(login.identity(), Some("jdoe".into()));
    let machine = SourceCreds::Machine {
        client_id: Zeroizing::new("ci-7f2a".into()),
        client_secret: Zeroizing::new("cs-91b0".into()),
    };
    assert_eq!(machine.identity(), Some("ci-7f2a".into()));
    let (identity, public) = sops::new_identity();
    assert_eq!(SourceCreds::Age { identity }.identity(), Some(public));
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core -j 2 --test vault_audit_fetch_test -- --test-threads 4`
Expected: fails to compile: `unresolved import dots_core::vault::audit::writer`.

- [ ] **Step 3: Implement**

**In `crates/dots-core/src/vault/audit/mod.rs`, replace:**

```rust
pub mod verify;

```

**with:**

```rust
pub mod verify;
pub mod writer;

```

**In `crates/dots-core/src/vault/audit/mod.rs`, replace:**

```rust
pub use verify::{verify_lines, ChainReport, ChainSummary, Finding};
```

**with:**

```rust
pub use verify::{verify_lines, ChainReport, ChainSummary, Finding};
pub use writer::{Audit, Happened, OrgAudit, Usage};
```

**Create `crates/dots-core/src/vault/audit/writer.rs`:**

```rust
//! Writing audit records (env vault spec §10): each one gets this install's ids, the next
//! `seq` of its org and the hash of the line before, in one transaction. A record that cannot
//! be written is logged and never fails what it records.

use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::{Notify, OnceCell};

use super::record::{stamp, AuditDot, AuditRecord, AuditUser, Event, RECORD_VERSION};
use crate::scheduler::Clock;
use crate::store::{AuditLimits, NewAudit, Store};

/// What a record says about the org it is in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrgAudit {
    pub org_id: String,
    /// The org's source kind: `openbao`, `infisical` or `sops`.
    pub source: String,
    /// Who the source knows this PC as (`AuditUser::source_identity`).
    pub source_identity: Option<String>,
    /// Whether the org's accepted policy has `audit`: its records are queued and sent. Without
    /// it they are only listed in History.
    pub queue: bool,
}

/// What used a variable: a run of a dot, or neither for a Library check.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Usage {
    pub dot: Option<AuditDot>,
    pub run_id: Option<String>,
}

impl Usage {
    /// A run of the dot `dot_id`, named `dot_name`.
    pub fn run(dot_id: &str, dot_name: &str, run_id: &str) -> Usage {
        Usage {
            dot: Some(AuditDot {
                id: dot_id.to_string(),
                name: dot_name.to_string(),
            }),
            run_id: Some(run_id.to_string()),
        }
    }
}

/// What happened, for one record.
#[derive(Debug, Clone, Copy)]
pub struct Happened<'a> {
    pub event: Event,
    pub name: Option<&'a str>,
    pub consumer: Option<&'a str>,
    /// Where a resolved value came from; `None` for the org's source.
    pub source: Option<&'a str>,
    pub reason: Option<&'a str>,
}

impl<'a> Happened<'a> {
    /// A fetch outcome, with `reason`.
    pub fn fetch(event: Event, reason: Option<&'a str>) -> Happened<'a> {
        Happened {
            event,
            name: None,
            consumer: None,
            source: None,
            reason,
        }
    }
}

/// The Windows account Dots runs as, `DOMAIN\user` (the user alone without a domain).
pub fn windows_user() -> Option<String> {
    let user = std::env::var("USERNAME").ok().filter(|u| !u.is_empty())?;
    match std::env::var("USERDOMAIN").ok().filter(|d| !d.is_empty()) {
        Some(domain) => Some(format!("{domain}\\{user}")),
        None => Some(user),
    }
}

/// The PC's name.
pub fn machine_name() -> Option<String> {
    std::env::var("COMPUTERNAME").ok().filter(|m| !m.is_empty())
}

/// This install's ids, read once.
#[derive(Debug, Clone)]
struct Ids {
    install: String,
    windows: Option<String>,
    machine: Option<String>,
    dots_id: Option<String>,
}

/// The audit writer. Cheap to clone: every clone shares one state.
#[derive(Clone)]
pub struct Audit {
    inner: Arc<Inner>,
}

struct Inner {
    store: Store,
    clock: Arc<dyn Clock>,
    limits: Mutex<AuditLimits>,
    ids: OnceCell<Ids>,
    /// One record at a time, so two never take the same `seq`.
    write: tokio::sync::Mutex<()>,
    /// Woken after each queued record, for the sender.
    written: Notify,
}

impl std::fmt::Debug for Audit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Audit").finish_non_exhaustive()
    }
}

impl Audit {
    pub fn new(store: Store, clock: Arc<dyn Clock>) -> Audit {
        Audit {
            inner: Arc::new(Inner {
                store,
                clock,
                limits: Mutex::new(AuditLimits::default()),
                ids: OnceCell::new(),
                write: tokio::sync::Mutex::new(()),
                written: Notify::new(),
            }),
        }
    }

    /// Keeps `limits` from now on, for tests.
    pub fn set_limits(&self, limits: AuditLimits) {
        *self
            .inner
            .limits
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = limits;
    }

    pub fn limits(&self) -> AuditLimits {
        *self
            .inner
            .limits
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    pub fn store(&self) -> &Store {
        &self.inner.store
    }

    pub fn clock(&self) -> &Arc<dyn Clock> {
        &self.inner.clock
    }

    /// Woken after each record that is queued.
    pub fn written(&self) -> &Notify {
        &self.inner.written
    }

    /// Records `what` in `org`, for `usage`. A failure is logged, never returned.
    pub async fn record(&self, org: &OrgAudit, usage: &Usage, what: Happened<'_>) {
        if let Err(e) = self.write(org, usage, what).await {
            tracing::warn!(
                "org {:?}: an audit record ({}) was not written: {e}",
                org.org_id,
                what.event.as_str()
            );
        }
    }

    async fn write(&self, org: &OrgAudit, usage: &Usage, what: Happened<'_>) -> crate::Result<()> {
        let ids = self.ids().await?;
        let record = AuditRecord {
            v: RECORD_VERSION,
            id: crate::util::new_id(),
            seq: 0,
            prev: None,
            at: stamp(self.inner.clock.now()),
            org_id: org.org_id.clone(),
            install: ids.install.clone(),
            user: AuditUser {
                windows: ids.windows.clone(),
                machine: ids.machine.clone(),
                source_identity: org.source_identity.clone(),
                dots_id: ids.dots_id.clone(),
            },
            dot: usage.dot.clone(),
            run_id: usage.run_id.clone(),
            event: what.event,
            name: what.name.map(str::to_string),
            consumer: what.consumer.map(str::to_string),
            source: Some(what.source.unwrap_or(&org.source).to_string()),
            reason: what.reason.map(str::to_string),
            dropped: None,
        };
        let new = NewAudit {
            record,
            queue: org.queue,
            history: what.event != Event::Fetched,
        };
        let limits = self.limits();
        let _one = self.inner.write.lock().await;
        self.inner.store.append_audit(new, &limits).await?;
        if org.queue {
            self.inner.written.notify_one();
        }
        Ok(())
    }

    async fn ids(&self) -> crate::Result<&Ids> {
        self.inner
            .ids
            .get_or_try_init(|| async {
                let store = &self.inner.store;
                let dots_id = match crate::accounts::identity(store).await {
                    Ok(identity) => Some(identity.fingerprint),
                    Err(_) => None,
                };
                Ok::<_, crate::Error>(Ids {
                    install: store.install_id().await?,
                    windows: windows_user(),
                    machine: machine_name(),
                    dots_id,
                })
            })
            .await
    }
}
```

**In `crates/dots-core/src/vault/org/service.rs`, replace:**

```rust
use crate::store::Store;
use crate::vault::entries::EntryKind;
```

**with:**

```rust
use crate::store::Store;
use crate::vault::audit::record::{REASON_DENIED, REASON_OFFLINE_COPY, REASON_REMOVED};
use crate::vault::audit::{Audit, Event, Happened, OrgAudit, Usage};
use crate::vault::entries::EntryKind;
```

**In `crates/dots-core/src/vault/org/service.rs`, replace:**

```rust
    failed_at: Mutex<HashMap<String, DateTime<Utc>>>,
}
```

**with:**

```rust
    failed_at: Mutex<HashMap<String, DateTime<Utc>>>,
    /// The audit writer: a record for each fetch outcome, and for each resolution in an org.
    audit: Audit,
    /// Who each org's source knows this PC as, from the sign-in details last used.
    identities: Mutex<HashMap<String, Option<String>>>,
}
```

**In `crates/dots-core/src/vault/org/service.rs`, replace:**

```rust
            Arc::new(move |settings: &SourceSettings| source_for(settings, http.clone()));
        Orgs {
```

**with:**

```rust
            Arc::new(move |settings: &SourceSettings| source_for(settings, http.clone()));
        let audit = Audit::new(store.clone(), clock.clone());
        Orgs {
```

**In `crates/dots-core/src/vault/org/service.rs`, replace:**

```rust
                failed_at: Mutex::default(),
            }),
```

**with:**

```rust
                failed_at: Mutex::default(),
                audit,
                identities: Mutex::default(),
            }),
```

**In `crates/dots-core/src/vault/org/service.rs`, replace:**

```rust

    /// Reads every joined org and its offline copy, at start. A copy that ran out is deleted,
```

**with:**

```rust

    /// The audit writer, which the resolver and the sender use too.
    pub fn audit(&self) -> &Audit {
        &self.inner.audit
    }

    /// What a record of the org `org_id` says about it; `None` when it is not joined, or its
    /// settings are damaged: no record is written for it.
    pub fn org_audit(&self, org_id: &str) -> Option<OrgAudit> {
        let (row, policy) = match self.read().get(org_id) {
            Some(Slot::Joined(joined)) => (joined.row.clone(), joined.policy.clone()),
            _ => return None,
        };
        Some(self.audit_of(&row, policy.as_ref()))
    }

    /// What a record says about the org of `row` under `policy`: its records are queued only
    /// while the policy has `audit`.
    fn audit_of(&self, row: &OrgRow, policy: Option<&OrgPolicy>) -> OrgAudit {
        OrgAudit {
            org_id: row.id.clone(),
            source: row.source.kind().to_string(),
            source_identity: self.identity_of(&row.id),
            queue: policy.is_some_and(|p| p.audit.is_some()),
        }
    }

    /// Who the org's source knows this PC as, from its saved sign-in details, read once.
    fn identity_of(&self, org_id: &str) -> Option<String> {
        if let Some(known) = self.identities().get(org_id) {
            return known.clone();
        }
        let identity = match SourceCreds::load(self.inner.secrets.as_ref(), org_id) {
            Ok(Some(creds)) => creds.identity(),
            _ => None,
        };
        self.identities()
            .insert(org_id.to_string(), identity.clone());
        identity
    }

    /// Keeps who `creds` sign in as, for the org's records.
    fn remember_identity(&self, org_id: &str, creds: &SourceCreds) {
        self.identities()
            .insert(org_id.to_string(), creds.identity());
    }

    /// Records a fetch outcome of the org of `row`, under `policy`.
    async fn record_fetch(
        &self,
        row: &OrgRow,
        policy: Option<&OrgPolicy>,
        event: Event,
        reason: Option<&str>,
    ) {
        let org = self.audit_of(row, policy);
        let what = Happened::fetch(event, reason);
        self.inner.audit.record(&org, &Usage::default(), what).await;
    }

    /// Records a fetch outcome of the joined org `org_id`, under its accepted policy.
    async fn record_outcome(&self, org_id: &str, event: Event, reason: Option<&str>) {
        if let Some(org) = self.org_audit(org_id) {
            let what = Happened::fetch(event, reason);
            self.inner.audit.record(&org, &Usage::default(), what).await;
        }
    }

    /// Records a fetch that failed with `event`, then the copy running out if it did.
    async fn record_failed(&self, org_id: &str, event: Event, ran_out: bool) {
        self.record_outcome(org_id, event, None).await;
        if ran_out {
            self.record_outcome(org_id, Event::Expired, Some(REASON_OFFLINE_COPY))
                .await;
        }
    }

    fn identities(&self) -> std::sync::MutexGuard<'_, HashMap<String, Option<String>>> {
        self.inner
            .identities
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Reads every joined org and its offline copy, at start. A copy that ran out is deleted,
```

**In `crates/dots-core/src/vault/org/service.rs`, replace:**

```rust
        };

        let source = self.source(&code.source);
        let fetched = match source.fetch(&creds).await {
```

**with:**

```rust
        };

        self.remember_identity(&id, &creds);
        let source = self.source(&code.source);
        let fetched = match source.fetch(&creds).await {
```

**In `crates/dots-core/src/vault/org/service.rs`, replace:**

```rust
        }
        let now = self.inner.clock.now();
```

**with:**

```rust
        }
        self.remember_identity(org_id, &creds);
        let now = self.inner.clock.now();
```

**In `crates/dots-core/src/vault/org/service.rs`, replace:**

```rust
        self.failed_at().remove(org_id);
        let (dir, id) = (self.inner.dir.clone(), org_id.to_string());
```

**with:**

```rust
        self.failed_at().remove(org_id);
        self.identities().remove(org_id);
        let (dir, id) = (self.inner.dir.clone(), org_id.to_string());
```

**In `crates/dots-core/src/vault/org/service.rs`, replace:**

```rust
        let outcome = self.fetch(&row, source.as_ref()).await;
        if !matches!(outcome, Outcome::Accepted(..)) {
```

**with:**

```rust
        let outcome = self.fetch(&row, source.as_ref()).await;
        let denied = matches!(outcome, Outcome::Denied(_));
        if !matches!(outcome, Outcome::Accepted(..)) {
```

**In `crates/dots-core/src/vault/org/service.rs`, replace:**

```rust
            }
            Outcome::Unreachable(reason) => self.fall_back(org_id, None, Some(reason)).await,
            Outcome::Invalid(error) => self.fall_back(org_id, Some(error), None).await,
        }
```

**with:**

```rust
            }
            Outcome::Unreachable(reason) => {
                let ran_out = self.fall_back(org_id, None, Some(reason)).await;
                self.record_failed(org_id, Event::Offline, ran_out).await;
            }
            Outcome::Invalid(error) => {
                let ran_out = self.fall_back(org_id, Some(error), None).await;
                self.record_failed(org_id, Event::PolicyInvalid, ran_out)
                    .await;
            }
        }
        if denied {
            self.record_outcome(org_id, Event::Revoked, Some(REASON_DENIED))
                .await;
        }
```

**In `crates/dots-core/src/vault/org/service.rs`, replace:**

```rust
        let creds = match SourceCreds::load(self.inner.secrets.as_ref(), &row.id) {
            Ok(Some(creds)) => creds,
            Ok(None) => {
```

**with:**

```rust
        let creds = match SourceCreds::load(self.inner.secrets.as_ref(), &row.id) {
            Ok(Some(creds)) => {
                self.remember_identity(&row.id, &creds);
                creds
            }
            Ok(None) => {
```

**In `crates/dots-core/src/vault/org/service.rs`, replace:**

```rust
    /// After a fetch that failed but did not revoke: the values stay while the copy holds.
    /// `error` makes the org `PolicyInvalid`; otherwise it is offline for `reason`.
    async fn fall_back(&self, org_id: &str, error: Option<String>, reason: Option<String>) {
        let now = self.inner.clock.now();
```

**with:**

```rust
    /// After a fetch that failed but did not revoke: the values stay while the copy holds.
    /// `error` makes the org `PolicyInvalid`; otherwise it is offline for `reason`. Whether the
    /// copy ran out with it, taking the values it gave.
    async fn fall_back(&self, org_id: &str, error: Option<String>, reason: Option<String>) -> bool {
        let now = self.inner.clock.now();
```

**In `crates/dots-core/src/vault/org/service.rs`, replace:**

```rust
            let expired = joined.copy_until.is_some() && kept.is_none();
            if kept.is_none() {
```

**with:**

```rust
            let expired = joined.copy_until.is_some() && kept.is_none();
            let had = joined.entries.is_some();
            if kept.is_none() {
```

**In `crates/dots-core/src/vault/org/service.rs`, replace:**

```rust
            }
            expired
        });
```

**with:**

```rust
            }
            (expired, expired && had)
        });
```

**In `crates/dots-core/src/vault/org/service.rs`, replace:**

```rust
        });
        if expired == Some(true) {
            self.delete_copy(org_id).await;
```

**with:**

```rust
        });
        let (expired, ran_out) = expired.unwrap_or_default();
        if expired {
            self.delete_copy(org_id).await;
```

**In `crates/dots-core/src/vault/org/service.rs`, replace:**

```rust
        if expired {
            self.delete_copy(org_id).await;
        }
    }

    /// Lets an org that is not `Connected` lose its copy once `offline_hours` have passed since
```

**with:**

```rust
        if expired {
            self.delete_copy(org_id).await;
        }
        ran_out
    }

    /// Lets an org that is not `Connected` lose its copy once `offline_hours` have passed since
```

**In `crates/dots-core/src/vault/org/service.rs`, replace:**

```rust
            if !due(joined) {
                return false;
            }
```

**with:**

```rust
            if !due(joined) {
                return (false, false);
            }
```

**In `crates/dots-core/src/vault/org/service.rs`, replace:**

```rust
            }
            joined.entries = None;
            if locked {
```

**with:**

```rust
            }
            let had = joined.entries.take().is_some();
            if locked {
```

**In `crates/dots-core/src/vault/org/service.rs`, replace:**

```rust
            }
            true
        });
```

**with:**

```rust
            }
            (true, had)
        });
```

**In `crates/dots-core/src/vault/org/service.rs`, replace:**

```rust
        });
        if expired == Some(true) && locked {
            self.delete_copy(org_id).await;
```

**with:**

```rust
        });
        let (expired, had) = expired.unwrap_or_default();
        if expired && locked {
            self.delete_copy(org_id).await;
```

**In `crates/dots-core/src/vault/org/service.rs`, replace:**

```rust
            self.delete_copy(org_id).await;
        }
    }

```

**with:**

```rust
            self.delete_copy(org_id).await;
        }
        if had {
            self.record_outcome(org_id, Event::Expired, Some(REASON_OFFLINE_COPY))
                .await;
        }
    }

```

**In `crates/dots-core/src/vault/org/service.rs`, replace:**

```rust
        if row.member.membership != Membership::Removed {
            return self
```

**with:**

```rust
        if row.member.membership != Membership::Removed {
            self.record_fetch(&row, Some(&policy), Event::Fetched, None)
                .await;
            return self
```

**In `crates/dots-core/src/vault/org/service.rs`, replace:**

```rust
        }
        self.failed_at().remove(&row.id);
```

**with:**

```rust
        }
        self.record_fetch(&row, Some(&policy), Event::Revoked, Some(REASON_REMOVED))
            .await;
        self.failed_at().remove(&row.id);
```

**In `crates/dots-core/src/vault/org/service.rs`, replace:**

```rust
            self.delete_copy(&id).await;
            joined.policy = Some(policy);
```

**with:**

```rust
            self.delete_copy(&id).await;
            self.record_fetch(
                row,
                Some(&policy),
                Event::Expired,
                Some(REASON_OFFLINE_COPY),
            )
            .await;
            joined.policy = Some(policy);
```

**In `crates/dots-core/src/vault/org/sources/mod.rs`, replace:**

```rust
            Self::Age { .. } => "age",
        }
```

**with:**

```rust
            Self::Age { .. } => "age",
        }
    }

    /// Who these details sign in as, for an audit record (`AuditUser::source_identity`): a
    /// login's user name, an Infisical client id or this PC's age public key; `None` for a
    /// token. Never a secret.
    pub fn identity(&self) -> Option<String> {
        match self {
            Self::Token { .. } => None,
            Self::Login { username, .. } => Some(username.to_string()),
            Self::Machine { client_id, .. } => Some(client_id.to_string()),
            Self::Age { identity } => sops::public_key(identity).ok(),
        }
```

- [ ] **Step 4: Run them to see them pass**

Run: `cargo test -p dots-core -j 2 --test vault_audit_fetch_test --test vault_orgs_test --test vault_org_timers_test --test accounts_membership_test -- --test-threads 4`
Expected: `test result: ok. 7 passed` (`vault_audit_fetch_test`), `test result: ok. 45 passed` (`vault_orgs_test`), `test result: ok. 7 passed` (`vault_org_timers_test`), `test result: ok. 19 passed` (`accounts_membership_test`).

- [ ] **Step 5: Check and commit**

Run `cargo fmt --check` and `cargo clippy -p dots-core --all-targets -j 2 -- -D warnings`, then:

```bash
git add crates/dots-core/src/vault/audit/mod.rs crates/dots-core/src/vault/audit/writer.rs crates/dots-core/src/vault/org/service.rs crates/dots-core/src/vault/org/sources/mod.rs crates/dots-core/tests/vault_audit_fetch_test.rs
git commit -m "feat(vault): each fetch outcome of an org writes an audit record, queued while its policy has audit"
```

---

### Task 4: A run or a Library check in an account records each variable it asks for

`Vault::resolve_in` takes a `&Usage` and records, in an account, each name it asks for: `resolved` (with `source` `personal` when Personal gave it) up to the first that does not resolve, which is `denied` with `Unresolved::code()`. `check_values` records with `Usage::default()`. `RunTools::open` builds the run's `Usage` from the run row's dot id and the dot's name, and passes it for each MCP server, the retry after a sign-in refresh and each shell name. `Vault::resolve` (callers with no dot) records nothing.

**Files:**
- Modify: `crates/dots-core/src/vault/resolve.rs`, `crates/dots-core/src/run_tools.rs`, `crates/dots-core/tests/vault_workspace_use_test.rs`
- Test: `crates/dots-core/tests/vault_audit_resolve_test.rs`

**Interfaces:**
- Consumes: Task 3's `Orgs::org_audit`, `Orgs::audit`, `Audit::record`, `Usage`, `Happened`; `Store::get_run`.
- Produces: `Vault::resolve_in(scope, workspace_id, grants, consumer, names, usage: &Usage)` (new last parameter); `Unresolved::code(&self) -> &'static str`.

- [ ] **Step 1: Write the failing tests**

**Create `crates/dots-core/tests/vault_audit_resolve_test.rs`:**

```rust
//! A record for each resolution attempt in an org (env vault spec §10): a run records every
//! name it resolves for a consumer and the first it cannot, with the dot and the run; a Library
//! check in the account records with neither; a Personal dot records nothing. No record holds
//! a value.

mod common;

use common::orgs::{OrgsHarness, ACME};
use dots_core::accounts::NotMember;
use dots_core::vault::audit::writer::Usage;
use dots_core::vault::audit::{AuditDot, AuditRecord, Event};
use dots_core::vault::{Consumer, DotScope, EntryKind, Grants, Unresolved, Vault};
use serde_json::{json, Value};

const API_KEY: &str = "org-secret-api-key-61f0d3";
const LEVEL: &str = "org-plain-level-2c7a90";
const PERSONAL: &str = "personal-secret-only-4e8b17";

fn entries() -> Value {
    json!({"API_KEY": API_KEY, "LOG_LEVEL": LEVEL, "DOTS_AUDIT_TOKEN": "audit-token-1d3f"})
}

/// Acme with `extra` in its policy, `LOG_LEVEL` plain and audit on; Personal holds `IN_PERSONAL`.
async fn setup(extra: Value) -> (OrgsHarness, Vault) {
    let h = OrgsHarness::new().await;
    let mut extra = extra;
    extra["plain"] = json!(["LOG_LEVEL"]);
    extra["audit"] =
        json!({"endpoint": "https://audit.acme.test/dots", "token_entry": "DOTS_AUDIT_TOKEN"});
    h.join_acme(entries(), extra).await;
    let vault = h.vault();
    vault
        .add("IN_PERSONAL", EntryKind::Secret, PERSONAL)
        .await
        .unwrap();
    (h, vault)
}

fn grants(consumer: &str, names: &[&str]) -> Grants {
    Grants(
        [(
            consumer.to_string(),
            names.iter().map(|n| n.to_string()).collect(),
        )]
        .into(),
    )
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|n| n.to_string()).collect()
}

fn usage() -> Usage {
    Usage::run("01J9DOT", "Triage", "01J9RUN")
}

/// The records after the join's `fetched`, as (event, name, consumer, source, reason).
async fn resolutions(h: &OrgsHarness) -> Vec<AuditRecord> {
    let batch = h.store.audit_batch(ACME, 500).await.unwrap();
    for (_, line) in &batch {
        for value in [API_KEY, LEVEL, PERSONAL] {
            assert!(!line.contains(value), "{line} holds {value}");
        }
    }
    batch
        .iter()
        .map(|(_, line)| AuditRecord::parse(line).unwrap())
        .filter(|r| r.event != Event::Fetched)
        .collect()
}

type Row = (
    Event,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

fn row(r: &AuditRecord) -> Row {
    (
        r.event,
        r.name.clone(),
        r.consumer.clone(),
        r.source.clone(),
        r.reason.clone(),
    )
}

fn s(text: &str) -> Option<String> {
    Some(text.to_string())
}

#[tokio::test]
async fn a_run_records_each_name_it_resolves_and_the_first_it_cannot() {
    let (h, vault) = setup(json!({})).await;
    let scope = DotScope::Org(ACME.into());
    let consumer = Consumer::Mcp("github".into());
    let granted = grants("mcp:github", &["LOG_LEVEL", "API_KEY", "MISSING", "AFTER"]);
    let asked = names(&["LOG_LEVEL", "API_KEY", "MISSING", "AFTER"]);
    let missing = vault
        .resolve_in(&scope, "default", &granted, &consumer, &asked, &usage())
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(missing.why, Unresolved::NotDefined);

    let records = resolutions(&h).await;
    assert_eq!(
        records.iter().map(row).collect::<Vec<_>>(),
        vec![
            (
                Event::Resolved,
                s("LOG_LEVEL"),
                s("mcp:github"),
                s("openbao"),
                None
            ),
            (
                Event::Resolved,
                s("API_KEY"),
                s("mcp:github"),
                s("openbao"),
                None
            ),
            (
                Event::Denied,
                s("MISSING"),
                s("mcp:github"),
                s("openbao"),
                s("not_defined")
            ),
        ]
    );
    for r in &records {
        assert_eq!(
            r.dot,
            Some(AuditDot {
                id: "01J9DOT".into(),
                name: "Triage".into()
            })
        );
        assert_eq!(r.run_id.as_deref(), Some("01J9RUN"));
    }
    // History lists them, newest first.
    let shown: Vec<Option<String>> = h
        .store
        .audit_history(ACME, 100)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.name)
        .collect();
    assert_eq!(shown, vec![s("MISSING"), s("API_KEY"), s("LOG_LEVEL")]);
}

#[tokio::test]
async fn a_personal_value_the_policy_lets_in_is_recorded_as_personal() {
    let (h, vault) = setup(json!({"personal": "fallback"})).await;
    let scope = DotScope::Org(ACME.into());
    let granted = grants("shell", &["IN_PERSONAL"]);
    vault
        .resolve_in(
            &scope,
            "default",
            &granted,
            &Consumer::Shell,
            &names(&["LOG_LEVEL"]),
            &usage(),
        )
        .await
        .unwrap()
        .unwrap_err();
    let granted = grants("mcp:x", &["IN_PERSONAL"]);
    vault
        .resolve_in(
            &scope,
            "default",
            &granted,
            &Consumer::Mcp("x".into()),
            &names(&["IN_PERSONAL"]),
            &usage(),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        resolutions(&h).await.iter().map(row).collect::<Vec<_>>(),
        vec![
            (
                Event::Denied,
                s("LOG_LEVEL"),
                s("shell"),
                s("openbao"),
                s("not_granted")
            ),
            (
                Event::Resolved,
                s("IN_PERSONAL"),
                s("mcp:x"),
                s("personal"),
                None
            ),
        ]
    );
}

#[tokio::test]
async fn a_secret_kept_from_the_shell_is_denied_with_its_code() {
    let (h, vault) = setup(json!({})).await;
    let scope = DotScope::Org(ACME.into());
    let granted = grants("shell", &["API_KEY"]);
    vault
        .resolve_in(
            &scope,
            "default",
            &granted,
            &Consumer::Shell,
            &names(&["API_KEY"]),
            &usage(),
        )
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(
        resolutions(&h).await.iter().map(row).collect::<Vec<_>>(),
        vec![(
            Event::Denied,
            s("API_KEY"),
            s("shell"),
            s("openbao"),
            s("shell_secrets_forbidden")
        )]
    );
}

#[tokio::test]
async fn a_personal_dot_writes_no_record() {
    let (h, vault) = setup(json!({})).await;
    let granted = grants("shell", &["IN_PERSONAL"]);
    vault
        .resolve_in(
            &DotScope::Personal,
            "default",
            &granted,
            &Consumer::Shell,
            &names(&["IN_PERSONAL"]),
            &usage(),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(resolutions(&h).await, vec![]);
    assert_eq!(
        h.store.audit_history("personal", 100).await.unwrap(),
        vec![]
    );
}

#[tokio::test]
async fn a_check_in_the_account_records_with_no_dot_or_run() {
    let (h, vault) = setup(json!({})).await;
    let scope = DotScope::Org(ACME.into());
    vault
        .check_values(
            &scope,
            &Consumer::Mcp("github".into()),
            &names(&["API_KEY"]),
        )
        .await
        .unwrap()
        .unwrap();
    let records = resolutions(&h).await;
    assert_eq!(
        records.iter().map(row).collect::<Vec<_>>(),
        vec![(
            Event::Resolved,
            s("API_KEY"),
            s("mcp:github"),
            s("openbao"),
            None
        )]
    );
    assert_eq!(
        (records[0].dot.clone(), records[0].run_id.clone()),
        (None, None)
    );
}

#[test]
fn every_reason_has_its_code() {
    let codes: Vec<&str> = [
        Unresolved::NotGranted,
        Unresolved::NotDefined,
        Unresolved::DeniedByPolicy,
        Unresolved::DeniedByWorkspace("ops".into()),
        Unresolved::ShellSecretsForbidden,
        Unresolved::OrgOffline,
        Unresolved::OrgExpired,
        Unresolved::OrgRevoked,
        Unresolved::PolicyInvalid("x".into()),
        Unresolved::OrgRemoved,
        Unresolved::NotMember(NotMember::Waiting),
    ]
    .iter()
    .map(Unresolved::code)
    .collect();
    assert_eq!(
        codes,
        vec![
            "not_granted",
            "not_defined",
            "denied_by_policy",
            "denied_by_workspace",
            "shell_secrets_forbidden",
            "org_offline",
            "org_expired",
            "org_revoked",
            "policy_invalid",
            "org_removed",
            "not_member",
        ]
    );
}
```

**In `crates/dots-core/tests/vault_workspace_use_test.rs`, replace:**

```rust
use dots_core::store::WorkspaceRow;
use dots_core::vault::org::{may_use_in, UseRules};
```

**with:**

```rust
use dots_core::store::WorkspaceRow;
use dots_core::vault::audit::Usage;
use dots_core::vault::org::{may_use_in, UseRules};
```

**In `crates/dots-core/tests/vault_workspace_use_test.rs`, replace:**

```rust
            &names,
        )
```

**with:**

```rust
            &names,
            &Usage::default(),
        )
```

**In `crates/dots-core/tests/vault_workspace_use_test.rs`, replace:**

```rust
    let ghost = vault
        .resolve_in(&scope, "ghost", &grants, &consumer, &names)
        .await
```

**with:**

```rust
    let ghost = vault
        .resolve_in(
            &scope,
            "ghost",
            &grants,
            &consumer,
            &names,
            &Usage::default(),
        )
        .await
```

**In `crates/dots-core/tests/vault_workspace_use_test.rs`, replace:**

```rust
        let ok = vault
            .resolve_in(&scope, id, &grants, &consumer, &names)
            .await
```

**with:**

```rust
        let ok = vault
            .resolve_in(&scope, id, &grants, &consumer, &names, &Usage::default())
            .await
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core -j 2 --test vault_audit_resolve_test --test vault_workspace_use_test -- --test-threads 4`
Expected: fails to compile: `resolve_in` takes 5 arguments but 6 were supplied, and no method `code` on `Unresolved`.

- [ ] **Step 3: Implement**

**In `crates/dots-core/src/run_tools.rs`, replace:**

```rust
use crate::store::Store;
use crate::vault::{Consumer, DotScope, Grants, Resolved, Vault};
```

**with:**

```rust
use crate::store::Store;
use crate::vault::audit::{AuditDot, Usage};
use crate::vault::{Consumer, DotScope, Grants, Resolved, Vault};
```

**In `crates/dots-core/src/run_tools.rs`, replace:**

```rust
        let mut notices = Vec::new();

```

**with:**

```rust
        let mut notices = Vec::new();
        // A dot in an org records each variable it resolves, with the dot and the run (env
        // vault spec §10).
        let usage = Usage {
            dot: store.get_run(run_id).await.ok().map(|run| AuditDot {
                id: run.dot_id,
                name: dot_name.to_string(),
            }),
            run_id: Some(run_id.to_string()),
        };

```

**In `crates/dots-core/src/run_tools.rs`, replace:**

```rust
            match vault
                .resolve_in(scope, workspace_id, grants, &consumer, &config.variables())
                .await?
```

**with:**

```rust
            match vault
                .resolve_in(
                    scope,
                    workspace_id,
                    grants,
                    &consumer,
                    &config.variables(),
                    &usage,
                )
                .await?
```

**In `crates/dots-core/src/run_tools.rs`, replace:**

```rust
        }
        let shell_env =
            shell_env(vault, scope, workspace_id, grants, dot_name, &mut notices).await?;
        let Some(mut opened) = open_sessions(&items, workspace, cancel).await else {
```

**with:**

```rust
        }
        let shell_env = shell_env(
            vault,
            scope,
            workspace_id,
            grants,
            dot_name,
            &usage,
            &mut notices,
        )
        .await?;
        let Some(mut opened) = open_sessions(&items, workspace, cancel).await else {
```

**In `crates/dots-core/src/run_tools.rs`, replace:**

```rust
            let Ok(env) = vault
                .resolve_in(scope, workspace_id, grants, &consumer, &config.variables())
                .await?
```

**with:**

```rust
            let Ok(env) = vault
                .resolve_in(
                    scope,
                    workspace_id,
                    grants,
                    &consumer,
                    &config.variables(),
                    &usage,
                )
                .await?
```

**In `crates/dots-core/src/run_tools.rs`, replace:**

```rust
    dot_name: &str,
    notices: &mut Vec<String>,
```

**with:**

```rust
    dot_name: &str,
    usage: &Usage,
    notices: &mut Vec<String>,
```

**In `crates/dots-core/src/run_tools.rs`, replace:**

```rust
        match vault
            .resolve_in(scope, workspace_id, grants, &Consumer::Shell, names)
            .await?
```

**with:**

```rust
        match vault
            .resolve_in(scope, workspace_id, grants, &Consumer::Shell, names, usage)
            .await?
```

**In `crates/dots-core/src/vault/resolve.rs`, replace:**

```rust
use crate::model::Dot;
use crate::{Error, Result};
```

**with:**

```rust
use crate::model::Dot;
use crate::vault::audit::{Event, Happened, Usage};
use crate::{Error, Result};
```

**In `crates/dots-core/src/vault/resolve.rs`, replace:**

```rust
    NotMember(NotMember),
}
```

**with:**

```rust
    NotMember(NotMember),
}

impl Unresolved {
    /// The reason's code in an audit record's `reason` (`docs/vault-audit-format.md`).
    pub fn code(&self) -> &'static str {
        match self {
            Unresolved::NotGranted => "not_granted",
            Unresolved::NotDefined => "not_defined",
            Unresolved::DeniedByPolicy => "denied_by_policy",
            Unresolved::DeniedByWorkspace(_) => "denied_by_workspace",
            Unresolved::ShellSecretsForbidden => "shell_secrets_forbidden",
            Unresolved::OrgOffline => "org_offline",
            Unresolved::OrgExpired => "org_expired",
            Unresolved::OrgRevoked => "org_revoked",
            Unresolved::PolicyInvalid(_) => "policy_invalid",
            Unresolved::OrgRemoved => "org_removed",
            Unresolved::NotMember(_) => "not_member",
        }
    }
}
```

**In `crates/dots-core/src/vault/resolve.rs`, replace:**

```rust
        let granted = |name: &str| grants.allows(consumer, name);
        self.resolve_where(scope, None, &granted, consumer, names)
            .await
```

**with:**

```rust
        let granted = |name: &str| grants.allows(consumer, name);
        self.resolve_where(scope, None, &granted, consumer, names, None)
            .await
```

**In `crates/dots-core/src/vault/resolve.rs`, replace:**

```rust
    /// workspace's own narrow the names its dots may use (members spec §6.3). A workspace with
    /// no `use` rules narrows nothing.
    pub async fn resolve_in(
```

**with:**

```rust
    /// workspace's own narrow the names its dots may use (members spec §6.3). A workspace with
    /// no `use` rules narrows nothing. In an org, each name asked for is recorded for `usage`
    /// (env vault spec §10): those that resolve, and the first that does not.
    pub async fn resolve_in(
```

**In `crates/dots-core/src/vault/resolve.rs`, replace:**

```rust
        consumer: &Consumer,
        names: &[String],
    ) -> Result<std::result::Result<Resolved, Missing>> {
        let granted = |name: &str| grants.allows(consumer, name);
        self.resolve_where(scope, Some(workspace_id), &granted, consumer, names)
            .await
    }

```

**with:**

```rust
        consumer: &Consumer,
        names: &[String],
        usage: &Usage,
    ) -> Result<std::result::Result<Resolved, Missing>> {
        let granted = |name: &str| grants.allows(consumer, name);
        self.resolve_where(
            scope,
            Some(workspace_id),
            &granted,
            consumer,
            names,
            Some(usage),
        )
        .await
    }

```

**In `crates/dots-core/src/vault/resolve.rs`, replace:**

```rust
    /// `personal` mode; the first that does not resolve is the answer. An org not joined on
    /// this PC is `NotFound`.
    pub async fn check_values(
```

**with:**

```rust
    /// `personal` mode; the first that does not resolve is the answer. An org not joined on
    /// this PC is `NotFound`. In an org, each name is recorded, with no dot or run.
    pub async fn check_values(
```

**In `crates/dots-core/src/vault/resolve.rs`, replace:**

```rust
        }
        self.resolve_where(scope, None, &|_| true, consumer, names)
            .await
    }
```

**with:**

```rust
        }
        self.resolve_where(
            scope,
            None,
            &|_| true,
            consumer,
            names,
            Some(&Usage::default()),
        )
        .await
    }
```

**In `crates/dots-core/src/vault/resolve.rs`, replace:**

```rust

    /// `resolve`, with `granted` saying which names are granted to `consumer`.
    async fn resolve_where(
```

**with:**

```rust

    /// `resolve`, with `granted` saying which names are granted to `consumer`. With `usage`, a
    /// dot in an org records each name it asks for; a Personal one records nothing.
    async fn resolve_where(
```

**In `crates/dots-core/src/vault/resolve.rs`, replace:**

```rust
        consumer: &Consumer,
        names: &[String],
    ) -> Result<std::result::Result<Resolved, Missing>> {
        let read = self.in_scope(scope, workspace_id, names).await?;
        let mut out = BTreeMap::new();
        for (i, name) in names.iter().enumerate() {
```

**with:**

```rust
        consumer: &Consumer,
        names: &[String],
        usage: Option<&Usage>,
    ) -> Result<std::result::Result<Resolved, Missing>> {
        let read = self.in_scope(scope, workspace_id, names).await?;
        let audit = match (usage, scope, self.orgs()) {
            (Some(usage), DotScope::Org(org_id), Some(orgs)) => orgs
                .org_audit(org_id)
                .map(|org| (orgs.audit().clone(), org, usage)),
            _ => None,
        };
        let key = consumer.key();
        let mut out = BTreeMap::new();
        for (i, name) in names.iter().enumerate() {
```

**In `crates/dots-core/src/vault/resolve.rs`, replace:**

```rust
                .await?;
            match answer {
```

**with:**

```rust
                .await?;
            if let Some((audit, org, usage)) = &audit {
                let what = match &answer {
                    Ok(found) => Happened {
                        event: Event::Resolved,
                        name: Some(name),
                        consumer: Some(&key),
                        source: (found.source == ValueSource::Personal).then_some("personal"),
                        reason: None,
                    },
                    Err((missing, _)) => Happened {
                        event: Event::Denied,
                        name: Some(name),
                        consumer: Some(&key),
                        source: None,
                        reason: Some(missing.why.code()),
                    },
                };
                audit.record(org, usage, what).await;
            }
            match answer {
```

- [ ] **Step 4: Run them to see them pass**

Run: `cargo test -p dots-core -j 2 --test vault_audit_resolve_test --test vault_workspace_use_test -- --test-threads 4`
Expected: `test result: ok. 6 passed` (`vault_audit_resolve_test`), `test result: ok. 8 passed` (`vault_workspace_use_test`).

- [ ] **Step 5: Check and commit**

Run `cargo fmt --check` and `cargo clippy -p dots-core --all-targets -j 2 -- -D warnings`, then:

```bash
git add crates/dots-core/src/vault/resolve.rs crates/dots-core/src/run_tools.rs crates/dots-core/tests/vault_workspace_use_test.rs crates/dots-core/tests/vault_audit_resolve_test.rs
git commit -m "feat(vault): a run or a Library check in an org records each variable it asks for"
```

---

### Task 5: Audit records are posted to the account's endpoint in batches of 500, with backoff after a failure

`AuditSender::pass` sends each account's queue that is due: the policy's endpoint and the value of its token entry (`Orgs::lookup`), at most `BATCH` lines per `POST`, through an `AuditTransport`. A 2xx calls `Store::audit_sent`; anything else keeps the batch and sets the next try `retry_after(failures, jitter)` ahead (jitter from a new ULID's random bits). No token sets the error without a wait. `spawn` runs a pass at start, `SEND_AFTER` after each queued record, and when a wait ends, until shutdown. `HttpTransport` posts with the vault's `http_client()`. `Config::audit_transport` (default `HttpTransport`) is what the runtime's sender uses; `Runtime::audit_sender()` gives it. The policy accepts an `http://` endpoint to `localhost` or `127.0.0.1`.

**Files:**
- Create: `crates/dots-core/src/vault/audit/sender.rs`
- Modify: `crates/dots-core/src/vault/audit/mod.rs`, `crates/dots-core/src/vault/org/policy.rs`, `crates/dots-core/src/runtime.rs`, `crates/dots-core/tests/vault_org_policy_test.rs`
- Test: `crates/dots-core/tests/vault_audit_send_test.rs`

**Interfaces:**
- Consumes: Task 2's `audit_waiting`, `audit_batch`, `audit_sent`; Task 3's `Orgs::audit`, `Audit::clock`, `Audit::written`; `Orgs::policy`, `Orgs::lookup`, `vault::org::http_client`.
- Produces: `vault::audit::sender::{BATCH, SEND_AFTER, FIRST_RETRY, MAX_RETRY, JITTER, retry_after(u32, f64) -> chrono::Duration, AuditTransport (async fn post(&self, endpoint: &str, token: &str, body: String) -> Result<(), String>), HttpTransport::new(reqwest::Client), SendState { last_sent_at, last_error, failed_at, next_try_at } (Serialize, Default), AuditSender::new(Orgs, Arc<dyn AuditTransport>) -> Arc<AuditSender>, state(&str) -> SendState, pass() -> usize, spawn(self: Arc<Self>, CancellationToken) -> JoinHandle<()>}`; `Config::audit_transport`; `Runtime::audit_sender() -> &Arc<AuditSender>`. `AuditSender`, `AuditTransport`, `HttpTransport`, `SendState` re-exported from `vault::audit`.

- [ ] **Step 1: Write the failing tests**

**Create `crates/dots-core/tests/vault_audit_send_test.rs`:**

```rust
//! Sending audit records (env vault spec §10, `docs/vault-audit-format.md`): an org's queue
//! goes to its endpoint with the token entry's value as the bearer, at most 500 records a
//! request; a 2xx empties what was sent, and anything else keeps it and waits, from 30 seconds
//! doubling to an hour. New records go 5 seconds after they are written.

mod common;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use common::orgs::{OrgsHarness, ACME};
use dots_core::store::HistoryState;
use dots_core::vault::audit::sender::{retry_after, AuditSender, AuditTransport};
use dots_core::vault::audit::{verify_lines, Event, Happened, Usage};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

const AUDIT_TOKEN: &str = "audit-token-value-5e92b1";
const ENDPOINT: &str = "https://audit.acme.test/dots";

#[derive(Debug, Clone)]
struct Post {
    endpoint: String,
    token: String,
    body: String,
}

/// Answers each post with the next scripted answer, or a 2xx when none is left.
#[derive(Debug, Default)]
struct FakeTransport {
    posts: Mutex<Vec<Post>>,
    answers: Mutex<VecDeque<Result<(), String>>>,
}

impl FakeTransport {
    fn answer(&self, answer: Result<(), String>) {
        self.answers.lock().unwrap().push_back(answer);
    }

    fn posts(&self) -> Vec<Post> {
        self.posts.lock().unwrap().clone()
    }
}

#[async_trait]
impl AuditTransport for FakeTransport {
    async fn post(&self, endpoint: &str, token: &str, body: String) -> Result<(), String> {
        self.posts.lock().unwrap().push(Post {
            endpoint: endpoint.into(),
            token: token.into(),
            body,
        });
        self.answers.lock().unwrap().pop_front().unwrap_or(Ok(()))
    }
}

fn entries() -> Value {
    json!({"DOTS_AUDIT_TOKEN": AUDIT_TOKEN, "API_KEY": "org-secret-api-key-90c4d1"})
}

fn audited() -> Value {
    json!({"audit": {"endpoint": ENDPOINT, "token_entry": "DOTS_AUDIT_TOKEN"}})
}

async fn joined(extra: Value) -> (OrgsHarness, Arc<FakeTransport>, Arc<AuditSender>) {
    let h = OrgsHarness::new().await;
    h.join_acme(entries(), extra).await;
    let transport = Arc::new(FakeTransport::default());
    let sender = AuditSender::new(h.orgs.clone(), transport.clone());
    (h, transport, sender)
}

/// `n` resolutions of `API_KEY` by the dot's shell.
async fn resolve(h: &OrgsHarness, n: usize) {
    let org = h.orgs.org_audit(ACME).unwrap();
    let usage = Usage::run("01J9DOT", "Triage", "01J9RUN");
    for _ in 0..n {
        let what = Happened {
            event: Event::Resolved,
            name: Some("API_KEY"),
            consumer: Some("shell"),
            source: None,
            reason: None,
        };
        h.orgs.audit().record(&org, &usage, what).await;
    }
}

fn at(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .unwrap()
        .with_timezone(&Utc)
}

#[test]
fn the_wait_doubles_from_thirty_seconds_up_to_an_hour_and_varies_by_a_fifth() {
    let secs = |failures, jitter| retry_after(failures, jitter).num_seconds();
    assert_eq!(
        [1, 2, 3, 4, 7, 8, 30].map(|f| secs(f, 0.0)),
        [30, 60, 120, 240, 1920, 3600, 3600]
    );
    assert_eq!((secs(1, 1.0), secs(1, -1.0)), (36, 24));
    assert_eq!((secs(8, 1.0), secs(8, -1.0)), (3600, 2880));
}

#[tokio::test]
async fn a_pass_posts_the_queue_with_the_token_and_a_2xx_empties_it() {
    let (h, transport, sender) = joined(audited()).await;
    resolve(&h, 2).await;
    assert_eq!(sender.pass().await, 3);

    let posts = transport.posts();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].endpoint, ENDPOINT);
    assert_eq!(posts[0].token, AUDIT_TOKEN);
    assert!(!posts[0].body.contains(AUDIT_TOKEN));
    assert!(posts[0].body.ends_with('\n'));
    assert_eq!(posts[0].body.lines().count(), 3);
    let report = verify_lines(&posts[0].body);
    assert!(report.findings.is_empty());
    assert_eq!(
        (report.chains[0].first_seq, report.chains[0].last_seq),
        (1, 3)
    );

    assert_eq!(h.store.audit_queued(ACME).await.unwrap(), 0);
    let states: Vec<HistoryState> = h
        .store
        .audit_history(ACME, 100)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.state)
        .collect();
    assert_eq!(states, vec![HistoryState::Sent, HistoryState::Sent]);
    let state = sender.state(ACME);
    assert_eq!(state.last_sent_at.as_deref(), Some("2026-10-22T09:00:00Z"));
    assert_eq!(state.last_error, None);
    // Nothing is left to send.
    assert_eq!(sender.pass().await, 0);
    assert_eq!(transport.posts().len(), 1);
}

#[tokio::test]
async fn more_than_five_hundred_records_go_five_hundred_at_a_time() {
    let (h, transport, sender) = joined(audited()).await;
    resolve(&h, 600).await;
    assert_eq!(sender.pass().await, 601);
    let sizes: Vec<usize> = transport
        .posts()
        .iter()
        .map(|p| p.body.lines().count())
        .collect();
    assert_eq!(sizes, vec![500, 101]);
    let all: String = transport.posts().iter().map(|p| p.body.clone()).collect();
    assert!(verify_lines(&all).findings.is_empty());
}

#[tokio::test]
async fn a_failed_send_keeps_the_batch_and_waits_before_the_next_try() {
    let (h, transport, sender) = joined(audited()).await;
    transport.answer(Err("the endpoint answered 503".into()));
    transport.answer(Err("the endpoint answered 503".into()));
    assert_eq!(sender.pass().await, 0);
    assert_eq!(h.store.audit_queued(ACME).await.unwrap(), 1);
    let state = sender.state(ACME);
    assert_eq!(
        state.last_error.as_deref(),
        Some("the endpoint answered 503")
    );
    assert_eq!(state.failed_at.as_deref(), Some("2026-10-22T09:00:00Z"));
    let next = at(state.next_try_at.as_deref().unwrap());
    let wait = next - h.now();
    assert!(
        wait >= Duration::seconds(24) && wait <= Duration::seconds(36),
        "{wait}"
    );

    // Not before the wait ends, even with a new record.
    resolve(&h, 1).await;
    assert_eq!(sender.pass().await, 0);
    assert_eq!(transport.posts().len(), 1);

    // The second failure waits twice as long.
    // `next_try_at` is to the second: one more passes the deadline.
    h.clock.set(next + Duration::seconds(1));
    assert_eq!(sender.pass().await, 0);
    let second = at(sender.state(ACME).next_try_at.as_deref().unwrap()) - h.now();
    assert!(
        second >= Duration::seconds(48) && second <= Duration::seconds(72),
        "{second}"
    );

    // A 2xx sends both and sets the wait back.
    h.advance(Duration::seconds(72));
    assert_eq!(sender.pass().await, 2);
    let state = sender.state(ACME);
    assert_eq!((state.last_error, state.next_try_at), (None, None));
    assert_eq!(h.store.audit_queued(ACME).await.unwrap(), 0);
}

#[tokio::test]
async fn without_audit_or_without_the_token_nothing_is_posted() {
    let (h, transport, sender) = joined(json!({})).await;
    assert_eq!(sender.pass().await, 0);
    assert_eq!(transport.posts().len(), 0);
    assert_eq!(h.store.audit_queued(ACME).await.unwrap(), 0);

    let h = OrgsHarness::new().await;
    h.join_acme(json!({"API_KEY": "x-value-77a1"}), audited())
        .await;
    let transport = Arc::new(FakeTransport::default());
    let sender = AuditSender::new(h.orgs.clone(), transport.clone());
    assert_eq!(sender.pass().await, 0);
    assert_eq!(transport.posts().len(), 0);
    assert_eq!(h.store.audit_queued(ACME).await.unwrap(), 1);
    assert_eq!(
        sender.state(ACME).last_error.as_deref(),
        Some("the audit token DOTS_AUDIT_TOKEN is not available: the account has no such entry")
    );
}

#[tokio::test]
async fn the_loop_sends_a_new_record_five_seconds_after_it_is_written() {
    let (h, transport, sender) = joined(audited()).await;
    let shutdown = CancellationToken::new();
    let task = sender.clone().spawn(shutdown.clone());
    // The first pass sends the join's record at once.
    let waited = |n: usize| {
        let transport = transport.clone();
        async move {
            for _ in 0..500 {
                if transport.posts().len() >= n {
                    return;
                }
                tokio::time::sleep(StdDuration::from_millis(10)).await;
            }
            panic!("no post {n}");
        }
    };
    waited(1).await;
    resolve(&h, 1).await;
    let deadline = h.clock.nth_sleep(1).await;
    assert_eq!(deadline, h.now() + Duration::seconds(5));
    assert_eq!(transport.posts().len(), 1);
    h.clock.set(deadline);
    waited(2).await;
    assert_eq!(transport.posts()[1].body.lines().count(), 1);
    shutdown.cancel();
    tokio::time::timeout(StdDuration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
}
```

**In `crates/dots-core/tests/vault_org_policy_test.rs`, replace:**

```rust
            invalid(&audit(bad, "DOTS_AUDIT_TOKEN")),
            format!("audit.endpoint '{bad}' must be an https:// URL")
        );
```

**with:**

```rust
            invalid(&audit(bad, "DOTS_AUDIT_TOKEN")),
            format!(
                "audit.endpoint '{bad}' must be an https:// URL (http:// only for localhost or \
                 127.0.0.1)"
            )
        );
    }
    // A sink on this PC may take plain http.
    for local in ["http://127.0.0.1:8080/dots", "http://localhost/audit"] {
        assert!(
            parse_policy(&audit(local, "DOTS_AUDIT_TOKEN")).is_ok(),
            "{local}"
        );
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core -j 2 --test vault_audit_send_test --test vault_org_policy_test -- --test-threads 4`
Expected: fails to compile: `unresolved import dots_core::vault::audit::sender`. Without that binary, `vault_org_policy_test::the_audit_target_needs_an_https_endpoint_and_an_entry_name` FAILS on the new error text.

- [ ] **Step 3: Implement**

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
    pub usage_sender: Arc<dyn crate::usage_counts::CountsSender>,
}
```

**with:**

```rust
    pub usage_sender: Arc<dyn crate::usage_counts::CountsSender>,
    /// What posts audit records to each account's endpoint (env vault spec §10). `Config::new`
    /// posts over HTTP, as the app does; an account whose policy has no `audit` sends nothing.
    pub audit_transport: Arc<dyn crate::vault::audit::AuditTransport>,
}
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
            usage_sender: Arc::new(crate::usage_counts::HttpSender::default()),
        }
```

**with:**

```rust
            usage_sender: Arc::new(crate::usage_counts::HttpSender::default()),
            audit_transport: Arc::new(crate::vault::audit::HttpTransport::default()),
        }
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
    usage_counts: Arc<crate::usage_counts::UsageCounts>,
    shutdown: CancellationToken,
```

**with:**

```rust
    usage_counts: Arc<crate::usage_counts::UsageCounts>,
    /// Sends each account's audit records (env vault spec §10).
    audit_sender: Arc<crate::vault::audit::AuditSender>,
    shutdown: CancellationToken,
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
        usage_counts.clone().spawn(shutdown.clone());
        // The runs apps start are queued as the runner's, and apps hear when a run of a topic
```

**with:**

```rust
        usage_counts.clone().spawn(shutdown.clone());
        // What each account queued goes to its audit endpoint, now and as it is written.
        let audit_sender =
            crate::vault::audit::AuditSender::new(orgs.clone(), cfg.audit_transport.clone());
        audit_sender.clone().spawn(shutdown.clone());
        // The runs apps start are queued as the runner's, and apps hear when a run of a topic
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
            usage_counts,
            shutdown,
```

**with:**

```rust
            usage_counts,
            audit_sender,
            shutdown,
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
        &self.usage_counts
    }
```

**with:**

```rust
        &self.usage_counts
    }

    /// The audit sender: how sending stands for each account, and a pass on demand.
    pub fn audit_sender(&self) -> &Arc<crate::vault::audit::AuditSender> {
        &self.audit_sender
    }
```

**In `crates/dots-core/src/vault/audit/mod.rs`, replace:**

```rust
pub mod record;
pub mod verify;
```

**with:**

```rust
pub mod record;
pub mod sender;
pub mod verify;
```

**In `crates/dots-core/src/vault/audit/mod.rs`, replace:**

```rust
};
pub use verify::{verify_lines, ChainReport, ChainSummary, Finding};
```

**with:**

```rust
};
pub use sender::{AuditSender, AuditTransport, HttpTransport, SendState};
pub use verify::{verify_lines, ChainReport, ChainSummary, Finding};
```

**Create `crates/dots-core/src/vault/audit/sender.rs`:**

```rust
//! Sending audit records (env vault spec §10, `docs/vault-audit-format.md`): each org's queued
//! lines go to the endpoint its policy names, at most `BATCH` in one `POST`, with the value of
//! its `token_entry` as the bearer token. A 2xx takes them off the queue; anything else leaves
//! them and waits before that org's next try, from `FIRST_RETRY` doubling up to `MAX_RETRY`.
//! New records go about `SEND_AFTER` after they are written. Neither the token nor an answer's
//! body is ever logged or shown.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::record::stamp;
use crate::vault::org::{OrgLookup, Orgs};

/// The most records in one request.
pub const BATCH: u64 = 500;
/// How long the sender waits after a record is written, so records that follow go with it.
pub const SEND_AFTER: Duration = Duration::seconds(5);
/// The wait after a first failure; each failure after it doubles the wait.
pub const FIRST_RETRY: Duration = Duration::seconds(30);
/// The longest wait between tries.
pub const MAX_RETRY: Duration = Duration::hours(1);
/// How far each wait is varied, either way.
pub const JITTER: f64 = 0.2;

/// The wait before the next try after `failures` failures in a row (1 after the first), spread
/// by `jitter`, from -1 to 1, by up to `JITTER` of it either way, and never over `MAX_RETRY`.
pub fn retry_after(failures: u32, jitter: f64) -> Duration {
    let doublings = failures.saturating_sub(1).min(20);
    let base = FIRST_RETRY.num_milliseconds() as f64 * f64::from(1u32 << doublings);
    let max = MAX_RETRY.num_milliseconds() as f64;
    let spread = base.min(max) * (1.0 + JITTER * jitter.clamp(-1.0, 1.0));
    Duration::milliseconds(spread.min(max).round() as i64)
}

/// A number from -1 to 1, from the random bits of a new ULID.
fn jitter() -> f64 {
    let bits = (ulid::Ulid::new().random() % 2_001) as f64;
    bits / 1_000.0 - 1.0
}

/// Posts a batch. Tests use a fake one.
#[async_trait]
pub trait AuditTransport: Send + Sync + std::fmt::Debug {
    /// Posts `body` to `endpoint` with `token` as the bearer token. `Err` is a short reason
    /// for the person: never the token, never the answer's body.
    async fn post(&self, endpoint: &str, token: &str, body: String) -> Result<(), String>;
}

/// Posts over HTTP with the vault's client: 20 seconds, no redirect (a redirect would take the
/// token to another host), the public and the Windows roots.
#[derive(Debug, Clone)]
pub struct HttpTransport {
    client: reqwest::Client,
}

impl HttpTransport {
    pub fn new(client: reqwest::Client) -> HttpTransport {
        HttpTransport { client }
    }
}

impl Default for HttpTransport {
    fn default() -> Self {
        HttpTransport::new(crate::vault::org::http_client())
    }
}

#[async_trait]
impl AuditTransport for HttpTransport {
    async fn post(&self, endpoint: &str, token: &str, body: String) -> Result<(), String> {
        let answer = self
            .client
            .post(endpoint)
            .header(reqwest::header::CONTENT_TYPE, "application/x-ndjson")
            .bearer_auth(token)
            .body(body)
            .send()
            .await;
        match answer {
            Ok(r) if r.status().is_success() => Ok(()),
            Ok(r) => Err(format!("the endpoint answered {}", r.status().as_u16())),
            Err(e) if e.is_timeout() => Err("no answer within 20 seconds".to_string()),
            Err(e) if e.is_connect() => Err("could not reach the endpoint".to_string()),
            Err(_) => Err("the send did not go through".to_string()),
        }
    }
}

/// How sending stands for one org, as its account page shows it. Times are RFC 3339.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SendState {
    pub last_sent_at: Option<String>,
    /// Why the last try failed; `None` once one works.
    pub last_error: Option<String>,
    pub failed_at: Option<String>,
    /// When the next try is due after a failure.
    pub next_try_at: Option<String>,
}

#[derive(Default)]
struct OrgState {
    shown: SendState,
    failures: u32,
    /// Set after a failure to send: no try before it.
    next_try: Option<DateTime<Utc>>,
}

/// The sender. One runs for the app, from `spawn`.
pub struct AuditSender {
    orgs: Orgs,
    transport: Arc<dyn AuditTransport>,
    states: Mutex<HashMap<String, OrgState>>,
    /// One pass at a time.
    one: tokio::sync::Mutex<()>,
}

impl std::fmt::Debug for AuditSender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuditSender").finish_non_exhaustive()
    }
}

impl AuditSender {
    pub fn new(orgs: Orgs, transport: Arc<dyn AuditTransport>) -> Arc<AuditSender> {
        Arc::new(AuditSender {
            orgs,
            transport,
            states: Mutex::default(),
            one: tokio::sync::Mutex::new(()),
        })
    }

    /// How sending stands for `org_id`.
    pub fn state(&self, org_id: &str) -> SendState {
        self.states()
            .get(org_id)
            .map(|s| s.shown.clone())
            .unwrap_or_default()
    }

    /// Sends what every org has queued, except an org whose next try is not due yet, and says
    /// how many records went. An org whose policy has no `audit` (or that is not joined) keeps
    /// its queue: that is no failure.
    pub async fn pass(&self) -> usize {
        let _one = self.one.lock().await;
        let waiting = match self.orgs.audit().store().audit_waiting().await {
            Ok(waiting) => waiting,
            Err(e) => {
                tracing::warn!("the audit queue could not be read: {e}");
                return 0;
            }
        };
        let now = self.orgs.audit().clock().now();
        let mut sent = 0;
        for (org_id, _) in waiting {
            let due = self
                .states()
                .get(&org_id)
                .and_then(|s| s.next_try)
                .is_none_or(|next| now >= next);
            if due {
                sent += self.send_org(&org_id).await;
            }
        }
        sent
    }

    /// Sends the org's queue, a batch at a time, until it is empty or a send fails.
    async fn send_org(&self, org_id: &str) -> usize {
        let Some(target) = self.orgs.policy(org_id).and_then(|p| p.audit) else {
            return 0;
        };
        let token = match self.orgs.lookup(org_id, &target.token_entry).await {
            OrgLookup::Value { value, .. } => value,
            other => {
                // No token while the account gives no values: the next record written (a
                // fetch that works) tries again, so no wait is set.
                let why = format!(
                    "the audit token {} is not available: {}",
                    target.token_entry,
                    no_value(&other)
                );
                self.update(org_id, |s, now| {
                    s.shown.last_error = Some(why);
                    s.shown.failed_at = Some(stamp(now));
                    s.shown.next_try_at = None;
                    s.next_try = None;
                });
                return 0;
            }
        };
        let store = self.orgs.audit().store();
        let mut sent = 0;
        loop {
            let batch = match store.audit_batch(org_id, BATCH).await {
                Ok(batch) => batch,
                Err(e) => {
                    tracing::warn!("org {org_id:?}: its audit queue could not be read: {e}");
                    return sent;
                }
            };
            let Some(&(through, _)) = batch.last() else {
                return sent;
            };
            let body: String = batch.iter().map(|(_, line)| format!("{line}\n")).collect();
            match self.transport.post(&target.endpoint, &token, body).await {
                Ok(()) => {
                    if let Err(e) = store.audit_sent(org_id, through).await {
                        tracing::warn!("org {org_id:?}: sent audit records stay queued: {e}");
                        return sent;
                    }
                    sent += batch.len();
                    self.update(org_id, |s, now| {
                        *s = OrgState {
                            shown: SendState {
                                last_sent_at: Some(stamp(now)),
                                ..SendState::default()
                            },
                            ..OrgState::default()
                        };
                    });
                    if (batch.len() as u64) < BATCH {
                        return sent;
                    }
                }
                Err(why) => {
                    tracing::warn!("org {org_id:?}: audit records were not sent: {why}");
                    self.update(org_id, |s, now| {
                        s.failures += 1;
                        let next = now + retry_after(s.failures, jitter());
                        s.shown.last_error = Some(why);
                        s.shown.failed_at = Some(stamp(now));
                        s.shown.next_try_at = Some(stamp(next));
                        s.next_try = Some(next);
                    });
                    return sent;
                }
            }
        }
    }

    /// The soonest try due after a failure, if any.
    fn next_due(&self) -> Option<DateTime<Utc>> {
        self.states().values().filter_map(|s| s.next_try).min()
    }

    /// Sends what is queued now, then again `SEND_AFTER` after each new record and when a
    /// failed org's wait ends, until `shutdown`.
    pub fn spawn(self: Arc<Self>, shutdown: CancellationToken) -> JoinHandle<()> {
        tokio::spawn(async move {
            let audit = self.orgs.audit().clone();
            let clock = audit.clock().clone();
            loop {
                let written = audit.written().notified();
                tokio::pin!(written);
                written.as_mut().enable();
                self.pass().await;
                let next = self.next_due();
                let retry = async {
                    match next {
                        Some(next) => clock.sleep_until(next).await,
                        None => std::future::pending().await,
                    }
                };
                tokio::select! {
                    _ = shutdown.cancelled() => return,
                    _ = retry => {}
                    _ = written => {
                        tokio::select! {
                            _ = shutdown.cancelled() => return,
                            _ = clock.sleep_until(clock.now() + SEND_AFTER) => {}
                        }
                    }
                }
            }
        })
    }

    fn states(&self) -> std::sync::MutexGuard<'_, HashMap<String, OrgState>> {
        self.states.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn update(&self, org_id: &str, change: impl FnOnce(&mut OrgState, DateTime<Utc>)) {
        let now = self.orgs.audit().clock().now();
        change(self.states().entry(org_id.to_string()).or_default(), now);
    }
}

/// Why an org gives no value for the token entry, in plain words.
fn no_value(lookup: &OrgLookup) -> &'static str {
    match lookup {
        OrgLookup::Value { .. } => "it has a value",
        OrgLookup::NotDefined => "the account has no such entry",
        OrgLookup::Expired => "the saved copy expired",
        OrgLookup::Revoked => "access was revoked",
        OrgLookup::PolicyInvalid(_) => "the policy is invalid",
        OrgLookup::Offline => "offline, with no saved copy",
        OrgLookup::Removed => "the account is not joined",
        OrgLookup::NotMember(_) => "this PC is not on the member list",
    }
}
```

**In `crates/dots-core/src/vault/org/policy.rs`, replace:**

```rust

fn check_audit(audit: &AuditTarget) -> Result<(), PolicyError> {
```

**with:**

```rust

/// The audit endpoint is `https://`, or `http://` only to `localhost` or `127.0.0.1`, as a
/// join code's address: a sink on the same PC, for a test.
fn check_audit(audit: &AuditTarget) -> Result<(), PolicyError> {
```

**In `crates/dots-core/src/vault/org/policy.rs`, replace:**

```rust
    let url = reqwest::Url::parse(&audit.endpoint);
    let https = url
        .as_ref()
        .is_ok_and(|u| u.scheme() == "https" && u.host_str().is_some_and(|h| !h.is_empty()));
    if !https {
        return Err(invalid(format!(
            "audit.endpoint '{}' must be an https:// URL",
            audit.endpoint
```

**with:**

```rust
    let url = reqwest::Url::parse(&audit.endpoint);
    let allowed = url
        .as_ref()
        .is_ok_and(|u| match (u.scheme(), u.host_str()) {
            ("https", Some(host)) => !host.is_empty(),
            ("http", Some(host)) => matches!(host, "localhost" | "127.0.0.1"),
            _ => false,
        });
    if !allowed {
        return Err(invalid(format!(
            "audit.endpoint '{}' must be an https:// URL (http:// only for localhost or \
             127.0.0.1)",
            audit.endpoint
```

- [ ] **Step 4: Run them to see them pass**

Run: `cargo test -p dots-core -j 2 --test vault_audit_send_test --test vault_org_policy_test --test runtime_org_test -- --test-threads 4`
Expected: `test result: ok. 6 passed` (`vault_audit_send_test`), `vault_org_policy_test` and `runtime_org_test` all passed.

- [ ] **Step 5: Check and commit**

Run `cargo fmt --check` and `cargo clippy --workspace --all-targets -j 2 -- -D warnings`, then:

```bash
git add crates/dots-core/src/runtime.rs crates/dots-core/src/vault/audit/mod.rs crates/dots-core/src/vault/org/policy.rs crates/dots-core/tests/vault_org_policy_test.rs crates/dots-core/src/vault/audit/sender.rs crates/dots-core/tests/vault_audit_send_test.rs
git commit -m "feat(vault): audit records are posted to the account's endpoint in batches of 500, with backoff after a failure"
```

---

### Task 6: dots-admin audit verify checks seq and the prev chain per account and install, and names each gap

`dots-admin audit verify <records.ndjson>` prints each chain (`<org> (<install>): <n> records, seq <a> to <b>`), the repeated lines, and each finding's text, from the core's `verify_lines`; it fails (exit 1) when a finding is a failure, or when the file holds no records; a wrong command line exits 2.

**Files:**
- Create: `crates/dots-admin/src/audit.rs`
- Modify: `crates/dots-admin/src/lib.rs`
- Test: `crates/dots-admin/tests/admin_test.rs`

**Interfaces:**
- Consumes: Task 1's `verify_lines`, `ChainReport`, `Finding::is_failure`, `Finding::text`; `dots_admin::{read_file, say, failed, usage}`.
- Produces: the command `audit verify` (`audit::audit_command(&[String], &mut Io)`).

- [ ] **Step 1: Write the failing tests**

**In `crates/dots-admin/tests/admin_test.rs`, replace:**

```rust
    ]);
    assert!(err.to_string().contains("default"), "{err}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), before);
}
```

**with:**

```rust
    ]);
    assert!(err.to_string().contains("default"), "{err}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), before);
}

// audit verify (env vault spec §12)

/// `n` chained records of acme-eng on one install, as the endpoint stored them.
fn audit_lines(n: u64) -> Vec<String> {
    use dots_core::vault::audit::{line_hash, AuditRecord, AuditUser, Event};
    let mut lines: Vec<String> = Vec::new();
    for seq in 1..=n {
        let record = AuditRecord {
            v: 1,
            id: format!("01J9REC{seq:04}"),
            seq,
            prev: lines.last().map(|l| line_hash(l)),
            at: "2026-10-21T09:12:03Z".into(),
            org_id: "acme-eng".into(),
            install: "01J9INSTALL".into(),
            user: AuditUser::default(),
            dot: None,
            run_id: None,
            event: Event::Fetched,
            name: None,
            consumer: None,
            source: Some("openbao".into()),
            reason: None,
            dropped: None,
        };
        lines.push(record.line());
    }
    lines
}

fn records_file(dir: &Path, lines: &[String]) -> String {
    let file = path(dir, "records.ndjson");
    let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
    std::fs::write(&file, text).unwrap();
    file
}

#[test]
fn audit_verify_prints_each_chain_and_ok_for_a_whole_file() {
    let dir = TempDir::new().unwrap();
    let file = records_file(dir.path(), &audit_lines(3));
    let ran = ok(&["audit", "verify", &file]);
    assert_eq!(
        ran.out,
        "acme-eng (01J9INSTALL): 3 records, seq 1 to 3\nok: every chain is whole\n"
    );
}

#[test]
fn audit_verify_fails_on_a_gap_and_names_it() {
    let dir = TempDir::new().unwrap();
    let mut lines = audit_lines(5);
    lines.remove(2);
    let file = records_file(dir.path(), &lines);
    let ran = run_with(&["audit", "verify", &file], None);
    assert_eq!(
        ran.out,
        "acme-eng (01J9INSTALL): 4 records, seq 1 to 5\n\
         acme-eng (01J9INSTALL): record 3 is missing before line 3\n"
    );
    let err = ran.result.unwrap_err();
    assert_eq!(err.to_string(), "the records are not whole: 1 problem");
    assert_eq!(err.exit_code(), 1);
}

#[test]
fn audit_verify_counts_a_batch_sent_twice_and_passes() {
    let dir = TempDir::new().unwrap();
    let mut lines = audit_lines(3);
    lines.extend(audit_lines(3)[1..].iter().cloned());
    let file = records_file(dir.path(), &lines);
    let ran = ok(&["audit", "verify", &file]);
    assert!(ran
        .out
        .contains("2 lines repeat a record already read (a batch sent twice)"));
    assert!(ran.out.ends_with("ok: every chain is whole\n"));
}

#[test]
fn audit_verify_refuses_an_empty_file_and_a_wrong_command_line() {
    let dir = TempDir::new().unwrap();
    let file = records_file(dir.path(), &[]);
    assert_eq!(
        failure(&["audit", "verify", &file]).to_string(),
        format!("{file} holds no records")
    );
    let err = failure(&["audit", "verify"]);
    assert_eq!(err.exit_code(), 2);
    assert_eq!(failure(&["audit", "check", &file]).exit_code(), 2);
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-admin -j 2 -- --test-threads 4`
Expected: the four `audit_verify_*` tests FAIL with `unknown command 'audit'`.

- [ ] **Step 3: Implement**

**Create `crates/dots-admin/src/audit.rs`:**

```rust
//! `dots-admin audit verify` (env vault spec §12): checks a file of audit records as a receiver
//! holds them, with `dots-core`'s own checker, so the admin and project 2 read the chain the way
//! Dots writes it (`docs/vault-audit-format.md`).

use dots_core::vault::audit::verify_lines;

use crate::{failed, read_file, say, usage, Io, Result};

/// `audit verify <records.ndjson>`.
pub(crate) fn audit_command(args: &[String], io: &mut Io<'_>) -> Result<()> {
    match args {
        [sub, file] if sub == "verify" => verify_command(file, io),
        [sub, ..] if sub == "verify" => Err(usage("audit verify takes one file of records")),
        [other, ..] => Err(usage(format!("unknown audit command '{other}'"))),
        [] => Err(usage("audit needs a command: verify")),
    }
}

/// Prints each chain (an org on one install) with its records and seq range, then every
/// finding. A gap the install's own `expired` record explains is printed but is no failure.
/// It fails when the file holds no record, or when any chain is not whole.
fn verify_command(file: &str, io: &mut Io<'_>) -> Result<()> {
    let text = read_file(file, "the records file")?;
    let report = verify_lines(&text);
    if report.chains.is_empty() && report.findings.is_empty() {
        return Err(failed(format!("{file} holds no records")));
    }
    for chain in &report.chains {
        say(
            io.out,
            &format!(
                "{} ({}): {} records, seq {} to {}",
                chain.org_id, chain.install, chain.records, chain.first_seq, chain.last_seq
            ),
        )?;
    }
    if report.repeated > 0 {
        say(
            io.out,
            &format!(
                "{} lines repeat a record already read (a batch sent twice); they are not \
                 counted",
                report.repeated
            ),
        )?;
    }
    for finding in &report.findings {
        say(io.out, &finding.text())?;
    }
    let problems = report.findings.iter().filter(|f| f.is_failure()).count();
    if problems > 0 {
        let s = if problems == 1 { "" } else { "s" };
        return Err(failed(format!(
            "the records are not whole: {problems} problem{s}"
        )));
    }
    say(io.out, "ok: every chain is whole")
}
```

**In `crates/dots-admin/src/lib.rs`, replace:**

```rust
//! `dots-admin`, the org admin's helper (env vault spec §12): the policy signing key, the policy
//! itself and the join codes people paste into Dots. It uses `dots-core`'s policy and join code
//! types and checks, so what it writes is what Dots reads.
//!
```

**with:**

```rust
//! `dots-admin`, the org admin's helper (env vault spec §12): the policy signing key, the policy
//! itself, the join codes people paste into Dots, and the check of a file of audit records. It
//! uses `dots-core`'s policy, join code and audit record types and checks, so what it writes is
//! what Dots reads.
//!
```

**In `crates/dots-admin/src/lib.rs`, replace:**

```rust
mod args;
pub mod join;
```

**with:**

```rust
mod args;
mod audit;
pub mod join;
```

**In `crates/dots-admin/src/lib.rs`, replace:**

```rust
      sops:      --location <full path or https:// url>

```

**with:**

```rust
      sops:      --location <full path or https:// url>
  dots-admin audit verify <records.ndjson>
      Checks audit records as an endpoint received them: for each account and install, seq
      counts up by one and each prev is the hash of the record before. Reports each gap.

```

**In `crates/dots-admin/src/lib.rs`, replace:**

```rust
        "join-code" => join::join_code_command(rest, io),
        other => Err(usage(format!("unknown command '{other}'"))),
```

**with:**

```rust
        "join-code" => join::join_code_command(rest, io),
        "audit" => audit::audit_command(rest, io),
        other => Err(usage(format!("unknown command '{other}'"))),
```

- [ ] **Step 4: Run them to see them pass**

Run: `cargo test -p dots-admin -j 2 -- --test-threads 4`
Expected: `test result: ok. 39 passed` (`admin_test`).

- [ ] **Step 5: Check and commit**

Run `cargo fmt --check` and `cargo clippy -p dots-admin --all-targets -j 2 -- -D warnings`, then:

```bash
git add crates/dots-admin/src/audit.rs crates/dots-admin/src/lib.rs crates/dots-admin/tests/admin_test.rs
git commit -m "feat(dots-admin): audit verify checks seq and the prev chain per account and install, and names each gap"
```

---

### Task 7: A run's audit records reach a local HTTP sink as a whole chain with the bearer token

The end-to-end test the spec asks for: a runtime on the fake clock joins an account (fake OpenBao) whose policy names an `http://127.0.0.1:<port>/dots` endpoint, runs a dot through the fake CLI, and its own `HttpTransport` posts to an axum sink in the test. The sink first answers 503 (the records stay queued, the state says why), then 200 after the wait. What arrives verifies as one chain, holds no value, and carries `Content-Type: application/x-ndjson` and the bearer token.

**Files:**
- Test: `crates/dots-core/tests/runtime_audit_test.rs`

**Interfaces:**
- Consumes: Tasks 1 to 5; `Runtime::audit_sender`, `Runtime::create_dot`, `Runtime::run_now`.
- Produces: nothing new.

- [ ] **Step 1: Write the test**

**Create `crates/dots-core/tests/runtime_audit_test.rs`:**

```rust
//! Audit records end to end (env vault spec §10, §13): a runtime joins an account whose policy
//! names an audit endpoint on this PC, runs a dot of it through the fake claude CLI, and its
//! sender posts the records over HTTP to a local sink. The sink gets NDJSON with the token
//! entry's value as the bearer, a refused post is kept and sent again, and what arrives is a
//! whole chain that never holds a value. Nothing leaves 127.0.0.1.

mod common;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use chrono::Duration;
use common::clock::FakeClock;
use common::fake_openbao::FakeOpenBao;
use common::orgs::{at, code, keys, openbao, policy, sign, token, ACME, ACME_NAME, START, TOKEN};
use common::{spec, wait_status};
use dots_core::model::{RunStatus, WorkspaceMode};
use dots_core::vault::audit::{verify_lines, AuditDot, AuditRecord, Event};
use dots_core::vault::{Grants, MemorySecrets};
use dots_core::{Config, Runtime};
use serde_json::json;

const LEVEL: &str = "org-plain-level-audit-8c2f";
const AUDIT_TOKEN: &str = "audit-bearer-token-3a91e7";

#[derive(Debug, Clone)]
struct Got {
    content_type: Option<String>,
    authorization: Option<String>,
    body: String,
    answered: u16,
}

/// A receiver on 127.0.0.1 that keeps every post and answers with `status`.
#[derive(Clone)]
struct Sink {
    got: Arc<Mutex<Vec<Got>>>,
    status: Arc<Mutex<u16>>,
}

impl Sink {
    async fn start() -> (String, Sink) {
        let sink = Sink {
            got: Arc::default(),
            status: Arc::new(Mutex::new(200)),
        };
        let app = axum::Router::new()
            .route("/dots", axum::routing::post(receive))
            .with_state(sink.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://127.0.0.1:{port}/dots"), sink)
    }

    fn answer(&self, status: u16) {
        *self.status.lock().unwrap() = status;
    }

    fn got(&self) -> Vec<Got> {
        self.got.lock().unwrap().clone()
    }
}

async fn receive(State(sink): State<Sink>, headers: HeaderMap, body: String) -> StatusCode {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    let answered = *sink.status.lock().unwrap();
    sink.got.lock().unwrap().push(Got {
        content_type: header("content-type"),
        authorization: header("authorization"),
        body,
        answered,
    });
    StatusCode::from_u16(answered).unwrap()
}

#[tokio::test]
async fn a_runs_records_reach_a_local_sink_as_a_whole_chain_with_the_bearer_token() {
    let (endpoint, sink) = Sink::start().await;
    // The sink refuses at first, whichever pass reaches it: the runtime's own loop or ours.
    sink.answer(503);
    let bao = FakeOpenBao::start().await;
    bao.allow_token(TOKEN, false);
    let dir = tempfile::tempdir().unwrap();
    let clock = FakeClock::new(at(START));
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.clock = clock.clone();
    cfg.secrets = Some(Arc::new(MemorySecrets::new()));
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    let args_out = dir.path().join("args.json");
    cfg.claude_env.push((
        "FAKE_CLAUDE_ARGS_OUT".into(),
        args_out.to_string_lossy().to_string(),
    ));
    let rt = Runtime::start(cfg).await.unwrap();

    let keys = keys();
    let extra = json!({
        "plain": ["DOTS_TEST_LEVEL"],
        "offline_hours": 0,
        "audit": {"endpoint": endpoint, "token_entry": "DOTS_AUDIT_TOKEN"},
    });
    let signed = sign(&keys, &policy(ACME, "2026-10-22T08:00:00Z", extra));
    bao.put_env(
        "secret",
        "dots",
        json!({"DOTS_TEST_LEVEL": LEVEL, "DOTS_AUDIT_TOKEN": AUDIT_TOKEN}),
    );
    bao.put_policy("secret", "dots", &signed);
    let join = code(ACME, ACME_NAME, openbao(&bao.address(), "dots"), &keys);
    let orgs = rt.vault().orgs().expect("the runtime's orgs");
    orgs.join(&join, Some(token())).await.unwrap();

    let mut triage = spec("triage");
    triage.workdir = dir.path().to_string_lossy().to_string();
    triage.workspace_mode = WorkspaceMode::Folder;
    triage.variables = Grants([("shell".to_string(), vec!["DOTS_TEST_LEVEL".to_string()])].into());
    let dot = rt
        .create_dot(triage, Some(ACME.into()), None)
        .await
        .unwrap();
    let run = rt.run_now(&dot.id).await.unwrap();
    wait_status(rt.store(), &run.id, RunStatus::Succeeded, 15).await;

    // Refused: the records stay queued, and the account says why.
    let sender = rt.audit_sender();
    assert_eq!(sender.pass().await, 0);
    let queued = rt.store().audit_queued(ACME).await.unwrap();
    assert_eq!(queued, 2);
    assert_eq!(
        sender.state(ACME).last_error.as_deref(),
        Some("the endpoint answered 503")
    );

    // After the wait, the same records go again and are taken.
    sink.answer(200);
    clock.set(at(START) + Duration::minutes(2));
    sender.pass().await;
    for _ in 0..500 {
        if rt.store().audit_queued(ACME).await.unwrap() == 0 {
            break;
        }
        tokio::time::sleep(StdDuration::from_millis(10)).await;
    }
    assert_eq!(rt.store().audit_queued(ACME).await.unwrap(), 0);
    assert_eq!(sender.state(ACME).last_error, None);

    let got = sink.got();
    assert!(got.len() >= 2);
    for post in &got {
        assert_eq!(post.content_type.as_deref(), Some("application/x-ndjson"));
        assert_eq!(
            post.authorization.as_deref(),
            Some(format!("Bearer {AUDIT_TOKEN}").as_str())
        );
        assert!(!post.body.contains(LEVEL) && !post.body.contains(AUDIT_TOKEN));
        assert!(!post.body.contains(TOKEN));
    }
    let taken: String = got
        .iter()
        .filter(|p| p.answered == 200)
        .map(|p| p.body.clone())
        .collect();
    let report = verify_lines(&taken);
    assert!(report.findings.is_empty(), "{:?}", report.findings);
    let records: Vec<AuditRecord> = taken
        .lines()
        .map(|line| AuditRecord::parse(line).unwrap())
        .collect();
    assert_eq!(records.len(), 2);
    assert_eq!((records[0].seq, records[0].event), (1, Event::Fetched));
    let resolved = &records[1];
    assert_eq!((resolved.seq, resolved.event), (2, Event::Resolved));
    assert_eq!(resolved.name.as_deref(), Some("DOTS_TEST_LEVEL"));
    assert_eq!(resolved.consumer.as_deref(), Some("shell"));
    assert_eq!(
        resolved.dot,
        Some(AuditDot {
            id: dot.id.clone(),
            name: "triage".into()
        })
    );
    assert_eq!(resolved.run_id.as_deref(), Some(run.id.as_str()));
    assert_eq!(resolved.install, rt.store().install_id().await.unwrap());
    rt.shutdown();
}
```

- [ ] **Step 2: Run it**

Run: `cargo test -p dots-core -j 2 --test runtime_audit_test -- --test-threads 4`
Expected: `test result: ok. 1 passed`. (It passes on Tasks 1 to 5: it is the end-to-end check, not a new behaviour. If it fails, a task before it is wrong; fix that task, not the test.)

- [ ] **Step 3: Run it twice more**

Run the same command twice. Expected: passes both times (the runtime's own sender and the test's pass may race; the sink refuses from the start so either reaches a 503 first).

- [ ] **Step 4: Commit**

Run `cargo fmt --check`, then:

```bash
git add crates/dots-core/tests/runtime_audit_test.rs
git commit -m "test(vault): a run's audit records reach a local HTTP sink as a whole chain with the bearer token"
```

---

### Task 8: org_audit gives an account's History and how sending its audit records stands

`Runtime::org_audit(org_id) -> AuditView` (`NotFound` for an account not joined): the policy's endpoint, how many records wait, the sender's `SendState` and History (`history_rows` rows). The app's `org_audit` command returns it; the contract gains two `AuditView` samples and the enums `AuditEvent` and `HistoryState`; the UI gains the schemas, `commands.orgAudit` and `keys.orgAudit` (under the orgs' key).

**Files:**
- Create: `crates/dots-core/src/vault/audit/view.rs`
- Modify: `crates/dots-core/src/vault/audit/mod.rs`, `crates/dots-core/src/runtime.rs`, `apps/dots-app/src/{commands,contract,lib}.rs`, `ui/src/api/{types.ts,commands.ts,contract.test.ts,__fixtures__/contract.json}`, `ui/src/lib/query.ts`
- Test: `crates/dots-core/tests/runtime_audit_test.rs`, `ui/src/api/commands.test.ts`

**Interfaces:**
- Consumes: Task 2's `audit_queued`, `audit_history`, `HistoryRow`; Task 5's `AuditSender::state`, `SendState`.
- Produces: `vault::audit::AuditView { endpoint: Option<String>, queued: u64, send: SendState, history: Vec<HistoryRow> }`; `Runtime::org_audit(&str) -> Result<AuditView>`; app command `org_audit(orgId)`; UI `T.AuditEvent`, `T.HistoryState`, `T.HistoryRow`, `T.SendState`, `T.AuditView` (and types `AuditEvent`, `HistoryRow`, `AuditView`), `commands.orgAudit(orgId)`, `keys.orgAudit(orgId) = ["orgs", orgId, "audit"]`.

- [ ] **Step 1: Write the failing tests**

**In `crates/dots-core/tests/runtime_audit_test.rs`, replace:**

```rust
use dots_core::model::{RunStatus, WorkspaceMode};
use dots_core::vault::audit::{verify_lines, AuditDot, AuditRecord, Event};
```

**with:**

```rust
use dots_core::model::{RunStatus, WorkspaceMode};
use dots_core::store::HistoryState;
use dots_core::vault::audit::{verify_lines, AuditDot, AuditRecord, Event};
```

**In `crates/dots-core/tests/runtime_audit_test.rs`, replace:**

```rust
    assert_eq!(resolved.install, rt.store().install_id().await.unwrap());
    rt.shutdown();
```

**with:**

```rust
    assert_eq!(resolved.install, rt.store().install_id().await.unwrap());

    // The account's page: its endpoint, nothing waiting, and the run's record sent. The fetch
    // is not listed.
    let view = rt.org_audit(ACME).await.unwrap();
    assert_eq!(view.endpoint.as_deref(), Some(endpoint.as_str()));
    assert_eq!(view.queued, 0);
    assert!(view.send.last_sent_at.is_some());
    let rows: Vec<(Event, Option<String>, HistoryState)> = view
        .history
        .into_iter()
        .map(|row| (row.event, row.name, row.state))
        .collect();
    assert_eq!(
        rows,
        vec![(
            Event::Resolved,
            Some("DOTS_TEST_LEVEL".into()),
            HistoryState::Sent
        )]
    );
    assert!(matches!(
        rt.org_audit("beta").await,
        Err(dots_core::Error::NotFound(_))
    ));
    rt.shutdown();
```

**In `ui/src/api/commands.test.ts`, replace:**

```ts
    expect(await commands.orgEntries("acme-eng")).toEqual(fixture.OrgEntry);
    expect(calls).toEqual([
```

**with:**

```ts
    expect(await commands.orgEntries("acme-eng")).toEqual(fixture.OrgEntry);
    reply = async () => fixture.AuditView[0];
    expect(await commands.orgAudit("acme-eng")).toEqual(fixture.AuditView[0]);
    expect(calls).toEqual([
```

**In `ui/src/api/commands.test.ts`, replace:**

```ts
      ["org_entries", { orgId: "acme-eng" }],
    ]);
    // An answer holding a value or the sign-in details is a shape this UI refuses.
```

**with:**

```ts
      ["org_entries", { orgId: "acme-eng" }],
      ["org_audit", { orgId: "acme-eng" }],
    ]);
    // A History row holding a value is a shape this UI refuses.
    const row = { ...fixture.AuditView[0].history[0], value: "x" };
    reply = async () => ({ ...fixture.AuditView[0], history: [row] });
    expect((await failure(commands.orgAudit("acme-eng"))).kind).toBe("internal");
    // An answer holding a value or the sign-in details is a shape this UI refuses.
```

**In `ui/src/api/contract.test.ts`, replace:**

```ts
  OrgEntry: T.OrgEntry.array(),
  SourceCreds: T.SourceCreds.array(),
```

**with:**

```ts
  OrgEntry: T.OrgEntry.array(),
  AuditView: T.AuditView.array(),
  SourceCreds: T.SourceCreds.array(),
```

**In `ui/src/api/contract.test.ts`, replace:**

```ts
  GrantState: T.GrantState,
  ValueSource: T.ValueSource,
```

**with:**

```ts
  GrantState: T.GrantState,
  AuditEvent: T.AuditEvent,
  HistoryState: T.HistoryState,
  ValueSource: T.ValueSource,
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core -j 2 --test runtime_audit_test -- --test-threads 4`
Expected: fails to compile: no method `org_audit` on `Runtime`.

- [ ] **Step 3: Implement**

**In `apps/dots-app/src/commands.rs`, replace:**

```rust
use dots_core::usage_counts::{Report, UsageCountsState};
use dots_core::vault::org::{
```

**with:**

```rust
use dots_core::usage_counts::{Report, UsageCountsState};
use dots_core::vault::audit::AuditView;
use dots_core::vault::org::{
```

**In `apps/dots-app/src/commands.rs`, replace:**

```rust

/// Every message that waits for the person and the newest others, newest first; of one dot
```

**with:**

```rust

/// The account's History on this PC and how sending its audit records stands (env vault spec
/// section 10). Never a value.
#[tauri::command]
pub async fn org_audit(state: State<'_, AppState>, org_id: String) -> Res<AuditView> {
    Ok(state.rt()?.org_audit(&org_id).await?)
}

/// Every message that waits for the person and the newest others, newest first; of one dot
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
use dots_core::skill_creator::{DraftRequest, DraftSummary};
use dots_core::usage_counts::{Report, UsageCountsState};
use dots_core::vault::org::{
```

**with:**

```rust
use dots_core::skill_creator::{DraftRequest, DraftSummary};
use dots_core::store::{HistoryRow, HistoryState};
use dots_core::usage_counts::{Report, UsageCountsState};
use dots_core::vault::audit::{AuditView, Event as AuditEvent, SendState};
use dots_core::vault::org::{
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
    ]);
    samples["SourceCreds"] = source_creds();
```

**with:**

```rust
    ]);
    // What `org_audit` answers: an account whose last send failed, with a row of each state, and
    // one that sends no records.
    samples["AuditView"] = to([
        AuditView {
            endpoint: Some("https://audit.acme.example/dots".into()),
            queued: 2,
            send: SendState {
                last_sent_at: Some("2026-10-22T09:00:00Z".into()),
                last_error: Some("the endpoint answered 503".into()),
                failed_at: Some("2026-10-22T09:05:00Z".into()),
                next_try_at: Some("2026-10-22T09:05:30Z".into()),
            },
            history: vec![
                HistoryRow {
                    id: "01J0AUDIT0003".into(),
                    at: "2026-10-22T09:05:00Z".into(),
                    event: AuditEvent::Denied,
                    name: Some("PROD_TOKEN".into()),
                    consumer: Some("mcp:github".into()),
                    dot_name: Some("acme-release".into()),
                    reason: Some("denied_by_policy".into()),
                    state: HistoryState::Queued,
                },
                HistoryRow {
                    id: "01J0AUDIT0002".into(),
                    at: "2026-10-22T09:00:00Z".into(),
                    event: AuditEvent::Resolved,
                    name: Some("GITHUB_TOKEN".into()),
                    consumer: Some("mcp:github".into()),
                    dot_name: Some("acme-release".into()),
                    reason: None,
                    state: HistoryState::Sent,
                },
                HistoryRow {
                    id: "01J0AUDIT0001".into(),
                    at: "2026-10-21T18:00:00Z".into(),
                    event: AuditEvent::Expired,
                    name: None,
                    consumer: None,
                    dot_name: None,
                    reason: Some("queue_full".into()),
                    state: HistoryState::Dropped,
                },
            ],
        },
        AuditView {
            endpoint: None,
            queued: 0,
            send: SendState::default(),
            history: vec![HistoryRow {
                id: "01J0AUDIT0004".into(),
                at: "2026-10-22T09:00:00Z".into(),
                event: AuditEvent::Offline,
                name: None,
                consumer: None,
                dot_name: None,
                reason: None,
                state: HistoryState::Local,
            }],
        },
    ]);
    samples["SourceCreds"] = source_creds();
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
    samples["Enums"]["GrantState"] = to(GrantState::ALL);
    samples["Enums"]["ValueSource"] = to(ValueSource::ALL);
```

**with:**

```rust
    samples["Enums"]["GrantState"] = to(GrantState::ALL);
    samples["Enums"]["AuditEvent"] = to(AuditEvent::ALL);
    samples["Enums"]["HistoryState"] = to(HistoryState::ALL);
    samples["Enums"]["ValueSource"] = to(ValueSource::ALL);
```

**In `apps/dots-app/src/lib.rs`, replace:**

```rust
    org_entries,
    messages_list,
```

**with:**

```rust
    org_entries,
    org_audit,
    messages_list,
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
        &self.audit_sender
    }
```

**with:**

```rust
        &self.audit_sender
    }

    /// The joined account `org_id`'s History and how sending its records stands (env vault
    /// spec §10). Never a value.
    pub async fn org_audit(&self, org_id: &str) -> Result<crate::vault::audit::AuditView> {
        if !self.orgs.is_joined(org_id) {
            return Err(Error::NotFound(format!(
                "no org '{org_id}' is joined on this PC"
            )));
        }
        let limits = self.orgs.audit().limits();
        Ok(crate::vault::audit::AuditView {
            endpoint: self
                .orgs
                .policy(org_id)
                .and_then(|p| p.audit)
                .map(|a| a.endpoint),
            queued: self.store.audit_queued(org_id).await?,
            send: self.audit_sender.state(org_id),
            history: self
                .store
                .audit_history(org_id, limits.history_rows)
                .await?,
        })
    }
```

**In `crates/dots-core/src/vault/audit/mod.rs`, replace:**

```rust
pub mod verify;
pub mod writer;
```

**with:**

```rust
pub mod verify;
pub mod view;
pub mod writer;
```

**In `crates/dots-core/src/vault/audit/mod.rs`, replace:**

```rust
pub use verify::{verify_lines, ChainReport, ChainSummary, Finding};
pub use writer::{Audit, Happened, OrgAudit, Usage};
```

**with:**

```rust
pub use verify::{verify_lines, ChainReport, ChainSummary, Finding};
pub use view::AuditView;
pub use writer::{Audit, Happened, OrgAudit, Usage};
```

**Create `crates/dots-core/src/vault/audit/view.rs`:**

```rust
//! What an account's page shows of its audit records (env vault spec §10): the History list
//! and, on Overview, how sending stands. It never holds a value.

use serde::Serialize;

use super::sender::SendState;
use crate::store::HistoryRow;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuditView {
    /// The endpoint the account's policy names; `None` when it sends no records.
    pub endpoint: Option<String>,
    /// How many records wait to be sent.
    pub queued: u64,
    pub send: SendState,
    /// This PC's records for the account, newest first: 30 days, at most 1,000, and no
    /// `fetched`.
    pub history: Vec<HistoryRow>,
}
```

**In `ui/src/api/commands.ts`, replace:**

```ts
  orgEntries: (orgId: string) => call(T.OrgEntry.array(), "org_entries", { orgId }),

```

**with:**

```ts
  orgEntries: (orgId: string) => call(T.OrgEntry.array(), "org_entries", { orgId }),
  /// The org's History on this PC and how sending its audit records stands. Never a value.
  orgAudit: (orgId: string) => call(T.AuditView, "org_audit", { orgId }),

```

**In `ui/src/api/types.ts`, replace:**

```ts
});
// Whether a dot's grant resolves in its scope now, or why not.
```

**with:**

```ts
});
// What an audit record says happened (env vault spec §10): a variable given or refused, or what
// a fetch of the account's source came to.
export const AuditEvent = z.enum([
  "resolved",
  "denied",
  "fetched",
  "offline",
  "revoked",
  "expired",
  "policy_invalid",
]);
// A History row: kept on this PC only (`local`, the account sends no records), waiting to be
// sent, sent, or dropped unsent from a full queue.
export const HistoryState = z.enum(["local", "queued", "sent", "dropped"]);
// One row of an account's History. Never a value.
export const HistoryRow = z.strictObject({
  id: z.string(),
  at: z.string(),
  event: AuditEvent,
  name: nullable(z.string()),
  consumer: nullable(z.string()),
  dot_name: nullable(z.string()),
  // A code: why a variable was refused, or what an expiry or revocation was.
  reason: nullable(z.string()),
  state: HistoryState,
});
// How sending an account's records stands; times are RFC 3339.
export const SendState = z.strictObject({
  last_sent_at: nullable(z.string()),
  last_error: nullable(z.string()),
  failed_at: nullable(z.string()),
  next_try_at: nullable(z.string()),
});
// An account's History, newest first, and how sending stands. `endpoint` is null when the
// account sends no records.
export const AuditView = z.strictObject({
  endpoint: nullable(z.string()),
  queued: z.number().int().nonnegative(),
  send: SendState,
  history: z.array(HistoryRow),
});
// Whether a dot's grant resolves in its scope now, or why not.
```

**In `ui/src/api/types.ts`, replace:**

```ts
export type OrgEntry = z.infer<typeof OrgEntry>;
export type PersonalMode = z.infer<typeof PersonalMode>;
```

**with:**

```ts
export type OrgEntry = z.infer<typeof OrgEntry>;
export type AuditEvent = z.infer<typeof AuditEvent>;
export type HistoryRow = z.infer<typeof HistoryRow>;
export type AuditView = z.infer<typeof AuditView>;
export type PersonalMode = z.infer<typeof PersonalMode>;
```

**In `ui/src/lib/query.ts`, replace:**

```ts
  orgEntries: (orgId: string) => ["orgs", orgId, "entries"] as const,
  /// An org's member list for the Members tab, under the orgs' key.
```

**with:**

```ts
  orgEntries: (orgId: string) => ["orgs", orgId, "entries"] as const,
  /// An org's History and audit sending, under the orgs' key.
  orgAudit: (orgId: string) => ["orgs", orgId, "audit"] as const,
  /// An org's member list for the Members tab, under the orgs' key.
```

Then regenerate the fixture: `UPDATE_CONTRACT=1 cargo test -p dots-app contract -j 2 -- --test-threads 4` (it adds the `AuditView` samples, the two enums and `org_audit` to `Commands`).

- [ ] **Step 4: Run them to see them pass**

Run: `cargo test -p dots-core -j 2 --test runtime_audit_test -- --test-threads 4`, `cargo test -p dots-app -j 2 -- --test-threads 4`, then `(cd ui && DEBUG_PRINT_LIMIT=0 bun run test --maxWorkers=2 src/api)` and `(cd ui && bun run typecheck)`.
Expected: `test result: ok. 1 passed`; dots-app all passed; the UI api tests pass; typecheck clean.

- [ ] **Step 5: Check and commit**

Run `cargo fmt --check` and `cargo clippy --workspace --all-targets -j 2 -- -D warnings`, then:

```bash
git add crates/dots-core/src/vault/audit/mod.rs crates/dots-core/src/vault/audit/view.rs crates/dots-core/src/runtime.rs crates/dots-core/tests/runtime_audit_test.rs apps/dots-app/src/commands.rs apps/dots-app/src/lib.rs apps/dots-app/src/contract.rs ui/src/api/__fixtures__/contract.json ui/src/api/commands.test.ts ui/src/api/commands.ts ui/src/api/contract.test.ts ui/src/api/types.ts ui/src/lib/query.ts
git commit -m "feat(app): org_audit gives an account's History and how sending its audit records stands"
```

---

### Task 9: An account's History tab lists its audit records, and Overview says when sending fails

`AccountHistory` reads `useAuditView(org.id)` (refetched every `AUDIT_REFRESH_MS`) and shows the failing-send line, where records go (or that they stay on this PC), and a table: When, What, Variable, Dot, For, Why, Sent, with the core's codes in words. `AuditProblem` puts the same line on Overview. The tab sits after API keys, for accounts that have tabs.

**Files:**
- Create: `ui/src/screens/accounts/AccountHistory.tsx`
- Modify: `ui/src/screens/Accounts.tsx`, `ui/src/test/fixtures.ts`
- Test: `ui/src/screens/accounts/AccountHistory.test.tsx`

**Interfaces:**
- Consumes: Task 8's `commands.orgAudit`, `keys.orgAudit`, `T.AuditView`; `localDateTime`, `localTime`.
- Produces: `AccountHistory({ org })`, `AuditProblem({ orgId })`, `useAuditView(orgId)`, `auditProblem(view) -> string | null`, `AUDIT_REFRESH_MS`, `EVENT`; `sample.audit` in the test fixtures.

- [ ] **Step 1: Write the failing tests**

**Create `ui/src/screens/accounts/AccountHistory.test.tsx`:**

```tsx
import { screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { AuditView } from "@/api/types";
import { localTime } from "@/lib/format";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const [acme] = sample.orgs;
const [failing, unsent] = sample.audit;

/// Answers `org_list`, `member_list` and `org_audit` like the core for Acme.
function account(view: AuditView) {
  backend.on("org_list", () => [acme]);
  backend.on("member_list", () => sample.members);
  backend.on("workspace_list", () => []);
  backend.on("api_key_list", () => []);
  backend.on("org_audit", () => view);
}

async function row() {
  const view = renderApp("/settings/account");
  const section = await screen.findByRole("group", { name: "Accounts" });
  const item = await within(section).findByRole("listitem", { name: "Acme Engineering" });
  return { ...view, item };
}

async function historyTab() {
  const view = await row();
  await view.user.click(within(view.item).getByRole("tab", { name: "History" }));
  const panel = await within(view.item).findByRole("tabpanel");
  return { ...view, panel };
}

describe("Settings, Accounts, History", () => {
  it("lists the account's records newest first, in words, with whether each was sent", async () => {
    account({ ...failing, send: { ...failing.send, last_error: null } });
    const { panel } = await historyTab();
    const table = await within(panel).findByRole("table", { name: "History of Acme Engineering" });
    const rows = within(table).getAllByRole("row").slice(1);
    const cells = rows.map((r) =>
      within(r)
        .getAllByRole("cell")
        .slice(1)
        .map((c) => c.textContent),
    );
    const refused = "the policy does not allow it";
    const dropped = "the queue was full; the oldest records were dropped";
    expect(cells).toEqual([
      ["Refused", "PROD_TOKEN", "acme-release", "github", refused, "Waiting to send"],
      ["Used", "GITHUB_TOKEN", "acme-release", "github", "", "Sent"],
      ["Expired", "", "", "", dropped, "Dropped unsent"],
    ]);
    expect(panel).toHaveTextContent("Records go to https://audit.acme.example/dots.");
    expect(panel).not.toHaveTextContent("last send failed");
    expect(backend.callsTo("org_audit")).toContainEqual({ orgId: "acme-eng" });
  });

  it("says when sending fails, on History and on Overview", async () => {
    account(failing);
    const { item, user } = await row();
    const line =
      `Audit: 2 records waiting; last send failed ${localTime(failing.send.failed_at)}: ` +
      "the endpoint answered 503";
    expect(await within(item).findByText(line)).toBeInTheDocument();
    await user.click(within(item).getByRole("tab", { name: "History" }));
    const panel = await within(item).findByRole("tabpanel");
    expect(await within(panel).findByText(line)).toBeInTheDocument();
  });

  it("an account with no audit endpoint keeps its records on this PC", async () => {
    account(unsent);
    const { panel } = await historyTab();
    expect(
      await within(panel).findByText(/has no audit endpoint: these records stay on this PC/),
    ).toBeInTheDocument();
    const table = within(panel).getByRole("table", { name: "History of Acme Engineering" });
    expect(table).toHaveTextContent("Offline");
    expect(table).toHaveTextContent("Kept on this PC");
  });

  it("an account with nothing recorded says so", async () => {
    account({ ...unsent, history: [] });
    const { panel } = await historyTab();
    expect(await within(panel).findByText("Nothing recorded yet.")).toBeInTheDocument();
  });
});
```

**In `ui/src/test/fixtures.ts`, replace:**

```ts
  orgEntries: T.OrgEntry.array().parse(contract.OrgEntry),
  /// acme-release's grants in Acme Engineering: GITHUB_TOKEN from the org, LOG_LEVEL from
```

**with:**

```ts
  orgEntries: T.OrgEntry.array().parse(contract.OrgEntry),
  audit: T.AuditView.array().parse(contract.AuditView),
  /// acme-release's grants in Acme Engineering: GITHUB_TOKEN from the org, LOG_LEVEL from
```

- [ ] **Step 2: Run them to see them fail**

Run: `(cd ui && DEBUG_PRINT_LIMIT=0 bun run test --maxWorkers=2 src/screens/accounts/AccountHistory.test.tsx)`
Expected: the file fails to load: `Failed to resolve import "@/screens/accounts/AccountHistory"` (and `sample.audit` is undefined).

- [ ] **Step 3: Implement**

**In `ui/src/screens/Accounts.tsx`, replace:**

```tsx
import { invalidate, keys } from "@/lib/query";
import { AccountMembers } from "@/screens/accounts/AccountMembers";
```

**with:**

```tsx
import { invalidate, keys } from "@/lib/query";
import { AccountHistory, AuditProblem } from "@/screens/accounts/AccountHistory";
import { AccountMembers } from "@/screens/accounts/AccountMembers";
```

**In `ui/src/screens/Accounts.tsx`, replace:**

```tsx
          )}
          {org.member_count !== null && org.roster_seq !== null && (
```

**with:**

```tsx
          )}
          <AuditProblem orgId={org.id} />
          {org.member_count !== null && org.roster_seq !== null && (
```

**In `ui/src/screens/Accounts.tsx`, replace:**

```tsx
          <TabsTrigger value="api-keys">API keys</TabsTrigger>
        </TabsList>
```

**with:**

```tsx
          <TabsTrigger value="api-keys">API keys</TabsTrigger>
          <TabsTrigger value="history">History</TabsTrigger>
        </TabsList>
```

**In `ui/src/screens/Accounts.tsx`, replace:**

```tsx
          <ApiKeys scope={org.id} />
        </TabsContent>
```

**with:**

```tsx
          <ApiKeys scope={org.id} />
        </TabsContent>
        <TabsContent value="history">
          <AccountHistory org={org} />
        </TabsContent>
```

**Create `ui/src/screens/accounts/AccountHistory.tsx`:**

```tsx
import { useQuery } from "@tanstack/react-query";

import { commands } from "@/api/commands";
import type { AuditEvent, AuditView, HistoryRow, OrgSummary } from "@/api/types";
import { ErrorNote } from "@/components/ErrorNote";
import { localDateTime, localTime } from "@/lib/format";
import { keys } from "@/lib/query";

/// How often an account's audit view is read again while it is on screen: a send changes it
/// with no runtime event.
export const AUDIT_REFRESH_MS = 30_000;

/// The account's History and how sending its audit records stands.
export function useAuditView(orgId: string) {
  return useQuery({
    queryKey: keys.orgAudit(orgId),
    queryFn: () => commands.orgAudit(orgId),
    refetchInterval: AUDIT_REFRESH_MS,
  });
}

export const EVENT: Record<AuditEvent, string> = {
  resolved: "Used",
  denied: "Refused",
  fetched: "Fetched",
  offline: "Offline",
  revoked: "Access revoked",
  expired: "Expired",
  policy_invalid: "Policy invalid",
};

const STATE: Record<HistoryRow["state"], string> = {
  local: "Kept on this PC",
  queued: "Waiting to send",
  sent: "Sent",
  dropped: "Dropped unsent",
};

/// The core's reason codes (docs/vault-audit-format.md) in plain words.
const REASON: Record<string, string> = {
  not_granted: "not granted",
  not_defined: "not in the account",
  denied_by_policy: "the policy does not allow it",
  denied_by_workspace: "the workspace does not allow it",
  shell_secrets_forbidden: "no secrets in the shell",
  org_offline: "offline, with no saved copy",
  org_expired: "the saved copy expired",
  org_revoked: "access revoked",
  policy_invalid: "the policy is invalid",
  org_removed: "the account is not joined",
  not_member: "this PC is not on the member list",
  denied: "the source refused the sign-in",
  removed: "taken off the member list",
  offline_copy: "the saved copy ran out",
  queue_full: "the queue was full; the oldest records were dropped",
};

/// `mcp:github` as "github", the shell as "Shell".
function consumerText(consumer: string | null): string {
  if (!consumer) return "";
  return consumer === "shell" ? "Shell" : consumer.replace(/^mcp:/, "");
}

/// What Overview and History say while sending fails: how many records wait, when the last
/// send failed and why. null while sending works, or when the account sends no records.
export function auditProblem(view: AuditView | undefined): string | null {
  if (!view?.endpoint || !view.send.last_error) return null;
  const waiting = `${view.queued.toLocaleString()} ${view.queued === 1 ? "record" : "records"}`;
  const when = localTime(view.send.failed_at);
  return `Audit: ${waiting} waiting; last send failed ${when}: ${view.send.last_error}`;
}

/// The Overview line for a failing send, or nothing.
export function AuditProblem({ orgId }: { orgId: string }) {
  const audit = useAuditView(orgId);
  const problem = auditProblem(audit.data);
  return problem ? <p className="text-xs text-needs">{problem}</p> : null;
}

/// The History tab: this PC's audit records for the account, newest first. A record never
/// holds a value.
export function AccountHistory({ org }: { org: OrgSummary }) {
  const audit = useAuditView(org.id);
  if (audit.isPending) return <p className="text-xs text-muted-foreground">Loading…</p>;
  if (audit.isError) return <ErrorNote error={audit.error} />;
  const view = audit.data;
  const problem = auditProblem(view);
  return (
    <div className="flex flex-col gap-2 text-xs">
      {problem && <p className="text-needs">{problem}</p>}
      <p className="text-muted-foreground">
        {view.endpoint
          ? `Records go to ${view.endpoint}.`
          : `${org.name} has no audit endpoint: these records stay on this PC.`}{" "}
        The last 30 days on this PC, newest first. Values are never recorded.
      </p>
      {view.history.length === 0 ? (
        <p className="text-muted-foreground">Nothing recorded yet.</p>
      ) : (
        <table aria-label={`History of ${org.name}`} className="w-full text-left">
          <thead className="text-muted-foreground">
            <tr>
              <th className="font-normal">When</th>
              <th className="font-normal">What</th>
              <th className="font-normal">Variable</th>
              <th className="font-normal">Dot</th>
              <th className="font-normal">For</th>
              <th className="font-normal">Why</th>
              <th className="font-normal">Sent</th>
            </tr>
          </thead>
          <tbody>
            {view.history.map((row) => (
              <tr key={row.id}>
                <td>{localDateTime(row.at)}</td>
                <td>{EVENT[row.event]}</td>
                <td className="font-mono">{row.name ?? ""}</td>
                <td>{row.dot_name ?? ""}</td>
                <td>{consumerText(row.consumer)}</td>
                <td>{row.reason ? (REASON[row.reason] ?? row.reason) : ""}</td>
                <td>{STATE[row.state]}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}
```

- [ ] **Step 4: Run them to see them pass**

Run: `(cd ui && DEBUG_PRINT_LIMIT=0 bun run test --maxWorkers=2 src/screens/accounts/AccountHistory.test.tsx src/screens/Accounts.test.tsx)`, then the typecheck.
Expected: `Test Files  2 passed (2)`, `Tests  33 passed (33)`; typecheck clean.

- [ ] **Step 5: Check and commit**

Run the whole UI suite once (`(cd ui && DEBUG_PRINT_LIMIT=0 bun run test --maxWorkers=2)`; the file count must equal `find ui/src -name '*.test.ts*' | wc -l`), then:

```bash
git add ui/src/screens/accounts/AccountHistory.tsx ui/src/screens/accounts/AccountHistory.test.tsx ui/src/screens/Accounts.tsx ui/src/test/fixtures.ts
git commit -m "feat(ui): an account's History tab lists its audit records, and Overview says when sending fails"
```

---

### Task 10: A run of a fresh account does not wait behind a fetch, and start-up fetches stop at shutdown

`refresh_org(_, only_if_stale)` asks `wants_refresh` before it takes the account's lock: a fresh account (or one within `RETRY_AFTER` of a failed fetch, a damaged or a removed one) returns at once, so a run start no longer waits behind a timer's or Refresh's fetch. A stale one waits for the lock and asks `due` again under it, so it does not fetch twice. `refresh_all` spawns into a `JoinSet` that it owns, so dropping it (the timers' start-up task, at shutdown) aborts the fetches. The existing test `a_run_whose_org_is_removed_while_it_waits_to_refresh_does_not_start` makes its account stale first, since only a stale one waits now.

**Files:**
- Modify: `crates/dots-core/src/vault/org/service.rs`
- Test: `crates/dots-core/tests/vault_org_resolve_test.rs`, `crates/dots-core/tests/vault_org_timers_test.rs`

**Interfaces:**
- Consumes: plan B's `Orgs::refresh_if_stale`, `Orgs::refresh_all`, `OrgTimers::start_up`; Task 3's audit records (the timers test counts the queue).
- Produces: private `Orgs::wants_refresh(&str) -> bool`, `Orgs::due(&OrgRow, Option<u32>) -> bool`; no public change.

- [ ] **Step 1: Write the failing tests**

**In `crates/dots-core/tests/vault_org_resolve_test.rs`, replace:**

```rust

    // A Refresh holds the org's lock while its source answers.
    let gate = scripted.hold();
```

**with:**

```rust

    // The org is stale, so the run's start waits for the org's lock. A Refresh holds it
    // while its source answers.
    h.advance(Duration::minutes(16));
    let gate = scripted.hold();
```

**In `crates/dots-core/tests/vault_org_resolve_test.rs`, replace:**

```rust
        other => panic!("expected the run refused, got {other:?}"),
    }
}

```

**with:**

```rust
        other => panic!("expected the run refused, got {other:?}"),
    }
}

/// A run's start does not wait behind a fetch under way when the org is fresh: it uses what it
/// has, and the fetch goes on.
#[tokio::test]
async fn a_run_of_a_fresh_org_starts_while_a_refresh_is_under_way() {
    let h = OrgsHarness::new().await;
    let scripted = Scripted::new();
    let orgs = h.service().with_sources(scripted.factory());
    let signed = sign(&h.keys, &policy(ACME, ISSUED, json!({})));
    scripted.serve(&[("BOTH", ORG_BOTH)], &signed);
    orgs.join(&h.acme_code(), Some(token())).await.unwrap();
    scripted.entered().await;
    let vault = Vault::new(h.store.clone(), h.secrets.clone()).with_orgs(orgs.clone());
    let dot = h
        .store
        .create_dot(&spec(DOT), Some(ACME), "default")
        .await
        .unwrap();

    let gate = scripted.hold();
    let refreshing = tokio::spawn({
        let orgs = orgs.clone();
        async move { orgs.refresh(ACME).await }
    });
    scripted.entered().await;
    tokio::time::timeout(std::time::Duration::from_secs(5), vault.before_run(&dot))
        .await
        .expect("the run's start waited for the refresh")
        .unwrap();
    // Stale, it waits for the refresh under way, and does not fetch again.
    h.advance(Duration::minutes(16));
    let start = vault.before_run(&dot);
    tokio::pin!(start);
    assert!(futures_util::poll!(&mut start).is_pending());
    gate.add_permits(1);
    refreshing.await.unwrap().unwrap();
    start.await.unwrap();
    assert_eq!(scripted.creds().len(), 2);
}

```

**In `crates/dots-core/tests/vault_org_timers_test.rs`, replace:**

```rust
use common::fake_openbao::Failure;
use common::orgs::{at, policy, sign, token, OrgsHarness, ACME, START};
use dots_core::events::{new_bus, Bus, RuntimeEvent};
```

**with:**

```rust
use common::fake_openbao::Failure;
use common::orgs::{at, policy, sign, token, OrgsHarness, Scripted, ACME, START};
use dots_core::events::{new_bus, Bus, RuntimeEvent};
```

**In `crates/dots-core/tests/vault_org_timers_test.rs`, replace:**

```rust

/// Every timer, and a start-up fetch that hangs, stop with the shutdown token.
```

**with:**

```rust

/// A start-up fetch that the shutdown stops does not finish later: its outcome is never
/// applied, and nothing is recorded for it.
#[tokio::test]
async fn a_start_up_fetch_stopped_by_the_shutdown_never_lands() {
    let h = OrgsHarness::new().await;
    let scripted = Scripted::new();
    let orgs = h.service().with_sources(scripted.factory());
    let extra = json!({"audit": {"endpoint": "https://audit.acme.test", "token_entry": "API_KEY"}});
    let signed = sign(&h.keys, &policy(ACME, "2026-10-22T08:00:00Z", extra));
    scripted.serve(&[("API_KEY", ORG_VALUE)], &signed);
    orgs.join(&h.acme_code(), Some(token())).await.unwrap();
    scripted.entered().await;
    h.advance(Duration::minutes(1));
    let shutdown = CancellationToken::new();
    let timers = OrgTimers::new(orgs.clone(), h.clock.clone(), new_bus(), shutdown.clone());

    let gate = scripted.hold();
    timers.start_up();
    scripted.entered().await;
    shutdown.cancel();
    eventually("the start-up fetch's end", || !timers.starting_up()).await;
    gate.add_permits(1);
    tokio::time::sleep(StdDuration::from_millis(200)).await;
    let row = h.store.get_org(ACME).await.unwrap().unwrap();
    assert_eq!(
        row.last_fetch_at.as_deref(),
        Some("2026-10-22T09:00:00.000Z")
    );
    assert_eq!(h.store.audit_queued(ACME).await.unwrap(), 1);
}

/// Every timer, and a start-up fetch that hangs, stop with the shutdown token.
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core -j 2 --test vault_org_resolve_test -- --test-threads 4 a_run_of_a_fresh_org`, then `cargo test -p dots-core -j 2 --test vault_org_timers_test -- --test-threads 4 a_start_up_fetch_stopped`
Expected: the first FAILS at `the run's start waited for the refresh` (the timeout); the second FAILS with `left: Some("2026-10-22T09:01:00.000Z")`, `right: Some("2026-10-22T09:00:00.000Z")`: the stopped fetch still landed.

- [ ] **Step 3: Implement**

**In `crates/dots-core/src/vault/org/service.rs`, replace:**

```rust

    /// Fetches every joined org at once, at start, after `load`. A failure is logged.
    pub async fn refresh_all(&self) {
```

**with:**

```rust

    /// Fetches every joined org at once, at start, after `load`. A failure is logged. The
    /// fetches belong to this call: when it is dropped (the app shuts down while a source
    /// hangs), they stop with it.
    pub async fn refresh_all(&self) {
```

**In `crates/dots-core/src/vault/org/service.rs`, replace:**

```rust
            .collect();
        let tasks: Vec<_> = ids
            .into_iter()
            .map(|id| {
                let orgs = self.clone();
                tokio::spawn(async move {
                    if let Err(e) = orgs.refresh(&id).await {
                        tracing::warn!("org {id:?} was not refreshed: {e}");
                    }
                })
            })
            .collect();
        for task in tasks {
            if let Err(e) = task.await {
                tracing::warn!("an org refresh did not finish: {e}");
```

**with:**

```rust
            .collect();
        let mut tasks = tokio::task::JoinSet::new();
        for id in ids {
            let orgs = self.clone();
            tasks.spawn(async move {
                if let Err(e) = orgs.refresh(&id).await {
                    tracing::warn!("org {id:?} was not refreshed: {e}");
                }
            });
        }
        while let Some(done) = tasks.join_next().await {
            if let Err(e) = done {
                tracing::warn!("an org refresh did not finish: {e}");
```

**In `crates/dots-core/src/vault/org/service.rs`, replace:**

```rust
    async fn refresh_org(&self, org_id: &str, only_if_stale: bool) -> Result<bool> {
        let _guard = self.lock(org_id).await;
```

**with:**

```rust
    async fn refresh_org(&self, org_id: &str, only_if_stale: bool) -> Result<bool> {
        // A run's start does not wait behind a fetch under way (a timer's, a Refresh) for an
        // org that is fresh: only a stale one waits for the lock, and is looked at again then.
        if only_if_stale && !self.wants_refresh(org_id) {
            return Ok(false);
        }
        let _guard = self.lock(org_id).await;
```

**In `crates/dots-core/src/vault/org/service.rs`, replace:**

```rust
        }
        if only_if_stale {
            let now = self.inner.clock.now();
            let failed = self.failed_at().get(org_id).copied();
            if !is_stale(&row, refresh_minutes, now)
                || failed.is_some_and(|at| now - at < RETRY_AFTER)
            {
                return Ok(false);
            }
        }
```

**with:**

```rust
        }
        if only_if_stale && !self.due(&row, refresh_minutes) {
            return Ok(false);
        }
```

**In `crates/dots-core/src/vault/org/service.rs`, replace:**

```rust
        Ok(true)
    }
```

**with:**

```rust
        Ok(true)
    }

    /// Whether a run's start should fetch the org: it is joined, on the member list or never
    /// was, and `due`. Read without the org's lock, so a fetch under way doesn't hold it up; an
    /// org that is not joined says yes, and the locked path gives the error.
    fn wants_refresh(&self, org_id: &str) -> bool {
        let (row, refresh_minutes) = match self.read().get(org_id) {
            Some(Slot::Joined(joined)) => (
                joined.row.clone(),
                joined.policy.as_ref().map(|p| p.refresh_minutes),
            ),
            Some(Slot::Damaged { .. }) => return false,
            Some(Slot::Pending(_)) | None => return true,
        };
        row.member.membership != Membership::Removed && self.due(&row, refresh_minutes)
    }

    /// Whether the org of `row` is stale, and not within `RETRY_AFTER` of a failed fetch.
    fn due(&self, row: &OrgRow, refresh_minutes: Option<u32>) -> bool {
        let now = self.inner.clock.now();
        let failed = self.failed_at().get(&row.id).copied();
        is_stale(row, refresh_minutes, now) && failed.is_none_or(|at| now - at >= RETRY_AFTER)
    }
```

- [ ] **Step 4: Run them to see them pass**

Run: `cargo test -p dots-core -j 2 --test vault_org_resolve_test --test vault_org_timers_test --test runtime_org_test -- --test-threads 4`
Expected: `test result: ok. 22 passed` (`vault_org_resolve_test`), `test result: ok. 8 passed` (`vault_org_timers_test`), `test result: ok. 7 passed` (`runtime_org_test`).

- [ ] **Step 5: Check and commit**

Run `cargo fmt --check` and `cargo clippy -p dots-core --all-targets -j 2 -- -D warnings`, then:

```bash
git add crates/dots-core/src/vault/org/service.rs crates/dots-core/tests/vault_org_resolve_test.rs crates/dots-core/tests/vault_org_timers_test.rs
git commit -m "fix(vault): a run of a fresh org does not wait behind a fetch, and start-up fetches stop at shutdown"
```

---

### Task 11: A half-done Remove keeps the account listed, and sign-in details kept for an account not joined can be removed

`Orgs::remove` moves the deletes after the values into `finish_remove`; when one fails, a joined account's slot comes back as `Damaged` with `removing it did not finish (<error>); remove it again`, so it is listed, gives no values, and can be removed again. `Orgs::leftover_keys()` lists the ids of `org/<id>/source` sign-in details whose account is not joined (a SOPS key from a join that never finished); `org_leftover_keys` serves it, and Settings › Accounts lists each with Remove (`org_remove`, which already takes back details alone).

**Files:**
- Modify: `crates/dots-core/src/vault/org/service.rs`, `apps/dots-app/src/{commands,lib}.rs`, `ui/src/api/{commands.ts,__fixtures__/contract.json}`, `ui/src/lib/query.ts`, `ui/src/screens/Accounts.tsx`
- Test: `crates/dots-core/tests/vault_orgs_test.rs`, `ui/src/api/commands.test.ts`, `ui/src/screens/Accounts.test.tsx`

**Interfaces:**
- Consumes: plan B's `Orgs::remove`, `Orgs::sops_identity`, `SecretStore::keys`; the UI's `ConfirmButton`.
- Produces: `Orgs::leftover_keys(&self) -> Result<Vec<String>>`; app command `org_leftover_keys()`; UI `commands.orgLeftoverKeys()`, `keys.orgLeftoverKeys = ["orgs", "leftover-keys"]`.

- [ ] **Step 1: Write the failing tests**

**In `crates/dots-core/tests/vault_orgs_test.rs`, replace:**

```rust
    spec
}
```

**with:**

```rust
    spec
}

/// A Remove that fails after the values are gone keeps the org listed, with no values, so it
/// can be removed again.
#[tokio::test]
async fn a_remove_that_does_not_finish_keeps_the_org_listed_without_values() {
    let h = OrgsHarness::new().await;
    h.join_acme(entries(), acme_policy()).await;
    // The copy can't be deleted: a folder stands where it is.
    let copy = h.copy_path(ACME);
    std::fs::remove_file(&copy).unwrap();
    std::fs::create_dir(&copy).unwrap();
    std::fs::write(copy.join("held"), "x").unwrap();

    let e = h.orgs.remove(ACME).await.unwrap_err();
    assert!(other(e).contains("deleting its offline copy failed"));
    let listed = h.orgs.summaries().await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, ACME_NAME);
    assert!(
        listed[0].status_text.contains(
            "removing it did not finish (org 'acme-eng': deleting its offline copy failed"
        ),
        "{}",
        listed[0].status_text
    );
    assert!(matches!(
        h.orgs.lookup(ACME, "API_KEY").await,
        OrgLookup::PolicyInvalid(_)
    ));

    std::fs::remove_dir_all(&copy).unwrap();
    h.orgs.remove(ACME).await.unwrap();
    assert_eq!(h.orgs.summaries().await, vec![]);
    assert!(h.store.get_org(ACME).await.unwrap().is_none());
}

/// Sign-in details kept for an org that is not joined (a SOPS key made for a join that never
/// finished) are listed by org id, and Remove takes them back.
#[tokio::test]
async fn sign_in_details_for_an_org_not_joined_are_listed_and_removable() {
    let h = OrgsHarness::new().await;
    h.join_acme(entries(), acme_policy()).await;
    h.orgs.sops_identity("design").await.unwrap();
    assert_eq!(h.orgs.leftover_keys().unwrap(), vec!["design".to_string()]);
    h.orgs.remove("design").await.unwrap();
    assert_eq!(h.orgs.leftover_keys().unwrap(), Vec::<String>::new());
    assert_eq!(stored_creds(&h, ACME), Some(token()));
}
```

**In `ui/src/api/commands.test.ts`, replace:**

```ts
    expect(await commands.orgAudit("acme-eng")).toEqual(fixture.AuditView[0]);
    expect(calls).toEqual([
```

**with:**

```ts
    expect(await commands.orgAudit("acme-eng")).toEqual(fixture.AuditView[0]);
    reply = async () => ["design"];
    expect(await commands.orgLeftoverKeys()).toEqual(["design"]);
    expect(calls).toEqual([
```

**In `ui/src/api/commands.test.ts`, replace:**

```ts
      ["org_audit", { orgId: "acme-eng" }],
    ]);
```

**with:**

```ts
      ["org_audit", { orgId: "acme-eng" }],
      ["org_leftover_keys", undefined],
    ]);
```

**In `ui/src/screens/Accounts.test.tsx`, replace:**

```tsx
    expect(await navigator.clipboard.readText()).toBe(REQUEST);
  });
});
```

**with:**

```tsx
    expect(await navigator.clipboard.readText()).toBe(REQUEST);
  });
});

describe("Settings, Accounts, sign-in details kept for an account not joined", () => {
  it("lists them by account id, and Remove takes them back", async () => {
    orgs([]);
    let kept = ["design"];
    backend.on("org_leftover_keys", () => kept);
    backend.on("org_remove", () => {
      kept = [];
      return null;
    });
    const { user } = renderApp("/settings/account");
    const list = await within(await section()).findByRole("list", {
      name: "Sign-in details kept for accounts not joined",
    });
    const item = within(list).getByRole("listitem", { name: "design" });
    expect(item).toHaveTextContent(
      "This PC keeps sign-in details for 'design', which is not joined.",
    );
    await user.click(within(item).getByRole("button", { name: "Remove" }));
    const dialog = await screen.findByRole("alertdialog");
    await user.click(within(dialog).getByRole("button", { name: "Remove" }));
    await waitFor(() => expect(backend.callsTo("org_remove")).toEqual([{ orgId: "design" }]));
    await waitFor(() =>
      expect(
        screen.queryByRole("list", { name: "Sign-in details kept for accounts not joined" }),
      ).toBeNull(),
    );
  });
});
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core -j 2 --test vault_orgs_test -- --test-threads 4`
Expected: fails to compile: no method `leftover_keys` on `Orgs`. (Before that method, `a_remove_that_does_not_finish_keeps_the_org_listed_without_values` fails at `assert_eq!(listed.len(), 1)`: the half-removed account was gone from the list.)

- [ ] **Step 3: Implement**

**In `apps/dots-app/src/commands.rs`, replace:**

```rust

/// The account's History on this PC and how sending its audit records stands (env vault spec
```

**with:**

```rust

/// The org ids this PC keeps sign-in details for while none such is joined (a SOPS key for a
/// join that never finished); `org_remove` takes them back.
#[tauri::command]
pub fn org_leftover_keys(state: State<'_, AppState>) -> Res<Vec<String>> {
    Ok(state.rt()?.orgs().leftover_keys()?)
}

/// The account's History on this PC and how sending its audit records stands (env vault spec
```

**In `apps/dots-app/src/lib.rs`, replace:**

```rust
    org_audit,
    messages_list,
```

**with:**

```rust
    org_audit,
    org_leftover_keys,
    messages_list,
```

**In `crates/dots-core/src/vault/org/service.rs`, replace:**

```rust
        self.identities().remove(org_id);
        let (dir, id) = (self.inner.dir.clone(), org_id.to_string());
```

**with:**

```rust
        self.identities().remove(org_id);
        if let Err(e) = self.finish_remove(org_id, has_row).await {
            // A joined org that is half removed stays listed, giving no values, so the person
            // sees it and can remove it again.
            if let Some((_, name)) = names.into_iter().find(|(id, _)| id == org_id) {
                let error = format!("removing it did not finish ({e}); remove it again");
                self.write()
                    .insert(org_id.to_string(), Slot::Damaged { name, error });
            }
            return Err(e);
        }
        Ok(())
    }

    /// What Remove deletes after the values: the copy, the sign-in details and the row.
    async fn finish_remove(&self, org_id: &str, has_row: bool) -> Result<()> {
        let (dir, id) = (self.inner.dir.clone(), org_id.to_string());
```

**In `crates/dots-core/src/vault/org/service.rs`, replace:**

```rust
            self.inner.store.remove_org(org_id).await?;
        }
        Ok(())
    }

    /// Fetches the org now (**Refresh**, the timer) and applies the outcome: `Connected` with
```

**with:**

```rust
            self.inner.store.remove_org(org_id).await?;
        }
        Ok(())
    }

    /// The org ids this PC keeps sign-in details for while no such org is joined: a SOPS key
    /// made for a join that never finished, or one waiting for the admin. Settings lists them,
    /// and **Remove** takes them back. Never a value.
    pub fn leftover_keys(&self) -> Result<Vec<String>> {
        let keys = self.inner.secrets.keys("org/")?;
        let slots = self.read();
        Ok(keys
            .iter()
            .filter_map(|key| key.strip_prefix("org/")?.strip_suffix("/source"))
            .filter(|id| !matches!(slots.get(*id), Some(Slot::Joined(_) | Slot::Damaged { .. })))
            .map(str::to_string)
            .collect())
    }

    /// Fetches the org now (**Refresh**, the timer) and applies the outcome: `Connected` with
```

**In `ui/src/api/commands.ts`, replace:**

```ts
  orgEntries: (orgId: string) => call(T.OrgEntry.array(), "org_entries", { orgId }),
  /// The org's History on this PC and how sending its audit records stands. Never a value.
```

**with:**

```ts
  orgEntries: (orgId: string) => call(T.OrgEntry.array(), "org_entries", { orgId }),
  /// The org ids this PC keeps sign-in details for while none such is joined; `orgRemove`
  /// takes them back.
  orgLeftoverKeys: () => call(z.array(z.string()), "org_leftover_keys"),
  /// The org's History on this PC and how sending its audit records stands. Never a value.
```

**In `ui/src/lib/query.ts`, replace:**

```ts
  orgEntries: (orgId: string) => ["orgs", orgId, "entries"] as const,
  /// An org's History and audit sending, under the orgs' key.
```

**with:**

```ts
  orgEntries: (orgId: string) => ["orgs", orgId, "entries"] as const,
  /// The org ids this PC keeps sign-in details for while none such is joined.
  orgLeftoverKeys: ["orgs", "leftover-keys"] as const,
  /// An org's History and audit sending, under the orgs' key.
```

**In `ui/src/screens/Accounts.tsx`, replace:**

```tsx

/// Settings → Accounts (env vault spec §9): the orgs joined on this PC, Join, and per org
```

**with:**

```tsx

/// Sign-in details this PC keeps for an account that is not joined (a SOPS key made for a join
/// that never finished), and Remove.
function LeftoverKey({ orgId }: { orgId: string }) {
  const qc = useQueryClient();
  const remove = useMutation({
    mutationFn: () => commands.orgRemove(orgId),
    onSuccess: () => invalidate(qc, keys.orgs),
    onError: notifyError,
  });
  return (
    <li aria-label={orgId} className="flex flex-wrap items-center gap-2 text-xs">
      <span className="flex-1">
        This PC keeps sign-in details for '{orgId}', which is not joined.
      </span>
      <ConfirmButton
        title={`Remove the sign-in details for '${orgId}'?`}
        description={
          "Joining this account later needs new ones. For a SOPS file, send your admin the " +
          "new key then."
        }
        confirmLabel="Remove"
        disabled={remove.isPending}
        onConfirm={() => remove.mutate()}
      >
        Remove
      </ConfirmButton>
    </li>
  );
}

/// Settings → Accounts (env vault spec §9): the orgs joined on this PC, Join, and per org
```

**In `ui/src/screens/Accounts.tsx`, replace:**

```tsx
  const list = useQuery({ queryKey: keys.orgs, queryFn: commands.orgList });
  // The org to update; null to join a new one; undefined while the dialog is closed.
```

**with:**

```tsx
  const list = useQuery({ queryKey: keys.orgs, queryFn: commands.orgList });
  const leftover = useQuery({
    queryKey: keys.orgLeftoverKeys,
    queryFn: commands.orgLeftoverKeys,
  });
  // The org to update; null to join a new one; undefined while the dialog is closed.
```

**In `ui/src/screens/Accounts.tsx`, replace:**

```tsx
      )}
      <section aria-label="Personal" className="flex flex-col gap-2 rounded-md border p-3">
```

**with:**

```tsx
      )}
      {(leftover.data ?? []).length > 0 && (
        <ul
          aria-label="Sign-in details kept for accounts not joined"
          className="flex flex-col gap-1"
        >
          {(leftover.data ?? []).map((id) => (
            <LeftoverKey key={id} orgId={id} />
          ))}
        </ul>
      )}
      <section aria-label="Personal" className="flex flex-col gap-2 rounded-md border p-3">
```

Then regenerate the fixture (`UPDATE_CONTRACT=1 cargo test -p dots-app contract -j 2 -- --test-threads 4`; it adds `org_leftover_keys` to `Commands`).

- [ ] **Step 4: Run them to see them pass**

Run: `cargo test -p dots-core -j 2 --test vault_orgs_test -- --test-threads 4`, then `(cd ui && DEBUG_PRINT_LIMIT=0 bun run test --maxWorkers=2 src/api src/screens/Accounts.test.tsx src/screens/accounts)` and the typecheck.
Expected: `test result: ok. 47 passed`; the UI files pass; typecheck clean.

- [ ] **Step 5: Check and commit**

Run `cargo fmt --check` and `cargo clippy --workspace --all-targets -j 2 -- -D warnings`, then:

```bash
git add crates/dots-core/src/vault/org/service.rs crates/dots-core/tests/vault_orgs_test.rs apps/dots-app/src/commands.rs apps/dots-app/src/lib.rs ui/src/api/__fixtures__/contract.json ui/src/api/commands.ts ui/src/api/commands.test.ts ui/src/lib/query.ts ui/src/screens/Accounts.tsx ui/src/screens/Accounts.test.tsx
git commit -m "fix(vault): a half-done Remove keeps the account listed, and sign-in details kept for an account not joined can be removed"
```

---

### Task 12: An output never replaces the command's input or a key, and a half-written key is deleted

`write_file(path, text, inputs)` refuses a `path` that is one of `inputs` (compared by canonical path) or that holds a minisign key (`holds_a_key`: a comment line and one base64 line starting with `Ed`), and replaces any other file as before. `policy sign --out` and `roster sign --out` pass the policy or list and the key; `join-code --file` passes the `--pub` file; in-place edits pass none. `write_new` deletes a key file whose write fails.

**Files:**
- Modify: `crates/dots-admin/src/{lib,join,policy,roster,keys}.rs`
- Test: `crates/dots-admin/tests/admin_test.rs`

**Interfaces:**
- Consumes: plan B's `policy sign`, `roster sign`, `join-code`, `keygen`.
- Produces: `write_file(&str, &str, &[&str]) -> Result<()>` (crate-private); no command line changes.

- [ ] **Step 1: Write the failing test**

**In `crates/dots-admin/tests/admin_test.rs`, replace:**

```rust

// audit verify (env vault spec §12)
```

**with:**

```rust

/// An output never replaces the files the command read, nor a key.
#[test]
fn an_output_file_never_replaces_the_input_or_a_key() {
    let dir = TempDir::new().unwrap();
    let out = dir.path().to_str().unwrap();
    ok(&["keygen", "--out", out, "--no-password"]);
    let policy_file = path(dir.path(), "policy.json");
    ok(&[
        "policy",
        "new",
        "--org-id",
        "acme-eng",
        "--out",
        &policy_file,
    ]);
    let key_file = path(dir.path(), "policy.key");
    let pub_file = path(dir.path(), "policy.pub");
    let (policy, key, public) = (
        std::fs::read_to_string(&policy_file).unwrap(),
        std::fs::read_to_string(&key_file).unwrap(),
        std::fs::read_to_string(&pub_file).unwrap(),
    );

    let sign = |target: &str| {
        failure(&[
            "policy",
            "sign",
            &policy_file,
            "--key",
            &key_file,
            "--out",
            target,
        ])
        .to_string()
    };
    assert_eq!(
        sign(&policy_file),
        format!("{policy_file} is a file this command reads; choose another output file")
    );
    assert_eq!(
        sign(&key_file),
        format!("{key_file} is a file this command reads; choose another output file")
    );
    assert_eq!(
        sign(&pub_file),
        format!(
            "{pub_file} holds a minisign key; dots-admin never overwrites a key. Choose another \
             output file"
        )
    );
    let join = failure(&[
        "join-code",
        "--org-id",
        "acme-eng",
        "--name",
        "Acme Engineering",
        "--source",
        "openbao",
        "--address",
        "https://bao.acme.internal",
        "--path",
        "dots",
        "--pub",
        &pub_file,
        "--file",
        &key_file,
    ]);
    assert!(join.to_string().contains("holds a minisign key"), "{join}");
    assert_eq!(std::fs::read_to_string(&policy_file).unwrap(), policy);
    assert_eq!(std::fs::read_to_string(&key_file).unwrap(), key);
    assert_eq!(std::fs::read_to_string(&pub_file).unwrap(), public);

    // Another file is still replaced, as before.
    let signed = path(dir.path(), "signed.txt");
    std::fs::write(&signed, "old").unwrap();
    ok(&[
        "policy",
        "sign",
        &policy_file,
        "--key",
        &key_file,
        "--out",
        &signed,
    ]);
    assert!(std::fs::read_to_string(&signed).unwrap().starts_with('{'));
}

// audit verify (env vault spec §12)
```

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test -p dots-admin -j 2 -- --test-threads 4 an_output_file_never_replaces`
Expected: FAILS at the first `failure(…)`: `the command should fail` (the sign overwrote its own input).

- [ ] **Step 3: Implement**

**In `crates/dots-admin/src/join.rs`, replace:**

```rust
    let source = source_settings(&args)?;
    let public_key = read_public_key(&read_file(args.required("pub")?, "the public key")?)?;
    let code = join_code(
```

**with:**

```rust
    let source = source_settings(&args)?;
    let pub_path = args.required("pub")?;
    let public_key = read_public_key(&read_file(pub_path, "the public key")?)?;
    let code = join_code(
```

**In `crates/dots-admin/src/join.rs`, replace:**

```rust
        Some(path) => {
            write_file(path, &format!("{}\n", code.encode()))?;
            say(io.out, &format!("wrote {path}"))?;
```

**with:**

```rust
        Some(path) => {
            write_file(path, &format!("{}\n", code.encode()), &[pub_path])?;
            say(io.out, &format!("wrote {path}"))?;
```

**In `crates/dots-admin/src/keys.rs`, replace:**

```rust
    })?;
    file.write_all(text.as_bytes())
        .map_err(|e| failed(format!("could not write {}: {e}", path.display())))
}
```

**with:**

```rust
    })?;
    if let Err(e) = file.write_all(text.as_bytes()) {
        // Half a key is no key: it goes, rather than stay where the next keygen refuses to
        // write.
        drop(file);
        let _ = std::fs::remove_file(path);
        return Err(failed(format!("could not write {}: {e}", path.display())));
    }
    Ok(())
}
```

**In `crates/dots-admin/src/lib.rs`, replace:**

```rust

/// Writes a result file, replacing one that is there.
pub(crate) fn write_file(path: &str, text: &str) -> Result<()> {
    std::fs::write(Path::new(path), text)
        .map_err(|e| failed(format!("could not write {path}: {e}")))
}
```

**with:**

```rust

/// Writes a result file, replacing one that is there. It never replaces a minisign key file
/// (the old policy key may be the one every join code names), nor one of `inputs`, the files
/// the command read: an `--out` that names the policy being signed would lose it.
pub(crate) fn write_file(path: &str, text: &str, inputs: &[&str]) -> Result<()> {
    let target = Path::new(path);
    if let Ok(existing) = std::fs::canonicalize(target) {
        let input = inputs
            .iter()
            .any(|i| std::fs::canonicalize(i).is_ok_and(|i| i == existing));
        if input {
            return Err(failed(format!(
                "{path} is a file this command reads; choose another output file"
            )));
        }
        if holds_a_key(&existing) {
            return Err(failed(format!(
                "{path} holds a minisign key; dots-admin never overwrites a key. Choose \
                 another output file"
            )));
        }
    }
    std::fs::write(target, text).map_err(|e| failed(format!("could not write {path}: {e}")))
}

/// Whether the file at `path` is a minisign key: a comment line, then one base64 line whose
/// bytes start with the `Ed` algorithm tag. A signature has more lines, and a policy or a
/// join code is not one at all.
fn holds_a_key(path: &Path) -> bool {
    use base64::Engine;
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines.len() == 2
        && lines[0].starts_with("untrusted comment:")
        && base64::engine::general_purpose::STANDARD
            .decode(lines[1].trim())
            .is_ok_and(|bytes| bytes.starts_with(b"Ed"))
}
```

**In `crates/dots-admin/src/policy.rs`, replace:**

```rust
        Some(out) => {
            write_file(out, &format!("{}\n", signed.text))?;
            say(io.out, &note)?;
```

**with:**

```rust
        Some(out) => {
            write_file(out, &format!("{}\n", signed.text), &[path, key_path])?;
            say(io.out, &note)?;
```

**In `crates/dots-admin/src/policy.rs`, replace:**

```rust
    parse_policy(&edited).map_err(refused)?;
    write_file(path, &edited)?;
    let fingerprint = key_fingerprint(admin).map_err(core_failed)?;
```

**with:**

```rust
    parse_policy(&edited).map_err(refused)?;
    write_file(path, &edited, &[])?;
    let fingerprint = key_fingerprint(admin).map_err(core_failed)?;
```

**In `crates/dots-admin/src/roster.rs`, replace:**

```rust
    parse_roster(&text, &roster.org_id).map_err(refused)?;
    write_file(path, &text)
}
```

**with:**

```rust
    parse_roster(&text, &roster.org_id).map_err(refused)?;
    write_file(path, &text, &[])
}
```

**In `crates/dots-admin/src/roster.rs`, replace:**

```rust
    let path = args.one("the member list")?;
    let key_text = read_file(args.required("key")?, "the key")?;
    let mut roster = read_roster(path)?;
```

**with:**

```rust
    let path = args.one("the member list")?;
    let key_path = args.required("key")?;
    let key_text = read_file(key_path, "the key")?;
    let mut roster = read_roster(path)?;
```

**In `crates/dots-admin/src/roster.rs`, replace:**

```rust
        .map_err(|e| failed(format!("the signed member list does not verify: {e}")))?;
    write_file(path, &text)?;

```

**with:**

```rust
        .map_err(|e| failed(format!("the signed member list does not verify: {e}")))?;
    write_file(path, &text, &[])?;

```

**In `crates/dots-admin/src/roster.rs`, replace:**

```rust
        Some(out) => {
            write_file(out, &format!("{signed}\n"))?;
            say(io.out, &note)?;
```

**with:**

```rust
        Some(out) => {
            write_file(out, &format!("{signed}\n"), &[path, key_path])?;
            say(io.out, &note)?;
```

- [ ] **Step 4: Run it to see it pass**

Run: `cargo test -p dots-admin -j 2 -- --test-threads 4`
Expected: `test result: ok. 40 passed` (`admin_test`).

- [ ] **Step 5: Check and commit**

Run `cargo fmt --check` and `cargo clippy -p dots-admin --all-targets -j 2 -- -D warnings`, then:

```bash
git add crates/dots-admin/src/lib.rs crates/dots-admin/src/join.rs crates/dots-admin/src/policy.rs crates/dots-admin/src/roster.rs crates/dots-admin/src/keys.rs crates/dots-admin/tests/admin_test.rs
git commit -m "fix(dots-admin): an output never replaces the command's input or a key, and a half-written key is deleted"
```

---

### Task 13: Personal gaps say Not in Personal and leave account dots alone, the offline status has no UTC time, and Add makes you pick Plain or Secret

The smoke follow-ups. `ToolsAndSkills` gets the dot's `orgId` and names Personal's gaps ("Not in Personal: …") only for a Personal dot; the Library's server row says "Not in Personal" too. The org status texts drop "until <UTC time>": "Saved copy until" beside them gives it in local time. The Env tab's `VariableDialog` starts a new variable with no kind (`initialKind`, which the watcher's chooser sets to `secret`), with two radio buttons under "Kind"; Save stays disabled until one is picked.

**Files:**
- Modify: `crates/dots-core/src/vault/org/service.rs`, `apps/dots-app/src/contract.rs`, `ui/src/api/__fixtures__/contract.json`, `ui/src/screens/dot-editor/{DotForm,ToolsAndSkills,WatchersTab}.tsx`, `ui/src/screens/library/{EnvTab,McpServersTab}.tsx`
- Test: `crates/dots-core/tests/vault_orgs_test.rs`, `ui/src/screens/dot-editor/ToolsAndSkills.test.tsx`, `ui/src/screens/library/LibraryScreen.test.tsx`

**Interfaces:**
- Consumes: `DotForm`'s `orgId`; `EnvEntry["kind"]`.
- Produces: `ToolsAndSkills({ …, orgId?: string | null })`; `VariableDialog({ …, initialKind?: EnvEntry["kind"] })`.

- [ ] **Step 1: Write the failing tests**

**In `crates/dots-core/tests/vault_orgs_test.rs`, replace:**

```rust
        format!(
            "offline ({} is sealed or not ready (HTTP 503)); using the saved copy until {}",
            h.bao.address(),
            start_plus(24)
        )
```

**with:**

```rust
        format!(
            "offline ({} is sealed or not ready (HTTP 503)); using the saved copy",
            h.bao.address()
        )
```

**In `crates/dots-core/tests/vault_orgs_test.rs`, replace:**

```rust
        summary.status_text,
        format!(
            "policy invalid: no secrets at secret/dots/env; using the saved copy until {}",
            start_plus(24)
        )
    );
```

**with:**

```rust
        summary.status_text,
        "policy invalid: no secrets at secret/dots/env; using the saved copy"
    );
```

**In `crates/dots-core/tests/vault_orgs_test.rs`, replace:**

```rust
    );
    assert_eq!(
        summary.status_text,
        format!("offline; using the saved copy until {}", start_plus(24))
    );
    assert_eq!(
```

**with:**

```rust
    );
    assert_eq!(summary.status_text, "offline; using the saved copy");
    assert_eq!(
```

**In `ui/src/screens/dot-editor/ToolsAndSkills.test.tsx`, replace:**

```tsx

function setup(opts: { uses?: Uses; engine?: EngineKind; library?: LibraryEntry[] } = {}) {
  backend.on("library_list", () => opts.library ?? sample.library);
```

**with:**

```tsx

function setup(
  opts: { uses?: Uses; engine?: EngineKind; library?: LibraryEntry[]; orgId?: string } = {},
) {
  backend.on("library_list", () => opts.library ?? sample.library);
```

**In `ui/src/screens/dot-editor/ToolsAndSkills.test.tsx`, replace:**

```tsx
    latest = uses;
    return <ToolsAndSkills uses={uses} engine={opts.engine ?? "claude"} onChange={setUses} />;
  }
```

**with:**

```tsx
    latest = uses;
    return (
      <ToolsAndSkills
        uses={uses}
        engine={opts.engine ?? "claude"}
        orgId={opts.orgId ?? null}
        onChange={setUses}
      />
    );
  }
```

**In `ui/src/screens/dot-editor/ToolsAndSkills.test.tsx`, replace:**

```tsx

  it("lists the variables the chosen servers and plugins need and the environment lacks", async () => {
    const needy = { ...pluginServer!, missing_variables: ["PDF_KEY"] } as LibraryEntry;
```

**with:**

```tsx

  it("lists the variables the chosen servers and plugins need and Personal lacks", async () => {
    const needy = { ...pluginServer!, missing_variables: ["PDF_KEY"] } as LibraryEntry;
```

**In `ui/src/screens/dot-editor/ToolsAndSkills.test.tsx`, replace:**

```tsx
    await screen.findByLabelText("github");
    expect(screen.queryByText(/Not set in Dots' environment/)).toBeNull();

```

**with:**

```tsx
    await screen.findByLabelText("github");
    expect(screen.queryByText(/Not in Personal/)).toBeNull();

```

**In `ui/src/screens/dot-editor/ToolsAndSkills.test.tsx`, replace:**

```tsx
    await user.click(screen.getByLabelText("github"));
    expect(screen.getByRole("status")).toHaveTextContent("Not set in Dots' environment: " + github!.missing_variables.join(", "));
    // a plugin's own server counts when its plugin is ticked
```

**with:**

```tsx
    await user.click(screen.getByLabelText("github"));
    expect(screen.getByRole("status")).toHaveTextContent(
      "Not in Personal: " + github!.missing_variables.join(", "),
    );
    // a plugin's own server counts when its plugin is ticked
```

**In `ui/src/screens/dot-editor/ToolsAndSkills.test.tsx`, replace:**

```tsx
    expect(screen.getByRole("status")).toHaveTextContent("PDF_KEY");
  });
```

**with:**

```tsx
    expect(screen.getByRole("status")).toHaveTextContent("PDF_KEY");
  });

  it("names no Personal gap for an account's dot, which reads the account", async () => {
    const { user } = setup({ library: [github!], orgId: "acme-eng" });
    await user.click(await screen.findByLabelText("github"));
    expect(screen.queryByText(/Not in Personal/)).toBeNull();
  });
```

**In `ui/src/screens/library/LibraryScreen.test.tsx`, replace:**

```tsx
    expect(within(row).getByText("stdio")).toBeVisible();
    expect(within(row).getByText("Missing variable: GITHUB_TOKEN")).toBeVisible();
    expect(within(row).getByText("Used by nightly-tests")).toBeVisible();
```

**with:**

```tsx
    expect(within(row).getByText("stdio")).toBeVisible();
    expect(within(row).getByText("Not in Personal: GITHUB_TOKEN")).toBeVisible();
    expect(within(row).getByText("Used by nightly-tests")).toBeVisible();
```

**In `ui/src/screens/library/LibraryScreen.test.tsx`, replace:**

```tsx
    expect(within(dialog).getByLabelText("Value")).toHaveAttribute("type", "password");
    await user.type(within(dialog).getByLabelText("Name"), "1BAD");
```

**with:**

```tsx
    expect(within(dialog).getByLabelText("Value")).toHaveAttribute("type", "password");
    // Neither kind is chosen for the person: Save waits for one.
    expect(within(dialog).getByLabelText("Plain")).not.toBeChecked();
    expect(within(dialog).getByLabelText("Secret")).not.toBeChecked();
    expect(within(dialog).getByRole("button", { name: "Save" })).toBeDisabled();
    await user.click(within(dialog).getByLabelText("Secret"));
    await user.type(within(dialog).getByLabelText("Name"), "1BAD");
```

**In `ui/src/screens/library/LibraryScreen.test.tsx`, replace:**

```tsx
    const dialog = await screen.findByRole("dialog", { name: "Add variable" });
    await user.click(within(dialog).getByLabelText("Secret"));
    expect(within(dialog).getByLabelText("Value")).toHaveAttribute("type", "text");
```

**with:**

```tsx
    const dialog = await screen.findByRole("dialog", { name: "Add variable" });
    await user.click(within(dialog).getByLabelText("Plain"));
    expect(within(dialog).getByLabelText("Value")).toHaveAttribute("type", "text");
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core -j 2 --test vault_orgs_test -- --test-threads 4`, then `(cd ui && DEBUG_PRINT_LIMIT=0 bun run test --maxWorkers=2 src/screens/dot-editor/ToolsAndSkills.test.tsx src/screens/library/LibraryScreen.test.tsx)`
Expected: three `vault_orgs_test` status assertions FAIL (the text still ends `until 2026-10-23T09:00:00.000Z`); the UI tests FAIL on `Not in Personal` and on `getByLabelText("Plain")`.

- [ ] **Step 3: Implement**

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
            },
            "offline (connection refused); using the saved copy until 2026-10-10T12:00:00.000Z",
        ),
```

**with:**

```rust
            },
            "offline (connection refused); using the saved copy",
        ),
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
                },
                "policy invalid: signature does not match; using the saved copy until \
                 2026-10-10T12:00:00.000Z; member list not accepted: the member list is not \
                 signed by the owner or an admin",
            )
```

**with:**

```rust
                },
                "policy invalid: signature does not match; using the saved copy; member list \
                 not accepted: the member list is not signed by the owner or an admin",
            )
```

**In `crates/dots-core/src/vault/org/service.rs`, replace:**

```rust
            (OrgStatus::Connected, _) => "connected".into(),
            (OrgStatus::Offline { until: Some(until) }, _) => {
                format!(
                    "{}; using the saved copy until {until}",
                    with_reason("offline")
                )
            }
```

**with:**

```rust
            (OrgStatus::Connected, _) => "connected".into(),
            // The time the copy holds until is the summary's `copy_until`, which the UI shows in
            // local time beside the status: the words don't repeat it in UTC.
            (OrgStatus::Offline { until: Some(_) }, _) => {
                format!("{}; using the saved copy", with_reason("offline"))
            }
```

**In `crates/dots-core/src/vault/org/service.rs`, replace:**

```rust
            (OrgStatus::Revoked, _) => with_reason("access revoked"),
            (OrgStatus::PolicyInvalid { error }, Some(until)) => {
                format!("policy invalid: {error}; using the saved copy until {until}")
            }
```

**with:**

```rust
            (OrgStatus::Revoked, _) => with_reason("access revoked"),
            (OrgStatus::PolicyInvalid { error }, Some(_)) => {
                format!("policy invalid: {error}; using the saved copy")
            }
```

**In `ui/src/screens/dot-editor/DotForm.tsx`, replace:**

```tsx
          engine={spec.engine}
          onChange={(uses) => set("uses", uses)}
```

**with:**

```tsx
          engine={spec.engine}
          orgId={orgId}
          onChange={(uses) => set("uses", uses)}
```

**In `ui/src/screens/dot-editor/ToolsAndSkills.tsx`, replace:**

```tsx
  onChange: (uses: Uses) => void;
}) {
```

**with:**

```tsx
  onChange: (uses: Uses) => void;
  /// The dot's account; null or absent for Personal. An account's dot reads its variables from
  /// the account, which Variables shows per grant, so Personal's gaps are not named here.
  orgId?: string | null;
}) {
```

**In `ui/src/screens/dot-editor/ToolsAndSkills.tsx`, replace:**

```tsx
      ))}
      {missing.length > 0 && (
        <p role="status" className="text-xs text-needs">
          Not set in Dots' environment: {missing.join(", ")}. A server that needs one cannot start
          until it is set.
        </p>
```

**with:**

```tsx
      ))}
      {missing.length > 0 && !props.orgId && (
        <p role="status" className="text-xs text-needs">
          Not in Personal: {missing.join(", ")}. A server that needs one cannot start until it
          is set.
        </p>
```

**In `ui/src/screens/dot-editor/WatchersTab.tsx`, replace:**

```tsx
          initialName={props.initialName}
          onSaved={(saved) => props.onChange(saved.name)}
```

**with:**

```tsx
          initialName={props.initialName}
          initialKind="secret"
          onSaved={(saved) => props.onChange(saved.name)}
```

**In `ui/src/screens/library/EnvTab.tsx`, replace:**

```tsx
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { errorMessage, notifyError } from "@/lib/notify";
```

**with:**

```tsx
import { Input } from "@/components/ui/input";
import { errorMessage, notifyError } from "@/lib/notify";
```

**In `ui/src/screens/library/EnvTab.tsx`, replace:**

```tsx
  initialName?: string;
  /// Gets what the core saved.
```

**with:**

```tsx
  initialName?: string;
  /// The kind a new variable starts with, when the caller knows it (a watcher's app password
  /// is a secret); otherwise the person picks one.
  initialKind?: EnvEntry["kind"];
  /// Gets what the core saved.
```

**In `ui/src/screens/library/EnvTab.tsx`, replace:**

```tsx
  const [name, setName] = useState(entry?.name ?? props.initialName ?? "");
  const [secret, setSecret] = useState(entry ? entry.kind === "secret" : true);
  const [value, setValue] = useState(entry?.kind === "plain" ? (entry.value ?? "") : "");
```

**with:**

```tsx
  const [name, setName] = useState(entry?.name ?? props.initialName ?? "");
  // A new variable has no kind until the person picks one (smoke follow-up of plan B): a
  // secret can't be made plain later, and a plain value is shown.
  const [kind, setKind] = useState<EnvEntry["kind"] | null>(
    entry ? entry.kind : (props.initialKind ?? null),
  );
  const secret = kind === "secret";
  const [value, setValue] = useState(entry?.kind === "plain" ? (entry.value ?? "") : "");
```

**In `ui/src/screens/library/EnvTab.tsx`, replace:**

```tsx
    name: entry || validVariableName(name) ? undefined : NAME_RULE,
    value: secret && value === "" ? "A secret needs a value." : undefined,
```

**with:**

```tsx
    name: entry || validVariableName(name) ? undefined : NAME_RULE,
    kind: kind === null ? "Choose Plain or Secret." : undefined,
    value: secret && value === "" ? "A secret needs a value." : undefined,
```

**In `ui/src/screens/library/EnvTab.tsx`, replace:**

```tsx
    setTried(true);
    if (errors.name || errors.value) return;
    setState({ busy: true });
```

**with:**

```tsx
    setTried(true);
    if (errors.name || errors.kind || errors.value) return;
    setState({ busy: true });
```

**In `ui/src/screens/library/EnvTab.tsx`, replace:**

```tsx
        ? await commands.envSet(entry.name, value)
        : await commands.envAdd(name, secret ? "secret" : "plain", value);
      invalidate(qc, keys.env);
```

**with:**

```tsx
        ? await commands.envSet(entry.name, value)
        : await commands.envAdd(name, kind ?? "secret", value);
      invalidate(qc, keys.env);
```

**In `ui/src/screens/library/EnvTab.tsx`, replace:**

```tsx
            </Field>
            <div className="flex items-center gap-2">
              <Switch id="env-secret" checked={secret} onCheckedChange={setSecret} />
              <Label htmlFor="env-secret">Secret</Label>
              <span className="text-xs text-muted-foreground">
                {secret
                  ? "Kept in Windows Credential Manager and never shown again."
                  : "Plain config: shown here and editable."}
              </span>
            </div>
          </>
```

**with:**

```tsx
            </Field>
            <fieldset className="flex flex-col gap-1">
              <legend className="text-sm font-medium">Kind</legend>
              <div className="flex items-center gap-4">
                {(["plain", "secret"] as const).map((k) => (
                  <label key={k} className="flex items-center gap-1.5 text-sm">
                    <input
                      type="radio"
                      name="env-kind"
                      value={k}
                      checked={kind === k}
                      onChange={() => setKind(k)}
                    />
                    {k === "plain" ? "Plain" : "Secret"}
                  </label>
                ))}
              </div>
              <span className="text-xs text-muted-foreground">
                {kind === "secret"
                  ? "Kept in Windows Credential Manager and never shown again."
                  : kind === "plain"
                    ? "Plain config: shown here and editable."
                    : "A secret is never shown again and can't be made plain later."}
              </span>
            </fieldset>
          </>
```

**In `ui/src/screens/library/EnvTab.tsx`, replace:**

```tsx
            id="env-value"
            type={secret ? "password" : "text"}
            autoComplete="off"
```

**with:**

```tsx
            id="env-value"
            type={kind === "plain" ? "text" : "password"}
            autoComplete="off"
```

**In `ui/src/screens/library/EnvTab.tsx`, replace:**

```tsx
          </Button>
          <Button type="button" disabled={state.busy} onClick={() => void save()}>
            Save
```

**with:**

```tsx
          </Button>
          <Button
            type="button"
            disabled={state.busy || errors.kind !== undefined}
            onClick={() => void save()}
          >
            Save
```

**In `ui/src/screens/library/McpServersTab.tsx`, replace:**

```tsx
          <Badge variant="secondary" className="border-needs-border bg-transparent text-needs">
            {`Missing variable: ${entry.missing_variables.join(", ")}`}
          </Badge>
```

**with:**

```tsx
          <Badge variant="secondary" className="border-needs-border bg-transparent text-needs">
            {`Not in Personal: ${entry.missing_variables.join(", ")}`}
          </Badge>
```

Then regenerate the fixture (`UPDATE_CONTRACT=1 cargo test -p dots-app contract -j 2 -- --test-threads 4`; two status texts change).

- [ ] **Step 4: Run them to see them pass**

Run the same commands, then `(cd ui && DEBUG_PRINT_LIMIT=0 bun run test --maxWorkers=2 src/screens/library src/screens/dot-editor src/screens/Accounts.test.tsx src/api)` and the typecheck.
Expected: `test result: ok. 47 passed`; `Test Files  18 passed (18)`; typecheck clean.

- [ ] **Step 5: Check and commit**

Run `cargo fmt --check`, then:

```bash
git add apps/dots-app/src/contract.rs crates/dots-core/src/vault/org/service.rs crates/dots-core/tests/vault_orgs_test.rs ui/src/api/__fixtures__/contract.json ui/src/screens/dot-editor/DotForm.tsx ui/src/screens/dot-editor/ToolsAndSkills.test.tsx ui/src/screens/dot-editor/ToolsAndSkills.tsx ui/src/screens/dot-editor/WatchersTab.tsx ui/src/screens/library/EnvTab.tsx ui/src/screens/library/LibraryScreen.test.tsx ui/src/screens/library/McpServersTab.tsx
git commit -m "fix(ui): Personal gaps say Not in Personal and leave account dots alone, the offline status has no UTC time, and Add makes you pick Plain or Secret"
```

---

### Task 14: The admin guide covers audit records, the spec says what plan C built, and the owner's smoke steps

`docs/vault-admin.md` gets the `audit` field's meaning, the `--out` rule, the new status texts, the chained-not-signed limit and a new section 11 "Audit records". The spec gets §17 "Changes made while building (plan C)" and §15's note. `docs/verification/2026-11-06-env-vault-c-smoke.md` lists the owner's steps with a real build and a local sink.

**Files:**
- Create: `docs/verification/2026-11-06-env-vault-c-smoke.md`
- Modify: `docs/vault-admin.md`, `docs/superpowers/specs/2026-10-21-env-vault-design.md`

- [ ] **Step 1: Write the docs**

**In `docs/superpowers/specs/2026-10-21-env-vault-design.md`, replace:**

```markdown

  *Settled for stages 1 and 2: the split into plans A, B and C (§14), and the service calls, crates and zeroizing type in §14 and §16. Stage 3 is still open.*
- The UI layout details within the screens in §9, following the existing Library and dot editor patterns. *Settled for the screens plans A and B built (§14, §16).*
```

**with:**

```markdown

  *Settled for stages 1 and 2: the split into plans A, B and C (§14), and the service calls, crates and zeroizing type in §14 and §16. Stage 3 is plan C (§17).*
- The UI layout details within the screens in §9, following the existing Library and dot editor patterns. *Settled for the screens plans A and B built (§14, §16).*
```

**In `docs/superpowers/specs/2026-10-21-env-vault-design.md`, replace:**

```markdown
- **Not in plan B:** audit records, the outbox and `dots-admin audit verify` (plan C); OIDC login; macOS and Linux.
```

**with:**

```markdown
- **Not in plan B:** audit records, the outbox and `dots-admin audit verify` (plan C); OIDC login; macOS and Linux.

## 17. Changes made while building (plan C)

Plan C covers audit records, the outbox, sending and `dots-admin audit verify` (§10, §12), the History list, and the findings plans A and B left open. The owner's answers (2026-11-06) and the prototype settled these:

- **Names.** The core module is `vault::audit`:
  - `record` (`AuditRecord`, `AuditUser`, `AuditDot`, `Dropped`, `Event`, `canonical`, `line_hash`, `stamp`, the `REASON_` codes)
  - `verify` (`verify_lines`, `ChainReport`, `ChainSummary`, `Finding`)
  - `writer` (`Audit`, `OrgAudit`, `Usage`, `Happened`)
  - `sender` (`AuditSender`, `AuditTransport`, `HttpTransport`, `SendState`, `retry_after`)
  - `view` (`AuditView`)

  The store has `audit_chain`, `audit_outbox` and `audit_history` (migration `0037_audit.sql`, after master's `0036_intents.sql`). The new commands are `org_audit` and `org_leftover_keys`. The account page's new tab is "History".
- **The record.** As §10's example, plus a top-level `install` (the install id, which keys the chain with `org_id`) and `user.dots_id` (the Dots ID fingerprint), and `dropped` (`{from, to}`) on a full queue's `expired` record. `user.source_identity` is an OpenBao login's user name, an Infisical client id or this PC's SOPS age public key; `null` for an OpenBao token. Every key is always present (`null` when it says nothing).
- **The line and the chain.** A line is canonical JSON: the keys of every object sorted by their bytes, no white space. `prev` is `sha256:` and the hex SHA-256 of the previous line's bytes; `null` for seq 1. A receiver hashes the lines as it got them.
- **Reasons are codes, never text.** `denied` gives the `Unresolved` code (`not_granted`, `not_defined`, `denied_by_policy`, `denied_by_workspace`, `shell_secrets_forbidden`, `org_offline`, `org_expired`, `org_revoked`, `policy_invalid`, `org_removed`, `not_member`); `revoked` gives `denied` (the source refused) or `removed` (off the member list); `expired` gives `offline_copy` or `queue_full`. The other events have none.
- **What is recorded.** A run's start records each name it asks for, for each MCP server, the shell and the retry after a sign-in refresh: those that resolve, and the first that does not (later names are not asked for). `source` is `personal` when the value came from Personal. A Library Check or probe in an account records with `dot` and `run_id` null. The dot editor's status reads record nothing. Each fetch of a joined org records its outcome: Refresh, the timers, the start-up fetch, a run start's refresh, Join and Update when they work, and an expiry (also one found at start). A Join or an Update that fails records nothing: the account did not change.
- **An account without `audit`** queues nothing and spends no `seq`; its records are only listed in History (`local`). A policy that turns `audit` on queues from then on.
- **History** is `audit_history`: this PC's records, newest first, 30 days and at most 1,000 rows per account, with no `fetched` (the queue still sends them). Each row says `local`, `queued`, `sent` or `dropped`. It holds no value. It is deleted when the account is removed.
- **The queue keeps at most 100,000 records per account.** Reaching it drops the oldest 1,000 at once and writes one `expired` record (`queue_full`, `dropped`) naming them.
- **Remove keeps the queue and the chain.** They hold no values. Joining the same account again sends what is queued and goes on from the next `seq`.
- **Sending.** One sender for the app. Every org's queue goes to its policy's endpoint, at most 500 records a `POST`, `Content-Type: application/x-ndjson`, `Authorization: Bearer <token_entry's value>`, with the vault's HTTP client (20 seconds, no redirects). A 2xx deletes the batch. Anything else waits 30 seconds, doubling to 1 hour, each wait ±20%, reset by a 2xx. New records go 5 seconds after they are written. With no value for the token entry (the account gives none), the try is not counted against the wait: the next record written tries again. The answer's body and the token are never logged.
- **The endpoint** may be `http://` for `localhost` and `127.0.0.1`, as a join code's address, so a sink on the same PC can take records.
- **A failed send shows on the account page only**: Overview says "Audit: <n> records waiting; last send failed <time>: <why>", and History says it at its top. No Inbox item or notification.
- **`dots-admin audit verify <records.ndjson>`** uses the core's `verify_lines`. Per org and install it reports gaps, a record changed after it (`prev` no longer matches), a seq out of order and another record with a seq read before; a line sent twice is counted, not reported. A gap the PC's own `queue_full` record names is reported but is no failure. It exits 0 when every chain is whole.
- **Findings of plans A and B, folded in.**
  - A run's start no longer waits behind a fetch under way when the org is fresh; a stale org still waits for it and does not fetch again.
  - The start-up fetches stop when the app shuts down (they belong to one `JoinSet`).
  - A Remove that fails after the values are gone keeps the account listed, giving no values ("removing it did not finish (…); remove it again"), so it can be removed again.
  - Sign-in details kept for an account that is not joined (a SOPS key made for a join that never finished) are listed in Settings › Accounts by id, with **Remove** (`org_leftover_keys`).
  - `dots-admin`: an `--out` or `--file` never replaces a file the command reads, or a minisign key file. A key file whose write fails is deleted.
  - The dot editor says "Not in Personal" (not "Not set in Dots' environment"), and only for a Personal dot: an account's dot reads its variables from the account, which Variables shows. The Library's server row says "Not in Personal" too.
  - An org's status says "using the saved copy" without the time in UTC: "Saved copy until" beside it gives it in local time.
  - The Env tab's **Add** has no kind chosen: the person picks Plain or Secret before **Save**. A watcher's "Add to vault…" starts as Secret.
- **Not in plan C:** signing records per device, and the admin server (project 2); OIDC login; macOS and Linux.
```

**In `docs/vault-admin.md`, replace:**

```markdown
| `offline_hours` | `24` | How long the saved copy lasts after the last good fetch. |
| `audit` | `null` | Read and checked, but not used yet. |

```

**with:**

```markdown
| `offline_hours` | `24` | How long the saved copy lasts after the last good fetch. |
| `audit` | `null` | Where each PC sends its audit records (section 11). |

```

**In `docs/vault-admin.md`, replace:**

```markdown
**`audit`** is either `null` or `{"endpoint": "https://...", "token_entry": "NAME"}`. The
endpoint must be an `https://` URL and the entry a variable name. Dots checks it and does
nothing else with it yet. A later release will send audit records there.

```

**with:**

```markdown
**`audit`** is either `null` or `{"endpoint": "https://...", "token_entry": "NAME"}`. The
endpoint must be an `https://` URL (`http://` only for `localhost` or `127.0.0.1`, for a test
sink on the same PC), and the entry a variable name. With it, each PC sends its audit records to
the endpoint, with the entry's value as the bearer token (section 11). Without it, nothing is
sent; each person still sees their own records in the account's History.

```

**In `docs/vault-admin.md`, replace:**

```markdown
signed text is one line of JSON. Store it in the source as section 4 says.

```

**with:**

```markdown
signed text is one line of JSON. Store it in the source as section 4 says.

An `--out` (or `join-code --file`) never replaces a file the command reads (the policy, the key)
or any minisign key file: `dots-admin` says so and writes nothing. Another file there is
replaced.

```

**In `docs/vault-admin.md`, replace:**

```markdown
- `connected`: the last fetch worked.
- `offline (<why>); using the saved copy until <time>`: the source can't be reached. Runs use
  the copy.
- `offline (<why>), with no saved copy`: the source can't be reached, and there is no copy. Runs
```

**with:**

```markdown
- `connected`: the last fetch worked.
- `offline (<why>); using the saved copy`: the source can't be reached. Runs use the copy.
  "Saved copy until", beside it, says until when, in the PC's local time.
- `offline (<why>), with no saved copy`: the source can't be reached, and there is no copy. Runs
```

**In `docs/vault-admin.md`, replace:**

```markdown
- `access revoked (<why>)`: the source refused the person. Their copy is gone.
- `policy invalid: <error>; using the saved copy until <time>`: the fetch, or its policy, was
  refused. Runs use the last good copy.
- `policy invalid: <error>`: the same, with no copy left.
```

**with:**

```markdown
- `access revoked (<why>)`: the source refused the person. Their copy is gone.
- `policy invalid: <error>; using the saved copy`: the fetch, or its policy, was refused.
  Runs use the last good copy.
- `policy invalid: <error>`: the same, with no copy left.
```

**In `docs/vault-admin.md`, replace:**

```markdown
  Dots.
- **Audit records are not sent yet.** The policy's `audit` field is checked, and a later release
  will use it. Until then, your source's own audit log is the record of who fetched what.
- **Windows only.** Dots keeps secrets in Windows Credential Manager and DPAPI.
```

**with:**

```markdown
  Dots.
- **Audit records are chained, not signed.** A gap or an edit after sending shows (section 11);
  which PC wrote a record is not proven, since that needs a key per device. Your source's own
  audit log stays the record of who fetched what.
- **Windows only.** Dots keeps secrets in Windows Credential Manager and DPAPI.
```

**In `docs/vault-admin.md`, replace:**

```markdown
- A stolen PC while its Windows login is unlocked.
```

**with:**

````markdown
- A stolen PC while its Windows login is unlocked.

## 11. Audit records

With `audit` in the policy, every PC in the account keeps a record of each variable a dot of the
account asks for, and of each fetch of the source, and sends them to the endpoint. A record never
holds a value. The format, the request and every field are in
[`vault-audit-format.md`](vault-audit-format.md); that is what your receiver implements.

- **What is recorded.** For each name a run asks for, at its start, for each MCP server and for
  the shell: `resolved`, or `denied` with the reason. A Library **Check** in the account records
  too, without a dot. Each fetch records `fetched`, `offline`, `revoked`, `expired` or
  `policy_invalid`. Personal dots record nothing.
- **The token.** `token_entry` names an ordinary entry of the account, for example
  `DOTS_AUDIT_TOKEN`. Every member's Dots reads it to send; grant it to no dot. A PC that gives
  no values (access revoked, offline with no copy, not on the member list) keeps its records
  until it has the token again.
- **Sending.** Records go about 5 seconds after they are written, at most 500 to a request, as
  `application/x-ndjson`. A `2xx` takes them off the PC's queue. Anything else keeps them: the
  next try waits 30 seconds, then twice as long each time, up to an hour. A batch can arrive
  twice when your `2xx` is lost: keep records by account, install and `seq`.
- **Limits.** Each PC queues at most 100,000 records per account. Past that, the oldest 1,000 are
  dropped and an `expired` record with reason `queue_full` names them.
- **Leaving.** A person who removes the account keeps its queue: joining again sends it and the
  numbering goes on.
- **What the person sees.** The account's **History** tab lists that PC's records of the last 30
  days (at most 1,000; successful fetches are left out), and whether each was sent. While sending
  fails, Overview says how many wait and why.

To check what your endpoint received:

```bash
dots-admin audit verify records.ndjson
```

It checks, for each account and install, that `seq` counts up by one and that each `prev` is the
hash of the record before, and names each gap:

```text
acme-eng (01J9…): 1240 records, seq 1 to 1240
ok: every chain is whole
```

A gap the PC's own `queue_full` record explains is named but not a failure. The command exits 1
when a chain is not whole.
````

**Create `docs/verification/2026-11-06-env-vault-c-smoke.md`:**

```markdown
# Env vault plan C smoke verification (the owner's)

These steps are for the owner, with a real build. No agent ran them: the plan's end-to-end test (`crates/dots-core/tests/runtime_audit_test.rs`) covers the same path against a local sink with the fake CLI. Use a fresh `DOTS_DATA_DIR`, never `%LOCALAPPDATA%\dots`. Write no value into this file.

## B1. A local sink

Run any HTTP receiver on this PC that keeps each request's headers and body and answers `200`, for example a small Node or Python script listening on `http://127.0.0.1:8787/dots`. Make it able to answer `503` on demand.

## B2. An account that sends records

1. With `dots-admin`, make a key and a policy whose `audit` is `{"endpoint": "http://127.0.0.1:8787/dots", "token_entry": "DOTS_AUDIT_TOKEN"}`. Sign it.
2. In the source (an OpenBao dev server or a SOPS file), add the entry `DOTS_AUDIT_TOKEN` with a made-up value, and one entry a dot uses.
3. Join the account in Settings › Accounts.

Expect: within about 5 seconds the sink gets one `POST` with `Content-Type: application/x-ndjson`, `Authorization: Bearer <the made-up token>`, and one line: a `fetched` record with `seq` 1 and `prev` `null`.

## B3. A run's records

Grant the entry to a dot of the account (its shell, or an MCP server), and run it.

Expect: the sink gets the `resolved` records with the dot's id and name and the run id. The account's **History** tab lists them as "Used", then "Sent". No line holds a value.

## B4. A refused send

Make the sink answer `503`, run the dot again.

Expect: Overview shows "Audit: <n> records waiting; last send failed <time>: the endpoint answered 503", and History shows "Waiting to send". Make the sink answer `200` again: within about a minute the records arrive and the line goes.

## B5. Verify what arrived

Save the bodies the sink got, one record per line, as `records.ndjson`, and run `dots-admin audit verify records.ndjson`.

Expect: one line for the account and install, then `ok: every chain is whole`. Delete one line from the middle and run it again: it names the missing record and exits 1.

## B6. Without audit

Sign a policy without `audit` and Update the account. Run the dot.

Expect: the sink gets nothing more; History lists the run's records as "Kept on this PC".

## B7. The folded fixes

- In Env, **Add variable**: neither Plain nor Secret is chosen, and **Save** waits for one.
- Stop the source while Dots runs: the status says "offline (…); using the saved copy", and "Saved copy until" gives the local time.
- In the dot editor of an account's dot, ticking a server whose variable only the account has shows no "Not in Personal" line.
```

- [ ] **Step 2: Check them against the code**

Read §17 and the guide's section 11 against Tasks 1 to 13 as built (the execution's fix rounds included): every number, text and name must be the shipped one. Where a fix round changed something the plan's blocks say, the docs say what shipped.

- [ ] **Step 3: Run the whole suite**

Run `cargo test --workspace -j 2 -- --test-threads 4`, `cargo clippy --workspace --all-targets -j 2 -- -D warnings`, `cargo fmt --check`, the whole UI suite and the typecheck.
Expected: core 2672, app 197, admin 40 passed; clippy and fmt clean; UI 142 files, 1544 tests; typecheck clean.

- [ ] **Step 4: Commit**

```bash
git add docs/vault-admin.md docs/superpowers/specs/2026-10-21-env-vault-design.md docs/verification/2026-11-06-env-vault-c-smoke.md
git commit -m "docs: the admin guide covers audit records, the spec says what plan C built, and the owner's smoke steps"
```
