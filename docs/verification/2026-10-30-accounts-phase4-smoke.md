# Accounts phase 4: smoke test

- Date: 2026-10-30
- Build: dev app (`bun run tauri dev`), branch `feat/accounts-workspaces`.
- Data: two Dots data folders on one PC (an Owner/Admin PC "A" and a member PC "B"), both run from this branch, sharing one dev OpenBao that holds a test org with a v2 policy (`policy new --roster`). The policy lists A's Dots ID in `admins` (`dots-admin policy admins add`). B is on the member list as a Member. A's OpenBao token can write `<mount>/data/<path>/roster`; B's can only read it. A has a few dots in the account.
- Steps: 9, all manual, to be done by a person at the machine. Mark each pass or fail and note anything odd.

## Steps

### 1. Add a workspace

Expect: on A, open Settings › Accounts, the account's **Workspaces** tab, choose **Add workspace**, name it "Payments Team" and add it. The list shows "Payments Team" with the id `payments-team` and "use: no extra rules". The member list number goes up by one. Add a second one called "Payments Team": its id is `payments-team-2`. On B, choose Refresh on the account: both appear on the account's **Workspaces** tab and in the switcher, if B is in them (see step 2).

Result: not run.

### 2. A member without an entry does not see it

Expect: B is a Member with an entry only for Default. B does not see Payments Team in the switcher or on the **Workspaces** tab. On A, open the **Members** tab, choose **Workspaces** next to B, tick Payments Team with the role member, and save. On B, Refresh: Payments Team appears. Put one dot of A's in Payments Team (step 8). Remove B's entry again and Refresh on B: the workspace is hidden, and opening that dot by its link shows the account name, "you aren't in the Payments Team workspace. Ask an Admin or Owner to add you.", and only delete and turn off still work.

Result: not run.

### 3. A workspace viewer cannot run

Expect: on A, set B's role in Payments Team to viewer. Refresh on B. In that workspace, **Save** and the run buttons are off with the reason ("a Viewer in the Payments Team workspace can't ..."). A manual run is refused. A scheduled run waits in the Inbox under **Waiting to run**. In Default, B can still run as a Member. Set B back to member: the held run goes by itself.

Result: not run.

### 4. Narrowing `use` with dots-admin

Expect: with the account's `roster.json`, run `dots-admin roster workspace use roster.json payments-team --allow "PAY_*"`, then `roster sign` and store the list. On B, Refresh. The **Workspaces** tab shows "use: allow PAY_*" for Payments Team, read-only. A dot in that workspace that asks for a name outside `PAY_*` is refused: "the Payments Team workspace doesn't allow <name>". Run the same command with no `--allow` or `--deny` to clear the rules, sign, store and Refresh: "use: no extra rules".

Result: not run.

### 5. Delete moves dots to Default

Expect: on A, delete Payments Team from the **Workspaces** tab. The confirmation says "Its dots on each PC move to Default." After confirming, the dots that were in it are in Default on A. On B, Refresh: the same, and the workspace is gone from every member's entry. Trying to delete Default: there is no delete button for it, and `dots-admin roster workspace remove roster.json default` says "The default workspace can't be deleted."

Result: not run.

### 6. Rename keeps the id

Expect: rename Payments Team-2 to "Payments" on A. The name changes, the id stays `payments-team-2`, and dots stay in it. On B, Refresh: the new name shows.

Result: not run.

### 7. Personal workspaces

Expect: in Settings › Accounts, under Personal, choose **Add workspace** and name it "Side projects". It appears and is listed in the switcher when Personal is selected. Rename it, then delete it: its dots go to Default. Nothing is signed or published, and B does not see it.

Result: not run.

### 8. Move a dot

Expect: open a dot in the account on A. Change **Workspace** in its editor to another workspace. It moves at once. Try a dot that is in a team: "<dot> is in the team <team>. Take it out of the team first." Try with a PC that is a workspace viewer in the target workspace: the move is refused with the role and the workspace named. Put a dot of another workspace in a team: the team's dot shows under **Needs attention** in the Inbox, "<dot> is in another workspace than the <team> team."

Result: not run.

### 9. The switcher filters Home

Expect: on A, with a workspace picked in the switcher, **Home** lists only that workspace's dots, and a new dot starts in that workspace (its **Workspace** field shows it). Your pick survives a restart. **Inbox**, **Needs you**, runs and Usage still show everything. An account with only Default shows no workspace list in the switcher.

Result: not run.

## Not an app step

An account with no member list has only Default. Its **Workspaces** tab is read-only and says "This account has no member list, so its only workspace is Default."
