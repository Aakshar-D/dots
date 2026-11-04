# Attachments in chat, and a Files panel — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The person can attach files to a chat message: with the paperclip, by dropping them, or by pasting a screenshot. The dot finds them in `attachments/<topic>/` in its workspace for the rest of the conversation. A Files panel beside the conversation lists those files and the files the dot wrote or edited, and opens them or shows them in their folder.

**Architecture:** A file is staged when it is added. The core copies it into `<data>/chat-files/<id>/` and records a `chat_attachments` row (migration 0033). Sending links the rows to the new chat run in the same transaction that inserts the run, and writes `payload.attachments`. Before every run of a topic, the runner copies the topic's missing files into `<workspace>/attachments/<root>/` (with a `.gitignore` of `*`), and the prompt names this message's files. The Files panel reads the topic's attachments plus the `tool_call` events of its runs. The app opens a path only after the core has resolved it. A scheduled message (send later) holds its rows through `scheduled_id` until it sends.

**Tech Stack:** Rust (tokio, sqlx/SQLite), Tauri 2 (dialog, opener, webview drag-drop, raw IPC body), React + TypeScript (zod, vitest, TanStack Query).

**Spec:** `docs/superpowers/specs/2026-11-04-attachments-design.md` (§3.1 lists the builder's calls).

## Global Constraints

- Branch `feat/attachments`, stacked on `feat/send-later`. Migration `0033_chat_attachments.sql` with the SQL of spec §4.1. Fill in `<send-later table>` with the scheduled-messages table of 0032 as merged.
- `MAX_FILES = 10`, `MAX_FILE_BYTES = 25 * 1024 * 1024` (25 MB, 26 214 400 bytes). Exactly the limit is accepted; one byte more is refused.
- Stored copy: `<data_dir>/chat-files/<attachment id>/<file_name>`. Workspace copy: `<workspace>/attachments/<topic root id>/<name>`, with `<workspace>/attachments/<topic root id>/.gitignore` = `*\n`.
- Names: `crate::watchers::files::safe_name` when staged, then `unique_names` within the topic when sent. Names already linked in the topic and `.gitignore` count as taken, ignoring case. Pasted image: `Pasted image.<png|jpg|gif|webp>`, unless the clipboard file has a name other than `image.png`.
- A topic's files = rows whose `run_id` is a run of that topic and not (`dismissed_at` set and `delivered_at` null), ordered by the run's `queued_at`, then the row's id.
- Written-file tools: `Write`, `Edit`, `MultiEdit`, `NotebookEdit`, `write_file`, `edit_file`. Path from `input.file_path`, else `input.notebook_path`. Only paths inside the run's `workspace_path`.
- Programs never opened: `exe com bat cmd ps1 psm1 vbs vbe js jse wsf wsh msi msp scr hta lnk url cpl reg jar pif application gadget msc scf inf appref-ms` (extension, ignoring case).
- Core refusal texts, verbatim (all `Error::Invalid` unless noted):
  - `You can attach up to 10 files to a message.`
  - `<name> is larger than 25 MB.`
  - `<name> is a folder. Attach the files in it instead.`
  - `<name> is a link. Attach the file it points to instead.`
  - `<name> could not be read: <error>`
  - `Conflict`: `An attached file is no longer there. Remove it and attach it again.`
  - `Invalid`: `Dots doesn't open programs or scripts. Use Show in folder.`
  - `NotFound`: `this file is no longer there`
- Prompt block (verbatim, spec §4.4): `## Attached files\n\nThe person attached these files to this message. They are in your workspace and stay there for the rest of this conversation:\n\n- <absolute path> (<size>)\n` (one line per file). It follows the message text after a blank line, or is the whole message when the text is empty. The size is written like `formatBytes`.
- Run failure text when this message's own file cannot be placed: `could not put the attached files in the workspace: <error>`. Notice for an earlier message's file: `Dots could not copy <name> into the workspace: <error>`.
- UI copy is spec §6, verbatim. The pref key is `dots.home.files`, with value `"hidden"`.
- Rust builds on this machine need `C:\Program Files\LLVM\bin` on PATH. No AI trailer in commits. Never commit `CLAUDE.local.md` or `.cargo/config.toml`.
- Tests must not depend on same-millisecond ULID order: set `queued_at` with `set_run_queued_at`, and assert by id. They must not depend on a dispatcher claiming runs: call `rt.hold_runs(true)` first, or use a `Runner` without `spawn_dispatcher`. No real clock, locale or time zone in UI tests. Read files only after the run has ended (`wait_status`), never while an async writer may still run.

## Review Focus

1. **The dispatcher claims a message before its files are linked.** Linking and inserting the run happen in one transaction, so the first run of a new topic always finds its files. Test `a_message_is_never_claimed_without_its_files` (Task 2).
2. **Every chat run deletes its own topic's files.** Today `remove_attachments` removes `attachments/<root>/` after every run. Test `chat_files_survive_the_end_of_a_run` (Task 2).
3. **An unchanged worktree holding attachments**: it must still be cleaned up (they are git-ignored), and a reply must find the files again in the restored worktree. Test `a_reply_finds_its_files_in_a_restored_worktree` (Task 2).
4. **The agent turns `attachments` into a junction or link pointing outside the workspace**: no file is written through it, and the run fails with the copy text. Test `copying_refuses_a_linked_attachments_folder` (Task 2).
5. **Open with a path the UI made up, or a `.ps1` the dot wrote**: the core refuses both. Nothing is passed to the opener. Tests `a_path_not_in_the_list_is_not_opened` and `a_program_is_listed_but_not_opened` (Task 3).

---

### Task 1: Storage, staging and the startup sweep

**Files:**
- Create: `crates/dots-core/migrations/0033_chat_attachments.sql`
- Create: `crates/dots-core/src/chat_files.rs` (register in `lib.rs`)
- Create: `crates/dots-core/src/store/attachments.rs` (register in `store/mod.rs`)
- Modify: `crates/dots-core/src/model.rs` (new types, next to `NewRun` :341)
- Modify: `crates/dots-core/src/runtime.rs`: start, next to `watcher_files.sweep` :331–334; `delete_dot` :1335; new staging methods next to `chat` :1407
- Test: `crates/dots-core/tests/chat_files_test.rs` (new), `crates/dots-core/tests/store_attachments_test.rs` (new)

**Interfaces:**
- Produces:
  - `pub struct ChatAttachment { id, dot_id, run_id: Option<String>, scheduled_id: Option<String>, file_name, name: Option<String>, size: u64, created_at }`. `pub struct StagedAttachment { pub id: String, pub name: String, pub size: u64 }`. Both Serialize/Deserialize/Clone/Debug/PartialEq.
  - `chat_files.rs`: `pub const MAX_FILES: usize = 10`, `pub const MAX_FILE_BYTES: u64`, `pub const DIR: &str = "chat-files"`, `pub const IGNORE_FILE: &str = ".gitignore"`.
  - `pub struct ChatFiles { root: PathBuf }` (Clone, Debug), with:
    - `new(root) -> Self`;
    - `stored_path(&self, a: &ChatAttachment) -> PathBuf`;
    - `stage_path(&self, id: &str, from: &Path) -> Result<(String, u64)>`: returns the safe name and size, or one of the refusal texts;
    - `stage_bytes(&self, id: &str, name: &str, bytes: &[u8]) -> Result<(String, u64)>`;
    - `remove(&self, id: &str)` (logs on failure);
    - `async fn sweep(&self, store: &Store)`.
  - `pub fn human_size(bytes: u64) -> String` (the `formatBytes` rule) and `pub fn can_open(path: &Path) -> bool`.
  - Store: `insert_staged_attachment(&self, id, dot_id, file_name, size) -> Result<ChatAttachment>`, `get_attachment(&self, id) -> Result<ChatAttachment>` (`NotFound`), `remove_unsent_attachment(&self, id) -> Result<bool>` (only while `run_id` and `scheduled_id` are NULL), `topic_attachments(&self, root_run_id) -> Result<Vec<ChatAttachment>>` (the Global Constraints rule), `delete_unsent_attachments(&self) -> Result<Vec<String>>` (returns the ids), `attachment_ids_of_dot(&self, dot_id) -> Result<Vec<String>>`, `all_attachment_ids(&self) -> Result<HashSet<String>>`.
  - Runtime: `add_attachment_path(&self, dot_id: &str, path: &Path) -> Result<StagedAttachment>`, `add_attachment_bytes(&self, dot_id: &str, name: &str, bytes: &[u8]) -> Result<StagedAttachment>`, `remove_attachment(&self, id: &str) -> Result<()>` (`Conflict` once it is sent or scheduled). Each checks `get_dot` + `require_run`. The id is a new ULID. A failed insert removes the folder.
  - `Runtime::start` builds `ChatFiles::new(cfg.data_dir.join(DIR))`, awaits `sweep`, and hands it to the runner (`runner.set_chat_files`, Task 2). `delete_dot` reads `attachment_ids_of_dot` before `delete_dot_if_idle` and calls `remove` for each after it.

- [ ] **Step 1: Write the failing tests.**
  - `chat_files_test.rs` (tempdir, no store for the first four):
    - `a_file_at_the_limit_is_staged`: a file of exactly `MAX_FILE_BYTES` gives `size == MAX_FILE_BYTES`, and the stored file has equal bytes.
    - `a_file_one_byte_over_is_refused`: the error text is `big.bin is larger than 25 MB.`, and no folder is left.
    - `folders_links_and_missing_paths_are_refused`: a folder gives `docs is a folder. Attach the files in it instead.`; a symlink (skip the test with a note when creating one fails on Windows without the privilege) gives `… is a link. Attach the file it points to instead.`; a missing path gives text starting `nope.txt could not be read: `.
    - `bytes_get_a_safe_name`: `stage_bytes(id, "..\\..\\CON.png", b"x")` gives `_CON.png`, stored at `<root>/<id>/_CON.png`.
    - `human_size_matches_the_ui`: 240 → `240 B`, 2458 → `2.4 KB`, 1_572_864 → `1.5 MB`, 31_457_280 → `30 MB`.
    - `programs_cannot_be_opened`: `can_open("a.PS1") == false`, `can_open("r.pdf") == true`, and no extension → `true`.
  - `store_attachments_test.rs`:
    - `a_staged_file_can_be_removed_until_it_is_sent`: insert, then remove → `true`, and the row is gone. A row with `run_id` set → `false`.
    - `the_sweep_removes_unsent_files_and_stray_folders`: one unsent row with its folder, one sent row (linked with Task 2's store function, or by SQL in the test) with its folder, and one folder with no row. After `sweep`, only the sent one's row and folder remain.
    - `deleting_a_dot_removes_its_stored_files` (runtime, `hold_runs(true)`): stage, `delete_dot` → the row is gone (cascade) and the folder is gone.
    - `staging_checks_the_dot`: an unknown dot gives `NotFound`, and no folder is made.
- [ ] **Step 2: Run** `cargo test -p dots-core --test chat_files_test --test store_attachments_test`. Expected: compile errors.
- [ ] **Step 3: Implement.** The copy reads through `File::open(..).take(MAX_FILE_BYTES + 1)`. When it read more than the limit, it deletes and refuses. `symlink_metadata` comes first, so a link is never followed.
- [ ] **Step 4: Run** the two files, plus `runtime_test` and `store_dots_test`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): files added to a chat box are staged in Dots' data folder`.

### Task 2: Sending, the workspace copy and the prompt

**Files:**
- Modify: `crates/dots-core/src/store/attachments.rs` (`create_chat_run`)
- Modify: `crates/dots-core/src/store/runs.rs` (use `insert_run(&mut conn, new)` :131 inside the new transaction)
- Modify: `crates/dots-core/src/runtime.rs`: `chat` :1407 becomes `chat_inner`; `retry_delivery` :1501 (carry the files)
- Modify: `crates/dots-core/src/runner.rs`: `set_chat_files` next to `set_watcher_files` :376; `copy_chat_files` called after `copy_attachments` :1274; `remove_attachments` :414 (now async, watcher roots only; callers :832, :1348, :1534)
- Modify: `crates/dots-core/src/chat_files.rs` (`copy_into`, `prompt_block`)
- Modify: `crates/dots-core/src/prompt.rs` (`build_prompt_full` :173: the message at :189 gets the block, used in all three return paths)
- Test: `crates/dots-core/tests/runtime_attachments_test.rs` (new), `crates/dots-core/tests/engine_prompt_test.rs` (extend)

**Interfaces:**
- Consumes: Task 1.
- Produces:
  - `pub struct ChatSend<'a> { pub message: &'a str, pub parent_run_id: Option<&'a str>, pub attachment_ids: &'a [String], pub scheduled_id: Option<&'a str>, pub carry_from: Option<&'a str> }`.
  - `Runtime::chat_with_files(&self, dot_id, message, parent_run_id, attachment_ids: &[String]) -> Result<Run>` (public). `Runtime::chat` keeps its signature and calls it with `&[]`. Both go through `async fn chat_inner(&self, dot_id: &str, send: ChatSend<'_>) -> Result<Run>`, which is `pub(crate)` so send later (Task 6) can call it. Order in `chat_inner`:
    1. more than `MAX_FILES` ids → the count refusal;
    2. empty text and no files (and no `carry_from`) → today's `message must not be empty`;
    3. each row: `get_attachment`. It must have this `dot_id`, `run_id` NULL, `scheduled_id` NULL or equal to `send.scheduled_id`, and its stored file must be a file. Otherwise → the `Conflict` "no longer there" text;
    4. today's dot, limit and parent checks, unchanged, under `chat_lock`;
    5. `new.id = Some(ulid)`; root = parent's root, else that id; names = `unique_names` over (the root's linked names + `.gitignore` + these rows' `file_name`s), taking the tail;
    6. `payload.attachments = [{id, name, size}]` (all of the run's files, the carried ones included);
    7. `store.create_chat_run(&new, &links, send.scheduled_id, send.carry_from)`, then `runner.enqueued(&run)`.
  - `Store::create_chat_run(&self, new: &NewRun, links: &[(String /*id*/, String /*name*/)], scheduled_id: Option<&str>, carry_from: Option<&str>) -> Result<Run>`. One transaction: `insert_run`; `UPDATE chat_attachments SET run_id = ?, name = ?, scheduled_id = NULL WHERE id = ? AND run_id IS NULL AND (scheduled_id IS NULL OR scheduled_id = ?)` for each link, where any affected count of 0 → rollback and `Conflict`; when `carry_from` is set, `UPDATE chat_attachments SET run_id = <new> WHERE run_id = <carry_from>`. Commit, then `get_run`.
  - `retry_delivery`: calls `chat_inner` with `carry_from: Some(old.id)`, no new ids, and the old text. The carried files keep their names. The text may be empty when the old message had files.
  - `ChatFiles::copy_into(&self, root_run_id: &str, files: &[ChatAttachment], workspace: &Path) -> Vec<(String /*id*/, Error)>`: the safety rules of spec §4.4. It returns the files it could not place and copies only missing ones.
  - `pub fn prompt_block(files: &[(PathBuf, u64)]) -> String` (the Global Constraints text). `build_prompt_full` reads `payload.attachments` and builds each path as `workspace.join("attachments").join(&run.root_run_id).join(name)`.
  - Runner `copy_chat_files(&self, run: &Run, workspace: &Path) -> Result<()>`: `topic_attachments(root)` → `copy_into`. A failed file whose row belongs to `run.id` → `Err(Error::Other("could not put the attached files in the workspace: <first error>"))` (the run fails before its engine starts). Others → `record(Notice { text })` with the notice text.
  - `remove_attachments(&self, workspace: &Path, root_run_id: &str)` becomes `async`. It calls `WatcherFiles::remove_copy` only when `store.get_run(root_run_id)` is a `Watcher` run.

- [ ] **Step 1: Write the failing tests** (`runtime_attachments_test.rs`, `Runtime::start` on a tempdir with `ScriptedEngine`, folder-mode dot unless named).
  - `a_message_is_never_claimed_without_its_files` (Review Focus 1): stage two files. Use `chat_with_files` with the dispatcher running and a scripted engine that records whether `<workspace>/attachments/<run id>/a.txt` exists at `start`. Expected: `true`. The run's `payload.attachments` has 2 entries with names `a.txt`, `b.txt`.
  - `names_are_unique_within_a_topic`: the first message has `report.pdf`. A reply with `REPORT.pdf` and `report.pdf` gives the names `REPORT (2).pdf` and `report (3).pdf`. A file named `.gitignore` becomes `.gitignore (2)`.
  - `limits_and_ownership_are_checked`: 11 ids → the count text. Another dot's staged id → `Conflict`. A sent id used again → `Conflict`. A staged id whose folder was deleted → `Conflict` with the "no longer there" text. After each refusal, no new run exists (`hold_runs(true)`; `list_runs`).
  - `a_message_may_be_files_only`: `""` with one file → `Ok`. `""` with none → `Invalid`.
  - `two_sends_of_the_same_files_link_once`: the second `chat_with_files` with the same ids → `Conflict`, and the first run's rows are unchanged.
  - `chat_files_survive_the_end_of_a_run` (Review Focus 2): a folder-mode dot, one message with a file, run to `Succeeded` (`wait_status`) → the file and `.gitignore` are still in `attachments/<root>/`.
  - `a_follow_up_gets_missing_files_back_and_edits_are_kept`: after the first run, delete one workspace copy and overwrite the other with `edited`. A reply runs, and then the deleted one is back with the stored bytes, and the edited one still reads `edited`.
  - `a_reply_finds_its_files_in_a_restored_worktree` (Review Focus 3): a worktree dot (`init_repo`). The first run changes nothing, so its worktree is cleaned up (assert the path is gone). The reply runs in the restored worktree and the file is there. `git status --porcelain` in it is empty.
  - `copying_refuses_a_linked_attachments_folder` (Review Focus 4): before the first run, make `<workdir>/attachments` a junction (`mklink /J` through `cmd`, or a symlink on Unix) to another tempdir. The run fails with an error starting `could not put the attached files in the workspace`. The other tempdir stays empty. `delivery == Some(NotDelivered)`.
  - `a_watcher_topic_still_loses_its_watcher_files`: the existing behaviour, a watcher root with staged files, is unchanged (reuse the `runtime_watchers_test` setup in one case).
  - `retry_carries_the_files`: a chat run with one file, failed undelivered (`finish_run(Failed, ..)` by the store, as in `runtime_delivery_test`). `retry_delivery` gives a new run whose `payload.attachments` has the same id and name. The row's `run_id` is the new run.
  - Prompt (`engine_prompt_test.rs`), `build_prompt_full` with a fake run whose payload is `{ message: "Shorten it", typed: true, attachments: [{id, name: "r.pdf", size: 2458}] }` and workspace `W`:
    - a first prompt contains `## Message from the user\n\nShorten it\n\n## Attached files\n\nThe person attached these files to this message. They are in your workspace and stay there for the rest of this conversation:\n\n- W/attachments/<root>/r.pdf (2.4 KB)` (path built with `Path::join`);
    - a resumed chat (`session_id` set) returns exactly `Shorten it\n\n` plus the block;
    - with `recent` runs, the block follows the message after `## Message from the user`;
    - message `""` → the block alone.
- [ ] **Step 2: Run** `cargo test -p dots-core --test runtime_attachments_test --test engine_prompt_test`. Expected: FAIL / compile errors.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the new file twice, then `runner_test`, `runtime_watchers_test`, `watchers_files_test`, `runtime_delivery_test`, `runtime_test`, `workspace_test`, and the full `dots-core` suite once. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): a chat message carries files into its topic's workspace and names them to the dot`.

### Task 3: The Files list, open targets, and the app

**Files:**
- Modify: `crates/dots-core/src/store/attachments.rs` (`written_files`), `crates/dots-core/src/runtime.rs` (new methods after `run_folder` :906), `crates/dots-core/src/model.rs`
- Modify: `apps/dots-app/src/commands.rs`: `chat` :588 (new argument); new commands next to `open_run_folder` :741
- Modify: `apps/dots-app/src/lib.rs` (`command_list!` :36, after `chat`)
- Modify: `apps/dots-app/src/contract.rs` (samples ~:1290, `Enums`)
- Regenerate: `ui/src/api/__fixtures__/contract.json`
- Test: `crates/dots-core/tests/runtime_files_test.rs` (new), `apps/dots-app` (`commands` tests if present, else contract only)

**Interfaces:**
- Consumes: Tasks 1–2.
- Produces:
  - `str_enum!(FileSource { Attached => "attached", Written => "written" })`. `pub struct ConversationFile { pub path: String, pub name: String, pub source: FileSource, pub run_id: String, pub at: String, pub exists: bool, pub can_open: bool }`.
  - `pub struct WrittenFile { pub run_id: String, pub at: String, pub path: String, pub workspace_path: Option<String> }`. `Store::written_files(&self, dot_id: &str, roots: &[String]) -> Result<Vec<WrittenFile>>`, one query: `run_events e JOIN runs r` on `r.dot_id = ?`, `r.root_run_id IN (SELECT value FROM json_each(?))`, `e.kind = 'tool_call'`, `json_extract(e.data, '$.tool')` in the six tools, path `COALESCE(json_extract(e.data,'$.input.file_path'), json_extract(e.data,'$.input.notebook_path'))` not null.
  - `Runtime::conversation_files(&self, dot_id: &str, roots: &[String]) -> Result<Vec<ConversationFile>>`: `get_dot` (`NotFound`); a root of another dot is skipped. Attached path: the workspace copy if it is a file (workspace = the topic's newest run with `workspace_path`), else `stored_path`. Written: relative paths joined to the run's workspace; lexical normalization (drop `.`, apply `..`); kept only if the normalized path starts with the normalized workspace, and, when it exists, its canonical path starts with the canonical workspace. Merge key: the path lowercased on Windows. The newest `at` wins; `Attached` if any source is. Sort `at` descending, then path ascending. `name` = the file name of `path`.
  - `Runtime::conversation_file_target(&self, dot_id: &str, roots: &[String], path: &str, open: bool) -> Result<PathBuf>`: not listed, or `!exists` → `NotFound("this file is no longer there")`. `open && !can_open` → `Invalid` with the programs text.
  - Commands:
    - `chat(dot_id, message, parent_run_id, attachment_ids: Option<Vec<String>>) -> Res<Run>` (calls `chat_with_files`);
    - `attachment_add_path(dot_id: String, path: String) -> Res<StagedAttachment>`;
    - `attachment_add_bytes(state, request: tauri::ipc::Request<'_>) -> Res<StagedAttachment>`: the body must be `InvokeBody::Raw` (else `invalid("expected the file's bytes")`); headers `x-dots-dot` and `x-dots-name`, percent-decoded with a small local `percent_decode(&str) -> Option<String>` (UTF-8); a missing header → `invalid`;
    - `attachment_remove(id: String) -> Res<()>`;
    - `conversation_files(dot_id: String, root_run_ids: Vec<String>) -> Res<Vec<ConversationFile>>`;
    - `conversation_file_open(app, state, dot_id, root_run_ids, path) -> Res<()>`: `target(.., open = true)` then `app.opener().open_path(..)`;
    - `conversation_file_reveal(..) -> Res<()>`: `target(.., open = false)` then `app.opener().reveal_item_in_dir(..)`. Errors are mapped as in `open_run_folder`.
  - Contract: `samples["StagedAttachment"] = { id, name: "report.pdf", size: 2458 }`; `samples["ConversationFile"]` (attached, exists) and `samples["ConversationFileGone"]` (written, `exists: false`, `can_open: true`); `samples["RunWithAttachments"]` (chat run, `payload.attachments` with one entry); `Enums.FileSource`.

- [ ] **Step 1: Write the failing tests** (`runtime_files_test.rs`). Build the runs with the store, set `queued_at` explicitly, and append `tool_call` events with `append_event(run, "tool_call", json!({"kind":"tool_call","id":"t","tool":..,"input":{..}}))`. Files are made in a tempdir workspace.
  - `written_files_come_from_every_tool`: `Write` (absolute), `Edit` (relative `out/a.md`), `MultiEdit`, `NotebookEdit` (`notebook_path`), `write_file`, `edit_file`. Each one is listed with an absolute path inside the workspace and `source == Written`. `Read` and `Bash` calls are not listed.
  - `paths_outside_the_workspace_are_left_out`: `C:\other\x.txt` (or `/other/x.txt`) and `../escape.txt` are not listed.
  - `files_merge_and_come_newest_first`: the same file written at t1 and edited at t3 (`A.md`, then `a.md` on Windows) gives one entry with `at == t3`. An attachment at t2 that the dot edits at t4 gives one `Attached` entry with `at == t4`. Order: t4, t3.
  - `a_deleted_file_is_gone`: `exists == false`.
  - `an_attachment_falls_back_to_its_stored_copy`: delete the workspace copy, and `path` is the stored copy with `exists == true`.
  - `a_path_not_in_the_list_is_not_opened` (Review Focus 5): `conversation_file_target(dot, roots, "C:\\Windows\\notepad.exe", true)` → `NotFound`. A gone file → `NotFound`.
  - `a_program_is_listed_but_not_opened` (Review Focus 5): `run.ps1` written by the dot is listed with `can_open == false`. Target with `open = true` → `Invalid` with the programs text, and with `open = false` → `Ok(path)`.
  - `topics_of_another_dot_are_ignored`.
- [ ] **Step 2: Run** `cargo test -p dots-core --test runtime_files_test`. Expected: compile errors.
- [ ] **Step 3: Implement** the core, then the commands and the samples.
- [ ] **Step 4: Regenerate and run**: `$env:UPDATE_CONTRACT=1; cargo test -p dots-app contract; Remove-Item Env:UPDATE_CONTRACT; cargo test -p dots-app; cargo test -p dots-core --test runtime_files_test`. Expected: PASS. The UI contract test fails until Task 4 updates `types.ts` and `commands.ts`; that is expected here.
- [ ] **Step 5: Commit** `feat(app): stage, send and list a conversation's files, and open them only through the core`.

### Task 4: UI — adding files, chips, sending, bubbles

**Files:**
- Modify: `ui/src/api/types.ts` (`StagedAttachment`, `ConversationFile`, `FileSource`, `MessageAttachment = z.object({ id: z.string(), name: z.string(), size: z.number() })`)
- Modify: `ui/src/api/commands.ts` (`chat` :171 gains `attachmentIds: string[] = []`; new `attachmentAddPath`, `attachmentAddBytes`, `attachmentRemove`; a `callRaw(schema, cmd, bytes, headers)` helper beside `call`)
- Create: `ui/src/lib/attachments.ts`, `ui/src/lib/file-drop.ts`, `ui/src/components/AttachmentChip.tsx`
- Modify: `ui/src/screens/home/Composer.tsx`, `ui/src/screens/home/ReplyArea.tsx` (`send` :94, the `Composer` props :223), `ui/src/components/Transcript.tsx` (`chatMessage` :57, the bubble :463)
- Modify: `ui/src/test/setup.ts`: the `invoke` mock passes `(cmd, args, options)`, and a raw call is recorded as `{ ...options.headers, bytes: args }`. Add a `vi.mock("@tauri-apps/api/webview")` whose `getCurrentWebview().onDragDropEvent(h)` is `backend.listen("drag-drop", (e) => h(e))`.
- Test: `ui/src/lib/attachments.test.ts`, `ui/src/screens/home/Attachments.test.tsx` (new), `ui/src/components/Transcript.test.ts` (bubble)

**Interfaces:**
- Consumes: Task 3 contract.
- Produces:
  - `attachments.ts`:
    - `MAX_FILES = 10`, `MAX_FILE_BYTES = 25 * 1024 * 1024`;
    - `TOO_MANY = "You can attach up to 10 files to a message."`;
    - `pastedName(file: { name: string; type: string }): string` (spec §3.1.6);
    - `messageAttachments(run: Run): MessageAttachment[]` (from `payload.attachments`, `[]` when absent or malformed);
    - `room(staged: number, adding: number): { take: number; refused: boolean }`.
  - `file-drop.ts`: `useFileDrop(enabled: boolean, onPaths: (paths: string[]) => void): boolean` (whether files are being dragged over the window). It listens with `getCurrentWebview().onDragDropEvent`: `enter`/`over` → true, `leave` → false, `drop` → false and `onPaths(paths)` when enabled. It unlistens on unmount.
  - `Composer` props gain `files: { key: string; name: string; size: number | null }[]` (size null = adding), `onAddPaths(paths: string[])`, `onAddFiles(files: File[])`, `onRemove(key: string)`, `refusals: string[]`, `canAttach: boolean`. The paperclip calls `open({ multiple: true, directory: false, title: "Attach files" })`. Paste: when `clipboardData.files` is non-empty, `preventDefault` and `onAddFiles`. Send is enabled when (`text` or `files.length > 0`) and no file is adding. `onSend(message)` is unchanged.
  - `ReplyArea` owns the staged list, keyed by a local key. Adding: `room(...)`; then for each path or file it calls the command. Each refusal's `CommandError.message` goes to `refusals`, and the count refusal is `TOO_MANY`. `send.mutationFn` passes the staged ids. On success it clears the files and refusals. On unmount it calls `attachmentRemove` for each staged id, ignoring errors. While `answering`, `canAttach` is false and the files are kept and not sent.
  - Bubble: `chatMessage` stays. The bubble renders when the text is non-empty or `messageAttachments(run).length > 0`, with an `AttachmentChip` per file under the text (no ×).

- [ ] **Step 1: Write the failing tests.**
  - `attachments.test.ts`:
    - `pastedName({name:"image.png",type:"image/png"}) === "Pasted image.png"`; `{name:"",type:"image/jpeg"}` gives `Pasted image.jpg`; `{name:"diagram.webp",type:"image/webp"}` gives `diagram.webp`; `{name:"",type:"application/x"}` gives `Pasted image.png`.
    - `room(8, 3)` gives `{ take: 2, refused: true }`, and `room(0, 10)` gives `{ take: 10, refused: false }`.
    - `messageAttachments` on a payload without attachments gives `[]`.
  - `Attachments.test.tsx` (home fixtures, a dot with a current chat topic that has ended):
    - `the_paperclip_attaches_picked_files`: `dialog_open` returns two paths; `attachment_add_path` is called with `{ dotId, path }` twice; two chips show `report.pdf` and `2.4 KB`.
    - `a_refused_file_says_why_and_the_others_stay`: the second add rejects with `{ kind: "invalid", message: "big.bin is larger than 25 MB." }`. The alert shows that text, and one chip stays.
    - `an_eleventh_file_is_refused`: with 10 chips, a drop of one path shows `You can attach up to 10 files to a message.` and calls no command.
    - `a_drop_attaches_and_a_drag_shows_the_hint`: `backend.emit("drag-drop", { type: "enter", paths: [] })` shows `Drop files to attach`; `{ type: "drop", paths: ["C:\\a.txt"] }` calls `attachment_add_path` and hides the hint.
    - `a_pasted_screenshot_is_sent_as_bytes`: paste an `image/png` `File` named `image.png` → one `attachment_add_bytes` call with `x-dots-name: "Pasted%20image.png"` and `x-dots-dot: <dot>`.
    - `removing_a_chip_unstages_it`: × with label `Remove report.pdf` → `attachment_remove` with `{ id }`, and the chip is gone.
    - `send_passes_the_files_and_clears_them`: type `Shorten it`, Send → `chat` with `attachmentIds: [id1, id2]`; chips cleared. Files only, with no text: Send is enabled, and `chat` gets `message: ""`.
    - `send_waits_for_files_being_added`: while `attachment_add_path` is pending (`later()`), Send is disabled and the chip shows `Adding…`.
    - `answering_takes_no_files`: with "Reply to question" pressed, the paperclip is disabled and a drop calls nothing.
  - Transcript: a chat run with `payload.attachments: [{ id, name: "r.pdf", size: 2458 }]` and message `""` renders a bubble with `r.pdf` and `2.4 KB`.
- [ ] **Step 2: Run** `cd ui; bun run test`. Expected: FAIL.
- [ ] **Step 3: Implement** with the spec §6 copy.
- [ ] **Step 4: Run** `cd ui; bun run typecheck; bun run test`. Expected: PASS, the contract test included.
- [ ] **Step 5: Commit** `feat(ui): attach files with the paperclip, a drop or a paste, and see them on the message`.

### Task 5: UI — the Files panel and its toggle

**Files:**
- Modify: `ui/src/api/commands.ts` (`conversationFiles`, `conversationFileOpen`, `conversationFileReveal`), `ui/src/lib/query.ts` (`keys.conversationFiles(dotId, roots) = ["runs", dotId, "files", ...roots]`, under `["runs"]` so `run_updated` refreshes it)
- Create: `ui/src/screens/home/FilesPanel.tsx`, `ui/src/screens/home/files-pref.ts`
- Modify: `ui/src/screens/home/DotPane.tsx` (:52: the conversation column and the `aside` side by side; the toggle among the `DotHeader` children :58)
- Test: `ui/src/screens/home/FilesPanel.test.tsx` (new)

**Interfaces:**
- Consumes: Task 3 commands; `view.shown` roots from `useConversation`.
- Produces:
  - `files-pref.ts`: `FILES_KEY = "dots.home.files"`, `filesHidden(): boolean`, `setFilesHidden(hidden: boolean): void`. Storage that cannot be read counts as shown, and storage that cannot be written is left alone (the `last-dot.ts` pattern).
  - `<FilesPanel dotId dotName roots files />`: rows as spec §6. A name click → `conversationFileOpen`, the folder button → `conversationFileReveal`, and errors → `notifyError`. `<FilesToggle hidden onToggle />` is in the same file.
  - `DotPane`: `useQuery(keys.conversationFiles(dot.id, roots), …)`, where `roots = view.shown.map(t => t.rootId)`, enabled when `roots.length > 0`. The toggle and panel show only when the list is non-empty. The panel shows when `!hidden`.

- [ ] **Step 1: Write the failing tests.**
  - `lists_attached_and_written_files_newest_first`: `conversation_files` answers an attached `r.pdf` (newer) and a written `out.md`. Rows in order: `r.pdf` / `Attached · …`, then `out.md` / `Changed by nightly · …`. The call's args are `{ dotId, rootRunIds: [root] }`.
  - `a_click_opens_and_the_folder_button_reveals`: the `r.pdf` button → `conversation_file_open` with `{ dotId, rootRunIds, path }`. `Show r.pdf in folder` → `conversation_file_reveal`.
  - `a_gone_file_has_no_actions`: `exists: false` shows `Gone`, with no button named `out.md` and no folder button.
  - `a_program_is_not_a_button`: `can_open: false` gives no button `run.ps1`, the title is the programs text, and the folder button is present.
  - `the_toggle_hides_the_panel_and_is_remembered`: click `Files` (pressed → not pressed) → no `Files` region, and `localStorage["dots.home.files"] === "hidden"`. Unmount and render again → still hidden. Click again → shown, and the key is removed.
  - `no_files_no_panel_no_toggle`: `[]` → neither the region nor the button.
  - `a_run_updated_refreshes_the_list`: `backend.emit("runtime", { type: "run_updated", run })` → `conversation_files` is called a second time.
- [ ] **Step 2: Run** `cd ui; bun run test`. Expected: FAIL.
- [ ] **Step 3: Implement.** The panel is `w-72 shrink-0 border-l overflow-y-auto`. Below the `md` breakpoint it is not drawn, so the composer keeps its width in a narrow window; the toggle still shows.
- [ ] **Step 4: Run** `cd ui; bun run typecheck; bun run test`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(ui): a Files panel beside the conversation, which you can hide`.

### Task 6: Send later carries files

Adapt the names to the send-later code as merged: its table, its schedule command, its send path in the sweep, "Send now", Cancel and the scheduled card.

**Files:**
- Modify: `crates/dots-core/migrations/0033_chat_attachments.sql` (the `scheduled_id` reference, if Task 1 left a placeholder)
- Modify: `crates/dots-core/src/store/attachments.rs` (`hold_for_scheduled`, `scheduled_attachments`)
- Modify: send later's runtime methods (schedule, send, send now, cancel), its app command, its UI schedule call and its scheduled card
- Test: send later's runtime test file (extend), `ui/src/screens/home/Attachments.test.tsx` (extend)

**Interfaces:**
- Consumes: Task 2 `chat_inner` / `ChatSend`, Task 4 composer state.
- Produces:
  - `Store::hold_for_scheduled(&self, tx: &mut SqliteConnection, scheduled_id: &str, dot_id: &str, ids: &[String]) -> Result<()>`: `UPDATE … SET scheduled_id = ? WHERE id = ? AND dot_id = ? AND run_id IS NULL AND scheduled_id IS NULL`. Any count of 0 → `Conflict` (the "no longer there" text). It runs inside send later's own insert transaction.
  - `Store::scheduled_attachments(&self, scheduled_id) -> Result<Vec<ChatAttachment>>`.
  - The schedule command gains `attachment_ids: Option<Vec<String>>`, and checks `MAX_FILES` and each row as `chat_inner` step 3 does.
  - Send (sweep and Send now): `chat_inner(dot, ChatSend { message, parent_run_id, attachment_ids: <scheduled rows' ids>, scheduled_id: Some(id), carry_from: None })`.
  - Cancel: read `scheduled_attachments`, delete the scheduled row (the rows cascade), then `ChatFiles::remove` each id. `ChatFiles::sweep` keeps rows whose `scheduled_id` is set.
  - UI: scheduling passes the staged ids and clears the chips like a send. The scheduled card renders `messageAttachments`-style chips from the scheduled message's attachment list (send later's read model gains `attachments: MessageAttachment[]`).

- [ ] **Step 1: Write the failing tests.**
  - `a_scheduled_message_keeps_its_files_until_it_sends` (FakeClock): stage one file, schedule for t0 + 1 h. `remove_attachment(id)` → `Conflict`. A restart-style `ChatFiles::sweep` keeps it. Advance the clock and sweep → the sent run's `payload.attachments` has the file, the row has `run_id` set and `scheduled_id` NULL, and its workspace copy exists after the run.
  - `cancelling_a_scheduled_message_removes_its_files`: the row and the folder are gone.
  - `send_now_carries_the_files`.
  - `a_scheduled_message_to_a_gone_conversation_takes_its_files_along`, if send later sends into a new conversation: the names are fixed against the new topic.
  - UI: scheduling with two chips calls the schedule command with `attachmentIds`, and the chips clear. The scheduled card lists `r.pdf`.
- [ ] **Step 2: Run** the extended tests. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the send-later and attachments test files, `cd ui; bun run typecheck; bun run test`, and regenerate the contract if send later's types changed. Expected: PASS.
- [ ] **Step 5: Commit** `feat: a message sent later carries its files`.

### Task 7: Whole-branch check and pull request

- [ ] **Step 1: Run** CI's steps:
  - `cargo fmt --all -- --check`;
  - `cargo clippy -p dots-core -p dots-app --all-targets -- -D warnings`;
  - `cargo test -p dots-core --locked --no-fail-fast -- --skip preview_schedule_lists_the_next_fire_times` (twice);
  - `cargo test -p dots-app --locked`;
  - `node --test tools/release/release.test.mjs`;
  - `cd ui; bun run typecheck; bun run test`.

  Expected: all pass, in both core runs.
- [ ] **Step 2: Manual check** in the debug app:
  - a folder-mode dot: paste a screenshot, drop a PDF, and send. The files are in `attachments/<root>/`, and `git status` in the folder is clean. A reply can read them.
  - the Files panel: it lists both, plus a file the dot wrote. Open and Show in folder work. Hide, restart Dots, and it is still hidden.
- [ ] **Step 3: Push** `feat/attachments` and open the PR with base `feat/send-later`. Note in the body:
  - that it is stacked on send later (and send later on snooze/done);
  - that it adds migration 0033;
  - the builder's calls of spec §3.1 and §4.5;
  - the `remove_attachments` change, which now removes watcher topics only.
