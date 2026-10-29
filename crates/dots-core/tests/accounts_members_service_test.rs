//! The Members tab's service (accounts spec §6.5, §7; members spec §6.2): an Admin invites
//! people, approves their signed request codes, changes and removes members, signs the new list
//! with this PC's Dots ID and publishes it, keeping a draft to retry when the source refuses.
//! A PC waiting to be added shows its request code. Everything goes to the fake OpenBao; a SOPS
//! account is a `Scripted` source on a SOPS location, which can't be written from Dots.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use async_trait::async_trait;
use chrono::Duration;
use common::orgs::{
    code, keys, policy, public_key, roster, sign, sign_roster, sops_at, token, OrgsHarness,
    Scripted, ACME, ACME_NAME, ISSUED,
};
use dots_core::accounts::edit::RosterEdit;
use dots_core::accounts::{
    ensure_identity, make_request, parse_request, set_profile, verify_roster, Membership, Role,
};
use dots_core::vault::org::{
    http_client_with_timeout, source_for, EditOutcome, Fetched, InviteState, MembersView,
    OrgSummary, PublishError, Source, SourceCreds, SourceError, SourceSettings,
};
use dots_core::Error;
use serde_json::{json, Value};
use zeroize::Zeroizing;

const CHANGED: &str = "Acme Engineering: the member list changed (now #2) while you were \
                       editing. Check it and make the change again.";
const NOT_SIGNER: &str = "Acme Engineering: your Dots ID isn't one of the account's signing \
                          admins yet. Ask the owner to add it with dots-admin policy admins add.";
/// A policy issued after `ISSUED`.
const LATER: &str = "2026-10-22T08:30:00Z";
const NO_INVITE: &str =
    "This person has no invite from this PC. Confirm to approve them anyway, and pick a role.";
const SOPS_COPY: &str = "Copy the signed list into the SOPS file (see the admin guide).";
const ROSTER_WRITE: &str = "POST /v1/secret/data/dots/roster";

/// A member of the list with the Dots ID `key`.
fn member(id: &str, email: &str, role: &str, key: &str) -> Value {
    json!({
        "id": id, "name": email, "email": email, "role": role, "keys": [key],
        "workspaces": {"default": "member"}, "added_at": "2026-10-28T09:00:00Z"
    })
}

/// A list at `seq`: an owner who is not this PC, Bob (a Member), then `others`.
fn list(seq: u64, others: &[Value]) -> String {
    let mut members = vec![
        member("01OWNER", "owner@acme.com", "owner", &public_key(&keys())),
        member("01BOB", "bob@acme.com", "member", &public_key(&keys())),
    ];
    members.extend(others.iter().cloned());
    roster(ACME, seq, Value::Array(members))
}

/// Policy v2 asking for a member list, with `admins`.
fn required(admins: &[String]) -> Value {
    json!({"v": 2, "roster": "required", "admins": admins})
}

/// Makes this PC's Dots ID and profile; its public key.
async fn profile(h: &OrgsHarness, name: &str, email: &str) -> String {
    let key = ensure_identity(&h.store, h.secrets.as_ref())
        .await
        .unwrap()
        .public_key;
    set_profile(&h.store, name, email).await.unwrap();
    key
}

/// Serves Acme with a v2 policy and `signed_roster`.
fn serve(h: &OrgsHarness, admins: &[String], signed_roster: &str) {
    let signed = sign(&h.keys, &policy(ACME, ISSUED, required(admins)));
    h.serve("dots", json!({"API_KEY": "k-1"}), &signed);
    h.bao.put_roster("secret", "dots", signed_roster);
}

/// This PC joined to Acme as `role` on the list at seq 1, signed with the root key, and in the
/// policy's `admins` when `signs`. Its Dots ID.
async fn setup(h: &OrgsHarness, role: &str, signs: bool) -> String {
    let mine = profile(h, "Ada", "ada@acme.com").await;
    let text = list(1, &[member("01ME", "ada@acme.com", role, &mine)]);
    let admins = if signs { vec![mine.clone()] } else { vec![] };
    serve(h, &admins, &sign_roster(&h.keys, &text));
    h.orgs.join(&h.acme_code(), Some(token())).await.unwrap();
    mine
}

/// The list at seq 2, published by the owner out of band: this PC stays an Admin, Bob is
/// gone.
fn publish_out_of_band(h: &OrgsHarness, mine: &str) {
    let mut members = vec![member(
        "01OWNER",
        "owner@acme.com",
        "owner",
        &public_key(&keys()),
    )];
    members.push(member("01ME", "ada@acme.com", "admin", mine));
    let text = roster(ACME, 2, Value::Array(members));
    h.bao
        .put_roster("secret", "dots", &sign_roster(&h.keys, &text));
}

/// The owner's list at seq 2 (this PC an Admin, Bob gone), signed with the root key.
fn other_list(h: &OrgsHarness, mine: &str) -> String {
    let members = vec![
        member("01OWNER", "owner@acme.com", "owner", &public_key(&keys())),
        member("01ME", "ada@acme.com", "admin", mine),
    ];
    sign_roster(&h.keys, &roster(ACME, 2, Value::Array(members)))
}

/// The real OpenBao source, except that right after it writes this PC's list it writes
/// `other` (once), as an admin elsewhere publishing at the same moment would.
struct Racing {
    inner: Box<dyn Source>,
    other: Arc<Mutex<Option<String>>>,
}

#[async_trait]
impl Source for Racing {
    async fn fetch(&self, creds: &SourceCreds) -> Result<Fetched, SourceError> {
        self.inner.fetch(creds).await
    }

    async fn publish_roster(&self, creds: &SourceCreds, signed: &str) -> Result<(), PublishError> {
        self.inner.publish_roster(creds, signed).await?;
        let other = self.other.lock().unwrap().take();
        if let Some(other) = other {
            self.inner.publish_roster(creds, &other).await?;
        }
        Ok(())
    }
}

/// Another PC with its own data folder, Dots ID and profile.
async fn newcomer(name: &str, email: &str) -> OrgsHarness {
    let n = OrgsHarness::new().await;
    profile(&n, name, email).await;
    n
}

/// A request code from `n` for Acme, answering `invite_id`.
async fn request(n: &OrgsHarness, invite_id: Option<&str>) -> String {
    make_request(&n.store, n.secrets.as_ref(), ACME, invite_id, None, n.now())
        .await
        .unwrap()
}

fn approve(request: &str) -> RosterEdit {
    RosterEdit::Approve {
        request: request.into(),
        role: None,
        confirm_no_invite: false,
    }
}

fn set_bob_viewer() -> RosterEdit {
    RosterEdit::SetRole {
        member_id: "01BOB".into(),
        role: Role::Viewer,
    }
}

fn published(outcome: EditOutcome) -> OrgSummary {
    match outcome {
        EditOutcome::Published { summary } => *summary,
        other => panic!("not published: {other:?}"),
    }
}

fn drafted(outcome: EditOutcome) -> dots_core::vault::org::DraftView {
    match outcome {
        EditOutcome::Draft { draft } => draft,
        other => panic!("not a draft: {other:?}"),
    }
}

async fn members(h: &OrgsHarness) -> MembersView {
    h.orgs.members(ACME).await.unwrap()
}

fn writes(h: &OrgsHarness) -> usize {
    h.bao.calls().iter().filter(|c| *c == ROSTER_WRITE).count()
}

#[tokio::test]
async fn invite_then_approve_publishes_and_newcomer_becomes_active() {
    let h = OrgsHarness::new().await;
    let mine = setup(&h, "admin", true).await;
    let view = members(&h).await;
    assert_eq!(view.seq, 1);
    assert!(view.can_manage && view.can_sign && view.publishes);
    assert_eq!(view.sign_blocker, None);
    assert_eq!(view.members.len(), 3);
    let me = view.members.iter().find(|m| m.id == "01ME").unwrap();
    assert!(me.keys[0].is_me && me.keys[0].signs);
    assert_eq!(me.keys[0].key, mine);
    let owner = view.members.iter().find(|m| m.id == "01OWNER").unwrap();
    assert!(owner.owner_locked && !owner.keys[0].is_me && !owner.keys[0].signs);

    let created = h
        .orgs
        .create_invite(ACME, "eve@acme.com", Role::Member)
        .await
        .unwrap();
    assert_eq!(created.invite.state, InviteState::Open);
    assert_eq!(created.invite.role, Role::Member);
    let preview = h.orgs.preview_join(&created.code).unwrap();
    let offer = preview.invite.unwrap();
    assert_eq!(offer.id, created.invite.id);
    assert_eq!(offer.expires_at, created.invite.expires_at);

    // The newcomer joins with the invite code: waiting, with its request code to send.
    let n = newcomer("Eve", "eve@acme.com").await;
    let joined = n.orgs.join(&created.code, Some(token())).await.unwrap();
    assert_eq!(joined.membership, Membership::Waiting);
    let request_code = joined.request_code.clone().expect("a request code");
    let request = parse_request(&request_code).unwrap();
    assert_eq!(
        request.invite_id.as_deref(),
        Some(created.invite.id.as_str())
    );
    let row = n.store.get_org(ACME).await.unwrap().unwrap();
    assert_eq!(row.pending_request.unwrap().code, request_code);

    let preview = h.orgs.inspect_request(ACME, &request_code).await.unwrap();
    assert_eq!(preview.problem, None);
    assert_eq!(preview.role, Some(Role::Member));
    assert_eq!(preview.email, "eve@acme.com");
    assert_eq!(preview.invite.unwrap().id, created.invite.id);

    let summary = published(
        h.orgs
            .edit_members(ACME, 1, approve(&request_code))
            .await
            .unwrap(),
    );
    assert_eq!(summary.roster_seq, Some(2));
    assert_eq!(summary.member_count, Some(4));
    assert_eq!(writes(&h), 1);
    let view = members(&h).await;
    assert_eq!(view.seq, 2);
    assert_eq!(view.draft, None);
    assert_eq!(view.invites[0].state, InviteState::Used);
    let eve = view
        .members
        .iter()
        .find(|m| m.email == "eve@acme.com")
        .unwrap();
    assert_eq!(eve.role, Role::Member);

    let after = n.orgs.refresh(ACME).await.unwrap();
    assert_eq!(after.membership, Membership::Active);
    assert_eq!(after.role, Some(Role::Member));
    assert_eq!(after.request_code, None);
    let row = n.store.get_org(ACME).await.unwrap().unwrap();
    assert_eq!(row.pending_request, None);
}

#[tokio::test]
async fn concurrent_edit_is_refused() {
    let h = OrgsHarness::new().await;
    let mine = setup(&h, "admin", true).await;
    publish_out_of_band(&h, &mine);
    let e = h
        .orgs
        .edit_members(ACME, 1, set_bob_viewer())
        .await
        .unwrap_err();
    assert!(
        matches!(&e, Error::Conflict(text) if text == CHANGED),
        "{e:?}"
    );
    assert_eq!(h.store.draft(ACME).await.unwrap(), None);
    assert_eq!(writes(&h), 0);
}

#[tokio::test]
async fn signer_not_in_admins_is_refused_before_signing() {
    let h = OrgsHarness::new().await;
    setup(&h, "admin", false).await;
    let view = members(&h).await;
    assert!(view.can_manage && !view.can_sign);
    assert_eq!(view.sign_blocker.as_deref(), Some(NOT_SIGNER));

    let e = h
        .orgs
        .edit_members(ACME, 1, set_bob_viewer())
        .await
        .unwrap_err();
    assert!(
        matches!(&e, Error::NotAllowed(text) if text == NOT_SIGNER),
        "{e:?}"
    );
    assert_eq!(h.store.draft(ACME).await.unwrap(), None);
    assert_eq!(writes(&h), 0);
}

#[tokio::test]
async fn member_role_cannot_manage() {
    let h = OrgsHarness::new().await;
    setup(&h, "member", true).await;
    let not_allowed = "Acme Engineering: a Member can't manage members. Ask an Admin or Owner.";
    let view = members(&h).await;
    assert!(!view.can_manage && !view.can_sign);
    assert_eq!(view.sign_blocker.as_deref(), Some(not_allowed));

    let e = h
        .orgs
        .edit_members(ACME, 1, set_bob_viewer())
        .await
        .unwrap_err();
    assert!(
        matches!(&e, Error::NotAllowed(text) if text == not_allowed),
        "{e:?}"
    );
    let e = h
        .orgs
        .create_invite(ACME, "eve@acme.com", Role::Member)
        .await
        .unwrap_err();
    assert!(
        matches!(&e, Error::NotAllowed(text) if text == not_allowed),
        "{e:?}"
    );
    assert_eq!(writes(&h), 0);
}

#[tokio::test]
async fn owner_invites_are_refused() {
    let h = OrgsHarness::new().await;
    setup(&h, "admin", true).await;
    let e = h
        .orgs
        .create_invite(ACME, "eve@acme.com", Role::Owner)
        .await
        .unwrap_err();
    assert_eq!(
        e.to_string(),
        "Owners can only be changed with the account owner's key, using dots-admin."
    );
    assert!(h.store.list_invites(ACME).await.unwrap().is_empty());
}

#[tokio::test]
async fn expired_used_and_mismatched_invites_are_problems() {
    let h = OrgsHarness::new().await;
    setup(&h, "admin", true).await;

    // Expired: eight days on, the seven-day offer has ended.
    let invite = h
        .orgs
        .create_invite(ACME, "eve@acme.com", Role::Member)
        .await
        .unwrap()
        .invite;
    let eve = newcomer("Eve", "eve@acme.com").await;
    let code = request(&eve, Some(&invite.id)).await;
    h.advance(Duration::days(8));
    let preview = h.orgs.inspect_request(ACME, &code).await.unwrap();
    assert_eq!(preview.problem.as_deref(), Some("This invite has expired."));
    assert_eq!(preview.invite.unwrap().state, InviteState::Expired);
    let e = h
        .orgs
        .edit_members(ACME, 1, approve(&code))
        .await
        .unwrap_err();
    assert!(
        matches!(&e, Error::Invalid(t) if t == "This invite has expired."),
        "{e:?}"
    );

    // Used: approving twice.
    let invite = h
        .orgs
        .create_invite(ACME, "fay@acme.com", Role::Viewer)
        .await
        .unwrap()
        .invite;
    let fay = newcomer("Fay", "fay@acme.com").await;
    let code = request(&fay, Some(&invite.id)).await;
    let summary = published(h.orgs.edit_members(ACME, 1, approve(&code)).await.unwrap());
    assert_eq!(summary.roster_seq, Some(2));
    let preview = h.orgs.inspect_request(ACME, &code).await.unwrap();
    assert_eq!(
        preview.problem.as_deref(),
        Some("This invite was already used.")
    );
    let e = h
        .orgs
        .edit_members(ACME, 2, approve(&code))
        .await
        .unwrap_err();
    assert!(
        matches!(&e, Error::Invalid(t) if t == "This invite was already used."),
        "{e:?}"
    );

    // Revoked.
    let invite = h
        .orgs
        .create_invite(ACME, "gus@acme.com", Role::Member)
        .await
        .unwrap()
        .invite;
    h.orgs.revoke_invite(ACME, &invite.id).await.unwrap();
    let gus = newcomer("Gus", "gus@acme.com").await;
    let code = request(&gus, Some(&invite.id)).await;
    let preview = h.orgs.inspect_request(ACME, &code).await.unwrap();
    assert_eq!(preview.problem.as_deref(), Some("This invite was revoked."));

    // Another email than the invite's.
    let invite = h
        .orgs
        .create_invite(ACME, "hal@acme.com", Role::Member)
        .await
        .unwrap()
        .invite;
    let mallory = newcomer("Mal", "mal@acme.com").await;
    let code = request(&mallory, Some(&invite.id)).await;
    let preview = h.orgs.inspect_request(ACME, &code).await.unwrap();
    assert_eq!(
        preview.problem.as_deref(),
        Some("This request is for mal@acme.com, but the invite was for hal@acme.com.")
    );

    // An invite this PC never made, another account, and a Dots ID already listed.
    let code = request(&mallory, Some("01UNKNOWNINVITE")).await;
    let preview = h.orgs.inspect_request(ACME, &code).await.unwrap();
    assert_eq!(
        preview.problem.as_deref(),
        Some("This invite wasn't made on this PC.")
    );
    let other = make_request(
        &mallory.store,
        mallory.secrets.as_ref(),
        "beta",
        None,
        None,
        mallory.now(),
    )
    .await
    .unwrap();
    let preview = h.orgs.inspect_request(ACME, &other).await.unwrap();
    assert_eq!(
        preview.problem.as_deref(),
        Some("This request is for another account (beta).")
    );
    let code = request(&fay, None).await;
    let preview = h.orgs.inspect_request(ACME, &code).await.unwrap();
    assert_eq!(
        preview.problem.as_deref(),
        Some("This Dots ID is already on the member list.")
    );

    // A code that doesn't read.
    let e = h.orgs.inspect_request(ACME, "hello").await.unwrap_err();
    assert!(
        matches!(&e, Error::Invalid(t) if t == "this is not a Dots request code"),
        "{e:?}"
    );
}

#[tokio::test]
async fn approve_without_invite_needs_confirm_and_role() {
    let h = OrgsHarness::new().await;
    setup(&h, "admin", true).await;
    let ivy = newcomer("Ivy", "ivy@acme.com").await;
    let code = request(&ivy, None).await;
    let preview = h.orgs.inspect_request(ACME, &code).await.unwrap();
    assert_eq!(preview.problem, None);
    assert_eq!(preview.invite, None);
    assert_eq!(preview.role, None);

    for edit in [
        RosterEdit::Approve {
            request: code.clone(),
            role: Some(Role::Member),
            confirm_no_invite: false,
        },
        RosterEdit::Approve {
            request: code.clone(),
            role: None,
            confirm_no_invite: true,
        },
    ] {
        let e = h.orgs.edit_members(ACME, 1, edit).await.unwrap_err();
        assert!(matches!(&e, Error::Invalid(t) if t == NO_INVITE), "{e:?}");
    }
    assert_eq!(writes(&h), 0);

    let edit = RosterEdit::Approve {
        request: code,
        role: Some(Role::Viewer),
        confirm_no_invite: true,
    };
    let summary = published(h.orgs.edit_members(ACME, 1, edit).await.unwrap());
    assert_eq!(summary.roster_seq, Some(2));
    let view = members(&h).await;
    let ivy = view
        .members
        .iter()
        .find(|m| m.email == "ivy@acme.com")
        .unwrap();
    assert_eq!(ivy.role, Role::Viewer);
}

#[tokio::test]
async fn publish_denied_keeps_a_draft() {
    let h = OrgsHarness::new().await;
    setup(&h, "admin", true).await;
    h.bao.deny_writes("secret", "dots/roster");
    let draft = drafted(
        h.orgs
            .edit_members(ACME, 1, set_bob_viewer())
            .await
            .unwrap(),
    );
    assert_eq!((draft.base_seq, draft.seq), (1, 2));
    assert!(
        draft.error.starts_with("the source refused the write: "),
        "{}",
        draft.error
    );
    assert_eq!(draft.summary, "Change bob@acme.com to Viewer");
    assert_eq!(members(&h).await.draft, Some(draft));
    assert_eq!(h.orgs.summary(ACME).await.unwrap().roster_seq, Some(1));
}

#[tokio::test]
async fn retry_after_list_changed_drops_draft() {
    let h = OrgsHarness::new().await;
    let mine = setup(&h, "admin", true).await;
    h.bao.deny_writes("secret", "dots/roster");
    drafted(
        h.orgs
            .edit_members(ACME, 1, set_bob_viewer())
            .await
            .unwrap(),
    );
    h.bao.allow_writes("secret", "dots/roster");
    publish_out_of_band(&h, &mine);

    let e = h.orgs.retry_draft(ACME).await.unwrap_err();
    assert!(
        matches!(&e, Error::Conflict(text) if text == CHANGED),
        "{e:?}"
    );
    assert_eq!(h.store.draft(ACME).await.unwrap(), None);
    assert_eq!(writes(&h), 1, "only the refused write");
}

#[tokio::test]
async fn retry_publishes_when_allowed() {
    let h = OrgsHarness::new().await;
    setup(&h, "admin", true).await;
    h.bao.deny_writes("secret", "dots/roster");
    drafted(
        h.orgs
            .edit_members(ACME, 1, set_bob_viewer())
            .await
            .unwrap(),
    );
    // Still refused: the draft stays, with the error.
    drafted(h.orgs.retry_draft(ACME).await.unwrap());
    assert!(members(&h).await.draft.is_some());

    h.bao.allow_writes("secret", "dots/roster");
    let summary = published(h.orgs.retry_draft(ACME).await.unwrap());
    assert_eq!(summary.roster_seq, Some(2));
    assert_eq!(members(&h).await.draft, None);
    let bob = members(&h)
        .await
        .members
        .into_iter()
        .find(|m| m.id == "01BOB")
        .unwrap();
    assert_eq!(bob.role, Role::Viewer);
}

#[tokio::test]
async fn discard_draft_drops_it() {
    let h = OrgsHarness::new().await;
    setup(&h, "admin", true).await;
    h.bao.deny_writes("secret", "dots/roster");
    drafted(
        h.orgs
            .edit_members(ACME, 1, set_bob_viewer())
            .await
            .unwrap(),
    );
    h.orgs.discard_draft(ACME).await.unwrap();
    assert_eq!(members(&h).await.draft, None);
}

#[tokio::test]
async fn sops_edit_is_a_draft_to_copy() {
    let h = OrgsHarness::new().await;
    let mine = profile(&h, "Ada", "ada@acme.com").await;
    let text = list(1, &[member("01ME", "ada@acme.com", "admin", &mine)]);
    let signed_policy = sign(
        &h.keys,
        &policy(ACME, ISSUED, required(std::slice::from_ref(&mine))),
    );
    let scripted = Scripted::new();
    let file = |roster: &str| {
        Ok(Fetched {
            entries: [("API_KEY".to_string(), Zeroizing::new("k-1".to_string()))].into(),
            policy: signed_policy.clone(),
            roster: Some(roster.to_string()),
            roster_note: None,
        })
    };
    scripted.answer(file(&sign_roster(&h.keys, &text)));
    let orgs = h.orgs.clone().with_sources(scripted.factory());
    let sops = code(ACME, ACME_NAME, sops_at("C:/sync/acme.enc.json"), &h.keys);
    let creds = SourceCreds::Age {
        identity: Zeroizing::new("AGE-SECRET-KEY-not-read-by-the-scripted-source".into()),
    };
    orgs.join(&sops, Some(creds)).await.unwrap();
    assert!(!members(&h).await.publishes);

    let draft = drafted(orgs.edit_members(ACME, 1, set_bob_viewer()).await.unwrap());
    assert_eq!(draft.error, SOPS_COPY);
    let accepted = verify_roster(&draft.signed, &mine, &[], ACME).unwrap();
    assert_eq!(accepted.signer, mine);
    assert_eq!(accepted.roster.seq, 2);

    // The admin copies it into the file; the next fetch accepts it and the draft is gone.
    scripted.answer(file(&draft.signed));
    let summary = orgs.refresh(ACME).await.unwrap();
    assert_eq!(summary.roster_seq, Some(2));
    assert_eq!(h.store.draft(ACME).await.unwrap(), None);
    assert_eq!(members(&h).await.draft, None);
}

#[tokio::test]
async fn request_access_for_waiting_pc_is_stable() {
    let h = OrgsHarness::new().await;
    setup(&h, "admin", true).await;
    let e = h.orgs.request_access(ACME).await.unwrap_err();
    assert!(matches!(e, Error::Invalid(_)), "{e:?}");

    let n = newcomer("Jo", "jo@acme.com").await;
    let joined = n.orgs.join(&h.acme_code(), Some(token())).await.unwrap();
    assert_eq!(joined.membership, Membership::Waiting);
    assert_eq!(joined.request_code, None);

    let first = n.orgs.request_access(ACME).await.unwrap();
    let code = first.request_code.clone().expect("a request code");
    let again = n.orgs.request_access(ACME).await.unwrap();
    assert_eq!(again.request_code.as_deref(), Some(code.as_str()));
    assert_eq!(
        n.orgs.summary(ACME).await.unwrap().request_code.as_deref(),
        Some(code.as_str())
    );
    let request = parse_request(&code).unwrap();
    assert_eq!(request.invite_id, None);
    assert_eq!(request.email, "jo@acme.com");
    assert_eq!(request.org_id, ACME);
}

#[tokio::test]
async fn request_with_a_listed_email_joins_that_member_with_their_role() {
    let h = OrgsHarness::new().await;
    setup(&h, "admin", true).await;
    // Someone claims this PC's person's email, in other case, with no invite.
    let other = newcomer("Ada again", "ADA@acme.com").await;
    let code = request(&other, None).await;
    let preview = h.orgs.inspect_request(ACME, &code).await.unwrap();
    assert_eq!(preview.problem, None);
    let joins = preview.joins_member.expect("the member the email names");
    assert_eq!((joins.id.as_str(), joins.role), ("01ME", Role::Admin));
    assert_eq!(preview.role, Some(Role::Admin));

    let picked = RosterEdit::Approve {
        request: code.clone(),
        role: Some(Role::Viewer),
        confirm_no_invite: true,
    };
    let e = h.orgs.edit_members(ACME, 1, picked).await.unwrap_err();
    let refused = "ADA@acme.com is already on the list as ada@acme.com (Admin). Approving adds \
                   this PC to them as Admin; change their role separately.";
    assert!(matches!(&e, Error::Invalid(t) if t == refused), "{e:?}");
    assert_eq!(writes(&h), 0);
    assert_eq!(h.store.draft(ACME).await.unwrap(), None);

    let edit = RosterEdit::Approve {
        request: code,
        role: None,
        confirm_no_invite: true,
    };
    let summary = published(h.orgs.edit_members(ACME, 1, edit).await.unwrap());
    assert_eq!(summary.roster_seq, Some(2));
    assert_eq!(summary.member_count, Some(3));
    let view = members(&h).await;
    let me = view.members.iter().find(|m| m.id == "01ME").unwrap();
    assert_eq!(me.role, Role::Admin);
    assert_eq!(me.keys.len(), 2);
}

#[tokio::test]
async fn another_list_read_back_after_publish_is_a_conflict() {
    // Lightest way to race: a source wrapping the real OpenBao one writes the owner's own
    // seq 2 right after this PC's, before the fetch that reads it back.
    let h = OrgsHarness::new().await;
    let other = Arc::new(Mutex::new(None));
    let racing = other.clone();
    h.orgs
        .clone()
        .with_sources(Arc::new(move |settings: &SourceSettings| {
            Box::new(Racing {
                inner: source_for(
                    settings,
                    http_client_with_timeout(StdDuration::from_secs(5)),
                ),
                other: racing.clone(),
            }) as Box<dyn Source>
        }));
    let mine = setup(&h, "admin", true).await;
    *other.lock().unwrap() = Some(other_list(&h, &mine));

    let e = h
        .orgs
        .edit_members(ACME, 1, set_bob_viewer())
        .await
        .unwrap_err();
    assert!(
        matches!(&e, Error::Conflict(text) if text == CHANGED),
        "{e:?}"
    );
    assert_eq!(h.store.draft(ACME).await.unwrap(), None);
    let view = members(&h).await;
    assert_eq!(view.seq, 2);
    assert!(
        view.members.iter().all(|m| m.id != "01BOB"),
        "the owner's list"
    );
}

#[tokio::test]
async fn retry_rechecks_the_signer() {
    let h = OrgsHarness::new().await;
    setup(&h, "admin", true).await;
    h.bao.deny_writes("secret", "dots/roster");
    drafted(
        h.orgs
            .edit_members(ACME, 1, set_bob_viewer())
            .await
            .unwrap(),
    );
    assert_eq!(writes(&h), 1, "the refused write");
    // The owner takes this PC's Dots ID out of `admins`.
    let signed = sign(&h.keys, &policy(ACME, LATER, required(&[])));
    h.bao.put_policy("secret", "dots", &signed);
    h.bao.allow_writes("secret", "dots/roster");

    let e = h.orgs.retry_draft(ACME).await.unwrap_err();
    assert!(
        matches!(&e, Error::NotAllowed(text) if text == NOT_SIGNER),
        "{e:?}"
    );
    assert_eq!(writes(&h), 1, "nothing more was written");
    assert!(
        h.store.draft(ACME).await.unwrap().is_some(),
        "the draft is kept"
    );
}
