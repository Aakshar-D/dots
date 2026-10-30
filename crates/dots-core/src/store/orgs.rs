//! The orgs joined on this PC (env vault spec §5.2): their settings, never a secret.

use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::Store;
use crate::accounts::{Membership, Role};
use crate::vault::org::policy::plain_serde;
use crate::vault::org::{check_org_id, MemberRow, OrgRow, PendingRequest, SourceSettings};
use crate::{Error, Result};

const ORG_COLUMNS: &str =
    "id, name, source, policy_key, last_issued_at, last_fetch_at, joined_at, \
     membership, my_member_id, my_role, my_workspaces, last_roster_seq, my_key, last_roster_hash, \
     last_owners_hash, pending_request";

/// The member columns of a row (members spec §6.4).
fn member_from_row(id: &str, row: &SqliteRow) -> Result<MemberRow> {
    let damaged = |what: &str, why: String| {
        Error::Other(format!(
            "org '{id}': its stored {what} is not readable: {why}"
        ))
    };
    let workspaces = match row.try_get::<Option<String>, _>("my_workspaces")? {
        Some(text) => serde_json::from_str(&text)
            .map_err(|e| damaged("member workspaces", plain_serde(&e)))?,
        None => Default::default(),
    };
    let last_roster_seq = match row.try_get::<Option<i64>, _>("last_roster_seq")? {
        Some(seq) => {
            Some(u64::try_from(seq).map_err(|_| damaged("member list number", seq.to_string()))?)
        }
        None => None,
    };
    Ok(MemberRow {
        membership: Membership::parse(&row.try_get::<String, _>("membership")?)?,
        member_id: row.try_get("my_member_id")?,
        role: row
            .try_get::<Option<String>, _>("my_role")?
            .map(|role| Role::parse(&role))
            .transpose()?,
        workspaces,
        last_roster_seq,
        key: row.try_get("my_key")?,
        roster_hash: row.try_get("last_roster_hash")?,
        owners_hash: row.try_get("last_owners_hash")?,
    })
}

/// The member columns as they are written: workspaces as JSON, NULL when there are none.
fn member_columns(member: &MemberRow) -> Result<(Option<String>, Option<i64>)> {
    let workspaces = if member.workspaces.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&member.workspaces)?)
    };
    let seq = member
        .last_roster_seq
        .map(i64::try_from)
        .transpose()
        .map_err(|_| Error::Invalid("a member list number is too large".into()))?;
    Ok((workspaces, seq))
}

fn org_from_row(row: &SqliteRow) -> Result<OrgRow> {
    let id: String = row.try_get("id")?;
    let source = serde_json::from_str(&row.try_get::<String, _>("source")?).map_err(|e| {
        Error::Other(format!(
            "org '{id}': its stored source settings are not readable: {}",
            plain_serde(&e)
        ))
    })?;
    let pending_request = match row.try_get::<Option<String>, _>("pending_request")? {
        Some(text) => Some(serde_json::from_str(&text).map_err(|e| {
            Error::Other(format!(
                "org '{id}': its stored pending request is not readable: {}",
                plain_serde(&e)
            ))
        })?),
        None => None,
    };
    Ok(OrgRow {
        pending_request,
        member: member_from_row(&id, row)?,
        name: row.try_get("name")?,
        source,
        policy_key: row.try_get("policy_key")?,
        last_issued_at: row.try_get("last_issued_at")?,
        last_fetch_at: row.try_get("last_fetch_at")?,
        joined_at: row.try_get("joined_at")?,
        id,
    })
}

fn not_joined(id: &str) -> Error {
    Error::NotFound(format!("no org '{id}' is joined on this PC"))
}

impl Store {
    /// Adds a joined org. An id already joined is a `Conflict`.
    pub async fn add_org(&self, org: &OrgRow) -> Result<()> {
        check_org_id(&org.id)?;
        let (workspaces, seq) = member_columns(&org.member)?;
        let pending_request = org
            .pending_request
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let added = sqlx::query(&format!(
            "INSERT INTO orgs ({ORG_COLUMNS}) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        ))
        .bind(&org.id)
        .bind(&org.name)
        .bind(serde_json::to_string(&org.source)?)
        .bind(&org.policy_key)
        .bind(&org.last_issued_at)
        .bind(&org.last_fetch_at)
        .bind(&org.joined_at)
        .bind(org.member.membership.as_str())
        .bind(&org.member.member_id)
        .bind(org.member.role.map(|role| role.as_str()))
        .bind(workspaces)
        .bind(seq)
        .bind(&org.member.key)
        .bind(&org.member.roster_hash)
        .bind(&org.member.owners_hash)
        .bind(pending_request)
        .execute(&self.pool)
        .await;
        if let Err(sqlx::Error::Database(db)) = &added {
            if db.is_unique_violation() {
                return Err(Error::Conflict(format!(
                    "the org '{}' is already joined on this PC",
                    org.id
                )));
            }
        }
        added?;
        Ok(())
    }

    /// The org `id`, if it is joined.
    pub async fn get_org(&self, id: &str) -> Result<Option<OrgRow>> {
        let row = sqlx::query(&format!("SELECT {ORG_COLUMNS} FROM orgs WHERE id = ?"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(org_from_row).transpose()
    }

    /// Every joined org, by name without regard to case.
    pub async fn list_orgs(&self) -> Result<Vec<OrgRow>> {
        let rows = sqlx::query(&format!(
            "SELECT {ORG_COLUMNS} FROM orgs ORDER BY name COLLATE NOCASE, id"
        ))
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(org_from_row).collect()
    }

    /// The id and name of every joined org, in `list_orgs`' order. It does not read the source
    /// settings, so one damaged row does not hide the others: the `Orgs` service reads each row
    /// on its own with `get_org`.
    pub async fn list_org_names(&self) -> Result<Vec<(String, String)>> {
        let rows = sqlx::query("SELECT id, name FROM orgs ORDER BY name COLLATE NOCASE, id")
            .fetch_all(&self.pool)
            .await?;
        rows.iter()
            .map(|row| Ok((row.try_get("id")?, row.try_get("name")?)))
            .collect()
    }

    /// Replaces the org's settings with those of a new join code for the same id (§6.4,
    /// **Update**). What the last fetch accepted stays.
    pub async fn update_org_settings(
        &self,
        id: &str,
        name: &str,
        source: &SourceSettings,
        policy_key: &str,
    ) -> Result<()> {
        let done = sqlx::query("UPDATE orgs SET name = ?, source = ?, policy_key = ? WHERE id = ?")
            .bind(name)
            .bind(serde_json::to_string(source)?)
            .bind(policy_key)
            .bind(id)
            .execute(&self.pool)
            .await?;
        if done.rows_affected() == 0 {
            return Err(not_joined(id));
        }
        Ok(())
    }

    /// Records a good fetch at `last_fetch_at`, whose policy was issued at `last_issued_at`.
    pub async fn record_org_fetch(
        &self,
        id: &str,
        last_fetch_at: &str,
        last_issued_at: &str,
    ) -> Result<()> {
        let done =
            sqlx::query("UPDATE orgs SET last_fetch_at = ?, last_issued_at = ? WHERE id = ?")
                .bind(last_fetch_at)
                .bind(last_issued_at)
                .bind(id)
                .execute(&self.pool)
                .await?;
        if done.rows_affected() == 0 {
            return Err(not_joined(id));
        }
        Ok(())
    }

    /// Records this PC's place on the org's member list, worked out after a fetch (members
    /// spec §6.4).
    pub async fn record_membership(&self, id: &str, member: &MemberRow) -> Result<()> {
        let (workspaces, seq) = member_columns(member)?;
        let done = sqlx::query(
            "UPDATE orgs SET membership = ?, my_member_id = ?, my_role = ?, my_workspaces = ?, \
             last_roster_seq = ?, my_key = ?, last_roster_hash = ?, last_owners_hash = ? WHERE id = ?",
        )
        .bind(member.membership.as_str())
        .bind(&member.member_id)
        .bind(member.role.map(|role| role.as_str()))
        .bind(workspaces)
        .bind(seq)
        .bind(&member.key)
        .bind(&member.roster_hash)
        .bind(&member.owners_hash)
        .bind(id)
        .execute(&self.pool)
        .await?;
        if done.rows_affected() == 0 {
            return Err(not_joined(id));
        }
        Ok(())
    }

    /// Records a good fetch and this PC's place on the member list it brought, in one write
    /// (`record_org_fetch` and `record_membership` together), so a refresh never keeps one
    /// without the other.
    pub async fn record_org_fetch_and_membership(
        &self,
        id: &str,
        last_fetch_at: &str,
        last_issued_at: &str,
        member: &MemberRow,
    ) -> Result<()> {
        let (workspaces, seq) = member_columns(member)?;
        let done = sqlx::query(
            "UPDATE orgs SET last_fetch_at = ?, last_issued_at = ?, membership = ?, \
             my_member_id = ?, my_role = ?, my_workspaces = ?, last_roster_seq = ?, my_key = ?, \
             last_roster_hash = ?, last_owners_hash = ? WHERE id = ?",
        )
        .bind(last_fetch_at)
        .bind(last_issued_at)
        .bind(member.membership.as_str())
        .bind(&member.member_id)
        .bind(member.role.map(|role| role.as_str()))
        .bind(workspaces)
        .bind(seq)
        .bind(&member.key)
        .bind(&member.roster_hash)
        .bind(&member.owners_hash)
        .bind(id)
        .execute(&self.pool)
        .await?;
        if done.rows_affected() == 0 {
            return Err(not_joined(id));
        }
        Ok(())
    }

    /// Sets (or with `None` clears) the request code this PC is waiting on for the org.
    pub async fn set_pending_request(
        &self,
        id: &str,
        request: Option<&PendingRequest>,
    ) -> Result<()> {
        let done = sqlx::query("UPDATE orgs SET pending_request = ? WHERE id = ?")
            .bind(request.map(serde_json::to_string).transpose()?)
            .bind(id)
            .execute(&self.pool)
            .await?;
        if done.rows_affected() == 0 {
            return Err(not_joined(id));
        }
        Ok(())
    }

    /// Removes the org's row. Its dots keep their `org_id` (§6.4). Its invites and roster draft
    /// go with it (their foreign keys cascade).
    pub async fn remove_org(&self, id: &str) -> Result<()> {
        let done = sqlx::query("DELETE FROM orgs WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        if done.rows_affected() == 0 {
            return Err(not_joined(id));
        }
        Ok(())
    }
}
