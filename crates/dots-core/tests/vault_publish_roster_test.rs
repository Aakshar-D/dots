//! Publishing the signed member list to a source (members spec §6.5), against the fakes:
//! OpenBao and Infisical take the write, SOPS can't, and each failure maps to the right
//! `PublishError`. No error ever holds the signed text or a token.

mod common;

use common::fake_infisical::{Failure as InfFailure, FakeInfisical};
use common::fake_openbao::{Failure, FakeOpenBao};
use dots_core::vault::org::{
    http_client, BaoAuth, Infisical, InfisicalSettings, OpenBao, OpenBaoSettings, PublishError,
    Sops, SopsSettings, Source, SourceCreds,
};
use serde_json::json;
use zeroize::Zeroizing;

const TOKEN: &str = "hvs.tok-4d9e1b7a";
const CLIENT_ID: &str = "ci-6f1a2b70";
const CLIENT_SECRET: &str = "cs-9d4e7c21";
const POLICY: &str = "signed-policy-71e2c0";
const ROSTER: &str = "signed-roster-5b83d1";
const ROSTER_2: &str = "signed-roster-second-c94e07";

fn z(s: &str) -> Zeroizing<String> {
    Zeroizing::new(s.to_string())
}

fn token() -> SourceCreds {
    SourceCreds::Token { token: z(TOKEN) }
}

fn machine() -> SourceCreds {
    SourceCreds::Machine {
        client_id: z(CLIENT_ID),
        client_secret: z(CLIENT_SECRET),
    }
}

fn assert_clean(e: &PublishError) {
    let text = format!("{e} {e:?}");
    for value in [TOKEN, CLIENT_SECRET, ROSTER, ROSTER_2, "inf.issued"] {
        assert!(!text.contains(value), "{text:?} holds {value:?}");
    }
}

async fn bao_fake() -> (FakeOpenBao, OpenBao) {
    let fake = FakeOpenBao::start().await;
    fake.put_env("secret", "dots", json!({ "API_KEY": "v" }));
    fake.put_policy("secret", "dots", POLICY);
    fake.allow_token(TOKEN, false);
    let bao = OpenBao::new(
        OpenBaoSettings {
            address: fake.address(),
            mount: "secret".into(),
            path: "dots".into(),
            auth: BaoAuth::Token,
            auth_mount: None,
            namespace: Some("team".into()),
        },
        http_client(),
    );
    (fake, bao)
}

async fn inf_fake() -> (FakeInfisical, Infisical) {
    let fake = FakeInfisical::start().await;
    fake.put_secret("proj", "dev", "/dots", "API_KEY", json!("v"));
    fake.put_policy("proj", "dev", "/dots", POLICY);
    fake.add_identity(CLIENT_ID, CLIENT_SECRET, 3600);
    let inf = Infisical::new(
        InfisicalSettings {
            site_url: fake.address(),
            project_id: "proj".into(),
            environment: "dev".into(),
            secret_path: "/dots".into(),
        },
        http_client(),
    );
    (fake, inf)
}

#[tokio::test]
async fn openbao_publish_then_fetch_reads_it() {
    let (fake, bao) = bao_fake().await;
    bao.publish_roster(&token(), ROSTER).await.unwrap();
    let fetched = bao.fetch(&token()).await.unwrap();
    assert_eq!(fetched.roster.as_deref(), Some(ROSTER));

    let write = fake
        .requests()
        .into_iter()
        .find(|s| s.method == "POST")
        .unwrap();
    assert_eq!(write.path, "/v1/secret/data/dots/roster");
    assert_eq!(write.token.as_deref(), Some(TOKEN));
    assert_eq!(write.namespace.as_deref(), Some("team"));
    assert_eq!(
        write.body,
        Some(json!({ "data": { "DOTS_ROSTER": ROSTER } }))
    );

    // A second publish replaces the first.
    bao.publish_roster(&token(), ROSTER_2).await.unwrap();
    let fetched = bao.fetch(&token()).await.unwrap();
    assert_eq!(fetched.roster.as_deref(), Some(ROSTER_2));
}

#[tokio::test]
async fn openbao_publish_denied_is_denied() {
    let (fake, bao) = bao_fake().await;
    fake.deny_writes("secret", "dots/roster");
    let e = bao.publish_roster(&token(), ROSTER).await.unwrap_err();
    assert_clean(&e);
    match e {
        PublishError::Denied(why) => assert!(why.contains("secret/data/dots/roster"), "{why}"),
        other => panic!("expected Denied, got {other:?}"),
    }
}

#[tokio::test]
async fn infisical_publish_creates_then_updates() {
    let (fake, inf) = inf_fake().await;
    inf.publish_roster(&machine(), ROSTER).await.unwrap();
    let fetched = inf.fetch(&machine()).await.unwrap();
    assert_eq!(fetched.roster.as_deref(), Some(ROSTER));

    let calls = fake.calls();
    let writes: Vec<&String> = calls.iter().filter(|c| !c.starts_with("GET")).collect();
    assert_eq!(
        writes,
        [
            "POST /api/v1/auth/universal-auth/login",
            "PATCH /api/v4/secrets/DOTS_ROSTER",
            "POST /api/v4/secrets/DOTS_ROSTER",
        ]
    );
    let post = fake
        .requests()
        .into_iter()
        .rfind(|s| s.method == "POST" && s.path.ends_with("/DOTS_ROSTER"))
        .unwrap();
    assert_eq!(
        post.body,
        Some(json!({
            "projectId": "proj",
            "environment": "dev",
            "secretPath": "/dots/policy",
            "secretValue": ROSTER,
            "type": "shared",
        }))
    );

    // Now it exists: the next publish is a PATCH alone.
    fake.clear_requests();
    inf.publish_roster(&machine(), ROSTER_2).await.unwrap();
    assert_eq!(fake.calls(), ["PATCH /api/v4/secrets/DOTS_ROSTER"]);
    let fetched = inf.fetch(&machine()).await.unwrap();
    assert_eq!(fetched.roster.as_deref(), Some(ROSTER_2));
}

#[tokio::test]
async fn infisical_publish_denied_is_denied() {
    let (fake, inf) = inf_fake().await;
    fake.deny_writes();
    let e = inf.publish_roster(&machine(), ROSTER).await.unwrap_err();
    assert_clean(&e);
    assert!(matches!(e, PublishError::Denied(_)), "{e:?}");
}

#[tokio::test]
async fn sops_publish_is_unsupported() {
    let sops = Sops::new(
        SopsSettings {
            location: "C:/nowhere/org.sops.json".into(),
        },
        http_client(),
    );
    let creds = SourceCreds::Age {
        identity: z("AGE-SECRET-KEY-1QQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQ"),
    };
    let e = sops.publish_roster(&creds, ROSTER).await.unwrap_err();
    assert_eq!(e, PublishError::Unsupported);
    assert_eq!(e.to_string(), "this source can't be written from Dots");
}

#[tokio::test]
async fn unreachable_is_unreachable() {
    let (fake, bao) = bao_fake().await;
    fake.fail_next(Failure::Status(503), 1);
    let e = bao.publish_roster(&token(), ROSTER).await.unwrap_err();
    assert_clean(&e);
    assert!(matches!(e, PublishError::Unreachable(_)), "{e:?}");

    let (fake, inf) = inf_fake().await;
    inf.fetch(&machine()).await.unwrap();
    fake.fail_next_write(InfFailure::Status(503), 1);
    let e = inf.publish_roster(&machine(), ROSTER).await.unwrap_err();
    assert!(matches!(e, PublishError::Unreachable(_)), "{e:?}");

    // Nothing listening at all.
    let (fake, bao) = bao_fake().await;
    drop(fake);
    let e = bao.publish_roster(&token(), ROSTER).await.unwrap_err();
    assert!(matches!(e, PublishError::Unreachable(_)), "{e:?}");
}

#[tokio::test]
async fn an_oddly_answered_write_is_invalid() {
    let (fake, bao) = bao_fake().await;
    fake.fail_next(Failure::Status(400), 1);
    let e = bao.publish_roster(&token(), ROSTER).await.unwrap_err();
    assert!(matches!(e, PublishError::Invalid(_)), "{e:?}");
}
