//! The `schedule_state` rows: where each dot's schedule stands (Scheduler v2 spec §4). The
//! runtime keeps a row in line with its dot (`sync_schedule_state`); the scheduler's pass
//! handles slots and retries with `record_slot` and `start_retry` and writes the rest with
//! `update_schedule_state`, each in one transaction. Callers pass `now`, so the scheduler's
//! clock decides every time written here. The judging of a watched run asks the runs what
//! it needs here too (`resume_child`, `called_a_tool`, `succeeded_after`), and the Inbox
//! lists the flags with `list_schedule_attention`.

use chrono::{DateTime, Utc};
use sqlx::sqlite::{SqliteConnection, SqliteRow};
use sqlx::Row;

use super::runs::insert_run;
use super::Store;
use crate::model::{
    NeedsAttention, NewRun, PendingRetry, ScheduleChange, ScheduleState, ScheduleStatus, SlotStart,
};
use crate::schedule::Schedule;
use crate::util::timestamp;
use crate::{Error, Result};

const STATE_COLUMNS: &str = "dot_id, rule_key, status, last_slot_at, fired_count, \
    watch_run_id, retry_slot_at, retry_attempt, retry_at, attention_run_id, updated_at";

fn state_from_row(row: &SqliteRow) -> Result<ScheduleState> {
    let retry = match (
        row.try_get::<Option<String>, _>("retry_slot_at")?,
        row.try_get::<Option<i64>, _>("retry_attempt")?,
        row.try_get::<Option<String>, _>("retry_at")?,
    ) {
        (Some(slot_at), Some(attempt), Some(at)) => Some(PendingRetry {
            slot_at,
            attempt: attempt as u32,
            at,
        }),
        _ => None,
    };
    Ok(ScheduleState {
        dot_id: row.try_get("dot_id")?,
        rule_key: row.try_get("rule_key")?,
        status: ScheduleStatus::parse(&row.try_get::<String, _>("status")?)?,
        last_slot_at: row.try_get("last_slot_at")?,
        fired_count: row.try_get::<i64, _>("fired_count")? as u32,
        watch_run_id: row.try_get("watch_run_id")?,
        retry,
        attention_run_id: row.try_get("attention_run_id")?,
        updated_at: row.try_get("updated_at")?,
    })
}

/// The dot's queued run if it has one (the oldest), or else `new` inserted; true when joined.
async fn start_or_join(conn: &mut SqliteConnection, new: &NewRun) -> Result<(String, bool)> {
    let queued: Option<String> = sqlx::query_scalar(
        "SELECT id FROM runs WHERE dot_id = ? AND status = 'queued' \
         ORDER BY queued_at, id LIMIT 1",
    )
    .bind(&new.dot_id)
    .fetch_optional(&mut *conn)
    .await?;
    match queued {
        Some(id) => Ok((id, true)),
        None => Ok((insert_run(conn, new).await?, false)),
    }
}

impl Store {
    /// The dot's schedule row, if it has one.
    pub async fn get_schedule_state(&self, dot_id: &str) -> Result<Option<ScheduleState>> {
        let row = sqlx::query(&format!(
            "SELECT {STATE_COLUMNS} FROM schedule_state WHERE dot_id = ?"
        ))
        .bind(dot_id)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(state_from_row).transpose()
    }

    /// The rows of the enabled dots, by dot id: what a scheduler pass works on. A disabled
    /// dot's row is left as it is.
    pub async fn list_schedule_states(&self) -> Result<Vec<ScheduleState>> {
        let rows = sqlx::query(&format!(
            "SELECT {STATE_COLUMNS} FROM schedule_state \
             WHERE dot_id IN (SELECT id FROM dots WHERE enabled = 1) ORDER BY dot_id"
        ))
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(state_from_row).collect()
    }

    /// Brings the dot's row in line with its schedule, in one statement (spec §4, lifecycle of
    /// a row). Without a schedule the row is deleted. With one, a missing row is created, and
    /// a row is reset when `reset` is set (the dot was enabled) or when its `rule_key` is not
    /// the schedule's. A new or reset row is `active`, with `last_slot_at = now`, no count, no
    /// watched run, no retry and no attention, so it never catches up a slot from before.
    /// True when the row changed.
    pub async fn sync_schedule_state(
        &self,
        dot_id: &str,
        schedule: Option<&Schedule>,
        reset: bool,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        let Some(schedule) = schedule else {
            let res = sqlx::query("DELETE FROM schedule_state WHERE dot_id = ?")
                .bind(dot_id)
                .execute(&self.pool)
                .await?;
            return Ok(res.rows_affected() > 0);
        };
        let ts = timestamp(now);
        let res = sqlx::query(
            "INSERT INTO schedule_state (dot_id, rule_key, status, last_slot_at, fired_count, \
             updated_at) VALUES (?, ?, ?, ?, 0, ?) \
             ON CONFLICT (dot_id) DO UPDATE SET rule_key = excluded.rule_key, \
             status = excluded.status, last_slot_at = excluded.last_slot_at, fired_count = 0, \
             watch_run_id = NULL, retry_slot_at = NULL, retry_attempt = NULL, retry_at = NULL, \
             attention_run_id = NULL, updated_at = excluded.updated_at \
             WHERE ? OR schedule_state.rule_key <> excluded.rule_key",
        )
        .bind(dot_id)
        .bind(schedule.rule_key())
        .bind(ScheduleStatus::Active.as_str())
        .bind(&ts)
        .bind(&ts)
        .bind(reset)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    /// A resume after a pause (spec §5, "Pause"): every active row's `last_slot_at` becomes
    /// `now`, so the slots of the pause are not caught up. A row that is already at or after
    /// `now` is left alone; the watch, a waiting retry and the attention stay. Returns the dots
    /// whose row moved.
    pub async fn resume_schedules(&self, now: DateTime<Utc>) -> Result<Vec<String>> {
        let ts = timestamp(now);
        let ids = sqlx::query_scalar(
            "UPDATE schedule_state SET last_slot_at = ?, updated_at = ? \
             WHERE status = ? AND last_slot_at < ? RETURNING dot_id",
        )
        .bind(&ts)
        .bind(&ts)
        .bind(ScheduleStatus::Active.as_str())
        .bind(&ts)
        .fetch_all(&self.pool)
        .await?;
        Ok(ids)
    }

    /// Handles the due slot `at`, in one transaction (spec §5). `fire: None` skips it.
    /// `Some(new)` fires it: the slot joins the dot's queued run if there is one, and `new`
    /// is inserted otherwise. Either way `last_slot_at = at`; a fired slot also counts, its
    /// run becomes the watched run, and a pending retry of an older slot is dropped (the
    /// caller gives `new` the dropped retry's `attempt`, so the count runs on).
    /// `Conflict`, and nothing is written, when the dot has no row or its `last_slot_at` is
    /// not before `at` (the row was reset or handled the slot after the caller read it).
    pub async fn record_slot(
        &self,
        dot_id: &str,
        at: DateTime<Utc>,
        fire: Option<&NewRun>,
        now: DateTime<Utc>,
    ) -> Result<SlotStart> {
        let slot = timestamp(at);
        let mut tx = self.pool.begin().await?;
        // The guarded update goes first: it takes the write lock, so the check and the run
        // below are one step.
        let res = sqlx::query(
            "UPDATE schedule_state SET last_slot_at = ?, updated_at = ? \
             WHERE dot_id = ? AND last_slot_at < ?",
        )
        .bind(&slot)
        .bind(timestamp(now))
        .bind(dot_id)
        .bind(&slot)
        .execute(&mut *tx)
        .await?;
        if res.rows_affected() == 0 {
            tx.rollback().await?;
            return Err(Error::Conflict(format!(
                "the schedule of dot {dot_id} has no row or has handled {slot} already"
            )));
        }
        let Some(new) = fire else {
            tx.commit().await?;
            return Ok(SlotStart::Skipped);
        };
        let (run_id, joined) = start_or_join(&mut tx, new).await?;
        sqlx::query(
            "UPDATE schedule_state SET fired_count = fired_count + 1, watch_run_id = ?, \
             retry_slot_at = NULL, retry_attempt = NULL, retry_at = NULL WHERE dot_id = ?",
        )
        .bind(&run_id)
        .bind(dot_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        let run = self.get_run(&run_id).await?;
        Ok(if joined {
            SlotStart::Coalesced(run)
        } else {
            SlotStart::Enqueued(run)
        })
    }

    /// Starts the pending retry, in one transaction (spec §6): it joins the dot's queued run
    /// if there is one, and `new` (which names the failed run in `retry_of`) is inserted
    /// otherwise. The run becomes the watched run and the retry fields are cleared; a retry
    /// does not count. `Conflict`, and nothing is written, when no retry is pending.
    pub async fn start_retry(
        &self,
        dot_id: &str,
        new: &NewRun,
        now: DateTime<Utc>,
    ) -> Result<SlotStart> {
        let mut tx = self.pool.begin().await?;
        let res = sqlx::query(
            "UPDATE schedule_state SET retry_slot_at = NULL, retry_attempt = NULL, \
             retry_at = NULL, updated_at = ? WHERE dot_id = ? AND retry_at IS NOT NULL",
        )
        .bind(timestamp(now))
        .bind(dot_id)
        .execute(&mut *tx)
        .await?;
        if res.rows_affected() == 0 {
            tx.rollback().await?;
            return Err(Error::Conflict(format!(
                "the schedule of dot {dot_id} has no pending retry"
            )));
        }
        let (run_id, joined) = start_or_join(&mut tx, new).await?;
        sqlx::query("UPDATE schedule_state SET watch_run_id = ? WHERE dot_id = ?")
            .bind(&run_id)
            .bind(dot_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        let run = self.get_run(&run_id).await?;
        Ok(if joined {
            SlotStart::Coalesced(run)
        } else {
            SlotStart::Enqueued(run)
        })
    }

    /// The resume run that took over from `run_id` after its approvals were decided, if any
    /// (the newest, should there be more than one).
    pub async fn resume_child(&self, run_id: &str) -> Result<Option<String>> {
        let id = sqlx::query_scalar(
            "SELECT id FROM runs WHERE parent_run_id = ? AND trigger_kind = 'resume' \
             ORDER BY queued_at DESC, id DESC LIMIT 1",
        )
        .bind(run_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(id)
    }

    /// Whether `run_id`, or a run it resumes (up its chain of parked parents), has a
    /// `tool_call` event: it may have changed something, so it is not retried (spec §6).
    pub async fn called_a_tool(&self, run_id: &str) -> Result<bool> {
        let called = sqlx::query_scalar(
            "WITH RECURSIVE chain(id) AS ( \
               SELECT ? \
               UNION SELECT r.parent_run_id FROM runs r JOIN chain c ON r.id = c.id \
               WHERE r.trigger_kind = 'resume' AND r.parent_run_id IS NOT NULL) \
             SELECT EXISTS (SELECT 1 FROM run_events \
               WHERE kind = 'tool_call' AND run_id IN (SELECT id FROM chain))",
        )
        .bind(run_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(called)
    }

    /// Whether a manual or scheduled run of `dot_id` queued after `run_id` has succeeded. A
    /// parked run closed by its decision does not count: its resume run carries the outcome.
    pub async fn succeeded_after(&self, dot_id: &str, run_id: &str) -> Result<bool> {
        let found = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM runs r, runs f \
             WHERE f.id = ? AND r.dot_id = ? AND r.id <> f.id \
             AND r.trigger_kind IN ('manual', 'schedule') AND r.status = 'succeeded' \
             AND r.queued_at >= f.queued_at \
             AND NOT EXISTS (SELECT 1 FROM runs c WHERE c.parent_run_id = r.id \
                             AND c.trigger_kind = 'resume'))",
        )
        .bind(run_id)
        .bind(dot_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(found)
    }

    /// The schedules that need attention (spec §8, the Inbox), newest failure first. A dot
    /// that is disabled keeps its flag until it is dismissed or the row is reset.
    pub async fn list_schedule_attention(&self) -> Result<Vec<NeedsAttention>> {
        let rows = sqlx::query(
            "SELECT s.dot_id, d.name, r.id, r.error, COALESCE(r.ended_at, r.queued_at) \
             AS failed_at FROM schedule_state s \
             JOIN dots d ON d.id = s.dot_id JOIN runs r ON r.id = s.attention_run_id \
             ORDER BY failed_at DESC, s.dot_id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok(NeedsAttention {
                    dot_id: row.try_get("dot_id")?,
                    dot_name: row.try_get("name")?,
                    run_id: row.try_get("id")?,
                    error: row.try_get("error")?,
                    failed_at: row.try_get("failed_at")?,
                })
            })
            .collect()
    }

    /// Writes the fields `change` names, in one statement, and returns the row. `None` when
    /// nothing was written: the dot has no row, the row does not watch `change.if_watching`,
    /// or its `last_slot_at` is not `change.if_last_slot_at` (a reset moved it after the
    /// caller read the row).
    pub async fn update_schedule_state(
        &self,
        dot_id: &str,
        change: &ScheduleChange,
        now: DateTime<Utc>,
    ) -> Result<Option<ScheduleState>> {
        let retry = change.retry.as_ref().map(Option::as_ref);
        let row = sqlx::query(&format!(
            "UPDATE schedule_state SET \
             status = CASE WHEN ? THEN ? ELSE status END, \
             watch_run_id = CASE WHEN ? THEN ? ELSE watch_run_id END, \
             retry_slot_at = CASE WHEN ? THEN ? ELSE retry_slot_at END, \
             retry_attempt = CASE WHEN ? THEN ? ELSE retry_attempt END, \
             retry_at = CASE WHEN ? THEN ? ELSE retry_at END, \
             attention_run_id = CASE WHEN ? THEN ? ELSE attention_run_id END, \
             updated_at = ? \
             WHERE dot_id = ? AND (? IS NULL OR watch_run_id = ?) \
             AND (? IS NULL OR last_slot_at = ?) \
             RETURNING {STATE_COLUMNS}"
        ))
        .bind(change.status.is_some())
        .bind(change.status.map(|s| s.as_str()))
        .bind(change.watch_run_id.is_some())
        .bind(change.watch_run_id.clone().flatten())
        .bind(retry.is_some())
        .bind(retry.flatten().map(|r| r.slot_at.clone()))
        .bind(retry.is_some())
        .bind(retry.flatten().map(|r| r.attempt as i64))
        .bind(retry.is_some())
        .bind(retry.flatten().map(|r| r.at.clone()))
        .bind(change.attention_run_id.is_some())
        .bind(change.attention_run_id.clone().flatten())
        .bind(timestamp(now))
        .bind(dot_id)
        .bind(&change.if_watching)
        .bind(&change.if_watching)
        .bind(&change.if_last_slot_at)
        .bind(&change.if_last_slot_at)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(state_from_row).transpose()
    }
}
