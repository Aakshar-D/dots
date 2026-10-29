//! The Members tab's work on an account (accounts spec §6.5, §7; members spec §6.2): invites,
//! request codes, and edits to the member list signed with this PC's Dots ID and published to
//! the account's source. A list the source refuses (or a SOPS file, which Dots can't write) is
//! kept as a draft to retry or copy by hand. A PC waiting to be added makes its request code
//! here too.
//!
//! Every action an Admin takes calls `accounts::require(.., ManageMembers)` first, and signing
//! needs this PC's Dots ID in the policy's `admins` (members spec §6.1).

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

use super::join::{key_fingerprint, same_policy_key, InviteOffer, JoinCode};
use super::row::PendingRequest;
use super::service::{not_joined, parse_time, stamp, MemberState, OrgSummary, Orgs};
use super::sources::{PublishError, SourceCreds};
use super::SourceSettings;
use crate::accounts::edit::{apply, default_workspace_role, Change, RosterEdit, OWNER_LOCKED};
use crate::accounts::{
    access, identity, make_request, open_accepted_roster, parse_request, require, roster_envelope,
    sign_with_identity, AccessRequest, Membership, Permission, Role, Roster, WorkspaceRole,
    ROSTER_VERSION,
};
use crate::store::{DraftRow, InviteRow};
use crate::{Error, Result};

/// How long an invite's offer holds (accounts spec §7).
const INVITE_DAYS: i64 = 7;

/// A draft's error for a SOPS account: Dots can't write the file.
const COPY_TO_SOPS: &str = "Copy the signed list into the SOPS file (see the admin guide).";

/// A draft's error when the source took the list but the fetch after it did not show it yet.
const NOT_READ_BACK: &str =
    "The source took the list but Dots couldn't read it back yet. Retry in a moment.";

const NO_INVITE: &str =
    "This person has no invite from this PC. Confirm to approve them anyway, and pick a role.";

/// The member list as the Members tab shows it, with what this PC may do to it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MembersView {
    /// The `seq` of the list accepted last: an edit is made against it.
    pub seq: u64,
    pub members: Vec<MemberView>,
    /// This PC's role may manage members (Admin or Owner).
    pub can_manage: bool,
    /// It may, and its Dots ID is one of the policy's `admins`, so it can sign.
    pub can_sign: bool,
    /// Why this PC can't sign, in plain words; `None` when it can.
    pub sign_blocker: Option<String>,
    /// Dots publishes the list to the source; false for a SOPS file, which is copied by hand.
    pub publishes: bool,
    /// The invites made on this PC, newest first.
    pub invites: Vec<InviteView>,
    /// A list signed but not yet published.
    pub draft: Option<DraftView>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MemberView {
    pub id: String,
    pub name: String,
    pub email: String,
    pub role: Role,
    pub keys: Vec<KeyView>,
    pub workspaces: BTreeMap<String, WorkspaceRole>,
    pub added_at: String,
    /// An Owner: only the owner's key, in `dots-admin`, may change them.
    pub owner_locked: bool,
}

/// One PC (Dots ID) of a member.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct KeyView {
    pub key: String,
    pub fingerprint: String,
    /// This PC's Dots ID.
    pub is_me: bool,
    /// One of the policy's `admins`: lists it signs are accepted.
    pub signs: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum InviteState {
    Open,
    Used,
    Expired,
    Revoked,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InviteView {
    pub id: String,
    pub email: String,
    pub role: Role,
    pub created_at: String,
    pub expires_at: String,
    pub state: InviteState,
}

/// A signed list waiting to be published (accounts spec §6.5).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DraftView {
    pub base_seq: u64,
    pub seq: u64,
    /// The change, in words.
    pub summary: String,
    /// Why it was not published.
    pub error: String,
    pub created_at: String,
    /// The signed list, to copy into a SOPS file.
    pub signed: String,
}

/// A new invite and the join code that carries its offer.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CreatedInvite {
    pub invite: InviteView,
    pub code: String,
}

/// What a request code asks, checked against this PC's invites and the member list.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RequestPreview {
    pub name: String,
    pub email: String,
    pub key: String,
    pub fingerprint: String,
    /// The invite the request answers, when this PC made it.
    pub invite: Option<InviteView>,
    /// The role the approval gives: the role of the member the email names, or else the
    /// invite's.
    pub role: Option<Role>,
    /// The member already on the list with the request's email: approving adds this PC to
    /// them, with their role.
    pub joins_member: Option<JoinsMember>,
    /// The age key a SOPS account's file must be encrypted to.
    pub age_recipient: Option<String>,
    /// Why the request can't be approved, in plain words; `None` when it can.
    pub problem: Option<String>,
}

/// A member a request's email names.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct JoinsMember {
    pub id: String,
    pub name: String,
    pub role: Role,
}

/// How an edit ended: published, or kept as a draft.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum EditOutcome {
    Published { summary: Box<OrgSummary> },
    Draft { draft: DraftView },
}

impl Orgs {
    /// The account's member list for the Members tab. `NotFound` when no list was accepted.
    pub async fn members(&self, org_id: &str) -> Result<MembersView> {
        let state = self.member_state(org_id)?;
        let (roster, _) = accepted(&state)?;
        let my_key = identity(self.store()).await.ok().map(|me| me.public_key);
        let admins = admins(&state);
        let signs = |key: &str| admins.iter().any(|admin| same_policy_key(admin, key));
        let can_manage = access(Some(self), Some(org_id)).allows(Permission::ManageMembers);
        let in_admins = my_key.as_deref().is_some_and(signs);
        let sign_blocker = if !can_manage {
            require(Some(self), Some(org_id), Permission::ManageMembers)
                .await
                .err()
                .map(|e| e.to_string())
        } else if !in_admins {
            Some(not_a_signer(&state.row.name))
        } else {
            None
        };
        let now = self.now();
        let members = roster
            .members
            .iter()
            .map(|m| MemberView {
                id: m.id.clone(),
                name: m.name.clone(),
                email: m.email.clone(),
                role: m.role,
                keys: m
                    .keys
                    .iter()
                    .map(|key| KeyView {
                        key: key.clone(),
                        fingerprint: key_fingerprint(key).unwrap_or_default(),
                        is_me: my_key.as_deref() == Some(key.as_str()),
                        signs: signs(key),
                    })
                    .collect(),
                workspaces: m.workspaces.clone(),
                added_at: m.added_at.clone(),
                owner_locked: m.role == Role::Owner,
            })
            .collect();
        let invites = self.store().list_invites(org_id).await?;
        Ok(MembersView {
            seq: roster.seq,
            members,
            can_manage,
            can_sign: can_manage && in_admins,
            sign_blocker,
            publishes: !matches!(state.row.source, SourceSettings::Sops(_)),
            invites: invites.iter().map(|i| invite_view(i, now)).collect(),
            draft: self.store().draft(org_id).await?.map(draft_view),
        })
    }

    /// Makes an invite for `email` as `role`, good for seven days, and the join code that
    /// offers it (a version 2 code). An Owner can't be invited from Dots.
    pub async fn create_invite(
        &self,
        org_id: &str,
        email: &str,
        role: Role,
    ) -> Result<CreatedInvite> {
        require(Some(self), Some(org_id), Permission::ManageMembers).await?;
        if role == Role::Owner {
            return Err(Error::NotAllowed(OWNER_LOCKED.into()));
        }
        let email = email.trim();
        match email.split_once('@') {
            Some((local, domain)) if !local.is_empty() && !domain.is_empty() => {}
            _ => return Err(Error::Invalid("an email address needs an @".into())),
        }
        let row = self.member_state(org_id)?.row;
        let now = self.now();
        let invite = InviteRow {
            id: ulid::Ulid::new().to_string(),
            org_id: org_id.to_string(),
            email: email.to_string(),
            role,
            workspaces: BTreeMap::from([("default".to_string(), default_workspace_role(role))]),
            created_at: stamp(now),
            expires_at: stamp(now + Duration::days(INVITE_DAYS)),
            used_at: None,
            revoked_at: None,
        };
        self.store().add_invite(&invite).await?;
        let code = JoinCode {
            v: 2,
            org_id: row.id,
            name: row.name,
            source: row.source,
            policy_key: row.policy_key,
            invite: Some(InviteOffer {
                id: invite.id.clone(),
                role,
                expires_at: invite.expires_at.clone(),
            }),
        }
        .encode();
        Ok(CreatedInvite {
            invite: invite_view(&invite, now),
            code,
        })
    }

    /// Revokes an invite made on this PC: a request answering it can't be approved.
    pub async fn revoke_invite(&self, org_id: &str, invite_id: &str) -> Result<()> {
        require(Some(self), Some(org_id), Permission::ManageMembers).await?;
        self.store()
            .revoke_invite(org_id, invite_id, &stamp(self.now()))
            .await
    }

    /// Makes this PC's request code for an account it waits to be added to, keeps it, and
    /// returns the summary that shows it. A code made already (with an invite, at join) is
    /// kept and given again. A SOPS account's code carries this PC's age key.
    pub async fn request_access(&self, org_id: &str) -> Result<OrgSummary> {
        let state = self.member_state(org_id)?;
        if state.row.member.membership != Membership::Waiting {
            return Err(Error::Invalid(format!(
                "{}: this PC isn't waiting to be added, so it needs no request code.",
                state.row.name
            )));
        }
        if state.row.pending_request.is_none() {
            let age = match state.row.source {
                SourceSettings::Sops(_) => Some(self.sops_identity(org_id).await?),
                _ => None,
            };
            let now = self.now();
            let code = make_request(
                self.store(),
                self.secrets(),
                org_id,
                None,
                age.as_deref(),
                now,
            )
            .await?;
            let request = PendingRequest {
                code,
                invite_id: None,
            };
            self.set_pending(org_id, Some(request)).await?;
        }
        self.summary(org_id).await.ok_or_else(|| not_joined(org_id))
    }

    /// What a request code asks, and why it can't be approved if it can't. A code that doesn't
    /// read is `Invalid` with why.
    pub async fn inspect_request(&self, org_id: &str, code: &str) -> Result<RequestPreview> {
        require(Some(self), Some(org_id), Permission::ManageMembers).await?;
        let state = self.member_state(org_id)?;
        Ok(self.check_request(&state, code).await?.1)
    }

    /// Makes one change to the member list (members spec §6.2), in this order: the role may
    /// manage members; this PC's Dots ID signs for the account; the list is fetched again and
    /// must still be `base_seq` (§6.2, concurrent edits); an approval's request checks out; the
    /// change is applied (never to an Owner); the list is signed with the Dots ID and
    /// published. A source that refuses it, or a SOPS file, leaves a draft.
    pub async fn edit_members(
        &self,
        org_id: &str,
        base_seq: u64,
        edit: RosterEdit,
    ) -> Result<EditOutcome> {
        self.check_signer(org_id).await?;
        self.refresh(org_id).await?;
        // The fetch may have brought another policy or list: check again against it.
        let state = self.check_signer(org_id).await?;
        let (roster, _) = accepted(&state)?;
        if roster.seq != base_seq {
            return Err(changed(&state.row.name, roster.seq));
        }

        let (change, invite_id) = match edit {
            RosterEdit::Approve {
                request,
                role,
                confirm_no_invite,
            } => {
                let (asked, preview, invite) = self.check_request(&state, &request).await?;
                if let Some(problem) = preview.problem {
                    return Err(Error::Invalid(problem));
                }
                // A request whose email is on the list adds this PC to that member, who keeps
                // their role: a role picked here (or by the invite) that differs is refused,
                // never silently dropped.
                if let Some(member) = &preview.joins_member {
                    let picked = invite.as_ref().map(|i| i.role).or(role);
                    if picked.is_some_and(|picked| picked != member.role) {
                        return Err(Error::Invalid(format!(
                            "{} is already on the list as {} ({label}). Approving adds this PC \
                             to them as {label}; change their role separately.",
                            asked.email,
                            member.name,
                            label = member.role.label()
                        )));
                    }
                }
                let joins = preview.joins_member.as_ref().map(|m| m.role);
                let (role, workspaces, invite_id) = match (invite, joins.or(role)) {
                    (Some(invite), _) => (invite.role, invite.workspaces, Some(invite.id)),
                    (None, Some(role)) if confirm_no_invite => (
                        role,
                        BTreeMap::from([("default".to_string(), default_workspace_role(role))]),
                        None,
                    ),
                    (None, _) => return Err(Error::Invalid(NO_INVITE.into())),
                };
                let change = Change::AddKey {
                    name: asked.name,
                    email: asked.email,
                    key: asked.key,
                    role,
                    workspaces,
                };
                (change, invite_id)
            }
            RosterEdit::SetRole { member_id, role } => (Change::SetRole { member_id, role }, None),
            RosterEdit::Remove { member_id } => (Change::Remove { member_id }, None),
            RosterEdit::RemoveKey { member_id, key } => {
                (Change::RemoveKey { member_id, key }, None)
            }
        };

        let now = self.now();
        let (next, summary) = apply(roster, &change, now)?;
        // `apply` never touches an Owner; this guards the rule a reader checks the list by.
        next.check_owners(false, state.row.member.owners_hash.as_deref())
            .map_err(|_| Error::NotAllowed(OWNER_LOCKED.into()))?;
        let text = roster_text(&next)?;
        let comment = format!("dots roster {org_id} {}", next.seq);
        let signature = sign_with_identity(self.store(), self.secrets(), &text, &comment).await?;
        let draft = DraftRow {
            org_id: org_id.to_string(),
            base_seq,
            seq: next.seq,
            signed: roster_envelope(&text, &signature),
            summary,
            error: String::new(),
            created_at: stamp(now),
        };
        self.publish(&state, draft, invite_id.as_deref()).await
    }

    /// Publishes the account's draft again. If the account has reached the draft's list since
    /// (published, or copied into a SOPS file) it is done; if the list changed otherwise, the
    /// draft is dropped and the change must be made again.
    pub async fn retry_draft(&self, org_id: &str) -> Result<EditOutcome> {
        require(Some(self), Some(org_id), Permission::ManageMembers).await?;
        let state = self.member_state(org_id)?;
        let draft = self.store().draft(org_id).await?.ok_or_else(|| {
            Error::NotFound(format!(
                "{} has no member list waiting to be published",
                state.row.name
            ))
        })?;
        self.refresh(org_id).await?;
        let state = self.member_state(org_id)?;
        let (roster, text) = accepted(&state)?;
        if reached(roster, text, &draft) {
            self.store().delete_draft(org_id).await?;
            return Ok(EditOutcome::Published {
                summary: Box::new(self.summary_or_missing(org_id).await?),
            });
        }
        if roster.seq != draft.base_seq {
            self.store().delete_draft(org_id).await?;
            return Err(changed(&state.row.name, roster.seq));
        }
        // The fetch may have brought a policy that no longer names this PC's Dots ID: the draft
        // is kept, but nothing is published.
        let state = self.check_signer(org_id).await?;
        self.publish(&state, draft, None).await
    }

    /// Drops the account's draft. Having none is not an error.
    pub async fn discard_draft(&self, org_id: &str) -> Result<()> {
        require(Some(self), Some(org_id), Permission::ManageMembers).await?;
        self.store().delete_draft(org_id).await
    }

    /// The role may manage members and this PC's Dots ID is in the policy's `admins`; the
    /// org's state as it stands.
    async fn check_signer(&self, org_id: &str) -> Result<MemberState> {
        require(Some(self), Some(org_id), Permission::ManageMembers).await?;
        let state = self.member_state(org_id)?;
        let mine = identity(self.store()).await?.public_key;
        if !admins(&state)
            .iter()
            .any(|admin| same_policy_key(admin, &mine))
        {
            return Err(Error::NotAllowed(not_a_signer(&state.row.name)));
        }
        Ok(state)
    }

    /// Reads a request code and checks it: the account, the invite it answers (made on this
    /// PC, not used, revoked or expired, for the same email), and that its Dots ID is not
    /// listed already. The first problem found is the preview's `problem`.
    async fn check_request(
        &self,
        state: &MemberState,
        code: &str,
    ) -> Result<(AccessRequest, RequestPreview, Option<InviteRow>)> {
        let asked = parse_request(code).map_err(|e| Error::Invalid(e.to_string()))?;
        let org_id = &state.row.id;
        let invite = match &asked.invite_id {
            Some(id) => self.store().invite(org_id, id).await?,
            None => None,
        };
        let now = self.now();
        let view = invite.as_ref().map(|i| invite_view(i, now));
        let roster = state.roster.as_ref().map(|(roster, _)| roster);
        let listed = roster.is_some_and(|roster| roster.member_with_key(&asked.key).is_some());
        let joins_member = roster
            .and_then(|roster| roster.member_with_email(&asked.email))
            .map(|m| JoinsMember {
                id: m.id.clone(),
                name: m.name.clone(),
                role: m.role,
            });
        let problem = (if asked.org_id != *org_id {
            Some(format!(
                "This request is for another account ({}).",
                asked.org_id
            ))
        } else if asked.invite_id.is_some() && invite.is_none() {
            Some("This invite wasn't made on this PC.".to_string())
        } else if let (Some(invite), Some(view)) = (&invite, &view) {
            match view.state {
                InviteState::Used => Some("This invite was already used.".to_string()),
                InviteState::Revoked => Some("This invite was revoked.".to_string()),
                InviteState::Expired => Some("This invite has expired.".to_string()),
                InviteState::Open if !invite.email.eq_ignore_ascii_case(asked.email.trim()) => {
                    Some(format!(
                        "This request is for {}, but the invite was for {}.",
                        asked.email, invite.email
                    ))
                }
                InviteState::Open => None,
            }
        } else {
            None
        })
        .or_else(|| listed.then(|| "This Dots ID is already on the member list.".to_string()));
        let preview = RequestPreview {
            name: asked.name.clone(),
            email: asked.email.clone(),
            fingerprint: key_fingerprint(&asked.key)?,
            key: asked.key.clone(),
            role: joins_member
                .as_ref()
                .map(|m| m.role)
                .or(invite.as_ref().map(|i| i.role)),
            joins_member,
            invite: view,
            age_recipient: asked.age_recipient.clone(),
            problem,
        };
        Ok((asked, preview, invite))
    }

    /// Publishes a signed list to the account's source. Read back at the list's `seq`, it is
    /// done: the draft is dropped. Otherwise it is kept as the draft, with why. Either way the
    /// invite it answers is used.
    async fn publish(
        &self,
        state: &MemberState,
        mut draft: DraftRow,
        invite_id: Option<&str>,
    ) -> Result<EditOutcome> {
        let org_id = state.row.id.as_str();
        let published = match SourceCreds::load(self.secrets(), org_id)? {
            Some(creds) => state
                .source
                .publish_roster(&creds, &draft.signed)
                .await
                .map_err(|e| match e {
                    PublishError::Unsupported => COPY_TO_SOPS.to_string(),
                    other => other.to_string(),
                }),
            None => Err("its sign-in details are missing; give them again with Update".into()),
        };
        let error = match published {
            Ok(()) => {
                let summary = self.refresh(org_id).await?;
                let now = self.member_state(org_id)?;
                if let Some((roster, text)) = &now.roster {
                    if reached(roster, text, &draft) {
                        self.use_invite(org_id, invite_id).await?;
                        self.store().delete_draft(org_id).await?;
                        return Ok(EditOutcome::Published {
                            summary: Box::new(summary),
                        });
                    }
                    // Another list of the same number took its place: this change is lost.
                    if roster.seq >= draft.seq {
                        self.store().delete_draft(org_id).await?;
                        return Err(changed(&now.row.name, roster.seq));
                    }
                }
                NOT_READ_BACK.to_string()
            }
            Err(error) => error,
        };
        draft.error = error;
        self.store().put_draft(&draft).await?;
        self.use_invite(org_id, invite_id).await?;
        Ok(EditOutcome::Draft {
            draft: draft_view(draft),
        })
    }

    async fn use_invite(&self, org_id: &str, invite_id: Option<&str>) -> Result<()> {
        match invite_id {
            Some(id) => {
                self.store()
                    .mark_invite_used(org_id, id, &stamp(self.now()))
                    .await
            }
            None => Ok(()),
        }
    }

    async fn summary_or_missing(&self, org_id: &str) -> Result<OrgSummary> {
        self.summary(org_id).await.ok_or_else(|| not_joined(org_id))
    }
}

/// Whether the account has reached the draft's list: a later one was accepted, or this very
/// list (the same text) at its `seq`. Another list of the same `seq` is not it.
fn reached(accepted: &Roster, accepted_text: &str, draft: &DraftRow) -> bool {
    let drafted = open_accepted_roster(&draft.signed, &draft.org_id).map(|(_, text)| text);
    accepted.seq > draft.seq
        || (accepted.seq == draft.seq && drafted.is_ok_and(|text| text == accepted_text))
}

/// The list accepted last, or `NotFound` when there is none.
fn accepted(state: &MemberState) -> Result<(&Roster, &String)> {
    state
        .roster
        .as_ref()
        .map(|(roster, text)| (roster, text))
        .ok_or_else(|| Error::NotFound(format!("{} has no member list yet.", state.row.name)))
}

fn admins(state: &MemberState) -> &[String] {
    state
        .policy
        .as_ref()
        .map(|p| p.admins.as_slice())
        .unwrap_or_default()
}

fn not_a_signer(account: &str) -> String {
    format!(
        "{account}: your Dots ID isn't one of the account's signing admins yet. Ask the owner \
         to add it with dots-admin policy admins add."
    )
}

fn changed(account: &str, seq: u64) -> Error {
    Error::Conflict(format!(
        "{account}: the member list changed (now #{seq}) while you were editing. Check it and \
         make the change again."
    ))
}

/// The roster as a roster file's text, the text that is signed.
fn roster_text(roster: &Roster) -> Result<String> {
    #[derive(Serialize)]
    struct File<'a> {
        v: u32,
        #[serde(flatten)]
        roster: &'a Roster,
    }
    Ok(serde_json::to_string(&File {
        v: ROSTER_VERSION,
        roster,
    })?)
}

fn invite_view(invite: &InviteRow, now: DateTime<Utc>) -> InviteView {
    let expired = parse_time(&invite.expires_at).is_none_or(|end| now >= end);
    let state = if invite.used_at.is_some() {
        InviteState::Used
    } else if invite.revoked_at.is_some() {
        InviteState::Revoked
    } else if expired {
        InviteState::Expired
    } else {
        InviteState::Open
    };
    InviteView {
        id: invite.id.clone(),
        email: invite.email.clone(),
        role: invite.role,
        created_at: invite.created_at.clone(),
        expires_at: invite.expires_at.clone(),
        state,
    }
}

fn draft_view(draft: DraftRow) -> DraftView {
    DraftView {
        base_seq: draft.base_seq,
        seq: draft.seq,
        summary: draft.summary,
        error: draft.error,
        created_at: draft.created_at,
        signed: draft.signed,
    }
}
