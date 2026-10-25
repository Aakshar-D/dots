//! Reading an org from Infisical (env vault spec §6.1 to §6.3), against a fake server: the
//! Universal Auth login and its kept token, the entries of one folder and the policy of its
//! `policy` sub-folder, the query sent, and what each failure means. No error ever holds a value.

mod common;

use std::time::{Duration, Instant};

use common::fake_infisical::{Failure, FakeInfisical};
use dots_core::vault::org::{
    http_client, http_client_with_timeout, source_for, Fetched, Infisical, InfisicalSettings,
    Source, SourceCreds, SourceError, SourceSettings,
};
use serde_json::{json, Value};
use zeroize::Zeroizing;

const CLIENT_ID: &str = "ci-6f1a2b70";
const CLIENT_SECRET: &str = "cs-9d4e7c21";
const WRONG_SECRET: &str = "cs-wrong-05ab";
const SECRET: &str = "val-secret-3b6a9f";
const PERSONAL: &str = "val-personal-8e1f";
const IMPORTED: &str = "val-imported-2c7d";
const NUMBER: u64 = 734911;
const POLICY: &str = "signed-policy-71e2c0";

const PROJECT: &str = "proj-4a1c";
const ENV: &str = "dev";
const PATH: &str = "/dots";

/// Every value a test hands the fake, and the prefix of the tokens its logins give.
const VALUES: &[&str] = &[
    CLIENT_ID,
    CLIENT_SECRET,
    WRONG_SECRET,
    SECRET,
    PERSONAL,
    IMPORTED,
    "734911",
    POLICY,
    "inf.issued",
];

fn assert_clean(e: &SourceError) {
    let text = format!("{e} {e:?}");
    for value in VALUES {
        assert!(!text.contains(value), "{text:?} holds {value:?}");
    }
}

fn z(s: &str) -> Zeroizing<String> {
    Zeroizing::new(s.to_string())
}

fn machine(client_id: &str, client_secret: &str) -> SourceCreds {
    SourceCreds::Machine {
        client_id: z(client_id),
        client_secret: z(client_secret),
    }
}

fn creds() -> SourceCreds {
    machine(CLIENT_ID, CLIENT_SECRET)
}

fn settings_at(site_url: &str, secret_path: &str) -> InfisicalSettings {
    InfisicalSettings {
        site_url: site_url.to_string(),
        project_id: PROJECT.into(),
        environment: ENV.into(),
        secret_path: secret_path.into(),
    }
}

fn settings(fake: &FakeInfisical) -> InfisicalSettings {
    settings_at(&fake.address(), PATH)
}

fn infisical(settings: InfisicalSettings) -> Infisical {
    Infisical::new(settings, http_client())
}

/// A fake with the org's entries in `dev:/dots`, its policy in `dev:/dots/policy`, and the
/// machine identity `CLIENT_ID`, whose tokens last an hour.
async fn seeded() -> FakeInfisical {
    let fake = FakeInfisical::start().await;
    seed(&fake, PATH);
    fake.add_identity(CLIENT_ID, CLIENT_SECRET, 3600);
    fake
}

fn seed(fake: &FakeInfisical, path: &str) {
    fake.put_secret(PROJECT, ENV, path, "API_KEY", json!(SECRET));
    fake.put_secret(PROJECT, ENV, path, "LOG_LEVEL", json!("debug"));
    fake.put_policy(PROJECT, ENV, path, POLICY);
}

fn names(fetched: &Fetched) -> Vec<&str> {
    fetched.entries.keys().map(String::as_str).collect()
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

/// The `secretPath` of each read so far.
fn read_paths(fake: &FakeInfisical) -> Vec<String> {
    fake.reads()
        .iter()
        .map(|s| s.param("secretPath").unwrap().to_string())
        .collect()
}

// Reading.

#[tokio::test]
async fn a_login_then_a_read_give_the_entries_and_the_policy() {
    let fake = seeded().await;
    let fetched = infisical(settings(&fake)).fetch(&creds()).await.unwrap();

    assert_eq!(names(&fetched), ["API_KEY", "LOG_LEVEL"]);
    assert_eq!(fetched.entries["API_KEY"].as_str(), SECRET);
    assert_eq!(fetched.entries["LOG_LEVEL"].as_str(), "debug");
    assert_eq!(fetched.policy, POLICY);
    assert_eq!(
        fake.calls(),
        [
            "POST /api/v1/auth/universal-auth/login",
            "GET /api/v4/secrets",
            "GET /api/v4/secrets",
        ]
    );

    let seen = fake.requests();
    assert_eq!(
        seen[0].body,
        Some(json!({ "clientId": CLIENT_ID, "clientSecret": CLIENT_SECRET }))
    );
    assert_eq!(seen[0].auth, None);
    assert!(seen[0].query.is_empty());
    for (read, path) in seen[1..].iter().zip(["/dots", "/dots/policy"]) {
        assert_eq!(read.auth.as_deref(), Some("Bearer inf.issued-1"));
        let query: Vec<(&str, &str)> = read
            .query
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        assert_eq!(
            query,
            [
                ("projectId", PROJECT),
                ("environment", ENV),
                ("secretPath", path),
                ("recursive", "false"),
                ("includeImports", "false"),
                ("viewSecretValue", "true"),
            ]
        );
    }
}

#[tokio::test]
async fn the_root_folder_reads_its_policy_from_slash_policy() {
    let fake = FakeInfisical::start().await;
    seed(&fake, "/");
    fake.add_identity(CLIENT_ID, CLIENT_SECRET, 3600);
    let source = infisical(settings_at(&fake.address(), "/"));
    let fetched = source.fetch(&creds()).await.unwrap();
    assert_eq!(names(&fetched), ["API_KEY", "LOG_LEVEL"]);
    assert_eq!(fetched.policy, POLICY);
    assert_eq!(read_paths(&fake), ["/", "/policy"]);
}

#[tokio::test]
async fn a_nested_folder_reads_its_policy_from_its_sub_folder() {
    let fake = FakeInfisical::start().await;
    seed(&fake, "/team/dots");
    fake.add_identity(CLIENT_ID, CLIENT_SECRET, 3600);
    for path in ["/team/dots", "/team/dots/"] {
        fake.clear_requests();
        let source = infisical(settings_at(&fake.address(), path));
        let fetched = source.fetch(&creds()).await.unwrap();
        assert_eq!(fetched.policy, POLICY);
        assert_eq!(
            read_paths(&fake),
            ["/team/dots", "/team/dots/policy"],
            "{path}"
        );
    }
}

#[tokio::test]
async fn a_site_url_with_a_slash_at_the_end_or_a_path_prefix_works() {
    let fake = seeded().await;
    let with_slash = format!("{}/", fake.address());
    let fetched = infisical(settings_at(&with_slash, PATH))
        .fetch(&creds())
        .await
        .unwrap();
    assert_eq!(fetched.policy, POLICY);
    assert!(fake.requests().iter().all(|s| s.path.starts_with("/api/")));

    let prefixed = FakeInfisical::start_at("/infisical").await;
    seed(&prefixed, PATH);
    prefixed.add_identity(CLIENT_ID, CLIENT_SECRET, 3600);
    assert!(prefixed.address().ends_with("/infisical"));
    for site in [prefixed.address(), format!("{}/", prefixed.address())] {
        prefixed.clear_requests();
        let fetched = infisical(settings_at(&site, PATH))
            .fetch(&creds())
            .await
            .unwrap();
        assert_eq!(fetched.policy, POLICY);
        let paths: Vec<String> = prefixed.requests().into_iter().map(|s| s.path).collect();
        assert_eq!(
            paths,
            [
                "/infisical/api/v1/auth/universal-auth/login",
                "/infisical/api/v4/secrets",
                "/infisical/api/v4/secrets",
            ]
        );
    }
}

#[tokio::test]
async fn personal_overrides_are_skipped() {
    let fake = seeded().await;
    fake.put_personal(PROJECT, ENV, PATH, "API_KEY", PERSONAL);
    fake.put_personal(PROJECT, ENV, PATH, "MINE_ONLY", PERSONAL);
    fake.put_personal(PROJECT, ENV, "/dots/policy", "DOTS_POLICY", PERSONAL);
    let fetched = infisical(settings(&fake)).fetch(&creds()).await.unwrap();
    assert_eq!(names(&fetched), ["API_KEY", "LOG_LEVEL"]);
    assert_eq!(fetched.entries["API_KEY"].as_str(), SECRET);
    assert_eq!(fetched.policy, POLICY);
}

#[tokio::test]
async fn a_personal_policy_alone_is_no_policy() {
    let fake = FakeInfisical::start().await;
    fake.put_secret(PROJECT, ENV, PATH, "API_KEY", json!(SECRET));
    fake.put_personal(PROJECT, ENV, "/dots/policy", "DOTS_POLICY", PERSONAL);
    fake.add_identity(CLIENT_ID, CLIENT_SECRET, 3600);
    let e = infisical(settings(&fake))
        .fetch(&creds())
        .await
        .unwrap_err();
    assert_eq!(invalid(e), "no DOTS_POLICY at dev:/dots/policy");
}

#[tokio::test]
async fn imports_are_ignored() {
    let fake = seeded().await;
    fake.put_import(PROJECT, ENV, PATH, "/shared", "IMPORTED_KEY", IMPORTED);
    fake.put_import(PROJECT, ENV, PATH, "/shared", "API_KEY", IMPORTED);
    fake.put_import(
        PROJECT,
        ENV,
        "/dots/policy",
        "/shared",
        "DOTS_POLICY",
        IMPORTED,
    );
    let fetched = infisical(settings(&fake)).fetch(&creds()).await.unwrap();
    assert_eq!(names(&fetched), ["API_KEY", "LOG_LEVEL"]);
    assert_eq!(fetched.entries["API_KEY"].as_str(), SECRET);
    assert_eq!(fetched.policy, POLICY);
}

#[tokio::test]
async fn an_empty_folder_gives_no_entries() {
    let fake = FakeInfisical::start().await;
    fake.add_folder(PROJECT, ENV, PATH);
    fake.put_policy(PROJECT, ENV, PATH, POLICY);
    fake.add_identity(CLIENT_ID, CLIENT_SECRET, 3600);
    let fetched = infisical(settings(&fake)).fetch(&creds()).await.unwrap();
    assert!(fetched.entries.is_empty());
    assert_eq!(fetched.policy, POLICY);
}

#[tokio::test]
async fn bad_names_and_the_policy_key_among_the_entries_are_skipped() {
    let fake = seeded().await;
    for name in [
        "1BAD",
        "has-dash",
        "",
        "bad name",
        "DOTS_POLICY",
        "dots_policy",
    ] {
        fake.put_secret(PROJECT, ENV, PATH, name, json!(SECRET));
    }
    // A skipped name is skipped whatever its value, and also when it is there twice: no
    // message names it, since it may be a value pasted in the wrong place.
    fake.put_secret(PROJECT, ENV, PATH, "has-dash", json!(SECRET));
    fake.put_secret(PROJECT, ENV, PATH, "DOTS_POLICY", json!(SECRET));
    fake.put_secret(PROJECT, ENV, PATH, "2BAD", json!(NUMBER));
    fake.put_raw(
        PROJECT,
        ENV,
        PATH,
        json!({ "secretKey": "3BAD", "type": "shared" }),
    );
    let fetched = infisical(settings(&fake)).fetch(&creds()).await.unwrap();
    assert_eq!(names(&fetched), ["API_KEY", "LOG_LEVEL"]);
}

// The kept access token.

#[tokio::test]
async fn the_token_is_kept_across_fetches() {
    let fake = seeded().await;
    let source = infisical(settings(&fake));
    source.fetch(&creds()).await.unwrap();
    source.fetch(&creds()).await.unwrap();
    assert_eq!(fake.logins(), 1);
    assert_eq!(fake.reads().len(), 4);
    for read in fake.reads() {
        assert_eq!(read.auth.as_deref(), Some("Bearer inf.issued-1"));
    }
}

#[tokio::test]
async fn a_token_about_to_expire_is_not_used() {
    let fake = seeded().await;
    fake.add_identity("ci-short", CLIENT_SECRET, 5);
    let source = infisical(settings(&fake));
    source
        .fetch(&machine("ci-short", CLIENT_SECRET))
        .await
        .unwrap();
    source
        .fetch(&machine("ci-short", CLIENT_SECRET))
        .await
        .unwrap();
    assert_eq!(fake.logins(), 2);
}

#[tokio::test]
async fn a_token_with_no_lifetime_is_kept() {
    let fake = seeded().await;
    fake.add_identity("ci-forever", CLIENT_SECRET, 0);
    let source = infisical(settings(&fake));
    for _ in 0..2 {
        source
            .fetch(&machine("ci-forever", CLIENT_SECRET))
            .await
            .unwrap();
    }
    assert_eq!(fake.logins(), 1);
}

#[tokio::test]
async fn other_creds_log_in_again() {
    let fake = seeded().await;
    fake.add_identity("ci-other", "cs-other", 3600);
    let source = infisical(settings(&fake));
    source.fetch(&creds()).await.unwrap();
    source
        .fetch(&machine("ci-other", "cs-other"))
        .await
        .unwrap();
    assert_eq!(fake.logins(), 2);
    let last = fake.requests().into_iter().nth(3).unwrap();
    assert_eq!(last.body.unwrap()["clientId"], json!("ci-other"));
}

#[tokio::test]
async fn a_401_on_the_kept_token_logs_in_again_once() {
    let fake = seeded().await;
    let source = infisical(settings(&fake));
    source.fetch(&creds()).await.unwrap();
    fake.revoke_all_tokens();
    fake.clear_requests();

    let fetched = source.fetch(&creds()).await.unwrap();
    assert_eq!(fetched.policy, POLICY);
    assert_eq!(
        fake.calls(),
        [
            "GET /api/v4/secrets",
            "POST /api/v1/auth/universal-auth/login",
            "GET /api/v4/secrets",
            "GET /api/v4/secrets",
        ]
    );
    let auths: Vec<Option<String>> = fake.reads().into_iter().map(|s| s.auth).collect();
    assert_eq!(
        auths,
        [
            Some("Bearer inf.issued-1".to_string()),
            Some("Bearer inf.issued-2".to_string()),
            Some("Bearer inf.issued-2".to_string()),
        ]
    );

    // The new token is kept.
    fake.clear_requests();
    source.fetch(&creds()).await.unwrap();
    assert_eq!(fake.logins(), 0);
}

#[tokio::test]
async fn a_403_on_the_kept_token_logs_in_again_once() {
    let fake = seeded().await;
    let source = infisical(settings(&fake));
    source.fetch(&creds()).await.unwrap();
    fake.fail_next(Failure::Status(403), 1);
    source.fetch(&creds()).await.unwrap();
    assert_eq!(fake.logins(), 2);
}

#[tokio::test]
async fn two_401s_in_a_row_are_denied() {
    let fake = seeded().await;
    let source = infisical(settings(&fake));
    source.fetch(&creds()).await.unwrap();
    fake.fail_next(Failure::Status(401), 2);

    let e = source.fetch(&creds()).await.unwrap_err();
    assert_eq!(
        denied(e),
        format!("{} refused access to dev:/dots (HTTP 401)", fake.address())
    );
    assert_eq!(fake.logins(), 2);

    // Nothing refused is kept: the next fetch logs in again.
    source.fetch(&creds()).await.unwrap();
    assert_eq!(fake.logins(), 3);
}

#[tokio::test]
async fn a_403_just_after_a_login_is_denied_at_once() {
    let fake = seeded().await;
    fake.fail_next(Failure::Status(403), 1);
    let e = infisical(settings(&fake))
        .fetch(&creds())
        .await
        .unwrap_err();
    assert_eq!(
        denied(e),
        format!("{} refused access to dev:/dots (HTTP 403)", fake.address())
    );
    assert_eq!(fake.logins(), 1);
}

// Logins that fail.

#[tokio::test]
async fn a_bad_client_secret_or_client_id_is_denied() {
    let fake = seeded().await;
    for creds in [
        machine(CLIENT_ID, WRONG_SECRET),
        machine("ci-unknown", CLIENT_SECRET),
    ] {
        fake.clear_requests();
        let e = infisical(settings(&fake)).fetch(&creds).await.unwrap_err();
        assert_eq!(
            denied(e),
            format!(
                "{} refused the client id or client secret (HTTP 401)",
                fake.address()
            )
        );
        assert_eq!(fake.calls(), ["POST /api/v1/auth/universal-auth/login"]);
    }
}

#[tokio::test]
async fn a_403_on_the_login_is_denied() {
    let fake = seeded().await;
    fake.fail_next_login(Failure::Status(403), 1);
    let e = infisical(settings(&fake))
        .fetch(&creds())
        .await
        .unwrap_err();
    assert_eq!(
        denied(e),
        format!(
            "{} refused the client id or client secret (HTTP 403)",
            fake.address()
        )
    );
    assert!(fake.reads().is_empty());
}

#[tokio::test]
async fn a_login_rate_limited_or_failing_is_unreachable() {
    let fake = seeded().await;
    let source = infisical(settings(&fake));
    fake.fail_next_login(Failure::Status(429), 1);
    let why = unreachable(source.fetch(&creds()).await.unwrap_err());
    assert_eq!(
        why,
        format!("{} is rate limiting Dots (HTTP 429)", fake.address())
    );
    for code in [500, 502, 503] {
        fake.fail_next_login(Failure::Status(code), 1);
        let why = unreachable(source.fetch(&creds()).await.unwrap_err());
        assert_eq!(
            why,
            format!("{} is not available (HTTP {code})", fake.address())
        );
    }
    assert!(fake.reads().is_empty());
}

#[tokio::test]
async fn a_login_answer_without_a_token_is_invalid() {
    let fake = seeded().await;
    let source = infisical(settings(&fake));
    fake.fail_next_login(Failure::EmptyObject, 1);
    let why = invalid(source.fetch(&creds()).await.unwrap_err());
    assert_eq!(
        why,
        format!(
            "{} gave no access token for the Universal Auth login",
            fake.address()
        )
    );

    fake.fail_next_login(Failure::NotJson, 1);
    let why = invalid(source.fetch(&creds()).await.unwrap_err());
    assert_eq!(
        why,
        format!(
            "the answer from {} for the Universal Auth login is not readable (line 1, column 1)",
            fake.address()
        )
    );
    assert!(fake.reads().is_empty());
}

#[tokio::test]
async fn wrong_or_empty_creds_are_invalid_and_nothing_is_sent() {
    let fake = seeded().await;
    let source = infisical(settings(&fake));
    let wrong = [
        SourceCreds::Token { token: z("t-1") },
        SourceCreds::Login {
            username: z("alice"),
            password: z("pw"),
        },
        SourceCreds::Age {
            identity: z("AGE-SECRET-KEY-1X"),
        },
        machine("", CLIENT_SECRET),
        machine(CLIENT_ID, ""),
    ];
    for creds in wrong {
        let why = invalid(source.fetch(&creds).await.unwrap_err());
        assert_eq!(
            why,
            "this org's Infisical signs in with a machine identity, so it needs a client id and \
             client secret"
        );
    }
    assert!(fake.requests().is_empty());
}

// Reads that fail.

#[tokio::test]
async fn a_read_rate_limited_is_unreachable() {
    let fake = seeded().await;
    fake.fail_next(Failure::Status(429), 1);
    let e = infisical(settings(&fake))
        .fetch(&creds())
        .await
        .unwrap_err();
    assert_eq!(
        unreachable(e),
        format!("{} is rate limiting Dots (HTTP 429)", fake.address())
    );
}

#[tokio::test]
async fn a_5xx_on_a_read_is_unreachable() {
    let fake = seeded().await;
    let source = infisical(settings(&fake));
    for code in [500, 502, 503, 504] {
        fake.fail_next(Failure::Status(code), 1);
        let why = unreachable(source.fetch(&creds()).await.unwrap_err());
        assert_eq!(
            why,
            format!("{} is not available (HTTP {code})", fake.address())
        );
    }
}

#[tokio::test]
async fn a_refused_connection_is_unreachable() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let source = infisical(settings_at(&format!("http://127.0.0.1:{port}"), PATH));
    let why = unreachable(source.fetch(&creds()).await.unwrap_err());
    assert!(
        why.starts_with(&format!("cannot reach http://127.0.0.1:{port}")),
        "{why}"
    );
}

#[tokio::test]
async fn a_source_that_hangs_is_unreachable() {
    let fake = seeded().await;
    let source = Infisical::new(
        settings(&fake),
        http_client_with_timeout(Duration::from_millis(300)),
    );
    for hang_login in [true, false] {
        if hang_login {
            fake.fail_next_login(Failure::Hang, 1);
        } else {
            fake.fail_next(Failure::Hang, 1);
        }
        let started = Instant::now();
        let why = unreachable(source.fetch(&creds()).await.unwrap_err());
        assert_eq!(why, format!("{} did not answer in time", fake.address()));
        assert!(started.elapsed() < Duration::from_secs(10));
    }
}

#[tokio::test]
async fn a_missing_folder_is_invalid() {
    let fake = seeded().await;
    fake.remove_folder(PROJECT, ENV, PATH);
    let e = infisical(settings(&fake))
        .fetch(&creds())
        .await
        .unwrap_err();
    assert_eq!(
        invalid(e),
        "no secrets at dev:/dots: the project, environment or folder does not exist"
    );
}

#[tokio::test]
async fn a_missing_policy_is_invalid() {
    // No policy folder at all (a 404).
    let fake = seeded().await;
    fake.remove_folder(PROJECT, ENV, "/dots/policy");
    let source = infisical(settings(&fake));
    let why = invalid(source.fetch(&creds()).await.unwrap_err());
    assert_eq!(why, "no DOTS_POLICY at dev:/dots/policy");

    // A policy folder without the secret, or with it in another case.
    fake.add_folder(PROJECT, ENV, "/dots/policy");
    fake.put_secret(PROJECT, ENV, "/dots/policy", "OTHER", json!(POLICY));
    fake.put_secret(PROJECT, ENV, "/dots/policy", "dots_policy", json!(POLICY));
    let why = invalid(source.fetch(&creds()).await.unwrap_err());
    assert_eq!(why, "no DOTS_POLICY at dev:/dots/policy");
}

#[tokio::test]
async fn a_policy_that_is_not_text_or_has_no_value_is_invalid() {
    let cases: [(Value, &str); 4] = [
        (
            json!({ "secretKey": "DOTS_POLICY", "secretValue": NUMBER }),
            "DOTS_POLICY is not text",
        ),
        (
            json!({ "secretKey": "DOTS_POLICY", "type": "shared" }),
            "DOTS_POLICY has no value",
        ),
        (
            json!({ "secretKey": "DOTS_POLICY", "secretValue": null }),
            "DOTS_POLICY has no value",
        ),
        (
            json!({
                "secretKey": "DOTS_POLICY",
                "secretValue": "<hidden-by-infisical>",
                "secretValueHidden": true,
            }),
            "the value of DOTS_POLICY is hidden from this machine identity",
        ),
    ];
    for (secret, expected) in cases {
        let fake = FakeInfisical::start().await;
        fake.put_secret(PROJECT, ENV, PATH, "API_KEY", json!(SECRET));
        fake.put_raw(PROJECT, ENV, "/dots/policy", secret);
        fake.add_identity(CLIENT_ID, CLIENT_SECRET, 3600);
        let e = infisical(settings(&fake))
            .fetch(&creds())
            .await
            .unwrap_err();
        assert_eq!(invalid(e), expected);
    }
}

#[tokio::test]
async fn a_value_that_is_not_text_or_is_missing_is_invalid_by_name() {
    let not_text = [
        json!(NUMBER),
        json!(true),
        json!({ "nested": SECRET }),
        json!([SECRET]),
    ];
    for value in not_text {
        let fake = seeded().await;
        fake.put_secret(PROJECT, ENV, PATH, "TOKEN_X", value.clone());
        let e = infisical(settings(&fake))
            .fetch(&creds())
            .await
            .unwrap_err();
        assert_eq!(invalid(e), "TOKEN_X is not text", "{value}");
    }
    let no_value = [
        json!({ "secretKey": "TOKEN_X", "type": "shared" }),
        json!({ "secretKey": "TOKEN_X", "secretValue": null }),
    ];
    for secret in no_value {
        let fake = seeded().await;
        fake.put_raw(PROJECT, ENV, PATH, secret);
        let e = infisical(settings(&fake))
            .fetch(&creds())
            .await
            .unwrap_err();
        assert_eq!(invalid(e), "TOKEN_X has no value");
    }
    let fake = seeded().await;
    fake.put_raw(
        PROJECT,
        ENV,
        PATH,
        json!({ "secretKey": "TOKEN_X", "secretValue": "", "secretValueHidden": true }),
    );
    let e = infisical(settings(&fake))
        .fetch(&creds())
        .await
        .unwrap_err();
    assert_eq!(
        invalid(e),
        "the value of TOKEN_X is hidden from this machine identity"
    );
}

#[tokio::test]
async fn a_secret_without_a_name_twice_the_same_or_a_case_clash_is_invalid() {
    let fake = seeded().await;
    fake.put_raw(PROJECT, ENV, PATH, json!({ "secretValue": SECRET }));
    let e = infisical(settings(&fake))
        .fetch(&creds())
        .await
        .unwrap_err();
    assert_eq!(invalid(e), "a secret at dev:/dots has no name");

    let fake = seeded().await;
    fake.put_secret(PROJECT, ENV, PATH, "API_KEY", json!(SECRET));
    let e = infisical(settings(&fake))
        .fetch(&creds())
        .await
        .unwrap_err();
    assert_eq!(invalid(e), "API_KEY is at dev:/dots twice");

    let fake = seeded().await;
    fake.put_secret(PROJECT, ENV, PATH, "Api_Key", json!(SECRET));
    let e = infisical(settings(&fake))
        .fetch(&creds())
        .await
        .unwrap_err();
    assert_eq!(
        invalid(e),
        "API_KEY and Api_Key are the same name in a different case"
    );
}

#[tokio::test]
async fn another_status_or_an_unreadable_answer_is_invalid() {
    let fake = seeded().await;
    let source = infisical(settings(&fake));
    fake.fail_next(Failure::Status(400), 1);
    let why = invalid(source.fetch(&creds()).await.unwrap_err());
    assert_eq!(
        why,
        format!("{} answered HTTP 400 for dev:/dots", fake.address())
    );

    fake.fail_next(Failure::NotJson, 1);
    let why = invalid(source.fetch(&creds()).await.unwrap_err());
    assert_eq!(
        why,
        format!(
            "the answer from {} for dev:/dots is not readable (line 1, column 1)",
            fake.address()
        )
    );

    fake.fail_next(Failure::Status(307), 1);
    let why = invalid(source.fetch(&creds()).await.unwrap_err());
    assert_eq!(
        why,
        format!("{} answered HTTP 307 for dev:/dots", fake.address())
    );
    assert!(fake.requests().iter().all(|s| s.path != "/api/elsewhere"));
}

#[tokio::test]
async fn an_answer_without_a_list_of_secrets_is_invalid() {
    let fake = seeded().await;
    fake.fail_next(Failure::EmptyObject, 1);
    let e = infisical(settings(&fake))
        .fetch(&creds())
        .await
        .unwrap_err();
    assert_eq!(
        invalid(e),
        format!(
            "the answer from {} for dev:/dots has no list of secrets",
            fake.address()
        )
    );
}

// Paths.

#[tokio::test]
async fn a_bad_secret_path_is_refused_before_any_request() {
    let fake = seeded().await;
    let cases = [
        ("", "must start with '/'"),
        ("dots", "must start with '/'"),
        ("//", "has an empty segment"),
        ("/a//b", "has an empty segment"),
        ("/a/b//", "has an empty segment"),
        ("/..", "must not hold a '.' or '..' segment"),
        ("/a/../b", "must not hold a '.' or '..' segment"),
        ("/a/./b", "must not hold a '.' or '..' segment"),
        ("/a/%2E%2e", "must not hold a '.' or '..' segment"),
        ("/a\nb", "holds a control character"),
    ];
    for (path, why) in cases {
        let source = infisical(settings_at(&fake.address(), path));
        let e = source.fetch(&creds()).await.unwrap_err();
        assert_eq!(
            invalid(e),
            format!("the source's secret_path '{}' {why}", path.escape_debug()),
            "{path:?}"
        );
    }
    // A bad path is refused before the creds are even looked at.
    let source = infisical(settings_at(&fake.address(), "/a/.."));
    let why = invalid(
        source
            .fetch(&SourceCreds::Token { token: z("t-1") })
            .await
            .unwrap_err(),
    );
    assert!(why.starts_with("the source's secret_path"), "{why}");
    assert!(fake.requests().is_empty());
}

// The source for a join code's settings.

#[tokio::test]
async fn source_for_gives_infisical() {
    let fake = seeded().await;
    let source = source_for(&SourceSettings::Infisical(settings(&fake)), http_client());
    let fetched = source.fetch(&creds()).await.unwrap();
    assert_eq!(fetched.policy, POLICY);
    assert_eq!(names(&fetched), ["API_KEY", "LOG_LEVEL"]);
}
