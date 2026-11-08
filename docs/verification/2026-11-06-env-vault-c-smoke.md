# Env vault plan C smoke verification (the owner's)

These steps are for the owner, with a real build. No agent ran them: the plan's end-to-end test (`crates/dots-core/tests/runtime_audit_test.rs`) covers the same path against a local sink with the fake CLI. Use a fresh `DOTS_DATA_DIR`, never `%LOCALAPPDATA%\dots`. Write no value into this file.

## B1. A local sink

Run any HTTP receiver on this PC that keeps each request's headers and body and answers `200`, for example a small Node or Python script listening on `http://127.0.0.1:8787/dots`. Make it able to answer `503` on demand.

## B2. The fake CLI, then an account that sends records

This smoke never runs the real `claude` CLI. Before any run, set it up as [`2026-10-24-env-vault-b-smoke.md`](2026-10-24-env-vault-b-smoke.md) does (its "How the agent steps were run" and A4):

1. Build the stand-in: `cargo build -p dots-core --bin fake-claude`, and copy `fake-claude.exe` from the target folder into the smoke folder.
2. Start Dots with `DOTS_DATA_DIR` set to `data\` in a fresh folder (from `mktemp -d`), `WEBVIEW2_USER_DATA_FOLDER` set to `webview\` in the same folder, and a `PATH` that leaves out the folder holding the real `claude` (`which -a claude` finds it), so nothing can reach it. Every start in this smoke uses the same three, as the b-smoke's "How the agent steps were run" does.
3. In Settings, save the copy's path as "Program path", then "Restart Dots". Check that `status` names the copy.
4. In Env, **Add variable** `FAKE_CLAUDE_SCRIPT` (a script file in the smoke folder, as in that file's A4): add it as Plain (Kind: **Plain**), then grant it to the dot's shell. The dot in B3 needs it.

Then the account:

1. With `dots-admin`, make the key and the policy: `dots-admin keygen --out keys`, then `dots-admin policy new --org-id acme-eng --out policy.json`, and in `policy.json` set `audit` to `{"endpoint": "http://127.0.0.1:8787/dots", "token_entry": "DOTS_AUDIT_TOKEN"}` and `personal` to `"fallback"`. The B3 dot is the account's, and it reads the Personal `FAKE_CLAUDE_SCRIPT` only with `personal: "fallback"`. Run `policy check`, then `policy sign` (admin guide, section 3, "Making, checking and signing").
2. Store the signed text as `DOTS_POLICY` in the source (an OpenBao dev server or a SOPS file, admin guide section 4) with the entry `DOTS_AUDIT_TOKEN` (a made-up value) and one entry a dot uses. For SOPS, add this PC's key as recipient after the first Join, as the b-smoke's A2 does.
3. Make the join code: `dots-admin join-code --org-id acme-eng --name "Acme Engineering" --source sops --location <path to the .enc.json> --pub keys/policy.pub --file acme-eng.dotsorg` (admin guide, section 5, has the OpenBao and Infisical forms).
4. In Settings › Accounts, join with the file.

Expect: within about 5 seconds the sink gets one `POST` with `Content-Type: application/x-ndjson`, `Authorization: Bearer <the made-up token>`, and one line: a `fetched` record with `seq` 1 and `prev` `null`.

## B3. A run's records

Make a dot in the account and grant the entry to it (its shell, or an MCP server). Run it with **Run now**; it runs the fake CLI from B2.

Expect: the sink gets the `resolved` records with the dot's id and name and the run id. The account's **History** tab lists them as "Used", then "Sent". No line holds a value.

## B4. A refused send

Make the sink answer `503`, run the dot again (still the fake CLI).

Expect: Overview shows "Audit: <n> records waiting; last send failed <time>: the endpoint answered 503", and History shows "Waiting to send". Make the sink answer `200` again. The next try comes after the wait: 30 seconds after the first failure, doubling after each further one up to an hour, each wait plus or minus 20%. Wait for that try; when it goes through, the records arrive and the line goes.

## B5. Verify what arrived

Save the bodies the sink got, one record per line, as `records.ndjson`, and run `dots-admin audit verify records.ndjson`.

Expect: one line for the account and install, then `ok: every chain is whole`. Delete one line from the middle and run it again: it names the missing record and exits 1.

## B6. Without audit

Sign a policy without `audit` and Update the account. Run the dot (the fake CLI).

Expect: the sink gets nothing more; History lists the run's records as "Kept on this PC".

## B7. The folded fixes

- In Env, **Add variable**: neither Plain nor Secret is chosen, and **Save** waits for one.
- Stop the source while Dots runs: the status says "offline (…); using the saved copy", and "Saved copy until" gives the local time.
- In the dot editor of an account's dot, ticking a server whose variable only the account has shows no "Not in Personal" line.
