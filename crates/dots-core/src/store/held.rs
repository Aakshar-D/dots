//! `watcher_held`: the events a watcher took outside its hours, kept until it opens (watch
//! hours spec section 4.2). Oldest first means `held_at`, then insertion order.

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::Store;
use crate::util::{new_id, timestamp};
use crate::Result;

/// An event held for a watcher: its payload as `fire` would have used it, and the files staged
/// for it, if any.
#[derive(Debug, Clone, PartialEq)]
pub struct HeldEvent {
    pub id: String,
    pub watcher_id: String,
    pub payload: Value,
    pub files: Option<Value>,
    pub held_at: String,
}

const SELECT: &str = "SELECT id, watcher_id, payload, files, held_at FROM watcher_held \
                      WHERE watcher_id = ? ORDER BY held_at, rowid";

fn held_from_row(row: &SqliteRow) -> Result<HeldEvent> {
    let payload: String = row.try_get("payload")?;
    let files: Option<String> = row.try_get("files")?;
    Ok(HeldEvent {
        id: row.try_get("id")?,
        watcher_id: row.try_get("watcher_id")?,
        payload: serde_json::from_str(&payload)?,
        files: files.as_deref().map(serde_json::from_str).transpose()?,
        held_at: row.try_get("held_at")?,
    })
}

impl Store {
    pub async fn hold_event(
        &self,
        watcher_id: &str,
        payload: &Value,
        files: Option<&Value>,
        at: DateTime<Utc>,
    ) -> Result<HeldEvent> {
        let id = new_id();
        let held_at = timestamp(at);
        sqlx::query(
            "INSERT INTO watcher_held (id, watcher_id, payload, files, held_at) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(watcher_id)
        .bind(serde_json::to_string(payload)?)
        .bind(files.map(serde_json::to_string).transpose()?)
        .bind(&held_at)
        .execute(&self.pool)
        .await?;
        Ok(HeldEvent {
            id,
            watcher_id: watcher_id.to_string(),
            payload: payload.clone(),
            files: files.cloned(),
            held_at,
        })
    }

    pub async fn held_count(&self, watcher_id: &str) -> Result<u32> {
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM watcher_held WHERE watcher_id = ?")
            .bind(watcher_id)
            .fetch_one(&self.pool)
            .await?;
        Ok(n as u32)
    }

    /// The watcher's held events, oldest first.
    pub async fn held_events(&self, watcher_id: &str) -> Result<Vec<HeldEvent>> {
        let rows = sqlx::query(SELECT)
            .bind(watcher_id)
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(held_from_row).collect()
    }

    /// Takes every held event: deleted and returned by one statement (so a hold landing
    /// meanwhile cannot make it fail), then sorted oldest first by `held_at`, then `rowid`.
    pub async fn take_held(&self, watcher_id: &str) -> Result<Vec<HeldEvent>> {
        let rows = sqlx::query(
            "DELETE FROM watcher_held WHERE watcher_id = ?              RETURNING id, watcher_id, payload, files, held_at, rowid AS row_no",
        )
        .bind(watcher_id)
        .fetch_all(&self.pool)
        .await?;
        let mut held = rows
            .iter()
            .map(|row| Ok((row.try_get::<i64, _>("row_no")?, held_from_row(row)?)))
            .collect::<Result<Vec<_>>>()?;
        held.sort_by(|(a_no, a), (b_no, b)| (&a.held_at, a_no).cmp(&(&b.held_at, b_no)));
        Ok(held.into_iter().map(|(_, h)| h).collect())
    }

    /// Drops every held event; they are returned so their staged files can be removed.
    pub async fn drop_held(&self, watcher_id: &str) -> Result<Vec<HeldEvent>> {
        self.take_held(watcher_id).await
    }

    /// Removes one held event (its files could not be staged).
    pub async fn unhold_event(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM watcher_held WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
