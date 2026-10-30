# Accounts phase 2: smoke test

- Date: 2026-10-29
- Build: dev app (`bun run tauri dev`), branch `feat/accounts-roster`.
- Data: a fake or real OpenBao holding a test org, the org's policy key, and `dots-admin` built from this branch. This PC joined to the org with a v1 policy before step 1. At least one dot belongs to the org, with a schedule.
- Steps: 7, all manual, to be done by a person at the machine. Mark each pass or fail and note anything odd.

## Steps

### 1. A v1 org is unchanged (legacy)

Expect: with the v1 policy, the account shows as usual, with no member-list status. New dot, Save and runs all work, as for a Member.

Result: not run.

### 2. A required roster without this PC shows Waiting

Expect: make a v2 policy (`dots-admin policy new --org-id <id> --roster`), sign and store it as `DOTS_POLICY`. Make a roster that does not have this PC (`roster new` with another owner key), sign it, and store it as `DOTS_ROSTER` at `<mount>/data/<path>/roster`. Refresh the account. It shows Waiting, with the text "you aren't on its member list yet. Send your Dots ID to an Admin." beside the **Copy my Dots ID** button. Nothing is wiped. Keep this signed text (`roster.signed.json`): step 7 publishes it again as the older roster.

Result: not run.

### 3. Adding the key makes the account Active

Expect: copy this PC's Dots ID (Settings › Profile). Run `roster add --name ... --email ... --key <id> --role member` and `roster sign`, store the new text as `DOTS_ROSTER`, and Refresh. The account shows Active with the role Member.

Result: not run.

### 4. Demote to Viewer

Expect: `roster set-role <file> <email> viewer`, sign, store, Refresh. New dot and Save are disabled with the reason, and so are the approval buttons. A manual run is refused with "a Viewer can't ... Ask an Admin or Owner." A scheduled run is held in the Inbox under "Waiting to run" with the reason, and does not fail.

Result: not run.

### 5. Promote back and the held run runs

Expect: `roster set-role <file> <email> member`, sign, store, Refresh. The controls are enabled again and the held run runs without being touched.

Result: not run.

### 6. Remove from the roster

Expect: `roster remove <file> <email>`, sign, store, Refresh. The account shows Removed and its secrets are gone from this PC. Rejoining and being added again brings it back to Active.

Result: not run.

### 7. A lower seq is refused

Expect: store the signed text kept from an earlier step (step 2's, or any `roster.signed.json` with a lower `seq` than the one this PC accepted) as `DOTS_ROSTER` again, and Refresh. The status says the member list is older than the one already accepted, and the previous roster, membership and role are kept.

Result: not run.
