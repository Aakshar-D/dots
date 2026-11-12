# Workspace guardrails: design

Extends `2026-10-28-accounts-members-design.md` (the member list, roles and workspaces).

## 1. Intent

Roles limit the person, not their dots. Today a Member can set a dot to Trusted and let it push to `main`, run `rm -rf`, or edit a migration, and can approve their own dot's risky calls. An Admin has no way to say "this is critical here".

Guardrails close that gap. An Admin or Owner marks, per workspace, what is protected: paths, git actions and commands, plus the highest tool policy a Member's dot may use. Dots then refuses those calls on a Member's dot, and always asks before them on an Admin's dot.

The owner approved this design on 2026-11-11 with no open questions. Calls marked "my call" are the builder's.

## 2. Goals and non-goals

**Goals**

- Guardrails per workspace, in the signed member list, edited by an Admin or Owner and signed like the rest of the list.
- Enforcement in core, by the role this PC holds in the dot's workspace, for both engines.
- A Member can't approve a protected call, can't save an allow rule for one, and can't save a dot policy above the workspace's ceiling.
- `dots-admin` commands to set guardrails, and the Tauri commands the UI needs.

**Non-goals**

- Guardrails in Personal, or in an account with no member list (`legacy`). Neither has an Admin to ask.
- Protecting against a person who patches their own Dots. Like roles (members spec §11), guardrails stop mistakes and honest use.
- Reading what a script the dot wrote does when it runs (the same limit as the send-and-delete guard).

## 3. The model

Each workspace on the member list may carry `guardrails`:

```json
{
  "id": "payments",
  "name": "Payments",
  "guardrails": {
    "protected_paths": [".github/**", "**/migrations/**", "infra/**", "**/*.env*",
                        "**/Cargo.lock", "**/package-lock.json", "**/bun.lock",
                        "**/yarn.lock", "**/pnpm-lock.yaml"],
    "protected_git": ["push_main", "force_push", "delete_branch", "tags", "merge_pr", "release"],
    "protected_commands": ["rm -rf", "terraform apply", "terraform destroy", "kubectl apply",
                           "kubectl delete", "helm install", "helm upgrade", "helm uninstall",
                           "docker push", "npm publish", "cargo publish", "vercel --prod",
                           "netlify deploy", "fly deploy", "wrangler deploy", "firebase deploy"],
    "member_max_preset": "sandboxed"
  }
}
```

- **No `guardrails` field means no guardrails.** Every list signed before this feature keeps today's behaviour.
- `dots-admin roster workspace guard <roster.json> <id> --defaults` and the in-app Guardrails tab write the defaults above.
- All four fields are required when `guardrails` is given. Unknown fields fail the whole list, as everywhere in the list.

### 3.1 Fields

`protected_paths`: globs, matched against the path relative to the run's folder. `*` stays within one folder, `**` crosses folders, `?` is one character. Matching ignores case and treats `\` as `/`. `dir/**` also covers `dir` itself. A leading `**/` also matches at the top. A bare folder (no glob characters, or ending in `/`) means everything in it: `deploy` and `deploy/` are stored as `deploy/**`, and one signed without that is read the same way. While any path is protected, a Windows short (8.3) name such as `MIGRA~1` is a hit, since the long name it stands for is unknown. A path outside the run's folder is matched at every folder depth (fails closed). Rules: 1 to 200 characters, relative, no `..`, no `(`, `)`, `,` or control characters, at most 100 entries, no duplicates.

`protected_git`: any of

| Value | Covers |
|---|---|
| `push_main` | `git push` to `main` or `master` (`refs/heads/` or `heads/` before it is ignored), `--all`, `--mirror`, `--branches`, a refspec with `*`, and a bare `:`. A push that doesn't name the branch (`git push`, `git push origin`, `git push -u origin HEAD`) is decided by the branch checked out in the run's folder, read from `.git/HEAD` (in a worktree, through the `gitdir` file `.git` names): on a named branch other than main or master it is allowed; on main or master, detached, unreadable, or after the line changes folder or repository (`cd`, `git -C`), it is `push_main`. |
| `force_push` | `git push -f`, `--force`, `--force-with-lease`, `--force-if-includes` (and any prefix git accepts, `--force-w`), a `+` refspec. |
| `delete_branch` | `git branch -d/-D/--delete`, `git push --delete`, a `:branch` refspec. |
| `tags` | `git tag <name>` (making or deleting a tag), `git push --tags`, `--follow-tags`, a `refs/tags/` refspec, `git push origin tag <name>`. Listing tags is not covered. |
| `merge_pr` | `gh pr merge`, `gh api` writes on `pulls/<n>/merge` and `merges`, and MCP tools whose name merges a pull request (`merge_pull_request`, `enable_auto_merge`). |
| `release` | `gh release create/edit/delete/upload/delete-asset`, `gh api` writes on `.../releases`, and MCP tools that create, edit or delete a release. |

No duplicates.

`protected_commands`: command patterns. A pattern is a command name and words, like `terraform apply`. It matches a simple command whose name is the first word (case, folder and `.exe` ignored) and which has every other word of the pattern anywhere among its words, in any order (`kubectl apply` matches `kubectl -n prod apply -f x.yaml`). `rm -rf` is read as "rm with recursive and force", so `rm -fr`, `rm -r -f`, `rm --recursive --force` and PowerShell's `Remove-Item -Recurse -Force` all match. Rules: 1 to 100 characters of letters, digits, spaces and `-_./=:@+`, at most 100 entries, no duplicates.

`member_max_preset`: `read-only`, `sandboxed` or `trusted` (not `custom`). Default `sandboxed`: Trusted is for Admin and Owner dots only.

### 3.2 Older Dots

The member list refuses unknown fields. A Dots from before this feature (0.4.0 and older) therefore refuses a list with `guardrails` in it. For it the account behaves as if the list were invalid: it keeps the last list it accepted (members spec §6.4) and, under policy `required` with nothing accepted, the PC shows Waiting. `dots-admin` prints a warning when it writes or signs a list with guardrails, and the in-app tab shows the same note before signing. The admin guide says: update every Dots before adding guardrails.

## 4. Who is guarded, and how

The role is this PC's role in the dot's workspace, `permissions::access_dot` (members spec §6.3): an account Owner or Admin is Admin in every workspace; an account Member is a Member (also where its workspace entry is `admin`: workspace admin rights cover API keys only).

| This PC in the dot's workspace | A protected call | Policy ceiling |
|---|---|---|
| Owner or Admin | always **asks**, like the send-and-delete guard; approvable | none |
| Member | **denied** with "This needs an Admin: <what> is protected in the <workspace> workspace." | `member_max_preset` |
| Viewer | runs nothing (unchanged); guarded as Member if a run happens anyway | as Member |
| Waiting, removed, outside the workspace | runs nothing (unchanged); guarded as Member | as Member |
| Personal, legacy account, workspace without guardrails | no guardrails | none |

`<what>` names the hit: "the file migrations/0001.sql", "pushing to main or master", "force pushing", "deleting a branch", "making or pushing tags", "merging a pull request", "publishing a GitHub release", "the command rm -rf".

### 4.1 Calls

`policy::rails::check(guardrails, tool, input, workspace)` reads a call and returns what it hits, or nothing:

- **File writes** (`Write`, `Edit`, `MultiEdit`, `NotebookEdit`): the path against `protected_paths`. Reads (`Read`, `Grep`, `Glob`, `LS`) are not guarded.
- **Shell** (`Bash`, `PowerShell`): the line is split the way the send-and-delete guard splits it (`policy::guard`: separators, quotes, substitutions, wrappers such as `sudo`, `env`, `xargs`, and the strings of `bash -c`, `cmd /c`, `powershell -Command`, `-EncodedCommand`, `eval`). Each simple command is read for git actions, `gh` actions and protected commands.
- **Git options and aliases**: long options are read with the prefixes git accepts (`--del`, `--tag`, `--mir`, `--follow`). A git subcommand that isn't one of git's own, `git -c alias.*`, `git config ... alias.*` and `gh alias set` make the line unclear.
- **`gh api`**: the values of `-H`, `-q`/`--jq`, `-t`/`--template`, `--hostname`, `-f`/`-F`, `--input` and `-X`/`--method` are skipped; the endpoint is matched by shape (`repos/<o>/<r>/...`): `pulls/<n>/merge` and `merges` (merge_pr), `releases...` (release), a write to `git/refs/heads/main|master` (push_main, and force_push with `force=true`), `DELETE git/refs/heads/<b>` (delete_branch), `git/refs/tags...`, `git/tags` and a `git/refs` POST of `refs/tags/...` (tags), `PUT`/`DELETE contents/...` (push_main: it commits to a branch). The method is GET unless `-X` or a field says otherwise. `gh api graphql` with a mutation (a `query=` field holding `mutation`, or `--input`) is a hit of its own.
- **Folders**: `cd`, `chdir`, `pushd`, `Set-Location` and `Push-Location` into a protected folder are a hit.
- **Paths in a shell line**: each word of the line with its quotes undone, redirection targets included, is matched whole and in its parts (split at `=`, `,` and spaces), unless every command on the line, and in its substitutions, only reads (`cat`, `type`, `less`, `head`, `tail`, `grep`, `rg`, `findstr`, `ls`, `dir`, `wc`, `stat`, `diff`, `Get-Content`, `Select-String`, and `git status/diff/log/show/blame/add` without `--output`/`-o`) and there is no `>`. On such a line a word Dots can't name (unquoted `$`, a backtick, `*`, `?`, `[`, `{`, `~`, `%` or a backslash escape outside a Windows path; `$` or a backtick inside double quotes) is a hit when, as a glob, it could match a protected name (`mig*`, `.git?ub`), or when it holds a protected path's literal part.
- **Interpreters and pipes**: `python`, `node`, `deno`, `bun` (but `bun test`, `bun install`, `bun run test`), `ruby`, `perl`, `php`, a shell given a script file, and `pwsh -File` are unclear; their inline code (`-c`, `-e`, `--eval`) is kept for the mentions. A shell, `pwsh`, `cmd` or `iex` that reads its commands from a pipe (`curl ... | bash`) is a hit whenever the workspace protects anything.
- **Fail closed**: a line Dots can't read (a command named by a variable, `eval` or `Invoke-Expression` of a variable, an encoded command that doesn't decode, an alias, an interpreter, nesting past 6 levels) is a hit when its text, or the text of a script inside it (`-c`, `-Command`, a decoded `-EncodedCommand`), mentions a protected thing: a git action word (`push`, `merge`, `release`, `tag`, `branch`), the first word of a protected command, or a protected path's literal part. For a Member, `git` and `gh` themselves count while any git action is protected.
- **MCP tools** (`mcp__<server>__<tool>`, gateway and CLI alike), by the tool's name: merging (`merge_pull_request`, `enable_auto_merge`), releases, `push_files`, `create_or_update_file`, `delete_file`, `update_ref` (push_main), `delete_branch`, `delete_ref` (delete_branch), `create_tag`, `create_ref` (tags), and deleting a repository.

### 4.2 Where it is enforced

- `ApprovalHub::check` and `check_mcp`, which every built-in tool call of both engines reaches (the Claude CLI through the `approve` tool, the local engine directly). For a Member the hit is denied before any one-shot grant is used. For an Admin a call the policy would allow asks instead; a deny stays a deny.
- **Claude CLI** (`engine/claude/args.rs`), for a dot with guardrails:
  - no `Bash`, `PowerShell`, `Write`, `Edit`, `MultiEdit` or `NotebookEdit` rule is in `--allowedTools`, so every shell command and every file write reaches `approve`, where Dots reads it (the CLI's own glob matching could be passed by case or a short 8.3 name);
  - `--setting-sources` is empty whatever `use_user_settings` says: the person's own settings could add a hook or a rule that skips `approve`;
  - the run's `--settings` file gets `ask` rules, whatever the role: `Write(<glob>)`, `Edit(<glob>)`, `MultiEdit(<glob>)`, `NotebookEdit(<glob>)` for each protected path, and the shell prefixes the CLI can express (`git push origin main`, `git push origin master`, `git push --force`, `git push -f`, `git push --delete`, `git push --tags`, `git branch -D`, `git branch -d`, `gh pr merge`, `gh release create|edit|delete|upload`, and each protected command, as `Bash(<cmd>:*)` and `PowerShell(<cmd>:*)`). Ask beats allow, so these calls reach `approve` even under an allow rule;
  - none of them go in `--disallowedTools`: a CLI deny would refuse a Member's call in the CLI's own words ("Permission to use Bash ... has been denied") before Dots could say "This needs an Admin". The hub refuses it instead. The dot's own policy denies stay in `--disallowedTools`.
- **Approvals**: `Runtime::decide_approval` refuses to approve a protected call when this PC is a Member in the dot's workspace ("This needs an Admin: ..."), and refuses any allow rule with a protected call, whatever the role (`GUARDRAIL_REFUSAL`: "this is protected by the workspace's guardrails, so it always asks: no rule can allow it"). `approval_rule_candidates` offers none for it. Such an approval can exist when the list changed while it waited.

### 4.3 Policy ceiling

A Member's dot may use a policy up to `member_max_preset`. `accounts::guardrails::over_ceiling(policy, max)` says why a policy goes past it, or nothing. Order `read-only` < `sandboxed` < `trusted`. With `trusted` nothing is past. Otherwise a policy is past the ceiling when:

- its preset is a named preset above the ceiling (Trusted above Sandboxed);
- its `default` or `mcp_default` allows more than the ceiling preset's (allow > ask > deny);
- "Always ask before sending or deleting" is off;
- it lacks a deny rule the ceiling preset has (Sandboxed's `.git` and `.claude` rules; Read-only's `Write`, `Edit`, `NotebookEdit`, `Bash`, `PowerShell`);
- an allow rule reaches past the ceiling (my call on what "reaches past" means, so approving "always allow `npm test`" doesn't push a dot over):
  - under both: `*`, a prefix rule (`mcp__*`), or a bare `Bash`, `PowerShell`, `Write`, `Edit`, `MultiEdit`, `NotebookEdit`, `WebFetch`;
  - under Read-only: any `Bash`, `PowerShell`, `Write`, `Edit`, `MultiEdit`, `NotebookEdit`, `WebFetch` or `WebSearch` rule, or an MCP tool rule;
  - under Sandboxed: a file rule with an absolute path or `..`, or a shell rule whose command is a wrapper (`sudo`, `env`... as `policy::WRAPPER_COMMANDS`) or runs code: `python`, `python3`, `py`, `node`, `deno`, `bun` (but `bun test` and `bun run`), `ruby`, `perl`, `php`, `bash`, `sh`, `zsh`, `pwsh`, `powershell`, `cmd`, `npx`. `make`, `npm run` and `npm test` stay within it.
- `use_user_settings` is on (for every ceiling): the person's own Claude Code settings are outside Dots' view.
  
  Rules for the read tools, Dots' own tools, plumbing tools, one named MCP tool (`mcp__github__create_issue`), a `WebFetch(domain:x)` and a plain shell command (`Bash(npm test:*)`) stay within Sandboxed.

Enforced:

- **Saving**: `create_dot`, `move_dot`, approving with a rule, and `update_dot` when the policy or `use_user_settings` changes, refuse a policy past the ceiling. A save that leaves both as they were goes, so a held dot can still be renamed or switched off. They refuse it with `NotAllowed`: "Acme: a Member's dot in the Payments workspace may use up to Sandboxed, and this policy is Trusted. Ask an Admin or Owner." (the reason after "and" is the first one found).
- **Starting by hand**: Run now and a chat are refused with the same words.
- **Dots already past it** (made before the guardrails arrived, or before a role went down): every queued run of the dot is held, unstarted, with `HoldReason::Role` and the text "Acme: this dot's tool policy is past what a Member may use in the Payments workspace (it is Trusted). Change the policy, or ask an Admin." It is the Viewer "Waiting to run" hold (`Runner::role_hold`), so it shows in the Inbox and Run anyway does not release it. It starts once the policy is brought down, or the role rises; saving a dot wakes the dispatcher.

### 4.4 Unknown guardrails

A dot of an account this PC has joined, whose guardrails it can't read, doesn't run: the account's settings are damaged, a member list is required but none was accepted, or the dot's workspace isn't on the accepted list. Its queued runs are held like the ceiling hold, with "Acme: Dots can't read the guardrails of this dot's workspace (<why>), so it doesn't run it. Refresh the account, or ask an Admin.", and Run now and chat are refused with it. Should a call of it reach the hub anyway, it is checked against the default guardrails as for a Member (`accounts::enforced_rails`); so is a call of a dot whose account isn't joined, which other checks already stop. Personal, a legacy account (no member list) and a workspace with no guardrails stay unguarded.

### 4.5 Changes during a run

The hub reads the guardrails and the role again on every call, and every shell command and file write of a guarded dot reaches it (§4.2), so a change to the list takes effect at the next call. The CLI's own lists (`--disallowedTools`, `--settings`) are written when the run starts and are not rewritten mid-run: a dot that gained guardrails during a run is still checked by the hub, but its CLI allow list is the one it started with. Restarting the CLI with fresh flags at the next turn is left for later.

## 5. Permissions

New `Permission::ManageGuardrails` ("change guardrails"), Owner and Admin, like `ManageWorkspaces`. Signing still needs the PC's Dots ID in the policy's `admins`.

The matrix goes to the UI as data, so the "What each role can do" table never decides a role itself (members spec §9): `Permission::ALL`, each with its action words, the roles that have it, and a note where a role has it with a condition.

## 6. Editing

- **In Dots**: `RosterEdit::SetGuardrails { workspace_id, guardrails }` (`null` removes them) goes through `Orgs::edit_members` like every list edit (fetch again, base `seq`, sign with this PC's Dots ID, publish or keep a draft). It needs `ManageGuardrails`. The guardrails are checked with the §3.1 rules first.
- **dots-admin**:
  - `roster workspace guard <roster.json> <id>` shows the workspace's guardrails.
  - `... --defaults` sets the defaults; `--off` removes them.
  - `--path <glob>`, `--git <action>`, `--command <pattern>` (each repeating) replace that list; `--no-paths`, `--no-git`, `--no-commands` empty one; `--member-max <preset>` sets the ceiling. Without `--defaults`, they start from the workspace's guardrails, or the defaults when it has none.
  - It writes the file only (sign afterwards) and warns about older Dots. `roster sign` warns too when any workspace has guardrails. `roster check` and `verify` list each workspace's guardrails.

## 7. What the UI gets (PR B)

Every command is in `apps/dots-app/src/commands.rs`, wrapped in `ui/src/api/commands.ts`, typed in `ui/src/api/types.ts` and sampled in the contract fixture.

| Command (`commands.ts`) | Arguments | Answer |
|---|---|---|
| `workspace_guardrails` (`workspaceGuardrails`) | `orgId`, `workspaceId` | `GuardrailsView` |
| `workspace_guardrails_set` (`workspaceGuardrailsSet`) | `orgId`, `baseSeq`, `workspaceId`, `guardrails: Guardrails \| null` | `EditOutcome` |
| `role_matrix` (`roleMatrix`) | none | `RoleRow[]` |
| `policy_ceiling` (`policyCeiling`) | `orgId: string \| null`, `workspaceId`, `policy: Policy \| null` | `PolicyCeiling` |
| `dot_ceiling` (`dotCeiling`) | `dotId` | `PolicyCeiling` |
| `approval_guardrail` (`approvalGuardrail`) | `id` | `ApprovalGuardrail \| null` |

`member_edit` also takes `{ kind: "set_guardrails", workspace_id, guardrails }`.

Shapes (serde names):

```ts
Guardrails = { protected_paths: string[], protected_git: GitAction[],
               protected_commands: string[], member_max_preset: Preset }
GitAction = "push_main" | "force_push" | "delete_branch" | "tags" | "merge_pr" | "release"
GuardRole = "member" | "admin"
GuardrailsView = {
  org_id: string, workspace_id: string, workspace_name: string,
  seq: number,                      // base_seq for workspace_guardrails_set
  guardrails: Guardrails | null,    // null: none
  defaults: Guardrails,             // what "Use defaults" fills in
  git_actions: { action: GitAction, label: string }[],
  edit_refusal: string | null,      // why this PC may not change them (role)
  sign_blocker: string | null,      // why this PC can't sign the list (not in admins)
  applies_as: GuardRole | null,     // how they treat this PC's dots here
  compat_note: string,              // the older-Dots warning, shown before signing
}
RoleRow = { permission: Permission, action: string, roles: Role[], note: string | null }
Permission = "view" | "edit_dots" | "run_dots" | "answer_approvals" | "grant_entries" | "add_mcp"
           | "create_api_keys" | "manage_members" | "manage_workspaces" | "sign_roster"
           | "edit_goals" | "manage_guardrails"
PolicyCeiling = {
  applies_as: GuardRole | null, workspace_name: string | null,
  max_preset: Preset | null,        // null: no ceiling for this PC here
  presets: { preset: Preset, refusal: string | null }[],  // read-only, sandboxed, trusted, custom
  policy_refusal: string | null,    // why the given (or the dot's) policy can't be saved
  held: string | null,              // dot_ceiling only: the hold text of its runs
  protected: Guardrails | null,     // what the workspace protects
}
ApprovalGuardrail = { what: string, workspace_name: string, needs_admin: boolean, text: string }
```

`presets[custom].refusal` is always null: a custom policy is judged by `policy_refusal`.

## 8. Testing

- Roster: parse with and without guardrails, each §3.1 rule, unknown field inside `guardrails`, `custom` as ceiling, round trip through `edit::apply`.
- The role x permission matrix as a table test, `ManageGuardrails` included.
- `rails::check`: each git action and its near misses, `gh`, protected commands and `rm` flag forms, `Remove-Item`, wrappers and `bash -c`, paths in file tools (relative, absolute inside and outside the folder), paths in shell lines (redirection, read-only commands), the fail-closed cases, MCP names.
- Ceiling: each preset against each ceiling, custom rules in and past, approval candidates staying within Sandboxed.
- Policy resolution per role through `ApprovalHub` (Member denied, Admin asks, Personal and legacy unchanged, grants).
- A runner test with `fake-claude`: a Member dot's `git push origin main` and an `Edit` of `migrations/x.sql` are refused; the same calls on an Admin dot ask.
- Ceiling hold: a Trusted dot of a Member waits with the hold text and starts once lowered.
- Claude args: the added ask, deny and allow-list changes per role.
- `dots-admin`: `guard --defaults`, `--off`, options, refusals, warnings.
- Contract fixture for every new type and command.

## 9. Known gaps

- A script the dot writes and runs, or a program that pushes on its own (`npm run release`), isn't read.
- `push_main` can't see where a bare `git push` lands, so it refuses it for a Member (fail closed).
- MCP tools are read by name only.
- Guardrails are local enforcement, like roles: a person who patches Dots is not stopped (members spec §11).
