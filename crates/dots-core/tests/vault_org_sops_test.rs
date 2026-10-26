//! Reading an org from a SOPS file encrypted with age (env vault spec §6.1 and §6.2), against
//! fixtures the real `sops` and `age` tools made (`tests/fixtures/sops/README.md`): the entries
//! and the policy, the MAC, the recipients, the metadata Dots refuses, a file on disk and one
//! over HTTP, and the identity a PC makes for an org. No error ever holds a value.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::fake_files::FakeFiles;
use dots_core::vault::org::sources::sops::{new_identity, public_key};
use dots_core::vault::org::{
    http_client, source_for, Fetched, Sops, SopsSettings, Source, SourceCreds, SourceError,
    SourceSettings,
};
use dots_core::Error;
use serde_json::Value;
use zeroize::Zeroizing;

const API_KEY: &str = "fixture-api-key-9d27b3";
const DATABASE_URL: &str = "postgres://dots:fixture-db-pass-4e1c@db.example.test:5432/app";
const POLICY: &str = "placeholder-signed-policy-5a8e: the SOPS source does not check it";

/// The public keys of `test-identity.txt` and `other-identity.txt`.
const TEST_RECIPIENT: &str = "age1a03d7rmd3zc7lpcxldf24f2g2gyhgphdwesczxmtf0vfvcjtcshseuswgt";
const OTHER_RECIPIENT: &str = "age16fa053f93s48cuu70erfpw7yygxuwkns96x5ptg2ldsrlceqgf8qw8r74u";

/// Values in the fixtures, and pieces of them, that no error may hold.
const VALUES: &[&str] = &[
    API_KEY,
    "fixture-db-pass-4e1c",
    "placeholder-signed-policy",
    "fixture-skipped-1f0a",
    "eu-west-1",
    "secret-sig-77",
];

const MIB: usize = 1024 * 1024;

fn assert_clean(e: &SourceError) {
    let text = format!("{e} {e:?}");
    for value in VALUES {
        assert!(!text.contains(value), "{text:?} holds {value:?}");
    }
    for file in ["test-identity.txt", "other-identity.txt"] {
        let identity = identity(file);
        assert!(
            !text.contains(identity.as_str()),
            "{text:?} holds an identity"
        );
    }
}

fn invalid(e: SourceError) -> String {
    assert_clean(&e);
    match e {
        SourceError::Invalid(why) => why,
        other => panic!("expected Invalid, got {other:?}"),
    }
}

fn denied(e: SourceError) -> String {
    assert_clean(&e);
    match e {
        SourceError::Denied(why) => why,
        other => panic!("expected Denied, got {other:?}"),
    }
}

fn unreachable(e: SourceError) -> String {
    assert_clean(&e);
    match e {
        SourceError::Unreachable(why) => why,
        other => panic!("expected Unreachable, got {other:?}"),
    }
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/sops")
        .join(name)
}

/// A fixture's text, with the line ends git may have changed made `\n` again.
fn fixture_text(name: &str) -> String {
    std::fs::read_to_string(fixture(name))
        .unwrap()
        .replace("\r\n", "\n")
}

/// The `AGE-SECRET-KEY-1…` line of an identity file.
fn identity(file: &str) -> Zeroizing<String> {
    let text = Zeroizing::new(fixture_text(file));
    let line = text
        .lines()
        .find(|line| line.starts_with("AGE-SECRET-KEY-1"))
        .expect("an identity line");
    Zeroizing::new(line.to_string())
}

fn age(file: &str) -> SourceCreds {
    SourceCreds::Age {
        identity: identity(file),
    }
}

fn mine() -> SourceCreds {
    age("test-identity.txt")
}

fn sops_at(location: &str) -> Sops {
    Sops::new(
        SopsSettings {
            location: location.to_string(),
        },
        http_client(),
    )
}

fn local(path: &Path) -> Sops {
    sops_at(&path.display().to_string())
}

async fn open(name: &str, creds: &SourceCreds) -> Result<Fetched, SourceError> {
    local(&fixture(name)).fetch(creds).await
}

/// Writes `text` to a new file in `dir` and gives its path.
fn write(dir: &tempfile::TempDir, text: &str) -> PathBuf {
    let path = dir.path().join("org.sops.json");
    std::fs::write(&path, text).unwrap();
    path
}

/// `good.enc.json` with `from` replaced by `to`, which must be there once, opened.
async fn open_changed(from: &str, to: &str) -> Result<Fetched, SourceError> {
    let text = fixture_text("good.enc.json");
    assert_eq!(text.matches(from).count(), 1, "{from:?}");
    let dir = tempfile::tempdir().unwrap();
    let path = write(&dir, &text.replace(from, to));
    local(&path).fetch(&mine()).await
}

/// The `ENC[…]` text of `env.<name>` in `good.enc.json`.
fn encrypted(name: &str) -> String {
    let file: Value = serde_json::from_str(&fixture_text("good.enc.json")).unwrap();
    file["env"][name].as_str().unwrap().to_string()
}

fn entries(fetched: &Fetched) -> Vec<(&str, &str)> {
    fetched
        .entries
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect()
}

fn good_entries() -> Vec<(&'static str, &'static str)> {
    vec![
        ("API_KEY", API_KEY),
        ("DATABASE_URL", DATABASE_URL),
        ("LOG_LEVEL", "debug"),
    ]
}

// Files that open.

#[tokio::test]
async fn the_good_file_gives_its_exact_entries_and_policy() {
    let fetched = open("good.enc.json", &mine()).await.unwrap();
    assert_eq!(entries(&fetched), good_entries());
    assert_eq!(fetched.policy, POLICY);
}

#[tokio::test]
async fn a_file_with_a_mac_of_its_encrypted_values_only_opens() {
    let text = fixture_text("mac-only.enc.json");
    assert!(text.contains("\"mac_only_encrypted\": true"));
    let fetched = open("mac-only.enc.json", &mine()).await.unwrap();
    assert_eq!(entries(&fetched), good_entries());
    assert_eq!(fetched.policy, POLICY);
}

#[tokio::test]
async fn a_file_for_two_recipients_opens_with_either_key() {
    for file in ["test-identity.txt", "other-identity.txt"] {
        let fetched = open("two-recipients.enc.json", &age(file)).await.unwrap();
        assert_eq!(entries(&fetched), good_entries(), "{file}");
        assert_eq!(fetched.policy, POLICY);
    }
}

#[tokio::test]
async fn a_file_that_encrypts_by_regex_hashes_its_plain_numbers_and_bools_too() {
    // Only `env` and `policy` are encrypted, so `extra`'s ints, floats, bools, empty text and
    // null stay plain and are hashed as SOPS hashes them.
    let text = fixture_text("encrypted-regex.enc.json");
    assert!(text.contains("\"encrypted_regex\": \"^(env|policy)$\""));
    assert!(text.contains("\"weight\": 0.25"));
    let fetched = open("encrypted-regex.enc.json", &mine()).await.unwrap();
    assert_eq!(entries(&fetched), good_entries());
    assert_eq!(fetched.policy, POLICY);
}

#[tokio::test]
async fn one_key_group_opens_like_the_top_level_age_list() {
    let text = fixture_text("good.enc.json");
    assert!(text.contains("\n\t\t],\n\t\t\"lastmodified\""));
    let changed = text
        .replace(
            "\"sops\": {\n\t\t\"age\": [",
            "\"sops\": {\n\t\t\"key_groups\": [{\"age\": [",
        )
        .replace(
            "\n\t\t],\n\t\t\"lastmodified\"",
            "\n\t\t]}],\n\t\t\"lastmodified\"",
        );
    let dir = tempfile::tempdir().unwrap();
    let fetched = local(&write(&dir, &changed)).fetch(&mine()).await.unwrap();
    assert_eq!(entries(&fetched), good_entries());

    // Two groups would split the key, which Dots does not read.
    let two = changed.replace("\"key_groups\": [{", "\"key_groups\": [{\"age\": []}, {");
    let e = local(&write(&dir, &two)).fetch(&mine()).await.unwrap_err();
    assert_eq!(
        invalid(e),
        "the file has 2 key groups, which is not supported"
    );
}

#[tokio::test]
async fn names_that_are_not_variables_and_the_policy_key_are_skipped() {
    let fetched = open("names.enc.json", &mine()).await.unwrap();
    assert_eq!(entries(&fetched), [("API_KEY", API_KEY)]);
    assert_eq!(fetched.policy, POLICY);
}

#[tokio::test]
async fn source_for_gives_the_sops_source() {
    let settings = SourceSettings::Sops(SopsSettings {
        location: fixture("good.enc.json").display().to_string(),
    });
    let fetched = source_for(&settings, http_client())
        .fetch(&mine())
        .await
        .unwrap();
    assert_eq!(entries(&fetched), good_entries());
}

// Files that are refused as changed, or not for this PC.

#[tokio::test]
async fn a_file_with_two_values_swapped_in_a_list_fails_the_mac() {
    // Both values have the AAD `extra:replicas:`, so each decrypts; only the MAC sees it.
    let e = open("tampered.enc.json", &mine()).await.unwrap_err();
    assert_eq!(denied(e), "the file's MAC does not match");
}

#[tokio::test]
async fn a_file_whose_unencrypted_text_was_edited_fails_the_mac() {
    let e = open("unencrypted-changed.enc.json", &mine())
        .await
        .unwrap_err();
    assert_eq!(denied(e), "the file's MAC does not match");
}

#[tokio::test]
async fn a_value_moved_to_another_name_does_not_decrypt() {
    let api_key = encrypted("API_KEY");
    let e = open_changed(
        &format!("\"API_KEY\": \"{api_key}\""),
        &format!("\"API_KEY\": \"{}\"", encrypted("LOG_LEVEL")),
    )
    .await
    .unwrap_err();
    assert_eq!(
        denied(e),
        "the file's value at env:API_KEY does not decrypt: the file was changed"
    );
}

#[tokio::test]
async fn a_file_with_no_mac_or_a_damaged_one_is_denied() {
    let text = fixture_text("good.enc.json");
    let file: Value = serde_json::from_str(&text).unwrap();
    let mac = file["sops"]["mac"].as_str().unwrap();

    let e = open_changed(&format!("\t\t\"mac\": \"{mac}\",\n"), "")
        .await
        .unwrap_err();
    assert_eq!(denied(e), "the file's MAC does not match");

    // The MAC of another file, made with another data key.
    let other: Value = serde_json::from_str(&fixture_text("mac-only.enc.json")).unwrap();
    let other_mac = other["sops"]["mac"].as_str().unwrap();
    let e = open_changed(mac, other_mac).await.unwrap_err();
    assert_eq!(denied(e), "the file's MAC does not match");

    // The right MAC, with a changed `lastmodified`, its AAD.
    let modified = file["sops"]["lastmodified"].as_str().unwrap();
    let e = open_changed(modified, "2020-01-01T00:00:00Z")
        .await
        .unwrap_err();
    assert_eq!(denied(e), "the file's MAC does not match");
}

#[tokio::test]
async fn a_key_that_is_not_a_recipient_is_denied() {
    let e = open("not-recipient.enc.json", &mine()).await.unwrap_err();
    assert_eq!(denied(e), "this PC's key is not a recipient of the file");

    let e = open("good.enc.json", &age("other-identity.txt"))
        .await
        .unwrap_err();
    assert_eq!(denied(e), "this PC's key is not a recipient of the file");

    // A file with no age entries at all, such as one for PGP keys only.
    let e = open_changed("\"sops\": {\n\t\t\"age\": [", "\"sops\": {\n\t\t\"pgp\": [")
        .await
        .unwrap_err();
    assert_eq!(denied(e), "this PC's key is not a recipient of the file");
}

// Files Dots cannot use.

#[tokio::test]
async fn a_value_that_is_not_text_is_refused_by_name() {
    let e = open("not-text.enc.json", &mine()).await.unwrap_err();
    assert_eq!(invalid(e), "RETRIES is not text");
}

#[tokio::test]
async fn two_names_in_a_different_case_are_refused() {
    let e = open("case-clash.enc.json", &mine()).await.unwrap_err();
    assert_eq!(
        invalid(e),
        "API_KEY and Api_Key are the same name in a different case"
    );
}

#[tokio::test]
async fn a_file_with_no_policy_is_refused() {
    let e = open("no-policy.enc.json", &mine()).await.unwrap_err();
    assert_eq!(invalid(e), "the file has no policy");
}

#[tokio::test]
async fn a_file_that_is_not_sops_is_refused() {
    let e = open("good.plain.json", &mine()).await.unwrap_err();
    assert_eq!(
        invalid(e),
        "the file is not a SOPS file: it has no sops metadata"
    );

    let dir = tempfile::tempdir().unwrap();
    let path = write(&dir, "[1, 2]");
    let e = local(&path).fetch(&mine()).await.unwrap_err();
    assert_eq!(
        invalid(e),
        "the file is not a SOPS file: it has no sops metadata"
    );

    let path = write(&dir, "API_KEY=fixture-api-key-9d27b3\n");
    let e = local(&path).fetch(&mine()).await.unwrap_err();
    assert_eq!(
        invalid(e),
        format!(
            "{} is not readable as JSON (line 1, column 1)",
            path.display()
        )
    );
}

#[tokio::test]
async fn a_value_that_is_not_sops_syntax_is_refused() {
    let api_key = encrypted("API_KEY");
    for bad in [
        "ENC[AES256_GCM,data:zz".to_string(),
        api_key.replace("type:str]", "type:str"),
        API_KEY.to_string(),
    ] {
        let e = open_changed(&api_key, &bad).await.unwrap_err();
        assert_eq!(
            invalid(e),
            "the file's value at env:API_KEY is not a SOPS value"
        );
    }
    let e = open_changed(&format!("\"{api_key}\""), "12")
        .await
        .unwrap_err();
    assert_eq!(
        invalid(e),
        "the file's value at env:API_KEY is not encrypted"
    );
}

#[tokio::test]
async fn the_same_key_twice_in_an_object_is_refused() {
    let e = open_changed(
        "\"env\": {\n",
        &format!(
            "\"env\": {{\n\t\t\"API_KEY\": \"{}\",\n",
            encrypted("API_KEY")
        ),
    )
    .await
    .unwrap_err();
    assert_eq!(
        invalid(e),
        "the file has the key \"API_KEY\" twice in one object"
    );
}

#[tokio::test]
async fn a_key_split_with_shamir_is_not_supported() {
    let e = open_changed(
        "\t\t\"lastmodified\":",
        "\t\t\"shamir_threshold\": 2,\n\t\t\"lastmodified\":",
    )
    .await
    .unwrap_err();
    assert_eq!(
        invalid(e),
        "the file splits its data key with shamir_threshold 2, which is not supported"
    );

    // A threshold of 1 is what SOPS writes for one group: nothing is split.
    let fetched = open_changed(
        "\t\t\"lastmodified\":",
        "\t\t\"shamir_threshold\": 1,\n\t\t\"lastmodified\":",
    )
    .await
    .unwrap();
    assert_eq!(entries(&fetched), good_entries());
}

#[tokio::test]
async fn more_than_one_rule_or_a_comment_rule_is_refused() {
    let rule = "\t\t\"unencrypted_suffix\": \"_unencrypted\",\n";
    let e = open_changed(
        rule,
        &format!("{rule}\t\t\"encrypted_suffix\": \"_secret\",\n"),
    )
    .await
    .unwrap_err();
    assert_eq!(
        invalid(e),
        "the file's sops metadata sets more than one of unencrypted_suffix, encrypted_suffix, \
         unencrypted_regex, encrypted_regex, unencrypted_comment_regex, encrypted_comment_regex"
    );

    let e = open_changed(rule, "\t\t\"encrypted_comment_regex\": \"sops:enc\",\n")
        .await
        .unwrap_err();
    assert_eq!(
        invalid(e),
        "the file's encrypted_comment_regex is not supported: a JSON file has no comments"
    );

    let e = open_changed(rule, "\t\t\"encrypted_regex\": \"(\",\n")
        .await
        .unwrap_err();
    assert_eq!(
        invalid(e),
        "the file's sops metadata is damaged: encrypted_regex is not a regular expression"
    );
}

#[tokio::test]
async fn with_no_rule_set_the_default_suffix_applies() {
    // Without the rule, `note_unencrypted` is still read as plain, as SOPS reads it.
    let fetched = open_changed("\t\t\"unencrypted_suffix\": \"_unencrypted\",\n", "")
        .await
        .unwrap();
    assert_eq!(entries(&fetched), good_entries());
}

#[tokio::test]
async fn a_damaged_last_modified_is_refused() {
    let file: Value = serde_json::from_str(&fixture_text("good.enc.json")).unwrap();
    let modified = file["sops"]["lastmodified"].as_str().unwrap();
    let e = open_changed(modified, "yesterday").await.unwrap_err();
    assert_eq!(
        invalid(e),
        "the file's sops metadata is damaged: lastmodified is not an RFC 3339 time"
    );
}

// Reading the file.

#[tokio::test]
async fn a_missing_file_is_unreachable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("not-synced-yet.sops.json");
    let why = unreachable(local(&path).fetch(&mine()).await.unwrap_err());
    assert!(
        why.starts_with(&format!("cannot read {}: ", path.display())),
        "{why}"
    );

    // A folder where the file should be cannot be read either.
    let why = unreachable(local(dir.path()).fetch(&mine()).await.unwrap_err());
    assert!(why.starts_with("cannot read "), "{why}");
}

#[tokio::test]
async fn a_file_of_1_mib_is_read_and_a_larger_one_is_refused() {
    let text = fixture_text("good.enc.json");
    let dir = tempfile::tempdir().unwrap();
    let full = format!("{text}{}", " ".repeat(MIB - text.len()));
    assert_eq!(full.len(), MIB);
    let fetched = local(&write(&dir, &full)).fetch(&mine()).await.unwrap();
    assert_eq!(entries(&fetched), good_entries());

    let path = write(&dir, &format!("{full} "));
    let e = local(&path).fetch(&mine()).await.unwrap_err();
    assert_eq!(
        invalid(e),
        format!("{} is larger than 1 MiB", path.display())
    );
}

#[tokio::test]
async fn the_wrong_kind_of_creds_or_a_bad_key_is_refused_before_reading() {
    // The location does not exist: a read would be Unreachable.
    let source = sops_at("C:/nowhere/org.sops.json");
    let token = SourceCreds::Token {
        token: Zeroizing::new("hvs.tok-4d9e1b7a".into()),
    };
    let e = source.fetch(&token).await.unwrap_err();
    assert_eq!(
        invalid(e),
        "this org's SOPS file is opened with this PC's age key, so it needs one"
    );

    for bad in [
        "",
        "AGE-SECRET-KEY-1NOTAKEY",
        "age1a03d7rmd3zc7lpcxldf24f2g2gyhgph",
    ] {
        let creds = SourceCreds::Age {
            identity: Zeroizing::new(bad.into()),
        };
        let e = source.fetch(&creds).await.unwrap_err();
        let text = format!("{e} {e:?}");
        assert!(bad.is_empty() || !text.contains(bad), "{text}");
        assert_eq!(
            invalid(e),
            "this PC's age key is not an age identity (AGE-SECRET-KEY-1…)"
        );
    }
}

// Reading the file over HTTP.

#[tokio::test]
async fn a_file_at_a_url_is_downloaded_and_opened() {
    let fake = FakeFiles::start().await;
    fake.put("/orgs/acme.sops.json", fixture_text("good.enc.json"));
    let fetched = sops_at(&fake.url("/orgs/acme.sops.json"))
        .fetch(&mine())
        .await
        .unwrap();
    assert_eq!(entries(&fetched), good_entries());
    assert_eq!(fetched.policy, POLICY);
    assert_eq!(fake.requests(), ["/orgs/acme.sops.json"]);
}

#[tokio::test]
async fn a_url_with_no_file_is_invalid_without_its_query() {
    let fake = FakeFiles::start().await;
    let source = sops_at(&fake.url("/orgs/gone.sops.json?sig=secret-sig-77"));
    let e = source.fetch(&mine()).await.unwrap_err();
    assert_eq!(
        invalid(e),
        format!(
            "there is no file at {} (HTTP 404)",
            fake.url("/orgs/gone.sops.json")
        )
    );
    assert_eq!(fake.requests(), ["/orgs/gone.sops.json?sig=secret-sig-77"]);
}

#[tokio::test]
async fn a_server_error_or_rate_limiting_is_unreachable_and_a_refusal_is_denied() {
    let fake = FakeFiles::start().await;
    fake.put("/org.sops.json", fixture_text("good.enc.json"));
    let source = sops_at(&fake.url("/org.sops.json"));
    let shown = fake.url("/org.sops.json");

    fake.fail_next(503, 1);
    let why = unreachable(source.fetch(&mine()).await.unwrap_err());
    assert_eq!(why, format!("{shown} is not available (HTTP 503)"));

    fake.fail_next(500, 1);
    let why = unreachable(source.fetch(&mine()).await.unwrap_err());
    assert_eq!(why, format!("{shown} is not available (HTTP 500)"));

    fake.fail_next(429, 1);
    let why = unreachable(source.fetch(&mine()).await.unwrap_err());
    assert_eq!(why, format!("{shown} is rate limiting Dots (HTTP 429)"));

    fake.fail_next(403, 1);
    let why = denied(source.fetch(&mine()).await.unwrap_err());
    assert_eq!(
        why,
        format!("{shown} refused access to the SOPS file (HTTP 403)")
    );

    // A redirect is not followed.
    fake.fail_next(302, 1);
    let why = invalid(source.fetch(&mine()).await.unwrap_err());
    assert_eq!(why, format!("{shown} answered HTTP 302 for the SOPS file"));

    // The file is there once the server is back.
    assert!(source.fetch(&mine()).await.is_ok());
}

#[tokio::test]
async fn a_refused_connection_is_unreachable() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let source = sops_at(&format!("http://127.0.0.1:{port}/org.sops.json"));
    let why = unreachable(source.fetch(&mine()).await.unwrap_err());
    assert!(
        why.starts_with(&format!(
            "cannot reach http://127.0.0.1:{port}/org.sops.json: "
        )),
        "{why}"
    );
}

#[tokio::test]
async fn a_downloaded_file_over_1_mib_or_not_json_is_invalid() {
    let fake = FakeFiles::start().await;
    let shown = fake.url("/org.sops.json");
    let source = sops_at(&shown);

    fake.put("/org.sops.json", " ".repeat(MIB + 1));
    let why = invalid(source.fetch(&mine()).await.unwrap_err());
    assert_eq!(
        why,
        format!("the answer from {shown} for the SOPS file is too large")
    );

    fake.put("/org.sops.json", "<html>sign in</html>");
    let why = invalid(source.fetch(&mine()).await.unwrap_err());
    assert!(
        why.starts_with(&format!(
            "the answer from {shown} for the SOPS file is not readable"
        )),
        "{why}"
    );
}

// Plain values outside the MAC.

/// `name` with `from` replaced by `to` (each once), opened.
async fn open_fixture_changed(
    name: &str,
    changes: &[(&str, &str)],
) -> Result<Fetched, SourceError> {
    let mut text = fixture_text(name);
    for (from, to) in changes {
        assert_eq!(text.matches(from).count(), 1, "{from:?}");
        text = text.replace(from, to);
    }
    let dir = tempfile::tempdir().unwrap();
    local(&write(&dir, &text)).fetch(&mine()).await
}

#[tokio::test]
async fn a_plain_env_entry_added_to_a_mac_only_file_is_refused() {
    // `mac_only_encrypted` leaves plain values out of the MAC, so this one changes nothing the
    // MAC covers; it still must not become an org secret.
    let e = open_fixture_changed(
        "mac-only.enc.json",
        &[(
            "\"env\": {\n",
            "\"env\": {\n\t\t\"EVIL_unencrypted\": \"x-evil-1\",\n",
        )],
    )
    .await
    .unwrap_err();
    let text = format!("{e} {e:?}");
    assert!(!text.contains("x-evil-1"), "{text}");
    assert_eq!(
        invalid(e),
        "the file's value at env:EVIL_unencrypted is not encrypted"
    );
}

#[tokio::test]
async fn a_rule_rewritten_to_leave_a_new_env_entry_plain_is_refused() {
    let e = open_fixture_changed(
        "mac-only.enc.json",
        &[
            (
                "\"unencrypted_suffix\": \"_unencrypted\"",
                "\"unencrypted_regex\": \"^(GITHUB_TOKEN|note_unencrypted)$\"",
            ),
            (
                "\"env\": {\n",
                "\"env\": {\n\t\t\"GITHUB_TOKEN\": \"x-evil-2\",\n",
            ),
        ],
    )
    .await
    .unwrap_err();
    let text = format!("{e} {e:?}");
    assert!(!text.contains("x-evil-2"), "{text}");
    assert_eq!(
        invalid(e),
        "the file's value at env:GITHUB_TOKEN is not encrypted"
    );
}

#[tokio::test]
async fn an_empty_env_value_is_skipped_and_never_an_entry() {
    // An empty text is never encrypted, so a file may carry one that its MAC does not cover.
    // What is logged for it is in `vault_org_sops_log_test.rs`.
    let fetched = open_fixture_changed(
        "mac-only.enc.json",
        &[(
            "\"env\": {\n",
            "\"env\": {\n\t\t\"EVIL_unencrypted\": \"\",\n",
        )],
    )
    .await
    .unwrap();
    assert_eq!(entries(&fetched), good_entries());
    assert!(!entries(&fetched)
        .iter()
        .any(|(name, _)| name.contains("EVIL")));
}

#[tokio::test]
async fn an_empty_env_value_added_to_the_good_file_is_skipped() {
    // The default rules encrypt every value, but never an empty text: the MAC still matches.
    let fetched = open_changed("\"env\": {\n", "\"env\": {\n\t\t\"EVIL\": \"\",\n")
        .await
        .unwrap();
    assert_eq!(entries(&fetched), good_entries());
    assert_eq!(fetched.policy, POLICY);
}

#[tokio::test]
async fn a_policy_left_plain_by_a_rewritten_rule_is_refused() {
    // The rule now leaves `policy` plain, and the plain text is the policy the MAC was made
    // over, so the MAC still matches: the policy must still be encrypted.
    let file: Value = serde_json::from_str(&fixture_text("good.enc.json")).unwrap();
    let sealed = file["policy"].as_str().unwrap().to_string();
    let plain = serde_json::to_string(POLICY).unwrap();
    let e = open_fixture_changed(
        "good.enc.json",
        &[
            (
                "\"unencrypted_suffix\": \"_unencrypted\"",
                "\"unencrypted_regex\": \"^(policy|note_unencrypted)$\"",
            ),
            (&format!("\"{sealed}\""), &plain),
        ],
    )
    .await
    .unwrap_err();
    assert_eq!(invalid(e), "the file's value at policy is not encrypted");
}

// This PC's identity for an org.

#[test]
fn the_test_identities_give_the_recipients_in_the_fixtures() {
    assert_eq!(
        public_key(&identity("test-identity.txt")).unwrap(),
        TEST_RECIPIENT
    );
    assert_eq!(
        public_key(&identity("other-identity.txt")).unwrap(),
        OTHER_RECIPIENT
    );
    // age-keygen wrote the same public key into each file's comment.
    let text = fixture_text("test-identity.txt");
    assert!(text.contains(&format!("# public key: {TEST_RECIPIENT}")));
    let good: Value = serde_json::from_str(&fixture_text("good.enc.json")).unwrap();
    assert_eq!(good["sops"]["age"][0]["recipient"], TEST_RECIPIENT);
}

#[test]
fn a_new_identity_matches_its_public_key() {
    let (secret, public) = new_identity();
    assert!(secret.starts_with("AGE-SECRET-KEY-1"), "an age identity");
    assert!(public.starts_with("age1"), "{public}");
    assert_eq!(public_key(&secret).unwrap(), public);
    assert_eq!(
        public_key(&format!("  {}\n", secret.as_str())).unwrap(),
        public
    );

    let (other, other_public) = new_identity();
    assert_ne!(other.as_str(), secret.as_str());
    assert_ne!(other_public, public);
}

#[test]
fn a_bad_identity_has_no_public_key() {
    for bad in [
        "",
        "age1a03d7rmd3zc7lpcxldf24f2g2gyhgph",
        "AGE-SECRET-KEY-1NOTAKEY",
    ] {
        match public_key(bad) {
            Err(Error::Invalid(why)) => {
                assert_eq!(
                    why,
                    "this PC's age key is not an age identity (AGE-SECRET-KEY-1…)"
                );
            }
            other => panic!("expected Invalid, got {other:?}"),
        }
    }
}

/// The real `age-keygen`, from `DOTS_TEST_AGE_KEYGEN` or the PATH, if it is installed.
fn age_keygen() -> Option<Command> {
    let program = std::env::var_os("DOTS_TEST_AGE_KEYGEN").unwrap_or("age-keygen".into());
    let found = Command::new(&program).arg("--version").output().is_ok();
    found.then(|| Command::new(program))
}

#[test]
fn the_age_cli_reads_a_new_identity() {
    let Some(mut keygen) = age_keygen() else {
        eprintln!("skipped: age-keygen is not installed (set DOTS_TEST_AGE_KEYGEN)");
        return;
    };
    let (secret, public) = new_identity();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("identity.txt");
    std::fs::write(&path, format!("{}\n", secret.as_str())).unwrap();
    // `-y` reads the identity file and prints its public key.
    let out = keygen.arg("-y").arg(&path).output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8(out.stdout).unwrap().trim(), public);
}
