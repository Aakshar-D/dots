//! Reading an org from OpenBao or Vault (env vault spec §6.1 to §6.3), against a fake server:
//! the entries and the policy, the headers, token and login sign-in, what each failure means,
//! and the sign-in details kept in the secret store. No error ever holds a value.

mod common;

use std::time::{Duration, Instant};

use common::fake_openbao::{Failure, FakeOpenBao};
use common::orgs::{keys, roster, sign_roster};
use dots_core::accounts::parse_roster;
use dots_core::vault::org::{
    http_client, http_client_with_timeout, source_for, BaoAuth, Fetched, OpenBao, OpenBaoSettings,
    Source, SourceCreds, SourceError, SourceSettings,
};
use dots_core::vault::{MemorySecrets, SecretStore, MAX_SECRET_BYTES};
use dots_core::Error;
use serde_json::{json, Value};
use zeroize::Zeroizing;

const TOKEN: &str = "hvs.tok-4d9e1b7a";
const PASSWORD: &str = "pw-8c2f5e01aa";
const WRONG_PASSWORD: &str = "pw-wrong-61d0c3";
const SECRET: &str = "val-secret-3b6a9f";
const NESTED: &str = "val-nested-5e0d";
const NUMBER: u64 = 734911;
const POLICY: &str = "signed-policy-71e2c0";

/// Every value a test hands the fake, and the prefix of the tokens its logins give.
const VALUES: &[&str] = &[
    TOKEN,
    PASSWORD,
    WRONG_PASSWORD,
    SECRET,
    NESTED,
    "734911",
    POLICY,
    "hvs.issued",
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

fn token() -> SourceCreds {
    SourceCreds::Token { token: z(TOKEN) }
}

fn login(username: &str, password: &str) -> SourceCreds {
    SourceCreds::Login {
        username: z(username),
        password: z(password),
    }
}

fn settings(fake: &FakeOpenBao) -> OpenBaoSettings {
    OpenBaoSettings {
        address: fake.address(),
        mount: "secret".into(),
        path: "dots".into(),
        auth: BaoAuth::Token,
        auth_mount: None,
        namespace: None,
    }
}

fn login_settings(fake: &FakeOpenBao, auth: BaoAuth) -> OpenBaoSettings {
    OpenBaoSettings {
        auth,
        ..settings(fake)
    }
}

fn bao(settings: OpenBaoSettings) -> OpenBao {
    OpenBao::new(settings, http_client())
}

/// A fake with the org's entries and policy at `secret/dots`, the token `TOKEN` (not
/// renewable), and the `userpass` user `alice`, whose logins last an hour.
async fn seeded() -> FakeOpenBao {
    let fake = FakeOpenBao::start().await;
    fake.put_env(
        "secret",
        "dots",
        json!({ "API_KEY": SECRET, "LOG_LEVEL": "debug" }),
    );
    fake.put_policy("secret", "dots", POLICY);
    fake.allow_token(TOKEN, false);
    fake.add_user("userpass", "alice", PASSWORD, 3600);
    fake
}

fn names(fetched: &Fetched) -> Vec<&str> {
    fetched.entries.keys().map(String::as_str).collect()
}

fn invalid(e: SourceError) -> String {
    match e {
        SourceError::Invalid(why) => why,
        other => panic!("expected Invalid, got {other:?}"),
    }
}

// Reading.

#[tokio::test]
async fn a_token_read_gives_the_entries_and_the_policy() {
    let fake = seeded().await;
    let fetched = bao(settings(&fake)).fetch(&token()).await.unwrap();

    assert_eq!(names(&fetched), ["API_KEY", "LOG_LEVEL"]);
    assert_eq!(fetched.entries["API_KEY"].as_str(), SECRET);
    assert_eq!(fetched.entries["LOG_LEVEL"].as_str(), "debug");
    assert_eq!(fetched.policy, POLICY);
    assert_eq!(
        fake.calls(),
        [
            "GET /v1/secret/data/dots/env",
            "GET /v1/secret/data/dots/policy",
            "GET /v1/secret/data/dots/roster",
            "GET /v1/auth/token/lookup-self",
        ]
    );
    for seen in fake.requests() {
        assert_eq!(seen.token.as_deref(), Some(TOKEN), "{seen:?}");
        assert_eq!(seen.namespace, None, "{seen:?}");
    }
}

#[tokio::test]
async fn the_namespace_header_is_sent_only_when_set() {
    let fake = seeded().await;
    let source = bao(OpenBaoSettings {
        namespace: Some("team-a/".into()),
        ..settings(&fake)
    });
    source.fetch(&token()).await.unwrap();
    let source = bao(OpenBaoSettings {
        namespace: Some("team-a/".into()),
        ..login_settings(&fake, BaoAuth::Userpass)
    });
    source.fetch(&login("alice", PASSWORD)).await.unwrap();

    let seen = fake.requests();
    assert_eq!(seen.len(), 8, "{:?}", fake.calls());
    for seen in seen {
        assert_eq!(seen.namespace.as_deref(), Some("team-a/"), "{seen:?}");
    }
}

#[tokio::test]
async fn the_address_may_end_with_a_slash() {
    let fake = seeded().await;
    let source = bao(OpenBaoSettings {
        address: format!("{}/", fake.address()),
        ..settings(&fake)
    });
    source.fetch(&token()).await.unwrap();
    assert_eq!(fake.calls()[0], "GET /v1/secret/data/dots/env");
}

#[tokio::test]
async fn each_path_segment_is_percent_encoded() {
    let fake = seeded().await;
    fake.put_env("kv/team", "eng ops/q?x#y", json!({ "API_KEY": SECRET }));
    fake.put_policy("kv/team", "eng ops/q?x#y", POLICY);
    fake.add_user("userpass", "a/b c", PASSWORD, 3600);
    let source = bao(OpenBaoSettings {
        mount: "kv/team".into(),
        path: "/eng ops//q?x#y/".into(),
        ..login_settings(&fake, BaoAuth::Userpass)
    });
    let fetched = source.fetch(&login("a/b c", PASSWORD)).await.unwrap();

    assert_eq!(names(&fetched), ["API_KEY"]);
    assert_eq!(
        fake.calls(),
        [
            "POST /v1/auth/userpass/login/a%2Fb%20c",
            "GET /v1/kv/team/data/eng%20ops/q%3Fx%23y/env",
            "GET /v1/kv/team/data/eng%20ops/q%3Fx%23y/policy",
            "GET /v1/kv/team/data/eng%20ops/q%3Fx%23y/roster",
        ]
    );
}

#[tokio::test]
async fn fetched_debug_shows_only_the_names() {
    let fake = seeded().await;
    let fetched = bao(settings(&fake)).fetch(&token()).await.unwrap();
    let debug = format!("{fetched:?}");
    assert!(debug.contains("API_KEY"), "{debug}");
    assert!(!debug.contains(SECRET), "{debug}");
    assert!(!debug.contains(POLICY), "{debug}");
}

// Logging in.

#[tokio::test]
async fn a_userpass_login_then_a_read_and_the_token_is_kept() {
    let fake = seeded().await;
    let source = bao(login_settings(&fake, BaoAuth::Userpass));
    let first = source.fetch(&login("alice", PASSWORD)).await.unwrap();
    let second = source.fetch(&login("alice", PASSWORD)).await.unwrap();

    assert_eq!(names(&first), ["API_KEY", "LOG_LEVEL"]);
    assert_eq!(first, second);
    assert_eq!(fake.logins(), 1, "{:?}", fake.calls());
    let seen = fake.requests();
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].path, "/v1/auth/userpass/login/alice");
    assert_eq!(seen[0].token, None);
    assert_eq!(seen[0].body, Some(json!({ "password": PASSWORD })));
    for read in &seen[1..] {
        assert_eq!(read.token.as_deref(), Some("hvs.issued-1"), "{read:?}");
    }
    assert_eq!(fake.count("/v1/auth/token/lookup-self"), 0);
}

#[tokio::test]
async fn an_ldap_login_uses_its_mount() {
    let fake = seeded().await;
    fake.add_user("ldap", "alice", PASSWORD, 3600);
    fake.add_user("corp-ldap", "alice", PASSWORD, 3600);

    let source = bao(login_settings(&fake, BaoAuth::Ldap));
    source.fetch(&login("alice", PASSWORD)).await.unwrap();
    let source = bao(OpenBaoSettings {
        auth_mount: Some("corp-ldap".into()),
        ..login_settings(&fake, BaoAuth::Ldap)
    });
    source.fetch(&login("alice", PASSWORD)).await.unwrap();

    let logins: Vec<String> = fake
        .calls()
        .into_iter()
        .filter(|c| c.starts_with("POST"))
        .collect();
    assert_eq!(
        logins,
        [
            "POST /v1/auth/ldap/login/alice",
            "POST /v1/auth/corp-ldap/login/alice",
        ]
    );
}

#[tokio::test]
async fn a_token_close_to_its_expiry_is_not_kept() {
    let fake = seeded().await;
    fake.add_user("userpass", "bob", PASSWORD, 5);
    fake.add_user("userpass", "carol", PASSWORD, 0);
    let source = bao(login_settings(&fake, BaoAuth::Userpass));

    source.fetch(&login("bob", PASSWORD)).await.unwrap();
    source.fetch(&login("bob", PASSWORD)).await.unwrap();
    assert_eq!(fake.logins(), 2, "a 5-second lease is too close to its end");

    fake.clear_requests();
    source.fetch(&login("carol", PASSWORD)).await.unwrap();
    source.fetch(&login("carol", PASSWORD)).await.unwrap();
    assert_eq!(fake.logins(), 1, "a lease of 0 does not expire");
}

#[tokio::test]
async fn other_sign_in_details_log_in_again() {
    let fake = seeded().await;
    fake.add_user("userpass", "bob", PASSWORD, 3600);
    let source = bao(login_settings(&fake, BaoAuth::Userpass));
    source.fetch(&login("alice", PASSWORD)).await.unwrap();
    source.fetch(&login("bob", PASSWORD)).await.unwrap();
    assert_eq!(fake.logins(), 2);
}

#[tokio::test]
async fn a_403_on_the_kept_token_logs_in_again_once() {
    let fake = seeded().await;
    let source = bao(login_settings(&fake, BaoAuth::Userpass));
    source.fetch(&login("alice", PASSWORD)).await.unwrap();

    fake.fail_next(Failure::Status(403), 1);
    let fetched = source.fetch(&login("alice", PASSWORD)).await.unwrap();
    assert_eq!(names(&fetched), ["API_KEY", "LOG_LEVEL"]);
    assert_eq!(fake.logins(), 2, "{:?}", fake.calls());

    // The new token is kept.
    source.fetch(&login("alice", PASSWORD)).await.unwrap();
    assert_eq!(fake.logins(), 2);

    // A token the source no longer accepts works the same way.
    fake.revoke_all_tokens();
    source.fetch(&login("alice", PASSWORD)).await.unwrap();
    assert_eq!(fake.logins(), 3);
}

#[tokio::test]
async fn two_403s_in_a_row_are_denied() {
    let fake = seeded().await;
    let source = bao(login_settings(&fake, BaoAuth::Userpass));
    source.fetch(&login("alice", PASSWORD)).await.unwrap();

    fake.fail_next(Failure::Status(403), 2);
    let e = source.fetch(&login("alice", PASSWORD)).await.unwrap_err();
    assert!(matches!(e, SourceError::Denied(_)), "{e:?}");
    assert!(e.to_string().contains("HTTP 403"), "{e}");
    assert_clean(&e);
    assert_eq!(fake.logins(), 2);

    // Nothing refused is kept: the next fetch logs in again.
    source.fetch(&login("alice", PASSWORD)).await.unwrap();
    assert_eq!(fake.logins(), 3);
}

#[tokio::test]
async fn a_403_right_after_a_login_is_denied_without_another() {
    let fake = seeded().await;
    let source = bao(login_settings(&fake, BaoAuth::Userpass));
    fake.fail_next(Failure::Status(403), 1);
    let e = source.fetch(&login("alice", PASSWORD)).await.unwrap_err();
    assert!(matches!(e, SourceError::Denied(_)), "{e:?}");
    assert_eq!(fake.logins(), 1);
}

#[tokio::test]
async fn a_bad_password_is_denied() {
    let fake = seeded().await;
    let source = bao(login_settings(&fake, BaoAuth::Userpass));
    for creds in [login("alice", WRONG_PASSWORD), login("nobody", PASSWORD)] {
        let e = source.fetch(&creds).await.unwrap_err();
        assert!(matches!(e, SourceError::Denied(_)), "{e:?}");
        assert!(
            e.to_string()
                .contains("refused the username or password (HTTP 400)"),
            "{e}"
        );
        assert_clean(&e);
    }
    assert_eq!(fake.calls().len(), 2, "no read follows a failed login");
}

#[tokio::test]
async fn a_login_400_is_denied_only_when_it_says_the_details_were_refused() {
    let fake = seeded().await;
    let source = bao(login_settings(&fake, BaoAuth::Ldap));
    for message in [
        "Invalid Credentials",
        "ldap operation failed: failed to bind as user",
    ] {
        fake.fail_logins(400, message);
        let e = source.fetch(&login("alice", PASSWORD)).await.unwrap_err();
        assert!(matches!(e, SourceError::Denied(_)), "{message}: {e:?}");
    }
    for message in ["ldap operation failed: unable to connect", "something else"] {
        fake.fail_logins(400, message);
        let e = source.fetch(&login("alice", PASSWORD)).await.unwrap_err();
        assert_eq!(
            e,
            SourceError::Unreachable(format!("{} is not available (HTTP 400)", fake.address()))
        );
        assert_clean(&e);
    }
}

#[tokio::test]
async fn a_huge_lease_does_not_panic() {
    let fake = seeded().await;
    fake.add_user("userpass", "dave", PASSWORD, u64::MAX);
    let source = bao(login_settings(&fake, BaoAuth::Userpass));
    source.fetch(&login("dave", PASSWORD)).await.unwrap();
    source.fetch(&login("dave", PASSWORD)).await.unwrap();
    assert_eq!(fake.logins(), 1);
}

#[tokio::test]
async fn an_answer_over_1_mib_is_too_large() {
    let fake = seeded().await;
    for failure in [Failure::TooLarge, Failure::TooLargeChunked] {
        fake.fail_next(failure, 1);
        let why = invalid(bao(settings(&fake)).fetch(&token()).await.unwrap_err());
        assert!(why.ends_with("is too large"), "{failure:?}: {why}");
    }
}

#[tokio::test]
async fn a_bad_token_is_denied() {
    let fake = seeded().await;
    fake.revoke_token(TOKEN);
    let e = bao(settings(&fake)).fetch(&token()).await.unwrap_err();
    assert!(matches!(e, SourceError::Denied(_)), "{e:?}");
    assert_eq!(
        e.to_string(),
        format!(
            "{} refused access to secret/dots/env (HTTP 403)",
            fake.address()
        )
    );
    assert_clean(&e);
    assert_eq!(fake.calls(), ["GET /v1/secret/data/dots/env"]);
}

#[tokio::test]
async fn the_wrong_kind_of_sign_in_details_is_invalid() {
    let fake = seeded().await;
    let by_token = bao(settings(&fake));
    for creds in [
        login("alice", PASSWORD),
        SourceCreds::Token { token: z("") },
    ] {
        let why = invalid(by_token.fetch(&creds).await.unwrap_err());
        assert_eq!(
            why,
            "this org's OpenBao signs in with a token, so it needs a token"
        );
    }
    let by_login = bao(login_settings(&fake, BaoAuth::Ldap));
    for creds in [
        token(),
        login("alice", ""),
        login("", PASSWORD),
        SourceCreds::Age {
            identity: z("AGE-SECRET-KEY-1"),
        },
    ] {
        let why = invalid(by_login.fetch(&creds).await.unwrap_err());
        assert_eq!(
            why,
            "this org's OpenBao signs in with ldap, so it needs a username and password"
        );
    }
    assert!(fake.calls().is_empty(), "{:?}", fake.calls());
}

// Renewing a token.

#[tokio::test]
async fn a_renewable_token_is_renewed_once_an_hour() {
    let fake = seeded().await;
    fake.allow_token(TOKEN, true);
    let source = bao(settings(&fake));
    source.fetch(&token()).await.unwrap();
    source.fetch(&token()).await.unwrap();

    assert_eq!(fake.count("/v1/auth/token/lookup-self"), 1);
    assert_eq!(fake.renewals(), 1);
    let renew = fake
        .requests()
        .into_iter()
        .find(|s| s.path == "/v1/auth/token/renew-self")
        .unwrap();
    assert_eq!(renew.method, "POST");
    assert_eq!(renew.token.as_deref(), Some(TOKEN));
}

#[tokio::test]
async fn a_token_that_is_not_renewable_is_not_renewed() {
    let fake = seeded().await;
    let source = bao(settings(&fake));
    source.fetch(&token()).await.unwrap();
    source.fetch(&token()).await.unwrap();
    assert_eq!(fake.count("/v1/auth/token/lookup-self"), 1);
    assert_eq!(fake.renewals(), 0);
}

#[tokio::test]
async fn a_failed_renewal_is_not_an_error() {
    let fake = seeded().await;
    fake.allow_token(TOKEN, true);
    fake.refuse_renewals();
    let fetched = bao(settings(&fake)).fetch(&token()).await.unwrap();
    assert_eq!(names(&fetched), ["API_KEY", "LOG_LEVEL"]);
    assert_eq!(fake.renewals(), 1);
}

// What each failure means.

#[tokio::test]
async fn sealed_a_server_error_or_rate_limiting_is_unreachable() {
    let fake = seeded().await;
    fake.fail_next(Failure::Status(503), 1);
    let e = bao(settings(&fake)).fetch(&token()).await.unwrap_err();
    assert_eq!(
        e,
        SourceError::Unreachable(format!(
            "{} is sealed or not ready (HTTP 503)",
            fake.address()
        ))
    );

    fake.fail_next(Failure::Status(502), 1);
    let e = bao(settings(&fake)).fetch(&token()).await.unwrap_err();
    assert!(matches!(e, SourceError::Unreachable(_)), "{e:?}");
    assert!(e.to_string().contains("HTTP 502"), "{e}");

    fake.fail_next(Failure::Status(500), 1);
    let e = bao(settings(&fake)).fetch(&token()).await.unwrap_err();
    assert_eq!(
        e,
        SourceError::Unreachable(format!("{} is not available (HTTP 500)", fake.address()))
    );

    fake.fail_next(Failure::Status(429), 1);
    let e = bao(settings(&fake)).fetch(&token()).await.unwrap_err();
    assert_eq!(
        e,
        SourceError::Unreachable(format!(
            "{} is rate limiting Dots (HTTP 429)",
            fake.address()
        ))
    );
}

#[tokio::test]
async fn a_refused_connection_is_unreachable() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let source = OpenBao::new(
        OpenBaoSettings {
            address: format!("http://127.0.0.1:{port}"),
            mount: "secret".into(),
            path: "dots".into(),
            auth: BaoAuth::Token,
            auth_mount: None,
            namespace: None,
        },
        http_client(),
    );
    let e = source.fetch(&token()).await.unwrap_err();
    assert!(matches!(e, SourceError::Unreachable(_)), "{e:?}");
    assert!(
        e.to_string()
            .starts_with(&format!("cannot reach http://127.0.0.1:{port}")),
        "{e}"
    );
    assert_clean(&e);
}

#[tokio::test]
async fn a_source_that_hangs_is_unreachable() {
    let fake = seeded().await;
    fake.fail_next(Failure::Hang, 1);
    let source = OpenBao::new(
        settings(&fake),
        http_client_with_timeout(Duration::from_millis(300)),
    );
    let started = Instant::now();
    let e = source.fetch(&token()).await.unwrap_err();
    assert_eq!(
        e,
        SourceError::Unreachable(format!("{} did not answer in time", fake.address()))
    );
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[tokio::test]
async fn no_entries_is_invalid() {
    let fake = seeded().await;
    fake.remove("secret", "dots/env");
    let e = bao(settings(&fake)).fetch(&token()).await.unwrap_err();
    assert_eq!(
        e,
        SourceError::Invalid("no secrets at secret/dots/env".into())
    );
}

#[tokio::test]
async fn no_policy_is_invalid() {
    let fake = seeded().await;
    fake.remove("secret", "dots/policy");
    let e = bao(settings(&fake)).fetch(&token()).await.unwrap_err();
    assert_eq!(
        e,
        SourceError::Invalid("no DOTS_POLICY at secret/dots/policy".into())
    );

    fake.put("secret", "dots/policy", json!({ "OTHER": POLICY }));
    let e = bao(settings(&fake)).fetch(&token()).await.unwrap_err();
    assert_eq!(
        e,
        SourceError::Invalid("no DOTS_POLICY at secret/dots/policy".into())
    );

    fake.put("secret", "dots/policy", json!({ "DOTS_POLICY": NUMBER }));
    let e = bao(settings(&fake)).fetch(&token()).await.unwrap_err();
    assert_eq!(e, SourceError::Invalid("DOTS_POLICY is not text".into()));
    assert_clean(&e);
}

/// A signed roster for `acme-eng` with one owner.
fn signed_roster() -> String {
    let org = keys();
    let owner = keys().pk.to_base64();
    let text = roster(
        "acme-eng",
        3,
        json!([{
            "id": "01A", "name": "Asha Rao", "email": "asha@acme.com", "role": "owner",
            "keys": [owner], "workspaces": {"default": "member"},
            "added_at": "2026-10-28T09:00:00Z"
        }]),
    );
    parse_roster(&text, "acme-eng").unwrap();
    sign_roster(&org, &text)
}

#[tokio::test]
async fn reads_roster_when_present() {
    let fake = seeded().await;
    let signed = signed_roster();
    fake.put_roster("secret", "dots", &signed);
    let fetched = bao(settings(&fake)).fetch(&token()).await.unwrap();
    assert_eq!(fetched.roster, Some(signed));
    assert_eq!(fetched.policy, POLICY);
    assert!(fake
        .calls()
        .contains(&"GET /v1/secret/data/dots/roster".to_string()));
}

#[tokio::test]
async fn no_roster_is_none() {
    let fake = seeded().await;
    // A 404 on `<path>/roster`.
    let fetched = bao(settings(&fake)).fetch(&token()).await.unwrap();
    assert_eq!(fetched.roster, None);
    assert_eq!(fetched.roster_note, None);
    // A secret there with no `DOTS_ROSTER` key.
    fake.put("secret", "dots/roster", json!({ "OTHER": "x" }));
    let fetched = bao(settings(&fake)).fetch(&token()).await.unwrap();
    assert_eq!(fetched.roster, None);
}

/// A roster the token may not read is no roster: a phase 1 ACL grants only `env` and
/// `policy`, and OpenBao refuses `roster` with a 403. The fetch keeps its entries and policy,
/// with a note saying why there is no roster. Server trouble and timeouts on the roster still
/// fail the fetch as `Unreachable`: they are never taken for no roster. The reads go entries,
/// policy, roster.
#[tokio::test]
async fn a_refused_roster_read_is_no_roster_and_other_failures_fail_the_fetch() {
    let fake = seeded().await;
    fake.put_roster("secret", "dots", &signed_roster());
    let source = OpenBao::new(
        settings(&fake),
        http_client_with_timeout(Duration::from_millis(300)),
    );
    let on_roster = |failure: Failure| {
        fake.clear_requests();
        fake.fail_next(Failure::Pass, 2);
        fake.fail_next(failure, 1);
    };

    for code in [401, 403] {
        on_roster(Failure::Status(code));
        let fetched = source.fetch(&token()).await.unwrap();
        assert_eq!(fetched.roster, None);
        assert_eq!(
            fetched.roster_note,
            Some(format!(
                "can't read DOTS_ROSTER at secret/dots/roster (HTTP {code})"
            ))
        );
        assert_eq!(names(&fetched), ["API_KEY", "LOG_LEVEL"]);
        assert_eq!(fetched.policy, POLICY);
        assert!(fake
            .calls()
            .contains(&"GET /v1/secret/data/dots/roster".to_string()));
    }

    on_roster(Failure::Status(500));
    let e = source.fetch(&token()).await.unwrap_err();
    assert!(matches!(e, SourceError::Unreachable(_)), "{e:?}");
    assert!(e.to_string().contains("HTTP 500"), "{e}");

    on_roster(Failure::Hang);
    let e = source.fetch(&token()).await.unwrap_err();
    assert_eq!(
        e,
        SourceError::Unreachable(format!("{} did not answer in time", fake.address()))
    );
    assert_clean(&e);

    // With nothing queued, the same fetch works and has the roster, with no note.
    let fetched = source.fetch(&token()).await.unwrap();
    assert!(fetched.roster.is_some());
    assert_eq!(fetched.roster_note, None);
}

/// A token refused everywhere is still refused: only the roster's read is taken for no roster.
#[tokio::test]
async fn a_refused_entries_read_still_fails_the_fetch() {
    let fake = seeded().await;
    fake.deny("secret", "dots/env");
    let e = bao(settings(&fake)).fetch(&token()).await.unwrap_err();
    assert!(matches!(e, SourceError::Denied(_)), "{e:?}");
}

#[tokio::test]
async fn a_roster_that_is_empty_or_not_text_is_invalid() {
    let fake = seeded().await;
    let source = bao(settings(&fake));
    for value in [json!(""), json!(NUMBER), json!({ "a": 1 })] {
        fake.put("secret", "dots/roster", json!({ "DOTS_ROSTER": value }));
        let e = source.fetch(&token()).await.unwrap_err();
        assert_eq!(e, SourceError::Invalid("DOTS_ROSTER is not text".into()));
    }
}

#[tokio::test]
async fn a_roster_key_in_the_entries_is_not_an_entry() {
    let fake = seeded().await;
    fake.put_env(
        "secret",
        "dots",
        json!({ "API_KEY": SECRET, "DOTS_ROSTER": "stray" }),
    );
    let fetched = bao(settings(&fake)).fetch(&token()).await.unwrap();
    assert_eq!(names(&fetched), ["API_KEY"]);
    assert_eq!(fetched.roster, None);
}

#[tokio::test]
async fn a_value_that_is_not_text_is_invalid_by_name() {
    let fake = seeded().await;
    let source = bao(settings(&fake));
    let cases: [(&str, Value); 5] = [
        ("PORT", json!(NUMBER)),
        ("NESTED", json!({ "inner": NESTED })),
        ("LIST", json!([NESTED, NUMBER])),
        ("FLAG", json!(true)),
        ("NOTHING", Value::Null),
    ];
    for (name, value) in cases {
        let mut env = json!({ "API_KEY": SECRET });
        env[name] = value;
        fake.put_env("secret", "dots", env);
        let e = source.fetch(&token()).await.unwrap_err();
        assert_eq!(e, SourceError::Invalid(format!("{name} is not text")));
        assert_clean(&e);
    }
}

#[tokio::test]
async fn bad_names_and_the_policy_key_are_not_entries() {
    let fake = seeded().await;
    fake.put_env(
        "secret",
        "dots",
        json!({
            "API_KEY": SECRET,
            "1BAD": "x",
            "has-dash": "y",
            "": "z",
            "bad name": NUMBER,
            "DOTS_POLICY": POLICY,
            "dots_policy": POLICY,
        }),
    );
    let fetched = bao(settings(&fake)).fetch(&token()).await.unwrap();
    assert_eq!(names(&fetched), ["API_KEY"]);
}

#[tokio::test]
async fn two_names_differing_only_in_case_are_invalid() {
    let fake = seeded().await;
    fake.put_env(
        "secret",
        "dots",
        json!({ "Api_Key": SECRET, "API_KEY": SECRET }),
    );
    let e = bao(settings(&fake)).fetch(&token()).await.unwrap_err();
    assert_eq!(
        e,
        SourceError::Invalid("API_KEY and Api_Key are the same name in a different case".into())
    );
}

#[tokio::test]
async fn another_status_or_an_unreadable_answer_is_invalid() {
    let fake = seeded().await;
    fake.fail_next(Failure::Status(400), 1);
    let e = bao(settings(&fake)).fetch(&token()).await.unwrap_err();
    assert_eq!(
        e,
        SourceError::Invalid(format!(
            "{} answered HTTP 400 for secret/dots/env",
            fake.address()
        ))
    );

    fake.fail_next(Failure::NotJson, 1);
    let why = invalid(bao(settings(&fake)).fetch(&token()).await.unwrap_err());
    assert!(
        why.starts_with(&format!(
            "the answer from {} for secret/dots/env is not readable",
            fake.address()
        )),
        "{why}"
    );
}

#[tokio::test]
async fn a_redirect_is_not_followed() {
    let fake = seeded().await;
    fake.fail_next(Failure::Status(307), 1);
    let why = invalid(bao(settings(&fake)).fetch(&token()).await.unwrap_err());
    assert!(why.contains("HTTP 307"), "{why}");
    assert_eq!(fake.calls(), ["GET /v1/secret/data/dots/env"]);
}

#[tokio::test]
async fn a_path_leaving_its_mount_is_refused_before_any_request() {
    let fake = seeded().await;
    let paths = [
        "../x", "dots/..", "a/./b", "%2e%2e/x", "%2E.", ".%2e", "", "/", "//", "a\nb",
    ];
    for path in paths {
        let source = bao(OpenBaoSettings {
            path: path.into(),
            ..settings(&fake)
        });
        let why = invalid(source.fetch(&token()).await.unwrap_err());
        assert!(why.starts_with("the source's path "), "{path:?}: {why}");
    }
    let source = bao(OpenBaoSettings {
        mount: "..".into(),
        ..settings(&fake)
    });
    let why = invalid(source.fetch(&token()).await.unwrap_err());
    assert_eq!(
        why,
        "the source's mount '..' must not hold a '.' or '..' segment"
    );

    let source = bao(OpenBaoSettings {
        auth_mount: Some("userpass/../token".into()),
        ..login_settings(&fake, BaoAuth::Userpass)
    });
    let why = invalid(source.fetch(&login("alice", PASSWORD)).await.unwrap_err());
    assert!(why.starts_with("the source's auth_mount "), "{why}");

    let source = bao(login_settings(&fake, BaoAuth::Userpass));
    for username in ["..", ".", "%2e%2E"] {
        let why = invalid(source.fetch(&login(username, PASSWORD)).await.unwrap_err());
        assert!(why.starts_with("the username cannot be"), "{why}");
    }
    assert!(fake.calls().is_empty(), "{:?}", fake.calls());
}

// The source for a join code's settings.

#[tokio::test]
async fn source_for_gives_openbao() {
    let fake = seeded().await;
    let source = source_for(&SourceSettings::OpenBao(settings(&fake)), http_client());
    let fetched = source.fetch(&token()).await.unwrap();
    assert_eq!(fetched.policy, POLICY);
}

#[test]
fn the_http_client_builds_with_the_public_and_the_machine_roots() {
    // Building it loads the Windows certificate store as well as the public roots, and would
    // panic if that failed.
    let _ = http_client();
    let _ = http_client_with_timeout(Duration::from_secs(1));
}

// The sign-in details in the secret store.

fn every_kind() -> Vec<SourceCreds> {
    vec![
        token(),
        login("alice", PASSWORD),
        SourceCreds::Machine {
            client_id: z("client-id-0c1d"),
            client_secret: z(SECRET),
        },
        SourceCreds::Age {
            identity: z("AGE-SECRET-KEY-1QYQSZQGPQYQSZQGPQYQSZQGPQYQSZQGP"),
        },
    ]
}

#[test]
fn sign_in_details_round_trip_through_the_secret_store() {
    let secrets = MemorySecrets::new();
    assert_eq!(SourceCreds::store_key("acme-eng"), "org/acme-eng/source");
    assert_eq!(SourceCreds::load(&secrets, "acme-eng").unwrap(), None);

    for creds in every_kind() {
        creds.save(&secrets, "acme-eng").unwrap();
        assert_eq!(
            SourceCreds::load(&secrets, "acme-eng").unwrap(),
            Some(creds)
        );
    }
    assert_eq!(secrets.keys("org/").unwrap(), ["org/acme-eng/source"]);

    SourceCreds::forget(&secrets, "acme-eng").unwrap();
    assert_eq!(SourceCreds::load(&secrets, "acme-eng").unwrap(), None);
    SourceCreds::forget(&secrets, "acme-eng").unwrap();
}

#[test]
fn sign_in_details_are_tagged_json() {
    let secrets = MemorySecrets::new();
    login("alice", PASSWORD).save(&secrets, "acme-eng").unwrap();
    let stored = secrets.get("org/acme-eng/source").unwrap().unwrap();
    let json: Value = serde_json::from_str(&stored).unwrap();
    assert_eq!(
        json,
        json!({ "kind": "login", "username": "alice", "password": PASSWORD })
    );

    let kinds: Vec<&str> = every_kind().iter().map(SourceCreds::kind).collect();
    assert_eq!(kinds, ["token", "login", "machine", "age"]);
    for creds in every_kind() {
        let json = serde_json::to_value(&creds).unwrap();
        assert_eq!(json["kind"], creds.kind());
    }
}

#[test]
fn sign_in_details_debug_shows_the_kind_only() {
    for creds in every_kind() {
        let debug = format!("{creds:?}");
        assert_eq!(debug, format!("SourceCreds({:?})", creds.kind()));
    }
}

#[test]
fn sign_in_details_too_long_to_keep_are_refused() {
    let secrets = MemorySecrets::new();
    // `{"kind":"token","token":""}` is 27 bytes.
    let fits = "t".repeat(MAX_SECRET_BYTES - 27);
    let creds = SourceCreds::Token { token: z(&fits) };
    creds.save(&secrets, "acme-eng").unwrap();
    assert_eq!(
        SourceCreds::load(&secrets, "acme-eng").unwrap(),
        Some(creds)
    );
    SourceCreds::forget(&secrets, "acme-eng").unwrap();

    let long = format!("{SECRET}{}", "t".repeat(MAX_SECRET_BYTES));
    let e = SourceCreds::Token { token: z(&long) }
        .save(&secrets, "acme-eng")
        .unwrap_err();
    assert!(matches!(e, Error::Invalid(_)), "{e:?}");
    assert_eq!(
        e.to_string(),
        "invalid input: these sign-in details are too long to keep: at most 2560 bytes in all"
    );
    assert!(secrets.keys("").unwrap().is_empty());
}

#[test]
fn damaged_sign_in_details_are_an_error_without_their_text() {
    let secrets = MemorySecrets::new();
    let damaged = [
        format!(r#"{{"kind":"token","token":{NUMBER}}}"#),
        format!(r#"{{"kind":"bearer","token":"{SECRET}"}}"#),
        format!(r#"{{"kind":"token","token":"{SECRET}","extra":"{NESTED}"}}"#),
        format!("not json {SECRET}"),
    ];
    for text in damaged {
        secrets.put("org/acme-eng/source", &text).unwrap();
        let e = SourceCreds::load(&secrets, "acme-eng").unwrap_err();
        let shown = format!("{e} {e:?}");
        assert!(shown.contains("org 'acme-eng'"), "{shown}");
        for value in VALUES {
            assert!(!shown.contains(value), "{shown:?} holds {value:?}");
        }
    }
}

#[test]
fn sign_in_details_need_a_good_org_id() {
    let secrets = MemorySecrets::new();
    for org_id in ["../x", "Acme", "", "a/b"] {
        assert!(token().save(&secrets, org_id).is_err(), "{org_id:?}");
        assert!(SourceCreds::load(&secrets, org_id).is_err(), "{org_id:?}");
        assert!(SourceCreds::forget(&secrets, org_id).is_err(), "{org_id:?}");
    }
    assert!(secrets.keys("").unwrap().is_empty());
}
