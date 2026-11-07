# Dots audit records: format and endpoint

This is what a Dots install sends to an account's audit endpoint, and what a receiver (project 2's admin server, or any HTTP sink) must accept. It is version 1. The env vault spec (`docs/superpowers/specs/2026-10-21-env-vault-design.md`, §10) says why each part exists.

A record never holds a value. It says which variable was used, by which dot and for which consumer, or what a fetch of the account's source came to.

## Turning it on

The account's signed policy names the endpoint and the token:

```json
"audit": { "endpoint": "https://audit.acme.internal/dots", "token_entry": "DOTS_AUDIT_TOKEN" }
```

- `endpoint` is an `https://` URL. `http://` is accepted only for `localhost` and `127.0.0.1`, for a test sink on the same PC. The URL holds no user name, password, query or fragment; the token is the vault entry `token_entry` names.
- `token_entry` names an ordinary entry of the account. Dots sends its value as the bearer token. Grant nothing for it: Dots reads it itself.

Without `audit`, Dots sends nothing. It still lists what happened in the account's **History** on that PC, but those records get no `seq` and are never sent. Records are queued from the first fetch that accepts a policy with `audit`.

Personal dots never write a record.

## The request

```http
POST <endpoint>
Content-Type: application/x-ndjson
Authorization: Bearer <value of token_entry>

<record>\n<record>\n…
```

- One request holds the records of one account, at most 500, in `seq` order. Each record is one line, ended by `\n`.
- Any `2xx` answer means the receiver has stored the batch. Dots deletes it from its queue. The body of the answer is not read.
- Any other answer, a timeout (20 seconds), or no connection: Dots keeps the batch and tries again later. Redirects are not followed, since a redirect would take the token to another host.
- A batch may arrive twice: when the receiver stored it but the `2xx` never reached Dots. Store records by (`org_id`, `install`, `seq`) and ignore a line you already hold.

### When Dots sends

- A new record is sent about 5 seconds after it is written, together with any others waiting.
- After a failure, the next try for that account waits 30 seconds, then 1, 2, 4, 8 minutes and so on, up to 1 hour between tries, each wait varied by up to 20% either way. A `2xx` sets the wait back to 30 seconds.
- While the account gives no values on that PC (access revoked, offline with no saved copy, not on the member list), there is no token, and the records wait.

### The queue

Each install queues at most 100,000 records per account. When that is reached, the oldest 1,000 are deleted unsent, and an `expired` record with reason `queue_full` says which (`dropped`). A receiver then sees a gap those records name: one `expired` record per drop, and when the queue stays full, drops in a row whose ranges meet end to end (1 to 1,000, then 1,001 to 2,000, and so on). After more than 100 drops in a row, the first drop's own `expired` record is itself dropped, so no record the receiver holds names that range, and `dots-admin audit verify` reports that part as a gap (a false failure, not a lost record it could have caught).

Leaving an account on a PC keeps that account's queue and its chain. Joining it again sends what was queued and carries on from the next `seq`.

## The record

One JSON object per line, in canonical form: the keys of every object sorted by their bytes, no white space, and every key present (`null` when it does not apply).

```json
{"at":"2026-10-21T09:12:03Z","consumer":"mcp:github","dot":{"id":"01J…","name":"Triage"},"dropped":null,"event":"resolved","id":"01J…","install":"01J…","name":"GITHUB_TOKEN","org_id":"acme-eng","prev":"sha256:…","reason":null,"run_id":"01J…","seq":412,"source":"openbao","user":{"dots_id":"B255 6546 …","machine":"LT-0423","source_identity":"jdoe","windows":"ACME\\jdoe"},"v":1}
```

| Key | Type | What it is |
|---|---|---|
| `v` | number | `1`. A receiver should refuse another version it does not know. |
| `id` | string | A ULID, unique for the record. |
| `seq` | number | `1` for the account's first record on this install, then one more for each record. Never reused. |
| `prev` | string or null | `sha256:` and the lower-case hex SHA-256 of the bytes of the previous record's line (no line end). `null` only for `seq` 1. |
| `at` | string | When it happened, RFC 3339 in UTC, to the second. |
| `org_id` | string | The account's id. |
| `install` | string | The install's id (a ULID). It is fixed for a data folder. |
| `user.windows` | string or null | The Windows account, `DOMAIN\user`. |
| `user.machine` | string or null | The PC's name. |
| `user.source_identity` | string or null | Who the source knows this PC as: the user name of an OpenBao `userpass` or `ldap` login, the Infisical client id, or this PC's SOPS age public key. `null` for an OpenBao token. |
| `user.dots_id` | string or null | The install's Dots ID as its fingerprint: 32 upper-case hex digits in groups of 4. |
| `dot` | object or null | `{ "id", "name" }` of the dot that resolved. `null` for a fetch and for a Library check. |
| `run_id` | string or null | The run that resolved. `null` for a fetch and for a Library check. |
| `event` | string | See below. |
| `name` | string or null | The variable. `null` for a fetch outcome. |
| `consumer` | string or null | `mcp:<server>` or `shell`. `null` for a fetch outcome. |
| `source` | string or null | For `resolved`, where the value came from: `openbao`, `infisical`, `sops`, or `personal` (the person's own entry, where the policy's `personal` mode allows it). For every other event, the account's source kind; `null` when a damaged account's stored settings do not give even that. |
| `reason` | string or null | A code, below. |
| `dropped` | object or null | `{ "from", "to" }`, both included, on an `expired` record with reason `queue_full`. |

A receiver must not depend on key order; Dots writes the canonical form so that the hash is reproducible from the line as received.

### Events

| `event` | When | `reason` |
|---|---|---|
| `resolved` | A variable was given to a consumer: an MCP server or the shell of a run, or a Library check in the account. | `null` |
| `denied` | A variable was asked for and not given. The run went on without that consumer (an MCP server) or without that variable (the shell). | Why, below. |
| `fetched` | A fetch of the source worked and its policy was accepted. | `null` |
| `offline` | A fetch could not reach the source. Dots uses its saved copy while that holds. | `null` |
| `revoked` | The source refused this PC's sign-in details, or this PC was taken off the member list. Dots wiped its copy. | `denied` or `removed` |
| `expired` | The saved copy ran out (`offline_hours` passed since the last good fetch), or the queue dropped records. | `offline_copy` or `queue_full` |
| `policy_invalid` | The source answered something Dots cannot use, or the policy did not verify. | `null` |

One record is written per variable asked for. A run start that resolves three variables for an MCP server writes three records. Names after the first that does not resolve for that consumer are not asked for, so they get none.

Fetches happen when Dots starts, every `refresh_minutes`, on **Refresh**, on **Update**, and before a run when the copy is older than `refresh_minutes`.

### Reasons of `denied`

| `reason` | Meaning |
|---|---|
| `not_granted` | The dot does not grant the variable to that consumer. |
| `not_defined` | Neither the account (nor Personal, where the policy reaches it) has the variable. |
| `denied_by_policy` | The account's `use` rules do not allow the name. |
| `denied_by_workspace` | The dot's workspace's `use` rules do not allow the name. |
| `shell_secrets_forbidden` | The value is a secret, the consumer is the shell, and the policy's `shell_secrets` is `false`. |
| `org_offline` | The source can't be reached and there is no saved copy. |
| `org_expired` | The saved copy ran out. |
| `org_revoked` | The source refused this PC's sign-in details. |
| `policy_invalid` | The account has no valid policy and no good copy. |
| `org_removed` | The account is not joined on this PC. |
| `not_member` | This PC is not on the account's member list. |

## Checking a file of records

`dots-admin audit verify <records.ndjson>` reads records as a receiver stores them, one per line, and checks each account and install on its own:

- `seq` goes up by one from one record to the next;
- each `prev` is the hash of the line before;
- `seq` 1 has no `prev`, and a later record has one.

It reports each gap, a changed record (the next record's `prev` no longer matches), a `seq` lower than the one before, and another record with a `seq` already read. A file may start part way through a chain, and a line sent twice is counted, not reported. A gap that the install's own `queue_full` records name, alone or together, is reported as dropped, and does not fail the check. It exits 0 when every chain is whole, 1 otherwise.

Records are not signed: a per-device key needs device registration, which is project 2's. The chain shows a gap or an edit to records already received; it does not prove which PC wrote them.
