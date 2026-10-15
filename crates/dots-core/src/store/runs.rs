use serde_json::Value;
use sqlx::sqlite::{SqliteConnection, SqliteRow};
use sqlx::Row;

use super::{opt_json, Store};
use crate::model::{NewRun, Run, RunEventRecord, RunStatus, TriggerKind};
use crate::util::{new_id, now};
use crate::{Error, Result};

const RUN_COLUMNS: &str = "id, dot_id, root_run_id, parent_run_id, trigger_kind, payload, status, \
    session_id, workspace_path, branch, base_commit, summary, error, tokens_in, tokens_out, \
    queued_at, started_at, ended_at, reviewed_at, retry_of";

fn run_from_row(row: &SqliteRow) -> Result<Run> {
    Ok(Run {
        id: row.try_get("id")?,
        dot_id: row.try_get("dot_id")?,
        root_run_id: row.try_get("root_run_id")?,
        parent_run_id: row.try_get("parent_run_id")?,
        trigger: TriggerKind::parse(&row.try_get::<String, _>("trigger_kind")?)?,
        payload: opt_json(row.try_get("payload")?)?,
        status: RunStatus::parse(&row.try_get::<String, _>("status")?)?,
        session_id: row.try_get("session_id")?,
        workspace_path: row.try_get("workspace_path")?,
        branch: row.try_get("branch")?,
        base_commit: row.try_get("base_commit")?,
        summary: row.try_get("summary")?,
        error: row.try_get("error")?,
        tokens_in: row.try_get("tokens_in")?,
        tokens_out: row.try_get("tokens_out")?,
        queued_at: row.try_get("queued_at")?,
        started_at: row.try_get("started_at")?,
        ended_at: row.try_get("ended_at")?,
        reviewed_at: row.try_get("reviewed_at")?,
        retry_of: row.try_get("retry_of")?,
    })
}

fn event_from_row(row: &SqliteRow) -> Result<RunEventRecord> {
    Ok(RunEventRecord {
        run_id: row.try_get("run_id")?,
        seq: row.try_get("seq")?,
        ts: row.try_get("ts")?,
        kind: row.try_get("kind")?,
        data: serde_json::from_str(&row.try_get::<String, _>("data")?)?,
    })
}

pub(super) async fn insert_run(conn: &mut SqliteConnection, new: &NewRun) -> Result<String> {
    let id = new_id();
    let root = new.root_run_id.clone().unwrap_or_else(|| id.clone());
    sqlx::query(
        "INSERT INTO runs (id, dot_id, root_run_id, parent_run_id, trigger_kind, payload, \
         status, session_id, workspace_path, branch, base_commit, queued_at, retry_of) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&new.dot_id)
    .bind(&root)
    .bind(&new.parent_run_id)
    .bind(new.trigger.as_str())
    .bind(new.payload.as_ref().map(|v| v.to_string()))
    .bind(RunStatus::Queued.as_str())
    .bind(&new.session_id)
    .bind(&new.workspace_path)
    .bind(&new.branch)
    .bind(&new.base_commit)
    .bind(now())
    .bind(&new.retry_of)
    .execute(conn)
    .await?;
    Ok(id)
}

impl Store {
    pub async fn create_run(&self, new: &NewRun) -> Result<Run> {
        let mut conn = self.pool.acquire().await?;
        let id = insert_run(&mut conn, new).await?;
        drop(conn);
        self.get_run(&id).await
    }

    /// Atomically turns the decided parked approvals of `parent_id` into a resume:
    /// inserts the one-shot `grants` (`(tool, grant_key)`, scoped to the parent's
    /// `root_run_id`, read inside the transaction), marks `resolved_ids` resolved, queues
    /// `queued_ids` (approved calls that Dots performs itself in the child run, which get no
    /// grant), inserts the optional `child` run, and closes the parent as `succeeded`.
    /// The parent must still be `awaiting_approval` (e.g. not cancelled concurrently), and each
    /// of `queued_ids` an approved, self-apply, not yet queued approval of the parent;
    /// otherwise nothing is written and `Error::Conflict` is returned.
    pub async fn resume_parked(
        &self,
        parent_id: &str,
        grants: &[(String, String)],
        resolved_ids: &[String],
        queued_ids: &[String],
        child: Option<&NewRun>,
    ) -> Result<Option<Run>> {
        let mut tx = self.pool.begin().await?;
        // The guarded update goes first: it takes the write lock and makes
        // check-and-close atomic with everything below.
        let res = sqlx::query(
            "UPDATE runs SET status = 'succeeded' WHERE id = ? AND status = 'awaiting_approval'",
        )
        .bind(parent_id)
        .execute(&mut *tx)
        .await?;
        if res.rows_affected() == 0 {
            tx.rollback().await?;
            return Err(Error::Conflict(format!(
                "run {parent_id} is no longer awaiting approval"
            )));
        }
        let root: String = sqlx::query_scalar("SELECT root_run_id FROM runs WHERE id = ?")
            .bind(parent_id)
            .fetch_one(&mut *tx)
            .await?;
        for (tool, key) in grants {
            sqlx::query(
                "INSERT INTO grants (id, root_run_id, tool, input_hash, created_at) \
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(new_id())
            .bind(&root)
            .bind(tool)
            .bind(key)
            .bind(now())
            .execute(&mut *tx)
            .await?;
        }
        for id in resolved_ids {
            sqlx::query("UPDATE approvals SET resolved = 1 WHERE id = ?")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        for id in queued_ids {
            // Only an approved call of this run that Dots is to perform, and not yet queued.
            let res = sqlx::query(
                "UPDATE approvals SET apply_state = 'queued' WHERE id = ? AND run_id = ? \
                 AND status = 'approved' AND self_apply = 1 AND apply_state IS NULL",
            )
            .bind(id)
            .bind(parent_id)
            .execute(&mut *tx)
            .await?;
            if res.rows_affected() != 1 {
                tx.rollback().await?;
                return Err(Error::Conflict(format!(
                    "approval {id} is not an approved call of run {parent_id} for Dots to perform"
                )));
            }
        }
        let child_id = match child {
            Some(new) => Some(insert_run(&mut tx, new).await?),
            None => None,
        };
        tx.commit().await?;
        match child_id {
            Some(id) => Ok(Some(self.get_run(&id).await?)),
            None => Ok(None),
        }
    }

    pub async fn get_run(&self, id: &str) -> Result<Run> {
        let row = sqlx::query(&format!("SELECT {RUN_COLUMNS} FROM runs WHERE id = ?"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| Error::NotFound(format!("run {id}")))?;
        run_from_row(&row)
    }

    pub async fn list_runs(&self, dot_id: Option<&str>, limit: i64) -> Result<Vec<Run>> {
        let rows = match dot_id {
            Some(d) => {
                sqlx::query(&format!(
                    "SELECT {RUN_COLUMNS} FROM runs WHERE dot_id = ? \
                     ORDER BY queued_at DESC, id DESC LIMIT ?"
                ))
                .bind(d)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
            }
            None => {
                sqlx::query(&format!(
                    "SELECT {RUN_COLUMNS} FROM runs ORDER BY queued_at DESC, id DESC LIMIT ?"
                ))
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
            }
        };
        rows.iter().map(run_from_row).collect()
    }

    /// Every run of the lineage rooted at `root_run_id`, oldest first.
    pub async fn list_thread(&self, root_run_id: &str) -> Result<Vec<Run>> {
        let rows = sqlx::query(&format!(
            "SELECT {RUN_COLUMNS} FROM runs WHERE root_run_id = ? ORDER BY queued_at, id"
        ))
        .bind(root_run_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(run_from_row).collect()
    }

    /// The newest run of `dot_id`, optionally only among queued, running and awaiting runs.
    pub async fn latest_run(&self, dot_id: &str, active_only: bool) -> Result<Option<Run>> {
        let filter = if active_only {
            "AND status IN ('queued', 'running', 'awaiting_approval')"
        } else {
            ""
        };
        let row = sqlx::query(&format!(
            "SELECT {RUN_COLUMNS} FROM runs WHERE dot_id = ? {filter} \
             ORDER BY queued_at DESC, id DESC LIMIT 1"
        ))
        .bind(dot_id)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(run_from_row).transpose()
    }

    /// The newest run of each lineage, if it is finished and not reviewed yet. Only runs the
    /// Inbox can ever show (failed, or with a branch) are returned; `Runtime::inbox` filters further.
    /// A failed scheduled run that called no tool is left out (Scheduler v2): it changed
    /// nothing, and its occurrence is retried or ends in "Needs attention".
    pub async fn unreviewed_thread_ends(&self) -> Result<Vec<Run>> {
        let rows = sqlx::query(&format!(
            "SELECT {RUN_COLUMNS} FROM runs r \
             WHERE r.reviewed_at IS NULL AND r.status IN ('succeeded', 'failed', 'cancelled') \
             AND (r.status = 'failed' OR r.branch IS NOT NULL) \
             AND NOT (r.status = 'failed' AND r.trigger_kind = 'schedule' \
                      AND NOT EXISTS (SELECT 1 FROM run_events e \
                                      WHERE e.run_id = r.id AND e.kind = 'tool_call')) \
             AND r.id = (SELECT x.id FROM runs x WHERE x.root_run_id = r.root_run_id \
                         ORDER BY x.queued_at DESC, x.id DESC LIMIT 1) \
             ORDER BY r.ended_at DESC, r.id DESC"
        ))
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(run_from_row).collect()
    }

    /// Sets `reviewed_at` on every run of the lineage that has none; returns the runs changed.
    pub async fn mark_thread_reviewed(&self, root_run_id: &str) -> Result<Vec<Run>> {
        let ids: Vec<String> = sqlx::query_scalar(
            "UPDATE runs SET reviewed_at = ? WHERE root_run_id = ? AND reviewed_at IS NULL \
             RETURNING id",
        )
        .bind(now())
        .bind(root_run_id)
        .fetch_all(&self.pool)
        .await?;
        let mut runs = Vec::new();
        for id in ids {
            runs.push(self.get_run(&id).await?);
        }
        Ok(runs)
    }

    pub async fn dispatchable_runs(&self) -> Result<Vec<Run>> {
        let rows = sqlx::query(&format!(
            "SELECT {RUN_COLUMNS} FROM runs r WHERE r.status = 'queued' AND NOT EXISTS \
             (SELECT 1 FROM runs x WHERE x.dot_id = r.dot_id AND x.status = 'running') \
             ORDER BY r.queued_at, r.id"
        ))
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(run_from_row).collect()
    }

    pub async fn claim_run(&self, id: &str) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE runs SET status = 'running', started_at = ? WHERE id = ? AND status = 'queued'",
        )
        .bind(now())
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() == 1)
    }

    pub async fn has_queued_run(&self, dot_id: &str) -> Result<bool> {
        let n: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM runs WHERE dot_id = ? AND status = 'queued'")
                .bind(dot_id)
                .fetch_one(&self.pool)
                .await?;
        Ok(n > 0)
    }

    pub async fn set_workspace(
        &self,
        id: &str,
        path: &str,
        branch: Option<&str>,
        base_commit: Option<&str>,
    ) -> Result<()> {
        sqlx::query("UPDATE runs SET workspace_path = ?, branch = ?, base_commit = ? WHERE id = ?")
            .bind(path)
            .bind(branch)
            .bind(base_commit)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_session(&self, id: &str, session_id: &str) -> Result<()> {
        sqlx::query("UPDATE runs SET session_id = ? WHERE id = ?")
            .bind(session_id)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn add_usage(&self, id: &str, tokens_in: i64, tokens_out: i64) -> Result<()> {
        sqlx::query(
            "UPDATE runs SET tokens_in = tokens_in + ?, tokens_out = tokens_out + ? WHERE id = ?",
        )
        .bind(tokens_in)
        .bind(tokens_out)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn finish_run(
        &self,
        id: &str,
        status: RunStatus,
        summary: Option<&str>,
        error: Option<&str>,
    ) -> Result<Run> {
        sqlx::query(
            "UPDATE runs SET status = ?, summary = COALESCE(?, summary), error = ?, ended_at = ? \
             WHERE id = ?",
        )
        .bind(status.as_str())
        .bind(summary)
        .bind(error)
        .bind(now())
        .bind(id)
        .execute(&self.pool)
        .await?;
        self.get_run(id).await
    }

    /// Marks a run `failed` with `error` only if it is still `running`; `None` otherwise.
    pub async fn fail_if_running(&self, id: &str, error: &str) -> Result<Option<Run>> {
        let res = sqlx::query(
            "UPDATE runs SET status = 'failed', error = ?, ended_at = ? \
             WHERE id = ? AND status = 'running'",
        )
        .bind(error)
        .bind(now())
        .bind(id)
        .execute(&self.pool)
        .await?;
        if res.rows_affected() == 0 {
            return Ok(None);
        }
        Ok(Some(self.get_run(id).await?))
    }

    /// Atomically cancels a run that is `queued` or `awaiting_approval`.
    /// Returns `None` when the run is in any other state (nothing changed).
    pub async fn cancel_if_inactive(&self, id: &str) -> Result<Option<Run>> {
        let res = sqlx::query(
            "UPDATE runs SET status = 'cancelled', ended_at = ?              WHERE id = ? AND status IN ('queued', 'awaiting_approval')",
        )
        .bind(now())
        .bind(id)
        .execute(&self.pool)
        .await?;
        if res.rows_affected() == 0 {
            return Ok(None);
        }
        Ok(Some(self.get_run(id).await?))
    }

    pub async fn set_status(&self, id: &str, status: RunStatus) -> Result<Run> {
        sqlx::query("UPDATE runs SET status = ? WHERE id = ?")
            .bind(status.as_str())
            .bind(id)
            .execute(&self.pool)
            .await?;
        self.get_run(id).await
    }

    /// Marks runs left `running` by a previous process as failed / interrupted and, in the
    /// same transaction, expires their pending approvals (no engine is left to act on them)
    /// and closes the approved calls they were performing (`applying` becomes `unknown`,
    /// `queued` becomes `skipped`). Also closes every call left `applying`, and every call left
    /// `queued` whose resume run is neither queued nor running. Returns the interrupted runs.
    pub async fn recover_interrupted(&self) -> Result<Vec<String>> {
        let ts = now();
        let mut tx = self.pool.begin().await?;
        let ids: Vec<String> = sqlx::query_scalar(
            "UPDATE runs SET status = 'failed', error = 'interrupted', ended_at = ? \
             WHERE status = 'running' RETURNING id",
        )
        .bind(&ts)
        .fetch_all(&mut *tx)
        .await?;
        for id in &ids {
            sqlx::query(
                "UPDATE approvals SET status = 'expired', decided_at = ? \
                 WHERE run_id = ? AND status = 'pending'",
            )
            .bind(&ts)
            .bind(id)
            .execute(&mut *tx)
            .await?;
            // An interrupted resume run was performing its parent's approved calls: one it had
            // claimed has no result, and one it had not reached will not be performed.
            sqlx::query(
                "UPDATE approvals SET apply_state = CASE apply_state WHEN 'applying' \
                 THEN 'unknown' ELSE 'skipped' END \
                 WHERE apply_state IN ('queued', 'applying') \
                 AND run_id = (SELECT parent_run_id FROM runs WHERE id = ?)",
            )
            .bind(id)
            .execute(&mut *tx)
            .await?;
        }
        // No run is running now, so nothing claimed can still get a result.
        sqlx::query("UPDATE approvals SET apply_state = 'unknown' WHERE apply_state = 'applying'")
            .execute(&mut *tx)
            .await?;
        // A queued call is performed only by its run's resume run while that run is still to
        // start or works. One whose resume run ended without closing it (the process stopped
        // between the run's last status and the close, or the close failed) is given up.
        sqlx::query(
            "UPDATE approvals SET apply_state = 'skipped' WHERE apply_state = 'queued' \
             AND NOT EXISTS (SELECT 1 FROM runs c WHERE c.parent_run_id = approvals.run_id \
             AND c.trigger_kind = 'resume' AND c.status IN ('queued', 'running'))",
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(ids)
    }

    pub async fn list_runs_by_status(&self, status: RunStatus) -> Result<Vec<Run>> {
        let rows = sqlx::query(&format!(
            "SELECT {RUN_COLUMNS} FROM runs WHERE status = ? ORDER BY queued_at, id"
        ))
        .bind(status.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(run_from_row).collect()
    }

    pub async fn append_event(
        &self,
        run_id: &str,
        kind: &str,
        data: &Value,
    ) -> Result<RunEventRecord> {
        let ts = now();
        let seq: i64 = sqlx::query_scalar(
            "INSERT INTO run_events (run_id, seq, ts, kind, data) \
             SELECT ?, COALESCE(MAX(seq), 0) + 1, ?, ?, ? FROM run_events WHERE run_id = ? \
             RETURNING seq",
        )
        .bind(run_id)
        .bind(&ts)
        .bind(kind)
        .bind(data.to_string())
        .bind(run_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(RunEventRecord {
            run_id: run_id.to_string(),
            seq,
            ts,
            kind: kind.to_string(),
            data: data.clone(),
        })
    }

    pub async fn list_events(&self, run_id: &str, after_seq: i64) -> Result<Vec<RunEventRecord>> {
        let rows = sqlx::query(
            "SELECT run_id, seq, ts, kind, data FROM run_events WHERE run_id = ? AND seq > ? \
             ORDER BY seq",
        )
        .bind(run_id)
        .bind(after_seq)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(event_from_row).collect()
    }
}
