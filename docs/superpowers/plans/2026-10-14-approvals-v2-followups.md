# Approvals v2: follow-ups

What the execution of `2026-10-13-approvals-v2.md` left open (PR #5, merged as `9993280`). Every item comes from a task review, the smoke or the whole-branch review. The final reviewer triaged all of them as "can wait": none runs a call twice, widens a policy or loses a recorded outcome. They are a starting list for the next plan, not decisions.

## Decisions to take

- **How performed calls are reported to the model.** The report is a user message that quotes tool output, with a sentence saying the outputs are not instructions (`crates/dots-core/src/engine/local/mod.rs`). A file read by an approved `Read` or `Grep` can still contain text shaped like a report line. The stronger form is a synthetic assistant tool call followed by tool results, which the spec set aside because it writes a message in the model's name. A per-report delimiter would be a smaller step.
- **A dot moved to another engine while a run is parked.** Its queued calls are given up (`skipped`). `update_dot` could refuse to change `engine` while the dot has a run that is queued, running or awaiting approval, as it already does for `workdir`.

## Behaviour

- The model is not told when queued calls are given up because the dot's engine changed. When every decision was a self-apply approval, the child's prompt is only "Continue the task…" (`runner.rs`, near the prompt build). Add a "not performed, request again" line for each skipped approval.
- Approved calls can run in a different folder than the one approved. This happens only for a folder-mode dot whose recorded folder was deleted and whose `workdir` was changed. The calls stay confined to that folder. Fix: hand over no `approved_calls` when the prepared path is not the run's recorded `workspace_path` (`runner.rs`, workspace selection).
- A claim whose update committed but whose announcement failed: the model is told "not performed", but the row stays `applying` and is closed as `unknown`. The UI then says "Dots stopped while this ran" for a call that never ran. The cause is that `ApprovalHub` re-reads the row after the write and returns `Err` if that read fails (`approvals.rs`, `announce_update`). The same applies to `settle_apply` (the run fails and its ToolResult is dropped) and `skip_apply` (only logged). Fix: treat the re-read as best effort, like the bus send.
- After a cancel or a stop mid-apply, the report is discarded. Calls already settled `applied` never reach the stored conversation, so a later chat follow-up does not know they were done (spec-level choice).
- The report repeats each call's full input JSON, so a large `Write` or `Edit` is sent to the model twice. On small contexts (the smoke ran 8192 tokens) a preview would be kinder.
- The dot editor:
  - It can unmount while open, dropping typed edits, when a later refetch of its dot runs without `setQueryData`. One trigger is `invalidate(qc)` after a lagged event stream (`ui/src/lib/runtime-events.ts`). Fix: apply the `refreshing` check only before the editor first mounts, for example with a `useRef` latch (`ui/src/screens/DotEditorScreen.tsx`).
  - If the refetch fails as it opens, it fills from the old copy with no error. This was already so before Approvals v2.
  - The core could refuse an `update_dot` whose `updated_at` differs from the copy the editor loaded.
- `runner.rs` maps a `Conflict` from `resume_parked` to "nothing to resume" without a log line. No current path reaches it; a `tracing::warn!` would show a future caller bug.
- `approvals_for_run` and `unresolved_parked_decisions` still order by `created_at, id`. Same-millisecond calls can show in a different order in the run view than in the Inbox.
- The Inbox batch dialog, once open, still lists a call that was decided from its own card meanwhile. The core leaves that call out, so only the dialog text is stale.
- `ApprovalHub` and `Store::approve_with_rule` accept any `Rule`; only `Runtime::decide_approval` checks it against `allow_candidates`. Consider `pub(crate)` or a doc line naming the one allowed entry point.
- `Engine::settle_apply`'s trait default returns `Ok` for an unclaimed call, while the hub returns `Conflict`.
- `affects_inbox` is true for `ApprovalUpdated`, so the tray count is refreshed on every claim and settle (`apps/dots-app/src/events.rs`).

## Rule candidates (`policy::allow_candidates`)

- `git -c a=b log` still offers `Bash(git -c a=b:*)`, which matches `git -c a=b -c core.fsmonitor=<prog> status`. Rule (b) looks only at the second token. Option: offer no prefix at all when the second token is an option.
- A `/` second token also drops prefixes for plain path arguments (`ls /tmp/a /tmp/b` gets only the exact rule).
- A shared-tenant host offers every tenant: `s3.amazonaws.com` or `github.io` gives "this host and its subdomains". A write outside the workspace offers its parent directory, for example `C:/Users/x/**`. Labels are accurate. Option: offer no directory rule outside the workspace.
- `Glob(<dir>)` is matched on `path` only, never on the Glob `pattern` (existing matcher). A file-writing tool missing from `FILE_TOOLS`, such as `MultiEdit` if the CLI still has it, is offered its bare name.
- More commands that run their arguments could join `WRAPPER_COMMANDS` as they are found. One option: treat a second token that is itself a wrapper as a wrapper.

## UI

- `ApprovalCard`:
  - `aria-controls` points at the rule fieldset while it is not mounted.
  - Cancel drops keyboard focus to `body`.
  - The "rules could not be loaded" note has no `role="status"`.
  - The fieldset is not disabled after the decision.
  - `picked` is not checked against the offered rules after a refetch (the core would refuse a stale one).
- `AppliedCalls` stands after the pending cards in the run view and before them in the chat.

## Tests

- No test for the panicked-engine close (`runner.rs`) or for cancelling a queued resume child that the dispatcher holds.
- Tests with weak assertions:
  - `a_run_cancelled_before_its_calls_are_reached_claims_and_performs_none`: its `tool_call` events assertion is vacuous (`start_by_hand` discards events).
  - The "no deny rule yields" check in `policy_test.rs` uses a bare deny rule, which is trivially true.
  - The 40-call order test in `store_runs_test.rs` may not distinguish `rowid` from `created_at, id` when ids are monotonic.
  - The migration test checks column defaults on an already-migrated database, and does not close its second pool.
- Tests that test less than their names say:
  - RunScreen "follows a call as the core announces what became of it" runs its loop once, with one run, so the parent-run refetch rests on the runtime-events test.
  - InboxScreen "offers no rule for a call the core has none for" asserts absence before the answer is applied.
- Timing-dependent tests:
  - `a_batch_given_while_the_run_still_works_is_carried_out_when_it_ends` (a 1500 ms window).
  - The RunScreen dot-name assertion right after a click.
  - Four older `resume_test` worktree tests failed once together under load (not reproduced).
  - One `DotEditorScreen` form test failed once in a full run.
- Missing tests: double click and keyboard use on the approval card; a failed decision with a rule leaving the dot's queries valid.

## Smaller

- `close_applies` reads changed rows back one by one.
- `rowid` order relies on SQLite keeping relative order across `VACUUM` (nothing runs `VACUUM`); a comment would say so.
- The `Approval` contract sample's workspace path deserves a comment (it matters only for file tools).
- The plan's note for fresh checkouts should say to build `ui/dist` before Task 1's Step 3, because `tsc` fails until the fixture is regenerated.

## Smoke

Part B of `docs/verification/2026-10-13-approvals-v2-smoke.md` (the person steps) is not run: a release build with the real data folder, closing Dots while an approved call runs, and Claude dots with the real `claude` CLI. The next agent smoke should start with the `claude` CLI off the `PATH`. The installed Dots held port 47321 during the smoke, so the debug app showed "local server is not running"; local dots were not affected.
