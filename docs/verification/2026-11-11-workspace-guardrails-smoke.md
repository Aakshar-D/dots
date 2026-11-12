# Workspace guardrails: two-PC smoke test

- Date: 2026-11-11
- Build: `bun run tauri build --debug --no-bundle` from branch `feat/guardrails-ui` (core #83 +
  UI #84), commit `20cc8e62` for steps 1 to 10, then again at `10058423` (the three UI fixes
  below) to check them. `dots-admin` built from the same branch (`cargo build -p dots-admin`).
  Claude CLI 2.1.322, model Haiku.
- Data: two Dots data folders on one PC, "PC A" and "PC B", sharing one fake OpenBao that holds
  a test org with a v2 policy and a member list. A is the account's Owner and its Dots ID is in
  the policy's `admins`. B starts as a Member, in Default and Payments.
- Run by: an agent driving both windows over WebView2's debugging port with
  `tools/smoke/webview.mjs`. Every step below was run.

## Setup

Run in Git Bash. `$QA` is any scratch folder; `$T` is the Cargo target folder
(`cargo metadata --format-version 1 --no-deps`, `target_directory`).

### Two PCs on one machine

Each PC gets its own data folder, WebView2 profile and debugging port. The Credential Manager
targets are `Dots/<install_id>/...`, and the install id is made per data folder, so each PC gets
its own Dots ID and the installed Dots' key is never touched.

```bash
# launch.sh <A|B> <debugging port>
mkdir -p "$QA/pc$1/data" "$QA/pc$1/webview"
export DOTS_DATA_DIR="$(cygpath -w "$QA/pc$1/data")"
export WEBVIEW2_USER_DATA_FOLDER="$(cygpath -w "$QA/pc$1/webview")"
export WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=$2"
exec "$T/debug/dots-app.exe"
```

Start `launch.sh A 9301` and `launch.sh B 9302`. The installed Dots holds port 47321, so give
each its own local server port, then restart it:

```bash
node tools/smoke/webview.mjs 9301 eval "invoke('get_settings').then(s=>invoke('update_settings',{settings:{...s,port:47421,autostart:false}}))"
node tools/smoke/webview.mjs 9302 eval "invoke('get_settings').then(s=>invoke('update_settings',{settings:{...s,port:47422,autostart:false}}))"
node tools/smoke/webview.mjs 9301 eval "invoke('restart_app')"   # "the page closed" is expected
node tools/smoke/webview.mjs 9302 eval "invoke('restart_app')"
```

Set each profile (Settings › Account): A "Asha Owner", `asha@qa.invalid`; B "Ben Member",
`ben@qa.invalid`. Copy each Dots ID (`invoke('identity_get')`, `public_key`).

### The org source

`tools/smoke/fake-openbao.py` is a small fake OpenBao: KV v2 reads and writes, and a token's
`lookup-self`, kept in a JSON file. It accepts the tokens it is given.

```bash
python -I tools/smoke/fake-openbao.py 8250 "$QA/kv.json" tok-a tok-b tok-admin &
```

`put.sh <policy|roster|env> <file>` stores a file the way `bao kv put` would:

```bash
case $1 in
  policy) body=$(python -I -c 'import json,sys; print(json.dumps({"data":{"DOTS_POLICY":open(sys.argv[1]).read().strip()}}))' "$2");;
  roster) body=$(python -I -c 'import json,sys; print(json.dumps({"data":{"DOTS_ROSTER":open(sys.argv[1]).read().strip()}}))' "$2");;
  env)    body=$(python -I -c 'import json,sys; print(json.dumps({"data":json.load(open(sys.argv[1]))}))' "$2");;
esac
curl -s --noproxy '*' -H "X-Vault-Token: tok-admin" -X POST --data "$body" \
  http://127.0.0.1:8250/v1/secret/data/dots/$1
```

### The org, policy and member list

```bash
export DOTS_ADMIN_PASSWORD=qa-pass
A=<A's Dots ID>; B=<B's Dots ID>
dots-admin keygen --out keys
dots-admin policy new --org-id qa-org --roster --admin $A --out policy.json
dots-admin policy sign policy.json --key keys/policy.key --out policy.signed.json
dots-admin roster new --org-id qa-org --owner-name "Asha Owner" --owner-email asha@qa.invalid \
    --owner-key $A --out roster.json
dots-admin roster workspace add roster.json Payments
dots-admin roster add roster.json --name "Ben Member" --email ben@qa.invalid --key $B \
    --role member --workspace payments=member
dots-admin roster sign roster.json --key keys/policy.key --out roster.signed.json
echo '{"LOG_LEVEL":"info"}' > env.json
put.sh env env.json; put.sh policy policy.signed.json; put.sh roster roster.signed.json
dots-admin join-code --org-id qa-org --name "QA Org" --source openbao \
    --address http://127.0.0.1:8250 --path dots --pub keys/policy.pub --file qa-org.dotsorg
```

A joins in Settings › Account › **Join an account** with the code and the token `tok-a`; B joins
with `tok-b`. Both show QA Org, Connected, list #1.

### The repositories

One per PC, each with a local bare remote, on `main`, with a commit ahead of the remote:

```bash
git init -q --bare -b main remoteB.git
git init -q -b main workB && cd workB
mkdir -p migrations .github && echo "-- original" > migrations/x.sql && echo "name: ci" > .github/ci.yml
echo hi > README.md && git add -A && git commit -qm init
git remote add origin ../remoteB.git && git push -q origin main
echo "local change" >> README.md && git commit -qam "local ahead"
```

The same for `workA` and `remoteA.git`. Dots below use the **Folder** mode (not a worktree), so
the run's folder is on `main`.

## Steps

### 1. Roles and the read-only tabs

Expect: on A (Owner), the account has **Members**, **Workspaces** and **Guardrails** tabs, all
editable. On B (Member) the same tabs show, every button off, each with the core's reason as its
tooltip; the Guardrails tab shows "QA Org: a Member can't change guardrails. Ask an Admin or
Owner." and the guardrails read-only. "What each role can do" lists Owner, Admin, Member and
Viewer, with "Change guardrails" for Owner and Admin only, and the notes under "Create or change
dots" and "Answer approvals".

Result: pass, 2026-11-11, build `20cc8e62`. The table matched the role matrix row by row. On B
the Members tab's controls say "QA Org: a Member can't manage members. Ask an Admin or Owner.",
and so do the **Workspaces** tab's (it could say "manage workspaces"; that wording comes from
before this branch). The reasons are tooltips only.

### 2. Changing B's role in Dots

Expect: on A, Members tab, set Ben's role to Viewer: the list is published (#2). On B, Refresh:
the account shows Viewer, and a new dot in QA Org has **Create dot** off with "QA Org: a Viewer
can't create or change dots. Ask an Admin or Owner." Set it back to Member (#3), Refresh: Create
is on again. Asha's own role menu is off with "Owners can only be changed with the account
owner's key, using dots-admin.", and the core refuses the same change with those words.

Result: pass, 2026-11-11. The Owner's change was also sent straight to `member_edit`; the core
answered `not_allowed` with the dots-admin sentence.

### 3. Setting guardrails

Expect: on A, Guardrails tab, workspace Payments: "Payments has no guardrails". **Use defaults**
fills the form; add `make deploy` to the commands. The older-Dots note shows above **Save**.
**Save** asks "Sign the guardrails of Payments?" with the same note; **Sign and save** publishes
and the list number rises (#3 to #4). On B, Refresh: the tab shows "Your dots here are guarded as
a Member's: protected calls are refused. Member list #4." and every path, git action and command.
**Turn off** then **Save** asks "Turn off the guardrails of Payments?" with "Its dots go back to
their own policies..." (#5); **Use defaults** and **Save** turns them on again (#6).

Then with `dots-admin`, from the list as published (the in-app edits are not in `roster.json`):

```bash
python -I -c "import json;kv=json.load(open('kv.json'));print(json.loads(kv['secret/data/dots/roster']['DOTS_ROSTER'])['roster'])" > roster.json
dots-admin roster workspace guard roster.json payments
dots-admin roster workspace guard roster.json payments --command "rm -rf" --command "make deploy"
dots-admin roster check roster.json --org-id qa-org
dots-admin roster sign roster.json --key keys/policy.key --out roster.signed.json
put.sh roster roster.signed.json
```

Both PCs, after Refresh, read list #7 with the commands `rm -rf` and `make deploy`.

Result: pass, 2026-11-11. The read-only list on B shows the git actions in lower case
("pushing to main or master") where A's form starts them with a capital; cosmetic.

### 4. The ceiling on a Member's dot

Expect: before step 3, B made "Held trusted" in Payments, Trusted, Folder `workB`. After the
guardrails, on B:

- A new dot in Payments: the Preset list has Trusted off, with "Trusted is off: QA Org: a
  Member's dot in the Payments workspace may use up to Sandboxed, and this policy is Trusted. Ask
  an Admin or Owner." and "Protected in Payments: 9 paths, 6 git actions, 2 commands".
- "Held trusted"'s editor shows the hold text and the refusal. **Run now** is refused with the
  ceiling sentence.
- A run of it from its schedule (a `once` two minutes ahead) waits in the Inbox under **Waiting
  to run** with "QA Org: this dot's tool policy is past what a Member may use in the Payments
  workspace (this policy is Trusted). Change the policy, or ask an Admin." There is no **Run
  anyway**.
- Lowering it to Sandboxed and saving starts the held run (it ran with the real CLI and
  succeeded).
- The held dot can still be renamed and switched off.

Result: pass after a fix, 2026-11-11. At `20cc8e62` **Save** of the held dot was off whenever its
policy was past the ceiling, so it could be neither renamed nor switched off in the editor,
though the core takes such a save. Fixed in `61ca02b6`: Save is held only when the policy or the
user-settings switch changed. Checked at `10058423`: renamed to "Held trusted renamed" and
switched off with **Save**, policy still Trusted. **Run now** is not greyed out for a held dot; a
click shows the refusal.

### 5. A Member's dot with the real Claude CLI

Expect: B's "Rails probe", Payments, Sandboxed, Folder `workB` on `main`, Haiku.

- "Run `git push origin main`; Edit `migrations/x.sql`": both refused, nothing pushed (the bare
  remote's `main` stays at `init`), the file unchanged.
- "Run `git push -u origin HEAD`; Write `.github/new.yml`" on `main`: the push is refused with
  "This needs an Admin: pushing to main or master is protected in the Payments workspace."; the
  write is refused.
- On a feature branch (`git checkout -b feat/qa` in `workB`), `git push -u origin HEAD` is not
  refused by the guardrails: it asks only because "Always ask before sending or deleting" is on,
  with no guardrails line. Approved, it pushes `feat/qa`.

Result: pass, 2026-11-11, with one open point. The CLI took every flag a guarded run gets
(`--setting-sources ""`, the trimmed `--allowedTools`, the deny rules, the settings file's `ask`
rules): every run started and ended `succeeded`. But where a call matches the Member's CLI deny
rules (`git push origin main`, `Edit(**/migrations/**)`, `Write(.github/**)`), the CLI refuses it
itself and the dot reads the CLI's words ("Permission to use Bash with command git push origin
main has been denied.", "File is in a directory that is denied by your permission settings."),
not "This needs an Admin: ...". Only calls that reach `approve` get Dots' sentence. The spec asks
for those deny rules (§4.2), so this is reported, not changed.

### 6. An Owner's dot

Expect: A's "Admin probe", Payments, Sandboxed, Folder `workA`: Edit `migrations/x.sql`, then
`git push origin main`. Each asks; the card says "The file migrations/x.sql is protected in the
Payments workspace, so it always asks." and "Pushing to main or master is protected in the
Payments workspace, so it always asks." No **Approve and always allow** is offered, and the core
refuses a rule (`decide_approval` with `Edit(migrations/**)`): "this is protected by the
workspace's guardrails, so it always asks: no rule can allow it". **Approve** on each: the file
changes and `remoteA.git` gets the commit.

Result: pass, 2026-11-11.

### 7. An approval that needs an Admin

Expect: B is made Admin on A (Members tab), B refreshes. B's "Rails probe" edits
`migrations/x.sql`: it asks. While it waits, A makes B a Member again; B refreshes. Without
reloading, the card shows **Needs an Admin** and "This needs an Admin: the file migrations/x.sql
is protected in the Payments workspace."; **Approve** is off with that text; **Deny** works and
the file is unchanged. As Admin, B's Guardrails tab says "your Dots ID isn't one of the account's
signing admins yet..." and stays read-only.

Result: pass after a fix, 2026-11-11. At `20cc8e62` the card kept its old state after the role
change until the page was reloaded (**Approve** stayed on; the core refused the click with the
right sentence). Fixed in `42ce0456`: an account change marks every approval's guardrail check
stale. Checked live at `10058423`. **Approve all** for several approvals of one run was not
seen: the real CLI asks for one call at a time, even for parallel tool calls, so no batch formed.
It is covered by `InboxGuardrails.test.tsx`.

### 8. Personal dots

Expect: B's "Personal probe" in Personal, Sandboxed, Folder `workB`: an Edit of
`migrations/x.sql` goes through without asking, and the editor shows nothing about guardrails.

Result: pass, 2026-11-11.

### 9. The older-Dots warning

Expect: the Guardrails tab shows "Dots 0.4.0 and older can't read a member list with
guardrails: they keep the last list they accepted, or show the account as Waiting. Update every
Dots in the account first." above **Save** and in the sign dialog. `dots-admin roster workspace
guard`, `roster check` and `roster sign` print the same note. (No old Dots was installed.)

Result: pass, 2026-11-11.

### 10. Narrow window and dark mode

Expect: at 800 px wide and in Dark, the Guardrails tab (form and read-only), the Members tab
and the role table read well, with no horizontal scroll.

Result: pass after a fix, 2026-11-11. At `20cc8e62` the sixth tab (Guardrails) made the tab row
wider than the account's box at 800 px, so the settings page scrolled sideways by 3 px. Fixed in
`10058423`: the tabs wrap to a second row. Dark mode had no contrast problems in the new
screens. Screenshots: `s10-*` in the run's folder.

## Found and fixed (UI, on `feat/guardrails-ui`)

- `61ca02b6`: a held dot can be saved when its policy stays as it is.
- `42ce0456`: an approval's guardrail line follows a role or guardrails change.
- `10058423`: the account's tabs wrap at 800 px.

## Open

- Step 5: a Member's dot sees the CLI's own refusal, not "This needs an Admin: ...", for calls
  the CLI deny rules cover. Design question for the spec.
- Step 4: **Run now** on a held dot is on and refuses on click.
- Step 1: the Workspaces tab's reason talks of managing members.
- Step 3: `roster.json` doesn't follow edits made in Dots; take the published list first.
