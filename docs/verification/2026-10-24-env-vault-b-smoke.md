# Env vault plan B smoke verification

- Date: 2026-10-25
- Build: debug (`bun run tauri build --debug --no-bundle` from `apps/dots-app`, then `cargo build -p dots-admin -p dots-core --bins`, into the shared target folder `C:/Users/aksha/dots/target` with `CARGO_BUILD_JOBS=2`), branch `feat/env-vault-b` at commit `c7e4dbb`. Windows 11. The test suites were not run again by this task.
- Part A (agent steps): 8 steps plus this file. All 8 passed. Each ran once, in order, on one data folder.
- Part B (the owner): 6 items, not run, for the owner.
- No secret value is written here. Every value was made up for the run (`dots-smoke-` and random hex), kept in the smoke folder and deleted with it.

## How the agent steps were run

- The app is the debug build, started with `DOTS_DATA_DIR` set to `data\` in a fresh folder from `mktemp -d`, `WEBVIEW2_USER_DATA_FOLDER` set to `webview\` in the same folder, `DOTS_CONFIRM=yes` and `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333`. The real data folder `%LOCALAPPDATA%\dots` and the installed Dots (`%LOCALAPPDATA%\Dots`) were not touched.
- The `PATH` given to the app leaves out `...\AppData\Roaming\npm`, the only folder that `which -a claude` found. With that `PATH`, `which claude`, `which claude.exe` and `which claude.cmd` found nothing, and no `claude*` file was in any of its folders. The real `claude` CLI was never run.
- The Claude program is a copy of `fake-claude.exe` in the smoke folder, saved in Settings as "Program path", then "Restart Dots". `status` then named the copy, version 2.1.295.
- The org was made in `org\` of the smoke folder with the HEAD build of `dots-admin` and the winget installs of `sops` 3.13.3 and `age` (`age-keygen`). A small script rebuilt the plain file from the signed policy and the current token, encrypted the whole file with `sops --encrypt --input-type json --output-type json --age <recipients>` (no unencrypted suffix or regex), and deleted the plain file.
- The window is driven with `node tools/smoke/webview.mjs 9333 click|fill|text|eval|goto|shot`. The driver clicks only buttons and links, so three controls were set through `eval` the way the page's own handlers see them: the `.dotsorg` file input (a `File` with the file's text, put in `input.files` with a `DataTransfer`, then a `change` event), the dot editor's tick box for `fake-mcp` (`click()`), and the native selects (Org, Mode, "Check as": the value setter, then a `change` event). The Tauri commands were called with `invoke(...)` through `eval` only to read what the page does not show: run statuses and events, `org_entries`, `env_list`, `status`, the webhook URL, and the run that `run_now` makes for a dot whose org was removed.
- `FAKE_CLAUDE_SCRIPT` and `FAKE_CLAUDE_ARGS_OUT` reach the fake CLI as plain Personal variables granted to the dot's shell, as in plan A's smoke. The org's policy has `personal: "fallback"`, and both resolved "from Personal" in the org dot (step 4). The fake CLI's script lists the tools of `fake-mcp` through the gateway (`mcp_list`), so each run starts the server.
- `fake-mcp.exe` writes a `start` record with its `FAKE_MCP_*` variables to a log in the smoke folder. The log was moved aside after each run, so each count below is that run's.
- Values were looked for with `grep -a -c -F` over `dots.db`, `dots.db-wal`, `vault\acme-eng.bin`, the log file, the app's stdout, the fake CLI's record and the server's logs. The server's own log is meant to hold the token; nothing else is. A server's value was matched by name ("token 1", "token 2") by a script that compares without printing.

## Part A: agent steps

### A1. Make the org

`dots-admin keygen --out keys --no-password` wrote `policy.key` and `policy.pub`, warned that the key is not password-protected, and printed the fingerprint `58A9 50C8 7B85 7590 FE7D 01C4 D87E 2A04` (32 hex digits in 8 groups of 4). `policy new --org-id acme-eng --out policy.json` wrote the defaults. In it, `plain` was set to `["DOTS_TEST_LEVEL"]` and `personal` to `"fallback"`. `policy check` printed `ok` and "plain entries, shown with their values: DOTS_TEST_LEVEL; every other org entry is a secret" and "personal entries: used when the org has no entry of that name". `policy sign` printed "signed the policy for org 'acme-eng', issued_at 2026-10-25T11:37:35Z, with key 58A9 50C8 7B85 7590 FE7D 01C4 D87E 2A04".

`age-keygen` made the admin key. The plain file held `env` `{DOTS_TEST_LEVEL, DOTS_SMOKE_ORG_TOKEN}` and the signed `policy`. It was encrypted to the admin key: both `env` values and the policy were `ENC[...]`, the file had 1 age recipient, and `grep -c` for the token in it was 0. `dots-admin join-code --org-id acme-eng --name "Acme Engineering" --source sops --location C:\Users\...\org\acme-eng.enc.json --pub keys/policy.pub --file acme.dotsorg` wrote the code and printed the same fingerprint.

Result: pass.

### A2. Join

Settings, Organisations, **Join an organisation**, then `acme.dotsorg` in "Or open a .dotsorg file".

Result: pass.
- The preview showed "Organisation: Acme Engineering acme-eng", "Source: SOPS file at C:\Users\aksha\AppData\Local\Temp\tmp.…\org\acme-eng.enc.json" and "Key fingerprint: 58A9 50C8 7B85 7590 FE7D 01C4 D87E 2A04", the one `keygen` printed, with "Check the key fingerprint with your admin before you go on."
- **Continue** showed "This PC's key" (an `age1...` key, 62 characters) with **Copy** and "Send this key to your admin, then press Join".
- **Join** showed "Waiting for your admin to add this PC's key", and the button became **Try again**.
- The file was encrypted again to both keys. **Try again** closed the dialog. The list showed Acme Engineering, "Connected", the source, the fingerprint, "Last fetch" and "Saved copy until" a day later. "Show the policy" gave the same lines as `policy check`.

### A3. The Env tab

Library, Env.

Result: pass. The "Acme Engineering" section said "Connected", then:
- `DOTS_SMOKE_ORG_TOKEN`, "secret", "Locked", "Managed by Acme Engineering", "Not used", and no value;
- `DOTS_TEST_LEVEL`, "plain", "Locked", its value, "Managed by Acme Engineering", "Not used".

Both rows say "Locked", as the guide says for `fallback`. `org_entries` answered `value: null` for the secret. `grep -c` for the token: the section's text 0, the `org_entries` answer 0, `dots.db` 0, `dots.db-wal` 0, `vault\acme-eng.bin` 0, the log 0. `AGE-SECRET-KEY` (which every age secret key starts with, the admin's and this PC's) and the admin's whole secret key: 0 in each of those files.

### A4. An org dot

Setup: `FAKE_CLAUDE_SCRIPT` and `FAKE_CLAUDE_ARGS_OUT` were added in Env as plain Personal variables. (The dialog's Secret switch is on by default. The first two were saved as secrets by mistake, removed, and added again with the switch off.) `fake-mcp` was added in Library, MCP servers, through the form: stdio, the copy of `fake-mcp.exe`, its script, and env `FAKE_MCP_TOKEN=${DOTS_SMOKE_ORG_TOKEN}`. **Test connection** said "could not start: DOTS_SMOKE_ORG_TOKEN is not set" (it tests with Personal), and the form said "Not in Personal: DOTS_SMOKE_ORG_TOKEN". After Save, in Edit, "Check as" Acme Engineering said "Checked as Acme Engineering: the tools changed. Review them in the list.", and **Review**, then **Accept**, accepted `whoami` "In Acme Engineering".

The dot was made in the editor: Org "Acme Engineering" (the picker offered Personal and Acme Engineering), Mode folder, `fake-mcp` ticked, and `DOTS_TEST_LEVEL`, `FAKE_CLAUDE_SCRIPT` and `FAKE_CLAUDE_ARGS_OUT` granted with **Grant to the shell**. The shell's list offered `DOTS_TEST_LEVEL`, `FAKE_CLAUDE_ARGS_OUT` and `FAKE_CLAUDE_SCRIPT`, not the secret, and said "Acme Engineering does not allow secrets in the shell, so its secrets are not offered here."

Result: pass.
- After **Create dot** the editor showed "Org: Acme Engineering" and "Its variables come from this org. It was set when the dot was made."
- Variables showed "DOTS_SMOKE_ORG_TOKEN · from Acme Engineering" under `fake-mcp`, and "DOTS_TEST_LEVEL · from Acme Engineering", "FAKE_CLAUDE_SCRIPT · from Personal" and "FAKE_CLAUDE_ARGS_OUT · from Personal" under Shell.
- **Run now** (run `01M5SDZHD5CM5Z5DVHKTGC0Q1D`) succeeded: `session_started`, the tool list of `fake-mcp` (`whoami`), `usage`, `finished`. The fake CLI's record has `DOTS_TEST_LEVEL` equal to the org's value and does not list `DOTS_SMOKE_ORG_TOKEN`; `grep -c` for the token in it: 0. The server's log has 1 `start` record, with `FAKE_MCP_TOKEN` equal to the token (`grep -c` 1).

### A5. Rotate

The file was encrypted again to both keys with a new token. **Refresh** in Settings: "Connected", with a new "Last fetch".

Result: pass. The next run (`01M5SE0XF67V5K18TMCJW5J7P4`) succeeded, and the server's `start` record had the new token, not the old one.

### A6. Offline

The encrypted file was moved out of its folder, then **Refresh**.

Result: pass. The status was "Offline (cannot read C:\Users\aksha\AppData\Local\Temp\tmp.…\org\acme-eng.enc.json: The system cannot find the file specified. (os error 2)); using the saved copy until 2026-10-26T11:44:29.492Z". A run (`01M5SE1NSHT6EHVMJGAZTK1ZM0`) succeeded: the fake CLI had the org's `DOTS_TEST_LEVEL`, and the server had the new token. The file was put back; **Refresh** gave "Connected".

Note: the time in the status is in UTC and in RFC 3339 form, while "Last fetch" and "Saved copy until" next to it are in local time.

### A7. Revoke

The file was encrypted to the admin key only, then **Refresh**.

Result: pass.
- The status was "Access revoked (this PC's key is not a recipient of the file)", and "Saved copy until" said "No saved copy".
- `vault\acme-eng.bin` (1,158 bytes before) was gone; the `vault` folder was empty.
- The next run (`01M5SE3192A91MVF3A8RZRDRN3`) has four `notice` events "Acme Engineering: access revoked", then `session_started`, `usage`, `finished` (status succeeded). The server did not start (no log), and the fake CLI ran its default script and wrote no record.

Note: the run got none of its four grants, the two Personal ones (`FAKE_CLAUDE_*`) included, and the notice is repeated once per grant. The run itself still counts as succeeded.

### A8. Remove

The file was encrypted to both keys again; **Refresh** gave "Connected" and a new `vault\acme-eng.bin`. Before Remove, `cmdkey /list:Dots/<install id>*` listed one target, `Dots/<install id>/org/acme-eng/source`.

**Remove** asked "Remove Acme Engineering?" with "The dots of Acme Engineering stay on this PC, but they can't run until you join it again. The saved copy of its variables and your sign-in details for it are deleted." and the buttons Cancel and Remove, the words of Task 13. Its Remove left "No organisation is joined on this PC."

Result: pass.
- Home: the dot's card says "org removed". The composer's field (labelled "org removed") and Send are off, and stay off after **Reply**. The page says "org removed: Its org 'acme-eng' was removed. Join it again to run this dot."
- The dot editor shows "Org: acme-eng", the "org removed" badge and "Its org 'acme-eng' was removed. Join it again to run this dot." **Run now** and **Save** are off, with that text as their tooltip. `run_now` called directly makes a run that fails at once with "dot 'org-smoke': its org 'acme-eng' was removed".
- A webhook token was made in the dot's Webhook section (**Make a token**). `POST` to the dot's URL with `Authorization: Bearer <token>` answered 202 and a run id. That run (trigger `webhook`) failed with "dot 'org-smoke': its org 'acme-eng' was removed"; the log has `run failed: dot 'org-smoke': its org 'acme-eng' was removed`.
- `vault\acme-eng.bin` is gone, and `cmdkey /list:Dots/<install id>*` lists no target, so `Dots/<install id>/org/acme-eng/source` is gone.

Note: the dot editor's tools section says "Not set in Dots' environment: DOTS_SMOKE_ORG_TOKEN. A server that needs one cannot start until it is set.", and the Library's server row says "Missing variable: DOTS_SMOKE_ORG_TOKEN" from step 4 on, while the org was connected too. Both check Personal only, so for an org dot they name a variable the org gives. The words also predate the vault ("Dots' environment").

### A9. This file

Written from the notes above. It is committed alone.

## Secrets at the end

`grep -a -c -F` for the first token, the second token, `AGE-SECRET-KEY`, the admin's secret key and the webhook token: 0 each in `dots.db`, `dots.db-wal`, the log file, the app's stdout and the fake CLI's record. Only the server's own `start` logs hold a token: the first run's the first token, the rotate and offline runs' the second.

## Cleanup

- The smoke app was stopped by its PID (13528, the process that "Restart Dots" started from the first one). `taskkill /PID` did not end it, so `taskkill /F /PID 13528` did. Then no `dots-app.exe`, `fake-claude.exe` or `fake-mcp.exe` was running.
- Credential Manager: the smoke's install id is `01M5SDNK9NG9AN9B8N9W393HY4`. After the app stopped, `cmdkey /list:Dots/01M5SDNK9NG9AN9B8N9W393HY4*` listed 0 targets, so there was nothing to delete. Remove had deleted the org's `source` target. The two Personal secrets saved by mistake in step 4 were removed in the app (`env_remove`) at once. No other target was listed, read or deleted.
- The smoke folder (data, webview data, the org's keys and files, the copies of the programs, the scripts and the made-up values) was deleted.

## Part B: for the owner (not run)

1. With the real `claude` CLI, logged in, run an org dot. The CLI runs, and its Bash tool sees the org's plain value (`echo $DOTS_TEST_LEVEL`) but no org secret while `shell_secrets` is false.
2. With a real OpenBao dev server (`bao server -dev`), follow `docs/vault-admin.md` §4:
   - put the entries and the signed policy, the read-only ACL and a userpass user;
   - join with the username and password, rotate a value, and cut the user off (delete the user and revoke its tokens): "access revoked".
   - The guide's `bao` commands were not run while building; correct the guide where they differ.
3. If an Infisical is at hand, join with a machine identity and remove it from the project. The guide's console steps were not tried either.
4. An offline expiry: a policy with `offline_hours: 1`, the source stopped, and an hour later "secrets expired offline; reconnect to refresh".
5. If a source sits behind the company's own CA, it connects (the Windows certificate store).
6. Record the outcome in this file.
