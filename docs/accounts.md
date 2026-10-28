# Accounts

An account is a group of people and dots that belong together, such as your team or company.
Your own dots live in **Personal**, which is always there. This guide explains how Dots knows who
you are, how to switch between accounts, and how to join one.

Members, roles, invites, shared workspaces and API keys are **coming next**. This version covers
your identity, the switcher, and joining.

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
  open, Home opens the first dot of the account you picked.
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

## Coming next

- Members of an account and who they are.
- Roles, so an admin can decide who may do what.
- Invites by Dots ID instead of a join code.
- Shared workspaces.
- API keys for an account.
