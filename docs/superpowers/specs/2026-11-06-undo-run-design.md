# Undo a run: see every file a run changed and put it back in one click

- Date: 2026-11-06
- Status: approved (owner's request, 2026-11-06)
- Builds on: folder and worktree modes (`WorkspaceMode`), the worktree diff (`worktree_diff`,
  `DiffView`), the runner (`runner.rs`), the run page and the conversation (`TopicRuns`).

## 1. Goal

"Replay and undo. Every agent change is logged, and you can roll back in one click."

A worktree run already has its own branch: its changes are a diff against `base_commit`, and
Discard throws them away. A folder run (the dot works directly in a folder of the person's) had
no record of what it changed and no way back. This adds both.

## 2. Checkpoints

Before a folder run's engine starts, Dots snapshots the folder; when the run ends (whatever its
status) it snapshots it again and lists the paths that differ. Both snapshots are git trees.
The record of a run is `<data>/checkpoints/runs/<run id>.json` (no database table, so no
migration). A snapshot never fails the run: a folder Dots can't snapshot gets a record whose
note says why, and the run goes on.

- **Folder in a git repository.** Dots copies the person's index to a temporary file in its data
  folder, runs `git add -A --ignore-errors` of the run's folder into that copy (`GIT_INDEX_FILE`),
  `write-tree` and `commit-tree` (identity "Dots", no config needed), and keeps the commit under
  `refs/dots/checkpoints/<run id>`: the "after" commit, whose parent is the "before". The
  person's index file, HEAD, branches and stash are never written. If the copy can't be used (a
  merge in progress, a split index) a fresh index of the folder alone is used. What the
  repository ignores is not tracked; untracked files over the size limit are left out (see 2.1).
  The refs are visible to `git log --all` and are removed with the checkpoint (section 6).
- **Any other folder.** Dots walks the folder itself and stores each file's exact bytes
  (`hash-object -w --no-filters`) in a private repository,
  `<data>/checkpoints/shadow/<folder key>.git`, through a fresh index (`update-index
  --index-info`, `write-tree`, `commit-tree`). It never follows a link and skips dependency and
  build folders (`node_modules`, `target`, `.git`, `.venv`, `__pycache__`, `.next`, … see
  `checkpoint::SKIP_DIRS`). A stat cache (size, modified time) avoids reading unchanged files
  again; a file modified within 3 s of being read is read again next time. Nothing is written in
  the person's folder.
- Dots' own `attachments/` folder at the top of the workspace (the topic's files, copied in for
  the run) is never part of a run's changes.

Because both snapshots cover the whole folder, changes made by shell commands are caught, not
only those of Write/Edit tools. The permission tool (`mcp__dots__approve`) was considered as a
pre-write hook and not used: the CLI consults it only for calls the policy does not already
allow, so it can't see every write.

### 2.1 Limits

| Limit | Value | Past it |
| --- | --- | --- |
| One file | 20 MiB (`MAX_FILE_BYTES`) | not kept; watched by size and time, listed as "can't be put back" |
| Files to keep | 20 000 (`MAX_FILES`) | the folder is not snapshotted; the run's Changes say why |
| Bytes to keep | 512 MiB (`MAX_TOTAL_BYTES`) | as above |
| One snapshot | 120 s (`SNAPSHOT_TIMEOUT`) | as above |

In a git repository only untracked files count toward these (tracked ones are already in git).

## 3. Changes

`run_changes(run_id)` reads the record only (no git): state (`recording` while the run works,
`recorded`, `unavailable` with a note), each changed path with its status (added, modified,
deleted), whether it can be put back and why not (too large, a link, a nested repository), and
when it was undone. `null` for a run with no checkpoint: worktree runs (their Changes stay the
worktree diff), runs from before this change, runs that never started.

`run_changes_preview(run_id)` adds the diff of each file (`git diff` between the two commits,
capped at 1 MiB of patch text like a worktree's), what undo or redo would do to each file now,
and the later runs in the same folder whose changes share a path and were not undone.

## 4. Undo and redo

**Undo these changes** puts each file back as it was before the run: a file the run created is
deleted, one it deleted is recreated, one it changed gets its old content. In a git repository
the content is written as git checks it out (`cat-file --filters`: its filters and line endings);
in other folders, the exact bytes.

- **Conflicts.** For each file Dots compares what is on disk now with what the run left. Same:
  it is put back. Already as undo would leave it: nothing to do. Different (the person or a later
  run changed it): a conflict, skipped unless the person ticks "overwrite". Files that can't be
  put back are listed and skipped.
- **Later runs.** The dialog warns when later runs in the folder changed some of the same files.
- **Redo.** The run's "after" snapshot is kept, so an undone run can be redone the same way (the
  roles of the two snapshots swapped). Undo and redo each add a notice to the run's transcript.
- **Refused** while the run is working, or while any run is working in the same folder.
- **Safety.** Writes happen only inside the run's folder: a path is refused if it is absolute,
  holds `..`, a drive or stream (`:`), a backslash, a `.git` part, a name Windows changes
  (trailing dot or space) or keeps for a device, or passes through a link (symlink or junction)
  anywhere from the folder down to the file. After its parent folders are made the path is
  checked again and the real parent must be inside the real folder. A file is written beside its
  place and renamed over it. Undo and the snapshots of one folder never run at once (one lock per
  folder in the process).

## 5. Interface

- **Conversation.** Under each finished folder run's turn: "Changed N files" (a link to the run
  page; "· undone" once undone) and **Undo these changes** / **Redo these changes**.
- **Run page.** A folder thread gets a **Changes** tab: per run, the changed files, those that
  can't be undone and why, the diff, and the Undo/Redo button. Worktree threads keep their
  Changes tab and Discard.
- **Undo dialog.** Each file with a badge (Will be put back, Changed since, Already back, Can't
  be put back) and its reason; the later-runs warning; the overwrite box when there are
  conflicts; "Undo N files". Nothing is written before the person confirms.
- **Run again** (replay; it did not exist): on the run page, the thread's first message sent to
  the same dot as a new conversation, or Run now for a manual or scheduled run. "Send again" for
  a failed message stays as it was.

## 6. Retention

After a run's closing snapshot (at most once an hour) Dots removes checkpoints older than 30
days (record and ref), then, while its private repositories hold more than 5 GiB, the oldest
private checkpoints (with `git gc --prune=now` of the repositories touched), and any private
repository no checkpoint uses. Objects in the person's repository go with their normal `git gc`
once the ref is gone. The values are constants in `checkpoint.rs`.

## 7. Known gaps

- Files a repository ignores, files over 20 MiB, links, and the contents of nested repositories
  can't be put back (they are listed when a size or time change shows them, or not at all for
  ignored files and links).
- Undo restores files, not git state: commits, branch switches or stashes a run made stay.
- Two runs working in the same folder at once see each other's changes in their snapshots.
- Folders a run created and that are left empty after undo are not removed.
- A run cut short by a crash of Dots has a "before" snapshot only and shows as not recorded.
