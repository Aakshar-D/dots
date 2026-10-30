//! Invites an admin made and the unpublished member list kept for an account (accounts spec §7
//! and §6.5). Both go with their org.

use std::collections::BTreeMap;

use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::Store;
use crate::accounts::{Role, WorkspaceRole};
use crate::vault::org::policy::plain_serde;
use crate::{Error, Result};

const INVITE_COLUMNS: &str =
    "id, org_id, email, role, workspaces, created_at, expires_at, used_at, revoked_at";

/// One invite: who it is for and what they get. Times are RFC 3339 UTC, given by the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteRow {
    pub id: String,
    pub org_id: String,
    pub email: String,
    pub role: Role,
    pub workspaces: BTreeMap<String, WorkspaceRole>,
    pub created_at: String,
    pub expires_at: String,
    pub used_at: Option<String>,
    pub revoked_at: Option<String>,
}

/// A member list signed but not yet published, one per account (§6.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftRow {
    pub org_id: String,
    /// The `seq` of the list this one was made from.
    pub base_seq: u64,
    pub seq: u64,
    /// The signed list as text.
    pub signed: String,
    /// What the change is, in words, for the person to confirm.
    pub summary: String,
    /// Why it could not be published, empty while it has not failed.
    pub error: String,
    pub created_at: String,
    /// The invite an approval in this list answers: it is used while the draft waits, and open
    /// again when the draft is dropped without being published.
    pub invite_id: Option<String>,
}

fn invite_from_row(row: &SqliteRow) -> Result<InviteRow> {
    let id: String = row.try_get("id")?;
    let workspaces =
        serde_json::from_str(&row.try_get::<String, _>("workspaces")?).map_err(|e| {
            Error::Other(format!(
                "invite '{id}': its stored workspaces are not readable: {}",
                plain_serde(&e)
            ))
        })?;
    Ok(InviteRow {
        role: Role::parse(&row.try_get::<String, _>("role")?)?,
        org_id: row.try_get("org_id")?,
        email: row.try_get("email")?,
        workspaces,
        created_at: row.try_get("created_at")?,
        expires_at: row.try_get("expires_at")?,
        used_at: row.try_get("used_at")?,
        revoked_at: row.try_get("revoked_at")?,
        id,
    })
}

fn draft_from_row(row: &SqliteRow) -> Result<DraftRow> {
    let org_id: String = row.try_get("org_id")?;
    let number = |column: &str| -> Result<u64> {
        let n: i64 = row.try_get(column)?;
        u64::try_from(n).map_err(|_| {
            Error::Other(format!(
                "org '{org_id}': its stored draft {column} is not readable: {n}"
            ))
        })
    };
    Ok(DraftRow {
        base_seq: number("base_seq")?,
        seq: number("seq")?,
        signed: row.try_get("signed")?,
        summary: row.try_get("summary")?,
        error: row.try_get("error")?,
        created_at: row.try_get("created_at")?,
        invite_id: row.try_get("invite_id")?,
        org_id,
    })
}

fn too_large() -> Error {
    Error::Invalid("a member list number is too large".into())
}

impl Store {
    /// Adds an invite. An id already used is a `Conflict`.
    pub async fn add_invite(&self, invite: &InviteRow) -> Result<()> {
        let added = sqlx::query(&format!(
            "INSERT INTO account_invites ({INVITE_COLUMNS}) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"
        ))
        .bind(&invite.id)
        .bind(&invite.org_id)
        .bind(&invite.email)
        .bind(invite.role.as_str())
        .bind(serde_json::to_string(&invite.workspaces)?)
        .bind(&invite.created_at)
        .bind(&invite.expires_at)
        .bind(&invite.used_at)
        .bind(&invite.revoked_at)
        .execute(&self.pool)
        .await;
        if let Err(sqlx::Error::Database(db)) = &added {
            if db.is_unique_violation() {
                return Err(Error::Conflict(format!(
                    "the invite '{}' already exists",
                    invite.id
                )));
            }
        }
        added?;
        Ok(())
    }

    /// The invite `id` of the org, if there is one.
    pub async fn invite(&self, org_id: &str, id: &str) -> Result<Option<InviteRow>> {
        let row = sqlx::query(&format!(
            "SELECT {INVITE_COLUMNS} FROM account_invites WHERE org_id = ? AND id = ?"
        ))
        .bind(org_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(invite_from_row).transpose()
    }

    /// The org's invites, newest first.
    pub async fn list_invites(&self, org_id: &str) -> Result<Vec<InviteRow>> {
        let rows = sqlx::query(&format!(
            "SELECT {INVITE_COLUMNS} FROM account_invites WHERE org_id = ? \
             ORDER BY created_at DESC, id DESC"
        ))
        .bind(org_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(invite_from_row).collect()
    }

    /// Records that the invite was used at `at`. An invite that is not there is `NotFound`.
    pub async fn mark_invite_used(&self, org_id: &str, id: &str, at: &str) -> Result<()> {
        self.stamp_invite("used_at", org_id, id, Some(at)).await
    }

    /// Records that the invite was revoked at `at`. An invite that is not there is `NotFound`.
    pub async fn revoke_invite(&self, org_id: &str, id: &str, at: &str) -> Result<()> {
        self.stamp_invite("revoked_at", org_id, id, Some(at)).await
    }

    /// Makes a used invite open again: its draft was dropped unpublished. An invite that is not
    /// there is `NotFound`.
    pub async fn release_invite(&self, org_id: &str, id: &str) -> Result<()> {
        self.stamp_invite("used_at", org_id, id, None).await
    }

    async fn stamp_invite(
        &self,
        column: &str,
        org_id: &str,
        id: &str,
        at: Option<&str>,
    ) -> Result<()> {
        let done = sqlx::query(&format!(
            "UPDATE account_invites SET {column} = ? WHERE org_id = ? AND id = ?"
        ))
        .bind(at)
        .bind(org_id)
        .bind(id)
        .execute(&self.pool)
        .await?;
        if done.rows_affected() == 0 {
            return Err(Error::NotFound(format!(
                "no invite '{id}' for the org '{org_id}'"
            )));
        }
        Ok(())
    }

    /// Keeps `draft` as the org's draft, replacing any earlier one.
    pub async fn put_draft(&self, draft: &DraftRow) -> Result<()> {
        sqlx::query(
            "INSERT OR REPLACE INTO roster_drafts \
             (org_id, base_seq, seq, signed, summary, error, created_at, invite_id) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&draft.org_id)
        .bind(i64::try_from(draft.base_seq).map_err(|_| too_large())?)
        .bind(i64::try_from(draft.seq).map_err(|_| too_large())?)
        .bind(&draft.signed)
        .bind(&draft.summary)
        .bind(&draft.error)
        .bind(&draft.created_at)
        .bind(&draft.invite_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The org's draft, if it has one.
    pub async fn draft(&self, org_id: &str) -> Result<Option<DraftRow>> {
        let row = sqlx::query(
            "SELECT org_id, base_seq, seq, signed, summary, error, created_at, invite_id \
             FROM roster_drafts WHERE org_id = ?",
        )
        .bind(org_id)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(draft_from_row).transpose()
    }

    /// Drops the org's draft. Having none is not an error.
    pub async fn delete_draft(&self, org_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM roster_drafts WHERE org_id = ?")
            .bind(org_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
