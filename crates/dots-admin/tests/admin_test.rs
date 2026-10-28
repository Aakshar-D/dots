//! `dots-admin` (env vault spec §12, §13): keys, policies and join codes made here are what
//! `dots-core` accepts, and what it would refuse is refused here first.

use std::cell::RefCell;
use std::path::Path;
use std::process::Command;

use base64::Engine;
use chrono::{DateTime, Utc};
use dots_admin::keys::{keygen, read_public_key};
use dots_admin::policy::{new_policy, sign_policy, verify_signed};
use dots_admin::{run, AdminError, Ask, Io};
use dots_core::vault::org::{
    key_fingerprint, parse_join_code, parse_policy, verify_policy, BaoAuth, InfisicalSettings,
    JoinCode, OpenBaoSettings, PersonalMode, PolicyError, SopsSettings, SourceSettings, UseRules,
};
use serde_json::Value;
use tempfile::TempDir;

/// What one `run` wrote, and how it ended.
struct Ran {
    out: String,
    err: String,
    result: Result<(), AdminError>,
    asked: Vec<Ask>,
}

/// Runs a command line with `password` as the only password, recording what was asked.
fn run_with(args: &[&str], password: Option<&str>) -> Ran {
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let asked = RefCell::new(Vec::new());
    let mut source = |ask: Ask| {
        asked.borrow_mut().push(ask);
        password
            .map(str::to_string)
            .ok_or_else(|| AdminError::Failed("no password in this test".into()))
    };
    let result = run(
        &args,
        &mut Io {
            out: &mut out,
            err: &mut err,
            password: &mut source,
        },
    );
    Ran {
        out: String::from_utf8(out).unwrap(),
        err: String::from_utf8(err).unwrap(),
        result,
        asked: asked.into_inner(),
    }
}

/// Runs a command line that needs no password and must succeed.
fn ok(args: &[&str]) -> Ran {
    let ran = run_with(args, None);
    assert!(ran.result.is_ok(), "{args:?}: {:?}", ran.result);
    assert!(ran.asked.is_empty(), "{args:?} asked for a password");
    ran
}

/// The message of a command line that must fail.
fn failure(args: &[&str]) -> AdminError {
    let ran = run_with(args, None);
    ran.result.expect_err("the command should fail")
}

fn path(dir: &Path, name: &str) -> String {
    dir.join(name).to_str().unwrap().to_string()
}

fn no_password(_: Ask) -> dots_admin::Result<String> {
    panic!("an unprotected key needs no password")
}

fn at(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .unwrap()
        .with_timezone(&Utc)
}

/// A policy with the required fields and `extra`.
fn policy_with(extra: &str) -> String {
    format!(r#"{{"v": 1, "org_id": "acme-eng", "issued_at": "2026-10-21T09:00:00Z"{extra}}}"#)
}

/// An unprotected key pair in a new folder.
fn keys() -> (TempDir, String, String) {
    let dir = TempDir::new().unwrap();
    let keys = keygen(dir.path(), None).unwrap();
    let secret = std::fs::read_to_string(&keys.secret_path).unwrap();
    (dir, secret, keys.public_key)
}

#[test]
fn round_trip_from_keygen_to_a_join_code_that_dots_accepts() {
    let dir = TempDir::new().unwrap();
    let keygen = ok(&[
        "keygen",
        "--out",
        dir.path().to_str().unwrap(),
        "--no-password",
    ]);
    assert!(keygen
        .err
        .contains("warning: policy.key is not password-protected"));
    let public = std::fs::read_to_string(dir.path().join("policy.pub")).unwrap();
    let public_key = read_public_key(&public).unwrap();
    assert!(keygen.out.contains(&public_key));

    let policy_file = path(dir.path(), "policy.json");
    ok(&[
        "policy",
        "new",
        "--org-id",
        "acme-eng",
        "--out",
        &policy_file,
    ]);
    // The admin's edit: two fields, and an old issued_at that signing replaces.
    let edited: String = std::fs::read_to_string(&policy_file)
        .unwrap()
        .lines()
        .map(|line| match line {
            l if l.starts_with(r#"  "issued_at": "#) => {
                r#"  "issued_at": "2020-01-01T00:00:00Z","#.to_string()
            }
            l => l
                .replace(r#""plain": []"#, r#""plain": ["LOG_LEVEL"]"#)
                .replace(r#""personal": "deny""#, r#""personal": "fallback""#),
        })
        .map(|line| line + "\n")
        .collect();
    assert!(edited.contains("LOG_LEVEL") && edited.contains("fallback") && edited.contains("2020"));
    std::fs::write(&policy_file, &edited).unwrap();

    let before = Utc::now();
    let signed_file = path(dir.path(), "signed.json");
    let key_file = path(dir.path(), "policy.key");
    let sign = ok(&[
        "policy",
        "sign",
        &policy_file,
        "--key",
        &key_file,
        "--out",
        &signed_file,
    ]);
    assert!(sign.out.contains("store its text as DOTS_POLICY"));

    let signed = std::fs::read_to_string(&signed_file).unwrap();
    let policy = verify_policy(&signed, &public_key, "acme-eng").unwrap();
    assert_eq!(policy.plain, ["LOG_LEVEL"]);
    assert_eq!(policy.personal, PersonalMode::Fallback);
    let issued = at(&policy.issued_at);
    assert!(issued >= before - chrono::Duration::seconds(1) && issued <= Utc::now());
    // The policy file itself is left as the admin wrote it.
    assert_eq!(std::fs::read_to_string(&policy_file).unwrap(), edited);

    let pub_file = path(dir.path(), "policy.pub");
    let verify = ok(&["policy", "verify", &signed_file, "--pub", &pub_file]);
    assert!(verify
        .out
        .starts_with("ok: a Dots client in org 'acme-eng' accepts this policy"));
    assert!(verify
        .out
        .contains("plain entries, shown with their values: LOG_LEVEL"));
    let verify = ok(&[
        "policy",
        "verify",
        &signed_file,
        "--pub",
        &pub_file,
        "--org-id",
        "acme-eng",
    ]);
    assert!(verify.out.starts_with("ok"));

    let join = ok(&[
        "join-code",
        "--org-id",
        "acme-eng",
        "--name",
        "Acme Engineering",
        "--source",
        "openbao",
        "--address",
        "https://bao.acme.internal",
        "--path",
        "dots",
        "--pub",
        &pub_file,
    ]);
    let code = parse_join_code(&join.out).unwrap();
    assert_eq!(code.policy_key, public_key);
    // What Dots does on joining: the fetched policy verifies against the join code.
    verify_policy(&signed, &code.policy_key, &code.org_id).unwrap();
}

#[test]
fn a_password_protected_key_signs_and_verifies() {
    let dir = TempDir::new().unwrap();
    let out = dir.path().to_str().unwrap();
    let keygen = run_with(&["keygen", "--out", out], Some("correct horse"));
    assert!(keygen.result.is_ok(), "{:?}", keygen.result);
    assert_eq!(keygen.asked, [Ask::NewKey]);
    assert!(
        keygen.err.is_empty(),
        "no warning for a protected key: {}",
        keygen.err
    );
    let secret = std::fs::read_to_string(dir.path().join("policy.key")).unwrap();
    assert!(!keygen.out.contains(secret.lines().nth(1).unwrap()));

    let policy_file = path(dir.path(), "policy.json");
    ok(&[
        "policy",
        "new",
        "--org-id",
        "acme-eng",
        "--out",
        &policy_file,
    ]);
    let key_file = path(dir.path(), "policy.key");
    let sign = run_with(
        &["policy", "sign", &policy_file, "--key", &key_file],
        Some("correct horse"),
    );
    assert!(sign.result.is_ok(), "{:?}", sign.result);
    assert_eq!(sign.asked, [Ask::Unlock]);
    assert!(sign.err.contains("signed the policy for org 'acme-eng'"));

    let public = std::fs::read_to_string(dir.path().join("policy.pub")).unwrap();
    let policy = verify_signed(&sign.out, &public, Some("acme-eng")).unwrap();
    assert_eq!(policy.org_id, "acme-eng");

    let policy_text = std::fs::read_to_string(&policy_file).unwrap();
    let wrong = sign_policy(
        &policy_text,
        &secret,
        &mut |_| Ok("battery staple".into()),
        Utc::now(),
    );
    assert_eq!(
        wrong.unwrap_err(),
        AdminError::Failed("wrong password for the policy key".into())
    );
}

#[test]
fn an_empty_password_is_refused() {
    let dir = TempDir::new().unwrap();
    let ran = run_with(&["keygen", "--out", dir.path().to_str().unwrap()], Some(""));
    let message = ran.result.unwrap_err().to_string();
    assert!(
        message.contains("an empty password does not protect the key"),
        "{message}"
    );
    assert!(!dir.path().join("policy.key").exists());
}

#[test]
fn policy_new_has_every_field_at_its_default() {
    let text = new_policy("acme-eng", at("2026-10-23T08:30:00Z")).unwrap();
    let policy = parse_policy(&text).unwrap();
    assert_eq!(policy.org_id, "acme-eng");
    assert_eq!(policy.issued_at, "2026-10-23T08:30:00Z");
    assert!(policy.plain.is_empty());
    assert_eq!(policy.use_rules, UseRules::default());
    assert_eq!(policy.personal, PersonalMode::default());
    assert!(!policy.shell_secrets);
    assert_eq!(policy.refresh_minutes, 15);
    assert_eq!(policy.offline_hours, 24);
    assert_eq!(policy.audit, None);
    // Every field is spelled out, so the admin sees what can be set.
    let value: Value = serde_json::from_str(&text).unwrap();
    let mut fields: Vec<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    fields.sort_unstable();
    let mut expected = [
        "v",
        "org_id",
        "issued_at",
        "plain",
        "use",
        "personal",
        "shell_secrets",
        "refresh_minutes",
        "offline_hours",
        "audit",
    ];
    expected.sort_unstable();
    assert_eq!(fields, expected);
    assert!(
        text.starts_with("{\n  \"v\": 1,\n  \"org_id\": \"acme-eng\","),
        "{text}"
    );

    let stdout = ok(&["policy", "new", "--org-id", "acme-eng"]);
    parse_policy(&stdout.out).unwrap();
}

#[test]
fn policy_new_refuses_a_bad_org_id_and_an_existing_file() {
    let message = failure(&["policy", "new", "--org-id", "Acme Eng"]).to_string();
    assert!(
        message.starts_with("org id 'Acme Eng': use 1 to 63 characters"),
        "{message}"
    );

    let dir = TempDir::new().unwrap();
    let file = path(dir.path(), "policy.json");
    std::fs::write(&file, "my edits").unwrap();
    let message = failure(&["policy", "new", "--org-id", "acme-eng", "--out", &file]);
    assert!(message.to_string().contains("already exists"), "{message}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "my edits");
}

#[test]
fn policy_check_says_ok_with_a_summary() {
    let dir = TempDir::new().unwrap();
    let file = path(dir.path(), "policy.json");
    std::fs::write(
        &file,
        policy_with(
            r#", "plain": ["LOG_LEVEL", "API_BASE_URL"],
            "use": {"allow": ["*"], "deny": ["PROD_*"]}, "personal": "override",
            "shell_secrets": true, "refresh_minutes": 1,
            "offline_hours": 0, "audit": {"endpoint": "https://audit.acme.internal/dots",
            "token_entry": "DOTS_AUDIT_TOKEN"}"#,
        ),
    )
    .unwrap();
    let check = ok(&["policy", "check", &file]);
    let expected = "\
ok
  org: acme-eng
  issued at: 2026-10-21T09:00:00Z
  plain entries, shown with their values: LOG_LEVEL, API_BASE_URL; every other org entry is a secret
  dots may be granted: every org entry except names matching PROD_*
  personal entries: win over the org's entry of the same name
  secrets in the shell: allowed
  refresh: every 1 minute
  offline copy: none; offline, the org's entries are not available
  audit: records go to https://audit.acme.internal/dots, with the token in entry DOTS_AUDIT_TOKEN
";
    assert_eq!(check.out, expected);
}

#[test]
fn policy_check_refuses_an_unknown_field_and_a_zero_refresh() {
    let dir = TempDir::new().unwrap();
    let file = path(dir.path(), "policy.json");
    for (extra, message) in [
        (r#", "foo": 1"#, "policy invalid: unknown field 'foo'"),
        (
            r#", "refresh_minutes": 0"#,
            "policy invalid: refresh_minutes must be at least 1",
        ),
    ] {
        std::fs::write(&file, policy_with(extra)).unwrap();
        let error = failure(&["policy", "check", &file]);
        assert_eq!(error, AdminError::Failed(message.into()));
        assert_eq!(error.exit_code(), 1);
    }
}

#[test]
fn policy_sign_refuses_an_invalid_policy() {
    let (dir, _, _) = keys();
    let policy_file = path(dir.path(), "policy.json");
    let signed_file = path(dir.path(), "signed.json");
    std::fs::write(&policy_file, policy_with(r#", "refresh_minutes": 0"#)).unwrap();
    let key_file = path(dir.path(), "policy.key");
    let ran = run_with(
        &[
            "policy",
            "sign",
            &policy_file,
            "--key",
            &key_file,
            "--out",
            &signed_file,
        ],
        Some("unused"),
    );
    assert_eq!(
        ran.result,
        Err(AdminError::Failed(
            "policy invalid: refresh_minutes must be at least 1".into()
        ))
    );
    assert!(
        ran.asked.is_empty(),
        "no password is asked for a policy that is refused"
    );
    assert!(!Path::new(&signed_file).exists());
}

#[test]
fn policy_sign_rewrites_only_issued_at() {
    let (_dir, secret, public_key) = keys();
    // Odd layout, an escaped key and look-alikes that are not the top-level issued_at.
    let text = concat!(
        "{ \"use\" : {\"allow\":[\"issued_at\"]},\n",
        "\t\"issued\\u005fat\":\"2020-01-01T00:00:00Z\" ,",
        "\"v\":1, \"org_id\":\"acme-eng\",\"plain\" : [ \"ISSUED_AT\" ] }",
    );
    let signed = sign_policy(text, &secret, &mut no_password, at("2026-10-23T10:00:00Z")).unwrap();
    let envelope: Value = serde_json::from_str(&signed.text).unwrap();
    let expected = text.replace(
        "\"issued\\u005fat\":\"2020-01-01T00:00:00Z\"",
        "\"issued\\u005fat\":\"2026-10-23T10:00:00Z\"",
    );
    assert_eq!(envelope["policy"], expected);
    assert_eq!(signed.policy.issued_at, "2026-10-23T10:00:00Z");
    let signature = envelope["signature"].as_str().unwrap();
    assert!(signature.contains("trusted comment: dots policy acme-eng 2026-10-23T10:00:00Z\n"));
    verify_policy(&signed.text, &public_key, "acme-eng").unwrap();
}

#[test]
fn a_signature_by_another_key_fails_verify() {
    let (dir, secret, _) = keys();
    let (_other_dir, _, other_public) = keys();
    let policy = new_policy("acme-eng", Utc::now()).unwrap();
    let signed = sign_policy(&policy, &secret, &mut no_password, Utc::now()).unwrap();
    let signed_file = path(dir.path(), "signed.json");
    std::fs::write(&signed_file, &signed.text).unwrap();
    let other_pub = path(dir.path(), "other.pub");
    std::fs::write(&other_pub, &other_public).unwrap();

    let error = failure(&["policy", "verify", &signed_file, "--pub", &other_pub]);
    assert_eq!(
        error,
        AdminError::Failed("a Dots client would refuse it: signature does not match".into())
    );
    assert_eq!(error.exit_code(), 1);
    assert_eq!(
        verify_policy(&signed.text, &other_public, "acme-eng").unwrap_err(),
        PolicyError::Signature
    );
}

#[test]
fn policy_verify_checks_the_org() {
    let (dir, secret, public_key) = keys();
    let policy = new_policy("acme-eng", Utc::now()).unwrap();
    let signed = sign_policy(&policy, &secret, &mut no_password, Utc::now()).unwrap();
    let error = verify_signed(&signed.text, &public_key, Some("other-org")).unwrap_err();
    assert_eq!(
        error,
        AdminError::Failed(
            "a Dots client would refuse it: for org 'acme-eng', not 'other-org'".into()
        )
    );
    let error = verify_signed("not signed", &public_key, None)
        .unwrap_err()
        .to_string();
    assert!(error.starts_with("a Dots client would refuse it: the signed policy is not readable"));
    let not_a_key = verify_signed(&signed.text, "RWnope", None)
        .unwrap_err()
        .to_string();
    assert!(
        not_a_key.contains("is not a minisign public key"),
        "{not_a_key}"
    );
    drop(dir);
}

#[test]
fn join_code_for_each_source_kind_reads_back_in_dots() {
    let (dir, _, public_key) = keys();
    let pub_file = path(dir.path(), "policy.pub");
    let common = [
        "--org-id",
        "acme-eng",
        "--name",
        "Acme Engineering",
        "--pub",
        &pub_file,
    ];
    let cases: Vec<(Vec<&str>, SourceSettings)> = vec![
        (
            vec![
                "--source",
                "openbao",
                "--address",
                "https://bao.acme.internal",
                "--path",
                "dots",
            ],
            SourceSettings::OpenBao(OpenBaoSettings {
                address: "https://bao.acme.internal".into(),
                mount: "secret".into(),
                path: "dots".into(),
                auth: BaoAuth::Token,
                auth_mount: None,
                namespace: None,
            }),
        ),
        (
            vec![
                "--source",
                "openbao",
                "--address",
                "https://vault.acme.internal:8200",
                "--mount",
                "kv",
                "--path",
                "teams/eng",
                "--auth",
                "ldap",
                "--auth-mount",
                "corp-ldap",
                "--namespace",
                "eng",
            ],
            SourceSettings::OpenBao(OpenBaoSettings {
                address: "https://vault.acme.internal:8200".into(),
                mount: "kv".into(),
                path: "teams/eng".into(),
                auth: BaoAuth::Ldap,
                auth_mount: Some("corp-ldap".into()),
                namespace: Some("eng".into()),
            }),
        ),
        (
            vec![
                "--source",
                "infisical",
                "--site-url",
                "https://app.infisical.com",
                "--project-id",
                "p-123",
                "--environment",
                "prod",
                "--secret-path",
                "/dots",
            ],
            SourceSettings::Infisical(InfisicalSettings {
                site_url: "https://app.infisical.com".into(),
                project_id: "p-123".into(),
                environment: "prod".into(),
                secret_path: "/dots".into(),
            }),
        ),
        (
            vec![
                "--source",
                "infisical",
                "--site-url",
                "https://infisical.acme.internal",
                "--project-id",
                "p-9",
                "--environment",
                "dev",
            ],
            SourceSettings::Infisical(InfisicalSettings {
                site_url: "https://infisical.acme.internal".into(),
                project_id: "p-9".into(),
                environment: "dev".into(),
                secret_path: "/".into(),
            }),
        ),
        (
            vec![
                "--source",
                "sops",
                "--location",
                "https://files.acme.internal/eng.sops.json",
            ],
            SourceSettings::Sops(SopsSettings {
                location: "https://files.acme.internal/eng.sops.json".into(),
            }),
        ),
    ];
    for (i, (source_args, source)) in cases.into_iter().enumerate() {
        let expected = JoinCode {
            v: 1,
            org_id: "acme-eng".into(),
            name: "Acme Engineering".into(),
            source,
            policy_key: public_key.clone(),
        };
        let mut args = vec!["join-code"];
        args.extend(common);
        args.extend(&source_args);
        let printed = ok(&args);
        assert_eq!(
            parse_join_code(&printed.out).unwrap(),
            expected,
            "{source_args:?}"
        );
        assert_eq!(
            printed.out.lines().count(),
            1,
            "only the code goes to stdout"
        );
        assert!(
            printed.err.contains("policy key fingerprint: "),
            "{}",
            printed.err
        );

        let file = path(dir.path(), &format!("org{i}.dotsorg"));
        args.extend(["--file", &file]);
        let written = ok(&args);
        assert!(written.out.starts_with(&format!("wrote {file}")));
        let text = std::fs::read_to_string(&file).unwrap();
        assert_eq!(parse_join_code(&text).unwrap(), expected, "{source_args:?}");
    }
}

#[test]
fn join_code_on_a_local_sops_file() {
    let (dir, _, _) = keys();
    let pub_file = path(dir.path(), "policy.pub");
    let location = path(dir.path(), "eng.sops.json");
    let printed = ok(&[
        "join-code",
        "--org-id",
        "acme-eng",
        "--name",
        "Acme",
        "--source",
        "sops",
        "--location",
        &location,
        "--pub",
        &pub_file,
    ]);
    let code = parse_join_code(&printed.out).unwrap();
    assert_eq!(code.source, SourceSettings::Sops(SopsSettings { location }));
}

#[test]
fn join_code_refuses_what_dots_would_refuse() {
    let (dir, _, _) = keys();
    let pub_file = path(dir.path(), "policy.pub");
    let key_file = path(dir.path(), "policy.key");
    let base = ["join-code", "--org-id", "acme-eng", "--name", "Acme"];
    let cases: [(&[&str], &str); 8] = [
        (
            &[
                "--source",
                "openbao",
                "--site-url",
                "https://x.example",
                "--pub",
                &pub_file,
            ],
            "--site-url is for --source infisical",
        ),
        (
            &[
                "--source",
                "openbao",
                "--address",
                "https://x.example",
                "--pub",
                &pub_file,
            ],
            "--source openbao needs --path",
        ),
        (
            &["--source", "dropbox", "--pub", &pub_file],
            "--source must be openbao, infisical or sops, not 'dropbox'",
        ),
        (
            &[
                "--source",
                "openbao",
                "--address",
                "https://x.example",
                "--path",
                "d",
                "--auth",
                "kerberos",
                "--pub",
                &pub_file,
            ],
            "--auth must be token, userpass or ldap, not 'kerberos'",
        ),
        (
            &[
                "--source",
                "openbao",
                "--address",
                "https://x.example",
                "--path",
                "d",
                "--auth-mount",
                "corp",
                "--pub",
                &pub_file,
            ],
            "--auth-mount is for --auth userpass or ldap",
        ),
        (
            &[
                "--source",
                "openbao",
                "--address",
                "http://x.example",
                "--path",
                "d",
                "--pub",
                &pub_file,
            ],
            "join code: the source's address 'http://x.example' must start with https://",
        ),
        (
            &[
                "--source",
                "sops",
                "--location",
                "relative/eng.json",
                "--pub",
                &pub_file,
            ],
            "join code: the source's location must be a full path",
        ),
        (
            &[
                "--source",
                "sops",
                "--location",
                "https://x.example/a.json",
                "--pub",
                &key_file,
            ],
            "the --pub file is not a minisign public key",
        ),
    ];
    for (extra, message) in cases {
        let mut args = base.to_vec();
        args.extend(extra);
        let error = failure(&args).to_string();
        assert!(error.starts_with(message), "{args:?}: {error}");
    }
    let bad_org = failure(&[
        "join-code",
        "--org-id",
        "-acme",
        "--name",
        "Acme",
        "--source",
        "sops",
        "--location",
        "https://x.example/a.json",
        "--pub",
        &pub_file,
    ]);
    assert!(
        bad_org.to_string().starts_with("org id '-acme'"),
        "{bad_org}"
    );
}

#[test]
fn keygen_refuses_to_overwrite_existing_files() {
    for existing in ["policy.key", "policy.pub"] {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join(existing), "old").unwrap();
        let ran = run_with(
            &["keygen", "--out", dir.path().to_str().unwrap()],
            Some("pw"),
        );
        let message = ran.result.unwrap_err().to_string();
        assert!(
            message.contains(&format!("{existing} already exists")),
            "{message}"
        );
        assert!(
            ran.asked.is_empty(),
            "no password is asked for a key that won't be written"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join(existing)).unwrap(),
            "old"
        );
        let other = if existing == "policy.key" {
            "policy.pub"
        } else {
            "policy.key"
        };
        assert!(!dir.path().join(other).exists());
        assert!(keygen(dir.path(), None).is_err());
    }
}

#[test]
fn keygen_writes_a_minisign_key_pair() {
    let dir = TempDir::new().unwrap();
    let keys = keygen(&dir.path().join("new-folder"), None).unwrap();
    let public = std::fs::read_to_string(&keys.public_path).unwrap();
    // minisign's own comment names the key id, which is not the fingerprint Dots shows.
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&keys.public_key)
        .unwrap();
    let key_id = u64::from_le_bytes(bytes[2..10].try_into().unwrap());
    assert_eq!(
        public,
        format!(
            "untrusted comment: minisign public key: {key_id:016X}\n{}\n",
            keys.public_key
        )
    );
    assert_eq!(keys.fingerprint, key_fingerprint(&keys.public_key).unwrap());
    assert_eq!(keys.fingerprint.len(), 39, "{}", keys.fingerprint);
    assert!(!keys.fingerprint.contains(&format!("{key_id:016X}")));
    let secret = std::fs::read_to_string(&keys.secret_path).unwrap();
    assert!(secret.starts_with("untrusted comment: dots-admin policy secret key"));
    assert_eq!(read_public_key(&keys.public_key).unwrap(), keys.public_key);
}

#[test]
fn a_wrong_command_line_is_a_usage_error() {
    for args in [
        &[][..],
        &["frobnicate"],
        &["policy"],
        &["policy", "sign", "p.json"],
        &["policy", "check"],
        &["policy", "check", "a.json", "b.json"],
        &["keygen", "--out", "x", "--colour", "red"],
        &["keygen", "--out", "x", "--out", "y"],
        &["keygen", "--out"],
    ] {
        let error = failure(args);
        assert!(matches!(error, AdminError::Usage(_)), "{args:?}: {error:?}");
        assert_eq!(error.exit_code(), 2);
    }
    let help = ok(&["help"]);
    assert!(help
        .out
        .contains("dots-admin policy sign <policy.json> --key <policy.key>"));
}

#[test]
fn the_binary_exits_non_zero_on_a_bad_policy() {
    let dir = TempDir::new().unwrap();
    let file = path(dir.path(), "policy.json");
    std::fs::write(&file, policy_with(r#", "foo": 1"#)).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_dots-admin"))
        .args(["policy", "check", &file])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(
        stderr.trim_end(),
        "dots-admin: policy invalid: unknown field 'foo'"
    );
}

// ---- member list (roster) and policy admins -------------------------------------------

/// A fresh Dots ID: the public key of a new unprotected pair.
fn dots_id() -> String {
    keys().2
}

/// A new roster for `acme-eng` in `dir`, owned by `owner@acme.test` with `owner_key`.
fn new_roster(dir: &Path, owner_key: &str) -> String {
    let file = path(dir, "roster.json");
    ok(&[
        "roster",
        "new",
        "--org-id",
        "acme-eng",
        "--owner-name",
        "Olive Owner",
        "--owner-email",
        "owner@acme.test",
        "--owner-key",
        owner_key,
        "--out",
        &file,
    ]);
    file
}

fn read_json(file: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap()
}

#[test]
fn roster_new_add_sign_verify_round_trip() {
    let (dir, secret, public) = keys();
    let owner_key = dots_id();
    let file = new_roster(dir.path(), &owner_key);
    let new = read_json(&file);
    assert_eq!(new["seq"], 0);
    assert_eq!(new["workspaces"][0]["id"], "default");
    assert_eq!(new["members"][0]["role"], "owner");
    assert_eq!(new["members"][0]["workspaces"]["default"], "admin");
    assert!(std::fs::read_to_string(&file).unwrap().ends_with("}\n"));

    let bob_key = dots_id();
    let added = ok(&[
        "roster",
        "add",
        &file,
        "--name",
        "Bob",
        "--email",
        "bob@acme.test",
        "--key",
        &bob_key,
        "--role",
        "member",
    ]);
    assert!(
        added.out.contains(&key_fingerprint(&bob_key).unwrap()),
        "{}",
        added.out
    );
    ok(&["roster", "check", &file, "--org-id", "acme-eng"]);

    let key_file = path(dir.path(), "root.key");
    std::fs::write(&key_file, secret).unwrap();
    let signed_file = path(dir.path(), "signed.json");
    run_with(
        &[
            "roster",
            "sign",
            &file,
            "--key",
            &key_file,
            "--out",
            &signed_file,
        ],
        None,
    )
    .result
    .unwrap();
    let pub_file = path(dir.path(), "policy.pub");
    std::fs::write(&pub_file, &public).unwrap();
    let verified = ok(&[
        "roster",
        "verify",
        &signed_file,
        "--pub",
        &pub_file,
        "--org-id",
        "acme-eng",
    ]);
    assert!(verified.out.starts_with("ok"), "{}", verified.out);
    // The signed text is what Dots reads.
    let text = std::fs::read_to_string(&signed_file).unwrap();
    let accepted =
        dots_core::accounts::roster::verify_roster(text.trim(), &public, &[], "acme-eng").unwrap();
    assert_eq!(accepted.roster.seq, 1);
    assert_eq!(accepted.roster.members.len(), 2);

    // Signed by another key: refused, unless that key is named as an admin.
    let (_d2, other_secret, other_public) = keys();
    let other_key = path(dir.path(), "other.key");
    std::fs::write(&other_key, other_secret).unwrap();
    let by_admin = path(dir.path(), "by-admin.json");
    run_with(
        &[
            "roster", "sign", &file, "--key", &other_key, "--out", &by_admin,
        ],
        None,
    )
    .result
    .unwrap();
    let refused = failure(&[
        "roster", "verify", &by_admin, "--pub", &pub_file, "--org-id", "acme-eng",
    ]);
    assert!(
        refused
            .to_string()
            .contains("not signed by the owner or an admin"),
        "{refused}"
    );
    ok(&[
        "roster",
        "verify",
        &by_admin,
        "--pub",
        &pub_file,
        "--admin",
        &other_public,
        "--org-id",
        "acme-eng",
    ]);
}

#[test]
fn roster_sign_bumps_seq() {
    let (dir, secret, _) = keys();
    let file = new_roster(dir.path(), &dots_id());
    let key_file = path(dir.path(), "root.key");
    std::fs::write(&key_file, secret).unwrap();
    for expected in [1, 2] {
        let ran = run_with(&["roster", "sign", &file, "--key", &key_file], None);
        ran.result.unwrap();
        let envelope: Value = serde_json::from_str(ran.out.trim()).unwrap();
        let roster: Value = serde_json::from_str(envelope["roster"].as_str().unwrap()).unwrap();
        assert_eq!(roster["seq"], expected);
        assert_eq!(
            read_json(&file)["seq"],
            expected,
            "the file keeps the new seq"
        );
        assert!(ran.err.contains("DOTS_ROSTER"), "{}", ran.err);
    }
}

#[test]
fn add_same_email_adds_key() {
    let (dir, _, _) = keys();
    let file = new_roster(dir.path(), &dots_id());
    let second_pc = dots_id();
    ok(&[
        "roster",
        "add",
        &file,
        "--name",
        "Olive",
        "--email",
        "owner@acme.test",
        "--key",
        &second_pc,
        "--role",
        "owner",
    ]);
    let roster = read_json(&file);
    assert_eq!(roster["members"].as_array().unwrap().len(), 1);
    assert_eq!(roster["members"][0]["keys"].as_array().unwrap().len(), 2);
    assert_eq!(roster["members"][0]["keys"][1], second_pc.as_str());

    // A key that belongs to someone else is refused.
    let bob = dots_id();
    ok(&[
        "roster",
        "add",
        &file,
        "--name",
        "Bob",
        "--email",
        "bob@acme.test",
        "--key",
        &bob,
        "--role",
        "viewer",
        "--workspace",
        "default=viewer",
    ]);
    let err = failure(&[
        "roster",
        "add",
        &file,
        "--name",
        "Bob",
        "--email",
        "bob@acme.test",
        "--key",
        &second_pc,
        "--role",
        "viewer",
    ]);
    assert!(err.to_string().contains("two members"), "{err}");
    // Remove just one PC, then the member.
    ok(&[
        "roster",
        "remove",
        &file,
        "owner@acme.test",
        "--key",
        &second_pc,
    ]);
    assert_eq!(
        read_json(&file)["members"][0]["keys"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    ok(&["roster", "set-role", &file, "bob@acme.test", "admin"]);
    assert_eq!(read_json(&file)["members"][1]["role"], "admin");
    ok(&["roster", "remove", &file, "bob@acme.test"]);
    assert_eq!(read_json(&file)["members"].as_array().unwrap().len(), 1);
}

#[test]
fn remove_last_owner_refused() {
    let (dir, _, _) = keys();
    let file = new_roster(dir.path(), &dots_id());
    let before = std::fs::read_to_string(&file).unwrap();
    for args in [
        vec!["roster", "remove", &file, "owner@acme.test"],
        vec!["roster", "set-role", &file, "owner@acme.test", "admin"],
    ] {
        let err = failure(&args);
        assert!(
            err.to_string()
                .contains("the member list needs at least one Owner"),
            "{err}"
        );
    }
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        before,
        "nothing was written"
    );
}

#[test]
fn bad_dots_id_refused() {
    let (dir, _, _) = keys();
    let file = path(dir.path(), "roster.json");
    let err = failure(&[
        "roster",
        "new",
        "--org-id",
        "acme-eng",
        "--owner-name",
        "O",
        "--owner-email",
        "o@a.test",
        "--owner-key",
        "nope",
        "--out",
        &file,
    ]);
    assert!(err.to_string().contains("not a Dots ID"), "{err}");
    assert!(!Path::new(&file).exists());
    let file = new_roster(dir.path(), &dots_id());
    let err = failure(&[
        "roster", "add", &file, "--name", "B", "--email", "b@a.test", "--key", "nope", "--role",
        "member",
    ]);
    assert!(err.to_string().contains("not a Dots ID"), "{err}");
    let policy = path(dir.path(), "policy.json");
    ok(&[
        "policy", "new", "--org-id", "acme-eng", "--roster", "--out", &policy,
    ]);
    let err = failure(&["policy", "admins", "add", &policy, "nope"]);
    assert!(err.to_string().contains("not a Dots ID"), "{err}");
}

#[test]
fn policy_new_roster_is_v2_required() {
    let ran = ok(&["policy", "new", "--org-id", "acme-eng", "--roster"]);
    let policy: Value = serde_json::from_str(&ran.out).unwrap();
    assert_eq!(policy["v"], 2);
    assert_eq!(policy["roster"], "required");
    assert_eq!(policy["admins"], serde_json::json!([]));
    let parsed = parse_policy(&ran.out).unwrap();
    assert_eq!(parsed.org_id, "acme-eng");
}

#[test]
fn policy_new_default_stays_v1() {
    let ran = ok(&["policy", "new", "--org-id", "acme-eng"]);
    let policy: Value = serde_json::from_str(&ran.out).unwrap();
    assert_eq!(policy["v"], 1);
    assert!(policy.get("roster").is_none() && policy.get("admins").is_none());
}

#[test]
fn policy_admins_add_remove() {
    let (dir, _, _) = keys();
    let file = path(dir.path(), "policy.json");
    ok(&[
        "policy", "new", "--org-id", "acme-eng", "--roster", "--out", &file,
    ]);
    let (a, b) = (dots_id(), dots_id());
    let added = ok(&["policy", "admins", "add", &file, &a]);
    assert!(
        added.out.contains(&key_fingerprint(&a).unwrap()),
        "{}",
        added.out
    );
    ok(&["policy", "admins", "add", &file, &b]);
    let text = std::fs::read_to_string(&file).unwrap();
    assert_eq!(
        parse_policy(&text).unwrap().admins,
        vec![a.clone(), b.clone()]
    );
    assert!(
        text.contains("\"refresh_minutes\": 15"),
        "other lines are kept"
    );
    let again = failure(&["policy", "admins", "add", &file, &a]);
    assert!(again.to_string().contains("already"), "{again}");
    ok(&["policy", "admins", "remove", &file, &a]);
    let text = std::fs::read_to_string(&file).unwrap();
    assert_eq!(parse_policy(&text).unwrap().admins, vec![b.clone()]);
    ok(&["policy", "admins", "remove", &file, &b]);
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(parse_policy(&text).unwrap().admins.is_empty());
    let gone = failure(&["policy", "admins", "remove", &file, &b]);
    assert!(gone.to_string().contains("not an admin"), "{gone}");

    // A v1 policy has no admins to edit.
    let v1 = path(dir.path(), "v1.json");
    ok(&["policy", "new", "--org-id", "acme-eng", "--out", &v1]);
    let err = failure(&["policy", "admins", "add", &v1, &a]);
    assert!(err.to_string().contains("policy new --roster"), "{err}");
}
