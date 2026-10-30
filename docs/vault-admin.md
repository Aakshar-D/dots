# Setting up Dots for an organisation

This guide is for an organisation's admin who hands Dots to their people. It covers the org's
policy key, the policy, each kind of source, join codes, and the day-to-day work of rotating
values and cutting people off.

The examples use `acme-eng` as the org id, "Acme Engineering" as its name, and hosts under
`example.invalid`. Use your own. No value in this guide is a real secret.

Shell commands are written for Git Bash or another POSIX shell. In Git Bash, a value that starts
with `/` (such as `--secret-path /dots`) is rewritten into a Windows path unless you set
`MSYS_NO_PATHCONV=1` first, and a UNC path (`\\server\share\...`) loses a backslash: give that one
from PowerShell. In PowerShell, `dots-admin` works the same, but quoting differs for the `sops`
and `bao` examples.

## 1. What an org is

An org is a set of variables that you manage in one place, the **source**, plus a **policy** that
you sign. Dots reads both and enforces the policy on each person's PC.

- A source is one of: OpenBao or HashiCorp Vault, Infisical, or a SOPS file encrypted with age.
- Each person joins with a **join code** that you give them. It says where the source is and which
  key signs the policy. It holds no secret, with one exception: a SOPS `location` URL whose
  query carries a token (a signed URL) does. Send such a code like a secret.
- Each person signs in to the source with their own credential: a token, a username and password,
  a machine identity, or their PC's age key. Dots never shows it again after Join.

What the person sees:

- **Settings → Organisations.** The list of joined orgs, with each one's status, source, key
  fingerprint, last fetch, how long the saved copy lasts, and the policy in plain words. The
  buttons are **Join an organisation**, **Refresh**, **Update** and **Remove**.
- **Library → Env.** Their Personal variables first, then one section per org with its status
  line. Org rows are read-only. They say "Managed by Acme Engineering", show the value only for
  plain entries, and carry a "Locked" badge when a Personal variable of the same name can't take
  its place.
- **The dot editor.** When a person makes a new dot, an **Org** picker offers Personal and each
  joined org. The picker shows only when an org is joined. Once the dot is made, its org is
  fixed and shows as text.

Every dot belongs to exactly one org, or to Personal. An org's values never reach a dot of
another org, even when the names are the same. A person can join several orgs at once, and they
never mix.

## 2. The policy key

The policy key is a minisign key pair. Your secret key signs the policy; every Dots checks the
signature with the public key from the join code. A policy that someone edits on their PC, or
that does not match your key, is refused.

Make the key once per org:

```bash
dots-admin keygen --out keys
```

It asks for a password twice and writes `keys/policy.key` (secret) and `keys/policy.pub`
(public). It never overwrites an existing key file. Its output ends with:

```text
public key: RW...
key fingerprint: 3F9A 1C2B 77D2 0E4C 8C41 E56D 0F51 C2E0
Keep policy.key offline. Join codes carry policy.pub; Dots shows the fingerprint above, not the key id in policy.pub's comment.
```

- **Keep `policy.key` offline**, for example in your password manager or on an encrypted drive.
  On Windows a key file gets its folder's permissions and nothing more, so keep it in a folder
  only you can read.
- **The password.** `dots-admin` reads it from the `DOTS_ADMIN_PASSWORD` environment variable
  when that is set, and from the terminal otherwise. Use the variable only in scripts you trust.
- `--no-password` writes an unprotected key, with a warning. Don't use it outside tests.
- **The fingerprint** is what a person sees when they join. Tell them to expect it. It is a
  digest of the whole public key: 32 hex digits in groups of 4. Read it from what `dots-admin`
  prints (`keygen`, `join-code`, `policy sign`), never from the comment line of `policy.pub`:
  that line shows minisign's key id, which whoever makes a key can set to anything, so it proves
  nothing.

When the key changes, a person who uses **Update** with the new join code sees the line "the
policy key changes from ... to ..." highlighted in the list of changes. Dots then asks in a
window of its own, "Accept the new policy key?", before it updates. See
[A leaked key](#a-leaked-key) for how to change keys without a gap.

## 3. The policy

### The fields

A policy is a JSON object. Unknown fields, a field given twice, or a value out of range make the
whole policy invalid: Dots fails closed.

```json
{
  "v": 1,
  "org_id": "acme-eng",
  "issued_at": "2026-10-21T09:00:00Z",
  "plain": ["LOG_LEVEL", "API_BASE_URL"],
  "use": { "allow": ["*"], "deny": ["PROD_*"] },
  "personal": "fallback",
  "shell_secrets": false,
  "refresh_minutes": 15,
  "offline_hours": 24,
  "audit": null
}
```

| Field | Default | Meaning |
|---|---|---|
| `v` | required | The policy format. It must be `1`. |
| `org_id` | required | The org this policy is for. It must match the join code's org id. |
| `issued_at` | required | An RFC 3339 time. `dots-admin policy sign` sets it to now. |
| `plain` | `[]` | Entries shown with their values. Every other entry is a secret. |
| `use` | `{"allow": ["*"], "deny": []}` | Which names dots may be granted. |
| `personal` | `"deny"` | Whether a person's Personal variables mix in. |
| `shell_secrets` | `false` | Whether a secret may be granted to a dot's shell. |
| `refresh_minutes` | `15` | How often Dots fetches the org. At least 1. |
| `offline_hours` | `24` | How long the saved copy lasts after the last good fetch. |
| `audit` | `null` | Read and checked, but not used yet. |

**`org_id`** is 1 to 63 characters of `a-z`, `0-9` and `-`, starting with a letter or digit. The
id `personal` is reserved.

**`plain`** lists variable names. A plain entry's value shows in the Env tab, and it may go to a
dot's shell even while `shell_secrets` is false. Use it for settings such as `LOG_LEVEL`, never
for credentials.

**`use`** has two lists of globs. A glob is a variable name in which `*` stands for any run of
characters, including none. Globs match without regard to case. A name may be granted when it
matches an `allow` glob and no `deny` glob: **deny wins**. An empty `allow` allows nothing.

```json
"use": { "allow": ["*"], "deny": ["PROD_*", "*_ADMIN_TOKEN"] }
```

With this rule, `prod_db_url` is denied (case doesn't matter), and so is `GITHUB_ADMIN_TOKEN`.
`use` applies to the name, whichever side its value would come from. A Personal `PROD_TOKEN` is
denied too, in a dot of this org.

**`personal`** says how a person's Personal variables mix with the org's, in a dot of the org.
Say the org has `GITHUB_TOKEN`, and the person has Personal `GITHUB_TOKEN` and `NPM_TOKEN`:

| `personal` | `GITHUB_TOKEN` | `NPM_TOKEN` |
|---|---|---|
| `"deny"` | the org's | not available |
| `"fallback"` | the org's | the person's |
| `"override"` | the person's | the person's |

- `"deny"`: only the org's entries resolve. Use it when every credential must be the org's.
- `"fallback"`: the org's entry wins, and a Personal one fills a name the org lacks. Use it when
  people bring their own keys for services the org does not manage.
- `"override"`: a Personal entry wins over the org's. Use it when the org's values are only
  defaults, such as a shared sandbox key that people may replace with their own.

The Env tab marks an org row "Locked" under `deny` and `fallback`, where a Personal variable
can't take its place.

**`shell_secrets`** decides whether a secret may be granted to a dot's shell, where the agent can
read it and could send it anywhere its tools reach. While it is `false`, the dot editor does not
offer secrets for the shell, and a run leaves them out. This covers an org secret, and a Personal
secret in a dot of the org. MCP servers still get the secrets granted to them.

**`refresh_minutes`** is how often each person's Dots fetches the org. Dots also fetches at start,
when the person presses **Refresh**, and before a run whose copy is older than this.

**`offline_hours`** is how long Dots may use the saved copy after the last good fetch, while the
source can't be reached. Once it runs out, the copy is deleted and runs that need the org fail
until the PC reconnects. With `0`, Dots writes no copy at all: the org's values are held in
memory only while the source answers, and an offline PC has none.

**`audit`** is either `null` or `{"endpoint": "https://...", "token_entry": "NAME"}`. The
endpoint must be an `https://` URL and the entry a variable name. Dots checks it and does
nothing else with it yet. A later release will send audit records there.

### Making, checking and signing

```bash
dots-admin policy new --org-id acme-eng --out policy.json
```

This writes a policy with every field at its default and `issued_at` set to now. Without `--out`,
it prints the policy. It never replaces an existing file, since that may hold your edits.

Edit `policy.json`, then check it as Dots will:

```bash
dots-admin policy check policy.json
```

It prints `ok` and the policy in plain words, the same words people see in Settings:

```text
ok
  org: acme-eng
  issued at: 2026-10-24T05:16:08Z
  plain entries: none; every org entry is a secret
  dots may be granted: every org entry
  personal entries: not used; only the org's entries resolve
  secrets in the shell: not allowed
  refresh: every 15 minutes
  offline copy: kept for 24 hours after the last good fetch
  audit: off
```

A policy Dots would refuse prints `policy invalid: <why>` (for example `policy invalid: unknown
field 'foo'`) and exits with code 1.

Sign it:

```bash
dots-admin policy sign policy.json --key keys/policy.key --out policy.signed.json
```

This checks the policy, sets `issued_at` to now, and signs the exact text. Only the `issued_at`
value changes, so your layout stays. It then checks the result as a Dots client would, and writes
it. The input file is not changed. The output says:

```text
signed the policy for org 'acme-eng', issued_at 2026-10-24T05:16:09Z, with key 3F9A 1C2B 77D2 0E4C 8C41 E56D 0F51 C2E0
wrote policy.signed.json; store its text as DOTS_POLICY
```

Without `--out`, the signed text goes to standard output and the note to standard error. The
signed text is one line of JSON. Store it in the source as section 4 says.

To see what a Dots client would accept:

```bash
dots-admin policy verify policy.signed.json --pub keys/policy.pub
dots-admin policy verify policy.signed.json --pub keys/policy.pub --org-id acme-eng
```

It prints `ok: a Dots client in org 'acme-eng' accepts this policy, signed by key ...`, the
summary, and a reminder that Dots still refuses it if it has accepted a policy issued later. A
refusal prints `a Dots client would refuse it: <why>`, such as `signature does not match` or
`for org 'acme-eng', not 'beta'`.

`dots-admin` exits with 0 on success, 1 when a command fails or finds the input invalid, and 2
for a wrong command line. `dots-admin help` lists every command.

### No going back

Each Dots remembers the `issued_at` of the last policy it accepted. It refuses a policy issued
before that, with the status `policy invalid: issued before the last accepted policy (...)`, and
keeps using its saved copy until the copy runs out.

There is no recovery from inside Dots. A policy signed with a far-future `issued_at` (a fast
clock, or a leaked key) makes every later policy count as older, even one that comes through
**Update** with a new key. The person must **Remove** the org and **Join** again.

So:

- Never store an older signed policy again. To undo a change, edit the policy and sign it again.
- Sign on a machine whose clock is right. A clock that runs ahead makes every later signature
  look old.

### Changing a policy or a value

- **A new policy**: edit, `policy check`, `policy sign`, and store the new text in place of the
  old one. Each Dots takes it at its next fetch.
- **A new value**: change it in the source. Each Dots takes it at its next fetch, which is at most
  `refresh_minutes` away while the PC is online. A run that has already started keeps the value it
  started with. A person can press **Refresh** to fetch at once.

## 4. Sources

Each subsection gives the join code's source fields, what the person types at Join, and where the
signed policy goes. The join code command itself is in section 5.

Every address must start with `https://`. `http://` is accepted only for `localhost` and
`127.0.0.1`, for trying things out. An address must not hold a user name or password.

### OpenBao and HashiCorp Vault

Dots reads a KV version 2 secrets engine. OpenBao and Vault use the same API, so this section
covers both.

**Join code fields:**

| Field | Default | Meaning |
|---|---|---|
| `address` | required | The server, such as `https://bao.example.invalid`. |
| `mount` | `secret` | The KV v2 mount. |
| `path` | required | The org's folder under the mount, such as `dots`. |
| `auth` | `token` | `token`, `userpass` or `ldap`. |
| `auth_mount` | `userpass` or `ldap` | The auth mount, when it is not at its default path. |
| `namespace` | none | Sent as `X-Vault-Namespace` on every request. |

**Layout.** Put the org's variables at `<mount>/<path>/env`, one key per variable. Put the signed
policy at `<mount>/<path>/policy`, in the key `DOTS_POLICY`. With the defaults that is
`secret/dots/env` and `secret/dots/policy`:

```bash
bao kv put -mount=secret dots/env GITHUB_TOKEN=test-value-1 LOG_LEVEL=info
bao kv put -mount=secret dots/policy DOTS_POLICY=@policy.signed.json
```

Every value must be text. A key that is not a variable name is skipped. Two names that differ only
in case make the fetch invalid, since Dots compares names without regard to case. `DOTS_POLICY`
is never an entry.

**A read-only ACL policy.** Each person needs `read` on these paths, and nothing else. In KV v2
the API path has `data/` after the mount. The token must also keep the `default` policy, since
Dots calls `auth/token/lookup-self` and `auth/token/renew-self`; or add those two paths to this
policy. A token made with `-no-default-policy` stops renewing and is then refused, which wipes the
copy:

```bash
bao policy write dots-acme - <<'EOF'
path "secret/data/dots/env"    { capabilities = ["read"] }
path "secret/data/dots/policy" { capabilities = ["read"] }
path "secret/data/dots/roster" { capabilities = ["read"] }
EOF
```

The `roster` path holds the member list (section 10). A token without it still works for an org
without a member list: Dots treats a refused read of `roster` as "no member list". Under a policy
that requires one, the account then waits, and shows "can't read DOTS_ROSTER at
secret/dots/roster (HTTP 403)" until you add the path.

**Signing in.** Give each person their own credential, so that you can cut one person off:

- **Token** (`auth: token`). Make a token with the `dots-acme` policy and hand it over in a safe
  channel. The person pastes it as **Token** at Join. Dots renews a renewable token at most once
  an hour, and only during a fetch. So a periodic token's period must be longer than both one
  hour and the policy's `refresh_minutes`. The token lapses if Dots stays closed longer than the
  period.
- **userpass** (`auth: userpass`). Make a user with the policy:

  ```bash
  bao auth enable userpass
  bao write auth/userpass/users/jdoe password=- token_policies=dots-acme
  ```

  The person types their **Username** and **Password** at Join. Dots logs in at
  `auth/<auth_mount>/login/<username>` and logs in again when the token runs out.
- **ldap** (`auth: ldap`). Map the person's LDAP user or group to the `dots-acme` policy in your
  ldap auth mount. The person types their directory **Username** and **Password**.

A login that fails because the directory can't be reached shows as **offline**, and the saved
copy is kept; only a refused password counts as access revoked.

**Namespaces.** For Vault Enterprise or HCP Vault, and for OpenBao namespaces, give the namespace
in the join code. Dots sends it as `X-Vault-Namespace` on every request, and the paths above are
inside it.

**Statuses.** A `403` from Vault means the token is bad or its policy lacks `read`. Dots treats
it as **access revoked** and wipes the person's copy. A missing path answers `404`, which Dots
shows as `policy invalid: no secrets at secret/dots/env`. A sealed server answers `503`, which is
**offline**.

### Infisical

Dots signs in with a machine identity that uses Universal Auth, and reads one folder of one
environment of one project. It works with Infisical Cloud and with a self-hosted Infisical.

**Join code fields:**

| Field | Default | Meaning |
|---|---|---|
| `site_url` | required | Your Infisical, such as `https://eu.infisical.com` or your own host. |
| `project_id` | required | The project's id. |
| `environment` | required | The environment's slug, such as `prod`. |
| `secret_path` | `/` | The folder, such as `/dots`. It must start with `/`. |

**Layout.** Put the org's variables as shared secrets in the folder `secret_path`. Put the signed
policy as the secret `DOTS_POLICY` in its sub-folder `policy`: `/dots/policy` for `/dots`, or
`/policy` for the root.

Dots reads only that folder. It reads no sub-folder and no secret imports, and it skips personal
overrides. A secret whose value Infisical hides from the identity makes the fetch invalid:
`the value of NAME is hidden from this machine identity`.

Infisical expands secret references (`${...}`) in values before Dots sees them. So a value can
pull in a secret from any folder the identity can read. Keep the identity's access to the org's
folders.

**Signing in.** Make one machine identity per person, with Universal Auth, and give it read access
to the folder and its `policy` sub-folder in that environment. Hand the person its client id and a
client secret. They type them as **Client id** and **Client secret** at Join.

### SOPS and age

The org's variables live in one JSON file, encrypted with SOPS to the age key of each person's PC.
The file can sit on a mapped network drive, in a folder that git or IT keeps in sync, or behind
an `https://` URL.

**Join code field:** `location`, required. It is either a full local path, or an `https://` URL
that answers the file to a plain `GET`:

- `C:\Sync\acme-eng.enc.json`
- `S:\dots\acme-eng.enc.json`, on a drive mapped to a share
- `https://files.example.invalid/dots/acme-eng.enc.json`

A network path such as `\\files.example.invalid\dots\acme-eng.enc.json` (also `//host/share/…`
and `\\?\UNC\…`) is refused: "the file's location is a network path; use a synced folder or a
mapped drive". Opening one would make Windows sign in to whatever host the path names. Have
each PC map the share to a drive letter, or sync the folder, and give that path.

**Only you may write the location.** Every recipient's key opens the file's data key, so anyone
who can write the file can change any `env` value and give the file a valid MAC: only the policy
is signed, by you. Keep the location writable by you alone, for example a protected branch that
only you can push to, a share whose permissions give everyone else read access only, or a URL
that only serves `GET`. A SOPS file protects the values against outsiders, not against members
who can write it.

**Layout.** Before encryption, the file is JSON with two fields. `env` holds the variables, each
value text. `policy` holds the signed policy's text as one JSON string:

```json
{
  "env": {
    "GITHUB_TOKEN": "test-value-1",
    "LOG_LEVEL": "info"
  },
  "policy": "{\"v\":1,\"policy\":\"...\",\"signature\":\"...\"}"
}
```

Only the JSON format is read. Other top-level fields are ignored. The file must use one key group
(the usual case) and no `shamir_threshold` above 1.

Every value under `env`, and the policy, must be encrypted. Don't give an `env` key the
`_unencrypted` suffix or match it with an `unencrypted_regex`: Dots refuses the whole file
("the file's value at env:NAME is not encrypted"). An empty value is skipped.

A small script builds the file from `env.json` (the variables) and the signed policy:

```bash
python - <<'EOF'
import json
env = json.load(open("env.json"))
policy = open("policy.signed.json").read()
open("acme-eng.plain.json", "w").write(json.dumps({"env": env, "policy": policy}, indent=2))
EOF
```

**This PC's key.** Dots makes an age key for each SOPS org on each PC. At Join, after the preview,
the person sees **This PC's key** (an `age1...` public key) with a **Copy** button and the hint
"Send this key to your admin, then press Join". If they press **Join** before you add the key,
Dots shows "Waiting for your admin to add this PC's key" and the button reads **Try again**. Their
key stays the same if they close the dialog or restart Dots and join again.

**Recipients.** List every person's key, and your own, in a `.sops.yaml` next to the file:

```yaml
creation_rules:
  - path_regex: acme-eng\.(plain|enc)\.json$
    age: >-
      age1<admin key>,
      age1<jdoe's PC key>
```

Encrypt, then delete the plain file:

```bash
sops --encrypt --input-type json --output-type json acme-eng.plain.json > acme-eng.enc.json
rm acme-eng.plain.json
```

To add or remove a person later, change the `age` list and run:

```bash
sops updatekeys -y acme-eng.enc.json
```

`sops rotate -i --add-age <key> acme-eng.enc.json` and `sops rotate -i --rm-age <key>
acme-eng.enc.json` do the same without the config file, and also make a new data key.

**Changing a value or the policy** without a plain copy on disk:

```bash
sops set acme-eng.enc.json '["env"]["GITHUB_TOKEN"]' '"test-value-2"'

python - <<'EOF'
import json
open("policy.value", "w").write(json.dumps(open("policy.signed.json").read()))
EOF
sops set --value-file acme-eng.enc.json '["policy"]' policy.value
```

Then copy the file to its location.

**Statuses.** A file Dots can't read at a local path counts as **offline**. A file that does not
include this PC's key, or that was changed by hand so that its MAC fails, counts as **access
revoked**. For a URL, `401` and `403` count as access revoked and `404` makes the fetch invalid.
So a URL that starts refusing Dots, such as a signed URL past its expiry, wipes each person's copy
at their next fetch. Dots never shows the URL's query in a message.

## 5. Join codes

```bash
dots-admin join-code --org-id acme-eng --name "Acme Engineering" --source openbao \
  --address https://bao.example.invalid --path dots --pub keys/policy.pub
```

This prints the code on one line, and notes on standard error:

```text
org: Acme Engineering (acme-eng)
source: openbao at https://bao.example.invalid
policy key fingerprint: 3F9A 1C2B 77D2 0E4C 8C41 E56D 0F51 C2E0; Dots shows it on joining, so tell people to expect it
```

The source options, by `--source`:

```text
openbao:   --address <url> --path <path> [--mount <kv mount>]
           [--auth <token|userpass|ldap>] [--auth-mount <mount>] [--namespace <ns>]
infisical: --site-url <url> --project-id <id> --environment <slug>
           [--secret-path </path>]
sops:      --location <full path or https:// url>
```

More examples:

```bash
dots-admin join-code --org-id acme-eng --name "Acme Engineering" --source openbao \
  --address https://bao.example.invalid --path dots --auth userpass --namespace eng \
  --pub keys/policy.pub --file acme-eng.dotsorg

MSYS_NO_PATHCONV=1 dots-admin join-code --org-id acme-eng --name "Acme Engineering" \
  --source infisical --site-url https://infisical.example.invalid --project-id <project id> \
  --environment prod --secret-path /dots --pub keys/policy.pub --file acme-eng.dotsorg

dots-admin join-code --org-id acme-eng --name "Acme Engineering" --source sops \
  --location https://files.example.invalid/dots/acme-eng.enc.json --pub keys/policy.pub
```

`dots-admin` builds the code, reads it back as Dots will, and refuses one Dots would refuse: an
`http://` address, a relative SOPS path, an option for another source, a missing required option,
or `--auth-mount` without `--auth userpass` or `--auth ldap`. `--pub` takes `policy.pub` from
`keygen`.

**The `.dotsorg` file.** `--file` writes the code to a file, on one line. The person can paste
the code or choose the file with **Or open a .dotsorg file** in the Join dialog. The code and
the file hold no secret, but they say where your source is, so send them to your people only.
The exception is a SOPS `location` URL whose query carries a token (a signed URL): that code
holds a secret, so send it like one.

**What the person does.** In **Settings → Organisations → Join an organisation**, they paste the
code and press **Preview**. Dots shows the org, the source and the key fingerprint, and fetches
nothing yet. They check the fingerprint with you and press **Continue**, give their sign-in
details, and press **Join**.
Dots fetches once and joins only when the fetch works and the policy's signature checks out.

**Changing the org's settings.** For a new address, path or key, make a new join code for the same
org id. People use **Update** on the org, not Join. Dots shows what changes and fetches with the
new settings, and replaces the old ones only when that fetch and the signature check work. Their
dots and grants stay. While the source stays the same, a person who leaves the sign-in fields
empty keeps their saved details.

**Update never sends the saved details to a changed source.** When the new code changes anything
about an OpenBao or Infisical source (the address or site, the auth method or its mount, the
namespace, mount, path, project, environment or secret path) or the kind of source, Dots asks
for the sign-in details again, as at Join, and sends nothing until they are given. A SOPS file's
new location keeps the PC's age key, which is never sent anywhere. The details a person types go
to the new source before the new policy's signature is checked, so tell people to use Update
only with a code that came from you.

## 6. Day to day

### Rotation

Change the value in the source. Each Dots uses the new value from its next fetch, which is at
most `refresh_minutes` away while the PC is online. A run that has already started keeps the
value it started with. An offline PC keeps the old value in its copy until it reconnects or the
copy runs out. When the old value must stop working, revoke it where it was issued.

For a SOPS file, check now and then that only you can write its location: any recipient who can
write it can change the values every other person's Dots reads (see [SOPS and age](#sops-and-age)).

### Cutting someone off

Take away their access at the source:

- **OpenBao or Vault**: revoke their token. For userpass or ldap, delete the user or remove
  their mapping to the policy, and also revoke the tokens they were given: Dots keeps using a
  login token until it runs out. For example:

  ```bash
  bao token revoke -mode=path auth/userpass/login/jdoe
  bao token revoke -mode=path auth/ldap/login/jdoe
  ```
- **Infisical**: remove their machine identity from the project, or delete it.
- **SOPS**: removing the key from the recipients is not enough: `sops updatekeys` wraps the
  same data key again, and the person's old copy of the file still gives them that key. Rotate
  the data key and remove their key in one step, then copy the file out:

  ```bash
  sops rotate -i --rm-age <their public key> acme-eng.enc.json
  ```

  Or run `sops updatekeys -y acme-eng.enc.json` and then `sops rotate -i acme-eng.enc.json`.
  Also remove their access to the file's location: the repository, the share or the URL.

At their Dots' next fetch the source refuses it. Dots marks the org **access revoked** and wipes
their saved copy and the values it holds. Their dots stay, but can't run with the org's values.

A PC that stays offline does not fetch, and keeps its copy until `offline_hours` run out. After the
step above, rotate the values the person could read, as you would for any leaver. For SOPS,
rotate the data key first: values changed with `sops set` before that stay under the old key.

### Offline

Each Dots keeps a saved copy of the org's values and policy at
`%LOCALAPPDATA%\dots\vault\acme-eng.bin`, encrypted with Windows DPAPI for that Windows user. It
can't be read by another Windows account or on another machine. It lasts `offline_hours` after the
last good fetch.

- While the source can't be reached, Dots runs on the copy and shows **offline**.
- When the copy runs out, Dots deletes it and shows **expired**.
- After a failed fetch, a run start does not try again for 5 minutes, so a dead source slows a run
  start at most once in that time. The timer and **Refresh** still try on their own schedule.
- With `offline_hours: 0` there is no copy.

Each status, as Dots words it. Settings and the Env tab show it with a capital first letter:

- `connected`: the last fetch worked.
- `offline (<why>); using the saved copy until <time>`: the source can't be reached. Runs use
  the copy.
- `offline (<why>), with no saved copy`: the source can't be reached, and there is no copy. Runs
  that need the org fail. Right after Dots starts, the status is offline until the first fetch.
- `access revoked (<why>)`: the source refused the person. Their copy is gone.
- `policy invalid: <error>; using the saved copy until <time>`: the fetch, or its policy, was
  refused. Runs use the last good copy.
- `policy invalid: <error>`: the same, with no copy left.
- `secrets expired offline; reconnect to refresh`: the copy ran out while offline.
- `waiting for the admin to add this PC's key`: a SOPS join waits for you to add the PC's key.
  The Join dialog shows it; the list does not.

A run that needs a value the org can't give fails with a message that names the variable, the org
and the reason, for example:

```text
GITHUB_TOKEN: not in Acme Engineering (dot 'triage', mcp:github)
PROD_DB_URL: Acme Engineering's policy does not allow it (dot 'triage')
GITHUB_TOKEN: Acme Engineering does not allow secrets in the shell (dot 'triage')
Acme Engineering: secrets expired offline; reconnect to refresh
Acme Engineering: access revoked
Acme Engineering: policy invalid: signature does not match
Acme Engineering: offline, with no saved copy
dot 'triage': its org 'acme-eng' was removed
```

### A leaked key

- **A leaked value**: change it at the source, and revoke the old one where it was issued. Each
  Dots takes the new value at its next fetch.
- **A leaked policy key**: make a new key with `dots-admin keygen --out keys-2`, sign the policy
  with it, and send everyone a new join code with the new `policy.pub`. They use **Update**, and
  Dots asks them to accept the new key.

To change the key without a gap, put the new copy next to the old one:

1. Store the org's values and the policy signed with the new key at a new place: a new OpenBao
   `path`, a new Infisical folder, or a new SOPS file.
2. Make the new join code for that place and send it.
3. Once everyone has updated, delete the old place.

If you instead replace the policy in place, Dots that have not updated yet show `policy invalid:
signature does not match` and run on their copy until they update or the copy runs out.

## 7. Network

- **TLS.** Dots trusts the public certificate roots and the Windows certificate store. A source
  behind your company's own CA works when that CA is in the Windows store, as IT usually puts it.
- **Errors.** `401` and `403` mean **access revoked**, and the person's copy is wiped. Every `5xx`
  and `429` counts as **offline**: the copy stays. Any other status, or an answer Dots can't read,
  is **policy invalid**, and the last good copy stays.
- **Timeout.** Each request has 20 seconds.
- **Redirects** are not followed: a `3xx` is policy invalid. Give the address that answers
  directly.
- **Size.** An answer, or a SOPS file, is at most 1 MiB.

## 8. MCP servers and origins

An org's values can fill `${VAR}` in a Library MCP server's settings, for a dot of that org, as
Personal values do. Two rules keep them from going where the person did not mean:

- **Confirmed origins.** An HTTP server whose URL reads a variable, such as
  `https://${MCP_HOST}/mcp`, starts only for an origin the person confirmed in that scope. A
  value in your org can point the URL at another host than the person's Personal value does, so
  Dots asks per org. Until then, a run leaves the server out with the notice "github:
  https://mcp.example.invalid is not a confirmed origin; open the server in the Library to
  confirm it". A server whose URL is fixed needs no confirmation.
- **Check as.** In the Library, the server's form has **Check as**, with Personal and each joined
  org. It lists the server's tools with that scope's values. When the origin is not confirmed in
  that scope, Dots first asks in its own window, naming the variables, the org and the origin. A
  no sends nothing.

Each scope keeps its own list of the server's tools. The person reviews a changed list per org in
the Inbox or the Library.

If you change a value that forms a server's host, each person confirms the new origin once.

## 9. Limits

- **Size.** A Personal secret, and each person's sign-in details for an org, are at most 2,560
  bytes: Windows Credential Manager's limit for one value. The org's own values are not kept there
  and have no such limit beyond the 1 MiB answer.
- **Names compare without regard to case**, as Windows environment names do. `github_token` and
  `GITHUB_TOKEN` are one variable.
- **Bedrock and Vertex.** Dots does not pass its own `AWS_*` and `GOOGLE_*` variables to the
  `claude` CLI, so a CLI set up for Amazon Bedrock or Google Vertex AI that way does not work in
  Dots.
- **Audit records are not sent yet.** The policy's `audit` field is checked, and a later release
  will use it. Until then, your source's own audit log is the record of who fetched what.
- **Windows only.** Dots keeps secrets in Windows Credential Manager and DPAPI.
- **What Dots can't stop.** Dots stops mistakes and honest use. It can't stop malware running as
  the same Windows user, a person who extracts a value they were granted, or a granted MCP server
  or shell that sends a value elsewhere. Revoking access at the source, rotating values, and the
  source's own audit log are your controls there.

## 10. Accounts with a member list

A plain org (policy v1) lets anyone who can read the source join, and every member counts as a
Member. An account with a **member list** (the roster) adds names, roles and a way to remove
someone. You can edit the list with `dots-admin`, or from the **Members** tab in Dots (see
"Inviting and approving in Dots" below). Owners can only be changed with `dots-admin` and the root
key.

### The root key and admin keys

- The **root key** is the policy key from section 2 (`policy.key`). It is the Owner's key. Only it
  can change the policy, and it can always sign the member list.
- An **admin key** is the Dots ID of a person you trust to sign the member list. It is listed in
  the policy's `admins`. An admin can sign new lists, but can't change the policy, the root key or
  `admins`, and can't add, change or remove an Owner: a list signed by an admin must keep the
  Owner entries of the list each Dots accepted last, word for word, or it is refused ("this
  member list changes the Owners, which only the owner's key may sign; an Admin signed it").
  Sign a list that changes Owners with the root key. Keep the root key off everyone else's PC.

Add or drop an admin in the policy, then sign the policy again (section 3):

```bash
dots-admin policy admins add policy.json <dots-id>
dots-admin policy admins remove policy.json <dots-id>
```

Dropping an admin does not undo lists they already signed: each Dots accepted those when it
fetched them, and keeps them. It refuses new lists signed by that admin.

### Making and signing the list

Start a new org's policy as v2 with the list required (to move an existing org, see below):

```bash
dots-admin policy new --org-id acme-eng --roster --admin <asha's dots-id> --out policy.json
dots-admin roster new --org-id acme-eng --owner-name "Asha Rao" --owner-email asha@acme.com \
    --owner-key <asha's dots-id> --out roster.json
dots-admin roster add roster.json --name "Ben Li" --email ben@acme.com --key <ben's dots-id> --role member
dots-admin roster set-role roster.json ben@acme.com admin
dots-admin roster check roster.json --org-id acme-eng
dots-admin roster sign roster.json --key keys/policy.key --out roster.signed.json
```

- Roles are `owner`, `admin`, `member` and `viewer`. The list needs at least one owner.
- The Dots ID is what the person copies with **Settings › Profile › Copy my Dots ID**.
- `roster add` with an email that is already listed adds the key as that person's **second PC**.
- `roster sign` signs with the root key, raises `seq` by one, sets `issued_at`, writes both back
  into `roster.json` and prints the signed text. Keep `roster.json`: it is the file you edit next
  time. Each Dots refuses a list whose `seq` is not higher than the last it accepted.
- `roster verify roster.signed.json --pub keys/policy.pub --org-id acme-eng` (add `--admin
  <dots-id>` for an admin-signed list) shows what a Dots client would accept.

### Where to store DOTS_ROSTER

Store the signed text next to `DOTS_POLICY`:

- **OpenBao or Vault**: key `DOTS_ROSTER` at `<mount>/data/<path>/roster`, a second secret beside
  the policy's.
- **Infisical**: secret `DOTS_ROSTER` in `<secret path>/policy`, the folder that holds
  `DOTS_POLICY`. It is not shown as an entry.
- **SOPS**: a top-level field `roster` in the file. Encrypt it like `policy`: Dots refuses a file
  whose `roster` is in clear text.

Give people read access to it as you did for the policy. Keep write access to the Owner and admins (see "Write rights and drafts").

### Inviting and approving in Dots

An Owner or Admin can add people from **Settings › Accounts**. Open the account and choose its
**Members** tab.

**Signing needs your Dots ID in `admins`.** Dots signs the new list with this PC's Dots ID, so that
ID must be in the policy's `admins`. This holds for the Owner's PC too. The owner adds it with the
root key, signs the policy and stores it (section 3):

```bash
dots-admin policy admins add policy.json <dots-id>
```

Until then the buttons on the tab are off. The tab says "Can sign the list once the owner adds
their Dots ID to the policy's admins."

**Invite.** Choose **Invite**, give the person's email and a role, and choose **Create invite**.
Dots shows an invite code. Choose **Copy**, or **Save as .dotsorg** to send a file. Send it to the
person.

- An invite lives only on the PC that made it. Another PC of yours does not see it.
- It expires after 7 days. **Open invites** lists what is waiting, with **Revoke**.
- The person joins with the code (see [Accounts](accounts.md)). Dots then shows them a request
  code, which they send back to you.
- **SOPS:** the person first sends you their PC's age key, shown in their Join dialog as **This
  PC's key**. Add it to the file's recipients (section 4) and store the file. Until then their join
  waits ("Waiting for your admin to add this PC's key") and there is no request code. Once the key
  is in, they choose **Try again**, the join completes, and Dots shows them the request code.

**Approve.** Choose **Approve request**, paste the request code and choose **Check**. Dots shows
the name, email and Dots ID fingerprint, and any problem. Choose **Approve**. Dots signs a new
list and publishes it.

- A request that matches one of your invites gets the invite's role.
- A request with no invite from this PC needs a tick on "This person has no invite from this PC"
  and a role.
- A request with an email that is already on the list joins that member, as another PC, with
  their existing role. The dialog says who.
- Approving uses up the invite, even when the result is only a draft (see below). If that draft
  is discarded, or dropped because another list took its place, the invite is open again and the
  same request code can be approved later.
- A request for another account, a revoked, used or expired invite, or a Dots ID already on the
  list is refused with the reason. So is a request with an Owner's email: "Owners can only be
  changed with the account owner's key, using dots-admin."

**Change a role, remove a PC, remove a person.** Each member has a role menu, **Remove this PC**
for each of their PCs, and **Remove**. Each change signs a new list. Dots does not touch an Owner:
it says "Owners can only be changed with the account owner's key, using dots-admin." Do that with
`dots-admin` and the root key, as above.

**Removing someone also needs their credential revoked at the source.** The confirmation says so.
The list stops Dots honouring them. Only the source cuts them off (section 6).

Take care when you remove the key of the PC you are on. Once the new list is published this PC is
not on it, and can't sign any more. **Remove this PC** always asks first, and says "This is this
PC. You'll lose access to <account> here." for your own PC, or "This is their last PC; they'll be
removed." for a person's only PC. Changing your own role asks too: "You're changing your own role.
You may lose the right to manage members."

### Write rights and drafts

Publishing writes `DOTS_ROSTER` to the source, so the signing PC's credential needs write rights
there. Reading is not enough.

- **OpenBao or Vault**: the token's policy must allow `create` and `update` on the roster path.
  Give each signing Admin an ACL policy like this (the read paths stay as in section 4):

```bash
bao policy write dots-acme-admin - <<'EOF'
path "secret/data/dots/env"    { capabilities = ["read"] }
path "secret/data/dots/policy" { capabilities = ["read"] }
path "secret/data/dots/roster" { capabilities = ["read", "create", "update"] }
EOF
```

- **Infisical**: the machine identity needs write on the policy folder's `DOTS_ROSTER` secret.
- **SOPS**: Dots can't write the file. See below.

If the source refuses the write, or can't be reached, Dots keeps the signed list as a **draft**. The
Members tab shows "Not published yet:" with what it changes, the error, and **Retry** and
**Discard**. Fix the rights, then choose **Retry**. A draft is not live: nobody else sees the
change until it is published. If the source took the list but Dots can't read it back yet, the
error says "The source took the list but Dots couldn't read it back yet. Retry in a moment."

One change at a time: while a draft waits, **Approve request**, the role menus and the remove
buttons are off with "<account>: a change is waiting to be published. Retry or discard it first."
**Invite** and **Revoke** still work.

If another admin publishes first, Dots refuses and says "the member list changed (now #N) while you
were editing. Check it and make the change again." Look at the new list and redo your change. A
draft that another list took the place of is never counted as published, even when that list's
number is higher: the draft stays with that message until you choose **Retry** (which drops it and
says so) or **Discard**.

**SOPS.** For a SOPS file the draft's message is "Copy the signed list into the SOPS file (see the
admin guide)." Choose **Copy signed list**, then put the text in the file's `roster` field with
`sops` (section 4):

```bash
sops set acme-eng.enc.json '["roster"]' '"<the signed list, as a JSON string>"'
```

Store the file as before. On **Refresh**, Dots reads the new list and drops the draft. The
approved person's age key is a recipient already: their join waited for it, before they had a
request code (see **Invite** above).

### Requests with dots-admin

You can read and add a request code without Dots:

```bash
dots-admin request inspect <code>
dots-admin roster add-request roster.json <code> --role member
```

`request inspect` checks the code's signature and prints the org, name, email, key fingerprint,
invite id (or `none`), age recipient if any, and when it was made. `roster add-request` adds the
sender to `roster.json`, or adds another PC if the email is already there, and prints the age
recipient the request carries (for a SOPS file it is a recipient already). A request is only what
its sender claims: for an email already on the list, `--role` must be that member's role, or it is
refused as `roster add` refuses it (change roles with `roster set-role`); when it matches, it
prints "This adds a PC to <name> (<Role>)." Sign and store the list as above.

### Moving a legacy org to a member list

Do it in this order so nobody is locked out by surprise:

1. Make sure every person has updated Dots first. An older Dots can't read a v2 policy.
2. Make `roster.json` with every current member (ask each for their Dots ID) and sign it. Store
   it as `DOTS_ROSTER`. Policy v1 ignores it, so nothing changes yet. On OpenBao or Vault, add the
   `roster` path to each person's ACL policy (section 4).
3. Only then make a v2 policy with `"roster": "required"` (`policy new --roster`, or copy your v1
   fields into a v2 file), sign it and store it as `DOTS_POLICY`. A v1 policy that has `roster`
   or `admins` is refused.

A PC that is not on the list when v2 starts shows **Waiting** with **Copy my Dots ID**. Nothing is
wiped. Ask the person for their ID, run `roster add`, `roster sign`, store the new list, and press
**Refresh**.

If someone's saved key was lost, their Dots ID changed. Their PC waits with "this PC's Dots ID
changed. Send the new one to an Admin." Run `roster add` with the same email and the new key (the
old key can go with `roster remove --key`), then sign and store.

If you change the policy key (**Update** with a new join code), sign the list again so `seq` goes
up by one. The same list at the same `seq` is refused.

### Removing someone

```bash
dots-admin roster remove roster.json ben@acme.com
dots-admin roster sign roster.json --key keys/policy.key --out roster.signed.json
```

You can also use **Remove** on the **Members** tab. Store the new list. At the person's next fetch their account shows **Removed** and Dots wipes that
account's secrets on that PC. If they come back, they join again and wait for you to add them.

The list only stops Dots honouring them. **Also revoke their credential at the source**, as in
section 6 (Cutting someone off). Without that, someone running a changed Dots can still read the
source.

### What this does and does not protect

Protected against:

- A member editing the member list or the policy on their own PC: both are signed.
- An old list being replayed: each Dots refuses a lower `seq`.
- A removed member using the account's secrets after their next refresh: the saved copy is wiped.
- A stolen admin PC promoting others to admin: only the root key changes `admins`.

Dots's local API (the webhook and MCP port on this PC) is not yet guarded by a Host check and a
Bearer key for everything it serves. That protection arrives with API keys in a later release
(phase 5).

Not protected against:

- A person patching their own Dots to ignore their role. Roles stop mistakes and honest use. The
  real cut-off is revoking the source credential, the same trade-off as for the vault.
- A removed person's PC that stays offline. It keeps its saved copy until `offline_hours` run out.
- A stolen PC while its Windows login is unlocked.
