# Env vault plan C smoke verification (the owner's)

These steps are for the owner, with a real build. No agent ran them: the plan's end-to-end test (`crates/dots-core/tests/runtime_audit_test.rs`) covers the same path against a local sink with the fake CLI. Use a fresh `DOTS_DATA_DIR`, never `%LOCALAPPDATA%\dots`. Write no value into this file.

## B1. A local sink

Run any HTTP receiver on this PC that keeps each request's headers and body and answers `200`, for example a small Node or Python script listening on `http://127.0.0.1:8787/dots`. Make it able to answer `503` on demand.

## B2. An account that sends records

1. With `dots-admin`, make a key and a policy whose `audit` is `{"endpoint": "http://127.0.0.1:8787/dots", "token_entry": "DOTS_AUDIT_TOKEN"}`. Sign it.
2. In the source (an OpenBao dev server or a SOPS file), add the entry `DOTS_AUDIT_TOKEN` with a made-up value, and one entry a dot uses.
3. Join the account in Settings › Accounts.

Expect: within about 5 seconds the sink gets one `POST` with `Content-Type: application/x-ndjson`, `Authorization: Bearer <the made-up token>`, and one line: a `fetched` record with `seq` 1 and `prev` `null`.

## B3. A run's records

Grant the entry to a dot of the account (its shell, or an MCP server), and run it.

Expect: the sink gets the `resolved` records with the dot's id and name and the run id. The account's **History** tab lists them as "Used", then "Sent". No line holds a value.

## B4. A refused send

Make the sink answer `503`, run the dot again.

Expect: Overview shows "Audit: <n> records waiting; last send failed <time>: the endpoint answered 503", and History shows "Waiting to send". Make the sink answer `200` again: within about a minute the records arrive and the line goes.

## B5. Verify what arrived

Save the bodies the sink got, one record per line, as `records.ndjson`, and run `dots-admin audit verify records.ndjson`.

Expect: one line for the account and install, then `ok: every chain is whole`. Delete one line from the middle and run it again: it names the missing record and exits 1.

## B6. Without audit

Sign a policy without `audit` and Update the account. Run the dot.

Expect: the sink gets nothing more; History lists the run's records as "Kept on this PC".

## B7. The folded fixes

- In Env, **Add variable**: neither Plain nor Secret is chosen, and **Save** waits for one.
- Stop the source while Dots runs: the status says "offline (…); using the saved copy", and "Saved copy until" gives the local time.
- In the dot editor of an account's dot, ticking a server whose variable only the account has shows no "Not in Personal" line.
