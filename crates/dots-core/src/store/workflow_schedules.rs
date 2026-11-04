//! The `workflow_schedule_state` rows: where each scheduled workflow's schedule stands (teams
//! spec section 5.1). Kept as a dot's `schedule_state` is (`store/schedules.rs`), without a
//! watched run, a retry or attention: a workflow schedule never catches up and never retries.
//! The scheduler's pass makes, resets and deletes the rows and handles their slots. Callers
//! pass `now`, so the scheduler's clock decides every time written here.

use chrono::{DateTime, Utc};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::Store;
use crate::model::{ScheduleStatus, WorkflowScheduleState};
use crate::schedule::Schedule;
use crate::util::timestamp;
use crate::Result;

const STATE_COLUMNS: &str = "workflow_id, rule_key, status, last_slot_at, fired_count, updated_at";

fn state_from_row(row: &SqliteRow) -> Result<WorkflowScheduleState> {
    Ok(WorkflowScheduleState {
        workflow_id: row.try_get("workflow_id")?,
        rule_key: row.try_get("rule_key")?,
        status: ScheduleStatus::parse(&row.try_get::<String, _>("status")?)?,
        last_slot_at: row.try_get("last_slot_at")?,
        fired_count: row.try_get::<i64, _>("fired_count")? as u32,
        updated_at: row.try_get("updated_at")?,
    })
}

impl Store {
    /// The workflow's schedule row, if it has one.
    pub async fn get_workflow_schedule_state(
        &self,
        workflow_id: &str,
    ) -> Result<Option<WorkflowScheduleState>> {
        let row = sqlx::query(&format!(
            "SELECT {STATE_COLUMNS} FROM workflow_schedule_state WHERE workflow_id = ?"
        ))
        .bind(workflow_id)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(state_from_row).transpose()
    }

    /// Every workflow's schedule row, by workflow id.
    pub async fn list_workflow_schedule_states(&self) -> Result<Vec<WorkflowScheduleState>> {
        let rows = sqlx::query(&format!(
            "SELECT {STATE_COLUMNS} FROM workflow_schedule_state ORDER BY workflow_id"
        ))
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(state_from_row).collect()
    }

    /// Makes the workflow's row for `schedule`, or resets it when its `rule_key` is not the
    /// schedule's, as `sync_schedule_state` does a dot's: the row is `active`, with
    /// `last_slot_at = now` and no count, so no slot from before is ever started. A row whose
    /// key is the schedule's is left as it is. Returns the row.
    pub async fn sync_workflow_schedule_state(
        &self,
        workflow_id: &str,
        schedule: &Schedule,
        now: DateTime<Utc>,
    ) -> Result<WorkflowScheduleState> {
        let ts = timestamp(now);
        sqlx::query(
            "INSERT INTO workflow_schedule_state (workflow_id, rule_key, status, last_slot_at, \
             fired_count, updated_at) VALUES (?, ?, ?, ?, 0, ?) \
             ON CONFLICT (workflow_id) DO UPDATE SET rule_key = excluded.rule_key, \
             status = excluded.status, last_slot_at = excluded.last_slot_at, fired_count = 0, \
             updated_at = excluded.updated_at \
             WHERE workflow_schedule_state.rule_key <> excluded.rule_key",
        )
        .bind(workflow_id)
        .bind(schedule.rule_key())
        .bind(ScheduleStatus::Active.as_str())
        .bind(&ts)
        .bind(&ts)
        .execute(&self.pool)
        .await?;
        let row = sqlx::query(&format!(
            "SELECT {STATE_COLUMNS} FROM workflow_schedule_state WHERE workflow_id = ?"
        ))
        .bind(workflow_id)
        .fetch_one(&self.pool)
        .await?;
        state_from_row(&row)
    }

    /// Deletes the workflow's row. True when there was one.
    pub async fn delete_workflow_schedule_state(&self, workflow_id: &str) -> Result<bool> {
        let res = sqlx::query("DELETE FROM workflow_schedule_state WHERE workflow_id = ?")
            .bind(workflow_id)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Takes the due slot `at`: `last_slot_at = at`. Written only while the row's
    /// `last_slot_at` is before `at`, so a slot is taken once, before its run is started; false
    /// when nothing was written (no row, or the slot was taken already).
    pub async fn record_workflow_slot(
        &self,
        workflow_id: &str,
        at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        let slot = timestamp(at);
        let res = sqlx::query(
            "UPDATE workflow_schedule_state SET last_slot_at = ?, updated_at = ? \
             WHERE workflow_id = ? AND last_slot_at < ?",
        )
        .bind(&slot)
        .bind(timestamp(now))
        .bind(workflow_id)
        .bind(&slot)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Counts a slot that started a workflow run.
    pub async fn count_workflow_slot(&self, workflow_id: &str, now: DateTime<Utc>) -> Result<()> {
        sqlx::query(
            "UPDATE workflow_schedule_state SET fired_count = fired_count + 1, updated_at = ? \
             WHERE workflow_id = ?",
        )
        .bind(timestamp(now))
        .bind(workflow_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Writes `status` while the row's `last_slot_at` is still `last_slot_at`, as the pass left
    /// it. True when written.
    pub async fn set_workflow_schedule_status(
        &self,
        workflow_id: &str,
        status: ScheduleStatus,
        last_slot_at: &str,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE workflow_schedule_state SET status = ?, updated_at = ? \
             WHERE workflow_id = ? AND last_slot_at = ?",
        )
        .bind(status.as_str())
        .bind(timestamp(now))
        .bind(workflow_id)
        .bind(last_slot_at)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    /// A resume after a pause, as `resume_schedules` does for dots: every active row's
    /// `last_slot_at` before `now` becomes `now`, so no slot of the pause is started. Returns
    /// the workflows whose row moved.
    pub async fn resume_workflow_schedules(&self, now: DateTime<Utc>) -> Result<Vec<String>> {
        let ts = timestamp(now);
        let ids = sqlx::query_scalar(
            "UPDATE workflow_schedule_state SET last_slot_at = ?, updated_at = ? \
             WHERE status = ? AND last_slot_at < ? RETURNING workflow_id",
        )
        .bind(&ts)
        .bind(&ts)
        .bind(ScheduleStatus::Active.as_str())
        .bind(&ts)
        .fetch_all(&self.pool)
        .await?;
        Ok(ids)
    }

    /// Whether the workflow has a run that has not ended (running or waiting).
    pub async fn workflow_has_runs_in_flight(&self, workflow_id: &str) -> Result<bool> {
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM workflow_runs WHERE workflow_id = ? \
             AND status IN ('running', 'waiting')",
        )
        .bind(workflow_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(n > 0)
    }
}
