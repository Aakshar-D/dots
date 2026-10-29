//! The workspaces of each account on this PC (accounts design §6.3): `scope` is `personal` or
//! an org id. Every dot and team is in exactly one; the Default workspace is always there.

use sqlx::sqlite::SqliteRow;
use sqlx::{Row, Sqlite, Transaction};

use super::Store;
use crate::accounts::{Workspace, DEFAULT_WORKSPACE, PERSONAL};
use crate::util::now;
use crate::vault::org::{policy::plain_serde, UseRules};
use crate::{Error, Result};

/// One workspace as it is stored here.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceRow {
    /// `personal`, or the id of the org the workspace belongs to.
    pub scope: String,
    pub id: String,
    pub name: String,
    /// Which entries its dots may use, when the workspace narrows them.
    pub use_rules: Option<UseRules>,
    pub created_at: String,
}

const COLUMNS: &str = "scope, id, name, use_rules, created_at";
const INSERT: &str =
    "INSERT INTO workspaces (scope, id, name, use_rules, created_at) VALUES (?, ?, ?, ?, ?)";

fn from_row(row: &SqliteRow) -> Result<WorkspaceRow> {
    let scope: String = row.try_get("scope")?;
    let id: String = row.try_get("id")?;
    let use_rules = match row.try_get::<Option<String>, _>("use_rules")? {
        Some(text) => Some(serde_json::from_str(&text).map_err(|e| {
            Error::Other(format!(
                "workspace '{id}' of '{scope}': its stored use rules are not readable: {}",
                plain_serde(&e)
            ))
        })?),
        None => None,
    };
    Ok(WorkspaceRow {
        scope,
        id,
        name: row.try_get("name")?,
        use_rules,
        created_at: row.try_get("created_at")?,
    })
}

fn rules_json(rules: &Option<UseRules>) -> Result<Option<String>> {
    Ok(rules.as_ref().map(serde_json::to_string).transpose()?)
}

/// Moves the dots and teams of `scope` that are in `workspace` to the Default workspace.
async fn move_to_default(
    tx: &mut Transaction<'_, Sqlite>,
    scope: &str,
    workspace: &str,
) -> Result<()> {
    let org = (scope != PERSONAL).then_some(scope);
    for table in ["dots", "teams"] {
        sqlx::query(&format!(
            "UPDATE {table} SET workspace_id = ? WHERE workspace_id = ? AND org_id IS ?"
        ))
        .bind(DEFAULT_WORKSPACE)
        .bind(workspace)
        .bind(org)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

impl Store {
    /// The workspaces of `scope`: Default first, then by name.
    pub async fn list_workspaces(&self, scope: &str) -> Result<Vec<WorkspaceRow>> {
        let rows = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM workspaces WHERE scope = ? \
             ORDER BY id != 'default', name, id"
        ))
        .bind(scope)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(from_row).collect()
    }

    pub async fn workspace(&self, scope: &str, id: &str) -> Result<Option<WorkspaceRow>> {
        let row = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM workspaces WHERE scope = ? AND id = ?"
        ))
        .bind(scope)
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(from_row).transpose()
    }

    /// Adds a workspace; `Conflict` when its id is taken in that scope.
    pub async fn add_workspace(&self, row: &WorkspaceRow) -> Result<()> {
        let added = sqlx::query(&format!(
            "INSERT INTO workspaces ({COLUMNS}) VALUES (?, ?, ?, ?, ?)"
        ))
        .bind(&row.scope)
        .bind(&row.id)
        .bind(&row.name)
        .bind(rules_json(&row.use_rules)?)
        .bind(&row.created_at)
        .execute(&self.pool)
        .await;
        if let Err(sqlx::Error::Database(db)) = &added {
            if db.is_unique_violation() {
                return Err(Error::Conflict(format!(
                    "a workspace '{}' already exists",
                    row.id
                )));
            }
        }
        added?;
        Ok(())
    }

    pub async fn rename_workspace(&self, scope: &str, id: &str, name: &str) -> Result<()> {
        let done = sqlx::query("UPDATE workspaces SET name = ? WHERE scope = ? AND id = ?")
            .bind(name)
            .bind(scope)
            .bind(id)
            .execute(&self.pool)
            .await?;
        if done.rows_affected() == 0 {
            return Err(Error::NotFound(format!("workspace {id}")));
        }
        Ok(())
    }

    /// Deletes a workspace; its dots and teams go to Default. Default itself stays.
    pub async fn delete_workspace(&self, scope: &str, id: &str) -> Result<()> {
        if id == DEFAULT_WORKSPACE {
            return Err(Error::Invalid(
                "The Default workspace can't be deleted.".into(),
            ));
        }
        let mut tx = self.pool.begin().await?;
        let done = sqlx::query("DELETE FROM workspaces WHERE scope = ? AND id = ?")
            .bind(scope)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        if done.rows_affected() == 0 {
            return Err(Error::NotFound(format!("workspace {id}")));
        }
        move_to_default(&mut tx, scope, id).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Makes the org's rows the roster's: names and `use` rules replaced, new ones added, the
    /// ones the roster dropped removed with their dots and teams moved to Default. Returns the
    /// ids that vanished. Default is kept even when the roster leaves it out.
    pub async fn mirror_workspaces(
        &self,
        org_id: &str,
        roster: &[Workspace],
    ) -> Result<Vec<String>> {
        let mut tx = self.pool.begin().await?;
        let old = sqlx::query("SELECT id, created_at FROM workspaces WHERE scope = ?")
            .bind(org_id)
            .fetch_all(&mut *tx)
            .await?;
        let mut old: Vec<(String, String)> = old
            .iter()
            .map(|r| Ok((r.try_get("id")?, r.try_get("created_at")?)))
            .collect::<Result<_>>()?;
        old.sort();
        sqlx::query("DELETE FROM workspaces WHERE scope = ?")
            .bind(org_id)
            .execute(&mut *tx)
            .await?;
        let ts = now();
        let insert = |id: &str, name: &str, rules: Option<String>| {
            let created = old
                .iter()
                .find(|(i, _)| i == id)
                .map_or_else(|| ts.clone(), |(_, c)| c.clone());
            sqlx::query(INSERT)
                .bind(org_id.to_string())
                .bind(id.to_string())
                .bind(name.to_string())
                .bind(rules)
                .bind(created)
        };
        for w in roster {
            insert(&w.id, &w.name, rules_json(&w.use_rules)?)
                .execute(&mut *tx)
                .await?;
        }
        if !roster.iter().any(|w| w.id == DEFAULT_WORKSPACE) {
            insert(DEFAULT_WORKSPACE, "Default", None)
                .execute(&mut *tx)
                .await?;
        }
        let mut vanished = Vec::new();
        for (id, _) in &old {
            if id != DEFAULT_WORKSPACE && !roster.iter().any(|w| &w.id == id) {
                move_to_default(&mut tx, org_id, id).await?;
                vanished.push(id.clone());
            }
        }
        tx.commit().await?;
        Ok(vanished)
    }
}
