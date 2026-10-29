# Accounts phase 3: smoke test

- Date: 2026-10-29
- Build: dev app (`bun run tauri dev`), branch `feat/accounts-invites`.
- Data: two Dots data folders on one PC (an Owner/Admin PC "A" and a joining PC "B"), both run from this branch, sharing one dev OpenBao that holds a test org with a v2 policy (`policy new --roster`). The policy lists A's Dots ID in `admins` (`dots-admin policy admins add`). Both profiles have a name and an email. A's OpenBao token can write `<mount>/data/<path>/roster`; B's can only read it.
- Steps: 12, all manual, to be done by a person at the machine. Mark each pass or fail and note anything odd.

## Steps

### 1. Invite

Expect: on A, open Settings › Accounts, the account's **Members** tab, choose **Invite**, give an email and a role, and choose **Create invite**. Dots shows an invite code. **Copy** puts it on the clipboard. The invite is listed under **Open invites** with its expiry 7 days away.

Result: not run.

### 2. Save as .dotsorg downloads a file

Expect: in the invite dialog, choose **Save as .dotsorg**. A file named `<org id>.dotsorg` is saved to the Downloads folder (or the Save dialog appears) in the running Tauri app, and it opens in B's Join dialog. This link is a `data:` URL in the webview and has not been checked in the app yet. If nothing downloads, note it: the Copy button is the fallback.

Result: not run.

### 3. Join with the invite

Expect: on B, choose **Join an account** and paste the invite code (or open the saved file). The preview says "Invited as ..." with the role and the expiry. Sign in to the source. Dots shows "You joined ..., but your Dots ID is not on its member list yet." and a request code with **Copy request code**.

Result: not run.

### 4. Request access (no invite)

Expect: with a join code of the same org (no invite) on a third data folder or after removing the account on B, join again. The account shows Waiting and a **Request access** button. Choose it. A request code appears with "Send this to an Admin of ...". Keep this code for step 6.

Result: not run.

### 5. Approve with an invite, then Active

Expect: on A, choose **Approve request**, paste the code from step 3, choose **Check** (name, email and fingerprint show, no problem), then **Approve**. The new list is published (member list number goes up by one). The invite leaves **Open invites**. On B, Refresh: the account is Active with the invited role.

Result: not run.

### 6. Approve with no invite

Expect: on A, approve the code from step 4. Dots asks for a tick on "This person has no invite from this PC" and a role. **Approve** stays off until it is ticked. After approving, the person is on the list with that role. A request with an email that is already on the list joins that member and keeps their role.

Result: not run.

### 7. Change a role

Expect: on A, change a Member to Viewer with the role menu. The list is signed and published. On B, Refresh: New dot and Save are off with the reason. Change back to Member to restore. Trying to change an Owner gives "Owners can only be changed with the account owner's key, using dots-admin."

Result: not run.

### 8. Remove a person

Expect: on A, choose **Remove** for a member. The confirmation mentions revoking their access in the source. Confirm. On their PC, Refresh: the account shows Removed and its secrets are gone from that PC.

Result: not run.

### 9. Remove this PC's own key

Expect: on A, with another Admin or Owner on the list, use **Remove this PC** next to the key marked "This PC" on A's own entry. The list is signed and published. After Refresh on A, A is no longer on the list (Removed or Waiting) and the Members tab buttons are off. Note exactly what A shows. Put A back with `dots-admin roster add` and sign, or with approval from the other Admin.

Result: not run.

### 10. Publish denied, draft, retry

Expect: on B (or any PC whose token cannot write `roster`), as an Admin whose Dots ID is in `admins`, make a change. The Members tab shows "Not published yet:" with what it changes, the source's error, **Retry** and **Discard**. Nothing is live for others. Add `create` and `update` on `<mount>/data/<path>/roster` to the token's policy, then choose **Retry**. The draft goes away and the list number goes up. Also check **Discard** on a second draft.

Result: not run.

### 11. Another admin publishes first

Expect: on A and on a second Admin PC, open the same account. Publish a change on one. On the other, make a change without refreshing. Dots refuses with "the member list changed (now #N) while you were editing. Check it and make the change again." Redo it and it works.

Result: not run.

### 12. SOPS copy flow

Expect: with a SOPS org (age key), make a change on the Members tab. A draft shows "Copy the signed list into the SOPS file (see the admin guide)." and **Copy signed list**. Choose it, put the text in the file's `roster` field with `sops set`, store the file, and Refresh. The draft disappears and the new list shows.

Result: not run.

## Not an app step

An Admin whose Dots ID is not in `admins` sees the buttons off, with "Can sign the list once the owner adds their Dots ID to the policy's admins." Check this once with a third key.
