# Attachments in chat, and a Files panel

- Date: 2026-11-04
- Status: draft for the owner's review (decisions of 2026-11-04 applied)
- Part of: Tier 1 of the build list, item 5. Third of three stacked branches: snooze/done (`feat/snooze-done`, migration 0031), send later (`feat/send-later`, 0032), attachments (`feat/attachments`, 0033).
- Builds on: `2026-10-08-dots-design.md` (runs, workspaces), the watchers spec §4.2 (staged watcher attachments), `2026-11-03-delivery-tracking-design.md` (Retry), the send-later spec (scheduled chat messages).

## 1. Goal

The person can give a dot files with a message: pick them, drop them on the chat box, or paste a screenshot. The dot finds them in its working folder and is told where. They stay there for the rest of the conversation, so a later message can say "now shorten the PDF I sent". Beside the conversation, a Files panel lists the files the person attached and the files the dot wrote or edited, newest first. A click opens a file, and "Show in folder" shows it in Explorer. The panel can be hidden, and Dots remembers that choice.

Success: a message with a 3 MB PDF and a pasted screenshot reaches the dot as two files at paths the prompt names. A follow-up two days later can still read them. The Files panel lists both, plus the report the dot wrote, and each opens on click.

## 2. Words

- **Conversation**: what a dot's side of the home shows. That is the current chat topic and the run topics after it, or one older topic opened from History (`ui/src/lib/home-topics.ts` `conversation`).
- **Topic**: one line of runs that share a `root_run_id`. Attachments belong to a topic.
- **Staged file**: a file the person added to the chat box but has not sent. Dots has already copied it into its own data folder.
- **Stored copy**: Dots' own copy of an attached file, `<data>/chat-files/<attachment id>/<file name>`. It is kept as long as its topic exists.
- **Workspace copy**: the copy the dot sees, `<workspace>/attachments/<topic root id>/<name>`. The workspace is the run's `workspace_path`: in folder mode, the dot's own folder (shared by all its topics); in worktree mode, the topic's worktree (every run of a topic shares it, `Runtime::chat` copies `workspace_path` from the parent).

## 3. Behaviour (decided with the owner)

1. **Adding**: a paperclip button opens a file picker (several files at once). Files dropped on the window go into the chat box. A pasted image (a screenshot) is added as a file. Each file shows as a chip under the text, with its name, its size and a × to remove it.
2. **Limits**: any file type. At most **10 files** per message. At most **25 MB** each. A file over a limit is refused with a reason (texts in §6); the others are still added.
3. **Delivery**: the files are copied into the dot's working folder under `attachments/`, and the message tells the dot their paths (§4.4).
4. **They stay** for the rest of that conversation, so follow-ups can use them. Watcher attachments, by contrast, are removed when their run ends.
5. **Send later** may carry files. They are staged when the message is scheduled and delivered when it sends (§4.6).
6. **Files panel**: beside the conversation. It lists your attachments and the files the dot wrote or edited in this conversation (its Write/Edit tool calls), newest first. A click opens the file. "Show in folder" shows it in Explorer. A toggle in the header hides the panel, and the choice is remembered.

### 3.1 Decided while designing *(builder's calls)*

1. **Files are staged when they are added, not when the message is sent.** Each add copies the file into Dots' data folder at once and returns an id. Send passes the ids. So a limit is refused the moment the file is added, a pasted image needs no path, and a scheduled message already holds its own copy.
2. **The stored copy is the original. The workspace copy is refreshed before every run of the topic.** Before each run of a topic, the runner copies every file of that topic that is missing from `attachments/<topic>/`. It never overwrites one that is there, because the dot may have edited it. This is what makes "they stay for the conversation" true everywhere. It covers worktrees that Dots removes when unchanged and recreates on the next reply, a dot whose workdir changed, and a person who deleted the folder.
3. **One folder per topic**: `attachments/<topic root id>/`. In folder mode every topic of a dot shares the workspace, so a flat `attachments/` would mix topics and collide on names.
4. **Git ignores the files.** The topic folder gets a `.gitignore` containing `*`. So the files are never "changes" of a worktree: the diff view does not list them, a workflow step's commit does not take them, and an unchanged worktree is still cleaned up. In a folder-mode dot whose folder is a repository, the person's `git status` stays clean.
5. **Name collisions**: names are made safe with the watchers' `safe_name`, which keeps only the last path part and removes `<>:"|?*`, control and bidi characters, and reserved device names. Within a topic they are made unique, ignoring case, with `unique_names` (`report.pdf`, `report (2).pdf`). Names already used in the topic and `.gitignore` count as taken. The final name is fixed when the message is sent.
6. **A pasted image** is named `Pasted image.<ext>` (png, jpg, gif or webp from its type; png otherwise) unless the clipboard file has a name other than `image.png`.
7. **A message may be files only** (empty text). It is refused only when it has neither text nor files.
8. **When files are removed**:
   - a staged file: when its chip's × is pressed; when the chat box goes away unsent (the dot pane closes); and, as a backstop, at the next start of Dots;
   - stored copies: when their dot is deleted. A sweep at startup also removes any folder under `<data>/chat-files/` that has no row;
   - workspace copies: Dots never deletes files in the person's own folder (folder mode). The person may delete `attachments/` themselves; it comes back at the topic's next run. In worktree mode they go with the worktree (cleanup, Discard). Topics are never deleted on their own, so "the rest of the conversation" means as long as the dot exists.
9. **A topic a watcher started** already has a watcher folder at `attachments/<root id>/`. That folder is removed after each run. A reply there with files puts them in the same folder. They are removed with it after the run and copied back before the next one. Nothing else changes for watchers.
10. **Retry** of a message that was not delivered (delivery tracking) moves its files to the new message. **Discard** leaves them with the dismissed message: they are no longer copied, and the Files panel no longer lists them.
11. **Answering a question** ("Reply to question") sends no files. While answering, the paperclip is disabled and drops and pastes are ignored. Files already in the box stay there for the next message.
12. **Drop target**: the whole window, while a conversation's chat box can send. While files are dragged over the window, the chat box shows "Drop files to attach". Tauri gives a drop to the window as a whole, so hit-testing the box would gain nothing.
13. **No thumbnails** in chips. The CSP allows no `blob:` images, and a chip only needs the name and size.

## 4. Core

### 4.1 Storage

- **Migration 0033** (`0033_chat_attachments.sql`):

```sql
CREATE TABLE chat_attachments (
    id           TEXT PRIMARY KEY,                                   -- ULID; its folder under <data>/chat-files
    dot_id       TEXT NOT NULL REFERENCES dots(id) ON DELETE CASCADE,
    run_id       TEXT REFERENCES runs(id) ON DELETE CASCADE,         -- the message it went with; NULL while staged
    scheduled_id TEXT REFERENCES <send-later table>(id) ON DELETE CASCADE, -- the scheduled message it waits with
    file_name    TEXT NOT NULL,   -- safe name as added: <data>/chat-files/<id>/<file_name>
    name         TEXT,            -- its name in attachments/<topic>/, fixed when sent
    size         INTEGER NOT NULL,
    created_at   TEXT NOT NULL,
    CHECK (run_id IS NULL OR name IS NOT NULL)
);
CREATE INDEX chat_attachments_run ON chat_attachments(run_id);
```

  `<send-later table>` is the scheduled-messages table of migration 0032, as merged. Sending clears `scheduled_id` in the same write that sets `run_id`, so a scheduled row deleted later never takes sent files with it.
- The run's payload also lists its files: `payload.attachments = [{ "id", "name", "size" }]`, written once when the run is created (the bubble and the prompt read it). The table is the source of truth for copying, the panel and cleanup. `Run` gains no field.

### 4.2 Staging (`crates/dots-core/src/chat_files.rs`)

- `MAX_FILES = 10`, `MAX_FILE_BYTES = 25 * 1024 * 1024`.
- **From a path** (picker, drop): `symlink_metadata`. A link (symlink or junction) is refused, a folder is refused, and a size over the limit is refused before anything is read. The copy then reads at most `MAX_FILE_BYTES + 1` bytes, so a file that grows while it is copied is refused too. Paths come from the UI, the way a dot's workdir does today. The UI is trusted to pass what the person picked.
- **From bytes** (paste): the same size limit, with the name from the UI made safe.
- Both check the dot (`get_dot`, `require_run`), insert the row (`run_id` and `scheduled_id` NULL) and answer `StagedAttachment { id, name, size }`.
- **Removing** a staged file deletes its row and folder only while `run_id` and `scheduled_id` are both NULL. Otherwise it is a `Conflict`.

### 4.3 Sending

- `Runtime::chat` gains attachments: `chat_with_files(dot_id, message, parent, attachment_ids)`. The old `chat` calls it with none.
- Checks, before anything is written: at most 10 ids; each row exists, belongs to this dot, and is unsent (`run_id` NULL, `scheduled_id` NULL or the scheduled message being sent), with its stored file present. An empty message with no files is still refused. Then the existing parent rules apply.
- The run id is made beforehand (`NewRun.id`). The topic root is the parent's `root_run_id`, or the new id. Names are fixed against the topic's names already taken. The run row is inserted and the attachment rows are linked (`run_id`, `name`, `scheduled_id = NULL`) **in one transaction**, so the dispatcher never claims a message whose files are not linked yet. If fewer rows link than were named, the transaction is rolled back (`Conflict`: another send took them).
- **Retry** (delivery tracking) relinks the old run's rows to the new run in the same transaction.

### 4.4 The run: copy and prompt

- In the runner, right after the workspace is prepared and next to the watchers' `copy_attachments`, `copy_chat_files` lists the topic's files and copies those that are missing into `<workspace>/attachments/<root>/`. The topic's files are the rows linked to runs of the topic, leaving out runs that were dismissed and never delivered. The `.gitignore` (`*`) is written if it is missing.
- **Safety while copying**: the root id must be letters and digits only. `attachments` and `attachments/<root>` must each be missing or a real folder, never a link or junction. Each file is created with `create_new`, so a link planted at its name is never followed. The canonical parent folder must be inside the canonical workspace. When a check fails, the file is not copied.
- If a file of **this run's own message** cannot be put in place, the run fails before its engine starts with `could not put the attached files in the workspace: <error>`. Delivery tracking then marks it not delivered, with Retry. An earlier message's file that fails is logged and recorded as a notice on the run: `Dots could not copy <name> into the workspace: <error>`.
- **The prompt**: the message text gets this block when the run's payload has attachments. The block is added in all three places the message goes: a first prompt, a resumed chat, and a message after "runs since your last message":

```
## Attached files

The person attached these files to this message. They are in your workspace and stay there for the rest of this conversation:

- <absolute path> (<size>)
```

  The size is written as the UI's `formatBytes` writes it ("240 B", "2.4 KB", "1.5 MB"). With an empty message text, the block is the whole message.
- **Removing watcher copies**: `remove_attachments` now removes `attachments/<root>/` only when the topic's root is a watcher run. Today it does so for every run, which would delete a chat topic's files after each run.

### 4.5 Files panel data

- **`Runtime::conversation_files(dot_id, root_run_ids) -> Vec<ConversationFile>`**:
  - **attached**: the topic's files (as in §4.4) of the given topics. Path: the workspace copy if it is a file now, else the stored copy. `at` = the run's `queued_at`.
  - **written**: `run_events` with `kind = 'tool_call'` of runs in the given topics (all triggers), whose tool is `Write`, `Edit`, `MultiEdit`, `NotebookEdit` (Claude) or `write_file`, `edit_file` (local engine). The path is `input.file_path`, or `input.notebook_path`. A relative path is joined to the run's `workspace_path`. Paths are normalized lexically. **Only paths inside the run's workspace are listed** *(builder's call: the owner's rule that paths stay inside the workspace; a call to a path outside, even a refused one, is left out)*. `at` = the event's `ts`.
  - Merged by path (ignoring case on Windows). Each path keeps its newest `at`. It is `attached` if any of its sources is an attachment. Sorted newest first, then by path.
  - `exists` = the path is a file now. `can_open` = its extension is not a program or script (§4.5.1).
- **`Runtime::conversation_file_target(dot_id, root_run_ids, path) -> PathBuf`**: recomputes the list and answers the entry's path. It answers `NotFound` (`this file is no longer there`) when it is not listed or does not exist, so the app never opens a path the UI made up. "Open" also requires `can_open`.

#### 4.5.1 Programs are never opened *(builder's call)*

Opening a file runs its default handler. For a `.exe` or `.ps1` the dot wrote, that means running it. These extensions are shown but not opened; "Show in folder" still works: `exe com bat cmd ps1 psm1 vbs vbe js jse wsf wsh msi msp scr hta lnk url cpl reg jar pif application gadget msc scf inf appref-ms`. Refusal: `Dots doesn't open programs or scripts. Use Show in folder.`

### 4.6 Send later *(adapt to the send-later code as merged)*

- Scheduling with files: the schedule command takes `attachment_ids`. In the transaction that stores the scheduled message, those unsent rows get its `scheduled_id`. The same checks as §4.3 apply, except the names, which are fixed when it sends.
- When it sends (the sweep, or "Send now"), the send path calls the chat path with the scheduled message's rows and its id. The rows link to the new run as in §4.3.
- Cancel: the rows go with the scheduled message (cascade), and their folders are removed right away (the startup sweep is the backstop). Edit changes the text only; the files stay. To change the files, cancel and schedule again.
- If send later sends into a new conversation because the old one is gone, the files go there. Their names are fixed against that topic.
- The scheduled card shows the files as chips, without ×.

### 4.7 Startup sweep and deleting a dot

- At start, next to `watcher_files.sweep`, `ChatFiles::sweep`: delete rows with `run_id` NULL and `scheduled_id` NULL (no chat box survives a restart), then remove every folder under `<data>/chat-files/` that has no row.
- `delete_dot` reads the dot's attachment ids before the delete and removes their folders after it.

## 5. App

- **Commands**: `attachment_add_path(dot_id, path) -> StagedAttachment`. `attachment_add_bytes` takes a raw body (`tauri::ipc::Request`): the bytes as an `InvokeBody::Raw`, with headers `x-dots-dot` and `x-dots-name` (the name percent-encoded). `attachment_remove(id)`. `chat` gains `attachment_ids: Option<Vec<String>>`. `conversation_files(dot_id, root_run_ids)`. `conversation_file_open(dot_id, root_run_ids, path)` and `conversation_file_reveal(...)` use the opener plugin already in the app: `open_path` and `reveal_item_in_dir`, on the path the core answers.
- **How files get from the UI to the core**: the picker is `@tauri-apps/plugin-dialog` `open({ multiple: true, directory: false })`, already allowed (`dialog:allow-open`), and it returns paths. A drop is `getCurrentWebview().onDragDropEvent` (`@tauri-apps/api/webview`, under `core:default`), with `paths` on `drop`. The window keeps Tauri's default drag-drop handling, so HTML drop events carry no files. A paste uses `clipboardData.files` in the text box, sent as bytes with the raw command, never as JSON numbers. No new plugin and no capability change.
- Contract: `StagedAttachment`, `ConversationFile`, `FileSource`, a chat `Run` sample with `payload.attachments`, the commands.

## 6. UI copy

- Paperclip button: `aria-label` and title **"Attach files"**. Picker title **"Attach files"**.
- While dragging over the window: the chat box shows **"Drop files to attach"**.
- Chip: name, then size (`formatBytes`). × has `aria-label` **"Remove `<name>`"**. While it is being added: **"Adding…"**.
- Refusals: one line each under the chips (`role="alert"`). They clear on the next add or a send:
  - **"You can attach up to 10 files to a message."**
  - **"`<name>` is larger than 25 MB."**
  - **"`<name>` is a folder. Attach the files in it instead."**
  - **"`<name>` is a link. Attach the file it points to instead."**
  - **"`<name>` could not be read: `<error>`"**
  - at send, a staged file that is gone: **"An attached file is no longer there. Remove it and attach it again."**
- Send is enabled with text or with at least one file, and never while a file is still being added.
- The person's bubble: the text, then the files as chips (name · size), not clickable.
- Header toggle: button **"Files"** with `aria-pressed`, title **"Hide files"** / **"Show files"**. It shows only when the conversation has files.
- Panel (`aside`, `aria-label="Files"`, heading **"Files"**), one row per file:
  - the name, as a button that opens the file (title: the full path);
  - under it, muted: **"Attached · `<day time>`"** or **"Changed by `<dot>` · `<day time>`"** (`localDayTime`);
  - a folder button: `aria-label` **"Show `<name>` in folder"**, title **"Show in folder"**;
  - a gone file: muted name, **"Gone"**, and no buttons;
  - a program: the name is not a button, and its title is **"Dots doesn't open programs or scripts. Use Show in folder."**
  - errors from open and reveal show as the usual error toast.
- The hide choice is kept per device in `localStorage` under **`dots.home.files`** (`"hidden"`, or absent for shown), like `dots.theme` and `dots.home.dot`. It is one choice for all dots. Shown is the default.

## 7. Edge cases

- **A file changed after it was added**: the stored copy is a snapshot from the moment it was added.
- **The dot edits an attachment**: its workspace copy is never overwritten. The panel lists it once, as Attached, with the newer time.
- **The person deletes `attachments/`**: it is copied back at the topic's next run.
- **The worktree is discarded**: the files come back if the topic continues (the worktree is restored).
- **Same name twice in one message**: `report.pdf`, `report (2).pdf`.
- **A file named `..\..\x.exe` or `CON`**: `safe_name` keeps `x.exe` and makes `_CON`. The file always lands in the topic folder.
- **The agent replaces `attachments` with a junction to `C:\Windows`**: copying refuses (§4.4), and the run fails or gets a notice.
- **Two windows or a stale screen send the same staged ids**: one transaction links them. The other gets `Conflict`, and its rows are untouched.
- **A topic still running when the person attaches**: the chat box is disabled while the dot works (today's rule), so files wait in the box.
- **The dot is disabled or removed from its org while files are staged**: the send is refused as today, and the files stay in the box.

## 8. Testing

- Store: link in the same transaction as the run (no claim without rows); a partial link rolls back; names unique within a topic, ignoring case, with `.gitignore` taken; Retry relinks; the topic's files leave out dismissed undelivered runs; the sweep removes unsent rows and stray folders but keeps sent and scheduled ones.
- Staging: the 25 MB limit at exactly `MAX_FILE_BYTES` (accepted) and one byte over (refused); a folder, a link and a missing path are each refused with their text; bytes with a hostile name.
- Runner: copies before the engine starts; refreshes only missing files; never overwrites; `.gitignore` written; refuses a junctioned `attachments`; fails the run when its own file cannot be placed; worktree cleanup still removes an unchanged worktree that holds attachments; watcher removal leaves chat topics alone.
- Prompt: the exact block in a first prompt, a resumed chat and after "Runs since your last message"; files only.
- Files list: written paths from each tool and engine; relative paths joined; outside paths left out; merge and order; gone; `can_open`; the target check refuses an unlisted path.
- App: the raw-bytes command; open and reveal resolve through the core; contract.
- UI: paperclip, drop, paste; limits and refusal texts; chip removal calls `attachment_remove`; send passes the ids and clears the chips; bubble chips; the panel rows, gone, programs, toggle and its memory.

## 9. Collisions with parallel work

Stacked on `feat/send-later` (0032). Shared files: `runtime.rs` (`chat`, `retry_delivery`, `delete_dot`, start), `runner.rs` (`execute_inner`, `remove_attachments`), `prompt.rs`, `watchers/files.rs` (reuse only), app `commands.rs` / `lib.rs` / `contract.rs`, `Composer.tsx`, `ReplyArea.tsx`, `DotPane.tsx`, `Transcript.tsx`, `api/types.ts`, `api/commands.ts`. Send later changes the same composer and chat path, so expect conflicts there and a contract regeneration.

## 10. Not in this part

- Attachments in watcher, schedule or API runs, and attachments on answers to questions.
- Previews or thumbnails.
- Uploading folders.
- Cleaning up a folder-mode dot's `attachments/` automatically.
- A Files panel for runs outside the conversation shown.
