//! Invite offers in join codes (accounts spec, phase 3): a join code of version 2 carries an
//! offer of a role; a plain code stays version 1.

mod common;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use common::orgs::{code, openbao, OrgsHarness};
use dots_core::accounts::Role;
use dots_core::vault::org::{parse_join_code, InviteOffer, JoinCode};
use serde_json::Value;

fn decode(text: &str) -> Value {
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(text).unwrap()).unwrap()
}

fn offer() -> InviteOffer {
    InviteOffer {
        id: "inv-1".into(),
        role: Role::Member,
        expires_at: "2026-11-05T09:00:00Z".into(),
    }
}

/// A plain code with `invite` set and `v` as given.
fn with_invite(h: &OrgsHarness, v: u32, invite: Option<InviteOffer>) -> String {
    let plain = parse_join_code(&h.acme_code()).unwrap();
    let json = serde_json::to_value(JoinCode { v, invite, ..plain }).unwrap();
    URL_SAFE_NO_PAD.encode(json.to_string())
}

#[tokio::test]
async fn plain_code_still_encodes_as_v1() {
    let h = OrgsHarness::new().await;
    let text = code(
        "acme-eng",
        "Acme",
        openbao(&h.bao.address(), "dots"),
        &h.keys,
    );
    let json = decode(&text);
    assert_eq!(json["v"], 1);
    assert!(json.get("invite").is_none());
    assert_eq!(parse_join_code(&text).unwrap().invite, None);
}

#[tokio::test]
async fn invite_code_round_trips_as_v2() {
    let h = OrgsHarness::new().await;
    let plain = parse_join_code(&h.acme_code()).unwrap();
    let with = JoinCode {
        invite: Some(offer()),
        ..plain
    };
    let text = with.encode();
    let json = decode(&text);
    assert_eq!(json["v"], 2);
    assert_eq!(json["invite"]["role"], "member");
    let back = parse_join_code(&text).unwrap();
    assert_eq!(back.v, 2);
    assert_eq!(back.invite, Some(offer()));
}

#[tokio::test]
async fn v1_with_invite_is_refused() {
    let h = OrgsHarness::new().await;
    let text = with_invite(&h, 1, Some(offer()));
    let e = parse_join_code(&text).unwrap_err().to_string();
    assert!(
        e.contains("a version 1 join code can't carry an invite"),
        "{e}"
    );
}

#[tokio::test]
async fn v3_is_refused() {
    let h = OrgsHarness::new().await;
    let text = with_invite(&h, 3, Some(offer()));
    let e = parse_join_code(&text).unwrap_err().to_string();
    assert!(e.contains("version 3 is not supported"), "{e}");
}

#[tokio::test]
async fn preview_shows_the_offer() {
    let h = OrgsHarness::new().await;
    let plain = parse_join_code(&h.acme_code()).unwrap();
    let text = JoinCode {
        invite: Some(offer()),
        ..plain
    }
    .encode();
    let preview = h.orgs.preview_join(&text).unwrap();
    let invite = preview.invite.unwrap();
    assert_eq!(invite.role, Role::Member);
    assert_eq!(invite.expires_at, "2026-11-05T09:00:00Z");
    assert!(h
        .orgs
        .preview_join(&h.acme_code())
        .unwrap()
        .invite
        .is_none());
}

#[tokio::test]
async fn invite_id_with_a_hidden_character_is_refused() {
    let h = OrgsHarness::new().await;
    let hidden = InviteOffer {
        id: "inv\u{202e}1".into(),
        ..offer()
    };
    let e = parse_join_code(&with_invite(&h, 2, Some(hidden)))
        .unwrap_err()
        .to_string();
    assert_eq!(
        e,
        "invalid input: join code: the invite's id holds a control character"
    );
}

#[tokio::test]
async fn invite_end_that_is_not_a_time_is_refused() {
    let h = OrgsHarness::new().await;
    let vague = InviteOffer {
        expires_at: "next week".into(),
        ..offer()
    };
    let e = parse_join_code(&with_invite(&h, 2, Some(vague)))
        .unwrap_err()
        .to_string();
    assert_eq!(
        e,
        "invalid input: join code: the invite's end isn't a date and time (RFC 3339)"
    );
}
