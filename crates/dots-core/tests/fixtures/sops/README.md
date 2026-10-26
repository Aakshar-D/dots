# SOPS fixtures

Test files for the SOPS + age org source (`src/vault/org/sources/sops.rs`, tested by
`tests/vault_org_sops_test.rs`). The real tools made every `*.enc.json` here. No file holds a
real secret.

- **Tools:** sops 3.13.3 and age v1.3.2 (`age-keygen` from the same release), installed on
  Windows with `winget install SecretsOPerationS.SOPS` and `winget install FiloSottile.age`.
- **Shell:** Git Bash, run in this folder. No `.sops.yaml` applies here, so the flags below are
  the whole configuration.
- **The `policy` value** in every file is a placeholder string, not a signed policy. The SOPS
  source hands the text on without checking it; the `Orgs` service verifies it.

## Files

- `test-identity.txt`: the test-only age identity the tests open the files with.
- `other-identity.txt`: a second test-only identity, not a recipient of most files.
- `*.plain.json`: the plain inputs.
- `good.enc.json`: `good.plain.json`, for `test-identity.txt`.
- `mac-only.enc.json`: the same, made with `--mac-only-encrypted`.
- `two-recipients.enc.json`: the same, for both identities.
- `not-recipient.enc.json`: the same, for `other-identity.txt` only.
- `encrypted-regex.enc.json`: the same, encrypting only `env` and `policy`. Its other values
  stay plain, and the MAC still covers them.
- `tampered.enc.json`: `good.enc.json` with the first two values of `extra.replicas` swapped.
  Both have the AAD `extra:replicas:`, so each still decrypts, and only the MAC catches it.
- `unencrypted-changed.enc.json`: `good.enc.json` with `note_unencrypted` edited, which fails
  the MAC.
- `not-text.enc.json`: an `env` value that is a number.
- `no-policy.enc.json`: no `policy`.
- `names.enc.json`: names the source skips: `DOTS_POLICY`, `dots_policy`, `1BAD` and
  `has-dash`.
- `case-clash.enc.json`: `API_KEY` and `Api_Key`.

The tests make the other bad files they need in a temporary folder, by changing the text of
`good.enc.json`.

## Making them again

The identities. `age-keygen` prints each public key. The tests check the public keys, so after
making new identities, also update `TEST_RECIPIENT` and `OTHER_RECIPIENT` in the test file.

```bash
age-keygen -o test-identity.txt
age-keygen -o other-identity.txt
```

Then add this as the first line of both files:

```text
# For tests only: a throwaway key that protects no real secret. Never use it elsewhere.
```

Add this as the second line of `other-identity.txt`:

```text
# It is not a recipient of the files encrypted to test-identity.txt alone.
```

The encrypted files:

```bash
T=$(age-keygen -y test-identity.txt)
O=$(age-keygen -y other-identity.txt)
enc() { sops --encrypt --input-type json --output-type json "$@"; }

enc --age "$T" good.plain.json > good.enc.json
enc --mac-only-encrypted --age "$T" good.plain.json > mac-only.enc.json
enc --age "$T,$O" good.plain.json > two-recipients.enc.json
enc --age "$O" good.plain.json > not-recipient.enc.json
enc --encrypted-regex '^(env|policy)$' --age "$T" good.plain.json > encrypted-regex.enc.json
for f in not-text no-policy names case-clash; do
  enc --age "$T" $f.plain.json > $f.enc.json
done
```

The two changed files, edited with Python so that nothing else changes:

```bash
python - <<'EOF'
import json

def change(name, edit):
    with open('good.enc.json') as f:
        file = json.load(f)
    edit(file)
    with open(name, 'w', newline='\n') as f:
        json.dump(file, f, indent='\t')
        f.write('\n')

def swap_replicas(file):
    replicas = file['extra']['replicas']
    replicas[0], replicas[1] = replicas[1], replicas[0]

def edit_note(file):
    file['note_unencrypted'] = 'visible text, edited by hand'

change('tampered.enc.json', swap_replicas)
change('unencrypted-changed.enc.json', edit_note)
EOF
```

To check them with sops itself: `good`, `mac-only`, `two-recipients` and `encrypted-regex`
decrypt; `tampered` and `unencrypted-changed` fail with "MAC mismatch"; `not-recipient` fails to
get the data key.

```bash
for f in *.enc.json; do
  echo "== $f"
  SOPS_AGE_KEY_FILE=test-identity.txt sops --decrypt $f > /dev/null && echo ok
done
```

The tests that use the files read the text exactly as written above, such as `"sops": {` and
the tab indents, so remake a file with these commands rather than by hand.
