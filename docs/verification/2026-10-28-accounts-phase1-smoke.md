# Accounts phase 1: smoke test

- Date: 2026-10-28
- Build: dev app (`bun run tauri dev`), branch `feat/accounts`.
- Data: a fresh data folder for step 1, then the same folder for the rest. A test organisation to join (see `docs/vault-admin.md`), and at least one dot that belongs to it.
- Steps: 8, all manual, to be done by a person at the machine. Mark each pass or fail and note anything odd.

## Steps

### 1. First start shows a fingerprint

Expect: on a fresh data folder, Settings › Profile shows a fingerprint, and "Copy my Dots ID" copies it.

Result: not run.

### 2. The fingerprint survives a restart

Expect: close and start the app again; the fingerprint is the same as in step 1.

Result: not run.

### 3. Saving a name changes the rail initials

Expect: type a name in Settings › Profile and save; the initials on the switcher at the top of the rail change to match.

Result: not run.

### 4. Joining a test org adds it to the switcher

Expect: Join an account (from the switcher or Settings › Accounts) with the test org's join code; the account appears in the switcher beside Personal.

Result: not run.

### 5. Switching filters Home

Expect: select the test account; Home lists only its dots. Select Personal; Home lists only the dots outside any account. Following a link to a dot of the other account switches the account.

Result: not run.

### 6. A new dot defaults to the selected account

Expect: with the test account selected, create a dot; it belongs to that account and shows on Home. With Personal selected, a new dot is Personal.

Result: not run.

### 7. Leaving the selected account returns to Personal

Expect: with the test account selected, leave it with Remove in Settings › Accounts; the switcher falls back to Personal and Home shows Personal's dots.

Result: not run.

### 8. Approvals outside the selected account still show

Expect: rejoin the test account, make one of its dots ask for approval, then select Personal. The approval still shows in the Inbox and in the Needs you count, and the switcher's menu shows a waiting count beside the test account.

Result: not run.
