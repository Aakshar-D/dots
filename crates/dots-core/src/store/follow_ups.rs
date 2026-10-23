//! The `follow_ups` rows: the runs dots scheduled for themselves (check-ins spec sections 6
//! and 7). The follow-up loop (`followups.rs`) fires a due row with `fire_follow_up`, which
//! stores its run in the same transaction, and retries or skips a row whose run could not be
//! stored. Callers pass the times, so the loop's clock decides every time written here.

use chrono::{DateTime, Utc};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::runs::insert_run;
use super::Store;
use crate::model::{FollowUp, FollowUpStatus, NewRun, Run};
use crate::util::{new_id, timestamp};
use crate::vault::redact::global;
use crate::{Error, Result};

const FOLLOW_UP_COLUMNS: &str =
    "id, dot_id, from_run_id, due_at, note, status, fired_run_id, attempts, retry_at, created_at";

fn follow_up_from_row(row: &SqliteRow) -> Result<FollowUp> {
    Ok(FollowUp {
        id: row.try_get("id")?,
        dot_id: row.try_get("dot_id")?,
        from_run_id: row.try_get("from_run_id")?,
        due_at: row.try_get("due_at")?,
        note: row.try_get("note")?,
        status: FollowUpStatus::parse(&row.try_get::<String, _>("status")?)?,
        fired_run_id: row.try_get("fired_run_id")?,
        attempts: row.try_get::<i64, _>("attempts")? as u32,
        retry_at: row.try_get("retry_at")?,
        created_at: row.try_get("created_at")?,
    })
}

impl Store {
    /// Stores a pending follow-up of `dot_id`, unless the dot has `max_pending` pending ones
    /// already (`None`). The count and the insert are one statement, so two calls at once
    /// cannot pass the limit together.
    pub async fn add_follow_up(
        &self,
        dot_id: &str,
        from_run_id: &str,
        due_at: DateTime<Utc>,
        note: &str,
        max_pending: usize,
        now: DateTime<Utc>,
    ) -> Result<Option<FollowUp>> {
        let id = new_id();
        let res = sqlx::query(
            "INSERT INTO follow_ups (id, dot_id, from_run_id, due_at, note, status, created_at) \
             SELECT ?, ?, ?, ?, ?, 'pending', ? WHERE \
             (SELECT COUNT(*) FROM follow_ups WHERE dot_id = ? AND status = 'pending') < ?",
        )
        .bind(&id)
        .bind(dot_id)
        .bind(from_run_id)
        .bind(timestamp(due_at))
        .bind(global().scrub(note).as_ref())
        .bind(timestamp(now))
        .bind(dot_id)
        .bind(max_pending as i64)
        .execute(&self.pool)
        .await?;
        if res.rows_affected() == 0 {
            return Ok(None);
        }
        self.get_follow_up(&id).await.map(Some)
    }

    pub async fn get_follow_up(&self, id: &str) -> Result<FollowUp> {
        let row = sqlx::query(&format!(
            "SELECT {FOLLOW_UP_COLUMNS} FROM follow_ups WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| Error::NotFound(format!("follow-up {id}")))?;
        follow_up_from_row(&row)
    }

    /// The pending follow-ups, of one dot or of all, soonest first.
    pub async fn list_pending_follow_ups(&self, dot_id: Option<&str>) -> Result<Vec<FollowUp>> {
        let rows = sqlx::query(&format!(
            "SELECT {FOLLOW_UP_COLUMNS} FROM follow_ups WHERE status = 'pending' \
             AND (? IS NULL OR dot_id = ?) ORDER BY due_at, id"
        ))
        .bind(dot_id)
        .bind(dot_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(follow_up_from_row).collect()
    }

    /// Fires the pending follow-up `id`: stores `new` and marks the row `fired` with its run, in
    /// one transaction. `Conflict` when the row is no longer pending; nothing is written then.
    pub async fn fire_follow_up(&self, id: &str, new: &NewRun) -> Result<Run> {
        let mut tx = self.pool.begin().await?;
        let run_id = insert_run(&mut tx, new).await?;
        let res = sqlx::query(
            "UPDATE follow_ups SET status = 'fired', fired_run_id = ?, retry_at = NULL \
             WHERE id = ? AND status = 'pending'",
        )
        .bind(&run_id)
        .bind(id)
        .execute(&mut *tx)
        .await?;
        if res.rows_affected() == 0 {
            tx.rollback().await?;
            return Err(Error::Conflict(format!(
                "follow-up {id} is no longer pending"
            )));
        }
        tx.commit().await?;
        self.get_run(&run_id).await
    }

    /// Counts a failed start of the pending follow-up `id`: `attempts` goes up and the next try
    /// is at `retry_at`. False when the row is no longer pending.
    pub async fn retry_follow_up(&self, id: &str, retry_at: DateTime<Utc>) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE follow_ups SET attempts = attempts + 1, retry_at = ? \
             WHERE id = ? AND status = 'pending'",
        )
        .bind(timestamp(retry_at))
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() == 1)
    }

    /// Skips the pending follow-up `id`; `failed` counts the start that failed last. False when
    /// the row is no longer pending.
    pub async fn skip_follow_up(&self, id: &str, failed: bool) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE follow_ups SET status = 'skipped', retry_at = NULL, \
             attempts = attempts + ? WHERE id = ? AND status = 'pending'",
        )
        .bind(failed as i64)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() == 1)
    }

    /// Cancels the pending follow-up `id` (the person's; dots have no cancel tool). `Conflict`
    /// once it fired, was skipped or was cancelled.
    pub async fn cancel_follow_up(&self, id: &str) -> Result<FollowUp> {
        let res = sqlx::query(
            "UPDATE follow_ups SET status = 'cancelled', retry_at = NULL \
             WHERE id = ? AND status = 'pending'",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        let row = self.get_follow_up(id).await?;
        if res.rows_affected() == 0 {
            let why = match row.status {
                FollowUpStatus::Fired => "this follow-up has already started its run",
                FollowUpStatus::Skipped => "this follow-up was skipped",
                FollowUpStatus::Cancelled | FollowUpStatus::Pending => {
                    "this follow-up was cancelled already"
                }
            };
            return Err(Error::Conflict(why.into()));
        }
        Ok(row)
    }
}
