//! An org's policy, the signed text that carries it and the join code (env vault spec §6.1,
//! §6.3): defaults, fail-closed parsing, minisign verification and the join code's checks.

use std::io::Cursor;

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use dots_core::vault::org::{
    check_org_id, envelope, key_fingerprint, parse_join_code, parse_policy, verify_policy,
    AuditTarget, BaoAuth, InfisicalSettings, JoinCode, OpenBaoSettings, OrgPolicy, PersonalMode,
    PolicyError, SopsSettings, SourceSettings, UseRules,
};
use dots_core::Error;
use minisign::KeyPair;
use serde_json::{json, Value};

/// The example policy of spec §6.3, as written there.
const SPEC_POLICY: &str = r#"{
  "v": 1,
  "org_id": "acme-eng",
  "issued_at": "2026-10-21T09:00:00Z",
  "plain": ["LOG_LEVEL", "API_BASE_URL"],
  "use": { "allow": ["*"], "deny": ["PROD_*"] },
  "personal": "fallback",
  "shell_secrets": false,
  "refresh_minutes": 15,
  "offline_hours": 24,
  "audit": { "endpoint": "https://audit.acme.internal/dots", "token_entry": "DOTS_AUDIT_TOKEN" }
}"#;

fn keys() -> KeyPair {
    KeyPair::generate_unencrypted_keypair().unwrap()
}

/// The `.minisig` text over `text`, as `minisign -S` or `dots-admin policy sign` writes it.
fn signature(keys: &KeyPair, text: &str) -> String {
    minisign::sign(Some(&keys.pk), &keys.sk, Cursor::new(text), None, None)
        .unwrap()
        .to_string()
}

fn signed(keys: &KeyPair, text: &str) -> String {
    envelope(text, &signature(keys, text))
}

/// A policy with only the required fields, plus `extra`.
fn policy_with(extra: Value) -> String {
    let mut policy = json!({"v": 1, "org_id": "acme-eng", "issued_at": "2026-10-21T09:00:00Z"});
    for (k, v) in extra.as_object().unwrap() {
        policy[k] = v.clone();
    }
    policy.to_string()
}

fn invalid(text: &str) -> String {
    match parse_policy(text) {
        Err(PolicyError::Invalid(why)) => why,
        other => panic!("expected an invalid policy, got {other:?}"),
    }
}

fn parsed(extra: Value) -> OrgPolicy {
    parse_policy(&policy_with(extra)).unwrap()
}

#[test]
fn the_spec_example_policy_parses_exactly() {
    let p = parse_policy(SPEC_POLICY).unwrap();
    assert_eq!(p.org_id, "acme-eng");
    assert_eq!(p.issued_at, "2026-10-21T09:00:00Z");
    assert_eq!(p.plain, ["LOG_LEVEL", "API_BASE_URL"]);
    assert_eq!(
        p.use_rules,
        UseRules {
            allow: vec!["*".into()],
            deny: vec!["PROD_*".into()],
        }
    );
    assert_eq!(p.personal, PersonalMode::Fallback);
    assert!(!p.shell_secrets);
    assert_eq!(p.refresh_minutes, 15);
    assert_eq!(p.offline_hours, 24);
    assert_eq!(
        p.audit,
        Some(AuditTarget {
            endpoint: "https://audit.acme.internal/dots".into(),
            token_entry: "DOTS_AUDIT_TOKEN".into(),
        })
    );
}

#[test]
fn a_policy_with_only_the_required_fields_takes_every_default() {
    let p = parsed(json!({}));
    assert!(p.plain.is_empty());
    assert_eq!(p.use_rules, UseRules::default());
    assert_eq!(p.use_rules.allow, ["*"]);
    assert!(p.use_rules.deny.is_empty());
    assert_eq!(p.personal, PersonalMode::Deny);
    assert!(!p.shell_secrets);
    assert_eq!(p.refresh_minutes, 15);
    assert_eq!(p.offline_hours, 24);
    assert_eq!(p.audit, None);

    // A `use` that gives only `deny` still allows everything else.
    let p = parsed(json!({"use": {"deny": ["PROD_*"]}}));
    assert_eq!(p.use_rules.allow, ["*"]);
    // `offline_hours: 0` means no offline copy, and is allowed.
    assert_eq!(parsed(json!({"offline_hours": 0})).offline_hours, 0);
    assert_eq!(
        parsed(json!({"personal": "override"})).personal,
        PersonalMode::Override
    );
}

#[test]
fn the_required_fields_are_required() {
    assert_eq!(
        invalid(r#"{"org_id": "acme-eng", "issued_at": "2026-10-21T09:00:00Z"}"#),
        "missing field 'v'"
    );
    assert_eq!(
        invalid(r#"{"v": 1, "issued_at": "2026-10-21T09:00:00Z"}"#),
        "missing field 'org_id'"
    );
    assert_eq!(
        invalid(r#"{"v": 1, "org_id": "acme-eng"}"#),
        "missing field 'issued_at'"
    );
}

#[test]
fn an_unknown_top_level_field_is_refused() {
    let err = parse_policy(&policy_with(json!({"foo": true}))).unwrap_err();
    assert_eq!(err, PolicyError::Invalid("unknown field 'foo'".into()));
    assert_eq!(err.to_string(), "unknown field 'foo'");
    // Inside `use` and `audit` too: the policy fails closed throughout.
    assert!(
        invalid(&policy_with(json!({"use": {"allow": ["*"], "except": []}})))
            .contains("unknown field 'except'")
    );
    assert!(invalid(&policy_with(
        json!({"audit": {"endpoint": "https://a.example", "token_entry": "T", "x": 1}})
    ))
    .contains("unknown field 'x'"));
}

#[test]
fn a_version_other_than_1_is_refused_before_its_fields_are_read() {
    let v2 = json!({"v": 2, "org_id": "acme-eng", "issued_at": "2026-10-21T09:00:00Z",
                    "new_v2_field": true});
    assert_eq!(
        invalid(&v2.to_string()),
        "policy version 2 is not supported; this Dots reads version 1"
    );
    assert!(invalid(&policy_with(json!({"v": "1"}))).contains("version \"1\" is not supported"));
}

#[test]
fn a_policy_that_is_not_a_json_object_or_repeats_a_field_is_refused() {
    assert!(invalid("not json").starts_with("not JSON: "));
    assert!(!invalid("{\"v\": 1,").contains("line"));
    assert_eq!(invalid("[1]"), "the policy must be a JSON object");
    let twice = r#"{"v": 1, "org_id": "acme-eng", "org_id": "evil",
                    "issued_at": "2026-10-21T09:00:00Z"}"#;
    assert_eq!(invalid(twice), "duplicate field 'org_id'");
}

#[test]
fn refresh_minutes_below_1_is_refused() {
    assert_eq!(
        invalid(&policy_with(json!({"refresh_minutes": 0}))),
        "refresh_minutes must be at least 1"
    );
    assert_eq!(parsed(json!({"refresh_minutes": 1})).refresh_minutes, 1);
    assert!(invalid(&policy_with(json!({"refresh_minutes": -5}))).contains("invalid value"));
}

#[test]
fn a_glob_with_anything_but_name_characters_and_star_is_refused() {
    for bad in ["PROD_?", "PROD_[0-9]", "", "A B", "PROD-*", "A**{B}"] {
        let why = invalid(&policy_with(json!({"use": {"deny": [bad]}})));
        assert!(
            why.starts_with(&format!("use.deny: '{bad}' ")),
            "{bad}: {why}"
        );
    }
    let why = invalid(&policy_with(json!({"use": {"allow": ["GITHUB_?"]}})));
    assert_eq!(
        why,
        "use.allow: 'GITHUB_?' may hold only A-Z, a-z, 0-9, '_' and '*'"
    );
    assert!(
        invalid(&policy_with(json!({"use": {"allow": ["A".repeat(129)]}})))
            .contains("must be 1 to 128 characters")
    );
}

#[test]
fn every_value_is_checked() {
    assert_eq!(
        invalid(&policy_with(json!({"plain": ["LOG LEVEL"]}))),
        "plain: 'LOG LEVEL' is not a variable name"
    );
    assert!(invalid(&policy_with(json!({"personal": "sometimes"})))
        .contains("unknown variant 'sometimes'"));
    assert!(invalid(&policy_with(json!({"shell_secrets": "yes"}))).contains("invalid type"));
    assert_eq!(
        invalid(&policy_with(json!({"issued_at": "21 Oct 2026"}))),
        "issued_at '21 Oct 2026' is not an RFC 3339 time, like 2026-10-21T09:00:00Z"
    );
    assert!(invalid(&policy_with(json!({"issued_at": "2026-10-21"}))).contains("RFC 3339"));
    assert!(invalid(&policy_with(json!({"org_id": "Acme Eng"})))
        .starts_with("org id 'Acme Eng': use 1 to 63 characters"));
}

#[test]
fn the_audit_target_needs_an_https_endpoint_and_an_entry_name() {
    let audit = |endpoint: &str, token_entry: &str| {
        policy_with(json!({"audit": {"endpoint": endpoint, "token_entry": token_entry}}))
    };
    for bad in [
        "http://audit.acme.internal/dots",
        "audit.acme.internal",
        "https://",
        "",
    ] {
        assert_eq!(
            invalid(&audit(bad, "DOTS_AUDIT_TOKEN")),
            format!("audit.endpoint '{bad}' must be an https:// URL")
        );
    }
    assert_eq!(
        invalid(&audit("https://audit.acme.internal", "DOTS-AUDIT")),
        "audit.token_entry 'DOTS-AUDIT' is not a variable name"
    );
    assert!(invalid(&policy_with(
        json!({"audit": {"endpoint": "https://a.example"}})
    ))
    .contains("missing field 'token_entry'"));
    assert_eq!(parsed(json!({"audit": null})).audit, None);
}

#[test]
fn may_use_applies_allow_then_deny_without_regard_to_case() {
    let p = parse_policy(SPEC_POLICY).unwrap();
    assert!(p.may_use("GITHUB_TOKEN"));
    assert!(!p.may_use("PROD_DB"));
    assert!(!p.may_use("prod_db"), "the deny glob ignores case");
    assert!(!p.may_use("Prod_Db_Password"));
    assert!(p.may_use("PROD"), "PROD_* needs the underscore");

    let p = parsed(json!({"use": {"allow": ["github_*", "LOG_LEVEL"], "deny": ["GITHUB_ADMIN*"]}}));
    assert!(p.may_use("GITHUB_TOKEN"));
    assert!(p.may_use("log_level"));
    assert!(!p.may_use("GITHUB_ADMIN"), "deny wins over allow");
    assert!(!p.may_use("github_admin_token"));
    assert!(!p.may_use("OTHER"), "a name no allow glob matches");

    let p = parsed(json!({"use": {"allow": ["*_TOKEN", "A*B*C"], "deny": []}}));
    assert!(p.may_use("_TOKEN"), "a star matches an empty run");
    assert!(p.may_use("NPM_TOKEN"));
    assert!(!p.may_use("NPM_TOKEN_2"));
    assert!(p.may_use("AxxBxxC"));
    assert!(p.may_use("ABBC"));
    assert!(!p.may_use("ABCD"));

    let none = parsed(json!({"use": {"allow": []}}));
    assert!(!none.may_use("ANY"), "an empty allow list allows nothing");
}

#[test]
fn is_plain_reads_plain_without_regard_to_case() {
    let p = parse_policy(SPEC_POLICY).unwrap();
    assert!(p.is_plain("LOG_LEVEL"));
    assert!(p.is_plain("api_base_url"));
    assert!(!p.is_plain("DOTS_AUDIT_TOKEN"));
}

#[test]
fn a_policy_signed_by_the_org_key_verifies() {
    let keys = keys();
    let text = signed(&keys, SPEC_POLICY);
    let policy = verify_policy(&text, &keys.pk.to_base64(), "acme-eng").unwrap();
    assert_eq!(policy, parse_policy(SPEC_POLICY).unwrap());
    // The envelope is the documented shape, with the policy text unchanged.
    let value: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["v"], 1);
    assert_eq!(value["policy"], SPEC_POLICY);
    assert!(value["signature"]
        .as_str()
        .unwrap()
        .starts_with("untrusted comment: "));
}

#[test]
fn a_policy_changed_after_signing_is_refused() {
    let keys = keys();
    let sig = signature(&keys, SPEC_POLICY);
    let tampered = SPEC_POLICY.replace(r#""deny": ["PROD_*"]"#, r#""deny": []"#);
    assert_ne!(tampered, SPEC_POLICY);
    let err =
        verify_policy(&envelope(&tampered, &sig), &keys.pk.to_base64(), "acme-eng").unwrap_err();
    assert_eq!(err, PolicyError::Signature);
    assert_eq!(err.to_string(), "signature does not match");
    // Even a change in white space only.
    let spaced = format!("{SPEC_POLICY}\n");
    assert_eq!(
        verify_policy(&envelope(&spaced, &sig), &keys.pk.to_base64(), "acme-eng"),
        Err(PolicyError::Signature)
    );
}

#[test]
fn a_policy_signed_by_another_key_is_refused() {
    let (org, other) = (keys(), keys());
    let text = signed(&other, SPEC_POLICY);
    assert_eq!(
        verify_policy(&text, &org.pk.to_base64(), "acme-eng"),
        Err(PolicyError::Signature)
    );
}

#[test]
fn a_signature_relabelled_as_legacy_is_refused() {
    // Turn the `ED` (prehashed) algorithm tag into legacy `Ed`, which signs the raw bytes: only
    // prehashed signatures are accepted.
    let keys = keys();
    let sig = signature(&keys, SPEC_POLICY);
    let mut lines: Vec<String> = sig.lines().map(String::from).collect();
    let mut bytes = STANDARD.decode(&lines[1]).unwrap();
    assert_eq!(&bytes[..2], b"ED");
    bytes[1] = b'd';
    lines[1] = STANDARD.encode(&bytes);
    let legacy = lines.join("\n") + "\n";
    assert_eq!(
        verify_policy(
            &envelope(SPEC_POLICY, &legacy),
            &keys.pk.to_base64(),
            "acme-eng"
        ),
        Err(PolicyError::Signature)
    );
}

#[test]
fn a_policy_for_another_org_is_refused() {
    let keys = keys();
    let err = verify_policy(
        &signed(&keys, SPEC_POLICY),
        &keys.pk.to_base64(),
        "other-org",
    )
    .unwrap_err();
    assert_eq!(
        err,
        PolicyError::WrongOrg {
            found: "acme-eng".into(),
            expected: "other-org".into(),
        }
    );
    assert_eq!(err.to_string(), "for org 'acme-eng', not 'other-org'");
}

#[test]
fn a_signed_policy_that_is_invalid_is_refused_after_its_signature() {
    let keys = keys();
    let text = policy_with(json!({"refresh_minutes": 0}));
    assert_eq!(
        verify_policy(&signed(&keys, &text), &keys.pk.to_base64(), "acme-eng"),
        Err(PolicyError::Invalid(
            "refresh_minutes must be at least 1".into()
        ))
    );
}

#[test]
fn a_bad_envelope_or_key_is_refused() {
    let keys = keys();
    let key = keys.pk.to_base64();
    let sig = signature(&keys, SPEC_POLICY);
    let envelope_error = |text: &str| match verify_policy(text, &key, "acme-eng") {
        Err(PolicyError::Envelope(why)) => why,
        other => panic!("expected a bad envelope, got {other:?}"),
    };
    // The bare policy, not wrapped.
    assert!(envelope_error(SPEC_POLICY).contains("unknown field 'org_id'"));
    assert!(!envelope_error("{").contains("line"));
    assert!(
        envelope_error(&json!({"v": 1, "policy": SPEC_POLICY}).to_string())
            .contains("missing field 'signature'")
    );
    assert!(envelope_error(
        &json!({"v": 1, "policy": SPEC_POLICY, "signature": sig, "by": "admin"}).to_string()
    )
    .contains("unknown field 'by'"));
    assert_eq!(
        envelope_error(&json!({"v": 2, "policy": SPEC_POLICY, "signature": sig}).to_string()),
        "version 2 is not supported; this Dots reads version 1"
    );
    assert_eq!(
        envelope_error(&envelope(SPEC_POLICY, "not a minisig")),
        "the signature is not a minisign signature"
    );
    let err = verify_policy(&envelope(SPEC_POLICY, &sig), "RWQnot-a-key", "acme-eng").unwrap_err();
    assert_eq!(err, PolicyError::Key);
    assert_eq!(
        err.to_string(),
        "the org's policy key is not a minisign public key"
    );
}

#[test]
fn an_older_policy_is_rolled_back_and_the_same_or_newer_is_accepted() {
    let at = |issued_at: &str| parsed(json!({"issued_at": issued_at}));
    let last = Some("2026-10-21T09:00:00Z");

    assert_eq!(
        at("2026-10-21T09:00:00Z").check_not_rolled_back(last),
        Ok(())
    );
    assert_eq!(
        at("2026-10-22T09:00:00Z").check_not_rolled_back(last),
        Ok(())
    );
    let err = at("2026-10-21T08:59:59Z")
        .check_not_rolled_back(last)
        .unwrap_err();
    assert_eq!(
        err,
        PolicyError::RolledBack {
            issued_at: "2026-10-21T08:59:59Z".into(),
            last: "2026-10-21T09:00:00Z".into(),
        }
    );
    assert!(err
        .to_string()
        .starts_with("issued before the last accepted policy"));

    // The comparison is of instants, not text: 10:00 at +01:00 is 09:00Z.
    assert_eq!(
        at("2026-10-21T10:00:00+01:00").check_not_rolled_back(last),
        Ok(())
    );
    assert!(at("2026-10-21T09:59:59+01:00")
        .check_not_rolled_back(last)
        .is_err());
    // As text "2026-10-21T08:30:00-01:00" sorts first, but it is later.
    assert_eq!(
        at("2026-10-21T08:30:00-01:00").check_not_rolled_back(last),
        Ok(())
    );

    // No policy accepted yet; and a damaged last value never turns the check off.
    assert_eq!(
        at("2020-01-01T00:00:00Z").check_not_rolled_back(None),
        Ok(())
    );
    assert!(at("2026-10-22T09:00:00Z")
        .check_not_rolled_back(Some("yesterday"))
        .is_err());
}

fn spec_join_code(policy_key: &str) -> JoinCode {
    JoinCode {
        v: 1,
        org_id: "acme-eng".into(),
        name: "Acme Engineering".into(),
        source: SourceSettings::OpenBao(OpenBaoSettings {
            address: "https://bao.acme.internal".into(),
            mount: "secret".into(),
            path: "dots".into(),
            auth: BaoAuth::Token,
            auth_mount: None,
            namespace: None,
        }),
        policy_key: policy_key.into(),
    }
}

/// The join code text for any JSON, so a test can write one Dots would refuse.
fn code_for(value: &Value) -> String {
    URL_SAFE_NO_PAD.encode(value.to_string())
}

fn join_value(key: &str, source: Value) -> Value {
    json!({"v": 1, "org_id": "acme-eng", "name": "Acme Engineering", "source": source,
           "policy_key": key})
}

fn refused(text: &str) -> String {
    match parse_join_code(text) {
        Err(Error::Invalid(why)) => why,
        other => panic!("expected a refused join code, got {other:?}"),
    }
}

#[test]
fn the_spec_join_code_reads_with_its_defaults() {
    let key = keys().pk.to_base64();
    let text = code_for(&json!({
        "v": 1, "org_id": "acme-eng", "name": "Acme Engineering",
        "source": { "kind": "openbao", "address": "https://bao.acme.internal", "path": "dots" },
        "policy_key": key
    }));
    let code = parse_join_code(&text).unwrap();
    assert_eq!(code, spec_join_code(&key));
    assert_eq!(code.source.kind(), "openbao");
    assert_eq!(code.source.address(), "https://bao.acme.internal");
    let SourceSettings::OpenBao(bao) = &code.source else {
        unreachable!()
    };
    assert_eq!(bao.login_mount(), None);
}

#[test]
fn a_join_code_round_trips_without_padding_and_reads_with_whitespace_or_padding() {
    let code = spec_join_code(&keys().pk.to_base64());
    let text = code.encode();
    assert!(!text.contains('='));
    assert!(
        !text.contains('+') && !text.contains('/'),
        "base64url, not base64"
    );
    assert_eq!(parse_join_code(&text).unwrap(), code);

    // Pasted with white space around it, and wrapped across lines as an email might.
    let wrapped = text
        .as_bytes()
        .chunks(40)
        .map(|c| std::str::from_utf8(c).unwrap())
        .collect::<Vec<_>>()
        .join("\r\n");
    assert_eq!(
        parse_join_code(&format!("  \n{wrapped}\n\t")).unwrap(),
        code
    );

    // With `=` padding, as a base64 tool that pads would write it.
    let padded = format!("{text}{}", "=".repeat((4 - text.len() % 4) % 4));
    assert_eq!(parse_join_code(&padded).unwrap(), code);
    let padded = URL_SAFE_NO_PAD.encode("x") + "==";
    assert!(refused(&padded).starts_with("join code: no JSON inside"));
}

#[test]
fn every_source_kind_round_trips_with_its_defaults_spelled_out() {
    let key = keys().pk.to_base64();
    let infisical = parse_join_code(&code_for(&join_value(
        &key,
        json!({"kind": "infisical", "site_url": "https://app.infisical.com",
               "project_id": "p-1", "environment": "prod"}),
    )))
    .unwrap();
    assert_eq!(
        infisical.source,
        SourceSettings::Infisical(InfisicalSettings {
            site_url: "https://app.infisical.com".into(),
            project_id: "p-1".into(),
            environment: "prod".into(),
            secret_path: "/".into(),
        })
    );
    assert_eq!(parse_join_code(&infisical.encode()).unwrap(), infisical);

    let bao = parse_join_code(&code_for(&join_value(
        &key,
        json!({"kind": "openbao", "address": "https://bao.acme.internal", "mount": "kv",
               "path": "teams/dots", "auth": "ldap", "auth_mount": "corp-ldap",
               "namespace": "eng"}),
    )))
    .unwrap();
    let SourceSettings::OpenBao(settings) = &bao.source else {
        unreachable!()
    };
    assert_eq!(settings.auth, BaoAuth::Ldap);
    assert_eq!(settings.login_mount(), Some("corp-ldap"));
    assert_eq!(settings.namespace.as_deref(), Some("eng"));
    assert_eq!(parse_join_code(&bao.encode()).unwrap(), bao);

    let userpass = parse_join_code(&code_for(&join_value(
        &key,
        json!({"kind": "openbao", "address": "http://127.0.0.1:8200", "path": "dots",
               "auth": "userpass"}),
    )))
    .unwrap();
    let SourceSettings::OpenBao(settings) = &userpass.source else {
        unreachable!()
    };
    assert_eq!(settings.login_mount(), Some("userpass"));

    for location in [
        r"C:\Shared\acme\dots.sops.json",
        "https://files.acme.example/dots.json",
    ] {
        let sops = parse_join_code(&code_for(&join_value(
            &key,
            json!({"kind": "sops", "location": location}),
        )))
        .unwrap();
        assert_eq!(
            sops.source,
            SourceSettings::Sops(SopsSettings {
                location: location.into()
            })
        );
        assert_eq!(sops.source.address(), location);
        assert_eq!(parse_join_code(&sops.encode()).unwrap(), sops);
    }
}

#[test]
fn a_join_code_with_an_unknown_version_is_refused() {
    let key = keys().pk.to_base64();
    let mut value = join_value(
        &key,
        json!({"kind": "sops", "location": "https://a.example/f"}),
    );
    value["v"] = json!(2);
    value["new_field"] = json!(true);
    assert_eq!(
        refused(&code_for(&value)),
        "join code: version 2 is not supported; update Dots"
    );
    value.as_object_mut().unwrap().remove("v");
    assert_eq!(refused(&code_for(&value)), "join code: no version 'v'");
}

#[test]
fn a_join_code_with_a_bad_org_id_is_refused() {
    let key = keys().pk.to_base64();
    let long = "a".repeat(64);
    for bad in [
        "Acme",
        "-acme",
        "",
        "acme_eng",
        "acme eng",
        "acme.eng",
        long.as_str(),
    ] {
        let mut value = join_value(
            &key,
            json!({"kind": "sops", "location": "https://a.example/f"}),
        );
        value["org_id"] = json!(bad);
        let why = refused(&code_for(&value));
        assert!(
            why.starts_with(&format!("org id '{bad}': use 1 to 63 characters")),
            "{bad}: {why}"
        );
    }
    for good in ["0", "a", "acme-eng", "9-lives-", &"a".repeat(63)] {
        let mut value = join_value(
            &key,
            json!({"kind": "sops", "location": "https://a.example/f"}),
        );
        value["org_id"] = json!(good);
        assert_eq!(parse_join_code(&code_for(&value)).unwrap().org_id, good);
    }
}

/// `personal` is the Personal scope's key, so no org may take it: not in a join code, not in a
/// policy, not anywhere an org id is checked.
#[test]
fn the_org_id_personal_is_reserved() {
    const RESERVED: &str = "'personal' is reserved for your own keys; choose another org id";
    match check_org_id("personal") {
        Err(Error::Invalid(why)) => assert_eq!(why, RESERVED),
        other => panic!("expected the id refused, got {other:?}"),
    }
    let key = keys().pk.to_base64();
    let mut value = join_value(
        &key,
        json!({"kind": "sops", "location": "https://a.example/f"}),
    );
    value["org_id"] = json!("personal");
    assert_eq!(refused(&code_for(&value)), RESERVED);
    assert_eq!(
        invalid(&policy_with(json!({"org_id": "personal"}))),
        RESERVED
    );
    // Only the exact id: one that holds the word is an id like any other.
    for good in ["personal-2", "my-personal", "personals"] {
        check_org_id(good).unwrap();
    }
}

#[test]
fn a_join_code_whose_policy_key_is_not_a_minisign_key_is_refused() {
    let keys = keys();
    let pub_file = keys.pk.to_box().unwrap().into_string();
    for bad in [
        "RWQ-not-a-key".to_string(),
        String::new(),
        pub_file,
        format!(" {}", keys.pk.to_base64()),
    ] {
        let value = join_value(
            &bad,
            json!({"kind": "sops", "location": "https://a.example/f"}),
        );
        assert_eq!(
            refused(&code_for(&value)),
            "join code: its policy_key is not a minisign public key (the RW… line of the .pub \
             file)",
            "{bad:?}"
        );
    }
}

#[test]
fn a_source_address_that_is_not_https_is_refused() {
    let key = keys().pk.to_base64();
    let bao = |address: &str| {
        code_for(&join_value(
            &key,
            json!({"kind": "openbao", "address": address, "path": "dots"}),
        ))
    };
    let not_https = "must start with https:// (http:// only for localhost or 127.0.0.1)";
    for bad in [
        "http://bao.acme.internal",
        "http://10.0.0.5:8200",
        "ftp://bao.acme.internal",
        "http://localhost.acme.example",
    ] {
        assert_eq!(
            refused(&bao(bad)),
            format!("join code: the source's address '{bad}' {not_https}")
        );
    }
    assert_eq!(
        refused(&bao("bao.acme.internal")),
        "join code: the source's address 'bao.acme.internal' is not a URL"
    );
    assert_eq!(
        refused(&bao("https://admin:hunter2@bao.acme.internal")),
        "join code: the source's address 'https://admin:hunter2@bao.acme.internal' must not \
         hold a user name or password"
    );
    for local in [
        "http://localhost:8200",
        "http://127.0.0.1:8200",
        "https://bao.acme.internal",
    ] {
        assert!(parse_join_code(&bao(local)).is_ok(), "{local}");
    }

    let infisical = code_for(&join_value(
        &key,
        json!({"kind": "infisical", "site_url": "http://infisical.acme.internal",
               "project_id": "p", "environment": "prod"}),
    ));
    assert_eq!(
        refused(&infisical),
        format!("join code: the source's site_url 'http://infisical.acme.internal' {not_https}")
    );
    let sops = code_for(&join_value(
        &key,
        json!({"kind": "sops", "location": "http://files.acme.example/dots.json"}),
    ));
    assert_eq!(
        refused(&sops),
        format!(
            "join code: the source's location 'http://files.acme.example/dots.json' {not_https}"
        )
    );
}

#[test]
fn a_join_code_missing_what_a_fetch_needs_is_refused() {
    let key = keys().pk.to_base64();
    let refused_for = |source: Value| refused(&code_for(&join_value(&key, source)));
    assert_eq!(
        refused_for(json!({"kind": "sops", "location": r"shared\dots.json"})),
        "join code: the source's location 'shared\\dots.json' must be a full path or an \
         https:// URL"
    );
    assert_eq!(
        refused_for(json!({"kind": "openbao", "address": "https://b.example", "path": " "})),
        "join code: the source's path is empty"
    );
    assert!(
        refused_for(json!({"kind": "openbao", "address": "https://b.example"}))
            .contains("missing field 'path'")
    );
    assert_eq!(
        refused_for(json!({"kind": "infisical", "site_url": "https://i.example",
                           "project_id": "p", "environment": "prod", "secret_path": "dots"})),
        "join code: the source's secret_path 'dots' must start with '/'"
    );
    assert!(
        refused_for(json!({"kind": "dropbox", "location": "https://d.example"}))
            .contains("unknown variant 'dropbox'")
    );
    assert!(
        refused_for(json!({"kind": "sops", "location": "https://d.example", "key": "x"}))
            .contains("unknown field 'key'")
    );
    assert!(
        refused_for(json!({"kind": "openbao", "address": "https://b.example",
                               "path": "dots", "auth": "oidc"}))
        .contains("unknown variant 'oidc'")
    );

    let mut value = join_value(
        &key,
        json!({"kind": "sops", "location": "https://d.example"}),
    );
    value["name"] = json!("  ");
    assert_eq!(
        refused(&code_for(&value)),
        "join code: the org's name is empty"
    );
    value["name"] = json!("Acme");
    value["admin"] = json!("me");
    assert!(refused(&code_for(&value)).contains("unknown field 'admin'"));
}

#[test]
fn text_that_is_not_a_join_code_is_refused() {
    assert_eq!(refused("not a join code!"), "join code: not base64url text");
    assert_eq!(refused(""), "join code: no JSON inside");
    assert_eq!(
        refused(&URL_SAFE_NO_PAD.encode("[1, 2]")),
        "join code: no version 'v'"
    );
}

#[test]
fn the_key_fingerprint_is_the_minisign_key_id() {
    let keys = keys();
    let fingerprint = key_fingerprint(&keys.pk.to_base64()).unwrap();
    let keynum: [u8; 8] = keys.pk.keynum().try_into().unwrap();
    assert_eq!(fingerprint, format!("{:016X}", u64::from_le_bytes(keynum)));
    assert_eq!(fingerprint.len(), 16);
    // The id `minisign` writes in the .pub file's comment.
    let pub_file = keys.pk.to_box().unwrap().into_string();
    assert!(
        pub_file.starts_with(&format!(
            "untrusted comment: minisign public key: {fingerprint}\n"
        )),
        "{pub_file}"
    );
    assert!(matches!(
        key_fingerprint("RWQ-not-a-key"),
        Err(Error::Invalid(_))
    ));
}
