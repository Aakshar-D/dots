use serde_json::Value;
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::Store;
use crate::model::{ApplyState, Approval, ApprovalStatus};
use crate::util::{json_hash, new_id, now};
use crate::{Error, Result};

const APPROVAL_COLUMNS: &str = "id, run_id, tool, input, input_hash, status, note, parked, \
    resolved, created_at, decided_at, self_apply, apply_state, applied_at";

fn approval_from_row(row: &SqliteRow) -> Result<Approval> {
    Ok(Approval {
        id: row.try_get("id")?,
        run_id: row.try_get("run_id")?,
        tool: row.try_get("tool")?,
        input: serde_json::from_str(&row.try_get::<String, _>("input")?)?,
        input_hash: row.try_get("input_hash")?,
        status: ApprovalStatus::parse(&row.try_get::<String, _>("status")?)?,
        note: row.try_get("note")?,
        parked: row.try_get::<i64, _>("parked")? != 0,
        resolved: row.try_get::<i64, _>("resolved")? != 0,
        created_at: row.try_get("created_at")?,
        decided_at: row.try_get("decided_at")?,
        self_apply: row.try_get::<i64, _>("self_apply")? != 0,
        apply_state: row
            .try_get::<Option<String>, _>("apply_state")?
            .as_deref()
            .map(ApplyState::parse)
            .transpose()?,
        applied_at: row.try_get("applied_at")?,
    })
}

impl Store {
    pub async fn create_approval(
        &self,
        run_id: &str,
        tool: &str,
        input: &Value,
    ) -> Result<Approval> {
        self.create_approval_with(run_id, tool, input, false).await
    }

    /// Like `create_approval`. `self_apply` records that Dots, not the model, performs the call
    /// when it is approved after the engine stopped waiting.
    pub async fn create_approval_with(
        &self,
        run_id: &str,
        tool: &str,
        input: &Value,
        self_apply: bool,
    ) -> Result<Approval> {
        let id = new_id();
        sqlx::query(
            "INSERT INTO approvals (id, run_id, tool, input, input_hash, status, created_at, \
             self_apply) VALUES (?, ?, ?, ?, ?, 'pending', ?, ?)",
        )
        .bind(&id)
        .bind(run_id)
        .bind(tool)
        .bind(input.to_string())
        .bind(json_hash(input))
        .bind(now())
        .bind(i64::from(self_apply))
        .execute(&self.pool)
        .await?;
        self.get_approval(&id).await
    }

    pub async fn get_approval(&self, id: &str) -> Result<Approval> {
        let row = sqlx::query(&format!(
            "SELECT {APPROVAL_COLUMNS} FROM approvals WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| Error::NotFound(format!("approval {id}")))?;
        approval_from_row(&row)
    }

    pub async fn list_pending_approvals(&self) -> Result<Vec<Approval>> {
        let rows = sqlx::query(&format!(
            "SELECT {APPROVAL_COLUMNS} FROM approvals WHERE status = 'pending' \
             ORDER BY created_at, id"
        ))
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(approval_from_row).collect()
    }

    pub async fn approvals_for_run(&self, run_id: &str) -> Result<Vec<Approval>> {
        let rows = sqlx::query(&format!(
            "SELECT {APPROVAL_COLUMNS} FROM approvals WHERE run_id = ? ORDER BY created_at, id"
        ))
        .bind(run_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(approval_from_row).collect()
    }

    pub async fn has_pending_for_run(&self, run_id: &str) -> Result<bool> {
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM approvals WHERE run_id = ? AND status = 'pending'",
        )
        .bind(run_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(n > 0)
    }

    pub async fn decide_approval(
        &self,
        id: &str,
        approved: bool,
        note: Option<&str>,
    ) -> Result<Approval> {
        self.decide(id, approved, note, None).await
    }

    /// Records a decision and the `parked` flag in one guarded update, so a decided row
    /// always says whether a live engine received it. `Conflict` if no longer pending.
    pub async fn decide_approval_with_parked(
        &self,
        id: &str,
        approved: bool,
        note: Option<&str>,
        parked: bool,
    ) -> Result<Approval> {
        self.decide(id, approved, note, Some(parked)).await
    }

    async fn decide(
        &self,
        id: &str,
        approved: bool,
        note: Option<&str>,
        parked: Option<bool>,
    ) -> Result<Approval> {
        let status = if approved {
            ApprovalStatus::Approved
        } else {
            ApprovalStatus::Denied
        };
        let res = sqlx::query(
            "UPDATE approvals SET status = ?, note = ?, decided_at = ?, \
             parked = COALESCE(?, parked) WHERE id = ? AND status = 'pending'",
        )
        .bind(status.as_str())
        .bind(note)
        .bind(now())
        .bind(parked.map(i64::from))
        .bind(id)
        .execute(&self.pool)
        .await?;
        if res.rows_affected() == 0 {
            let existing = self.get_approval(id).await?;
            return Err(Error::Conflict(format!(
                "approval {id} is already {}",
                existing.status.as_str()
            )));
        }
        self.get_approval(id).await
    }

    pub async fn mark_parked(&self, id: &str) -> Result<()> {
        sqlx::query("UPDATE approvals SET parked = 1 WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn expire_pending_for_run(&self, run_id: &str) -> Result<u64> {
        let res = sqlx::query(
            "UPDATE approvals SET status = 'expired', decided_at = ? \
             WHERE run_id = ? AND status = 'pending'",
        )
        .bind(now())
        .bind(run_id)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected())
    }

    pub async fn unresolved_parked_decisions(&self, run_id: &str) -> Result<Vec<Approval>> {
        let rows = sqlx::query(&format!(
            "SELECT {APPROVAL_COLUMNS} FROM approvals WHERE run_id = ? AND parked = 1 \
             AND resolved = 0 AND status IN ('approved', 'denied') ORDER BY created_at, id"
        ))
        .bind(run_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(approval_from_row).collect()
    }

    pub async fn mark_resolved(&self, id: &str) -> Result<()> {
        sqlx::query("UPDATE approvals SET resolved = 1 WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// The approved calls of `run_id` that wait for Dots to perform them, in the order they
    /// were asked. That is the order of insertion (`rowid`): `created_at` has millisecond
    /// resolution, and two ids made in the same millisecond are in no order.
    pub async fn queued_applies(&self, run_id: &str) -> Result<Vec<Approval>> {
        let rows = sqlx::query(&format!(
            "SELECT {APPROVAL_COLUMNS} FROM approvals WHERE run_id = ? AND apply_state = 'queued' \
             ORDER BY rowid"
        ))
        .bind(run_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(approval_from_row).collect()
    }

    /// Claims a queued call for the caller to perform. False when the call is not queued (any
    /// more), so of several callers exactly one wins.
    pub async fn claim_apply(&self, id: &str) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE approvals SET apply_state = 'applying' WHERE id = ? AND apply_state = 'queued'",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() == 1)
    }

    /// Records the outcome of a claimed call, `applied` or `failed`, and when. False when the
    /// call is not `applying`: an outcome is written once.
    pub async fn settle_apply(&self, id: &str, state: ApplyState) -> Result<bool> {
        if !matches!(state, ApplyState::Applied | ApplyState::Failed) {
            return Err(Error::Invalid(format!(
                "a call is settled as applied or failed, not as {}",
                state.as_str()
            )));
        }
        let res = sqlx::query(
            "UPDATE approvals SET apply_state = ?, applied_at = ? \
             WHERE id = ? AND apply_state = 'applying'",
        )
        .bind(state.as_str())
        .bind(now())
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() == 1)
    }

    /// Gives up a queued call that will not be performed. False when it is not queued.
    pub async fn skip_apply(&self, id: &str) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE approvals SET apply_state = 'skipped' WHERE id = ? AND apply_state = 'queued'",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() == 1)
    }

    /// Closes what a resume run left open on the approvals of its parent `run_id`: a claimed
    /// call without a result becomes `unknown`, a queued one `skipped`. Returns the approvals
    /// it changed, in the order they were asked.
    pub async fn close_applies(&self, run_id: &str) -> Result<Vec<Approval>> {
        let mut rows: Vec<(i64, String)> = sqlx::query_as(
            "UPDATE approvals SET apply_state = CASE apply_state WHEN 'applying' THEN 'unknown' \
             ELSE 'skipped' END WHERE run_id = ? AND apply_state IN ('queued', 'applying') \
             RETURNING rowid, id",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await?;
        rows.sort();
        let mut changed = Vec::new();
        for (_, id) in rows {
            changed.push(self.get_approval(&id).await?);
        }
        Ok(changed)
    }

    pub async fn create_grant(
        &self,
        root_run_id: &str,
        tool: &str,
        input_hash: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO grants (id, root_run_id, tool, input_hash, created_at) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(new_id())
        .bind(root_run_id)
        .bind(tool)
        .bind(input_hash)
        .bind(now())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Consumes one unused grant; returns whether one existed.
    pub async fn take_grant(
        &self,
        root_run_id: &str,
        tool: &str,
        input_hash: &str,
    ) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE grants SET used = 1 WHERE id = (SELECT id FROM grants WHERE root_run_id = ? \
             AND tool = ? AND input_hash = ? AND used = 0 LIMIT 1)",
        )
        .bind(root_run_id)
        .bind(tool)
        .bind(input_hash)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() == 1)
    }
}
