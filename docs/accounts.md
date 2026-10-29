# Accounts

An account is a group of people and dots that belong together, such as your team or company.
Your own dots live in **Personal**, which is always there. This guide explains how Dots knows who
you are, how to switch between accounts, and how to join one.

Shared workspaces and API keys are **coming next**. This version covers
your identity, the switcher, joining (with an invite or by asking), and the member list with roles.

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

Switching only narrows **Home**. **Inbox**, **Needs you** and notifications always show everything,
from every account, so an approval is never hidden because you are looking at another account.

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

## Coming next

- Shared workspaces.
- API keys for an account.
