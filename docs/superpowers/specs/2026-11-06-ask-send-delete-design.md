# Ask first: a dot always asks before it sends or deletes

- Date: 2026-11-06
- Status: built (defaults chosen by the builder, as the owner prefers; see §7)
- Builds on: the per-dot policy (`policy.rs`), the approval guards (2026-11-03: private folders, plumbing tools), the browser gate (`browser/gate.rs`, risky words), the MCP gateway (`resolve_mcp`), the Claude engine's argument list (`engine/claude/args.rs`).

## 1. Goal

Part of "Permissions you control": an approval step for sending and deleting, set in the UI. Until now only the built-in browser asked before risky acts (its `RISKY_WORDS`), and a dot with the Trusted preset or a wide allow rule (`Bash(git:*)`, `mcp__gmail__*`) could send an email or delete a folder without anyone seeing it.

## 2. Words

- **The switch**: "Always ask before sending or deleting", one per dot, stored as `Policy::ask_send_delete`.
- **Guarded call**: a call that `policy::guard::classify` finds sending (an email, a message, a post, a push, a pull request, data to a web address), deleting (files, branches, records, history), or running a command Dots cannot read.

## 3. Behaviour

- While the switch is on, a guarded call that the rules, the preset or the default would **allow** asks instead. A **deny** still refuses it, and a call that would ask anyway still asks. Plumbing tools (`TodoWrite`, `Task` ...), file tools (`Read`, `Write`, `Edit`), web searches and fetches, and Dots' own tools (check-ins, memory, workflows, instructions) are never guarded.
- Where it applies:
  - `Policy::resolve_in` (every built-in tool call of the Claude and local engines, and an MCP tool the CLI reached outside the gateway);
  - `Policy::resolve_mcp` (every call through the MCP gateway: connectors and Library servers). A tool's accepted `readOnlyHint` does **not** exempt a tool named to send or delete: the name wins.
  - the browser gate (§5).
- The approval is the ordinary one: an Inbox card, the run waits as it does for any approval (`approval_wait_secs`), then parks; a background run with nobody watching waits on the card like every other approval. Approve once or Deny, with a note.
- No rule can be saved with the approval of a guarded call while the switch is on: the card offers none, and `decide_approval` refuses a forged one (`GUARD_REFUSAL`). An allow rule would not stop the next call from asking, so offering one would promise what it cannot do. The person can switch the guard off for the dot.
- The card says plainly why, above the exact command or arguments: "Scout wants to send an email with Gmail (send_email)", "Scout wants to delete 3 files: a.txt, b.txt, c.txt", "Scout wants to force-push or delete on a remote with git ... (git push --force origin main)". Under it: "This sends or deletes, so it always asks while "Always ask before sending or deleting" is on for Scout. You can switch that off in the dot's permissions." For an unreadable command: "Dots can't tell whether this sends or deletes, so it always asks ...". With the switch off the sentence still says what the call does (it is true and useful), without the always-asks line.

### 3.1 The CLI's own allow list

The Claude CLI runs a call that matches `--allowedTools` without asking Dots. While the switch is on, no `Bash(...)`/`PowerShell(...)` allow rule is passed to the CLI: every shell command reaches `mcp__dots__approve`, where `resolve_in` reads it. Dots answers a harmless one with an allow at once, so the cost is one local round trip per command. MCP calls already all go through the gateway; the browser through its gate.

## 4. The classifier (`crates/dots-core/src/policy/guard.rs`)

Lexical, conservative: a false positive costs one click; a false negative lets a dot send or delete unseen.

### 4.1 Shell commands (`Bash`, `PowerShell`, the local engine's `shell`)

- The line is split into simple commands at `;`, `&&`, `||`, `|`, `&`, new lines, `( ) { }`, and the bodies of `$(...)`, `` `...` ``, `<(...)` are read as commands of their own (also inside double quotes). Quotes are undone (`r'm'` is `rm`), `\;` stays a word, a redirection's target is dropped (`ls > rm` runs `ls`), `#` comments are skipped. Leading `VAR=value`, `$x =`/`$x=`, and shell keywords (`if then else elif do while until !`) are skipped.
- A command name is read without its folder, leading `\`, `&`, and `.exe/.cmd/.bat/.com`, in lower case (`/bin/rm`, `\rm`, `RM.EXE`, `& "C:\...\cmd.exe"`).
- Wrappers (`sudo`, `doas`, `env`, `nohup`, `time`, `timeout`, `nice`, `xargs`, `npx`, `bunx`, `wsl`, `ssh`, `start`, `Start-Process`, `ForEach-Object`/`%`, ...) are looked through: every later word is tried as the start of a command, and every argument with a space is read as a command line (`ssh host 'rm -rf x'`, `Start-Process cmd -ArgumentList '/c del x'`).
- `sh/bash/zsh/... -c '<line>'`, `cmd /c <line>`, `powershell`/`pwsh` `-Command <line>` (or a bare command), `-EncodedCommand <base64>` (decoded from UTF-16LE; one Dots cannot decode is **unclear**), `eval`, `iex`/`Invoke-Expression`: the string is read as a command line, up to 6 levels deep (deeper is unclear). `eval`/`iex` of a variable (`iex $payload`) is unclear.
- Deletes: `rm`, `unlink`, `shred`, `srm`, `trash`, `rimraf`, `del`, `erase`, `rmdir`, `rd`, `truncate`, `Remove-Item`/`ri` and every `Remove-*` cmdlet, `Clear-Content`/`clc`, `Clear-Item`, `Clear-RecycleBin`, `find -delete` and `find -exec <delete>`, disk tools (`mkfs*`, `format`, `diskpart`, `dd of=`, `cipher /w`), `gio trash`; git: `clean`, `reset --hard/--merge/--keep`, `push` with `-f/--force*/--delete/-d/--mirror/--prune`, a `+refspec` or `:branch`, `branch -d/-D/--delete`, `tag -d`, `rm`, `stash drop/clear`, `checkout -- / . / -f`, `restore` (not `--staged` alone), `switch -f`, `worktree/remote/submodule remove`, `reflog expire`, `update-ref -d`, `filter-branch/filter-repo`, `gc --prune`/`prune`; `gh ... delete/remove/archive`, `gh api -X DELETE`; `curl -X DELETE`, `Invoke-RestMethod -Method Delete`; `rsync --delete*`; cloud and tool CLIs (`aws az gcloud kubectl docker podman terraform helm heroku vercel firebase redis-cli npm ...`) whose first four operands hold `delete rm rmi rb remove destroy purge prune drop flushall unpublish uninstall yank ...` or `delete-*`, `remove-*`, `terminate-*`, `*:destroy` (a package manager's `uninstall/remove/rm`, which only changes the project's packages, is not one); SQL in a database client's command line (`sqlite3`, `psql`, `mysql`, `sqlcmd`, `mongosh`, `redis-cli` ...: `DELETE FROM`, `DROP TABLE/DATABASE/...`, `TRUNCATE`, `FLUSHALL`, also in a heredoc); code in the line (`shutil.rmtree`, `os.remove(`, `.unlink(`, `.rmSync(`, `[IO.File]::Delete(`, `.delete()`, `deleteMany(` ...).
- Sends: `git push` (plain), `git send-email`; `gh pr create/comment/merge/review/edit/close`, `gh issue create/comment/edit/close`, `gh release create`, `gh repo create/fork`, `gh gist create`, `gh secret set`, `gh workflow run`, `gh api` with `-X POST/PUT/PATCH` or fields (`-f/-F`); `curl` with `-X POST/PUT/PATCH`, `-d`, `--data*`, `-F`, `--form`, `-T`, `--json`; `wget --post-*`; `Invoke-WebRequest`/`iwr`/`Invoke-RestMethod`/`irm` (and PowerShell 5.1's `curl`/`wget` aliases) with `-Method Post/Put/Patch` or `-Body`/`-InFile`/`-Form`; `http POST ...`; mail senders (`sendmail`, `mail`, `mutt`, `msmtp`, `swaks`, `Send-MailMessage`), every `Send-*`, `Publish-*`, `Submit-*` cmdlet; `npm/cargo/twine/gem/poetry publish`, `docker push`; `scp`/`rsync` to `host:path`; code (`requests.post(`, `smtplib`, `fetch` with `method: 'post'` ...).
- Not guarded: `echo rm -rf x`, `grep 'rm -rf'`, `git commit -m "remove old code"`, `git push --help`, `git branch -a`, `git restore --staged`, `npm uninstall`, `cargo remove`, `docker run --rm`, `curl https://x`, `curl -X GET`, `Copy-Item`, `mv`, `Write-Output 'Remove-Item'`.

### 4.2 MCP and app tools

The tool's name as words (split at `_ - . space` and camelCase: `chat.postMessage` is `chat post message`).

- Delete if a word is one of `delete remove rm del archive trash destroy purge erase wipe drop unlink truncate discard obliterate shred unpublish terminate`.
- Otherwise not guarded when it drafts (`draft`/`drafts` without `send`: `create_draft`, `draft_reply`) or starts with a read verb (`get list search read fetch find query view show describe count lookup download check preview export browse inspect watch`) without `send`.
- Otherwise send if a word is one of `send post reply respond publish share invite forward comment tweet retweet broadcast push merge submit notify email mail dm message react upload transfer`, or a making verb (`create add new write compose open start`) comes with a sent thing (`message email comment issue pr pull review event invitation meeting chat dm tweet status reply discussion release gist webhook post thread announcement reaction`).
- Examples: `send_email`, `send_draft`, `reply_to_email`, `chat.postMessage`, `add_reaction`, `create_issue`, `add_issue_comment`, `create_pull_request`, `merge_pull_request`, `push_files`, `create_event` (invites attendees), `share_file` send; `delete_email`, `trash_message`, `archive_thread`, `delete_branch`, `drop_table` delete; `search_emails`, `get_message`, `list_issues`, `get_post`, `create_draft`, `query_database` do not.
- An app tool (`App`, the input's `tool`) is read the same way, for deletes only: "delete task with the app projects (delete_task)". An app keeps its data on this machine, so its `comment` or `send` sends nothing to anyone.
- Dots' own tools (`mcp__dots__*`) and the built-in browser (`mcp__browser__*`) are left to their own checks.

## 5. The browser

The gate's `RISKY_WORDS` already make a risky act ask unless `BrowserRisky(<site>)` allows it. While the switch is on, an act whose target's name or role holds a `GUARD_WORDS` word (`send delete remove post publish submit reply share comment forward invite tweet trash archive discard erase destroy empty unsubscribe transfer merge push`), an act on a target that is not a reference of the snapshot (a selector could name a Send button), and accepting a dialog (it may confirm a delete) ask even where `BrowserRisky` allows risky acts. A deny still refuses. The card (`Ask.guarded`) says "This may send or delete, so it always asks while ... is on", and offers no "Always allow risky actions" rule for it ("Always allow on <site>" is still offered: it covers going there and plain acts). "Pay now" is risky but neither sends nor deletes: `BrowserRisky` still allows it.

## 6. Defaults and migration

- Every preset (`Policy::preset`) has the switch on, so a new dot starts with it on.
- A policy stored before the switch existed reads as **on** (`#[serde(default = true)]`): existing dots get the guard at once, without a migration. *Why:* this is a safety feature, and the people who want sends and deletes to go through unasked can switch it off per dot in one click; the other way round, a dot that sends an email nobody approved cannot be undone. No SQL migration is needed because the policy is stored as JSON in the dot's spec.
- Choosing another preset in the editor keeps the person's switch as they set it.
- No Settings-wide default: one switch per dot, next to the preset, is where the person already decides what a dot may do. A Settings default can come later if people ask.

## 7. Decisions the builder made

- The guard is absolute while on: no "always allow this" for a guarded call. The person switches it off instead.
- Opening and commenting on GitHub pull requests and issues, merging, pushing files, creating calendar events (which invite attendees) count as sending. Creating a page or a record in a workspace tool (Notion `create_page`, a database row) does not: it publishes nothing to other people by itself.
- Drafting is not sending; sending a draft is.
- Archiving counts as deleting (it takes the item out of the person's view and is how several connectors delete).
- The browser's guarded acts include an unknown target and accepting any dialog: conservative, at the cost of an extra question.
- `git push` of new commits counts as sending (it publishes the work); a force or delete push counts as deleting.

## 8. Known gaps

The guard is a lexical check in front of the real boundaries (the per-run worktree, the CLI deny lists). It cannot see:

- **Scripts**: a dot writes `cleanup.sh`, `deploy.py` or a `package.json` script and runs it (`bash cleanup.sh`, `python deploy.py`, `npm run clean`, `make clean`, `powershell -File x.ps1`). Only the command line is read, not the file.
- **Programs that send or delete on their own**: a build tool, a test that calls an API, `terraform apply` (which can destroy), `docker compose down -v`, `git gc`, a CLI the lists do not name.
- **Commands built at run time**: `$cmd args`, `& $exe`, `"r"+"m"` in a language, `alias x=rm; x f`, a function defined earlier in the session, `Set-Alias`. `eval`/`iex` of a variable asks as unclear; other forms do not.
- **Overwriting**: `> file`, `Set-Content`, `Out-File`, `mv a b` over an existing `b`, `cp` over a file, `Write`/`Edit` of a file all destroy content without "deleting".
- **Sending without a send word**: an MCP tool named `run`, `execute`, `call`, `update_record` or `http_request` that sends; a tool whose name lies. Its accepted MCP annotations (`destructiveHint`) are not read yet: Dots keeps only `readOnlyHint` today.
- **The browser**: pressing Enter in a message box sends it (a key is not a target with a name); a button labelled only with an icon has no word to match.
- **Settings the person brings**: with "Use my Claude Code settings" on, the user's own `~/.claude/settings.json` allow rules reach the CLI, which then runs a matching command without asking Dots.
- **Grants**: a call approved after its run parked is allowed once on the retry, as for every approval.
- **Quoting edge cases**: the tokenizer is a careful approximation of bash, cmd and PowerShell, not any one of them exactly. It leans to splitting more, so it over-reads rather than under-reads.

## 9. Files

- Core: `policy/guard.rs` (classifier, `Guarded`, `ApprovalGuard`), `policy.rs` (`ask_send_delete`, `guarded`, `resolve_in`, `resolve_mcp`, `GUARD_REFUSAL`), `engine/claude/args.rs` (shell rules kept from the CLI), `browser/gate.rs` (`GUARD_WORDS`, `Ask.guarded`), `runtime.rs` (`approval_guard`, no rules for a guarded call).
- App: the `approval_guard` command; the contract sample `ApprovalGuard`.
- UI: the switch in `PolicyEditor`, the sentence and the always-asks line on `ApprovalCard`, the line on `BrowserApprovalCard`.
- Tests: `tests/policy_guard_test.rs` (classifier tables and policy decisions), `browser_gate_test.rs`, `browser_call_test.rs`, `claude_cli_test.rs`, `runtime_test.rs`; UI `PolicyEditor.test.tsx`, `InboxScreen.test.tsx`, `BrowserApproval.test.tsx`.
