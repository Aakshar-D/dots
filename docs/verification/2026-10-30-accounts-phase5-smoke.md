# Accounts phase 5: smoke test

- Date: 2026-10-30
- Build: dev app (`bun run tauri dev`), branch `feat/accounts-api-keys`.
- Data: the same two PCs as phase 4: an Owner/Admin PC "A" and a member PC "B", sharing one dev OpenBao with a test org on a member list. The account has the workspaces Default and Payments (with a few dots in each). B is a Member in Default and a viewer in Payments. PORT below is the port in Settings (curl is run on the PC that made the key).
- Steps: 10, all manual, to be done by a person at the machine. Mark each pass or fail and note anything odd.

## Steps

### 1. Make a key

Expect: on A, open Settings › Accounts, the account's **API keys** tab and choose **Create key**. Name it "smoke", pick Default and choose **Create**. The dialog says "Copy it now. Dots won't show it again." and shows a key starting `dots_sk_` (51 characters), a **Copy** button, "Base URL: http://127.0.0.1:PORT/v1" and a `curl` line. Choose **Done**. The list shows "smoke", the first 12 characters, "Default", and "Last used Never". The full key is not shown anywhere again.

Result: not run.

### 2. List dots

Expect: `curl -H "Authorization: Bearer dots_sk_..." http://127.0.0.1:PORT/v1/dots` answers 200 with only the dots of Default (`id`, `name`, `enabled`, `org_id`, `workspace_id`). None from Payments or Personal. In Dots, "Last used" for the key now shows a time.

Result: not run.

### 3. Trigger a dot

Expect: `curl -X POST -H "Authorization: Bearer dots_sk_..." -d '{"order":"A-17"}' http://127.0.0.1:PORT/v1/dots/DOT_ID/trigger` for an enabled dot of Default answers 202 with `{"run_id":"..."}`. The run appears in Dots marked **API**, with the body as its payload. A disabled dot answers 409 `{"error":"dot is disabled"}`. With Claude usage held high, the run waits like a webhook run.

Result: not run.

### 4. Read the run

Expect: `curl -H "Authorization: Bearer dots_sk_..." http://127.0.0.1:PORT/v1/runs/RUN_ID` answers 200 with `id`, `dot_id`, `status`, `queued_at`, `started_at`, `ended_at`, `summary` and `error`. Poll it until `status` is final.

Result: not run.

### 5. Another workspace is "Not found."

Expect: with the Default key, `GET /v1/runs/<a run id of a Payments dot>` and `POST /v1/dots/<a Payments dot id>/trigger` both answer 404 `{"error":"Not found."}`. So does an id that does not exist. Move a dot from Default to Payments in its editor: the key now gets 404 for it and for its earlier runs.

Result: not run.

### 6. A Viewer cannot trigger

Expect: on B, in the **API keys** tab, **Create key** is off for Payments, with the reason ("... a Viewer in the Payments workspace can't create API keys. Ask an Admin or Owner."). In Default, B can make a key. On A, set B's role in Default to viewer and Refresh on B. B's Default key still answers 200 for `GET /v1/dots`, but `POST .../trigger` answers 403 with the role refusal. Set B back to member and Refresh: the trigger answers 202 again.

Result: not run.

### 7. Revoke

Expect: on A, choose **Revoke** next to "smoke". The confirmation says "Apps using this key stop working at once." After confirming, the key is marked **Revoked** and has no Revoke button. The same `curl` as step 2 now answers 401 `{"error":"That API key isn't valid. Make a new one in Dots."}`. A made-up key `dots_sk_` plus 43 letters, and a request with no key, answer the same 401.

Result: not run.

### 8. Deleting a workspace revokes its keys

Expect: make a key for Payments on A and use it (200). Delete Payments on the **Workspaces** tab, sign, store, and Refresh. The key is marked **Revoked** and the curl answers 401. Leaving an account (**Remove** in Settings › Accounts) removes its keys from the list.

Result: not run.

### 9. The Host check

Expect: `curl -H "Host: evil.example:PORT" http://127.0.0.1:PORT/health` answers 403. `curl -H "Host: evil.example:PORT" -H "Authorization: Bearer dots_sk_..." http://127.0.0.1:PORT/v1/dots` answers 403 `{"error":"This server only answers requests addressed to 127.0.0.1 or localhost."}`. `curl http://localhost:PORT/health` and `curl http://127.0.0.1:PORT/health` answer 200. A `Host` with another port (`127.0.0.1:1`) answers 403.

Result: not run.

### 10. Webhooks and MCP still work

Expect: a dot's webhook (its URL from the dot's editor, with its token) still starts a run (202). A Claude run that uses an MCP server still reaches it. Neither needs an API key.

Result: not run.
