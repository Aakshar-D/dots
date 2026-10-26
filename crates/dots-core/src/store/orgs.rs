//! The orgs joined on this PC (env vault spec §5.2): their settings, never a secret.

use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::Store;
use crate::vault::org::policy::plain_serde;
use crate::vault::org::{check_org_id, OrgRow, SourceSettings};
use crate::{Error, Result};

const ORG_COLUMNS: &str = "id, name, source, policy_key, last_issued_at, last_fetch_at, joined_at";

fn org_from_row(row: &SqliteRow) -> Result<OrgRow> {
    let id: String = row.try_get("id")?;
    let source = serde_json::from_str(&row.try_get::<String, _>("source")?).map_err(|e| {
        Error::Other(format!(
            "org '{id}': its stored source settings are not readable: {}",
            plain_serde(&e)
        ))
    })?;
    Ok(OrgRow {
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
        let added = sqlx::query(&format!(
            "INSERT INTO orgs ({ORG_COLUMNS}) VALUES (?, ?, ?, ?, ?, ?, ?)"
        ))
        .bind(&org.id)
        .bind(&org.name)
        .bind(serde_json::to_string(&org.source)?)
        .bind(&org.policy_key)
        .bind(&org.last_issued_at)
        .bind(&org.last_fetch_at)
        .bind(&org.joined_at)
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

    /// Removes the org's row. Its dots keep their `org_id` (§6.4).
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
