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

`protected_paths`: globs, matched against the path relative to the run's folder. `*` stays within one folder, `**` crosses folders, `?` is one character. Matching ignores case and treats `\` as `/`. `dir/**` also covers `dir` itself. A leading `**/` also matches at the top. A path outside the run's folder is matched at every folder depth (fails closed). Rules: 1 to 200 characters, relative, no `..`, no `(`, `)`, `,` or control characters, at most 100 entries, no duplicates.

`protected_git`: any of

| Value | Covers |
|---|---|
| `push_main` | `git push` to `main` or `master`. Also a push that doesn't name the branch (`git push`, `git push origin`, `git push origin HEAD`), `--all` and `--mirror`: Dots can't tell where it lands (my call: fail closed). |
| `force_push` | `git push -f`, `--force`, `--force-with-lease`, `--force-if-includes`, a `+` refspec. |
| `delete_branch` | `git branch -d/-D/--delete`, `git push --delete`, a `:branch` refspec. |
| `tags` | `git tag <name>` (making or deleting a tag), `git push --tags`, `--follow-tags`, a `refs/tags/` refspec, `git push origin tag <name>`. Listing tags is not covered. |
| `merge_pr` | `gh pr merge`, `gh api` calls on `.../merge`, and MCP tools whose name merges a pull request (`merge_pull_request`). |
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
- **Paths in a shell line**: any word of the line (redirection targets included) that matches a protected path is a hit, unless every command on the line only reads (`cat`, `type`, `less`, `head`, `tail`, `grep`, `rg`, `findstr`, `ls`, `dir`, `wc`, `stat`, `diff`, `Get-Content`, `Select-String`, and `git status/diff/log/show/blame/add`) and there is no `>` redirection.
- **Fail closed**: a line Dots can't read (a command named by a variable, `eval` or `Invoke-Expression` of a variable, an encoded command that doesn't decode, nesting past 6 levels) is a hit when its text mentions any protected thing: a git action word (`push`, `merge`, `release`, `tag`, `branch`), the first word of a protected command, or a protected path's literal part (`.github`, `migrations`, `.env`, `cargo.lock`...).
- **MCP tools** (`mcp__<server>__<tool>`, gateway and CLI alike): only `merge_pr` and `release`, by the tool's name.

### 4.2 Where it is enforced

- `ApprovalHub::check` and `check_mcp`, which every built-in tool call of both engines reaches (the Claude CLI through the `approve` tool, the local engine directly). For a Member the hit is denied before any one-shot grant is used. For an Admin a call the policy would allow asks instead; a deny stays a deny.
- **Claude CLI** (`engine/claude/args.rs`), for a dot with guardrails:
  - shell commands are never in `--allowedTools`, so every command reaches `approve`;
  - the run's `--settings` file gets `ask` rules `Write(<glob>)`, `Edit(<glob>)`, `MultiEdit(<glob>)`, `NotebookEdit(<glob>)` for each protected path, so an allowed `Write(./**)` still reaches `approve`;
  - for a Member, `--disallowedTools` also gets those path rules and the shell prefixes the CLI can express: `git push origin main`, `git push origin master`, `git push --force`, `git push -f`, `git push --delete`, `git push --tags`, `git branch -D`, `git branch -d`, `gh pr merge`, `gh release create|edit|delete|upload`, and each protected command (`Bash(<cmd>:*)`, `PowerShell(<cmd>:*)`). The `approve` path stays the backstop for everything else.
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
  - under Sandboxed: a file rule with an absolute path or `..`, or a shell rule whose command is a wrapper (`bash`, `sudo`, `env`... as `policy::WRAPPER_COMMANDS`).
  
  Rules for the read tools, Dots' own tools, plumbing tools, one named MCP tool (`mcp__github__create_issue`), a `WebFetch(domain:x)` and a plain shell command (`Bash(npm test:*)`) stay within Sandboxed.

Enforced:

- **Saving**: `create_dot`, `update_dot`, `move_dot` and approving with a rule refuse a policy past the ceiling with `NotAllowed`: "Acme: a Member's dot in the Payments workspace may use up to Sandboxed, and this policy is Trusted. Ask an Admin or Owner." (the reason after "and" is the first one found).
- **Starting by hand**: Run now and a chat are refused with the same words.
- **Dots already past it** (made before the guardrails arrived, or before a role went down): every queued run of the dot is held, unstarted, with `HoldReason::Role` and the text "Acme: this dot's tool policy is past what a Member may use in the Payments workspace (it is Trusted). Change the policy, or ask an Admin." It is the Viewer "Waiting to run" hold (`Runner::role_hold`), so it shows in the Inbox and Run anyway does not release it. It starts once the policy is brought down, or the role rises; saving a dot wakes the dispatcher.

## 5. Permissions

New `Permission::ManageGuardrails` ("change guardrails"), Owner and Admin, like `ManageWorkspaces`. Signing still needs the PC's Dots ID in the policy's `admins`.

The matrix goes to the UI as data, so the "What each role can do" table never decides a role itself (members spec §9): `Permission::ALL`, each with its action words, the roles that have it, and a note where a role has it with a condition.

## 6. Editing

- **In Dots**: `RosterEdit::SetGuardrails { workspace_id, guardrails }` (`null` removes them) goes through `Orgs::edit_members` like every list edit (fetch again, base `seq`, sign with this PC's Dots ID, publish or keep a draft). It needs `ManageGuardrails`. The guardrails are checked with the §3.1 rules first.
- **dots-admin**:
  - `roster workspace guard <roster.json> <id>` shows the workspace's guardrails.
  - `... --defaults` sets the defaults; `--off` removes them.
  - `--path <glob>`, `--git <action>`, `--command <pattern>` (each repeating) replace that list; `--member-max <preset>` sets the ceiling. Without `--defaults`, they start from the workspace's guardrails, or the defaults when it has none.
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
