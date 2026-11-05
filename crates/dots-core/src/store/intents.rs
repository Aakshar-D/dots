//! `intents`: the person's goals. Each write is one statement, so nothing is read and then
//! written in a transaction.

use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::Store;
use crate::intents::{IntentFields, GONE};
use crate::model::Intent;
use crate::util::{new_id, now};
use crate::{Error, Result};

const COLUMNS: &str =
    "id, org_id, workspace_id, text, due_date, urgent, dot_id, created_at, done_at";

fn intent_from_row(row: &SqliteRow) -> Result<Intent> {
    Ok(Intent {
        id: row.try_get("id")?,
        org_id: row.try_get("org_id")?,
        workspace_id: row.try_get("workspace_id")?,
        text: row.try_get("text")?,
        due_date: row.try_get("due_date")?,
        urgent: row.try_get::<i64, _>("urgent")? != 0,
        dot_id: row.try_get("dot_id")?,
        created_at: row.try_get("created_at")?,
        done_at: row.try_get("done_at")?,
    })
}

fn gone() -> Error {
    Error::NotFound(GONE.into())
}

impl Store {
    /// The goals of the workspace `workspace_id` of `org_id` (`None` is Personal), oldest first.
    pub async fn list_intents(
        &self,
        org_id: Option<&str>,
        workspace_id: &str,
    ) -> Result<Vec<Intent>> {
        let rows = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM intents WHERE org_id IS ? AND workspace_id = ? \
             ORDER BY created_at, id"
        ))
        .bind(org_id)
        .bind(workspace_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(intent_from_row).collect()
    }

    /// The goal `id`; `NotFound` with `GONE` for one that isn't there.
    pub async fn get_intent(&self, id: &str) -> Result<Intent> {
        let row = sqlx::query(&format!("SELECT {COLUMNS} FROM intents WHERE id = ?"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(gone)?;
        intent_from_row(&row)
    }

    /// Stores a goal from `fields`, already checked (`intents::clean`).
    pub async fn add_intent(
        &self,
        org_id: Option<&str>,
        workspace_id: &str,
        fields: &IntentFields,
    ) -> Result<Intent> {
        let row = sqlx::query(&format!(
            "INSERT INTO intents (id, org_id, workspace_id, text, due_date, urgent, dot_id, \
             created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?) RETURNING {COLUMNS}"
        ))
        .bind(new_id())
        .bind(org_id)
        .bind(workspace_id)
        .bind(&fields.text)
        .bind(&fields.due_date)
        .bind(i64::from(fields.urgent))
        .bind(&fields.dot_id)
        .bind(now())
        .fetch_one(&self.pool)
        .await?;
        intent_from_row(&row)
    }

    /// Replaces the goal's text, date, urgency and dot with `fields`, already checked.
    pub async fn set_intent_fields(&self, id: &str, fields: &IntentFields) -> Result<Intent> {
        let row = sqlx::query(&format!(
            "UPDATE intents SET text = ?, due_date = ?, urgent = ?, dot_id = ? WHERE id = ? \
             RETURNING {COLUMNS}"
        ))
        .bind(&fields.text)
        .bind(&fields.due_date)
        .bind(i64::from(fields.urgent))
        .bind(&fields.dot_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(gone)?;
        intent_from_row(&row)
    }

    /// Marks the goal done (keeping the first time when it already is), or open again.
    pub async fn set_intent_done(&self, id: &str, done: bool) -> Result<Intent> {
        let row = sqlx::query(&format!(
            "UPDATE intents SET done_at = CASE WHEN ? THEN COALESCE(done_at, ?) ELSE NULL END \
             WHERE id = ? RETURNING {COLUMNS}"
        ))
        .bind(done)
        .bind(now())
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(gone)?;
        intent_from_row(&row)
    }

    /// Deletes the goal; `NotFound` with `GONE` for one that isn't there.
    pub async fn delete_intent(&self, id: &str) -> Result<()> {
        let done = sqlx::query("DELETE FROM intents WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        if done.rows_affected() == 0 {
            return Err(gone());
        }
        Ok(())
    }

    /// How many goals are handed to the dot `dot_id`.
    pub async fn count_intents_of_dot(&self, dot_id: &str) -> Result<usize> {
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM intents WHERE dot_id = ?")
            .bind(dot_id)
            .fetch_one(&self.pool)
            .await?;
        Ok(n as usize)
    }
}
