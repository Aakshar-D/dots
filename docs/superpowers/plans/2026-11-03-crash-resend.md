# Crash resend: plan

Spec: `docs/superpowers/specs/2026-11-03-crash-resend-design.md`. TDD: each task writes its failing test first. Build with `CARGO_TARGET_DIR=C:/dev/dots-target/approval-guards`.

### Task 1: Resend at recovery (store)

**Files:** `crates/dots-core/src/model.rs` (`NewRun::resend_of`), `crates/dots-core/src/store/runs.rs` (`recover_interrupted`), test `crates/dots-core/tests/store_resend_test.rs` (new).

- [ ] Step 1: failing tests: `an_interrupted_message_is_resent` (new chat and a reply: same payload plus `resent`, parent, root, session from the parent, `retry_of`); `other_triggers_are_not_resent`; `a_message_with_an_approved_call_is_not_resent`; `resending_stops_after_three_tries`; `recovering_twice_resends_once`; `a_step_chat_is_not_resent`.
- [ ] Step 2: run `cargo test -p dots-core --test store_resend_test`: FAIL.
- [ ] Step 3: implement.
- [ ] Step 4: run it and `--test store_runs_test`: PASS.
- [ ] Step 5: commit `feat(core): at start, a message Dots was answering when it stopped is sent again`.

### Task 2: Send again (runtime)

**Files:** `crates/dots-core/src/runtime.rs` (`Runtime::resend`), test `crates/dots-core/tests/runtime_resend_test.rs` (new).

- [ ] Step 1: failing tests: queues the resend with `retry_of` and no note; refuses a run that succeeded, a non-chat run, and one that is not the last of its topic.
- [ ] Step 2–4: run FAIL, implement, run PASS.
- [ ] Step 5: commit `feat(core): a failed message can be sent again`.

### Task 3: App command

**Files:** `apps/dots-app/src/commands.rs`, `apps/dots-app/src/lib.rs`, `ui/src/api/commands.ts`, regenerate contract if commands are listed there.

- [ ] Run `cargo test -p dots-app` (contract test against `commands.ts`). Commit `feat(app): resend_run`.

### Task 4: UI

**Files:** `ui/src/components/Transcript.tsx`, `ui/src/screens/home/TopicRuns.tsx`, tests next to them.

- [ ] Failing tests: the note shows for a resent run and not for others; Send again shows under a failed chat run and calls `resendRun`.
- [ ] Implement; `bunx vitest run` and `bunx tsc --noEmit`. Commit `feat(ui): the conversation says a message was resent, and offers Send again`.

### Task 5: Verify

`cargo test -p dots-core`, `cargo test -p dots-app`, `cargo clippy -p dots-core -p dots-app --all-targets -- -D warnings`, vitest, tsc.
