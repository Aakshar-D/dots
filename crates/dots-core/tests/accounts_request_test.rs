//! Signing with the Dots ID, and the signed request codes a new person sends to an admin.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{TimeZone, Utc};
use dots_core::accounts::{
    ensure_identity, make_request, parse_request, set_profile, sign_with_identity, AccessRequest,
    RequestError, REQUEST_PREFIX,
};
use dots_core::store::Store;
use dots_core::vault::MemorySecrets;
use dots_core::Error;
use minisign_verify::{PublicKey, Signature};
use serde_json::{json, Value};

async fn setup(profile: bool) -> (tempfile::TempDir, Store, MemorySecrets) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("dots.db")).await.unwrap();
    let secrets = MemorySecrets::new();
    ensure_identity(&store, &secrets).await.unwrap();
    if profile {
        set_profile(&store, "Asha Rao", "asha@acme.test")
            .await
            .unwrap();
    }
    (dir, store, secrets)
}

fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 29, 9, 0, 0).unwrap()
}

/// Wraps request text and a signature as a code.
fn code_of(request: &str, signature: &str) -> String {
    let envelope = json!({"request": request, "signature": signature}).to_string();
    format!("{REQUEST_PREFIX}{}", URL_SAFE_NO_PAD.encode(envelope))
}

fn request_text_of(code: &str) -> (String, String) {
    let bytes = URL_SAFE_NO_PAD
        .decode(&code[REQUEST_PREFIX.len()..])
        .unwrap();
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    (
        v["request"].as_str().unwrap().to_string(),
        v["signature"].as_str().unwrap().to_string(),
    )
}

#[tokio::test]
async fn request_round_trips() {
    let (_d, store, secrets) = setup(true).await;
    let code = make_request(&store, &secrets, "acme-eng", Some("inv1"), None, now())
        .await
        .unwrap();
    assert!(code.starts_with("dotsreq_"));
    let me = dots_core::accounts::identity(&store).await.unwrap();
    let request = parse_request(&code).unwrap();
    assert_eq!(request.org_id, "acme-eng");
    assert_eq!(request.invite_id.as_deref(), Some("inv1"));
    assert_eq!(request.key, me.public_key);
    assert_eq!(request.name, "Asha Rao");
    assert_eq!(request.email, "asha@acme.test");
    assert_eq!(request.created_at, "2026-10-29T09:00:00Z");
    // Padding and line breaks are tolerated.
    let spaced = format!("{}=\n", code.replace("_", "_ "));
    assert_eq!(parse_request(&spaced).unwrap(), request);
}

#[tokio::test]
async fn request_without_invite_has_none() {
    let (_d, store, secrets) = setup(true).await;
    let code = make_request(&store, &secrets, "acme-eng", None, None, now())
        .await
        .unwrap();
    assert_eq!(parse_request(&code).unwrap().invite_id, None);
    assert!(request_text_of(&code).0.contains("\"invite_id\":null"));
}

#[tokio::test]
async fn tampered_request_is_refused() {
    let (_d, store, secrets) = setup(true).await;
    let code = make_request(&store, &secrets, "acme-eng", None, None, now())
        .await
        .unwrap();
    let (text, signature) = request_text_of(&code);
    let tampered = text.replace("asha@acme.test", "evil@acme.test");
    assert_ne!(tampered, text);
    assert_eq!(
        parse_request(&code_of(&tampered, &signature)),
        Err(RequestError::Signature)
    );
}

#[tokio::test]
async fn request_signed_by_another_key_is_refused() {
    let (_d, store, secrets) = setup(true).await;
    let me = dots_core::accounts::identity(&store).await.unwrap();
    let stranger = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    let request = AccessRequest {
        v: 1,
        org_id: "acme-eng".into(),
        invite_id: None,
        key: stranger.pk.to_base64(),
        name: me.name,
        email: me.email,
        age_recipient: None,
        created_at: "2026-10-29T09:00:00Z".into(),
    };
    let text = serde_json::to_string(&request).unwrap();
    let signature = sign_with_identity(&store, &secrets, &text, "dots request acme-eng")
        .await
        .unwrap();
    assert_eq!(
        parse_request(&code_of(&text, &signature)),
        Err(RequestError::Signature)
    );
}

#[test]
fn garbage_is_not_a_request() {
    assert_eq!(parse_request("hello"), Err(RequestError::NotACode));
    assert!(matches!(
        parse_request("dotsreq_!!"),
        Err(RequestError::Unreadable(_))
    ));
}

#[tokio::test]
async fn needs_a_profile() {
    let (_d, store, secrets) = setup(false).await;
    let err = make_request(&store, &secrets, "acme-eng", None, None, now())
        .await
        .unwrap_err();
    match err {
        Error::Invalid(text) => {
            assert_eq!(text, "Add your name and email in Settings › Profile first.")
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
}

#[tokio::test]
async fn sign_with_identity_verifies() {
    let (_d, store, secrets) = setup(false).await;
    let me = dots_core::accounts::identity(&store).await.unwrap();
    let signature = sign_with_identity(&store, &secrets, "hello", "a comment")
        .await
        .unwrap();
    let key = PublicKey::from_base64(&me.public_key).unwrap();
    let signature = Signature::decode(&signature).unwrap();
    key.verify(b"hello", &signature, false).unwrap();
    assert!(key.verify(b"hullo", &signature, false).is_err());
}

#[tokio::test]
async fn signing_without_the_secret_is_an_error() {
    let (_d, store, _secrets) = setup(false).await;
    let empty = MemorySecrets::new();
    assert!(matches!(
        sign_with_identity(&store, &empty, "x", "c").await,
        Err(Error::Other(_))
    ));
}
