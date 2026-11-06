//! `watcher_held`: the events a watcher took outside its hours, kept until it opens (watch
//! hours spec section 4.2). Oldest first means `held_at`, then insertion order.

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::runs::insert_run;
use super::Store;
use crate::model::{NewRun, Run};
use crate::util::{new_id, timestamp};
use crate::{Error, Result};

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

    /// When the oldest held event was held (`held_at`), if the watcher holds any.
    pub async fn oldest_held_at(&self, watcher_id: &str) -> Result<Option<String>> {
        let at = sqlx::query_scalar("SELECT MIN(held_at) FROM watcher_held WHERE watcher_id = ?")
            .bind(watcher_id)
            .fetch_one(&self.pool)
            .await?;
        Ok(at)
    }

    /// The opening's one step (watch hours spec section 4.4): queues `new` and takes the
    /// watcher's held events, in one transaction, so they are never both carried by a run and
    /// still held, nor gone without one. `ids` are the events the run carries or counts; when
    /// the events taken are not exactly those, nothing is written (`Conflict`). The caller
    /// announces the run (`Runner::enqueued`).
    pub async fn open_held(
        &self,
        watcher_id: &str,
        ids: &[String],
        new: &NewRun,
    ) -> Result<(Run, Vec<HeldEvent>)> {
        let mut tx = self.pool.begin().await?;
        let run_id = insert_run(&mut tx, new).await?;
        let rows = sqlx::query(
            "DELETE FROM watcher_held WHERE watcher_id = ? \
             RETURNING id, watcher_id, payload, files, held_at",
        )
        .bind(watcher_id)
        .fetch_all(&mut *tx)
        .await?;
        let taken = rows.iter().map(held_from_row).collect::<Result<Vec<_>>>()?;
        let mut taken_ids: Vec<&str> = taken.iter().map(|e| e.id.as_str()).collect();
        let mut read_ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        taken_ids.sort_unstable();
        read_ids.sort_unstable();
        if taken_ids != read_ids {
            tx.rollback().await?;
            return Err(Error::Conflict(format!(
                "the held events of watcher {watcher_id} changed while it opened"
            )));
        }
        tx.commit().await?;
        let run = self.get_run(&run_id).await?;
        Ok((run, taken))
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
