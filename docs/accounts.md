# Accounts

An account is a group of people and dots that belong together, such as your team or company.
Your own dots live in **Personal**, which is always there. This guide explains how Dots knows who
you are, how to switch between accounts, and how to join one.

This guide covers your identity, the switcher, joining (with an invite or by asking), the member
list with roles, workspaces, and API keys.

## Your Dots ID

The first time Dots starts, it makes a key pair for you on this PC. The private half never leaves
the PC. The public half is your **Dots ID**, shown as a short **fingerprint** so it is easy to read
out and compare.

- Open **Settings › Profile** to see your fingerprint.
- Choose **Copy my Dots ID** to copy it. Send it to an account's admin when they ask who you are.
- Your fingerprint stays the same across restarts and updates. It changes if you delete the Dots
  data folder, or if this PC's saved key is lost from Windows Credential Manager (for example after
  a reset of your Windows profile). Dots then makes a new key, and your Dots ID is a new one: send
  it to your admins again.

## Your profile

In **Settings › Profile** you can set the name other people see. The initials on the account
switcher in the rail come from that name. Save the name and the initials change right away.

## Switching accounts

At the top of the rail is the account switcher. Click it to see **Personal** and every account you
have joined.

- Pick one and **Home** shows only that account's dots. Your pick is remembered. If a dot was
  open, Home opens the dot you last opened, if it is in the account you picked, or else that
  account's first dot.
- A new dot you create starts in the selected account.
- If you follow a link to a dot in another account, Dots switches to that account for you.
- Next to each account the menu shows how many things are waiting for you there.

If the account has more than one workspace, they are listed under it in the switcher. Pick one and
**Home** shows only that workspace's dots, and a new dot starts in it. Your pick is remembered.

Switching only narrows **Home**'s dot list and where new dots start.
**Inbox**, **Needs you**, notifications, runs and Usage always show everything,
from every account and workspace, so an approval is never hidden because you are looking at another
one.

## Joining an account

Your admin gives you a join code. See [Setting up Dots for an organisation](vault-admin.md) for
what the admin does.

1. Go to **Settings › Accounts** and choose **Join an account**. (**Join an account** in the
   switcher takes you to **Settings › Accounts**; choose **Join an account** there.)
2. Paste the join code, or open the `.dotsorg` file your admin sent. Check what it joins.
3. Most accounts then ask you to sign in to the source with your own credential.
   Some accounts need no sign-in. Instead the dialog shows a key for this PC. Send that key to your
   admin, who adds it to the account.
4. The account now appears in the switcher.

### Joining with an invite

An admin can invite you by email. They send you an invite code or a `.dotsorg` file. Use it in
**Join an account** like a join code. The preview says "Invited as ..." with the role and when the
invite expires. An invite is good for 7 days. If it has expired, ask for a new one.

Fill in your name and email in **Settings › Profile** first, because your request carries them.
After you join, Dots shows a **request code**. Choose **Copy request code** and send it to the
admin. They approve you in Dots, and you become Active at the next refresh.

If the Join dialog shows a key for this PC instead of a sign-in, send that key to the admin first.
The join waits until they have added it; then choose **Try again**, the join completes, and the
request code appears.

### Asking for access

If your Dots ID is not on the member list, the account shows Waiting. Choose **Request access** to
get a request code, copy it, and send it to an Admin. Dots checks again on its normal refresh, or
you can choose Refresh. Nothing is lost while you wait.

If you see **waiting for the admin to add this PC's key**, the join worked but the admin has not
added your key yet. Nothing is wrong on your side. Once they have, open **Settings › Accounts**
and choose Refresh on the account.

**Manage accounts** in the switcher opens **Settings › Accounts**, where you can update an
account's sign-in or remove it (leave it). If you remove the account that is selected, Dots goes back to
**Personal**.

The dots of an account you removed are not deleted. They appear under **Personal**, marked
**account removed**, so you can still open them, turn them off or delete them. They can't run until
you join the account again, and then they move back to it.

Settings used to call these "Organisations". It is the same thing under a new name.

## Member lists and roles

Some accounts keep a **member list**, signed by the account's Owner or an Admin. On those accounts
you have a role:

- **Owner** and **Admin** can do everything a Member can, and change the member list.
- **Member** can make and edit the account's dots, run them, answer approvals and grant the
  account's values.
- **Viewer** can look at the account's dots and runs, but not change or run them. **New dot**,
  **Save** and the approval buttons are greyed out with the reason. A manual run, an app's start
  of a dot and a webhook call are refused (the webhook answers HTTP 403 with the reason). A
  scheduled run, watcher or follow-up waits in the Inbox under **Waiting to run**, with the
  reason, and runs by itself when your role is back to Member or above.

**Personal** is always yours as Owner. An account without a member list treats everyone as a Member.

Next to each account you see one of these states:

- **Active**: you are on the list, with your role.
- **Waiting**: the account has a member list and you are not on it yet. Choose **Copy my Dots ID**
  (in **Settings › Profile**) and send it to your admin. Nothing is lost while you wait. When they
  have added you, choose Refresh on the account.
- **Waiting, "this PC's Dots ID changed"**: the saved key on this PC was lost, so your Dots ID is a
  new one. Send the new ID to an Admin. Nothing is wiped meanwhile.
- **Removed**: an admin took you off the list. Dots deletes that account's secrets from this PC.
  Your dots stay. Join the account again to come back.

While you are waiting or removed you can still clean up: switch the account's dots off or delete
them. Turning them on, editing them and changing their watchers wait until you are on the list
again. To drop the account altogether, leave it: **Remove** it in **Settings › Accounts**.

Owners and Admins also see a **Members** tab on each account. It lists the members and lets an
Admin invite people, approve requests, change roles and remove people. See
[Setting up Dots for an organisation](vault-admin.md), section 10. Owners can only be changed with
`dots-admin`.

## Workspaces

A workspace is a part of an account, such as a team or a project. Every account has **Default**.
Your dots are listed in one workspace. Dots you made before workspaces are in Default.

- On an account with a member list, an Owner or Admin sees every workspace. A Member or Viewer
  sees only the workspaces the list puts them in. Their role there can narrow what they can do:
  in a workspace where you are a **viewer** you can look but not change or run, even if you are a
  Member of the account. A workspace you are not in is hidden. If you open one of its dots by a
  link, it refuses everything except delete and turning it off: "... you aren't in the ...
  workspace. Ask an Admin or Owner to add you."
- An Owner or Admin adds, renames and deletes workspaces and sets who is in each, on the account's
  **Workspaces** tab and **Members** tab. Deleting a workspace moves its dots and teams to Default.
  The **Workspaces** tab also shows each workspace's `use` rules (which variable names its dots may
  use). Only `dots-admin` changes those. See [Setting up Dots for an
  organisation](vault-admin.md), section 10.
- **Personal** has workspaces too. They are only on this PC. Add, rename and delete them in
  **Settings › Accounts**, under Personal.
- To move a dot, change its **Workspace** in the dot's editor. It moves at once. You need Member
  rights in both workspaces. A dot can move within its account only, and not while it is in a team.
  A team's dots must be in the team's workspace.

An account without a member list has only Default, and Personal is not narrowed.

## API keys

An API key lets an app or a script on this PC list a workspace's dots, run one, and read how a run
went. Keys are made and used in Dots on this PC. They never leave it.

### Making a key

1. Open **Settings › Accounts**. On an account, open its **API keys** tab. For Personal, the
   **API keys** list is under **Personal**.
2. Choose **Create key**, give it a name and pick the workspace.
3. The key appears once, with a **Copy** button and a ready `curl` line: "Copy it now. Dots won't
   show it again." Dots keeps only the first 12 characters, to recognise it by, and a hash. Close
   the dialog and the key is gone. If you lose it, revoke it and make another.

A key looks like `dots_sk_` followed by 43 letters and digits. Treat it like a password.

- A key belongs to **one workspace** on **this PC**. It is not shared through the member list. Your
  teammates make their own keys on their own PCs.
- You need **Member** rights or above in that workspace to make or revoke a key. A Viewer sees the
  list, with **Create** and **Revoke** off and the reason, such as "Acme: a Viewer in the Payments
  workspace can't create API keys. Ask an Admin or Owner."
- The list shows each key's name, its first 12 characters, the workspace, when it was made and
  when it was last used (**Never** until it is used).

### Using a key

The local server answers on `http://127.0.0.1:PORT`, where PORT is the port in **Settings**. The
create dialog shows the exact base URL (`http://127.0.0.1:PORT/v1`). If the local server is not
running, it says so instead. Send the key as a Bearer token:

On Windows, use `curl.exe` (in PowerShell, plain `curl` is a different command). This works in cmd and PowerShell:

```
curl.exe -H "Authorization: Bearer dots_sk_..." http://127.0.0.1:PORT/v1/dots
curl.exe -X POST -H "Authorization: Bearer dots_sk_..." -d "{\"order\":\"A-17\"}" http://127.0.0.1:PORT/v1/dots/DOT_ID/trigger
curl.exe -H "Authorization: Bearer dots_sk_..." http://127.0.0.1:PORT/v1/runs/RUN_ID
```

Or put the body in a file and send it with `-d @payload.json`. In bash (macOS, Linux, Git Bash):

```bash
curl -H "Authorization: Bearer dots_sk_..." http://127.0.0.1:PORT/v1/dots
curl -X POST -H "Authorization: Bearer dots_sk_..." -d '{"order":"A-17"}' \
  http://127.0.0.1:PORT/v1/dots/DOT_ID/trigger
curl -H "Authorization: Bearer dots_sk_..." http://127.0.0.1:PORT/v1/runs/RUN_ID
```

- `GET /v1/dots` lists the dots of the key's workspace: `id`, `name`, `enabled`, `org_id` and
  `workspace_id`.
- `POST /v1/dots/{id}/trigger` starts a run. The body is the run's payload, the same as for the
  dot's webhook. It answers 202 with `{"run_id": "..."}`. A disabled dot answers 409 "dot is
  disabled".
- `GET /v1/runs/{id}` gives the run's status: `id`, `dot_id`, `status`, `queued_at`, `started_at`,
  `ended_at`, `summary` and `error`. Only status, no events.

What a key can reach:

- Only the dots and runs of **its own workspace**. A dot or run anywhere else, or one that does not
  exist, answers 404 "Not found." so a caller can't tell them apart.
- Every request checks your **current role** in that workspace. A Viewer key can list and read
  runs, but a trigger answers 403 with the reason. If your role changes, the key follows at once.
- A key that is wrong, revoked, or whose workspace was deleted gets the same 401: "That API key
  isn't valid. Make a new one in Dots."
- Runs started this way show as **API** in Dots. They are background runs: like webhooks, they wait
  while Claude usage is high.

### Revoking a key

Choose **Revoke** next to the key and confirm: "Apps using this key stop working at once." It stays
in the list, marked **Revoked**.

- Deleting a workspace revokes its keys.
- Leaving an account (**Remove** in **Settings › Accounts**) deletes its keys from this PC.

### The Host check

Every route of the local server (webhooks, MCP and `/v1`) only answers requests addressed to
`127.0.0.1:PORT` or `localhost:PORT`. Any other `Host` gets 403. This stops a web page that
re-points its own name at your PC (DNS rebinding) from reaching Dots. Normal use of webhooks and
MCP is unchanged.
